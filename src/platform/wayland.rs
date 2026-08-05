//! Wayland backend: grim (wlr-screencopy) screenshots and slurp (layer-shell)
//! region selection.
use crate::ScreenRegion;
use crate::Settings;
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

pub fn screenshot(settings: &Settings) -> anyhow::Result<ScreenshotData> {
    let region = settings.game_window_region.ok_or_else(|| {
        anyhow::anyhow!("Game window region not set — run: little_oil set-region window")
    })?;
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
        origin: (region.x, region.y),
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

pub fn select_region(prompt: &str) -> anyhow::Result<ScreenRegion> {
    let _ = Command::new("notify-send")
        .args(["-u", "critical", "Little Oil", prompt])
        .spawn();

    let output = Command::new("slurp")
        .arg("-f")
        .arg("%x %y %w %h")
        .output()
        .context("slurp failed to start — is it installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("slurp selection failed: {stderr}");
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<u32> = text
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    if parts.len() != 4 {
        bail!("slurp output unexpected format: expected 4 ints, got: {text}");
    }

    Ok(ScreenRegion {
        x: parts[0],
        y: parts[1],
        width: parts[2],
        height: parts[3],
    })
}
