//! Test-only helpers shared by in-crate tests. Compiled only under `cargo test`.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::ScreenRegion;
use crate::Settings;
use crate::app::App;
use crate::platform::{Backend, InputButton, InputKey};
use crate::screenshot::ScreenshotData;

/// Every `.txt` file under `tests/example_items/`, recursively, sorted.
/// Directories named `broken` are skipped: known-unparseable pastes live there
/// so they do not fail the corpus.
pub(crate) fn example_item_files() -> Vec<PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("example_items");
    let mut out = Vec::new();
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() == "broken" {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "txt") {
                out.push(path);
            }
        }
    }
    walk(&root, &mut out);
    out
}

/// One recorded backend call, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ev {
    Move(i32, i32),
    Button(InputButton, bool),
    Key(InputKey, bool),
    Sleep(u64),
    Notify(String),
}

/// A backend that records calls and replays queued frames and clipboard values.
///
/// * `frames` — popped by every `capture_*`; the last queued frame repeats, so
///   a test can queue one frame and let the macro re-capture it.
/// * `clipboard` — popped by every `read_clipboard_text`; an empty queue reads
///   as `None` (empty clipboard).
/// * `sleep_ms` records instead of sleeping, so macros run instantly.
#[derive(Default)]
pub(crate) struct FakeBackend {
    frames: Mutex<VecDeque<ScreenshotData>>,
    clipboard: Mutex<VecDeque<Option<String>>>,
    events: Mutex<Vec<Ev>>,
}

impl FakeBackend {
    /// Queue a frame returned by the next `capture_*` call.
    pub(crate) fn push_frame(&self, frame: ScreenshotData) {
        self.frames.lock().push_back(frame);
    }

    /// Queue a clipboard read: `None` for "empty", `Some(text)` for text.
    pub(crate) fn push_clipboard(&self, value: Option<String>) {
        self.clipboard.lock().push_back(value);
    }

    /// Every recorded call, oldest first.
    pub(crate) fn events(&self) -> Vec<Ev> {
        self.events.lock().clone()
    }

    fn record(&self, ev: Ev) {
        self.events.lock().push(ev);
    }

    fn next_frame(&self) -> anyhow::Result<ScreenshotData> {
        let mut frames = self.frames.lock();
        match frames.len() {
            0 => anyhow::bail!("FakeBackend has no frame queued"),
            1 => Ok(frames[0].clone()),
            _ => Ok(frames.pop_front().expect("len > 1")),
        }
    }
}

impl Backend for FakeBackend {
    fn capture_region(&self, _region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
        self.next_frame()
    }

    fn capture_desktop(&self) -> anyhow::Result<ScreenshotData> {
        self.next_frame()
    }

    fn move_to(&self, x: i32, y: i32) {
        self.record(Ev::Move(x, y));
    }

    fn button(&self, b: InputButton, pressed: bool) {
        self.record(Ev::Button(b, pressed));
    }

    fn key(&self, k: InputKey, pressed: bool) {
        self.record(Ev::Key(k, pressed));
    }

    fn clear_clipboard(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn read_clipboard_text(&self) -> Option<String> {
        self.clipboard.lock().pop_front().unwrap_or_default()
    }

    fn game_window(&self) -> Option<ScreenRegion> {
        None
    }

    fn window_at(&self, _x: i32, _y: i32) -> Option<ScreenRegion> {
        None
    }

    fn monitor_at(&self, _x: i32, _y: i32) -> Option<ScreenRegion> {
        None
    }

    fn notify(&self, msg: &str) {
        self.record(Ev::Notify(msg.to_string()));
    }

    fn sleep_ms(&self, ms: u64) {
        self.record(Ev::Sleep(ms));
    }

    fn name(&self) -> &'static str {
        "fake"
    }
}

impl Backend for Arc<FakeBackend> {
    fn capture_region(&self, region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
        (**self).capture_region(region)
    }

    fn capture_desktop(&self) -> anyhow::Result<ScreenshotData> {
        (**self).capture_desktop()
    }

    fn move_to(&self, x: i32, y: i32) {
        (**self).move_to(x, y);
    }

    fn button(&self, b: InputButton, pressed: bool) {
        (**self).button(b, pressed);
    }

    fn key(&self, k: InputKey, pressed: bool) {
        (**self).key(k, pressed);
    }

    fn clear_clipboard(&self) -> anyhow::Result<()> {
        (**self).clear_clipboard()
    }

    fn read_clipboard_text(&self) -> Option<String> {
        (**self).read_clipboard_text()
    }

    fn game_window(&self) -> Option<ScreenRegion> {
        (**self).game_window()
    }

    fn window_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        (**self).window_at(x, y)
    }

    fn monitor_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        (**self).monitor_at(x, y)
    }

    fn notify(&self, msg: &str) {
        (**self).notify(msg);
    }

    fn sleep_ms(&self, ms: u64) {
        (**self).sleep_ms(ms);
    }

    fn name(&self) -> &'static str {
        (**self).name()
    }
}

