use anyhow::bail;
use mouse_keyboard_input::key_codes;
//use inputbot::KeybdKey;
use tracing::{debug, info, trace};
use parking_lot::RwLock;
use std::sync::LazyLock;

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::process::Command;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use device::{click, click_right, ctrl_click, FAKE_DEVICE};
use crate::auto_roll::AutoRollConfig;
use crate::auto_roll::AutoRollMod;
use screenshot::ScreenshotData;
use platform::Platform;

mod auto_roll;
mod chaos_recipe;
mod dicts;
pub mod item;
mod device;
mod screenshot;
mod platform;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    chaos_recipe_settings: Option<chaos_recipe::ChaosRecipe>,
    pull_delay: u64,
    push_delay: u64,
    div_delay: u64,
    inv_colors: Option<Vec<u32>>,
    screen_height: Option<u32>,
    pos: InvPositions,
    pub platform: Option<Platform>,
    pub inv_region: Option<ScreenRegion>,
    pub stash_region: Option<ScreenRegion>,
    pub game_window_region: Option<ScreenRegion>,
}

impl Settings {
    fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        let platform = self.platform.unwrap_or_else(Platform::detect);
        platform.screenshot(self)
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct ScreenRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}



static DEFAULT_SETTINGS: Settings = Settings {
    chaos_recipe_settings: None,
    pull_delay: 50,
    push_delay: 40,
    div_delay: 100,
    inv_colors: None,
    screen_height: Some(1440),
    pos: InvPositions {
        alt: (149, 368),
        aug: (303, 444),
        scour: (580, 688),
        regal: (579, 365),
        annul: (226, 372),
        transmute: (71, 368),
        inv: (1713, 828),
    },
    platform: None,
    inv_region: None,
    stash_region: None,
    game_window_region: None,
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

    // init wayland
    //let conn = Connection::connect_to_env().expect("Wayland not initialized");
    //let display = conn.display();
    //let mut event_queue = conn.new_event_queue();
    //let qh = event_queue.handle();

    //let _registry = display.get_registry(&qh, ());

    //let mut dat = AppData;
    //event_queue.roundtrip(&mut dat);
    //event_queue.blocking_dispatch(&mut dat);

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
            let s = serde_json::to_string(&DEFAULT_SETTINGS).unwrap();
            println!("{}", s);
            return Ok(())
        }
        Some("version") | Some("--version") => {
            println!("little_oil v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("sort") => {
            dbg!(&args);
            let times = args
                .get(1)
                .map(|x| x.parse())
                .unwrap_or(Ok(40))
                .expect("invalid number");

            return sort_quad(times);
        }
        Some("empty") => {
            return empty_inv(&SETTINGS.read());
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

    println!("starting in inputbot mode");

    //KeybdKey::HomeKey.bind(move || {
    //sort_quad(40);
    //});
    //KeybdKey::AKey.bind(move || {
    //empty_inv();
    //});

    //KeybdKey::F7Key.bind(move || {
    //chance();
    //});

    //let inputs = std::thread::spawn(|| inputbot::handle_input_events());

    let cmdline = std::thread::spawn(move || {
        command_line();
    });

    //inputs.join().unwrap();
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

fn read_item_on_cursor() -> String {
    use wl_clipboard_rs::utils::{is_primary_selection_supported, PrimarySelectionCheckError};

    match is_primary_selection_supported() {
        Ok(_supported) => {
            // We have our definitive result. False means that ext/wlr-data-control is present
            // and did not signal the primary selection support, or that only wlr-data-control
            // version 1 is present (which does not support primary selection).
            //println!("primary selection supported: {}", supported);
        },
        Err(PrimarySelectionCheckError::NoSeats) => {
            // Impossible to give a definitive result. Primary selection may or may not be
            // supported.

            // The required protocol (ext-data-control, or wlr-data-control version 2) is there,
            // but there are no seats. Unfortunately, at least one seat is needed to check for the
            // primary clipboard support.
            println!("no seats, cannot check for primary selection support");
            return String::new();
        },
        Err(PrimarySelectionCheckError::MissingProtocol) => {
            // The data-control protocol (required for wl-clipboard-rs operation) is not
            // supported by the compositor.
            println!("data-control protocol not supported");
            return String::new();
        },
        Err(e) => {
            println!("error checking for primary selection support: {:?}", e);
            return String::new();
            // Some communication error occurred.
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
                        return clip_res.to_string();
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
            panic!("could not read item on cursor");
        }

        std::thread::sleep(std::time::Duration::from_millis(rand::random_range(1..150)));
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

static HELP: &str = r#"
help: Show this menu
version: Print version and exit
set-region <inventory|stash|window>: Select and save a screen region
pull <delay>: Change delay for pulling out of quad tab
push <delay>: Change delay for pushing into tab/trade
div <delay>: Change delay for div macro
chrome <file> <times>: Open a autoroll file, with name <file>, and roll item <times>
mchrome <file>: Create example chrome file with name <file>. To be used with chrome later.

Press Home to pull from tab
Press Insert to push into inv
Press F7 to use chance macro

Press CTRL + C to quit this program.
"#;

fn command_line() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match split_space(&line.unwrap()) {
            //TODO find rusty way to do this DRY
            ("pull", rest @ _) => {
                println!("pull delay is {}", rest);
                match rest.parse() {
                    Ok(x) => {
                        let mut s = SETTINGS.write();
                        s.pull_delay = x;
                        save_config(&config_path(), &*s).unwrap();
                    }
                    Err(_) => println!("could not delay"),
                }
            }
            ("push", rest @ _) => {
                println!("push delay is {}", rest);
                match rest.parse() {
                    Ok(x) => {
                        let mut s = SETTINGS.write();
                        s.push_delay = x;
                        //save_config(CONFIG_PATH, &s).unwrap();
                    }
                    Err(_) => println!("could not delay"),
                }
            }
            ("div", rest @ _) => {
                println!("div delay is {}", rest);
                match rest.parse() {
                    Ok(x) => {
                        let mut s = SETTINGS.write();
                        s.div_delay = x;
                        //save_config(CONFIG_PATH, &s).unwrap();
                    }
                    Err(_) => println!("could not delay"),
                }
            }
            ("chrome", rest @ _) => {
                let (file, times) = split_space(rest);
                println!("Loading chrome file {}", file);

                match auto_roll::auto_roll(&file, times.parse().unwrap()) {
                    None => println!("failed to roll"),
                    Some(res) => {
                        println!("{:?}", res);
                    }
                }
            }
            ("mchrome", file @ _) => {
                println!("Making chrome file {}", file);

                save_config(
                    std::path::Path::new(file),
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
            ("help", _) => {
                println!("Available Commands: {}", HELP);
            }
            (_, _) => println!("Unknown command"),
        }
    }
}


fn reset_inv_colors() -> anyhow::Result<()> {
    let settings = SETTINGS.read();
    let inv_region = settings
        .inv_region
        .expect("Inventory region not calibrated — run: little_oil set-region inventory");
    let inv_delta = inv_region.width / 12;
    let inv_loc = (inv_region.x, inv_region.y);

    let frame = settings.screenshot()?;
    drop(settings);

    //click(618, 618);


    let mut colors = Vec::with_capacity(60);
    colors.resize(60, 0);

    for x in 0..12 {
        for y in 0..5 {
            let mousex = x * inv_delta + inv_loc.0;
            let mousey = y * inv_delta + inv_loc.1;
            let color = frame.get_pixel(mousex as usize, mousey as usize);

            colors[(x * 5 + y) as usize] = color;
        }
    }

    let mut settings = SETTINGS.write();

    let note = Command::new("notify-send")
        .args(["-u", "low", "Little Oil", &format!("Inventory colors calibrated: {} slots", colors.len())])
        .spawn();
    if let Err(e) = note {
        eprintln!("notify-send failed: {e}");
    }
    settings.inv_colors = Some(colors);

    save_config(&config_path(), &*settings)?;
    Ok(())
}

fn empty_inv_macro(settings: &Settings, start_slot: u32, delay: u64) -> anyhow::Result<u32> {
    let inv_region = settings
        .inv_region
        .expect("Inventory region not calibrated — run: little_oil set-region inventory");
    let inv_delta = inv_region.width / 12;
    let inv_loc = (inv_region.x, inv_region.y);

    info!(inv_delta, x = inv_loc.0, y = inv_loc.1, "Emptying inv");

    let frame = settings.screenshot()?;

    //TODO make it not allocate
    let default_colors = {
        let mut x = vec![0; 60];
        x.resize(60, 0);
        x
    };

    let inv_color = settings.inv_colors.as_ref().unwrap_or(&default_colors);
    let mut clicked: u32 = 0;

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

                ctrl_click(rx, ry);
                clicked += 1;
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        //return Ok(());
    }

    Ok(clicked)
    //move_mouse(655, 801);
}

fn empty_inv(settings: &Settings) -> anyhow::Result<()> {
    let slot = 0;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let clicked = empty_inv_macro(&settings, slot, settings.push_delay)?;
    let note = Command::new("notify-send")
        .args(["-u", "low", "Little Oil", &format!("Inventory cleared: {} items moved", clicked)])
        .spawn();
    if let Err(e) = note {
        eprintln!("notify-send failed: {e}");
    }
    Ok(())
}


fn sort_quad(times: u32) -> anyhow::Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let settings = SETTINGS.read();
    let delay = settings.pull_delay;

    let game_window = settings.game_window_region.clone();
    let screen_height = settings.screen_height;
    let frame = settings.screenshot()?;
    drop(settings);

    let (left_edge, px, pys) = if let Some(win) = &game_window {
        // Grid is 24x24 in the quad tab. Derive from window dimensions.
        let margin = win.width / 72;
        let cell_w = (win.width - margin * 2) / 24;
        let cell_h = (win.height as f64 * 0.75) as u32 / 24;
        let py_base = (win.y + win.height / 4) as usize;

        let mut pys_arr = [0usize; 24];
        for (i, entry) in pys_arr.iter_mut().enumerate() {
            *entry = py_base + i * cell_h as usize;
        }
        (margin as usize, cell_w as usize, pys_arr)
    } else {
        // FALLBACK: existing hardcoded math (keeps working without calibration)
        let height = screen_height.unwrap_or(1080);
        let left_edge_v = if height == 1080 { 21usize } else if height == 1440 { 29 } else { panic!("invalid screen size") };
        let px_v = if height == 1080 { ((2573 - 1920 - 15) / 24) as usize } else if height == 1440 { (830 - 795) as usize } else { panic!("invalid screen size") };
        let pys_v = if height == 1080 {
            [160usize, 186, 212, 239, 265, 291, 318, 344, 370, 397, 423, 449, 476, 502, 528, 555, 581, 607, 634, 660, 686, 712, 739, 765]
        } else if height == 1440 {
            [260, 295, 330, 365, 400, 436, 471, 506, 541, 576, 611, 646, 681, 716, 751, 787, 822, 857, 892, 927, 962, 997, 1032, 1067]
        } else {
            panic!("invalid screen size");
        };
        (left_edge_v, px_v, pys_v)
    };

    println!("take tab (delay {})", delay);

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
    //.unwrap();
}
