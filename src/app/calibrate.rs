//! Calibration: pointer scale, the stash/map grids, named points and the screen
//! regions (through the GUI picker).

use super::{App, prompt_enter};
use crate::screenshot::{Rect, ScreenshotData};
use crate::stash_grid::{CellGrid, MAP_COLS, MAP_ROWS, QUAD_COLS, QUAD_ROWS};
use crate::{NamedPoint, ScreenRegion, Settings};
use anyhow::bail;

impl App {
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
        prompt_enter(self, &format!("Open the {label} tab."))?;
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
                prompt_enter(
                    self,
                    &format!(
                        "Put the 1×1 {noun} in the {where_text} slot. In the search box type a \
                 nonsense string (e.g. \"zzz\") so NOTHING is highlighted."
                    ),
                )?;
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

                prompt_enter(
                    self,
                    &format!(
                        "Now search the {noun}'s exact name so ONLY it is highlighted (keep it in the {where_text} slot)."
                    ),
                )?;
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

    pub(crate) fn calibrate_stash(&self) -> anyhow::Result<()> {
        let snapshot = { self.settings.read().clone() };

        let _game_region = snapshot.game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — open the GUI Setup tab or run: little_oil set-region window")
        })?;
        let stash_region = snapshot.stash_region.ok_or_else(|| {
            anyhow::anyhow!(
                "Stash region not set — open the GUI Setup tab or run: little_oil set-region stash"
            )
        })?;

        self.calibrate_grid::<QUAD_COLS, QUAD_ROWS>(stash_region, "stash", "item", |s, g| {
            s.stash_grid = Some(g)
        })
    }

    pub(crate) fn calibrate_map(&self) -> anyhow::Result<()> {
        let snapshot = { self.settings.read().clone() };

        let _game_region = snapshot.game_window_region.ok_or_else(|| {
            anyhow::anyhow!("Game window region not set — open the GUI Setup tab or run: little_oil set-region window")
        })?;
        let map_region = snapshot.map_region.ok_or_else(|| {
            anyhow::anyhow!(
                "Map region not set — open the GUI Setup tab or run: little_oil set-region map"
            )
        })?;

        self.calibrate_grid::<MAP_COLS, MAP_ROWS>(map_region, "map", "map", |s, g| {
            s.map_grid = Some(g)
        })
    }

    /// Select one box around a named clickable target in the GUI picker and
    /// upsert it into `points`.
    /// Idempotent: re-running overwrites the entry for the same name. Points are
    /// screen-space, so no screenshot or `set-region` prerequisite.
    pub(crate) fn calibrate_point(&self, name: &str) -> anyhow::Result<()> {
        let region = crate::gui::run_region_picker(
            self,
            name,
            &format!("Select a small box around {name} (the click lands at its centre)"),
        )?;
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

    /// Open the GUI region picker for `name`, save the chosen region to the
    /// config and announce it. `name` is one of the `set-region` targets.
    pub(crate) fn set_region(&self, name: &str) -> anyhow::Result<()> {
        let prompt = match name {
            "inventory" => "Select the INVENTORY grid region (top-left slot to bottom-right slot)",
            "stash" => "Select the STASH grid region (top-left slot to bottom-right slot)",
            "window" => "Select the GAME WINDOW (drag around the entire PoE window)",
            "map" => "Select the MAP grid region (the 12x7 map window, not the sub-tab row)",
            other => bail!("unknown region '{other}' — use inventory, stash, window or map"),
        };
        let region = crate::gui::run_region_picker(self, name, prompt)?;
        {
            let mut settings = self.settings.write();
            match name {
                "inventory" => settings.inv_region = Some(region),
                "stash" => settings.stash_region = Some(region),
                "window" => settings.game_window_region = Some(region),
                "map" => settings.map_region = Some(region),
                _ => unreachable!("region name validated above"),
            }
        }
        self.save_settings()?;
        let message = format!(
            "{name} region saved: {}x{} at ({}, {})",
            region.width, region.height, region.x, region.y
        );
        println!("{message}");
        self.backend.notify(&message);
        Ok(())
    }
}
