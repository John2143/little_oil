//! The injected `App` context and every command: pointer/keyboard device ops,
//! screenshots, calibration, stash/map/currency clicking, rolling, and the REPL.
use crate::platform::{Input, InputButton, InputKey, Platform};
use anyhow::bail;
use parking_lot::{Mutex, RwLock};
use rand::seq::SliceRandom;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info};

use crate::auto_roll::{self, AutoRollConfig, AutoRollMod};
use crate::chaos_recipe;
use crate::screenshot::{Rect, ScreenshotData};
use crate::stash_grid::{CellGrid, MAP_COLS, MAP_ROWS, QUAD_COLS, QUAD_ROWS};
use crate::{NamedPoint, ScreenRegion, Settings, config_path, save_config};

/// The injected context: every command is a method on `App`.
///
/// `App` owns all state that used to live in process globals:
/// `settings` (was `static SETTINGS`), `device` (was `static FAKE_DEVICE`),
/// and `vpointer` (was `static VP`). Construct once in `main`, pass
/// `&App`/`&mut App`-equivalent (`&self` + interior mutability) to everything.
pub struct App {
    /// Global settings, shared through a read/write lock. `pub(crate)` so
    /// `auto_roll`/`chaos_recipe` can read the (crate-root-private) `Settings`
    /// fields — see `Settings` in main.rs.
    pub(crate) settings: RwLock<Settings>,
    /// Cross-platform input backend (mouse + keyboard), constructed once.
    input: Mutex<Input>,
}

impl App {
    pub(crate) fn new(settings: Settings) -> anyhow::Result<Self> {
        let platform = settings.platform.unwrap_or_else(Platform::detect);
        let input = Input::new(platform)?;
        Ok(Self {
            settings: RwLock::new(settings),
            input: Mutex::new(input),
        })
    }

    /// Read the currently-configured platform (auto-detected on first run).
    fn platform(&self) -> Platform {
        self.settings
            .read()
            .platform
            .unwrap_or_else(Platform::detect)
    }

    // ── pointer / keyboard primitives ─────────────────────────────

