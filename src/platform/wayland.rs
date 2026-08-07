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

/// Clamp raw window bounds into the non-negative screen-region type used by
/// the config. `None` if the window is entirely off the positive area.
#[cfg(target_os = "linux")]
fn region_from_bounds(x: i32, y: i32, w: i32, h: i32) -> Option<ScreenRegion> {
    if w <= 0 || h <= 0 {
        return None;
    }
    // Intersect with the non-negative quadrant the config can represent.
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w).max(0) as u32;
    let y1 = (y + h).max(0) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(ScreenRegion {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// Bounds of the window under a screen point, via `hyprctl clients`.
/// Compositors without hyprctl return None (the GUI shows a hint).
#[cfg(target_os = "linux")]
pub fn window_under_point(x: i32, y: i32) -> Option<ScreenRegion> {
    let out = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let clients: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for w in clients.as_array()?.iter() {
        let Some(title) = w["title"].as_str() else {
            continue;
        };
        if title.contains("Little Oil") {
            continue; // our own window
        }
        let (Some(at), Some(size)) = (w["at"].as_array(), w["size"].as_array()) else {
            continue;
        };
        let (Some(wx), Some(wy)) = (
            at.first().and_then(|v| v.as_i64()),
            at.get(1).and_then(|v| v.as_i64()),
        ) else {
            continue;
        };
        let (Some(ww), Some(wh)) = (
            size.first().and_then(|v| v.as_i64()),
            size.get(1).and_then(|v| v.as_i64()),
        ) else {
            continue;
        };
        let (wx, wy, ww, wh) = (wx as i32, wy as i32, ww as i32, wh as i32);
        if x >= wx && x < wx + ww && y >= wy && y < wy + wh {
            return region_from_bounds(wx, wy, ww, wh);
        }
    }
    None
}

/// Bounds of the monitor under a screen point, via `hyprctl monitors`.
#[cfg(target_os = "linux")]
pub fn monitor_under_point(x: i32, y: i32) -> Option<ScreenRegion> {
    let out = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let monitors: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for m in monitors.as_array()?.iter() {
        let (Some(mx), Some(my)) = (m["x"].as_i64(), m["y"].as_i64()) else {
            continue;
        };
        let (Some(mw), Some(mh)) = (m["width"].as_i64(), m["height"].as_i64()) else {
            continue;
        };
        let (mx, my, mw, mh) = (mx as i32, my as i32, mw as i32, mh as i32);
        if x >= mx && x < mx + mw && y >= my && y < my + mh {
            return region_from_bounds(mx, my, mw, mh);
        }
    }
    None
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