/// `App` wired to a [`FakeBackend`] and a throwaway config file, so a test can
/// drive macros without a desktop and without touching the real config.
///
/// The temp directory is kept (not deleted on drop): the `App` keeps pointing
/// at it for as long as the test's fixtures do.
pub(crate) fn test_app(settings: Settings) -> (App, Arc<FakeBackend>) {
    let fake = Arc::new(FakeBackend::default());
    let config_path = tempfile::tempdir()
        .expect("temp config dir")
        .keep()
        .join("config.json");
    let backend: Box<dyn Backend> = Box::new(Arc::clone(&fake));
    (App::new(settings, backend, config_path), fake)
}

/// A solid 12×5 inventory frame covering `region`: every slot painted
/// `slot_color`, every listed `(col, row)` cell painted `item_color`.
///
/// Colors use the layout `ScreenshotData::get_pixel` returns, `0xRRGGBBAA`
/// (e.g. `0xFF0000FF` is opaque red).
pub(crate) fn synth_inventory(
    region: ScreenRegion,
    slot_color: u32,
    occupied: &[(u32, u32)],
    item_color: u32,
) -> ScreenshotData {
    let w = region.width as usize;
    let h = region.height as usize;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for _ in 0..w * h {
        pixels.extend_from_slice(&rgba(slot_color));
    }

    // Slot size uses the same integer division as `App::inv_probes`, so every
    // probe pixel of a painted cell lands on `item_color`.
    let dx = w / 12;
    let dy = h / 5;
    for &(col, row) in occupied {
        if col >= 12 || row >= 5 {
            continue;
        }
        let x0 = col as usize * dx;
        let y0 = row as usize * dy;
        for y in y0..(y0 + dy).min(h) {
            for x in x0..(x0 + dx).min(w) {
                let i = (y * w + x) * 4;
                pixels[i..i + 4].copy_from_slice(&rgba(item_color));
            }
        }
    }

    ScreenshotData {
        width: w,
        height: h,
        pixels,
        origin: (region.x as i32, region.y as i32),
    }
}

/// `0xRRGGBBAA` → the four RGBA bytes `ScreenshotData` stores.
fn rgba(color: u32) -> [u8; 4] {
    [
        (color >> 24) as u8,
        (color >> 16) as u8,
        (color >> 8) as u8,
        color as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_replays_frames_and_clipboard_through_the_fake_backend() {
        let region = ScreenRegion {
            x: 100,
            y: 200,
            width: 1200,
            height: 500,
        };
        let mut settings = crate::default_settings();
        settings.game_window_region = Some(region);
        settings.inv_region = Some(region);

        let (app, fake) = test_app(settings);
        assert_eq!(app.backend().name(), "fake");
        assert!(fake.events().is_empty(), "no backend calls before work");
        assert!(
            app.config_path.starts_with(std::env::temp_dir()),
            "config must not point at the user's real config: {:?}",
            app.config_path
        );
        app.save_settings().expect("save to the temp config");
        assert!(app.config_path.is_file(), "settings written beside the App");

        // A frame painted with two occupied cells: the probes of cell (0, 4)
        // must read the item color, an untouched cell the empty-slot color.
        let item = 0xFF0000FF;
        let empty = 0x101010FF;
        fake.push_frame(synth_inventory(region, empty, &[(0, 4), (2, 1)], item));

        let frame = app.screenshot().expect("queued frame");
        assert_eq!((frame.width, frame.height), (1200, 500));
        assert_eq!(frame.origin, (100, 200));
        assert_eq!(frame.get_pixel(50, 450), item, "cell (0, 4) probes");
        assert_eq!(frame.get_pixel(250, 150), item, "cell (2, 1) probes");
        assert_eq!(frame.get_pixel(50, 50), empty, "cell (0, 0) is empty");

        // The last queued frame repeats when the macro re-captures.
        let again = app.screenshot().expect("last frame repeats");
        assert_eq!(again.pixels, frame.pixels);

        // Clipboard reads pop the queue and then read as empty.
        fake.push_clipboard(None);
        fake.push_clipboard(Some("Rarity: Rare".to_string()));
        assert_eq!(app.backend().read_clipboard_text(), None);
        assert_eq!(
            app.backend().read_clipboard_text(),
            Some("Rarity: Rare".to_string())
        );
        assert_eq!(
            app.backend().read_clipboard_text(),
            None,
            "drained queue reads as an empty clipboard"
        );

        // Without a game-window region the capture must keep the setup hint.
        let (fresh, _fake) = test_app(crate::default_settings());
        let err = match fresh.screenshot() {
            Ok(_) => panic!("screenshot without a game window region must fail"),
            Err(e) => e,
        };
        assert!(
            format!("{err:#}").contains("set-region window"),
            "got: {err:#}"
        );
    }
}
