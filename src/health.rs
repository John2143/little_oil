//! `little_oil doctor` and the GUI Health tab share these checks.
//!
//! Every check is read-only: no clicks, no injection, no config writes — safe
//! to run while the game is open. The CLI prints the checks with `[ OK ]` /
//! `[WARN]` / `[ ERR]` tags and exits non-zero on any Error; the GUI Health
//! tab renders the same list.
use crate::ScreenRegion;
use crate::app::App;
use crate::platform::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Good,
    Warn,
    Error,
}

pub struct Check {
    pub name: &'static str,
    pub status: CheckStatus,
    pub message: String,
}

fn ok(name: &'static str, message: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Good,
        message: message.into(),
    }
}

fn warn(name: &'static str, message: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Warn,
        message: message.into(),
    }
}

fn err(name: &'static str, message: impl Into<String>) -> Check {
    Check {
        name,
        status: CheckStatus::Error,
        message: message.into(),
    }
}

/// True when `region` is fully inside the rect [ox, ox+w) x [oy, oy+h).
/// i64 arithmetic so overflowing i32 sums (pathological regions) compare
/// correctly instead of wrapping.
pub fn region_inside(region: ScreenRegion, ox: i32, oy: i32, w: u32, h: u32) -> bool {
    let x0 = region.x as i64;
    let y0 = region.y as i64;
    let x1 = x0 + region.width as i64;
    let y1 = y0 + region.height as i64;
    x0 >= ox as i64 && y0 >= oy as i64 && x1 <= ox as i64 + w as i64 && y1 <= oy as i64 + h as i64
}

