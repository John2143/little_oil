//! Inventory macros: sample empty-slot colours, detect occupied slots, and
//! empty the inventory with the `empty`/`emptyr` click loops.

use super::App;
use anyhow::bail;
use rand::seq::SliceRandom;
use tracing::{debug, info};

use crate::ScreenRegion;
use crate::platform::InputKey;
use crate::screenshot::ScreenshotData;

impl App {
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
                "Inventory region not calibrated — open the GUI Setup tab or run: little_oil set-region inventory"
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
                "Inventory region not calibrated — open the GUI Setup tab or run: little_oil set-region inventory"
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
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::{
        EMPTY, ITEM, cell_center, count_key, inv_region, inventory_settings, pressed_at,
    };
    use crate::platform::{InputButton, InputKey};
    use crate::test_support::{Ev, synth_inventory, test_app};

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
}
