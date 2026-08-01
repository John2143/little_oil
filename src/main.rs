use anyhow::bail;
use mouse_keyboard_input::key_codes;
use tracing::{debug, info};
use parking_lot::RwLock;
use std::sync::LazyLock;

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::process::Command;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use device::{click, click_right, ctrl_click, ctrl_right_click, move_mouse, FAKE_DEVICE};
use crate::auto_roll::AutoRollConfig;
use crate::auto_roll::AutoRollMod;
use screenshot::{Rect, ScreenshotData};
use platform::Platform;

mod auto_roll;
mod chaos_recipe;
mod dicts;
pub mod item;
mod device;
mod screenshot;
mod platform;
mod stash_grid;

use stash_grid::{QUAD_COLS, QUAD_ROWS, StashGrid};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    chaos_recipe_settings: Option<chaos_recipe::ChaosRecipe>,
    #[serde(default = "default_pull_delay")]
    pull_delay: u64,
    #[serde(default = "default_push_delay")]
    push_delay: u64,
    #[serde(default = "default_div_delay")]
    div_delay: u64,
    /// Three probe colors per inventory slot, 60 slots, column-major
    /// (index = col * 5 + row) to match the existing loop order.
    #[serde(default)]
    inv_samples: Option<Vec<[u32; 3]>>,
    #[serde(default)]
    pub platform: Option<Platform>,
    #[serde(default)]
    pub inv_region: Option<ScreenRegion>,
    #[serde(default)]
    pub stash_region: Option<ScreenRegion>,
    #[serde(default)]
    pub game_window_region: Option<ScreenRegion>,
    /// Relative device units emitted per screen pixel of pointer motion.
    /// Depends on pointer DPI, compositor sensitivity, and accel profile, so it
    /// is machine-specific. Measure with: little_oil calibrate-pointer
    #[serde(default)]
    pub pointer_scale: Option<f32>,
    #[serde(default)]
    pub stash_grid: Option<stash_grid::StashGrid>,
}

impl Settings {
    fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        let platform = self.platform.unwrap_or_else(Platform::detect);
        platform.screenshot(self)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScreenRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

const fn default_pull_delay() -> u64 { 50 }
const fn default_push_delay() -> u64 { 40 }
const fn default_div_delay() -> u64 { 100 }



static DEFAULT_SETTINGS: Settings = Settings {
    chaos_recipe_settings: None,
    pull_delay: 50,
    push_delay: 40,
    div_delay: 100,
    inv_samples: None,
    platform: None,
    inv_region: None,
    stash_region: None,
    game_window_region: None,
    pointer_scale: Some(1.25),
    stash_grid: None,
};

static SETTINGS: LazyLock<RwLock<Settings>> = LazyLock::new(|| RwLock::new(DEFAULT_SETTINGS.clone()));


/// Returns the path to the config file: $XDG_CONFIG_HOME/little_oil/config.json
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .expect("no XDG config directory")
        .join("little_oil")
        .join("config.json")
}

pub fn save_config<T: Serialize>(path: &Path, set: &T) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&set).unwrap().as_bytes())?;
    Ok(())
}