    /// Emit a raw relative pointer move in device units, with no scaling.
    /// Linux only — used by pointer calibration.
    #[cfg(target_os = "linux")]
    fn move_mouse_raw(&self, dx: i32, dy: i32) {
        self.input.lock().move_mouse_raw(dx, dy);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    #[cfg(target_os = "linux")]
    fn pin_cursor_to_origin(&self) {
        self.move_mouse_raw(-50000, -50000);
    }

    fn move_mouse(&self, x: i32, y: i32) {
        let scale = self.settings.read().pointer_scale.unwrap_or(1.25);
        self.input.lock().move_mouse(x, y, scale, self.platform());
    }

    fn emit_button(&self, btn: InputButton, pressed: bool) {
        self.input.lock().button(btn, pressed, self.platform());
    }

    pub(crate) fn click(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        self.emit_button(InputButton::Left, true);
        std::thread::sleep(std::time::Duration::from_millis(10));
        self.emit_button(InputButton::Left, false);
    }

    pub(crate) fn click_right(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        self.emit_button(InputButton::Right, true);
        std::thread::sleep(std::time::Duration::from_millis(10));
        self.emit_button(InputButton::Right, false);
    }

    /// Left click with the minimum settle the input path needs. Used by the
    /// empty macro, which holds Ctrl across a whole pass and re-verifies each
    /// pass with a fresh screenshot — a mistimed click is retried, not lost.
    pub(crate) fn click_fast(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.emit_button(InputButton::Left, true);
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.emit_button(InputButton::Left, false);
    }

    /// Right-click variant of [`click_fast`], for `emptyr`.
    pub(crate) fn click_right_fast(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.emit_button(InputButton::Right, true);
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.emit_button(InputButton::Right, false);
    }

    /// Resolve a named calibrated point to screen coordinates, falling back to
    /// `fallback` when the name is not calibrated. `names` are aliases checked
    /// in order, so both "augment" and "aug" resolve.
    pub(crate) fn point_pos(&self, names: &[&str], fallback: (i32, i32)) -> (i32, i32) {
        let points = self.settings.read().points.clone().unwrap_or_default();
        names
            .iter()
            .find_map(|n| points.iter().find(|p| p.name == *n))
            .map(|p| (p.region.center().0 as i32, p.region.center().1 as i32))
            .unwrap_or(fallback)
    }

    fn try_read_item_on_cursor(&self) -> Option<String> {
        let platform = self.platform();

        // On Wayland, primary selection availability gates the whole tooltip
        // pipeline. On other platforms we try anyway — the regular clipboard
        // is what the game writes to.
        #[cfg(target_os = "linux")]
        if !platform.primary_selection_available() {
            tracing::warn!("primary selection protocol not available; tooltip read may fail");
        }

        // Clear the clipboard so we detect new text from the game.
        if let Err(e) = platform.clear_clipboard() {
            tracing::warn!(?e, "failed to clear clipboard");
            return None;
        }

        // Send Ctrl+Alt+C — the in-game item-tooltip copy shortcut.
        {
            let mut input = self.input.lock();
            input.key(InputKey::Ctrl, true);
            input.key(InputKey::Alt, true);
            std::thread::sleep(std::time::Duration::from_millis(5));
            input.key(InputKey::C, true);
            std::thread::sleep(std::time::Duration::from_millis(rand::random_range(4..25)));
            input.key(InputKey::C, false);
            input.key(InputKey::Alt, false);
            input.key(InputKey::Ctrl, false);
        }

        // Poll the clipboard for up to ~250ms (50 ticks of 5ms).
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            if let Some(text) = platform.read_clipboard_text() {
                return Some(text);
            }
        }

        // Retry up to 5 more times with a random stagger (game lag).
        for retry in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(rand::random_range(1..150)));
            {
                let mut input = self.input.lock();
                input.key(InputKey::Ctrl, true);
                input.key(InputKey::Alt, true);
                input.key(InputKey::C, true);
                std::thread::sleep(std::time::Duration::from_millis(rand::random_range(4..25)));
                input.key(InputKey::C, false);
                input.key(InputKey::Alt, false);
                input.key(InputKey::Ctrl, false);
            }
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(5));
                if let Some(text) = platform.read_clipboard_text() {
                    return Some(text);
                }
            }
            if retry == 4 {
                tracing::warn!("clipboard was always empty, giving up");
            }
        }

        None
    }

    pub(crate) fn read_item_on_cursor(&self) -> Option<String> {
        self.try_read_item_on_cursor()
    }

    /// Append one rolled item's tooltip to the persistent roll log
    /// (`$XDG_CONFIG_HOME/little_oil/rolls.log`, JSONL). Logging never fails the
    /// roll: errors are only traced.
    pub(crate) fn log_roll_item(&self, source: &str, item_text: &str) {
        match crate::rolls_log_path() {
            Ok(path) => {
                if let Err(e) = append_roll_log(&path, source, item_text) {
                    tracing::warn!(?e, "could not write roll log");
                }
            }
            Err(e) => tracing::warn!(?e, "could not determine roll log path"),
        }
    }

    #[cfg(target_os = "linux")]
    fn calibrate_pointer(&self) -> anyhow::Result<()> {
        const D: i32 = 400; // device units; large enough that the two cursor
        // positions cannot overlap even at max deceleration
        println!("Measuring pointer scale. Do not move the mouse or type.");
        println!("Close any animated window; a static desktop measures cleanly.");

        self.pin_cursor_to_origin();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let before = self.platform().capture_all()?;

        self.move_mouse_raw(D, D);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after = self.platform().capture_all()?;

        let bounds = Rect {
            x: 0,
            y: 0,
            width: before.width as u32,
            height: before.height as u32,
        };
        let clusters = crate::screenshot::diff_clusters(&before, &after, bounds, 4)?;
        let [a, b] = clusters.as_slice() else {
            bail!(
                "Expected exactly 2 changed regions (cursor before and after), found {}. \
                 Something on screen is animating — close it and retry.",
                clusters.len()
            );
        };

        let (ax, ay) = a.center();
        let (bx, by) = b.center();
        let dx = (bx as f32 - ax as f32).abs();
        let dy = (by as f32 - ay as f32).abs();
        if dx < 1.0 || dy < 1.0 {
            bail!("Cursor did not move measurably on both axes — got dx={dx}, dy={dy}");
        }

        let sx = D as f32 / dx;
        let sy = D as f32 / dy;
        if (sx - sy).abs() / sx.max(sy) > 0.02 {
            bail!(
                "Axes scale differently (x={sx:.4}, y={sy:.4}) — pointer_scale assumes \
                 one factor. Set pointer_scale manually in config."
            );
        }
        let scale = (sx + sy) / 2.0;

        {
            let mut settings = self.settings.write();
            settings.pointer_scale = Some(scale);
            save_config(&config_path()?, &*settings)?;
        }
        println!("pointer_scale = {scale:.4} (was measured over {D} device units)");
        Ok(())
    }

    /// Generic grid calibration: capture three cells (TOP-LEFT, BOTTOM-RIGHT, any
    /// MIDDLE), each with a nonsense-search base frame and a real-search capture,
    /// then derive the grid from the two corner cells and validate against the
    /// middle cell. `label`/`noun` drive the prompts; `save` persists the grid.
    fn calibrate_grid<const C: usize, const R: usize>(
        &self,
        snapshot: &Settings,
        region: ScreenRegion,
        label: &str, // "stash" | "map"
        noun: &str,  // "item" | "map"
        save: impl FnOnce(&mut Settings, CellGrid<C, R>),
    ) -> anyhow::Result<()> {
        prompt_enter(&format!("Open the {label} tab."))?;
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Each calibration position is captured twice: first with the item in place
        // and a search that matches NOTHING (nonsense string), then with a search
        // matching exactly the item. Diffing those two isolates ONLY the search
        // highlight: the item art is present in both frames (so it cancels out),
        // and any non-matching items in the stash stay dimmed in both frames (so
        // they cancel out too). The highlight ring around the cell is the largest
        // changed region; its bounding box is the cell.
        let capture_pos =
            |where_text: &str| -> anyhow::Result<(ScreenshotData, ScreenshotData, Rect, Rect)> {
                prompt_enter(&format!(
                    "Put the 1×1 {noun} in the {where_text} slot. In the search box type a \
                 nonsense string (e.g. \"zzz\") so NOTHING is highlighted."
                ))?;
                std::thread::sleep(std::time::Duration::from_millis(300));
                let base = snapshot.screenshot()?;

                // Convert region from screen space to frame-pixel space.
                let (bx, by) = base.screen_to_frame(region.x, region.y).ok_or_else(|| {
                anyhow::anyhow!(
                    "{label} region starts outside the game window region — re-run set-region window and set-region {label}"
                )
            })?;
                let bounds = Rect {
                    x: bx,
                    y: by,
                    width: region.width.min(base.width as u32 - bx),
                    height: region.height.min(base.height as u32 - by),
                };

                prompt_enter(&format!(
                    "Now search the {noun}'s exact name so ONLY it is highlighted (keep it in the {where_text} slot)."
                ))?;
                std::thread::sleep(std::time::Duration::from_millis(300));
                let cap = snapshot.screenshot()?;

                let clusters = crate::screenshot::diff_clusters(&base, &cap, bounds, 20)?;
                let cell = clusters
                .iter()
                .max_by_key(|r| r.width * r.height)
                .copied()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No changed pixels in the {where_text} capture — check that the search actually highlights the {noun}"
                    )
                })?;
                Ok((base, cap, bounds, cell))
            };

        let (base, tl, _bounds, tl_cell) = capture_pos("TOP-LEFT")?;

        // Derive highlight_color from the tl cell's bottom boundary row.
        use std::collections::HashMap;
        let mut color_tally: HashMap<u32, u32> = HashMap::new();
        let by_abs = (tl_cell.y + tl_cell.height.saturating_sub(1)) as usize;
        for x in tl_cell.x as usize..(tl_cell.x + tl_cell.width) as usize {
            if base.try_get_pixel(x, by_abs) != tl.try_get_pixel(x, by_abs)
                && let Some(c) = tl.try_get_pixel(x, by_abs)
            {
                *color_tally.entry(c).or_insert(0) += 1;
            }
        }
        let highlight_color = color_tally
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(c, _)| c)
            .ok_or_else(|| anyhow::anyhow!(
                "Could not determine highlight color — the highlighted cell's bottom edge did not change"
            ))?;

        let (_base2, _br, _bounds2, br_cell) = capture_pos("BOTTOM-RIGHT")?;

        let grid = CellGrid::<C, R>::from_corners(tl_cell, br_cell, highlight_color)?;

        let (_base3, mid, _bounds3, mid_cell) = capture_pos("any MIDDLE")?;

        // Find the (col, row) whose cell_center is nearest the middle cluster's center.
        let (mcx, mcy) = mid_cell.center();
        let mut best_col = 0usize;
        let mut best_row = 0usize;
        let mut best_dist = f64::MAX;
        for col in 0..C {
            for row in 0..R {
                let (ccx, ccy) = grid.cell_center(col, row);
                let dx = ccx as f64 - mcx as f64;
                let dy = ccy as f64 - mcy as f64;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < best_dist {
                    best_dist = dist;
                    best_col = col;
                    best_row = row;
                }
            }
        }

        // Validate: the cell center must be inside the mid cluster expanded by 4 px per side.
        let (ccx, ccy) = grid.cell_center(best_col, best_row);
        if ccx < mid_cell.x.saturating_sub(4)
            || ccx > mid_cell.x + mid_cell.width + 4
            || ccy < mid_cell.y.saturating_sub(4)
            || ccy > mid_cell.y + mid_cell.height + 4
        {
            bail!(
                "Calibration failed validation: middle item at ({}, {}) does not line up with computed cell ({}, {}) — re-run calibration",
                mcx,
                mcy,
                ccx,
                ccy
            );
        }

        if !grid.is_highlighted(&mid, best_col, best_row) {
            bail!(
                "Computed grid found the middle cell but its bottom edge does not match the calibrated highlight color — re-run calibration"
            );
        }

        // Hoist the printed values before the grid is moved into the save closure.
        let (sx, sy) = base.frame_to_screen(grid.cols[0], grid.rows[0]);
        let cell_w = grid.cell_w;
        let cell_h = grid.cell_h;
        let origin_x = grid.cols[0];
        let origin_y = grid.rows[0];
        let highlight = grid.highlight_color;

        {
            let mut settings = self.settings.write();
            save(&mut settings, grid);
            save_config(&config_path()?, &*settings)?;
        }

        println!(
            "{label} grid calibrated: cell {cell_w}x{cell_h}, frame-pixel origin ({origin_x}, {origin_y}) = screen ({sx}, {sy}), highlight 0x{highlight:08X}"
        );
        Ok(())
    }

    fn calibrate_stash(&self) -> anyhow::Result<()> {
        let snapshot = { self.settings.read().clone() };

        let _game_region = snapshot.game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — run: little_oil set-region window")
        })?;
        let stash_region = snapshot.stash_region.ok_or_else(|| {
            anyhow::anyhow!("Stash region not set — run: little_oil set-region stash")
        })?;

        self.calibrate_grid::<QUAD_COLS, QUAD_ROWS>(
            &snapshot,
            stash_region,
            "stash",
            "item",
            |s, g| s.stash_grid = Some(g),
        )
    }

    fn calibrate_map(&self) -> anyhow::Result<()> {
        let snapshot = { self.settings.read().clone() };

        let _game_region = snapshot.game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — run: little_oil set-region window")
        })?;
        let map_region = snapshot.map_region.ok_or_else(|| {
            anyhow::anyhow!("Map region not set — run: little_oil set-region map")
        })?;

        self.calibrate_grid::<MAP_COLS, MAP_ROWS>(&snapshot, map_region, "map", "map", |s, g| {
            s.map_grid = Some(g)
        })
    }

    /// Slurp one box around a named clickable target and upsert it into `points`.
    /// Idempotent: re-running overwrites the entry for the same name. Points are
    /// screen-space, so no screenshot or `set-region` prerequisite.
    fn calibrate_point(&self, name: &str) -> anyhow::Result<()> {
        let settings = self.settings.read();
        let platform = settings.platform.unwrap_or_else(Platform::detect);
        drop(settings);
        let region = platform.select_region(&format!("Slurp a small box around {name}"))?;
        let mut settings = self.settings.write();
        let mut points = settings.points.take().unwrap_or_default();
        points.retain(|p| p.name != name);
        points.push(NamedPoint {
            name: name.to_string(),
            region,
        });
        settings.points = Some(points);
        save_config(&config_path()?, &*settings)?;
        println!("Point '{name}' saved at ({}, {})", region.x, region.y);
        Ok(())
    }

    /// The screen point we normally click to focus the game: bottom-middle of
    /// the inventory panel (falling back to the game window). Parking the
    /// cursor here before screenshots keeps no item hovered in the capture.
    fn focus_click_point(&self) -> anyhow::Result<(i32, i32)> {
        let region = {
            let settings = self.settings.read();
            settings
                .inv_region
                .or(settings.game_window_region)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No region to click for focus — run: little_oil set-region inventory (or set-region window)"
                    )
                })?
        };
        Ok((
            (region.x + region.width / 2) as i32,
            (region.y + region.height.saturating_sub(2)) as i32,
        ))
    }

    /// Move the cursor to the focus-click spot and wait a frame, so the next
    /// screenshot captures no hover highlight from where the cursor was (the
    /// last clicked slot) and no item is hovered while we probe.
    fn park_cursor(&self) -> anyhow::Result<()> {
        let (sx, sy) = self.focus_click_point()?;
        self.move_mouse(sx, sy);
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(())
    }

    /// Click the bottom-middle of the player inventory panel so the game window
    /// receives keyboard focus before any automation starts. Without this, a
    /// terminal-launched command leaves keyboard focus in the terminal, and the
    /// Ctrl the operator holds (or the empty macro sends) never reaches the game.
    pub(crate) fn focus_game_window(&self) -> anyhow::Result<()> {
        let (sx, sy) = self.focus_click_point()?;
        println!("Focus click at ({sx}, {sy}) — game window should come to the foreground");
        // Click `focus_clicks` times: click-to-focus compositors (Hyprland) consume
        // the first click just to hand focus to the window, so the second lands in
        // the now-focused game. Compositors that pass the first click through (some
        // niri setups) would double-grab an item on the click target — set
        // focus_clicks = 1 there (config.json).
        let clicks = { self.settings.read().focus_clicks };
        for _ in 0..clicks {
            self.click(sx, sy);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        Ok(())
    }

    /// Click a calibrated named point (currency slot, filter button, …).
    fn click_point(&self, name: &str) -> anyhow::Result<()> {
        self.focus_game_window()?;
        let settings = self.settings.read();
        let point = settings
            .points
            .as_ref()
            .and_then(|ps| ps.iter().find(|p| p.name == name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No calibrated point named '{name}' — run: little_oil calibrate-point {name}"
                )
            })?;
        let (sx, sy) = point.region.center();
        drop(settings);
        self.click(sx as i32, sy as i32);
        Ok(())
    }

    /// Click cell (col, row) of the calibrated map grid.
    fn click_map_cell(&self, col: usize, row: usize) -> anyhow::Result<()> {
        self.focus_game_window()?;
        let settings = self.settings.read();
        let grid = match &settings.map_grid {
            Some(g) => g.clone(),
            None => bail!("Map grid not calibrated — run: little_oil calibrate-map"),
        };
        let frame = settings.screenshot()?;
        drop(settings);
        let (px, py) = grid.cell_center(col, row);
        let (sx, sy) = frame.frame_to_screen(px, py);
        self.click(sx, sy);
        Ok(())
    }

    fn stash_copy(&self) -> anyhow::Result<()> {
        self.focus_game_window()?;

        let settings = self.settings.read();
        let grid = match &settings.stash_grid {
            Some(g) => g.clone(),
            None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
        };
        let frame = settings.screenshot()?;
        drop(settings);

        let mut seen: Vec<String> = Vec::new();
        let mut failed = 0u32;

        for row in 0..QUAD_ROWS {
            for col in 0..QUAD_COLS {
                if !grid.is_highlighted(&frame, col, row) {
                    continue;
                }
                let (px, py) = grid.cell_center(col, row);
                let (sx, sy) = frame.frame_to_screen(px, py);
                self.move_mouse(sx, sy);
                std::thread::sleep(std::time::Duration::from_millis(30));
                match self.try_read_item_on_cursor() {
                    Some(text) if !seen.contains(&text) => seen.push(text),
                    Some(_) => {}
                    None => failed += 1,
                }
            }
        }

        for item in &seen {
            println!("{item}");
            println!("--------");
        }
        println!(
            "{} unique items, {} cells failed to copy",
            seen.len(),
            failed
        );

        Ok(())
    }

    fn chance(&self) -> anyhow::Result<()> {
        let chance = self.point_pos(&["chance"], (237, 292));
        let scour = self.point_pos(&["scour"], (169, 472));
        let slot = self.point_pos(&["slot"], (323, 522));
        let sleep_click = 30;
        let sleep_read = 250;

        for _ in 1..10 {
            self.click_right(chance.0, chance.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_click));
            self.click(slot.0, slot.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_read));

            self.click_right(scour.0, scour.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_click));
            self.click(slot.0, slot.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_read));
        }

        Ok(())
    }

    /// Three probes across the vertical middle of an inventory slot, at 25%, 50%
    /// and 75% of its width, in frame-pixel space. None if the slot falls outside
    /// the captured frame.
    fn inv_probes(
        frame: &ScreenshotData,
        region: ScreenRegion,
        col: u32,
        row: u32,
    ) -> Option<[(usize, usize); 3]> {
        if region.width < 12 || region.height < 5 {
            return None;
        }
        let dx = region.width / 12;
        let dy = region.height / 5;
        // Validate the far corner of the slot is in frame.
        let _far =
            frame.screen_to_frame(region.x + (col + 1) * dx - 1, region.y + (row + 1) * dy - 1)?;
        let (ox, oy) = frame.screen_to_frame(region.x + col * dx, region.y + row * dy)?;
        let y = (oy + dy / 2) as usize;
        Some([
            ((ox + dx / 4) as usize, y),
            ((ox + dx / 2) as usize, y),
            ((ox + dx * 3 / 4) as usize, y),
        ])
    }

    fn reset_inv_colors(&self) -> anyhow::Result<()> {
        let settings = self.settings.read();
        let inv_region = settings.inv_region.ok_or_else(|| {
            anyhow::anyhow!(
                "Inventory region not calibrated — run: little_oil set-region inventory"
            )
        })?;

        let frame = settings.screenshot()?;
        drop(settings);

        let mut samples = vec![[0u32; 3]; 60];

        for x in 0..12 {
            for y in 0..5 {
                let probes = Self::inv_probes(&frame, inv_region, x, y).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Inventory slot ({x}, {y}) falls outside the game window region — re-run set-region window and set-region inventory"
                    )
                })?;
                samples[(x * 5 + y) as usize] = [
                    frame.try_get_pixel(probes[0].0, probes[0].1).unwrap_or(0),
                    frame.try_get_pixel(probes[1].0, probes[1].1).unwrap_or(0),
                    frame.try_get_pixel(probes[2].0, probes[2].1).unwrap_or(0),
                ];
            }
        }

        let mut settings = self.settings.write();

        let note = Command::new("notify-send")
            .args([
                "-u",
                "low",
                "Little Oil",
                &format!(
                    "Inventory colors calibrated: {} slots x 3 samples",
                    samples.len()
                ),
            ])
            .spawn();
        if let Err(e) = note {
            eprintln!("notify-send failed: {e}");
        }
        settings.inv_samples = Some(samples);

        save_config(&config_path()?, &*settings)?;
        Ok(())
    }

    /// Occupied inventory cells in `frame` — fewer than 2 of 3 probe pixels
    /// matching the calibrated empty-slot sample — as screen coordinates.
    fn occupied_inv_cells(
        frame: &ScreenshotData,
        inv_region: ScreenRegion,
        expected: &[[u32; 3]],
    ) -> anyhow::Result<Vec<(i32, i32)>> {
        let mut cells = Vec::new();
        for x in 0..12 {
            for y in 0..5 {
                let probes = Self::inv_probes(frame, inv_region, x, y).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Inventory slot ({x}, {y}) falls outside the game window region — re-run set-region window and set-region inventory"
                    )
                })?;
                let actual = [
                    frame.try_get_pixel(probes[0].0, probes[0].1).unwrap_or(0),
                    frame.try_get_pixel(probes[1].0, probes[1].1).unwrap_or(0),
                    frame.try_get_pixel(probes[2].0, probes[2].1).unwrap_or(0),
                ];
                let stored = expected[(x * 5 + y) as usize];
                let matches = actual
                    .iter()
                    .zip(stored.iter())
                    .filter(|(a, e)| a == e)
                    .count();

                if matches < 2 {
                    debug!(x, y, "clicking inv");
                    let (px, py) = (probes[1].0 as u32, probes[1].1 as u32);
                    let (sx, sy) = frame.frame_to_screen(px, py);
                    cells.push((sx, sy));
                }
            }
        }
        Ok(cells)
    }

    /// Empty the inventory: screenshot, click every occupied cell fast, then
    /// re-screenshot and repeat so clicks the game missed get retried. Up to 3
    /// passes; Ctrl is held for the whole pass so every click is a move.
    /// Returns (items clicked, cells still occupied after the last pass).
    fn empty_inv_macro(&self, clicker: fn(&App, i32, i32)) -> anyhow::Result<(u32, u32)> {
        let settings = self.settings.read();
        let inv_region = settings.inv_region.ok_or_else(|| {
            anyhow::anyhow!(
                "Inventory region not calibrated — run: little_oil set-region inventory"
            )
        })?;

        info!("Emptying inv");

        let expected = match settings.inv_samples.as_ref() {
            Some(s) if s.len() == 60 => s,
            _ => bail!("Inventory colors not calibrated — run: little_oil reset_inv"),
        };

        let mut clicked: u32 = 0;
        let mut remaining: u32 = 0;
        let mut prev_occupied: u32 = 0;
        for pass in 0..3 {
            // Park the cursor off any item and wait a frame, so no slot is
            // hovered (and no highlight pollutes the probe pixels).
            self.park_cursor()?;
            let frame = settings.screenshot()?;
            let mut cells = Self::occupied_inv_cells(&frame, inv_region, expected)?;
            let occupied = cells.len() as u32;
            info!(pass = pass + 1, occupied, first_cell = ?cells.first(), "empty pass");
            if occupied == 0 {
                remaining = 0;
                if pass == 0 {
                    // First pass detected nothing — dump the frame and the
                    // probe-vs-sample pixels so we can see what grim captured
                    // and whether the probes still land on the game window.
                    let path = std::path::Path::new("/tmp/little_oil_empty_pass1.png");
                    match frame.save_png(path) {
                        Ok(()) => info!(?path, "saved pass-1 frame for inspection"),
                        Err(e) => tracing::error!(?e, "failed to save pass-1 frame"),
                    }
                    if let Some(probes) = Self::inv_probes(&frame, inv_region, 0, 0) {
                        info!(
                            sample = ?expected[0],
                            probe = ?frame.try_get_pixel(probes[1].0, probes[1].1),
                            "pass-1 cell (0,0) probe vs sample"
                        );
                    }
                }
                break;
            }
            if occupied == 60 && pass == 0 {
                // The whole inventory reads occupied — correct, or the capture
                // is off (wrong region, black frame from direct scanout).
                let path = std::path::Path::new("/tmp/little_oil_empty_pass1.png");
                if let Err(e) = frame.save_png(path) {
                    tracing::error!(?e, "failed to save pass-1 frame");
                }
            }
            if occupied == prev_occupied {
                // The last pass's clicks moved nothing (stash full, game lost
                // the input) — retrying the same cells again is pointless.
                remaining = occupied;
                break;
            }
            prev_occupied = occupied;
            remaining = occupied;

            // Click the occupied cells in a random order so the macro never
            // produces the same fixed scan pattern twice.
            cells.shuffle(&mut rand::rng());

            self.input.lock().key(InputKey::Ctrl, true);
            std::thread::sleep(std::time::Duration::from_millis(5));
            for (sx, sy) in &cells {
                clicker(self, *sx, *sy);
                clicked += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            self.input.lock().key(InputKey::Ctrl, false);
            // Give the last item's move a beat before the next screenshot.
            std::thread::sleep(std::time::Duration::from_millis(30));
        }

        // If the loop ended while cells were still occupied (the 3rd pass
        // clicked something, or the stuck-break), count what actually remains
        // so the notification never claims items were cleared when they weren't.
        if remaining > 0 {
            // Same park before the recount — the cursor is still over the last
            // clicked slot.
            self.park_cursor()?;
            let frame = settings.screenshot()?;
            remaining = Self::occupied_inv_cells(&frame, inv_region, expected)?.len() as u32;
        }

        Ok((clicked, remaining))
    }

    fn empty_inv(&self) -> anyhow::Result<()> {
        self.empty_inv_with(App::click_fast)
    }

    fn empty_inv_right(&self) -> anyhow::Result<()> {
        self.empty_inv_with(App::click_right_fast)
    }

    fn empty_inv_with(&self, clicker: fn(&App, i32, i32)) -> anyhow::Result<()> {
        self.focus_game_window()?;

        // A short beat after the focus click; the macro's screenshots drive
        // the actual pacing from here.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let (clicked, remaining) = self.empty_inv_macro(clicker)?;
        let note = Command::new("notify-send")
            .args([
                "-u",
                "low",
                "Little Oil",
                &format!(
                    "Inventory cleared: {} clicked, {} remain",
                    clicked, remaining
                ),
            ])
            .spawn();
        if let Err(e) = note {
            tracing::error!(?e, "notify-send failed");
        }
        Ok(())
    }

    fn sort_quad(&self, times: u32) -> anyhow::Result<()> {
        self.focus_game_window()?;
        std::thread::sleep(std::time::Duration::from_millis(300));

        let settings = self.settings.read();
        let delay = settings.pull_delay;
        let grid = match &settings.stash_grid {
            Some(g) => g.clone(),
            None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
        };
        let frame = settings.screenshot()?;
        drop(settings);

        let mut movesleft = times;
        for row in 0..QUAD_ROWS {
            for col in 0..QUAD_COLS {
                if movesleft < 1 {
                    return Ok(());
                }
                if grid.is_highlighted(&frame, col, row) {
                    let (px, py) = grid.cell_center(col, row);
                    let (sx, sy) = frame.frame_to_screen(px, py);
                    self.click(sx, sy);
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    movesleft -= 1;
                }
            }
        }
        Ok(())
    }

    /// Returns after one command, or starts the interactive REPL when no command
    /// was given (matching the previous behavior of main()).
    pub(crate) fn run(self, args: &[String]) -> anyhow::Result<()> {
        match args.first().map(|x| &**x) {
            Some("config") => {
                // Print the ACTIVE config. Calibration is only inspectable this way.
                println!("{}", serde_json::to_string_pretty(&*self.settings.read())?);
                return Ok(());
            }
            Some("version") | Some("--version") => {
                println!("little_oil v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("sort") => {
                let times: u32 = args.get(1).map(|x| x.parse()).transpose()?.unwrap_or(40);
                return self.sort_quad(times);
            }
            Some("empty") => return self.empty_inv(),
            Some("emptyr") => return self.empty_inv_right(),
            Some("roll") => {
                let file = args.get(1).ok_or_else(|| {
                    anyhow::anyhow!("Usage: little_oil roll <chrome-file> <times>")
                })?;
                let times: i64 = args
                    .get(2)
                    .ok_or_else(|| anyhow::anyhow!("Usage: little_oil roll <chrome-file> <times>"))?
                    .parse()?;

                auto_roll::auto_roll(&self, file, times);
                return Ok(());
            }
            Some("reset_inv") => return self.reset_inv_colors(),
            Some("calibrate-pointer") => {
                // Platforms with absolute pointer positioning (niri on Linux,
                // Windows) don't need pointer_scale calibration.
                if self.platform().uses_absolute_pointer() {
                    bail!(
                        "calibrate-pointer is not needed on this platform — pointer \
                         control uses absolute coordinates. Set pointer_scale manually \
                         in config only if you see coordinate drift."
                    );
                }
                #[cfg(target_os = "linux")]
                return self.calibrate_pointer();
                #[cfg(not(target_os = "linux"))]
                bail!(
                    "calibrate-pointer is not supported on this platform — pointer control uses absolute coordinates."
                );
            }
            Some("calibrate-stash") => return self.calibrate_stash(),
            Some("calibrate-map") => return self.calibrate_map(),
            Some("calibrate-point") => {
                let name = args
                    .get(1)
                    .ok_or_else(|| anyhow::anyhow!("Usage: little_oil calibrate-point <name>"))?;
                return self.calibrate_point(name);
            }
            Some("calibrate-currency") => {
                for name in [
                    "transmute",
                    "alt",
                    "annul",
                    "chance",
                    "augment",
                    "regal",
                    "chaos",
                    "scour",
                    "alchemy",
                    "exalt",
                ] {
                    prompt_enter(&format!(
                        "Open the currency tab if it isn't already. Next point: {name} — press Enter to slurp its slot."
                    ))?;
                    self.calibrate_point(name)?;
                }
                return Ok(());
            }
            Some("stash") => {
                let mode = args.get(1).map(|x| &**x);
                match mode {
                    Some("click") => {
                        let times: u32 = args.get(2).map(|x| x.parse()).transpose()?.unwrap_or(40);
                        return self.sort_quad(times);
                    }
                    Some("copy") => return self.stash_copy(),
                    _ => {
                        println!("Usage: little_oil stash <click|copy> [times]");
                        println!(
                            "  click <times>  Left-click every highlighted cell (hold Ctrl to pull, Shift to identify)"
                        );
                        println!(
                            "  copy           Hover every highlighted cell and Ctrl+Alt+C it, printing unique items"
                        );
                        return Ok(());
                    }
                }
            }
            Some("click") => match args.get(1).map(|x| &**x) {
                Some("map") => {
                    let col: usize = args
                        .get(2)
                        .ok_or_else(|| anyhow::anyhow!("Usage: little_oil click map <col> <row>"))?
                        .parse()?;
                    let row: usize = args
                        .get(3)
                        .ok_or_else(|| anyhow::anyhow!("Usage: little_oil click map <col> <row>"))?
                        .parse()?;
                    if col >= MAP_COLS || row >= MAP_ROWS {
                        bail!(
                            "map cell ({col}, {row}) out of range — cols 0..{MAP_COLS}, rows 0..{MAP_ROWS}"
                        );
                    }
                    return self.click_map_cell(col, row);
                }
                Some(name) => return self.click_point(name),
                None => {
                    println!("Usage: little_oil click <name> | click map <col> <row>");
                    println!(
                        "  <name>    A calibrated point: filter, or a currency (chaos, alch, …)"
                    );
                    return Ok(());
                }
            },
            Some("set-region") => {
                let region_name = args.get(1).map(|x| &**x).unwrap_or("help");
                let platform = self
                    .settings
                    .read()
                    .platform
                    .unwrap_or_else(Platform::detect);
                let region = match region_name {
                    "inventory" => {
                        let r = platform.select_region(
                            "Select the INVENTORY grid region (top-left slot to bottom-right slot)",
                        )?;
                        self.settings.write().inv_region = Some(r);
                        r
                    }
                    "stash" => {
                        let r = platform.select_region(
                            "Select the STASH grid region (top-left slot to bottom-right slot)",
                        )?;
                        self.settings.write().stash_region = Some(r);
                        r
                    }
                    "window" => {
                        let r = platform.select_region(
                            "Select the GAME WINDOW (drag around the entire PoE window)",
                        )?;
                        self.settings.write().game_window_region = Some(r);
                        r
                    }
                    "map" => {
                        let r = platform.select_region(
                            "Select the MAP grid region (the 12x7 map window, not the sub-tab row)",
                        )?;
                        self.settings.write().map_region = Some(r);
                        r
                    }
                    _ => {
                        eprintln!("Usage: little_oil set-region <inventory|stash|window|map>");
                        return Ok(());
                    }
                };
                #[cfg(not(target_os = "linux"))]
                let _ = region;

                save_config(&config_path()?, &*self.settings.read())?;
                #[cfg(target_os = "linux")]
                let _ = Command::new("notify-send")
                    .args([
                        "-u",
                        "low",
                        "Little Oil",
                        &format!(
                            "{region_name} region saved: {}x{} at ({}, {})",
                            region.width, region.height, region.x, region.y
                        ),
                    ])
                    .spawn();

                return Ok(());
            }
            Some("chance") => return self.chance(),
            Some("tally") => {
                let c = self
                    .settings
                    .read()
                    .chaos_recipe_settings
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("No chaos recipe config found"))?;
                chaos_recipe::get_tally(&self, &c)?;
                return Ok(());
            }
            Some("chaos") => {
                let amt: usize = args.get(1).map(|x| x.parse()).transpose()?.unwrap_or(1);
                let c = self
                    .settings
                    .read()
                    .chaos_recipe_settings
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("No chaos recipe config found"))?;
                chaos_recipe::do_recipe(&self, &c, amt)?;
                return Ok(());
            }
            Some(n) => {
                println!("Invalid command: {n}");
                return Ok(());
            }
            None => {}
        }

        let app = Arc::new(self);
        let app2 = Arc::clone(&app);
        std::thread::spawn(move || app2.command_line())
            .join()
            .unwrap();
        Ok(())
    }
}

