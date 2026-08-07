//! little_oil — PoE automation. Entry point, config I/O, and the
//! `Settings`/`ScreenRegion`/`NamedPoint` types; all commands live in
//! [`app`].

use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::platform::Platform;
use screenshot::ScreenshotData;

mod app;
mod auto_roll;
mod chaos_recipe;
mod dicts;
mod gui;
mod health;
pub mod item;
mod platform;
mod screenshot;
mod stash_grid;
#[cfg(test)]
mod test_support;
pub use app::App;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    chaos_recipe_settings: Option<chaos_recipe::ChaosRecipe>,
    #[serde(default = "default_pull_delay")]
    pull_delay: u64,
    #[serde(default = "default_div_delay")]
    div_delay: u64,
    /// Inter-click settle for the roll macro (auto_roll). Tune down on a fast
    /// machine; raise if orbs don't get picked up. Set via config.json.
    #[serde(default = "default_roll_click_delay")]
    roll_click_delay: u64,
    /// Settle after applying an orb before re-reading the tooltip. Raise if
    /// rolls read a stale item text. Set via config.json.
    #[serde(default = "default_roll_read_delay")]
    roll_read_delay: u64,
    /// How many clicks focus_game_window sends when grabbing game focus.
    /// Click-to-focus compositors (Hyprland) consume the first click for focus,
    /// so the default 2 ensures the second lands in the game. Compositors that
    /// pass the first click through (some niri setups) would double-grab an
    /// item — set focus_clicks = 1 there. Set via config.json.
    #[serde(default = "default_focus_clicks")]
    focus_clicks: u32,
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
    #[serde(default)]
    pub map_region: Option<ScreenRegion>,
    #[serde(default)]
    pub map_grid: Option<stash_grid::MapGrid>,
    /// Named clickable points (currency slots, filter button, …), screen space.
    #[serde(default)]
    pub points: Option<Vec<NamedPoint>>,
    /// True once the GUI first-run wizard has been completed. Cosmetic only —
    /// decides which tab the GUI opens on. Old configs load as false via serde.
    #[serde(default)]
    setup_complete: bool,
}

impl Settings {
    pub(crate) fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
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

impl ScreenRegion {
    /// Middle of the region, in screen pixels.
    pub fn center(&self) -> (u32, u32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NamedPoint {
    pub name: String,
    pub region: ScreenRegion,
}

const fn default_pull_delay() -> u64 {
    50
}
const fn default_div_delay() -> u64 {
    100
}
const fn default_roll_click_delay() -> u64 {
    10
}
const fn default_roll_read_delay() -> u64 {
    75
}
const fn default_focus_clicks() -> u32 {
    2
}

fn default_settings() -> Settings {
    Settings {
        chaos_recipe_settings: None,
        pull_delay: 50,
        div_delay: 100,
        roll_click_delay: 10,
        roll_read_delay: 75,
        focus_clicks: 2,
        inv_samples: None,
        platform: None,
        inv_region: None,
        stash_region: None,
        game_window_region: None,
        pointer_scale: Some(1.25),
        stash_grid: None,
        map_region: None,
        map_grid: None,
        points: None,
        setup_complete: false,
    }
}

/// Returns the path to the config file: $XDG_CONFIG_HOME/little_oil/config.json
pub fn config_path() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("no XDG config directory — set XDG_CONFIG_HOME or HOME"))
        .map(|d| d.join("little_oil").join("config.json"))
}

/// Path to the persistent roll log: $XDG_CONFIG_HOME/little_oil/rolls.log
pub fn rolls_log_path() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("no XDG config directory — set XDG_CONFIG_HOME or HOME"))
        .map(|d| d.join("little_oil").join("rolls.log"))
}

pub fn save_config<T: Serialize>(path: &Path, set: &T) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    let json = serde_json::to_string_pretty(&set)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

fn load_config<T>(path: &Path, default: Option<&T>) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
{
    match fs::File::open(path) {
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
            Some(obj) => match save_config(path, &obj) {
                Ok(_) => Ok(obj.clone()),
                Err(e) => bail!("Could not write defualt settings: {}", e),
            },
            None => bail!("File not found and no default given"),
        },
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    crate::platform::windows::set_dpi_awareness();
    tracing_subscriber::fmt::init();
    let mut set = load_config(&config_path()?, Some(&default_settings()))?;
    // Ensure platform is set (auto-detect on first run, or use config value).
    if set.platform.is_none() {
        set.platform = Some(Platform::detect());
        save_config(&config_path()?, &set)?;
    }
    let app = App::new(set)?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    app.run(&args)
}