fn load_config<T>(path: &Path, default: Option<&T>) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
{
    match fs::File::open(&path) {
        Ok(mut f) => {
            let mut config_text = String::new();
            if let Err(msg) = f.read_to_string(&mut config_text) {
                bail!("Could not read settings: {}", msg);
            }

            let x = serde_json::from_str(&config_text);

            match x {
                Ok(settings) => Ok(settings),
                Err(msg) => bail!("Could not parse settings: {}", msg),
            }
        }
        Err(_f) => match default {
            Some(obj) => match save_config(&path, &obj) {
                Ok(_) => Ok(obj.clone()),
                Err(e) => bail!("Could not write defualt settings: {}", e),
            },
            None => bail!("File not found and no default given"),
        },
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting main loop");

    // plug in our fake input device
    let _ = FAKE_DEVICE.lock().synchronize();

    let set = load_config(&config_path(), Some(&DEFAULT_SETTINGS))?;

    *SETTINGS.write() = set;

    // Ensure platform is set (auto-detect on first run, or use config value)
    {
        let mut settings = SETTINGS.write();
        if settings.platform.is_none() {
            settings.platform = Some(Platform::detect());
            save_config(&config_path(), &*settings)?;
        }
    }

    //println!("got config: {:?}", SETTINGS.read());
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.get(0).map(|x| &**x) {
        Some("config") => {
            // Print the ACTIVE config. Calibration is only inspectable this way.
            println!("{}", serde_json::to_string_pretty(&*SETTINGS.read())?);
            return Ok(());
        }
        Some("version") | Some("--version") => {
            println!("little_oil v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("sort") => {
            let times = args
                .get(1)
                .map(|x| x.parse())
                .unwrap_or(Ok(40))
                .expect("invalid number");

            return sort_quad(times);
        }
        Some("empty") => {
            let settings = SETTINGS.read().clone();
            return empty_inv(&settings);
        }
        Some("emptyr") => {
            let settings = SETTINGS.read().clone();
            return empty_inv_right(&settings);
        }
        Some("roll") => {
            let file = args.get(1).expect("missing name to roll");
            let times = args
                .get(2)
                .expect("missing number of times to roll")
                .parse()
                .expect("invalid number");

            auto_roll::auto_roll(&file, times);
            return Ok(());
        }
        Some("reset_inv") => {
            return reset_inv_colors();
        }
        Some("calibrate-pointer") => {
            if SETTINGS.read().platform.unwrap_or_else(Platform::detect) != Platform::Wayland {
                bail!("calibrate-pointer is Wayland-only; set pointer_scale manually in config");
            }
            return calibrate_pointer();
        }
        Some("calibrate-stash") => {
            return calibrate_stash();
        }
        Some("stash") => {
            let mode = args.get(1).map(|x| &**x);
            match mode {
                Some("click") => {
                    let times = args
                        .get(2)
                        .map(|x| x.parse())
                        .unwrap_or(Ok(40))
                        .expect("invalid number");
                    return sort_quad(times);
                }
                Some("copy") => {
                    return stash_copy();
                }
                _ => {
                    println!("Usage: little_oil stash <click|copy> [times]");
                    println!("  click <times>  Left-click every highlighted cell (hold Ctrl to pull, Shift to identify)");
                    println!("  copy           Hover every highlighted cell and Ctrl+Alt+C it, printing unique items");
                    return Ok(());
                }
            }
        }
        Some("set-region") => {
            let region_name = args.get(1).map(|x| &**x).unwrap_or("help");
            let settings = SETTINGS.read();
            let platform = settings.platform.unwrap_or_else(Platform::detect);
            drop(settings);

            let region = match region_name {
                "inventory" => {
                    let r = platform.select_region("Select the INVENTORY grid region (top-left slot to bottom-right slot)")?;
                    SETTINGS.write().inv_region = Some(r);
                    r
                }
                "stash" => {
                    let r = platform.select_region("Select the STASH grid region (top-left slot to bottom-right slot)")?;
                    SETTINGS.write().stash_region = Some(r);
                    r
                }
                "window" => {
                    let r = platform.select_region("Select the GAME WINDOW (drag around the entire PoE window)")?;
                    SETTINGS.write().game_window_region = Some(r);
                    r
                }
                _ => {
                    eprintln!("Usage: little_oil set-region <inventory|stash|window>");
                    return Ok(());
                }
            };

            save_config(&config_path(), &*SETTINGS.read())?;
            #[cfg(target_os = "linux")]
            let _ = Command::new("notify-send")
                .args(["-u", "low", "Little Oil", &format!("{region_name} region saved: {}x{} at ({}, {})", region.width, region.height, region.x, region.y)])
                .spawn();

            return Ok(());
        }
        Some("chance") => {
            return chance();
        }
        Some("tally") => {
            let settings = SETTINGS.read();
            let c = match settings.chaos_recipe_settings.clone() {
                Some(s) => s,
                None => bail!("No chaos recipe config found"),
            };

            drop(settings);

            chaos_recipe::get_tally(&c);
            return Ok(());
        }
        Some("chaos") => {
            let amt: usize = args
                .get(1)
                .unwrap_or(&"1".to_string())
                .parse()
                .expect("Invalid number of recipes, try 1 or 2");

            let settings = SETTINGS.read();
            let c = match settings.chaos_recipe_settings.clone() {
                Some(s) => s,
                None => {
                    bail!("No chaos recipe config found");
                }
            };

            drop(settings);

            chaos_recipe::do_recipe(&c, amt);
            return Ok(());
        }
        Some(n) => {
            println!("Invalid command: {}", n);
            return Ok(());
        }

        None => {}
    }

    let cmdline = std::thread::spawn(move || {
        command_line();
    });

    cmdline.join().unwrap();
    Ok(())
}

fn split_space(input: &str) -> (&str, &str) {
    for (i, c) in input.chars().enumerate() {
        if c == ' ' {
            return (&input[0..i], &input[i + 1..]);
        }
    }
    return (input, "");
}

fn try_read_item_on_cursor() -> Option<String> {
    use wl_clipboard_rs::utils::{is_primary_selection_supported, PrimarySelectionCheckError};

    match is_primary_selection_supported() {
        Ok(_supported) => {}
        Err(PrimarySelectionCheckError::NoSeats) => {
            println!("no seats, cannot check for primary selection support");
            return None;
        }
        Err(PrimarySelectionCheckError::MissingProtocol) => {
            println!("data-control protocol not supported");
            return None;
        }
        Err(e) => {
            println!("error checking for primary selection support: {:?}", e);
            return None;
        }
    }

    // clear the clipboard
    {
        use wl_clipboard_rs::copy::{copy, MimeType, Options, Source};
        let opts = Options::new();
        copy(opts, Source::Bytes([].into()), MimeType::Autodetect).unwrap();
    }

    let mut i = 0;
    loop {
        {
            let mut device = FAKE_DEVICE.lock();
            // press ctrl alt c
            device.press(key_codes::KEY_LEFTCTRL).unwrap();
            device.press(key_codes::KEY_LEFTALT).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
            device.press(key_codes::KEY_C).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(rand::random_range(4..25)));
            device.release(key_codes::KEY_C).unwrap();
            device.release(key_codes::KEY_LEFTALT).unwrap();
            device.release(key_codes::KEY_LEFTCTRL).unwrap();
        }

        //250 ms total
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));

            use wl_clipboard_rs::{paste::{get_contents, ClipboardType, Error, MimeType, Seat}};

            match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
                Ok((mut pipe, _x)) => {
                    let mut contents = vec![];
                    pipe.read_to_end(&mut contents).unwrap();
                    let clip_res = String::from_utf8_lossy(&contents);
                    if clip_res.len() > 0 {
                        return Some(clip_res.to_string());
                    }
                }
                Err(Error::NoSeats) => {
                    println!("no seats");
                }
                Err(Error::ClipboardEmpty) => {
                    println!("empty");
                }
                Err(Error::NoMimeType) => {
                    println!("no mimetype");
                }
                Err(e) => {
                    println!("clipboard error: {:?}", e);
                }
            }
        }

        i += 1;
        if i > 5 {
            println!("clipboard was always empty, giving up");
            return None;
        }

        std::thread::sleep(std::time::Duration::from_millis(rand::random_range(1..150)));
    }
}

