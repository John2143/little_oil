use anyhow::{bail, Context};
use clap::Parser;
use mouse::{click, click_right};
//use inputbot::KeybdKey;
use rand::Rng;
use screenshot::ScreenshotData;
use tracing::{debug, info, trace};

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod auto_roll;
mod chaos_recipe;
mod dicts;
pub mod item;
mod screenshot;

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

/// A Rectangle defined by its top left corner, width and height.
/// From the image crate
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
    None
}

impl Settings {
    fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        trace!(?self.screenshot_method, "Taking a screenshot");
        match self.screenshot_method {
            #[cfg(feature = "input_wayland")]
            ScreenshotMethod::Grim => screenshot::take_screenshot_grim(&self),
            #[cfg(feature = "input_x")]
            ScreenshotMethod::Scrot => screenshot::take_screenshot_scrap(&self),
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
            return 53;
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
    Scrot,
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
    /// Then, running the [`Empty`] command will clear out any inventory slots with items in it.
    ResetInv,

    /// [old] Roll an item in your currency tab to get specific stats.
    Roll { times: usize, config: PathBuf },
    /// [old] chance an item into a unique
    Chance,

    /// [old] Count how many chaos items are in a tab.
    Tally,

    /// [old] Do the chaos recipe?
    Chaos,
}

impl CliCommand {
    fn run(&self, settings: &Settings) -> anyhow::Result<()> {
        match self {
            CliCommand::PrintConfig => {
                let s = serde_json::to_string(&DEFAULT_SETTINGS).unwrap();
                println!("{}", s);
                return Ok(());
            }
            CliCommand::Sort { times } => {
                return sort_quad(&settings, *times);
            }
            CliCommand::Empty => {
                return empty_inv(&settings);
            }
            CliCommand::Roll {
                times,
                config: target_item_config,
            } => {
                let ar_config: auto_roll::AutoRollConfig =
                    load_config(target_item_config, None).context("Loading auto_roll_config")?;

                let roll_res = auto_roll::auto_roll(&settings, &ar_config, *times);
                info!(?roll_res);
                return Ok(());
            }
            CliCommand::ResetInv => {
                return reset_inv_colors(&settings);
            }
            CliCommand::Chance => {
                return chance();
            }
            CliCommand::Tally => {
                let c = match settings.chaos_recipe_settings.clone() {
                    Some(s) => s,
                    None => bail!("No chaos recipe config found"),
                };

                chaos_recipe::get_tally(&c);
                return Ok(());
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
                return Ok(());
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
                    if s != "" {
                        return s;
                    }
                }
                Err(_) => {}
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(trng.gen_range(1..150)));
    }
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

#[cfg(feature = "input_x")]
mod mouse {
    use tracing::trace;
    use once_cell::sync::Lazy;
    use mouse_rs::types::keys::Keys;

    thread_local! {
        static FAKE_DEVICE: Lazy<mouse_rs::Mouse> = Lazy::new(|| mouse_rs::Mouse::new());
    }

    pub fn init() {
    }

    pub fn click(x: i32, y: i32) {
        move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        click_release(&Keys::RIGHT);
    }

    pub fn click_right(x: i32, y: i32) {
        move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(30));
        click_release(&Keys::LEFT);
    }

