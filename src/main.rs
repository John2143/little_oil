use anyhow::bail;
use mouse_keyboard_input::{key_codes, Button, VirtualDevice};
//use inputbot::KeybdKey;
use rand::Rng;
use tracing::{debug, info, trace};
use wayland_client::protocol::wl_registry;
use wayland_client::Connection;

use std::io::{self, BufRead, Cursor};
use std::process::Command;

use serde::{Deserialize, Serialize};

mod auto_roll;
mod chaos_recipe;
mod dicts;
pub mod item;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    chaos_recipe_settings: Option<chaos_recipe::ChaosRecipe>,
    pull_delay: u64,
    push_delay: u64,
    div_delay: u64,
    inv_colors: Option<Vec<u32>>,
    screen_height: Option<u32>,
    screenshot_method: ScreenshotMethod,
    pos: InvPositions,
}

impl Settings {
    fn screenshot(&self) -> anyhow::Result<ScreenshotData> {
        match self.screenshot_method {
            ScreenshotMethod::Grim => take_screenshot_grim(),
            ScreenshotMethod::Scrot => take_screenshot_scrap(),
        }
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
    /// Wayland users should use an external program like "grim"
    Grim,
    /// Windows and Linux users can use scrot
    Scrot,
}

use std::fs;
use std::io::{Read, Write};

static DEFAULT_SETTINGS: Settings = Settings {
    chaos_recipe_settings: None,
    pull_delay: 50,
    push_delay: 40,
    div_delay: 100,
    inv_colors: None,
    screen_height: Some(1440),
    screenshot_method: ScreenshotMethod::Scrot,
    pos: InvPositions {
        alt: (149, 368),
        aug: (303, 444),
        scour: (580, 688),
        regal: (579, 365),
        annul: (226, 372),
        transmute: (71, 368),
        inv: (1713, 828),
    }
};

static SETTINGS: Lazy<RwLock<Settings>> = Lazy::new(|| RwLock::new(DEFAULT_SETTINGS.clone()));

static CONFIG_PATH: &str = "/home/john/little_oil/config.json";

pub fn save_config<T: Serialize>(path: &str, set: &T) -> Result<(), std::io::Error> {
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&set).unwrap().as_bytes())?;

    Ok(())
}

fn load_config<T>(path: &str, default: Option<&T>) -> anyhow::Result<T>
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

struct AppData;
impl wayland_client::Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        _state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &wayland_client::QueueHandle<AppData>,
    ) {
        // When receiving events from the wl_registry, we are only interested in the
        // `global` event, which signals a new available global.
        // When receiving this event, we just print its characteristics in this example.
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            println!("[{}] {} (v{})", name, interface, version);
        }
    }
}