fn read_item_on_cursor() -> String {
    try_read_item_on_cursor().expect("could not read item on cursor")
}


/// Print a prompt, mirror it to notify-send on Linux, and block until Enter.
fn prompt_enter(msg: &str) -> anyhow::Result<()> {
    println!("\n>>> {msg}\n    Press Enter when ready...");
    #[cfg(target_os = "linux")]
    let _ = Command::new("notify-send")
        .args(["-u", "critical", "Little Oil — calibration", msg])
        .spawn();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}


fn calibrate_pointer() -> anyhow::Result<()> {
    const D: i32 = 400; // device units; large enough that the two cursor
                        // positions cannot overlap even at max deceleration
    println!("Measuring pointer scale. Do not move the mouse or type.");
    println!("Close any animated window; a static desktop measures cleanly.");

    device::pin_cursor_to_origin();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let before = platform::wayland::capture_all_with_cursor()?;

    device::move_mouse_raw(D, D);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let after = platform::wayland::capture_all_with_cursor()?;

    let bounds = Rect {
        x: 0,
        y: 0,
        width: before.width as u32,
        height: before.height as u32,
    };
    let clusters = screenshot::diff_clusters(&before, &after, bounds, 4)?;
    let [a, b] = clusters.as_slice() else {
        bail!(
            "Expected exactly 2 changed regions (cursor before and after), found {}. \
             Something on screen is animating — close it and retry.",
            clusters.len()
        );
    };

    let (ax, ay) = a.center();
    let (bx, by) = b.center();
    let dx = (bx as f32 - ax as f32).abs();
    let dy = (by as f32 - ay as f32).abs();
    if dx < 1.0 || dy < 1.0 {
        bail!("Cursor did not move measurably on both axes — got dx={dx}, dy={dy}");
    }

    let sx = D as f32 / dx;
    let sy = D as f32 / dy;
    if (sx - sy).abs() / sx.max(sy) > 0.02 {
        bail!(
            "Axes scale differently (x={sx:.4}, y={sy:.4}) — pointer_scale assumes \
             one factor. Set pointer_scale manually in config."
        );
    }
    let scale = (sx + sy) / 2.0;

    {
        let mut settings = SETTINGS.write();
        settings.pointer_scale = Some(scale);
        save_config(&config_path(), &*settings)?;
    }
    println!("pointer_scale = {scale:.4} (was measured over {D} device units)");
    Ok(())
}

