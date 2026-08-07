//! Platform abstraction: screenshot capture, region selection, input injection,
//! clipboard I/O — all per display server / OS.
pub(crate) mod input;
#[cfg(target_os = "linux")]
pub(crate) mod virtual_pointer;
pub(crate) mod wayland;
#[cfg(target_os = "windows")]
pub(crate) mod windows;
mod x11;

pub(crate) use input::{Input, InputButton, InputKey};
#[cfg(target_os = "linux")]
use std::io::Read;

use crate::ScreenRegion;
use crate::Settings;
use crate::screenshot::ScreenshotData;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    #[serde(rename = "wayland")]
    Wayland,
    #[serde(rename = "x11")]
    X11,
    #[serde(rename = "windows")]
    Windows,
}

impl Platform {
    /// Auto-detect from environment. Called when config has no platform set.
    /// X11 is never auto-detected — it exists as a compile-time shim only.
    pub fn detect() -> Self {
        if cfg!(windows) {
            return Platform::Windows;
        }
        // Default to Wayland on Linux. X11 users must set platform manually in config.
        Platform::Wayland
    }

    pub fn screenshot(&self, settings: &Settings) -> anyhow::Result<ScreenshotData> {
        match self {
            Platform::Wayland => wayland::screenshot(settings),
            Platform::X11 => x11::screenshot(settings),
            #[cfg(target_os = "windows")]
            Platform::Windows => windows::screenshot(settings),
            #[cfg(not(target_os = "windows"))]
            Platform::Windows => {
                anyhow::bail!("the Windows backend is only built on Windows")
            }
        }
    }

    pub fn select_region(&self, prompt: &str) -> anyhow::Result<ScreenRegion> {
        match self {
            Platform::Wayland => wayland::select_region(prompt),
            Platform::X11 => x11::select_region(prompt),
            #[cfg(target_os = "windows")]
            Platform::Windows => windows::select_region(prompt),
            #[cfg(not(target_os = "windows"))]
            Platform::Windows => {
                anyhow::bail!("the Windows backend is only built on Windows")
            }
        }
    }

    /// Full-screen capture with the cursor drawn. Used by `calibrate-pointer`
    /// (cursor diff detection) on Wayland. On Windows, returns the full desktop.
    pub fn capture_all(&self) -> anyhow::Result<ScreenshotData> {
        #[cfg(target_os = "linux")]
        match self {
            Platform::Wayland => wayland::capture_all_with_cursor(),
            _ => anyhow::bail!("capture_all not implemented for platform {:?}", self),
        }
        #[cfg(target_os = "windows")]
        {
            windows::capture_all()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        anyhow::bail!("capture_all: unsupported platform")
    }

    /// Full virtual-desktop capture without the cursor. Used by the GUI
    /// calibration preview so the whole screen can be seen and dragged over.
    pub fn capture_desktop(&self) -> anyhow::Result<ScreenshotData> {
        #[cfg(target_os = "linux")]
        match self {
            Platform::Wayland => wayland::capture_desktop(),
            _ => anyhow::bail!("capture_desktop not implemented for platform {:?}", self),
        }
        #[cfg(target_os = "windows")]
        {
            windows::capture_desktop()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        anyhow::bail!("capture_desktop: unsupported platform")
    }

    /// Bounds of the window under a screen point. Used by the GUI's
    /// right-click "select whole window" calibration shortcut.
    pub fn window_under_point(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            return wayland::window_under_point(x, y);
        }
        #[cfg(target_os = "windows")]
        if matches!(self, Platform::Windows) {
            return windows::window_under_point(x, y);
        }
        None
    }

    /// Bounds of the monitor under a screen point. Used by the GUI's
    /// right-click "select whole monitor" calibration shortcut.
    pub fn monitor_under_point(&self, x: i32, y: i32) -> Option<ScreenRegion> {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            return wayland::monitor_under_point(x, y);
        }
        #[cfg(target_os = "windows")]
        if matches!(self, Platform::Windows) {
            return windows::monitor_under_point(x, y);
        }
        None
    }

    /// Bounds of the Path of Exile window if it is on screen. Used by the Setup
    /// wizard and health checks. `None` when the game is closed or the platform
    /// cannot enumerate windows.
    pub fn find_game_window(&self) -> Option<ScreenRegion> {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            return wayland::find_game_window();
        }
        #[cfg(target_os = "windows")]
        if matches!(self, Platform::Windows) {
            return windows::find_game_window().and_then(windows::window_rect);
        }
        None
    }

    /// Returns `true` when the platform uses absolute pointer positioning
    /// (no `pointer_scale` needed). On Linux this is niri only (wlr virtual
    /// pointer). On Windows all positioning is absolute.
    pub fn uses_absolute_pointer(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Platform::Wayland => std::process::Command::new("niri")
                .args(["msg", "outputs"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            Platform::Windows => true,
            _ => false,
        }
    }

    /// Clear the system clipboard (equivalent to copying zero bytes).
    pub fn clear_clipboard(&self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            use wl_clipboard_rs::copy::{MimeType, Options, Source, copy};
            let opts = Options::new();
            copy(opts, Source::Bytes([].into()), MimeType::Autodetect)?;
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        {
            windows::clear_clipboard()?;
        }
        Ok(())
    }

    /// Read text from the system clipboard. Returns `None` when the clipboard
    /// is empty or unavailable.
    pub fn read_clipboard_text(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            use wl_clipboard_rs::paste::{ClipboardType, Error, Seat, get_contents};
            match get_contents(
                ClipboardType::Regular,
                Seat::Unspecified,
                wl_clipboard_rs::paste::MimeType::Text,
            ) {
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
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            // Non-Wayland on Linux (e.g. X11) — clipboard not implemented.
            None
        }
        #[cfg(target_os = "windows")]
        {
            windows::read_clipboard_text()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        None
    }

    /// Returns `true` when the Wayland primary selection protocol is available
    /// (required for tooltip reading). On non-Linux platforms always returns
    /// `false` — clipboard polling still works via regular clipboard.
    pub fn primary_selection_available(&self) -> bool {
        #[cfg(target_os = "linux")]
        if matches!(self, Platform::Wayland) {
            wl_clipboard_rs::utils::is_primary_selection_supported().is_ok()
        } else {
            false
        }
        #[cfg(not(target_os = "linux"))]
        false
    }
}
