pub(crate) mod wayland;
mod x11;
mod windows;

// No pub use for inner modules — they export free functions, not types.

use crate::screenshot::ScreenshotData;
use crate::ScreenRegion;
use crate::Settings;
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
            Platform::Windows => windows::screenshot(settings),
        }
    }

    pub fn select_region(&self, prompt: &str) -> anyhow::Result<ScreenRegion> {
        match self {
            Platform::Wayland => wayland::select_region(prompt),
            Platform::X11 => x11::select_region(prompt),
            Platform::Windows => windows::select_region(prompt),
        }
    }
}
