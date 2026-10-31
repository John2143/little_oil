//! Stash macros: copy the tooltip of every highlighted quad-tab cell, and sort
//! a quad tab by clicking its highlighted cells.

use super::App;
use crate::stash_grid::{QUAD_COLS, QUAD_ROWS};
use anyhow::bail;

impl App {
    pub(crate) fn stash_copy(&self) -> anyhow::Result<()> {
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
    pub(crate) fn sort_quad(&self, times: u32) -> anyhow::Result<()> {
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
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::{count_key, grid_settings, highlight_cell};
    use crate::platform::{InputButton, InputKey};
    use crate::test_support::{Ev, blank_frame, test_app};

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
}