fn calibrate_stash() -> anyhow::Result<()> {
    let snapshot = { SETTINGS.read().clone() };

    let _game_region = snapshot
        .game_window_region
        .ok_or_else(|| anyhow::anyhow!("Game window region not set — run: little_oil set-region window"))?;
    let stash_region = snapshot
        .stash_region
        .ok_or_else(|| anyhow::anyhow!("Stash region not set — run: little_oil set-region stash"))?;

    prompt_enter("Open the quad tab.")?;
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Each calibration position is captured twice: first with the item in place
    // and a search that matches NOTHING (nonsense string), then with a search
    // matching exactly the item. Diffing those two isolates ONLY the search
    // highlight: the item art is present in both frames (so it cancels out),
    // and any non-matching items in the stash stay dimmed in both frames (so
    // they cancel out too). The highlight ring around the cell is the largest
    // changed region; its bounding box is the cell.
    let capture_pos = |where_text: &str| -> anyhow::Result<(ScreenshotData, ScreenshotData, Rect, Rect)> {
        prompt_enter(&format!(
            "Put the 1×1 item in the {where_text} slot. In the search box type a \
             nonsense string (e.g. \"zzz\") so NOTHING is highlighted."
        ))?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        let base = snapshot.screenshot()?;

        // Convert stash_region from screen space to frame-pixel space.
        let (bx, by) = base.from_screen(stash_region.x, stash_region.y).ok_or_else(|| {
            anyhow::anyhow!(
                "Stash region starts outside the game window region — re-run set-region window and set-region stash"
            )
        })?;
        let bounds = Rect {
            x: bx,
            y: by,
            width: stash_region.width.min(base.width as u32 - bx),
            height: stash_region.height.min(base.height as u32 - by),
        };

        prompt_enter(&format!(
            "Now search the item's exact name so ONLY it is highlighted (keep it in the {where_text} slot)."
        ))?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        let cap = snapshot.screenshot()?;

        let clusters = screenshot::diff_clusters(&base, &cap, bounds, 20)?;
        let cell = clusters
            .iter()
            .max_by_key(|r| r.width * r.height)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No changed pixels in the {where_text} capture — check that the search actually highlights the item"
                )
            })?;
        Ok((base, cap, bounds, cell))
    };

    let (base, tl, _bounds, tl_cell) = capture_pos("TOP-LEFT")?;

    // Derive highlight_color from the tl cell's bottom boundary row.
    use std::collections::HashMap;
    let mut color_tally: HashMap<u32, u32> = HashMap::new();
    let by_abs = (tl_cell.y + tl_cell.height.saturating_sub(1)) as usize;
    for x in tl_cell.x as usize..(tl_cell.x + tl_cell.width) as usize {
        if base.try_get_pixel(x, by_abs) != tl.try_get_pixel(x, by_abs) {
            if let Some(c) = tl.try_get_pixel(x, by_abs) {
                *color_tally.entry(c).or_insert(0) += 1;
            }
        }
    }
    let highlight_color = color_tally
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(c, _)| c)
        .ok_or_else(|| anyhow::anyhow!(
            "Could not determine highlight color — the highlighted cell's bottom edge did not change"
        ))?;

    let (_base2, _br, _bounds2, br_cell) = capture_pos("BOTTOM-RIGHT")?;

    let grid = StashGrid::from_corners(tl_cell, br_cell, highlight_color)?;

    let (_base3, mid, _bounds3, mid_cell) = capture_pos("any MIDDLE")?;

    // Find the (col, row) whose cell_center is nearest the middle cluster's center.
    let (mcx, mcy) = mid_cell.center();
    let mut best_col = 0usize;
    let mut best_row = 0usize;
    let mut best_dist = f64::MAX;
    for col in 0..QUAD_COLS {
        for row in 0..QUAD_ROWS {
            let (ccx, ccy) = grid.cell_center(col, row);
            let dx = ccx as f64 - mcx as f64;
            let dy = ccy as f64 - mcy as f64;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < best_dist {
                best_dist = dist;
                best_col = col;
                best_row = row;
            }
        }
    }

    // Validate: the cell center must be inside the mid cluster expanded by 4 px per side.
    let (ccx, ccy) = grid.cell_center(best_col, best_row);
    if ccx < mid_cell.x.saturating_sub(4)
        || ccx > mid_cell.x + mid_cell.width + 4
        || ccy < mid_cell.y.saturating_sub(4)
        || ccy > mid_cell.y + mid_cell.height + 4
    {
        bail!(
            "Calibration failed validation: middle item at ({}, {}) does not line up with computed cell ({}, {}) — re-run calibration",
            mcx, mcy, ccx, ccy
        );
    }

    if !grid.is_highlighted(&mid, best_col, best_row) {
        bail!(
            "Computed grid found the middle cell but its bottom edge does not match the calibrated highlight color — re-run calibration"
        );
    }

    {
        let mut settings = SETTINGS.write();
        settings.stash_grid = Some(grid.clone());
        save_config(&config_path(), &*settings)?;
    }

    let (sx, sy) = base.to_screen(grid.cols[0], grid.rows[0]);
    println!(
        "Stash grid calibrated: cell {}x{}, frame-pixel origin ({}, {}) = screen ({}, {}), highlight 0x{:08X}",
        grid.cell_w, grid.cell_h, grid.cols[0], grid.rows[0], sx, sy, grid.highlight_color
    );
    Ok(())
}



