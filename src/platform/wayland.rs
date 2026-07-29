use crate::screenshot::ScreenshotData;
use crate::ScreenRegion;
use crate::Settings;
use anyhow::{Context, bail};
use std::io::Cursor;
use std::process::Command;

pub fn screenshot(settings: &Settings) -> anyhow::Result<ScreenshotData> {
    let geom = if let Some(r) = &settings.game_window_region {
        format!("{},{} {}x{}", r.x, r.y, r.width, r.height)
    } else {
        "0,0 2560x1440".to_string()
    };

    let cmd = Command::new("grim")
        .arg("-g")
        .arg(&geom)
        .arg("-t")
        .arg("ppm")
        .arg("-")
        .output()
        .context("grim failed to start — is it installed?")?;

    let stdout = Cursor::new(cmd.stdout);
    let img =
        image::load(stdout, image::ImageFormat::Pnm).context("failed to decode grim output")?;

    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
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