fn split_space(input: &str) -> (&str, &str) {
    for (i, _c) in input.char_indices() {
        if input.as_bytes()[i] == b' ' {
            return (&input[0..i], &input[i + 1..]);
        }
    }
    (input, "")
}
/// Print a prompt, mirror it to notify-send on Linux, and block until Enter.
fn prompt_enter(msg: &str) -> anyhow::Result<()> {
    println!("\n>>> {msg}\n    Press Enter when ready...");
    #[cfg(target_os = "linux")]
    let _ = Command::new("notify-send")
        .args(["-u", "critical", "Little Oil — calibration", msg])
        .spawn();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

const HELP: &str = r#"
help: Show this menu
version: Print version and exit
empty: Empty the inventory into the stash (ctrl+left click)
emptyr: Empty the inventory into the stash (ctrl+right click)
set-region <inventory|stash|window|map>: Select and save a screen region
calibrate-pointer: Measure pointer scale (run once per machine)
calibrate-stash: Calibrate the 24x24 quad tab grid (3 positions, base + search capture each)
calibrate-map: Calibrate the 12x7 map tab grid (3 positions, base + search capture each)
calibrate-point <name>: Slurp a small box and save it as a named clickable point
calibrate-currency: Calibrate the 10 currency slots (transmute, alt, annul, chance, augment, regal, chaos, scour, alchemy, exalt)
click <name>: Click a calibrated point (e.g. filter, chaos)
click map <col> <row>: Click a cell in the calibrated map grid
stash <click|copy> [times]: Act on highlighted quad-tab cells
pull <delay>: Change delay for pulling out of quad tab
div <delay>: Change delay for div macro
chrome <file> <times>: Open an auto-roll file, with name <file>, and roll item <times>
mchrome <file>: Create example chrome file with name <file>. To be used with chrome later.

Press CTRL + C to quit this program.
"#;

impl App {
    fn command_line(&self) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let (cmd, rest) = split_space(&line);
            match cmd {
                "pull" | "div" => match rest.parse::<u64>() {
                    Ok(x) => {
                        let mut s = self.settings.write();
                        match cmd {
                            "pull" => s.pull_delay = x,
                            "div" => s.div_delay = x,
                            _ => unreachable!(),
                        }
                        println!("{cmd} delay is {x}");
                        let path = match config_path() {
                            Ok(p) => p,
                            Err(e) => {
                                println!("could not determine config path: {e}");
                                continue;
                            }
                        };
                        if let Err(e) = save_config(&path, &*s) {
                            println!("could not save config: {e}");
                        }
                    }
                    Err(_) => println!("invalid delay: {rest}"),
                },
                "chrome" => {
                    let (file, times) = split_space(rest);
                    println!("Loading chrome file {}", file);

                    let Ok(times) = times.parse::<i64>() else {
                        println!("invalid times: {times}");
                        continue;
                    };
                    match auto_roll::auto_roll(self, file, times) {
                        None => println!("failed to roll"),
                        Some(res) => {
                            println!("{:?}", res);
                        }
                    }
                }
                "mchrome" => {
                    println!("Making chrome file {}", rest);

                    if let Err(e) = save_config(
                        Path::new(rest),
                        &AutoRollConfig {
                            auto_aug_regal: false,
                            item_name: "Medium Cluster Jewel".to_string(),
                            any_two_t1: false,
                            needs_prefix_and_suffix: false,
                            mods: vec![
                                AutoRollMod {
                                    name: "heraldry".into(),
                                    is_prefix: true,
                                },
                                AutoRollMod {
                                    name: "harbinger".into(),
                                    is_prefix: true,
                                },
                                AutoRollMod {
                                    name: "endbringer".into(),
                                    is_prefix: true,
                                },
                            ],
                        },
                    ) {
                        println!("could not save chrome file: {e}");
                    }
                }
                "help" => {
                    println!("Available Commands: {}", HELP);
                }
                _ => println!("Unknown command"),
            }
        }
    }
}
/// Append one JSONL record `{"time": <unix secs>, "source": <chrome file>, "item": <tooltip>}`
/// to `path`, creating it if missing. `item_text` is multi-line; serde_json
/// escapes it, so the file stays one JSON object per line.
fn append_roll_log(path: &std::path::Path, source: &str, item_text: &str) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let rec = serde_json::json!({
        "time": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "source": source,
        "item": item_text,
    });
    let mut line = serde_json::to_string(&rec).map_err(io::Error::other)?;
    line.push('\n');
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_roll_log_writes_jsonl_records() {
        let path =
            std::env::temp_dir().join(format!("little_oil_roll_log_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        append_roll_log(&path, "chrome_a.json", "line1\nline2").unwrap();
        append_roll_log(&path, "chrome_b.json", "second item").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON object per line");
        for line in &lines {
            assert!(!line.is_empty());
        }

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["source"], "chrome_a.json");
        assert_eq!(first["item"], "line1\nline2");
        assert!(first["time"].is_number());

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["source"], "chrome_b.json");
        assert_eq!(second["item"], "second item");
    }
}
