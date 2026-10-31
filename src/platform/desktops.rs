//! Window and monitor geometry lookup for each Linux desktop.
//!
//! Each compositor is queried through its own CLI (`hyprctl`, `swaymsg`,
//! `niri msg`). Every lookup is split into a pure parser (`*_in`, taking the
//! tool's JSON output) and a thin command wrapper, so the parsers are unit
//! tested against the fixtures in `tests/fixtures/desktops/`.
//!
//! KDE and GNOME give unprivileged clients no window geometry at all; their
//! lookups return `None` and the GUI asks the user to drag the region instead.
//! That is also why niri needs the workspace list: its `Window` payload has no
//! output field, so the window's workspace maps to an output whose logical
//! origin is added to `layout.tile_pos_in_workspace_view`.

use crate::ScreenRegion;
use crate::platform::Desktop;
use std::collections::HashMap;
use std::process::Command;

/// Bounds of the Path of Exile window, if this desktop can tell us.
pub(crate) fn game_window(desktop: Desktop) -> Option<ScreenRegion> {
    match desktop {
        Desktop::Hyprland => hyprland::game_window(),
        Desktop::Sway => sway::game_window(),
        Desktop::Niri => niri::game_window(),
        _ => None,
    }
}

/// Bounds of the window under a screen point, if this desktop can tell us.
pub(crate) fn window_at(desktop: Desktop, x: i32, y: i32) -> Option<ScreenRegion> {
    match desktop {
        Desktop::Hyprland => hyprland::window_at(x, y),
        Desktop::Sway => sway::window_at(x, y),
        Desktop::Niri => niri::window_at(x, y),
        _ => None,
    }
}

/// Bounds of the monitor under a screen point, if this desktop can tell us.
pub(crate) fn monitor_at(desktop: Desktop, x: i32, y: i32) -> Option<ScreenRegion> {
    match desktop {
        Desktop::Hyprland => hyprland::monitor_at(x, y),
        Desktop::Sway => sway::monitor_at(x, y),
        Desktop::Niri => niri::monitor_at(x, y),
        _ => None,
    }
}

/// Run `program` with `args` and return stdout. Failures are not errors for
/// the callers (every lookup is best-effort), so they are only traced.
fn run(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    match Command::new(program).args(args).output() {
        Ok(out) if out.status.success() => Some(out.stdout),
        Ok(out) => {
            tracing::debug!(
                program,
                stderr = %String::from_utf8_lossy(&out.stderr),
                "geometry lookup failed"
            );
            None
        }
        Err(e) => {
            tracing::debug!(program, error = %e, "geometry lookup could not start");
            None
        }
    }
}

/// Clamp raw window bounds into the non-negative screen-region type used by
/// the config. `None` if the window is entirely off the positive area.
fn region_from_bounds(x: i32, y: i32, w: i32, h: i32) -> Option<ScreenRegion> {
    if w <= 0 || h <= 0 {
        return None;
    }
    // Intersect with the non-negative quadrant the config can represent.
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x.saturating_add(w)).max(0) as u32;
    let y1 = (y.saturating_add(h)).max(0) as u32;
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

/// Logical positions are fractional on niri (e.g. at 1.25 scale a 2 physical
/// pixel border is 1.6 logical pixels), so round them before clamping.
fn region_from_logical(x: f64, y: f64, w: f64, h: f64) -> Option<ScreenRegion> {
    if [x, y, w, h].iter().any(|v| !v.is_finite()) {
        return None;
    }
    region_from_bounds(
        x.round() as i32,
        y.round() as i32,
        w.round() as i32,
        h.round() as i32,
    )
}

