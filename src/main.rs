use anyhow::{bail, Context};
use clap::Parser;
use mouse::{click, click_right};
use rand::Rng;
use screenshot::ScreenshotData;
use tracing::{info, trace};
use once_cell::sync::Lazy;
use std::sync::RwLock;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

mod auto_roll;
mod chaos_recipe;
mod dicts;
pub mod item;
pub mod mouse;
mod screenshot;
mod actions;

/// This file is loaded from your config directory (`$HOME/.config/little_oil.json`)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    chaos_recipe_settings: Option<chaos_recipe::ChaosRecipe>,
    pull_delay: u64,
    push_delay: u64,
    div_delay: u64,
    inv_colors: Option<Vec<u32>>,
    poe_window_location: Rect,
    inv_delta_override: Option<u32>,
    monitor_scaling_factor: f32,
    screenshot_method: ScreenshotMethod,
    input_method: InputMethod,
    pos: InvPositions,
}

/// From the image crate
/// A Rectangle defined by its top left corner, width and height.
#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    /// The x coordinate of the top left corner.
    pub x: u32,
    /// The y coordinate of the top left corner.
    pub y: u32,
    /// The rectangle's width.
    pub width: u32,
    /// The rectangle's height.
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum InputMethod {
    #[cfg(feature = "input_wayland")]
    Uinput,
    #[cfg(feature = "input_x")]
    InputBot,
    None,
}

impl Settings {
    fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        trace!(?self.screenshot_method, "Taking a screenshot");
        match self.screenshot_method {
            #[cfg(feature = "input_wayland")]
            ScreenshotMethod::Grim => screenshot::take_screenshot_grim(&self),
            #[cfg(feature = "input_x")]
            ScreenshotMethod::Scrot { primary_monitor } => {
                screenshot::take_screenshot_scrap(self, primary_monitor)
            }
            ScreenshotMethod::None => bail!("No screenshot method defined. check your config!"),
        }
    }

    fn inv_delta(&self) -> u32 {
        // If it's configured, use that
        if let Some(d) = self.inv_delta_override {
            return d;
        }

        // Infer from screen height
        let height = self.poe_window_location.height;
        if height == 1080 {
            53
        } else if height == 1440 {
            return 70;
        } else if height == 1000 {
            return 54;
        } else {
            return (height as f32 / 20.50) as _;
        }

        //panic!("Please set either inv_delta_override or screen_height");
    }
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct InvPositions {
    alt: (u32, u32),
    aug: (u32, u32),
    scour: (u32, u32),
    regal: (u32, u32),
    annul: (u32, u32),
    transmute: (u32, u32),

    inv: (u32, u32),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ScreenshotMethod {
    #[cfg(feature = "input_wayland")]
    /// Wayland users should use an external program like "grim"
    Grim,
    #[cfg(feature = "input_x")]
    /// Windows and Linux users can use scrot
    Scrot {
        primary_monitor: usize,
    },
    None,
}

use std::fs;
use std::io::{Read, Write};

static DEFAULT_SETTINGS: Settings = Settings {
    chaos_recipe_settings: None,
    pull_delay: 50,
    push_delay: 40,
    div_delay: 100,
    inv_colors: None,
    inv_delta_override: None,
    monitor_scaling_factor: 1.0,
    poe_window_location: Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1440,
    },
    screenshot_method: ScreenshotMethod::None,
    input_method: InputMethod::None,
    pos: InvPositions {
        alt: (149, 368),
        aug: (303, 444),
        scour: (580, 688),
        regal: (579, 365),
        annul: (226, 372),
        transmute: (71, 368),
        inv: (1713, 828),
    },
};

static SETTINGS: Lazy<RwLock<Settings>> = Lazy::new(|| RwLock::new(DEFAULT_SETTINGS.clone()));

pub fn get_config_path() -> PathBuf {
    let mut con = dirs::config_dir().unwrap();
    con.push("little_oil.json");
    con
}

pub fn save_config<T: Serialize, P: AsRef<Path>>(path: P, set: &T) -> Result<(), std::io::Error> {
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&set).unwrap().as_bytes())?;

    Ok(())
}