/// Run every health check in order. Single source of truth for both the CLI
/// `doctor` command and the GUI Health tab.
pub fn run(app: &App) -> Vec<Check> {
    let mut checks = Vec::new();
    let s = app.settings.read();
    let platform = app.platform();

    // 1. Platform.
    checks.push(match platform {
        Platform::Wayland => ok("platform", "wayland"),
        Platform::Windows => ok("platform", "windows"),
        Platform::X11 => err(
            "platform",
            "X11 is only a compile-time shim — set \"platform\": \"wayland\" in config, \
             or run on Windows",
        ),
    });

    // 2. Config file readable and writable.
    match crate::config_path() {
        Err(e) => checks.push(err("config", format!("{e:#}"))),
        Ok(path) => match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Err(e) => checks.push(err(
                "config",
                format!("cannot open {}: {e}", path.display()),
            )),
            Ok(_) => checks.push(ok("config", format!("{}", path.display()))),
        },
    }

    // 3. Desktop capture — also feeds the region-bounds check.
    let capture = platform.capture_desktop();
    let bounds: Option<(i32, i32, u32, u32)> = match &capture {
        Err(e) => {
            checks.push(err("desktop-capture", format!("{e:#}")));
            None
        }
        Ok(d) if d.width == 0 || d.height == 0 => {
            checks.push(err("desktop-capture", "capture returned an empty image"));
            None
        }
        Ok(d) => {
            checks.push(ok(
                "desktop-capture",
                format!(
                    "{}x{} (origin {}, {})",
                    d.width, d.height, d.origin.0, d.origin.1
                ),
            ));
            Some((d.origin.0, d.origin.1, d.width as u32, d.height as u32))
        }
    };

    // 4. Game window on screen?
    match platform.find_game_window() {
        Some(r) => checks.push(ok(
            "game-window",
            format!("found at {}x{} @ ({}, {})", r.width, r.height, r.x, r.y),
        )),
        None => match s.game_window_region {
            Some(r) => checks.push(warn(
                "game-window",
                format!(
                    "PoE window not on screen (game closed?) — saved region {}x{} @ ({}, {})",
                    r.width, r.height, r.x, r.y
                ),
            )),
            None => checks.push(err(
                "game-window",
                "Path of Exile window not found — launch the game first \
                 (window detection uses hyprctl; unsupported on other compositors)",
            )),
        },
    }

    // 5. Regions configured?
    for (name, region, required, missing_msg) in [
        (
            "region-game",
            s.game_window_region,
            true,
            "not set — run the Setup wizard",
        ),
        (
            "region-inv",
            s.inv_region,
            true,
            "not set — run the Setup wizard",
        ),
        (
            "region-stash",
            s.stash_region,
            false,
            "not set — only needed for stash features",
        ),
        (
            "region-map",
            s.map_region,
            false,
            "not set — only needed for map features",
        ),
    ] {
        match region {
            Some(r) => checks.push(ok(
                name,
                format!("{}x{} @ ({}, {})", r.width, r.height, r.x, r.y),
            )),
            None if required => checks.push(err(name, missing_msg)),
            None => checks.push(warn(name, missing_msg)),
        }
    }

    // 6. Regions inside the visible desktop (and inventory inside the game).
    if let Some((ox, oy, w, h)) = bounds {
        let mut all_inside = true;
        for (label, region) in [
            ("game window", s.game_window_region),
            ("inventory", s.inv_region),
            ("stash", s.stash_region),
            ("map", s.map_region),
        ] {
            if let Some(r) = region
                && !region_inside(r, ox, oy, w, h)
            {
                all_inside = false;
                checks.push(err(
                    "region-bounds",
                    format!(
                        "{label} region {}x{} @ ({}, {}) lies outside the visible \
                         desktop — monitors changed? re-run Setup",
                        r.width, r.height, r.x, r.y
                    ),
                ));
            }
        }
        if let (Some(inv), Some(game)) = (s.inv_region, s.game_window_region)
            && !region_inside(inv, game.x as i32, game.y as i32, game.width, game.height)
        {
            all_inside = false;
            checks.push(warn(
                "region-bounds",
                "inventory region does not lie inside the game window region — \
                 re-drag it in Setup",
            ));
        }
        if all_inside {
            checks.push(ok(
                "region-bounds",
                "all regions inside the visible desktop",
            ));
        }
    }

    // 7. Inventory color samples.
    match &s.inv_samples {
        Some(v) if v.len() == 60 => checks.push(ok("inv-samples", "60 slots sampled")),
        Some(v) => checks.push(err(
            "inv-samples",
            format!("expected 60 samples, found {}", v.len()),
        )),
        None => checks.push(err(
            "inv-samples",
            "inventory colors not sampled — run the Setup wizard step 3 or \
             Recalibrate on the Actions tab",
        )),
    }

    // 8. Focus clicks.
    let v = s.focus_clicks;
    checks.push(if (1..=5).contains(&v) {
        ok("focus-clicks", format!("{v} clicks per focus"))
    } else if v == 0 {
        err(
            "focus-clicks",
            "focus_clicks=0 means macros never focus the game window — set it to 2",
        )
    } else {
        warn("focus-clicks", format!("unusual value {v}"))
    });

    // 9. Pointer scale — irrelevant when the platform positions absolutely.
    if !platform.uses_absolute_pointer() {
        checks.push(match s.pointer_scale {
            Some(sc) if (0.5..=3.0).contains(&sc) => ok("pointer-scale", format!("{sc:.2}")),
            Some(sc) => warn("pointer-scale", format!("unusual value {sc}")),
            None => warn(
                "pointer-scale",
                "not set — clicks may land short or overshoot; \
                 run Setup step 4 (pointer calibration)",
            ),
        });
    }

    // 10. Linux external prerequisites.
    #[cfg(target_os = "linux")]
    {
        for (name, bin) in [
            ("bin-grim", "grim"),
            ("bin-slurp", "slurp"),
            ("bin-wl-copy", "wl-copy"),
        ] {
            match std::process::Command::new(bin).arg("--version").output() {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    checks.push(err(name, format!("'{bin}' not found in PATH — install it")))
                }
                Err(e) => checks.push(warn(name, format!("{e}"))),
                Ok(_) => checks.push(ok(name, "found")),
            }
        }
        match std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
            Ok(_) => checks.push(ok("uinput", "writable")),
            Err(e) => checks.push(err(
                "uinput",
                format!(
                    "cannot open /dev/uinput for writing — add your user to the \
                     'input' group or set a udev rule ({e})"
                ),
            )),
        }
    }

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: u32, y: u32, w: u32, h: u32) -> ScreenRegion {
        ScreenRegion {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn region_inside_positive() {
        assert!(region_inside(region(10, 10, 100, 100), 0, 0, 2560, 1440));
    }

    #[test]
    fn region_inside_negative_origin() {
        // Second monitor left of the primary: its origin is -1920, 0.
        assert!(region_inside(
            region(0, 100, 100, 100),
            -1920,
            0,
            4480,
            1440
        ));
    }

    #[test]
    fn region_inside_overflows() {
        // x1 = 2600 > 2560 — sticks out of the desktop.
        assert!(!region_inside(region(2500, 10, 100, 100), 0, 0, 2560, 1440));
    }

    #[test]
    fn region_inside_zero_size() {
        // Degenerate region sits at the origin — inside, but useless.
        assert!(region_inside(region(0, 0, 0, 0), 0, 0, 2560, 1440));
    }
}
