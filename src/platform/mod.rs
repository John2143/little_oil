//! Platform abstraction: screenshot capture, region selection, input injection,
//! clipboard I/O — all per display server / OS.
//!
//! Everything the commands need is reached through [`Backend`] (see
//! `backend.rs`); what remains here is the legacy `Platform` value used by
//! config and by the region-selection / pointer-mode checks that have not moved
//! to the backend yet.
pub(crate) mod backend;
pub(crate) mod input;
#[cfg(target_os = "linux")]
pub(crate) mod virtual_pointer;
#[cfg(target_os = "linux")]
pub(crate) mod wayland;
#[cfg(target_os = "windows")]
pub(crate) mod windows;
mod x11;

pub(crate) use backend::{Backend, detect_backend};
pub(crate) use input::{Input, InputButton, InputKey};

use crate::ScreenRegion;
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

    pub fn select_region(&self, prompt: &str) -> anyhow::Result<ScreenRegion> {
        match self {
            #[cfg(target_os = "linux")]
            Platform::Wayland => wayland::select_region(prompt),
            #[cfg(not(target_os = "linux"))]
            Platform::Wayland => anyhow::bail!("the Wayland backend is only built on Linux"),
            Platform::X11 => x11::select_region(prompt),
            #[cfg(target_os = "windows")]
            Platform::Windows => windows::select_region(prompt),
            #[cfg(not(target_os = "windows"))]
            Platform::Windows => {
                anyhow::bail!("the Windows backend is only built on Windows")
            }
        }
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
}
