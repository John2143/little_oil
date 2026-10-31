//! The injected `App` context and every command: pointer/keyboard device ops,
//! screenshots, calibration, stash/map/currency clicking, rolling, and the REPL.
use crate::platform::{self, Backend, Desktop, InputButton, InputKey};
use anyhow::bail;
use parking_lot::RwLock;
use rand::seq::SliceRandom;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info};

use crate::auto_roll::{self, AutoRollConfig, AutoRollMod};
use crate::chaos_recipe;
use crate::screenshot::{Rect, ScreenshotData};
use crate::stash_grid::{CellGrid, MAP_COLS, MAP_ROWS, QUAD_COLS, QUAD_ROWS};
use crate::{NamedPoint, ScreenRegion, Settings, save_config};

/// The injected context: every command is a method on `App`.
///
/// `App` owns all state that used to live in process globals:
/// `settings` (was `static SETTINGS`), `backend` (was `static FAKE_DEVICE` and
/// `static VP`). Construct once in `main`, pass `&App`/`&mut App`-equivalent
/// (`&self` + interior mutability) to everything.
pub struct App {
    /// Global settings, shared through a read/write lock. `pub(crate)` so
    /// `auto_roll`/`chaos_recipe` can read the (crate-root-private) `Settings`
    /// fields — see `Settings` in main.rs.
    pub(crate) settings: RwLock<Settings>,
    /// Capture, input, clipboard and notification backend, chosen once at
    /// startup (see `platform::detect_backend`).
    backend: Box<dyn Backend>,
    /// Where `save_settings` writes. Passed in so tests never touch the real
    /// user config.
    pub(crate) config_path: PathBuf,
    /// Test-only redirect for the roll log. `None` (always, outside tests)
    /// means `rolls_log_path()`, i.e. the user's real `$XDG_CONFIG_HOME`.
    #[cfg(test)]
    pub(crate) roll_log_override: Option<PathBuf>,
}

impl App {
    pub(crate) fn new(settings: Settings, backend: Box<dyn Backend>, config_path: PathBuf) -> Self {
        Self {
            settings: RwLock::new(settings),
            backend,
            config_path,
            #[cfg(test)]
            roll_log_override: None,
        }
    }

    /// The desktop backend (capture, input, clipboard).
    pub(crate) fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    /// The desktop we are automating: the config override, else auto-detected.
    pub(crate) fn desktop(&self) -> Desktop {
        self.settings
            .read()
            .desktop
            .unwrap_or_else(platform::detect_desktop)
    }

    // ── pointer / keyboard primitives ─────────────────────────────

    fn move_mouse(&self, x: i32, y: i32) {
        self.backend.move_to(x, y);
    }

    fn emit_button(&self, btn: InputButton, pressed: bool) {
        self.backend.button(btn, pressed);
    }

    pub(crate) fn click(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        self.backend.sleep_ms(30);
        self.emit_button(InputButton::Left, true);
        self.backend.sleep_ms(10);
        self.emit_button(InputButton::Left, false);
    }

    pub(crate) fn click_right(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        self.backend.sleep_ms(30);
        self.emit_button(InputButton::Right, true);
        self.backend.sleep_ms(10);
        self.emit_button(InputButton::Right, false);
    }

    /// Left click with the minimum settle the input path needs. Used by the
    /// empty macro, which holds Ctrl across a whole pass and re-verifies each
    /// pass with a fresh screenshot — a mistimed click is retried, not lost.
    pub(crate) fn click_fast(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        self.backend.sleep_ms(5);
        self.emit_button(InputButton::Left, true);
        self.backend.sleep_ms(5);
        self.emit_button(InputButton::Left, false);
    }