/// Click the bottom-middle of the player inventory panel so the game window
/// receives keyboard focus before any automation starts. Without this, a
/// terminal-launched command leaves keyboard focus in the terminal, and the
/// Ctrl the operator holds (or ctrl_click sends) never reaches the game.
fn focus_game_window() -> anyhow::Result<()> {
    let region = {
        let settings = SETTINGS.read();
        settings
            .inv_region
            .or(settings.game_window_region)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No region to click for focus — run: little_oil set-region inventory (or set-region window)"
                )
            })?
    };
    let sx = region.x + region.width / 2;
    let sy = region.y + region.height.saturating_sub(2);
    println!("Focus click at ({sx}, {sy}) — game window should come to the foreground");
    // Click twice: the first click is often consumed by the compositor just to
    // hand focus to the window, and the second lands in the now-focused game.
    click(sx as i32, sy as i32);
    std::thread::sleep(std::time::Duration::from_millis(20));
    click(sx as i32, sy as i32);
    std::thread::sleep(std::time::Duration::from_millis(100));
    Ok(())
}
fn stash_copy() -> anyhow::Result<()> {
    focus_game_window()?;

    let settings = SETTINGS.read();
    let grid = match &settings.stash_grid {
        Some(g) => g.clone(),
        None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
    };
    let frame = settings.screenshot()?;
    drop(settings);

    let mut seen: Vec<String> = Vec::new();
    let mut failed = 0u32;

    for row in 0..QUAD_ROWS {
        for col in 0..QUAD_COLS {
            if !grid.is_highlighted(&frame, col, row) {
                continue;
            }
            let (px, py) = grid.cell_center(col, row);
            let (sx, sy) = frame.to_screen(px, py);
            move_mouse(sx, sy);
            std::thread::sleep(std::time::Duration::from_millis(30));
            match try_read_item_on_cursor() {
                Some(text) if !seen.contains(&text) => seen.push(text),
                Some(_) => {}
                None => failed += 1,
            }
        }
    }

    for item in &seen {
        println!("{item}");
        println!("--------");
    }
    println!("{} unique items, {} cells failed to copy", seen.len(), failed);

    Ok(())
}

fn chance() -> anyhow::Result<()> {
    let chance = (237, 292);
    let scour = (169, 472);
    let slot = (323, 522);
    let sleep_click = 30;
    let sleep_read = 250;

    for _ in 1..10 {
        click_right(chance.0, chance.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click(slot.0, slot.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_read));

        click_right(scour.0, scour.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click(slot.0, slot.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_read));
    }

    Ok(())
}