fn load_config<T, P>(path: P, default: Option<&T>) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
    P: AsRef<Path>,
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

#[derive(clap::Parser, Debug)]
struct CliArgs {
    #[command(subcommand)]
    cmd: CliCommand,
}

#[derive(clap::Subcommand, Clone, Debug)]
enum CliCommand {
    /// Print the default configuration file and exit.
    PrintConfig,

    /// Pull items out of a quad tab and into your inventory.
    Sort { times: usize },

    /// Emtpy your inventory into your stash. You must use reset_inv at least once before running
    /// this.
    Empty,
    /// To use this command, open your inventory first. It will take a screenshot and remember your
    /// layout of portal scrolls, maps, essences, or other inventory items you want to keep.
    ///
    /// Then, running the [`CliCommand::Empty`] command will clear out any inventory slots with items in it.
    ResetInv,

    /// (old) Roll an item in your currency tab to get specific stats.
    Roll { times: usize, config: PathBuf },
    /// (old) chance an item into a unique
    Chance,

    /// (old) Count how many chaos items are in a tab.
    Tally,

    /// (old) Do the chaos recipe?
    Chaos,
}

impl CliCommand {
    fn run(&self, settings: &Settings) -> anyhow::Result<()> {
        match self {
            CliCommand::PrintConfig => {
                let s = serde_json::to_string(&DEFAULT_SETTINGS).unwrap();
                println!("{}", s);
                Ok(())
            }
            CliCommand::Sort { times } => {
                sort_quad(settings, *times)
            }
            CliCommand::Empty => {
                empty_inv(settings)
            }
            CliCommand::Roll {
                times,
                config: target_item_config,
            } => {
                let ar_config: auto_roll::AutoRollConfig =
                    load_config(target_item_config, None).context("Loading auto_roll_config")?;

                let roll_res = auto_roll::auto_roll(settings, &ar_config, *times);
                info!(?roll_res);
                Ok(())
            }
            CliCommand::ResetInv => {
                reset_inv_colors(settings)
            }
            CliCommand::Chance => {
                chance()
            }
            CliCommand::Tally => {
                let c = match settings.chaos_recipe_settings.clone() {
                    Some(s) => s,
                    None => bail!("No chaos recipe config found"),
                };

                chaos_recipe::get_tally(&c);
                Ok(())
            }
            CliCommand::Chaos => {
                //let amt: usize = args
                //.get(1)
                //.unwrap_or(&"1".to_string())
                //.parse()
                //.expect("Invalid number of recipes, try 1 or 2");
                let amt = 1;

                let settings = SETTINGS.read().unwrap();
                let c = match settings.chaos_recipe_settings.clone() {
                    Some(s) => s,
                    None => {
                        bail!("No chaos recipe config found");
                    }
                };

                chaos_recipe::do_recipe(&c, amt);
                Ok(())
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cmd = CliArgs::try_parse()?;
    tracing_subscriber::fmt::init();
    info!("Trying to create an input device.");
    // Wake the mouse device first, assume we will need to use it
    mouse::init();

    let set = load_config(get_config_path(), Some(&DEFAULT_SETTINGS))?;

    *SETTINGS.write().unwrap() = set.clone();

    cmd.cmd.run(&set)?;

    Ok(())
}

use clipboard::ClipboardContext;
use clipboard::ClipboardProvider;

fn read_item_on_cursor() -> String {
    static mut CTX: Option<ClipboardContext> = None;

    let safectx =
        unsafe { CTX.get_or_insert_with(|| clipboard::ClipboardProvider::new().unwrap()) };
    safectx.set_contents("".into()).unwrap();
    let mut trng = rand::thread_rng();

    loop {
        std::thread::sleep(std::time::Duration::from_millis(5));
        //inputbot::KeybdKey::CKey.press();
        std::thread::sleep(std::time::Duration::from_millis(trng.gen_range(4..25)));
        //inputbot::KeybdKey::CKey.release();

        //250 ms total
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            match safectx.get_contents() {
                Ok(s) => {
                    if !s.is_empty() {
                        return s;
                    }
                }
                Err(_) => {}
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(trng.gen_range(1..150)));
    }
}