fn main() -> anyhow::Result<()> {
    FAKE_DEVICE.lock().unwrap().synchronize();
    tracing_subscriber::fmt::init();
    tracing::info!("Starting main loop");

    // init wayland
    //let conn = Connection::connect_to_env().expect("Wayland not initialized");
    //let display = conn.display();
    //let mut event_queue = conn.new_event_queue();
    //let qh = event_queue.handle();

    //let _registry = display.get_registry(&qh, ());

    //let mut dat = AppData;
    //event_queue.roundtrip(&mut dat);
    //event_queue.blocking_dispatch(&mut dat);

    let mut _rand = rand::thread_rng();
    let set = load_config(CONFIG_PATH, Some(&DEFAULT_SETTINGS))?;

    *SETTINGS.write().unwrap() = set;

    //println!("got config: {:?}", SETTINGS.read().unwrap());
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.get(0).map(|x| &**x) {
        Some("config") => {
            let s = serde_json::to_string(&DEFAULT_SETTINGS).unwrap();
            println!("{}", s);
            return Ok(())
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
            return empty_inv(&SETTINGS.read().unwrap());
        }
        Some("roll") => {
            let file = args.get(1).expect("missing name to roll");
            let times = args
                .get(2)
                .expect("missing number of times to roll")
                .parse()
                .expect("invalid number");

            auto_roll::auto_roll(&SETTINGS.read().unwrap(), &file, times);
            return Ok(());
        }
        Some("reset_inv") => {
            return reset_inv_colors();
        }
        Some("chance") => {
            return chance();
        }
        Some("tally") => {
            let settings = SETTINGS.read().unwrap();
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

            let settings = SETTINGS.read().unwrap();
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

static HELP: &str = r#"
help: Show this menu
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
                        let mut s = SETTINGS.write().unwrap();
                        s.pull_delay = x;
                        save_config(CONFIG_PATH, &*s).unwrap();
                    }
                    Err(_) => println!("could not delay"),
                }
            }
            ("push", rest @ _) => {
                println!("push delay is {}", rest);
                match rest.parse() {
                    Ok(x) => {
                        let mut s = SETTINGS.write().unwrap();
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
                        let mut s = SETTINGS.write().unwrap();
                        s.div_delay = x;
                        //save_config(CONFIG_PATH, &s).unwrap();
                    }
                    Err(_) => println!("could not delay"),
                }
            }
            ("chrome", rest @ _) => {
                let (file, times) = split_space(rest);
                println!("Loading chrome file {}", file);

                match auto_roll::auto_roll(&SETTINGS.read().unwrap(), &file, times.parse().unwrap()) {
                    None => println!("failed to roll"),
                    Some(res) => {
                        println!("{:?}", res);
                    }
                }
            }
            ("mchrome", file @ _) => {
                println!("Making chrome file {}", file);

                save_config(
                    &file,
                    &AutoRollConfig {
                        auto_aug_regal: false,
                        item_name: "Medium Cluster Jewel".to_string(),
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

//thread_local!(static MOUSE: Lazy<mouse_rs::Mouse> = Lazy::new(|| mouse_rs::Mouse::new()));

static FAKE_DEVICE: Lazy<Mutex<VirtualDevice>> =
    Lazy::new(|| Mutex::new(VirtualDevice::default().unwrap()));

fn click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    click_release(key_codes::BTN_LEFT);
}

fn click_right(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    click_release(key_codes::BTN_RIGHT);
}

fn click_release(m: Button) {
    trace!(?m, "click_release");
    let mut device = FAKE_DEVICE.lock().unwrap();

    device.click(m).unwrap();
    //device.synchronize().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
}

fn move_mouse(x: i32, y: i32) {
    trace!(x, y, "mouse_move");
    let mut device = FAKE_DEVICE.lock().unwrap();
    device.move_mouse(-5000, -5000).unwrap();
    device.move_mouse((x as f32 * 1.25) as _, (y as f32 * 1.25) as _).unwrap();
    //device.synchronize().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
}

use once_cell::sync::Lazy;
use std::sync::{Mutex, RwLock};

use crate::auto_roll::AutoRollConfig;
use crate::auto_roll::AutoRollMod;

fn reset_inv_colors() -> anyhow::Result<()> {
    let settings = SETTINGS.read().unwrap();
    let height = settings.screen_height.unwrap_or(1080);

    let inv_loc = settings.pos.inv;

    let inv_delta = if height == 1080 {
        53
    } else if height == 1440 {
        70
    } else if height == 1000 {
        54
    } else {
        panic!("invalid screen size");
    };

    //click(618, 618);

    let frame = settings.screenshot()?;
    drop(settings);

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

    let mut settings = SETTINGS.write().unwrap();

    settings.inv_colors = Some(colors);
    dbg!("ok");

    save_config(CONFIG_PATH, &*settings)?;
    Ok(())
}

fn empty_inv_macro(settings: &Settings, start_slot: u32, delay: u64) -> anyhow::Result<()> {
    let height = settings.screen_height.unwrap_or(1080);

    let inv_loc = settings.pos.inv;
    let inv_delta = if height == 1080 {
        53
    } else if height == 1440 {
        70
    } else if height == 1000 {
        54
    } else {
        panic!("invalid screen size");
    };

    info!(height, x = inv_loc.0, y = inv_loc.1, inv_delta, "Emptying inv");

    let frame = settings.screenshot()?;

    //TODO make it not allocate
    let default_colors = {
        let mut x = vec![0; 60];
        x.resize(60, 0);
        x
    };

    let inv_color = settings.inv_colors.as_ref().unwrap_or(&default_colors);

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

                click(rx, ry);
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        //return Ok(());
    }

    Ok(())
    //move_mouse(655, 801);
}

fn empty_inv(settings: &Settings) -> anyhow::Result<()> {
    println!("empty inv (delay {})", settings.push_delay);
    //let slot = if KeybdKey::NumLockKey.is_toggled() { 5 } else { 0 };
    let slot = 0;

    std::thread::sleep(std::time::Duration::from_millis(500));
    return empty_inv_macro(&settings, slot, settings.push_delay);
    //empty_inv_macro(slot, delay);
}

pub struct ScreenshotData {
    height: usize,
    width: usize,
    pixels: Vec<u8>,
}

pub fn take_screenshot_grim() -> anyhow::Result<ScreenshotData> {
    let cmd = Command::new("grim")
        // whole left screen
        .arg("-g")
        .arg("0,0 2560x1440")
        // png out
        .arg("-t")
        .arg("ppm")
        .arg("-")
        .output()
        .unwrap();

    // for .seek()
    let stdout = Cursor::new(cmd.stdout);
    // the output format ppm "portable pixel map" from grim is called
    // pnm "portable any map" in the image crate.
    let img = image::load(stdout, image::ImageFormat::Pnm).unwrap();

    //let path = Path::new("./last_screnshot.png");
    //info!(path = ?path.canonicalize().unwrap(), "saving screenshot");
    //img.save(path).unwrap();

    Ok(ScreenshotData {
        height: img.height() as usize,
        width: img.width() as usize,
        pixels: img.to_rgba8().to_vec(),
    })
}

pub fn take_screenshot_scrap() -> anyhow::Result<ScreenshotData> {
    println!("taking screenshot...");
    let disp = scrap::Display::primary().unwrap();
    //let disps = scrap::Display::all().unwrap();
    let mut cap = scrap::Capturer::new(disp).unwrap();
    //for disp in disps.into_iter().skip(2) {
    //cap = scrap::Capturer::new(disp).unwrap();
    //println!("doing cap");
    //break;
    //}

    let width = cap.width();
    let height = cap.height();

    let sleep = 50;

    //max 2 seconds before fail
    let maxloops = 2000 / sleep;

    println!("trying to screenshot...");

    for _ in 0..maxloops {
        match cap.frame() {
            Ok(fr) => {
                println!("got screenshot");
                return Ok(ScreenshotData {
                    height,
                    width,
                    pixels: fr.to_vec(),
                });
            }
            Err(e) => {
                println!("screenshot failed... {}", e);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(sleep));
    }

    bail!("was not able to take screenshot after {maxloops} tries");
}

impl ScreenshotData {
    //return RGBA8888 pixel as u32
    fn get_pixel(&self, x: usize, y: usize) -> u32 {
        assert!(x < self.width);
        assert!(y < self.height);

        let pos: usize = y * self.width + x;
        let pos = pos * 4; //pixel format ARGB8888;

        //TODO find the rust idiomatic way to do this
        unsafe {
            std::mem::transmute([
                self.pixels[pos + 3],
                self.pixels[pos + 2],
                self.pixels[pos + 1],
                self.pixels[pos],
            ])
        }
    }
}

fn sort_quad(times: u32) -> anyhow::Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let settings = SETTINGS.read().unwrap();
    let (delay, height) = { (settings.pull_delay, settings.screen_height.unwrap_or(1080)) };

    let frame = settings.screenshot()?;

    drop(settings);

    println!("take tab (delay {})", delay);

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
    //.unwrap();
}