    pub fn click_release(key: &Keys) {
        FAKE_DEVICE.with(|mouse| {
            mouse.press(key);
        });
        std::thread::sleep(std::time::Duration::from_millis(10));

        FAKE_DEVICE.with(|mouse| {
            mouse.release(key);
        });
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    pub fn move_mouse(x: i32, y: i32) {
        trace!(x, y, "mouse_move");
        FAKE_DEVICE.with(|mouse| {
            mouse.move_to(x, y)
        });

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

//#[cfg(feature = "input_wayland")]
//mod mouse {
    //use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
    //use once_cell::sync::Lazy;
    //use tracing::trace;

    //use std::sync::Mutex;

    //pub fn init() {
        //FAKE_DEVICE.lock().unwrap().synchronize().unwrap();
    //}

    //static FAKE_DEVICE: Lazy<Mutex<VirtualDevice>> =
        //Lazy::new(|| Mutex::new(VirtualDevice::default().unwrap()));

    //pub fn click(x: i32, y: i32) {
        //move_mouse(x, y);
        //std::thread::sleep(std::time::Duration::from_millis(30));
        //click_release(key_codes::BTN_LEFT);
    //}

    //pub fn click_right(x: i32, y: i32) {
        //move_mouse(x, y);
        //std::thread::sleep(std::time::Duration::from_millis(30));
        //click_release(key_codes::BTN_RIGHT);
    //}

    //pub fn click_release(m: Button) {
        //trace!(?m, "click_release");
        //let mut device = FAKE_DEVICE.lock().unwrap();

        //device.click(m).unwrap();
        ////device.synchronize().unwrap();
        //std::thread::sleep(std::time::Duration::from_millis(10));
    //}

    //pub fn move_mouse(x: i32, y: i32) {
        //trace!(x, y, "mouse_move");
        //let mut device = FAKE_DEVICE.lock().unwrap();
        //device.move_mouse(-5000, -5000).unwrap();
        //device
            //.move_mouse((x as f32 * 1.25) as _, (y as f32 * 1.25) as _)
            //.unwrap();
        ////device.synchronize().unwrap();
        //std::thread::sleep(std::time::Duration::from_millis(10));
    //}
//}

use once_cell::sync::Lazy;
use std::sync::RwLock;

fn reset_inv_colors(settings: &Settings) -> anyhow::Result<()> {
    let inv_loc = settings.pos.inv;
    let inv_delta = settings.inv_delta();
    let frame = settings.screenshot()?;

    let mut colors = vec![0; 60];

    for x in 0..12 {
        for y in 0..5 {
            let mousex = x * inv_delta + inv_loc.0;
            let mousey = y * inv_delta + inv_loc.1;
            let color = frame.get_pixel(mousex as usize, mousey as usize);

            colors[(x * 5 + y) as usize] = color;
        }
    }

    let mut settings = SETTINGS.write().unwrap();

    settings.inv_colors = Some(colors);
    dbg!("ok");

    save_config(get_config_path(), &*settings)?;
    Ok(())
}

fn empty_inv_macro(settings: &Settings, start_slot: u32, delay: u64) -> anyhow::Result<()> {
    let height = settings.poe_window_location.height;

    let inv_loc = settings.pos.inv;
    let inv_delta = settings.inv_delta();
    let frame = settings.screenshot()?;

    let inv_color = settings
        .inv_colors
        .as_ref()
        .context("inv_colors not set: consider running `reset_inv_colors`.")?;

    info!(
        height,
        x = inv_loc.0,
        y = inv_loc.1,
        inv_delta,
        "Emptying inv"
    );
    for x in (start_slot / 5)..12 {
        for y in (start_slot % 5)..5 {
            let mousex = x * inv_delta + inv_loc.0;
            let mousey = y * inv_delta + inv_loc.1;
            let color = frame.get_pixel(mousex as usize, mousey as usize);
            //println!("{},", color);
            let is_right_color = color == inv_color[(x * 5 + y) as usize];
            //println!("{} {} {} {}", x, y, color, is_right_color);

            if !is_right_color {
                let (rx, ry) = (
                    (x * inv_delta + inv_loc.0) as i32,
                    (y * inv_delta + inv_loc.1) as i32,
                );

                debug!(x, y, "clicking inv");

                mouse::click(rx, ry);
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        //return Ok(());
    }

    Ok(())
    //move_mouse(655, 801);
}

fn empty_inv(settings: &Settings) -> anyhow::Result<()> {
    info!(settings.push_delay, "empty inv");
    //let slot = if KeybdKey::NumLockKey.is_toggled() { 5 } else { 0 };
    let slot = 0;

    std::thread::sleep(std::time::Duration::from_millis(500));
    return empty_inv_macro(&settings, slot, settings.push_delay);
    //empty_inv_macro(slot, delay);
}

fn sort_quad(settings: &Settings, times: usize) -> anyhow::Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let (delay, height) = { (settings.pull_delay, settings.poe_window_location.height) };

    let frame = settings.screenshot()?;

    info!(delay, "sort_quad");

    //let px: f64 = (625f64 - 17f64) / 23f64;
    //let pys = [
    //160, 186, 212, 239, 265, 291, 318, 344, 370, 397, 423, 449, 476, 502, 528, 555, 581, 607,
    //634, 660, 686, 712, 739, 765, //792,
    //];
    let left_edge = if height == 1080 {
    21
    } else if height == 1440 {
    29
    } else {
    panic!("invalid screen size");
    };

    let px = if height == 1080 {
    (2573 - 1920 - 15) / 24
    } else if height == 1440 {
    830 - 795
    } else {
    panic!("invalid screen size");
    };

    let pys = if height == 1080 {
    [
        160, 186, 212, 239, 265, 291, 318, 344, 370, 397, 423, 449, 476, 502, 528, 555, 581,
        607, 634, 660, 686, 712, 739, 765, //792,
    ]
    } else if height == 1440 {
    [
        260, 295, 330, 365, 400, 436, 471, 506, 541, 576, 611, 646, 681, 716, 751, 787, 822,
        857, 892, 927, 962, 997, 1032, 1067,
    ]
    } else {
    panic!("invalid screen size");
    };

    //160, 186, 212, 239, 265, 291, 318, 344, 370, 397, 423, 449, 476, 502, 528, 555, 581, 607,
    //634, 660, 686, 712, 739, 765, //792,
    //];

    let mut movesleft = times;
    for y in 0..24 {
    let ry = pys[y];

    for x in 0..24 {
        if movesleft < 1 {
            break;
        }

        let rx = x * px + left_edge;

        let col1 = frame.get_pixel(rx, ry);
        let col2 = frame.get_pixel(rx + 7, ry);
        let col3 = frame.get_pixel(rx + 15, ry);

        //let select_color = 2008344320;
        //let select_color = 2008344575;
        let select_color = 3887364095;
        debug!(x, y, "pixels");
        trace!(col1, col2, col3, select_color);

        if col1 == select_color || col2 == select_color || col3 == select_color {
            click((rx + 10) as i32, (ry - 10) as i32);
            std::thread::sleep(std::time::Duration::from_millis(delay - 10));
            movesleft -= 1;
        };

        //if(slotIsSelected(img, rx, ry) || slotIsSelected(img, rx + 15, ry)){
        //img.setPixelColor(Jimp.cssColorToHex("#FF0000"), rx + 1, ry);
        //await stash.click([rx + 10, ry - 10]);
        //await robot.moveMouse(654, 801);
        //await sleep(delays.grabTab);
        //movesleft--;
        //}
        //img.setPixelColor(Jimp.cssColorToHex("#FFFFFF"), rx, ry);
    }
    }

    Ok(())
    //use std::convert::TryInto;
    //image::save_buffer(
    //"./image2.png",
    //&frame.pixels,
    //frame.width.try_into().unwrap(),
    //frame.height.try_into().unwrap(),
    //image::ColorType::Rgba8,
    //)
}