/// `(x, y, width, height)` from a JSON object's `rect` member.
fn rect_of(node: &serde_json::Value) -> Option<(i32, i32, i32, i32)> {
    let rect = &node["rect"];
    let (x, y) = (rect["x"].as_i64()?, rect["y"].as_i64()?);
    let (w, h) = (rect["width"].as_i64()?, rect["height"].as_i64()?);
    Some((x as i32, y as i32, w as i32, h as i32))
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Match on title and app id the way the old hyprctl code matched title and
/// class: `steam_app_238960` and `pathofexile` both identify the game.
fn is_game_window(text: &str) -> bool {
    contains_any(text, &["pathofexile", "path of exile", "steam_app_238960"])
}

fn is_own_window(text: &str) -> bool {
    text.to_lowercase().contains("little oil")
}

// ── Hyprland ──────────────────────────────────────────────────────

mod hyprland {
    use super::*;

    pub(super) fn game_window() -> Option<ScreenRegion> {
        game_window_in(&run("hyprctl", &["clients", "-j"])?)
    }

    pub(super) fn window_at(x: i32, y: i32) -> Option<ScreenRegion> {
        window_at_in(&run("hyprctl", &["clients", "-j"])?, x, y)
    }

    pub(super) fn monitor_at(x: i32, y: i32) -> Option<ScreenRegion> {
        monitor_at_in(&run("hyprctl", &["monitors", "-j"])?, x, y)
    }

    /// `hyprctl clients -j`: match a window on a case-insensitive title or
    /// class containing the game's name.
    pub(super) fn game_window_in(clients: &[u8]) -> Option<ScreenRegion> {
        let clients: serde_json::Value = serde_json::from_slice(clients).ok()?;
        for w in clients.as_array()?.iter() {
            let (Some(title), Some(class)) = (w["title"].as_str(), w["class"].as_str()) else {
                continue;
            };
            if !is_game_window(&format!("{} {}", title, class).to_lowercase()) {
                continue;
            }
            let (x, y, width, height) = bounds(w)?;
            return region_from_bounds(x, y, width, height);
        }
        None
    }

    pub(super) fn window_at_in(clients: &[u8], x: i32, y: i32) -> Option<ScreenRegion> {
        let clients: serde_json::Value = serde_json::from_slice(clients).ok()?;
        for w in clients.as_array()?.iter() {
            let Some(title) = w["title"].as_str() else {
                continue;
            };
            if is_own_window(title) {
                continue; // our own window
            }
            let Some((wx, wy, ww, wh)) = bounds(w) else {
                continue;
            };
            if x >= wx && x < wx + ww && y >= wy && y < wy + wh {
                return region_from_bounds(wx, wy, ww, wh);
            }
        }
        None
    }

    pub(super) fn monitor_at_in(monitors: &[u8], x: i32, y: i32) -> Option<ScreenRegion> {
        let monitors: serde_json::Value = serde_json::from_slice(monitors).ok()?;
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

    /// `at` and `size` arrays from a `hyprctl clients -j` entry.
    fn bounds(w: &serde_json::Value) -> Option<(i32, i32, i32, i32)> {
        let (at, size) = (w["at"].as_array()?, w["size"].as_array()?);
        let x = at.first()?.as_i64()? as i32;
        let y = at.get(1)?.as_i64()? as i32;
        let ww = size.first()?.as_i64()? as i32;
        let wh = size.get(1)?.as_i64()? as i32;
        Some((x, y, ww, wh))
    }
}

// ── Sway ──────────────────────────────────────────────────────────

mod sway {
    use super::*;

    pub(super) fn game_window() -> Option<ScreenRegion> {
        game_window_in(&run("swaymsg", &["-r", "-t", "get_tree"])?)
    }

    pub(super) fn window_at(x: i32, y: i32) -> Option<ScreenRegion> {
        window_at_in(&run("swaymsg", &["-r", "-t", "get_tree"])?, x, y)
    }

    pub(super) fn monitor_at(x: i32, y: i32) -> Option<ScreenRegion> {
        monitor_at_in(&run("swaymsg", &["-r", "-t", "get_outputs"])?, x, y)
    }

    /// `swaymsg -t get_tree`: walk every container, including tiled `nodes`
    /// and `floating_nodes`, and take the first game window's `rect`.
    pub(super) fn game_window_in(tree: &[u8]) -> Option<ScreenRegion> {
        let tree: serde_json::Value = serde_json::from_slice(tree).ok()?;
        let mut found = None;
        walk(&tree, &mut |node| {
            if found.is_some() {
                return;
            }
            let name = node["name"].as_str().unwrap_or_default();
            let app_id = node["app_id"].as_str().unwrap_or_default();
            if is_game_window(&format!("{name} {app_id}").to_lowercase())
                && let Some((x, y, w, h)) = rect_of(node)
            {
                found = region_from_bounds(x, y, w, h);
            }
        });
        found
    }

    pub(super) fn window_at_in(tree: &[u8], x: i32, y: i32) -> Option<ScreenRegion> {
        let tree: serde_json::Value = serde_json::from_slice(tree).ok()?;
        let mut found = None;
        walk(&tree, &mut |node| {
            if found.is_some() {
                return;
            }
            // Containers and workspaces have names and rects too; only actual
            // toplevels carry an X11 `window` id or a Wayland `app_id`.
            if node["window"].as_u64().is_none() && node["app_id"].as_str().is_none() {
                return;
            }
            let Some((wx, wy, ww, wh)) = rect_of(node) else {
                return;
            };
            if ww <= 0 || wh <= 0 {
                return;
            }
            let name = node["name"].as_str().unwrap_or_default();
            if is_own_window(name) {
                return;
            }
            if x >= wx && x < wx + ww && y >= wy && y < wy + wh {
                found = region_from_bounds(wx, wy, ww, wh);
            }
        });
        found
    }

    /// `swaymsg -t get_outputs`: outputs carry an absolute `rect`; `scale` is
    /// fractional but the rect is already in logical pixels.
    pub(super) fn monitor_at_in(outputs: &[u8], x: i32, y: i32) -> Option<ScreenRegion> {
        let outputs: serde_json::Value = serde_json::from_slice(outputs).ok()?;
        for o in outputs.as_array()?.iter() {
            let Some((ox, oy, ow, oh)) = rect_of(o) else {
                continue;
            };
            if ow <= 0 || oh <= 0 {
                continue;
            }
            if x >= ox && x < ox + ow && y >= oy && y < oy + oh {
                return region_from_bounds(ox, oy, ow, oh);
            }
        }
        None
    }

    /// Depth-first visit of a sway container and its tiled + floating children.
    fn walk(node: &serde_json::Value, f: &mut impl FnMut(&serde_json::Value)) {
        f(node);
        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = node[key].as_array() {
                for child in children {
                    walk(child, f);
                }
            }
        }
    }
}

// ── niri ──────────────────────────────────────────────────────────

mod niri {
    use super::*;

    pub(super) fn game_window() -> Option<ScreenRegion> {
        game_window_in(&windows()?, &workspaces()?, &outputs()?)
    }

    pub(super) fn window_at(x: i32, y: i32) -> Option<ScreenRegion> {
        window_at_in(&windows()?, &workspaces()?, &outputs()?, x, y)
    }

    pub(super) fn monitor_at(x: i32, y: i32) -> Option<ScreenRegion> {
        monitor_at_in(&outputs()?, x, y)
    }

    fn windows() -> Option<Vec<u8>> {
        run("niri", &["msg", "-j", "windows"])
    }

    fn workspaces() -> Option<Vec<u8>> {
        run("niri", &["msg", "-j", "workspaces"])
    }

    fn outputs() -> Option<Vec<u8>> {
        run("niri", &["msg", "-j", "outputs"])
    }

    /// `niri msg -j windows` + workspaces + outputs. Tile positions are in the
    /// workspace view's coordinates, whose origin is the window's output, so
    /// add the output's logical origin. Windows without a
    /// `tile_pos_in_workspace_view` (scrolling-layout tiles on some niri
    /// versions) are skipped.
    pub(super) fn game_window_in(
        windows: &[u8],
        workspaces: &[u8],
        outputs: &[u8],
    ) -> Option<ScreenRegion> {
        windows_in(windows, workspaces, outputs, |_, _| true)
    }

    pub(super) fn window_at_in(
        windows: &[u8],
        workspaces: &[u8],
        outputs: &[u8],
        x: i32,
        y: i32,
    ) -> Option<ScreenRegion> {
        windows_in(windows, workspaces, outputs, |w, text| {
            !is_own_window(text) && region_contains(w, x, y)
        })
    }

    /// Shared window walk. The predicate sees the candidate region and the
    /// lowercased "title app_id" text.
    fn windows_in(
        windows: &[u8],
        workspaces: &[u8],
        outputs: &[u8],
        accept: impl Fn(ScreenRegion, &str) -> bool,
    ) -> Option<ScreenRegion> {
        let windows: serde_json::Value = serde_json::from_slice(windows).ok()?;
        let origin_by_workspace = workspace_outputs(workspaces);
        let logical = logical_outputs(outputs);
        for w in windows.as_array()?.iter() {
            let title = w["title"].as_str().unwrap_or_default();
            let app_id = w["app_id"].as_str().unwrap_or_default();
            let text = format!("{title} {app_id}").to_lowercase();
            let Some(workspace_id) = w["workspace_id"].as_u64() else {
                continue;
            };
            let Some(output) = origin_by_workspace.get(&workspace_id) else {
                continue;
            };
            let Some((ox, oy)) = logical.get(output).copied() else {
                continue;
            };
            let Some(region) = window_region(w, ox, oy) else {
                continue;
            };
            if accept(region, &text) {
                return Some(region);
            }
        }
        None
    }

    /// A window's on-screen bounds: output logical origin + tile position,
    /// sized by the tile (which is what the user sees, decorations included).
    fn window_region(w: &serde_json::Value, ox: i32, oy: i32) -> Option<ScreenRegion> {
        let layout = &w["layout"];
        let pos = layout["tile_pos_in_workspace_view"].as_array()?;
        let px = pos.first()?.as_f64()?;
        let py = pos.get(1)?.as_f64()?;
        let size = layout["tile_size"]
            .as_array()
            .filter(|size| size.len() == 2)
            .or_else(|| {
                layout["window_size"]
                    .as_array()
                    .filter(|size| size.len() == 2)
            })?;
        let (sw, sh) = (size.first()?.as_f64()?, size.get(1)?.as_f64()?);
        region_from_logical(ox as f64 + px, oy as f64 + py, sw, sh)
    }

    fn region_contains(region: ScreenRegion, x: i32, y: i32) -> bool {
        let (x0, y0) = (region.x as i64, region.y as i64);
        let x1 = x0 + region.width as i64;
        let y1 = y0 + region.height as i64;
        (x as i64) >= x0 && (x as i64) < x1 && (y as i64) >= y0 && (y as i64) < y1
    }

    /// `niri msg -j workspaces`: workspace id → output name.
    fn workspace_outputs(workspaces: &[u8]) -> HashMap<u64, String> {
        let workspaces: serde_json::Value = match serde_json::from_slice(workspaces) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        let mut map = HashMap::new();
        for w in workspaces.as_array().into_iter().flatten() {
            if let (Some(id), Some(output)) = (w["id"].as_u64(), w["output"].as_str()) {
                map.insert(id, output.to_string());
            }
        }
        map
    }

    /// `niri msg -j outputs`: output name → logical origin `(x, y)`.
    fn logical_outputs(outputs: &[u8]) -> HashMap<String, (i32, i32)> {
        let outputs: serde_json::Value = match serde_json::from_slice(outputs) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        let mut map = HashMap::new();
        for (name, o) in outputs.as_object().into_iter().flatten() {
            let logical = &o["logical"];
            if let (Some(x), Some(y)) = (logical["x"].as_i64(), logical["y"].as_i64()) {
                map.insert(name.clone(), (x as i32, y as i32));
            }
        }
        map
    }

    /// `niri msg -j outputs`: pick the output whose logical rect contains the
    /// point, then clamp it (outputs left of the primary have negative x).
    pub(super) fn monitor_at_in(outputs: &[u8], x: i32, y: i32) -> Option<ScreenRegion> {
        let outputs: serde_json::Value = serde_json::from_slice(outputs).ok()?;
        for (_, o) in outputs.as_object()?.iter() {
            let logical = &o["logical"];
            let (Some(ox), Some(oy)) = (logical["x"].as_i64(), logical["y"].as_i64()) else {
                continue;
            };
            let (Some(ow), Some(oh)) = (logical["width"].as_i64(), logical["height"].as_i64())
            else {
                continue;
            };
            let (ox, oy, ow, oh) = (ox as i32, oy as i32, ow as i32, oh as i32);
            if ow <= 0 || oh <= 0 {
                continue;
            }
            if x >= ox && x < ox + ow && y >= oy && y < oy + oh {
                return region_from_bounds(ox, oy, ow, oh);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HYPR_CLIENTS: &str = include_str!("../../tests/fixtures/desktops/hyprland/clients.json");
    const HYPR_CLIENTS_GAME: &str =
        include_str!("../../tests/fixtures/desktops/hyprland/clients-game.json");
    const HYPR_CLIENTS_GAME_NEG: &str =
        include_str!("../../tests/fixtures/desktops/hyprland/clients-game-negative.json");
    const HYPR_MONITORS: &str =
        include_str!("../../tests/fixtures/desktops/hyprland/monitors.json");

    const SWAY_TREE: &str = include_str!("../../tests/fixtures/desktops/sway/tree.json");
    const SWAY_OUTPUTS: &str = include_str!("../../tests/fixtures/desktops/sway/outputs.json");

    const NIRI_WINDOWS: &str = include_str!("../../tests/fixtures/desktops/niri/windows.json");
    const NIRI_WINDOWS_NEG: &str =
        include_str!("../../tests/fixtures/desktops/niri/windows-negative.json");
    const NIRI_WINDOWS_NULL: &str =
        include_str!("../../tests/fixtures/desktops/niri/windows-null.json");
    const NIRI_WORKSPACES: &str =
        include_str!("../../tests/fixtures/desktops/niri/workspaces.json");
    const NIRI_OUTPUTS: &str = include_str!("../../tests/fixtures/desktops/niri/outputs.json");

    fn region(x: u32, y: u32, width: u32, height: u32) -> ScreenRegion {
        ScreenRegion {
            x,
            y,
            width,
            height,
        }
    }

    // ── Hyprland ─────────────────────────────────────────────────

    #[test]
    fn hyprland_game_window_uses_the_reported_rect() {
        assert_eq!(
            hyprland::game_window_in(HYPR_CLIENTS_GAME.as_bytes()),
            Some(region(100, 50, 1280, 720))
        );
    }

    #[test]
    fn hyprland_game_window_clamps_a_negative_origin() {
        // at [-200, -40], size [1000, 600] → visible part is 800x560 at 0,0.
        assert_eq!(
            hyprland::game_window_in(HYPR_CLIENTS_GAME_NEG.as_bytes()),
            Some(region(0, 0, 800, 560))
        );
    }

    #[test]
    fn hyprland_real_clients_have_no_game_window() {
        assert_eq!(hyprland::game_window_in(HYPR_CLIENTS.as_bytes()), None);
    }

    #[test]
    fn hyprland_window_at_finds_the_window_under_the_point() {
        // qpwgraph covers [2567, 4473) x [49, 1073) on the real layout.
        assert_eq!(
            hyprland::window_at_in(HYPR_CLIENTS.as_bytes(), 3000, 500),
            Some(region(2567, 49, 1906, 1024))
        );
        assert_eq!(
            hyprland::window_at_in(HYPR_CLIENTS.as_bytes(), 1, 1079),
            None
        );
    }

    #[test]
    fn hyprland_window_at_ignores_our_own_window() {
        let json = r#"[{"at": [10, 10], "size": [400, 300], "class": "little_oil",
                         "title": "Little Oil — select inventory"}]"#;
        assert_eq!(hyprland::window_at_in(json.as_bytes(), 20, 20), None);
    }

    #[test]
    fn hyprland_monitor_at_handles_a_right_of_primary_layout() {
        // DP-2 is the 2560x1440 primary at 0,0; DP-1 starts at x=2560.
        assert_eq!(
            hyprland::monitor_at_in(HYPR_MONITORS.as_bytes(), 0, 0),
            Some(region(0, 0, 2560, 1440))
        );
        assert_eq!(
            hyprland::monitor_at_in(HYPR_MONITORS.as_bytes(), 3000, 100),
            Some(region(2560, 0, 1920, 1080))
        );
        assert_eq!(
            hyprland::monitor_at_in(HYPR_MONITORS.as_bytes(), 5000, 0),
            None
        );
    }

    // ── sway ─────────────────────────────────────────────────────

    #[test]
    fn sway_game_window_is_clamped_from_a_left_hand_output() {
        // The game sits at [-600, 40] 1000x720 on a monitor spanning [-1920, 640).
        assert_eq!(
            sway::game_window_in(SWAY_TREE.as_bytes()),
            Some(region(0, 40, 400, 720))
        );
    }

    #[test]
    fn sway_window_at_walks_tiled_and_floating_nodes() {
        // Firefox tile on the right-hand output: [700, 1500) x [50, 650).
        assert_eq!(
            sway::window_at_in(SWAY_TREE.as_bytes(), 800, 100),
            Some(region(700, 50, 800, 600))
        );
        assert_eq!(sway::window_at_in(SWAY_TREE.as_bytes(), 3000, 3000), None);
    }

    #[test]
    fn sway_monitor_at_uses_output_rects_with_fractional_scale() {
        // DP-1 has scale 1.5 and spans [-1920, 640) in logical pixels.
        assert_eq!(
            sway::monitor_at_in(SWAY_OUTPUTS.as_bytes(), 100, 100),
            Some(region(0, 0, 640, 1080))
        );
        assert_eq!(
            sway::monitor_at_in(SWAY_OUTPUTS.as_bytes(), 1000, 100),
            Some(region(640, 0, 1920, 1080))
        );
    }

    // ── niri ─────────────────────────────────────────────────────

    #[test]
    fn niri_game_window_adds_the_output_origin_and_rounds() {
        // HDMI-A-1 at 0,0; tile pos [100.5, 40.25], tile size [1280.5, 720.25].
        assert_eq!(
            niri::game_window_in(
                NIRI_WINDOWS.as_bytes(),
                NIRI_WORKSPACES.as_bytes(),
                NIRI_OUTPUTS.as_bytes()
            ),
            Some(region(101, 40, 1281, 720))
        );
    }

    #[test]
    fn niri_game_window_clamps_across_a_negative_origin() {
        // DP-1 at x -2560, tile pos 200, tile 3000 wide → visible 640 px.
        assert_eq!(
            niri::game_window_in(
                NIRI_WINDOWS_NEG.as_bytes(),
                NIRI_WORKSPACES.as_bytes(),
                NIRI_OUTPUTS.as_bytes()
            ),
            Some(region(0, 100, 640, 1440))
        );
    }

    #[test]
    fn niri_game_window_without_a_tile_position_is_none() {
        // Scrolling-layout tiles on some niri versions have a null position.
        assert_eq!(
            niri::game_window_in(
                NIRI_WINDOWS_NULL.as_bytes(),
                NIRI_WORKSPACES.as_bytes(),
                NIRI_OUTPUTS.as_bytes()
            ),
            None
        );
    }

    #[test]
    fn niri_window_at_finds_the_game_and_skips_our_own_window() {
        let json = r#"[{"id": 30, "title": "Little Oil", "app_id": "little_oil",
                       "workspace_id": 2,
                       "layout": {"tile_size": [400.0, 300.0],
                                  "tile_pos_in_workspace_view": [1500.0, 700.0]}}]"#;
        assert_eq!(
            niri::window_at_in(
                json.as_bytes(),
                NIRI_WORKSPACES.as_bytes(),
                NIRI_OUTPUTS.as_bytes(),
                1600,
                800
            ),
            None
        );
        assert_eq!(
            niri::window_at_in(
                NIRI_WINDOWS.as_bytes(),
                NIRI_WORKSPACES.as_bytes(),
                NIRI_OUTPUTS.as_bytes(),
                500,
                300
            ),
            Some(region(101, 40, 1281, 720))
        );
    }

    #[test]
    fn niri_monitor_at_uses_logical_rects() {
        assert_eq!(
            niri::monitor_at_in(NIRI_OUTPUTS.as_bytes(), 100, 100),
            Some(region(0, 0, 1920, 1080))
        );
        // DP-1 spans [-2560, 0) at 1.5 scale: nothing left of 0 can be saved.
        assert_eq!(
            niri::monitor_at_in(NIRI_OUTPUTS.as_bytes(), -100, 100),
            None
        );
    }

    // ── dispatch ─────────────────────────────────────────────────

    #[test]
    fn desktops_without_window_geometry_return_none() {
        for desktop in [
            Desktop::Kde,
            Desktop::Gnome,
            Desktop::OtherWayland,
            Desktop::Windows,
        ] {
            assert_eq!(game_window(desktop), None, "{desktop:?}");
            assert_eq!(window_at(desktop, 10, 10), None, "{desktop:?}");
            assert_eq!(monitor_at(desktop, 10, 10), None, "{desktop:?}");
        }
    }
}