static HELP: &str = r#"
help: Show this menu
version: Print version and exit
empty: Empty the inventory into the stash (ctrl+left click)
emptyr: Empty the inventory into the stash (ctrl+right click)
set-region <inventory|stash|window>: Select and save a screen region
calibrate-pointer: Measure pointer scale (run once per machine)
calibrate-stash: Calibrate the 24x24 quad tab grid (3 positions, base + search capture each)
stash <click|copy> [times]: Act on highlighted quad-tab cells
pull <delay>: Change delay for pulling out of quad tab
push <delay>: Change delay for pushing into tab/trade
div <delay>: Change delay for div macro
chrome <file> <times>: Open a autoroll file, with name <file>, and roll item <times>
mchrome <file>: Create example chrome file with name <file>. To be used with chrome later.

Press CTRL + C to quit this program.
"#;

fn command_line() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let (cmd, rest) = split_space(&line);
        match cmd {
            "pull" | "push" | "div" => match rest.parse::<u64>() {
                Ok(x) => {
                    let mut s = SETTINGS.write();
                    match cmd {
                        "pull" => s.pull_delay = x,
                        "push" => s.push_delay = x,
                        "div" => s.div_delay = x,
                        _ => unreachable!(),
                    }
                    println!("{cmd} delay is {x}");
                    if let Err(e) = save_config(&config_path(), &*s) {
                        println!("could not save config: {e}");
                    }
                }
                Err(_) => println!("invalid delay: {rest}"),
            },
            "chrome" => {
                let (file, times) = split_space(rest);
                println!("Loading chrome file {}", file);

                match auto_roll::auto_roll(&file, times.parse().unwrap()) {
                    None => println!("failed to roll"),
                    Some(res) => {
                        println!("{:?}", res);
                    }
                }
            }
            "mchrome" => {
                println!("Making chrome file {}", rest);

                save_config(
                    std::path::Path::new(rest),
                    &AutoRollConfig {
                        auto_aug_regal: false,
                        item_name: "Medium Cluster Jewel".to_string(),
                        any_two_t1: false,
                        needs_prefix_and_suffix: false,
                        mods: vec![
                            AutoRollMod {
                                name: "heraldry".into(),
                                is_prefix: true,
                            },
                            AutoRollMod {
                                name: "harbinger".into(),
                                is_prefix: true,
                            },
                            AutoRollMod {
                                name: "endbringer".into(),
                                is_prefix: true,
                            },
                        ],
                    },
                )
                .unwrap();
            }
            "help" => {
                println!("Available Commands: {}", HELP);
            }
            _ => println!("Unknown command"),
        }
    }
}

/// Three probes across the vertical middle of an inventory slot, at 25%, 50%
/// and 75% of its width, in frame-pixel space. None if the slot falls outside
/// the captured frame.
fn inv_probes(
    frame: &ScreenshotData,
    region: ScreenRegion,
    col: u32,
    row: u32,
) -> Option<[(usize, usize); 3]> {
    if region.width < 12 || region.height < 5 {
        return None;
    }
    let dx = region.width / 12;
    let dy = region.height / 5;
    // Validate the far corner of the slot is in frame.
    let _far = frame.from_screen(region.x + (col + 1) * dx - 1, region.y + (row + 1) * dy - 1)?;
    let (ox, oy) = frame.from_screen(region.x + col * dx, region.y + row * dy)?;
    let y = (oy + dy / 2) as usize;
    Some([
        ((ox + dx / 4) as usize, y),
        ((ox + dx / 2) as usize, y),
        ((ox + dx * 3 / 4) as usize, y),
    ])
}

