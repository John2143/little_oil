//! X11 platform module — compile-time shim only.
//! Wayland and Windows are the supported platforms.
//! X11 exists so the code compiles, but all functions bail at runtime.

use crate::ScreenRegion;
use anyhow::bail;

pub fn select_region(_prompt: &str) -> anyhow::Result<ScreenRegion> {
    bail!("X11 region selection not implemented — use wayland or set platform in config")
}
