//! X11 platform module — compile-time shim only.
//! Wayland and Windows are the supported platforms.
//! X11 exists so the code compiles, but all functions bail at runtime.

use crate::screenshot::ScreenshotData;
use crate::ScreenRegion;
use crate::Settings;
use anyhow::bail;

pub fn screenshot(_settings: &Settings) -> anyhow::Result<ScreenshotData> {
    bail!("X11 screenshot not implemented — use wayland or set platform in config")
}

pub fn select_region(_prompt: &str) -> anyhow::Result<ScreenRegion> {
    bail!("X11 region selection not implemented — use wayland or set platform in config")
}
