//! little_oil — PoE automation. Entry point, config I/O, and the
//! `Settings`/`ScreenRegion`/`NamedPoint` types; all commands live in
//! [`app`].

use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::platform::{Desktop, detect_backend};

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
    /// Desktop override. `None` auto-detects from the environment; unknown or
    /// unsupported values are rejected at startup by `detect_backend`. The
    /// retired `platform` key is dropped when the config loads (see
    /// `load_config`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<Desktop>,
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
        desktop: None,
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
    let json = serde_json::to_string_pretty(set)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Temp file in the destination directory so the final rename is atomic.
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(json.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Drop the retired `platform` key from a raw config value.
///
/// `platform` used to select `wayland` / `x11` / `windows`; Wayland and Windows
/// now auto-detect (so the value is ignored) and X11 is gone entirely, which
/// must be an error rather than a silent behavior change.
fn migrate_config_value(
    mut value: serde_json::Value,
    path: &Path,
) -> anyhow::Result<serde_json::Value> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(value);
    };
    let Some(old) = obj.remove("platform") else {
        return Ok(value);
    };
    if old.as_str() == Some("x11") {
        bail!(
            "X11 is no longer supported — remove \"platform\" from {}",
            path.display()
        );
    }
    Ok(value)
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

            // Value pre-pass so the retired `platform` key can be migrated
            // before `T` is deserialized.
            let value: serde_json::Value = match serde_json::from_str(&config_text) {
                Ok(value) => value,
                Err(msg) => bail!(
                    "Could not parse {}: {msg} — fix or delete the file",
                    path.display()
                ),
            };
            let value = migrate_config_value(value, path)?;

            match serde_json::from_value(value) {
                Ok(settings) => Ok(settings),
                Err(msg) => bail!(
                    "Could not parse {}: {msg} — fix or delete the file",
                    path.display()
                ),
            }
        }
        Err(_f) => match default {
            Some(obj) => match save_config(path, &obj) {
                Ok(_) => Ok(obj.clone()),
                Err(e) => bail!("Could not write default settings: {}", e),
            },
            None => bail!("File not found and no default given"),
        },
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    crate::platform::windows::set_dpi_awareness();
    tracing_subscriber::fmt::init();
    let config_path = config_path()?;
    let set = load_config(&config_path, Some(&default_settings()))?;
    let backend = detect_backend(&set)?;
    tracing::debug!(backend = backend.name(), "platform backend");
    let app = App::new(set, backend, config_path);
    let args: Vec<String> = std::env::args().skip(1).collect();
    app.run(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entry names of `dir`, sorted so the assertion does not depend on
    /// readdir order.
    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("directory must exist")
            .map(|entry| {
                entry
                    .expect("readable directory entry")
                    .file_name()
                    .into_string()
                    .expect("utf-8 file name")
            })
            .collect();
        names.sort();
        names
    }

    /// `Settings` has no `PartialEq`, so compare the serialized form.
    fn settings_value(settings: &Settings) -> serde_json::Value {
        serde_json::to_value(settings).expect("Settings must serialize")
    }

    #[test]
    fn save_creates_missing_parent_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.json");
        assert!(
            !path.parent().unwrap().exists(),
            "precondition: the parent directory must not exist yet"
        );
        let original = default_settings();

        save_config(&path, &original).expect("save_config must create the parent and the file");

        let loaded: Settings = load_config::<Settings>(&path, None).expect("load back");
        assert_eq!(settings_value(&loaded), settings_value(&original));
    }

    #[test]
    fn save_leaves_no_temp_files_beside_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        let path = sub.join("config.json");

        save_config(&path, &default_settings()).expect("save_config");

        // The write goes through a same-directory temp file that must be renamed
        // (or removed), never left behind next to the config.
        assert_eq!(dir_entries(&sub), vec!["config.json".to_string()]);
    }

    #[test]
    fn save_over_existing_file_replaces_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        let path = sub.join("config.json");
        save_config(&path, &default_settings()).expect("initial save");

        let mut modified = default_settings();
        modified.focus_clicks = 5;
        modified.pointer_scale = Some(2.0);
        save_config(&path, &modified).expect("overwrite");

        let loaded: Settings = load_config::<Settings>(&path, None).expect("load back");
        assert_eq!(settings_value(&loaded), settings_value(&modified));
        assert_ne!(settings_value(&loaded), settings_value(&default_settings()));
        assert_eq!(dir_entries(&sub), vec!["config.json".to_string()]);
    }

    #[test]
    fn unparseable_config_errors_and_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let broken = "{ this is not json";
        fs::write(&path, broken).unwrap();

        // Even with a default available, a corrupt file must be reported rather
        // than silently replaced: falling back would wipe the calibration that
        // the user would otherwise repair by hand.
        let err = load_config::<Settings>(&path, Some(&default_settings()))
            .expect_err("corrupt config must not load");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("fix or delete the file"),
            "error must tell the user how to recover, got: {msg}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn missing_config_is_written_from_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        let default = default_settings();

        let returned = load_config::<Settings>(&path, Some(&default)).expect("load default");
        assert_eq!(settings_value(&returned), settings_value(&default));

        // The default must be persisted, not just returned: the next run has to
        // find a real file instead of re-writing the default.
        assert!(path.is_file(), "default config must be written to disk");
        let reloaded: Settings =
            load_config::<Settings>(&path, None).expect("reload written default");
        assert_eq!(settings_value(&reloaded), settings_value(&default));
    }

    #[test]
    fn legacy_platform_wayland_key_loads_as_auto_detect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"platform":"wayland","focus_clicks":3}"#).unwrap();

        let loaded: Settings =
            load_config::<Settings>(&path, None).expect("legacy wayland config must load");

        assert_eq!(
            loaded.desktop, None,
            "auto-detect means no explicit override"
        );
        assert_eq!(loaded.focus_clicks, 3, "unrelated settings must survive");
        let reserialized = serde_json::to_string(&loaded).unwrap();
        assert!(
            !reserialized.contains("\"platform\""),
            "the retired key must not be written back: {reserialized}"
        );
    }

    #[test]
    fn legacy_platform_windows_key_loads_as_auto_detect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"platform":"windows"}"#).unwrap();

        let loaded: Settings =
            load_config::<Settings>(&path, None).expect("legacy windows config must load");
        assert_eq!(loaded.desktop, None);
    }

    #[test]
    fn legacy_platform_x11_key_errors_and_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let original = r#"{"platform":"x11"}"#;
        fs::write(&path, original).unwrap();

        let err = load_config::<Settings>(&path, None).expect_err("X11 must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("X11 is no longer supported"),
            "error must explain the removal, got: {msg}"
        );
        assert!(
            msg.contains(&path.display().to_string()),
            "error must name the config file, got: {msg}"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original,
            "a rejected config must be left untouched"
        );
    }

    #[test]
    fn desktop_override_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut settings = default_settings();
        settings.desktop = Some(Desktop::Hyprland);

        save_config(&path, &settings).expect("save override");
        let loaded: Settings = load_config::<Settings>(&path, None).expect("load override");

        assert_eq!(loaded.desktop, Some(Desktop::Hyprland));
        assert_eq!(settings_value(&loaded), settings_value(&settings));
    }

    #[test]
    fn unknown_config_keys_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"future_option":true,"focus_clicks":4}"#).unwrap();

        let loaded: Settings =
            load_config::<Settings>(&path, None).expect("unknown keys must not fail the load");
        assert_eq!(loaded.focus_clicks, 4);
    }
}
