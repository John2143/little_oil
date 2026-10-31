//! The backend seam: every screen capture, pointer/keyboard injection,
//! clipboard and notification operation the app performs goes through
//! [`Backend`].
//!
//! One implementation per desktop: [`WlrBackend`] on Linux (grim screenshots,
//! wlr virtual pointer with the uinput relative fallback) and [`WindowsBackend`]
//! on Windows (WGC window capture, SendInput). `App` holds a `Box<dyn Backend>`,
//! so commands never reach into a platform module directly and tests can swap
//! in a fake (see `test_support::FakeBackend`).

use crate::ScreenRegion;
use crate::platform::{Desktop, Input, InputButton, InputKey};
use crate::screenshot::ScreenshotData;

use crate::Settings;
#[cfg(target_os = "linux")]
use crate::platform::wayland;
use parking_lot::Mutex;
#[cfg(target_os = "linux")]
use parking_lot::RwLock;

/// Everything the automation needs from the desktop.
///
/// Implementations are shared across threads (`App` lives behind an `Arc`), so
/// every method takes `&self` and guards its own state.
pub(crate) trait Backend: Send + Sync {
    /// Capture `region` (screen coordinates) into a frame whose `origin` is the
    /// region's top-left corner.
    fn capture_region(&self, region: ScreenRegion) -> anyhow::Result<ScreenshotData>;

    /// Whole virtual desktop, without the cursor. Used by the GUI preview.
    fn capture_desktop(&self) -> anyhow::Result<ScreenshotData>;

    /// Move the pointer to an absolute screen coordinate.
    fn move_to(&self, x: i32, y: i32);

    /// Press or release a mouse button.
    fn button(&self, b: InputButton, pressed: bool);

    /// Press or release a keyboard key.
    fn key(&self, k: InputKey, pressed: bool);

    /// Clear the system clipboard (equivalent to copying zero bytes).
    fn clear_clipboard(&self) -> anyhow::Result<()>;

    /// Text on the system clipboard. `None` when empty or unavailable.
    fn read_clipboard_text(&self) -> Option<String>;

    /// Bounds of the Path of Exile window if it is on screen.
    fn game_window(&self) -> Option<ScreenRegion>;

    /// Bounds of the window under a screen point.
    fn window_at(&self, x: i32, y: i32) -> Option<ScreenRegion>;

    /// Bounds of the monitor under a screen point.
    fn monitor_at(&self, x: i32, y: i32) -> Option<ScreenRegion>;

    /// Best-effort desktop notification; never fails the calling command.
    fn notify(&self, msg: &str);

    /// Sleep for `ms` milliseconds. Tests record the call instead of waiting.
    fn sleep_ms(&self, ms: u64);

    /// Short backend identifier, e.g. `"windows"`, `"wlr:hyprland"`.
    fn name(&self) -> &'static str;

    /// Update the pointer scale used by relative-move backends. Absolute
    /// backends ignore it.
    fn set_pointer_scale(&self, _scale: f32) {}

    /// Linux only: the wlr backend, for pointer calibration's raw-move and
    /// cursor-included capture primitives. `None` on other backends.
    #[cfg(target_os = "linux")]
    fn as_wlr(&self) -> Option<&WlrBackend> {
        None
    }
}

// ── Linux / Wayland ───────────────────────────────────────────────

/// Wayland backend on wlr-family compositors: grim for capture, the wlr
/// virtual pointer for absolute motion (uinput relative move with
/// `pointer_scale` as fallback), wl-clipboard-rs for clipboard I/O.
#[cfg(target_os = "linux")]
pub(crate) struct WlrBackend {
    input: Mutex<Input>,
    /// Desktop this backend is serving; reported by `name()` and used to pick
    /// the compositor's window-lookup tooling.
    desktop: Desktop,
    /// Relative device units per screen pixel for the uinput fallback path.
    /// Seeded from the config and refreshed by `App::save_settings`.
    pointer_scale: RwLock<f32>,
}

