//! Wayland backend: grim (wlr-screencopy) screenshots.
use crate::ScreenRegion;
use crate::screenshot::ScreenshotData;
use anyhow::{Context, bail};
use std::io::Cursor;
use std::process::Command;

/// Run grim with a forced 1:1 scale. `geom` is None to capture all outputs
/// composited at origin (0,0). `cursor` includes the pointer.
fn grim_capture(geom: Option<&str>, cursor: bool) -> anyhow::Result<image::DynamicImage> {
    let mut cmd = Command::new("grim");
    // -s 1 pins the output scale. Without it grim defaults to the greatest
    // output scale factor, which returns upscaled pixels on HiDPI setups and
    // breaks the pixel-to-screen mapping.
    cmd.args(["-s", "1", "-t", "ppm"]);
    if cursor {
        cmd.arg("-c");
    }
    if let Some(g) = geom {
        cmd.args(["-g", g]);
    }
    let out = cmd
        .arg("-")
        .output()
        .context("grim failed to start — is it installed?")?;
    if !out.status.success() {
        bail!("grim failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    image::load(Cursor::new(out.stdout), image::ImageFormat::Pnm)
        .context("failed to decode grim output")
}

pub fn capture_region(region: ScreenRegion) -> anyhow::Result<ScreenshotData> {
    let geom = format!(
        "{},{} {}x{}",
        region.x, region.y, region.width, region.height
    );
    let img = grim_capture(Some(&geom), false)?;

    if img.width() != region.width || img.height() != region.height {
        bail!(
            "grim returned {}x{} for a {}x{} region — output scaling is active. \
             Frame pixels would not map 1:1 to screen coordinates.",
            img.width(),
            img.height(),
            region.width,
            region.height
        );
    }

    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
        origin: (region.x as i32, region.y as i32),
    })
}

/// Capture the whole desktop (every output composited at origin 0,0), without
/// the cursor. Used by the GUI calibration preview so any region can be
/// dragged on screen.
#[cfg(target_os = "linux")]
pub fn capture_desktop() -> anyhow::Result<ScreenshotData> {
    let img = grim_capture(None, false)?;
    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
        origin: (0, 0),
    })
}

/// Capture every output, composited, with the cursor drawn. Origin is (0,0).
#[cfg(target_os = "linux")]
pub fn capture_all_with_cursor() -> anyhow::Result<ScreenshotData> {
    let img = grim_capture(None, true)?;
    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
        origin: (0, 0),
    })
}