fn reset_inv_colors() -> anyhow::Result<()> {
    let settings = SETTINGS.read();
    let inv_region = settings
        .inv_region
        .ok_or_else(|| anyhow::anyhow!("Inventory region not calibrated — run: little_oil set-region inventory"))?;

    let frame = settings.screenshot()?;
    drop(settings);

    let mut samples = vec![[0u32; 3]; 60];

    for x in 0..12 {
        for y in 0..5 {
            let probes = inv_probes(&frame, inv_region, x, y).ok_or_else(|| {
                anyhow::anyhow!(
                    "Inventory slot ({x}, {y}) falls outside the game window region — re-run set-region window and set-region inventory"
                )
            })?;
            samples[(x * 5 + y) as usize] = [
                frame.try_get_pixel(probes[0].0, probes[0].1).unwrap_or(0),
                frame.try_get_pixel(probes[1].0, probes[1].1).unwrap_or(0),
                frame.try_get_pixel(probes[2].0, probes[2].1).unwrap_or(0),
            ];
        }
    }

    let mut settings = SETTINGS.write();

    let note = Command::new("notify-send")
        .args(["-u", "low", "Little Oil", &format!("Inventory colors calibrated: {} slots x 3 samples", samples.len())])
        .spawn();
    if let Err(e) = note {
        eprintln!("notify-send failed: {e}");
    }
    settings.inv_samples = Some(samples);

    save_config(&config_path(), &*settings)?;
    Ok(())
}

fn empty_inv_macro(settings: &Settings, delay: u64, clicker: fn(i32, i32)) -> anyhow::Result<u32> {
    let inv_region = settings
        .inv_region
        .ok_or_else(|| anyhow::anyhow!("Inventory region not calibrated — run: little_oil set-region inventory"))?;

    info!("Emptying inv");

    let frame = settings.screenshot()?;

    let expected = match settings.inv_samples.as_ref() {
        Some(s) if s.len() == 60 => s,
        _ => bail!("Inventory colors not calibrated — run: little_oil reset_inv"),
    };
    let mut clicked: u32 = 0;

    for x in 0..12 {
        for y in 0..5 {
            let probes = inv_probes(&frame, inv_region, x, y).ok_or_else(|| {
                anyhow::anyhow!(
                    "Inventory slot ({x}, {y}) falls outside the game window region — re-run set-region window and set-region inventory"
                )
            })?;
            let actual = [
                frame.try_get_pixel(probes[0].0, probes[0].1).unwrap_or(0),
                frame.try_get_pixel(probes[1].0, probes[1].1).unwrap_or(0),
                frame.try_get_pixel(probes[2].0, probes[2].1).unwrap_or(0),
            ];
            let stored = expected[(x * 5 + y) as usize];
            let matches = actual.iter().zip(stored.iter()).filter(|(a, e)| a == e).count();

            if matches < 2 {
                debug!(x, y, "clicking inv");
                let (px, py) = (probes[1].0 as u32, probes[1].1 as u32);
                let (sx, sy) = frame.to_screen(px, py);
                clicker(sx, sy);
                clicked += 1;
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
    }

    Ok(clicked)
}

fn empty_inv(settings: &Settings) -> anyhow::Result<()> {
    empty_inv_with(settings, ctrl_click)
}

fn empty_inv_right(settings: &Settings) -> anyhow::Result<()> {
    empty_inv_with(settings, ctrl_right_click)
}

fn empty_inv_with(settings: &Settings, clicker: fn(i32, i32)) -> anyhow::Result<()> {
    focus_game_window()?;

    std::thread::sleep(std::time::Duration::from_millis(500));
    let clicked = empty_inv_macro(settings, settings.push_delay, clicker)?;
    let note = Command::new("notify-send")
        .args(["-u", "low", "Little Oil", &format!("Inventory cleared: {} items moved", clicked)])
        .spawn();
    if let Err(e) = note {
        eprintln!("notify-send failed: {e}");
    }
    Ok(())
}



fn sort_quad(times: u32) -> anyhow::Result<()> {
    focus_game_window()?;
    std::thread::sleep(std::time::Duration::from_millis(300));

    let settings = SETTINGS.read();
    let delay = settings.pull_delay;
    let grid = match &settings.stash_grid {
        Some(g) => g.clone(),
        None => bail!("Stash grid not calibrated — run: little_oil calibrate-stash"),
    };
    let frame = settings.screenshot()?;
    drop(settings);

    let mut movesleft = times;
    for row in 0..QUAD_ROWS {
        for col in 0..QUAD_COLS {
            if movesleft < 1 {
                return Ok(());
            }
            if grid.is_highlighted(&frame, col, row) {
                let (px, py) = grid.cell_center(col, row);
                let (sx, sy) = frame.to_screen(px, py);
                click(sx, sy);
                std::thread::sleep(std::time::Duration::from_millis(delay));
                movesleft -= 1;
            }
        }
    }
    Ok(())
}