#[cfg(target_os = "linux")]
impl WlrBackend {
    pub(crate) fn new(settings: &Settings, desktop: Desktop) -> anyhow::Result<Self> {
        Ok(Self {
            input: Mutex::new(Input::new()?),
            desktop,
            pointer_scale: RwLock::new(settings.pointer_scale.unwrap_or(1.25)),
        })
    }

    /// Raw relative pointer move in device units, plus the settle beat pointer
    /// calibration needs before the next capture.
    pub(crate) fn move_mouse_raw(&self, dx: i32, dy: i32) {
        self.input.lock().move_mouse_raw(dx, dy);
        self.sleep_ms(10);
    }

    pub(crate) fn pin_cursor_to_origin(&self) {
        self.move_mouse_raw(-50000, -50000);
    }

    /// Full-desktop capture with the cursor drawn. Used by pointer calibration
    /// to diff the two cursor positions; nowhere else.
    pub(crate) fn capture_all(&self) -> anyhow::Result<ScreenshotData> {
        wayland::capture_all_with_cursor()
    }

    /// True when the Wayland primary selection protocol is available (required
    /// for tooltip reading).
    pub(crate) fn primary_selection_available(&self) -> bool {
        wl_clipboard_rs::utils::is_primary_selection_supported().is_ok()
    }
}

#[cfg(target_os = "linux")]
impl Backend for WlrBackend {
    fn capture_region(&self, region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
        wayland::capture_region(region)
    }

    fn capture_desktop(&self) -> anyhow::Result<ScreenshotData> {
        wayland::capture_desktop()
    }

    fn move_to(&self, x: i32, y: i32) {
        let scale = *self.pointer_scale.read();
        self.input.lock().move_mouse(x, y, scale);
    }

    fn button(&self, b: InputButton, pressed: bool) {
        self.input.lock().button(b, pressed);
    }

    fn key(&self, k: InputKey, pressed: bool) {
        self.input.lock().key(k, pressed);
    }

    fn clear_clipboard(&self) -> anyhow::Result<()> {
        use wl_clipboard_rs::copy::{MimeType, Options, Source, copy};
        let opts = Options::new();
        copy(opts, Source::Bytes([].into()), MimeType::Autodetect)?;
        Ok(())
    }

    fn read_clipboard_text(&self) -> Option<String> {
        use std::io::Read;
        use wl_clipboard_rs::paste::{ClipboardType, Error, MimeType, Seat, get_contents};
        match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
            Ok((mut pipe, _)) => {
                let mut contents = vec![];
                if pipe.read_to_end(&mut contents).is_ok() {
                    let s = String::from_utf8_lossy(&contents).to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
            Err(Error::ClipboardEmpty) => {}
            Err(Error::NoSeats) => {}
            Err(Error::NoMimeType) => {}
            Err(e) => tracing::debug!("clipboard error: {:?}", e),
        }
        None
    }

    fn game_window(&self) -> Option<ScreenRegion> {
        wayland::find_game_window()
    }

    fn window_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        wayland::window_under_point(x, y)
    }

    fn monitor_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        wayland::monitor_under_point(x, y)
    }

    fn notify(&self, msg: &str) {
        let note = std::process::Command::new("notify-send")
            .args(["-u", "low", "Little Oil", msg])
            .spawn();
        if let Err(e) = note {
            tracing::error!(?e, "notify-send failed");
        }
    }

    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn name(&self) -> &'static str {
        match self.desktop {
            Desktop::Hyprland => "wlr:hyprland",
            Desktop::Niri => "wlr:niri",
            Desktop::Sway => "wlr:sway",
            Desktop::Kde => "wlr:kde",
            Desktop::Gnome => "wlr:gnome",
            Desktop::OtherWayland => "wlr:other",
            // Unreachable: `detect_backend` refuses the Windows override on
            // Linux.
            Desktop::Windows => "wlr",
        }
    }

    fn set_pointer_scale(&self, scale: f32) {
        *self.pointer_scale.write() = scale;
    }

    fn as_wlr(&self) -> Option<&WlrBackend> {
        Some(self)
    }
}

// ── Windows ───────────────────────────────────────────────────────

