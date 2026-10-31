//! The injected `App` context and the primitives every command shares:
//! pointer/keyboard device ops, screenshots, focus clicks, clipboard reads and
//! config saving.
//!
//! Commands live in their feature module: [`inventory`] (empty, reset_inv),
//! [`stash`] (sort, stash-copy), [`map`] (map clicks), [`roll`] (chance and the
//! roll log), [`calibrate`] (regions, grids, points) and [`repl`] (the stdin
//! REPL). `gui`, `cli`, `health` and `actions` drive the same `App` methods.

use crate::platform::{self, Backend, Desktop, InputButton, InputKey};
use parking_lot::RwLock;
use std::io;
use std::path::PathBuf;

use crate::screenshot::ScreenshotData;
use crate::{Settings, save_config};

mod calibrate;
mod inventory;
mod map;
mod repl;
mod roll;
mod stash;
#[cfg(test)]
mod test_helpers;

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
            anyhow::anyhow!("Game window region not set — open the GUI Setup tab or run: little_oil set-region window")
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
                        "No region to click for focus — open the GUI Setup tab or run: little_oil set-region inventory (or set-region window)"
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
    pub(crate) fn click_point(&self, name: &str) -> anyhow::Result<()> {
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
}

/// Print a prompt, mirror it to a desktop notification, and block until Enter.
pub(crate) fn prompt_enter(app: &App, msg: &str) -> anyhow::Result<()> {
    println!("\n>>> {msg}\n    Press Enter when ready...");
    app.backend.notify(msg);
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_helpers::{count_key, count_sleeps};
    use crate::test_support::{Ev, test_app};

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
}