    /// Right-click variant of [`click_fast`], for `emptyr`.
    pub(crate) fn click_right_fast(&self, x: i32, y: i32) {
        self.move_mouse(x, y);
        self.backend.sleep_ms(5);
        self.emit_button(InputButton::Right, true);
        self.backend.sleep_ms(5);
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

    /// Persist the current settings to the config file. Snapshots under a short
    /// read lock and writes with no lock held, so disk I/O never blocks other
    /// threads' settings access. Also refreshes the backend's pointer scale.
    pub(crate) fn save_settings(&self) -> anyhow::Result<()> {
        let (snapshot, pointer_scale) = {
            let s = self.settings.read();
            (s.clone(), s.pointer_scale.unwrap_or(1.25))
        };
        self.backend.set_pointer_scale(pointer_scale);
        save_config(&self.config_path, &snapshot)?;
        Ok(())
    }

    /// Capture the configured game-window region through the backend.
    pub(crate) fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        let region = self.settings.read().game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — run: little_oil set-region window")
        })?;
        self.backend.capture_region(region)
    }

    fn try_read_item_on_cursor(&self) -> Option<String> {
        // On Wayland, primary selection availability gates the whole tooltip
        // pipeline. Elsewhere we try anyway — the regular clipboard is what the
        // game writes to.
        #[cfg(target_os = "linux")]
        if let Some(wlr) = self.backend.as_wlr()
            && !wlr.primary_selection_available()
        {
            tracing::warn!("primary selection protocol not available; tooltip read may fail");
        }

        // Clear the clipboard so we detect new text from the game.
        if let Err(e) = self.backend.clear_clipboard() {
            tracing::warn!(?e, "failed to clear clipboard");
            return None;
        }

        // Send Ctrl+Alt+C — the in-game item-tooltip copy shortcut.
        self.backend.key(InputKey::Ctrl, true);
        self.backend.key(InputKey::Alt, true);
        self.backend.sleep_ms(5);
        self.backend.key(InputKey::C, true);
        self.backend.sleep_ms(rand::random_range(4..25));
        self.backend.key(InputKey::C, false);
        self.backend.key(InputKey::Alt, false);
        self.backend.key(InputKey::Ctrl, false);

        // Poll the clipboard for up to ~250ms (50 ticks of 5ms).
        for _ in 0..50 {
            self.backend.sleep_ms(5);
            if let Some(text) = self.backend.read_clipboard_text() {
                return Some(text);
            }
        }

        // Retry up to 5 more times with a random stagger (game lag).
        for retry in 0..5 {
            self.backend.sleep_ms(rand::random_range(1..150));
            self.backend.key(InputKey::Ctrl, true);
            self.backend.key(InputKey::Alt, true);
            self.backend.key(InputKey::C, true);
            self.backend.sleep_ms(rand::random_range(4..25));
            self.backend.key(InputKey::C, false);
            self.backend.key(InputKey::Alt, false);
            self.backend.key(InputKey::Ctrl, false);
            for _ in 0..50 {
                self.backend.sleep_ms(5);
                if let Some(text) = self.backend.read_clipboard_text() {
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
        #[cfg(test)]
        let redirected = self.roll_log_override.clone();
        #[cfg(not(test))]
        let redirected: Option<PathBuf> = None;
        let path = redirected.map_or_else(crate::rolls_log_path, Ok);
        match path {
            Ok(path) => {
                if let Err(e) = append_roll_log(&path, source, item_text) {
                    tracing::warn!(?e, "could not write roll log");
                }
            }
            Err(e) => tracing::warn!(?e, "could not determine roll log path"),
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn calibrate_pointer(&self) -> anyhow::Result<()> {
        const D: i32 = 400; // device units; large enough that the two cursor
        // positions cannot overlap even at max deceleration
        println!("Measuring pointer scale. Do not move the mouse or type.");
        println!("Close any animated window; a static desktop measures cleanly.");

        let wlr = self.backend.as_wlr().ok_or_else(|| {
            anyhow::anyhow!("pointer calibration needs the Wayland (wlr) backend")
        })?;

        wlr.pin_cursor_to_origin();
        self.backend.sleep_ms(200);
        let before = wlr.capture_all()?;

        wlr.move_mouse_raw(D, D);
        self.backend.sleep_ms(200);
        let after = wlr.capture_all()?;

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

        self.settings.write().pointer_scale = Some(scale);
        self.save_settings()?;
        println!("pointer_scale = {scale:.4} (was measured over {D} device units)");
        Ok(())
    }

    /// Generic grid calibration: capture three cells (TOP-LEFT, BOTTOM-RIGHT, any
    /// MIDDLE), each with a nonsense-search base frame and a real-search capture,
    /// then derive the grid from the two corner cells and validate against the
    /// middle cell. `label`/`noun` drive the prompts; `save` persists the grid.
    fn calibrate_grid<const C: usize, const R: usize>(
        &self,
        region: ScreenRegion,
        label: &str, // "stash" | "map"
        noun: &str,  // "item" | "map"
        save: impl FnOnce(&mut Settings, CellGrid<C, R>),
    ) -> anyhow::Result<()> {
        prompt_enter(&format!("Open the {label} tab."))?;
        self.backend.sleep_ms(300);

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
                self.backend.sleep_ms(300);
                let base = self.screenshot()?;

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
                self.backend.sleep_ms(300);
                let cap = self.screenshot()?;

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
        }
        self.save_settings()?;

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

        self.calibrate_grid::<QUAD_COLS, QUAD_ROWS>(stash_region, "stash", "item", |s, g| {
            s.stash_grid = Some(g)
        })
    }

    fn calibrate_map(&self) -> anyhow::Result<()> {
        let snapshot = { self.settings.read().clone() };

        let _game_region = snapshot.game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — run: little_oil set-region window")
        })?;
        let map_region = snapshot.map_region.ok_or_else(|| {
            anyhow::anyhow!("Map region not set — run: little_oil set-region map")
        })?;

        self.calibrate_grid::<MAP_COLS, MAP_ROWS>(map_region, "map", "map", |s, g| {
            s.map_grid = Some(g)
        })
    }

    /// Slurp one box around a named clickable target and upsert it into `points`.
    /// Idempotent: re-running overwrites the entry for the same name. Points are
    /// screen-space, so no screenshot or `set-region` prerequisite.
    fn calibrate_point(&self, name: &str) -> anyhow::Result<()> {
        let region = platform::select_region(&format!("Slurp a small box around {name}"))?;
        {
            let mut settings = self.settings.write();
            let mut points = settings.points.take().unwrap_or_default();
            points.retain(|p| p.name != name);
            points.push(NamedPoint {
                name: name.to_string(),
                region,
            });
            settings.points = Some(points);
        }
        self.save_settings()?;
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
        self.backend.sleep_ms(10);
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
            self.backend.sleep_ms(20);
        }
        self.backend.sleep_ms(100);
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
        let settings = self.settings.read().clone();
        let grid = match &settings.map_grid {
            Some(g) => g.clone(),
            None => bail!("Map grid not calibrated — run: little_oil calibrate-map"),
        };
        let frame = self.screenshot()?;
        let (px, py) = grid.cell_center(col, row);
        let (sx, sy) = frame.frame_to_screen(px, py);
        self.click(sx, sy);
        Ok(())
    }

    fn stash_copy(&self) -> anyhow::Result<()> {
        self.focus_game_window()?;

        let settings = self.settings.read().clone();
        let grid = match &settings.stash_grid {
            Some(g) => g.clone(),
            None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
        };
        let frame = self.screenshot()?;

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
                self.backend.sleep_ms(30);
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
            self.backend.sleep_ms(sleep_click);
            self.click(slot.0, slot.1);
            self.backend.sleep_ms(sleep_read);

            self.click_right(scour.0, scour.1);
            self.backend.sleep_ms(sleep_click);
            self.click(slot.0, slot.1);
            self.backend.sleep_ms(sleep_read);
        }

        Ok(())
    }

    /// Three probes across the vertical middle of an inventory slot, at 25%, 50%
    /// and 75% of its width, in frame-pixel space. None if the slot falls outside
    /// the captured frame.
    pub(crate) fn inv_probes(
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

    pub(crate) fn reset_inv_colors(&self) -> anyhow::Result<()> {
        let settings = self.settings.read().clone();
        let inv_region = settings.inv_region.ok_or_else(|| {
            anyhow::anyhow!(
                "Inventory region not calibrated — run: little_oil set-region inventory"
            )
        })?;

        let frame = self.screenshot()?;

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

        self.backend.notify(&format!(
            "Inventory colors calibrated: {} slots x 3 samples",
            samples.len()
        ));
        self.settings.write().inv_samples = Some(samples);
        self.save_settings()?;
        Ok(())
    }

    /// Occupied inventory cells in `frame` — fewer than 2 of 3 probe pixels
    /// matching the calibrated empty-slot sample — as screen coordinates.
    pub(crate) fn occupied_inv_cells(
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
        let settings = self.settings.read().clone();
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
            let frame = self.screenshot()?;
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

            self.backend.key(InputKey::Ctrl, true);
            self.backend.sleep_ms(5);
            for (sx, sy) in &cells {
                clicker(self, *sx, *sy);
                clicked += 1;
                self.backend.sleep_ms(10);
            }
            self.backend.key(InputKey::Ctrl, false);
            // Give the last item's move a beat before the next screenshot.
            self.backend.sleep_ms(30);
        }

        // If the loop ended while cells were still occupied (the 3rd pass
        // clicked something, or the stuck-break), count what actually remains
        // so the notification never claims items were cleared when they weren't.
        if remaining > 0 {
            // Same park before the recount — the cursor is still over the last
            // clicked slot.
            self.park_cursor()?;
            let frame = self.screenshot()?;
            remaining = Self::occupied_inv_cells(&frame, inv_region, expected)?.len() as u32;
        }

        Ok((clicked, remaining))
    }

    pub(crate) fn empty_inv(&self) -> anyhow::Result<()> {
        self.empty_inv_with(App::click_fast)
    }

    pub(crate) fn empty_inv_right(&self) -> anyhow::Result<()> {
        self.empty_inv_with(App::click_right_fast)
    }

    fn empty_inv_with(&self, clicker: fn(&App, i32, i32)) -> anyhow::Result<()> {
        self.focus_game_window()?;

        // A short beat after the focus click; the macro's screenshots drive
        // the actual pacing from here.
        self.backend.sleep_ms(150);
        let (clicked, remaining) = self.empty_inv_macro(clicker)?;
        self.backend.notify(&format!(
            "Inventory cleared: {} clicked, {} remain",
            clicked, remaining
        ));
        Ok(())
    }

    fn sort_quad(&self, times: u32) -> anyhow::Result<()> {
        self.focus_game_window()?;
        self.backend.sleep_ms(300);

        let settings = self.settings.read().clone();
        let delay = settings.pull_delay;
        let grid = match &settings.stash_grid {
            Some(g) => g.clone(),
            None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
        };
        let frame = self.screenshot()?;

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
                    self.backend.sleep_ms(delay);
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
            Some("gui") => {
                let app = Arc::new(self);
                return crate::gui::run(app);
            }
            Some("config") => {
                // Print the ACTIVE config. Calibration is only inspectable this way.
                println!("{}", serde_json::to_string_pretty(&*self.settings.read())?);
                return Ok(());
            }
            Some("version") | Some("--version") => {
                println!("little_oil v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("doctor") => {
                // checkhealth-style diagnostics; exits non-zero on any Error.
                let checks = crate::health::run(&self);
                for c in &checks {
                    let tag = match c.status {
                        crate::health::CheckStatus::Good => "[ OK ]",
                        crate::health::CheckStatus::Warn => "[WARN]",
                        crate::health::CheckStatus::Error => "[ ERR]",
                    };
                    println!("{tag} {:<20} {}", c.name, c.message);
                }
                let errors = checks
                    .iter()
                    .filter(|c| c.status == crate::health::CheckStatus::Error)
                    .count();
                if errors > 0 {
                    bail!("{errors} check(s) failed");
                }
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
                if self.desktop().uses_absolute_pointer() {
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
                let region = match region_name {
                    "inventory" => {
                        let r = platform::select_region(
                            "Select the INVENTORY grid region (top-left slot to bottom-right slot)",
                        )?;
                        self.settings.write().inv_region = Some(r);
                        r
                    }
                    "stash" => {
                        let r = platform::select_region(
                            "Select the STASH grid region (top-left slot to bottom-right slot)",
                        )?;
                        self.settings.write().stash_region = Some(r);
                        r
                    }
                    "window" => {
                        let r = platform::select_region(
                            "Select the GAME WINDOW (drag around the entire PoE window)",
                        )?;
                        self.settings.write().game_window_region = Some(r);
                        r
                    }
                    "map" => {
                        let r = platform::select_region(
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

                self.save_settings()?;
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
                        {
                            let mut s = self.settings.write();
                            match cmd {
                                "pull" => s.pull_delay = x,
                                "div" => s.div_delay = x,
                                _ => unreachable!(),
                            }
                        }
                        println!("{cmd} delay is {x}");
                        if let Err(e) = self.save_settings() {
                            println!("could not save config: {e:#}");
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
    use crate::stash_grid::{MapGrid, StashGrid};
    use crate::test_support::{Ev, blank_frame, fill_rect, synth_inventory, test_app};

    /// Opaque colour of an empty inventory slot in synthetic frames.
    const EMPTY: u32 = 0x101010FF;
    /// Opaque colour of an occupied inventory slot in synthetic frames.
    const ITEM: u32 = 0xFF0000FF;

    /// The synthetic inventory occupies 12x5 slots of 100x100 px.
    fn inv_region() -> ScreenRegion {
        ScreenRegion {
            x: 100,
            y: 200,
            width: 1200,
            height: 500,
        }
    }

    /// Screen centre of inventory cell (`col`, `row`): the middle probe point,
    /// which is where the empty macro clicks.
    fn cell_center(col: u32, row: u32) -> (i32, i32) {
        let r = inv_region();
        (
            (r.x + col * r.width / 12 + r.width / 24) as i32,
            (r.y + row * r.height / 5 + r.height / 10) as i32,
        )
    }

    /// Settings for the inventory macros: calibrated regions, every slot
    /// empty, and no focus clicks so click assertions see only macro clicks.
    fn inventory_settings() -> Settings {
        let mut s = crate::default_settings();
        s.focus_clicks = 0;
        s.game_window_region = Some(inv_region());
        s.inv_region = Some(inv_region());
        s.inv_samples = Some(vec![[EMPTY; 3]; 60]);
        s
    }

    /// A uniform `step`-pixel grid whose highlighted cells are painted with
    /// `highlight` in the synthetic frames.
    fn grid<const C: usize, const R: usize>(step: u32, highlight: u32) -> CellGrid<C, R> {
        CellGrid {
            cols: std::array::from_fn(|i| i as u32 * step),
            rows: std::array::from_fn(|i| i as u32 * step),
            cell_w: step,
            cell_h: step,
            highlight_color: highlight,
        }
    }

    /// Paint a cell's bottom border — the row `is_highlighted` probes.
    fn highlight_cell(frame: &mut ScreenshotData, grid: &StashGrid, col: usize, row: usize) {
        let y = grid.rows[row] + grid.cell_h - 2;
        fill_rect(
            frame,
            grid.cols[col],
            y,
            grid.cell_w,
            2,
            grid.highlight_color,
        );
    }

    /// Settings for the stash/map macros: a 300x300 window at (100, 200) with
    /// a 10-px grid, no focus clicks.
    fn grid_settings() -> (Settings, ScreenRegion, StashGrid) {
        let region = ScreenRegion {
            x: 100,
            y: 200,
            width: 300,
            height: 300,
        };
        let grid: StashGrid = grid(10, 0xFF00FFFF);
        let mut s = crate::default_settings();
        s.focus_clicks = 0;
        s.game_window_region = Some(region);
        s.stash_grid = Some(grid.clone());
        (s, region, grid)
    }

    /// Screen positions of every press of `button`, taken from the `Move` that
    /// immediately precedes it.
    fn pressed_at(events: &[Ev], button: InputButton) -> Vec<(i32, i32)> {
        let mut last_move = None;
        let mut out = Vec::new();
        for ev in events {
            match ev {
                Ev::Move(x, y) => last_move = Some((*x, *y)),
                Ev::Button(b, true) if *b == button => {
                    out.push(last_move.expect("every press follows a move"));
                }
                _ => {}
            }
        }
        out
    }

    fn count_key(events: &[Ev], key: InputKey, pressed: bool) -> usize {
        events
            .iter()
            .filter(|e| **e == Ev::Key(key, pressed))
            .count()
    }

    fn count_sleeps(events: &[Ev]) -> usize {
        events.iter().filter(|e| matches!(e, Ev::Sleep(_))).count()
    }

    #[test]
    fn empty_inv_clicks_every_occupied_cell_then_stops_on_an_empty_pass() {
        let (app, fake) = test_app(inventory_settings());
        let occupied = [(0, 0), (2, 3), (11, 4)];
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &occupied, ITEM));
        // Pass 2 sees nothing occupied, so the macro must stop clicking.
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &[], ITEM));

        app.empty_inv().expect("empty the inventory");

        let events = fake.events();
        assert_eq!(
            fake.capture_count(),
            2,
            "one capture per pass: the empty pass must not click"
        );

        let mut clicked = pressed_at(&events, InputButton::Left);
        clicked.sort();
        let mut expected: Vec<(i32, i32)> =
            occupied.iter().map(|&(c, r)| cell_center(c, r)).collect();
        expected.sort();
        assert_eq!(
            clicked, expected,
            "exactly one click per occupied cell, at its centre"
        );
        assert!(
            pressed_at(&events, InputButton::Right).is_empty(),
            "empty_inv must only press the left button"
        );

        // Ctrl is held across a whole pass: down before the first press, up
        // after the last, and only once.
        assert_eq!(count_key(&events, InputKey::Ctrl, true), 1);
        assert_eq!(count_key(&events, InputKey::Ctrl, false), 1);
        let ctrl_down = events
            .iter()
            .position(|e| *e == Ev::Key(InputKey::Ctrl, true))
            .expect("ctrl down");
        let ctrl_up = events
            .iter()
            .position(|e| *e == Ev::Key(InputKey::Ctrl, false))
            .expect("ctrl up");
        let first_press = events
            .iter()
            .position(|e| *e == Ev::Button(InputButton::Left, true))
            .expect("a click");
        let last_press = events
            .iter()
            .rposition(|e| *e == Ev::Button(InputButton::Left, true))
            .expect("a click");
        assert!(
            ctrl_down < first_press && last_press < ctrl_up,
            "Ctrl must span the pass"
        );

        assert!(
            events.contains(&Ev::Notify(
                "Inventory cleared: 3 clicked, 0 remain".to_string()
            )),
            "notification reports the real totals"
        );
    }

    #[test]
    fn empty_inv_clicks_nothing_when_the_inventory_is_already_empty() {
        let (app, fake) = test_app(inventory_settings());
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &[], ITEM));

        app.empty_inv().expect("empty inventory is a no-op");

        let events = fake.events();
        assert!(
            !events.iter().any(|e| matches!(e, Ev::Button(_, true))),
            "no clicks on an empty inventory: {events:?}"
        );
        assert_eq!(fake.capture_count(), 1, "one pass is enough");
        assert!(events.contains(&Ev::Notify(
            "Inventory cleared: 0 clicked, 0 remain".to_string()
        )));
    }

    #[test]
    fn empty_inv_requires_calibrated_colours() {
        let mut settings = inventory_settings();
        settings.inv_samples = None;
        let (app, _fake) = test_app(settings);

        let err = app
            .empty_inv()
            .expect_err("uncalibrated colours must error");
        assert!(
            format!("{err:#}").contains("reset_inv"),
            "error must point at reset_inv: {err:#}"
        );
    }

    #[test]
    fn empty_inv_requires_an_inventory_region() {
        let mut settings = inventory_settings();
        settings.inv_region = None;
        let (app, _fake) = test_app(settings);

        let err = app.empty_inv().expect_err("missing inv_region must error");
        assert!(
            format!("{err:#}").contains("set-region inventory"),
            "error must keep the setup hint: {err:#}"
        );
    }

    #[test]
    fn empty_inv_right_uses_the_right_button() {
        let (app, fake) = test_app(inventory_settings());
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &[(11, 0)], ITEM));
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &[], ITEM));

        app.empty_inv_right()
            .expect("empty the inventory with right clicks");

        let events = fake.events();
        assert_eq!(
            pressed_at(&events, InputButton::Right),
            vec![cell_center(11, 0)],
            "one right click on the occupied cell"
        );
        assert!(
            pressed_at(&events, InputButton::Left).is_empty(),
            "emptyr must not press the left button"
        );
        assert_eq!(count_key(&events, InputKey::Ctrl, true), 1);
        assert_eq!(count_key(&events, InputKey::Ctrl, false), 1);
    }

    #[test]
    fn reset_inv_colors_samples_every_slot_and_saves_them() {
        let mut settings = inventory_settings();
        settings.inv_samples = None;
        let (app, fake) = test_app(settings);
        let occupied = [(0u32, 0u32), (5, 2)];
        fake.push_frame(synth_inventory(inv_region(), EMPTY, &occupied, ITEM));

        app.reset_inv_colors().expect("calibrate inventory colours");

        let samples = app
            .settings
            .read()
            .inv_samples
            .clone()
            .expect("samples stored on the App");
        assert_eq!(samples.len(), 60, "12x5 slots");
        for col in 0..12u32 {
            for row in 0..5u32 {
                let expected = if occupied.contains(&(col, row)) {
                    ITEM
                } else {
                    EMPTY
                };
                assert_eq!(
                    samples[(col * 5 + row) as usize],
                    [expected; 3],
                    "slot ({col}, {row})"
                );
            }
        }

        // Saved to the App's config path (the test tempdir), not the user's.
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&app.config_path).expect("config saved"))
                .expect("config is JSON");
        assert_eq!(
            saved["inv_samples"],
            serde_json::to_value(&samples).unwrap()
        );
        assert!(fake.events().contains(&Ev::Notify(
            "Inventory colors calibrated: 60 slots x 3 samples".to_string()
        )));
    }

    #[test]
    fn sort_quad_clicks_highlighted_cells_in_order_up_to_times() {
        let (settings, region, stash) = grid_settings();
        let (app, fake) = test_app(settings);

        let mut frame = blank_frame(region, 0);
        // Row-major scan order: (0, 0), then (5, 5), then (23, 23). Only the
        // first two fit in `times = 2`.
        for (col, row) in [(0usize, 0usize), (5, 5), (23, 23)] {
            highlight_cell(&mut frame, &stash, col, row);
        }
        fake.push_frame(frame);

        app.sort_quad(2).expect("sort");

        let mut expected = vec![
            Ev::Sleep(100), // focus_game_window's settle
            Ev::Sleep(300), // sort_quad's pre-capture settle
        ];
        for (col, row) in [(0usize, 0usize), (5, 5)] {
            let (x, y) = stash.cell_center(col, row);
            let (sx, sy) = (region.x as i32 + x as i32, region.y as i32 + y as i32);
            expected.extend([
                Ev::Move(sx, sy),
                Ev::Sleep(30),
                Ev::Button(InputButton::Left, true),
                Ev::Sleep(10),
                Ev::Button(InputButton::Left, false),
                Ev::Sleep(50), // pull_delay
            ]);
        }
        assert_eq!(
            fake.events(),
            expected,
            "clicks the first `times` highlighted cells in scan order and stops"
        );
    }

    #[test]
    fn stash_copy_hovers_each_highlighted_cell_and_copies_it() {
        let (settings, region, stash) = grid_settings();
        let (app, fake) = test_app(settings);

        let mut frame = blank_frame(region, 0);
        for (col, row) in [(1usize, 1usize), (2, 1)] {
            highlight_cell(&mut frame, &stash, col, row);
        }
        fake.push_frame(frame);
        fake.push_clipboard(Some("alpha".to_string()));
        fake.push_clipboard(Some("alpha".to_string()));

        app.stash_copy().expect("copy the stash");

        let events = fake.events();
        let moves: Vec<(i32, i32)> = events
            .iter()
            .filter_map(|e| match e {
                Ev::Move(x, y) => Some((*x, *y)),
                _ => None,
            })
            .collect();
        let mut expected = Vec::new();
        for (col, row) in [(1usize, 1usize), (2, 1)] {
            let (x, y) = stash.cell_center(col, row);
            expected.push((region.x as i32 + x as i32, region.y as i32 + y as i32));
        }
        assert_eq!(
            moves, expected,
            "hover each highlighted cell (focus_clicks is 0, so the focus point is not clicked)"
        );

        // One Ctrl+Alt+C per cell, no retries: the clipboard answered at once.
        assert_eq!(count_key(&events, InputKey::C, true), 2);
        assert_eq!(count_key(&events, InputKey::Ctrl, true), 2);
        assert_eq!(count_key(&events, InputKey::Alt, true), 2);
        assert!(
            !events.iter().any(|e| matches!(e, Ev::Button(_, true))),
            "copying never clicks"
        );
    }

    #[test]
    fn click_map_cell_clicks_the_calibrated_cell() {
        let (mut settings, region, _stash) = grid_settings();
        let map: MapGrid = grid(10, 0);
        settings.map_grid = Some(map.clone());
        let (app, fake) = test_app(settings);
        fake.push_frame(blank_frame(region, 0));

        app.click_map_cell(3, 4).expect("click the map cell");

        let (x, y) = map.cell_center(3, 4);
        let expected = vec![
            Ev::Sleep(100),
            Ev::Move(region.x as i32 + x as i32, region.y as i32 + y as i32),
            Ev::Sleep(30),
            Ev::Button(InputButton::Left, true),
            Ev::Sleep(10),
            Ev::Button(InputButton::Left, false),
        ];
        assert_eq!(fake.events(), expected);
    }

    #[test]
    fn chance_alternates_chance_and_scour_nine_times() {
        let mut settings = crate::default_settings();
        let point = |name: &str, x: u32| NamedPoint {
            name: name.to_string(),
            region: ScreenRegion {
                x,
                y: x,
                width: 20,
                height: 20,
            },
        };
        // Centres: chance (20, 20), scour (40, 40), slot (60, 60).
        settings.points = Some(vec![
            point("chance", 10),
            point("scour", 30),
            point("slot", 50),
        ]);
        let (app, fake) = test_app(settings);

        app.chance().expect("chance an item");

        let click = |button: InputButton, at: (i32, i32), settle: u64| {
            vec![
                Ev::Move(at.0, at.1),
                Ev::Sleep(30),
                Ev::Button(button, true),
                Ev::Sleep(10),
                Ev::Button(button, false),
                Ev::Sleep(settle),
            ]
        };
        let mut expected = Vec::new();
        for _ in 1..10 {
            expected.extend(click(InputButton::Right, (20, 20), 30)); // chance
            expected.extend(click(InputButton::Left, (60, 60), 250)); // slot
            expected.extend(click(InputButton::Right, (40, 40), 30)); // scour
            expected.extend(click(InputButton::Left, (60, 60), 250)); // slot
        }
        assert_eq!(
            fake.events(),
            expected,
            "nine chance/scour passes with the configured settle times"
        );
    }

    #[test]
    fn reading_the_item_returns_the_clipboard_text_and_sends_ctrl_alt_c() {
        let (app, fake) = test_app(crate::default_settings());
        fake.push_clipboard(None);
        fake.push_clipboard(None);
        fake.push_clipboard(Some("Rarity: Rare".to_string()));

        assert_eq!(
            app.try_read_item_on_cursor().as_deref(),
            Some("Rarity: Rare"),
            "the third poll sees the game's copy"
        );

        let events = fake.events();
        let keys: Vec<Ev> = events
            .iter()
            .filter(|e| matches!(e, Ev::Key(..)))
            .cloned()
            .collect();
        assert_eq!(
            keys,
            vec![
                Ev::Key(InputKey::Ctrl, true),
                Ev::Key(InputKey::Alt, true),
                Ev::Key(InputKey::C, true),
                Ev::Key(InputKey::C, false),
                Ev::Key(InputKey::Alt, false),
                Ev::Key(InputKey::Ctrl, false),
            ],
            "one Ctrl+Alt+C chord, no retry after the text arrives"
        );
        // Two chord settles plus one 5 ms poll per read.
        assert_eq!(count_sleeps(&events), 5);
    }

    #[test]
    fn reading_the_item_gives_up_when_the_clipboard_stays_empty() {
        let (app, fake) = test_app(crate::default_settings());

        assert_eq!(app.try_read_item_on_cursor(), None);

        let events = fake.events();
        // 6 keys for the first chord plus 5 retries of 6 keys each.
        assert_eq!(count_key(&events, InputKey::C, true), 6);
        assert_eq!(count_key(&events, InputKey::Ctrl, true), 6);
        assert_eq!(count_key(&events, InputKey::Ctrl, false), 6);
        // 2 chord settles + 50 polls, then per retry a stagger, a chord settle
        // and 50 polls.
        assert_eq!(count_sleeps(&events), 2 + 50 + 5 * (2 + 50));
    }

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