/// Windows backend: WGC window capture cropped to the requested region, GDI
/// desktop capture, SendInput injection and the win32 clipboard.
#[cfg(target_os = "windows")]
pub(crate) struct WindowsBackend {
    input: Mutex<Input>,
}

#[cfg(target_os = "windows")]
impl WindowsBackend {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            input: Mutex::new(Input::new()?),
        })
    }
}

#[cfg(target_os = "windows")]
impl Backend for WindowsBackend {
    fn capture_region(&self, region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
        // WGC always captures the whole game window; crop it to the configured
        // region (the identity case is today's behavior).
        let frame = crate::platform::windows::capture_window()?;
        crop(&frame, region)
    }

    fn capture_desktop(&self) -> anyhow::Result<ScreenshotData> {
        crate::platform::windows::capture_desktop()
    }

    fn move_to(&self, x: i32, y: i32) {
        // Absolute positioning: no pointer_scale needed.
        self.input.lock().move_mouse(x, y, 1.0);
    }

    fn button(&self, b: InputButton, pressed: bool) {
        self.input.lock().button(b, pressed);
    }

    fn key(&self, k: InputKey, pressed: bool) {
        self.input.lock().key(k, pressed);
    }

    fn clear_clipboard(&self) -> anyhow::Result<()> {
        crate::platform::windows::clear_clipboard()
    }

    fn read_clipboard_text(&self) -> Option<String> {
        crate::platform::windows::read_clipboard_text()
    }

    fn game_window(&self) -> Option<ScreenRegion> {
        crate::platform::windows::find_game_window().and_then(crate::platform::windows::window_rect)
    }

    fn window_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        crate::platform::windows::window_under_point(x, y)
    }

    fn monitor_at(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        crate::platform::windows::monitor_under_point(x, y)
    }

    fn notify(&self, msg: &str) {
        // Native notifications land with the action registry pass; tracing
        // keeps the backlog visible until then.
        tracing::info!(notify = msg, "notification");
    }

    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn name(&self) -> &'static str {
        "windows"
    }
}

/// Crop `frame` (screen coordinates in `origin`) to `region`, which must start
/// inside the frame. Rows are copied with `frame`'s stride.
#[cfg(target_os = "windows")]
fn crop(frame: &ScreenshotData, region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
    let dx = region.x as i64 - frame.origin.0 as i64;
    let dy = region.y as i64 - frame.origin.1 as i64;
    if dx < 0 || dy < 0 || dx as usize >= frame.width || dy as usize >= frame.height {
        anyhow::bail!(
            "Game window region starts outside the game window — run: little_oil set-region window"
        );
    }
    let (dx, dy) = (dx as usize, dy as usize);
    let w = (region.width as usize).min(frame.width - dx);
    let h = (region.height as usize).min(frame.height - dy);
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let start = ((dy + y) * frame.width + dx) * 4;
        pixels.extend_from_slice(&frame.pixels[start..start + w * 4]);
    }
    Ok(ScreenshotData {
        width: w,
        height: h,
        pixels,
        origin: (region.x as i32, region.y as i32),
    })
}

// ── construction ──────────────────────────────────────────────────

/// Build the backend for this OS and desktop.
///
/// The config's `desktop` key overrides auto-detection. On Linux every desktop
/// currently gets the wlr backend; the KDE/GNOME portal backend lands in a
/// later step.
pub(crate) fn detect_backend(settings: &Settings) -> anyhow::Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        let desktop = settings.desktop.unwrap_or_else(super::detect_desktop);
        if desktop == Desktop::Windows {
            anyhow::bail!(
                "desktop \"windows\" is not supported on Linux — remove \"desktop\" from the config"
            );
        }
        Ok(Box::new(WlrBackend::new(settings, desktop)?))
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(desktop) = settings.desktop {
            if desktop != Desktop::Windows {
                anyhow::bail!(
                    "desktop \"{}\" is not supported on Windows — remove \"desktop\" from the config",
                    desktop.name()
                );
            }
        }
        Ok(Box::new(WindowsBackend::new()?))
    }
}
