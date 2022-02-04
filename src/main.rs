use inputbot::KeybdKey;
use inputbot::MouseButton;

use rand::Rng;
use regex::Regex;

use std::io::{self, BufRead};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Settings {
    pull_delay: u64,
    push_delay: u64,
    div_delay: u64,
    inv_colors: Option<Vec<u32>>,
    screen_height: Option<u32>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct AutoRollMod {
    name: String,
    is_prefix: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AutoRollConfig {
    item_name: String,
    mods: Vec<AutoRollMod>,
    auto_aug_regal: bool,
}

impl AutoRollConfig {
    fn needs_prefix(&self) -> bool {
        self.mods.iter().any(|x| x.is_prefix)
    }

    fn needs_suffix(&self) -> bool {
        self.mods.iter().any(|x| !x.is_prefix)
    }
}

use std::fs;
use std::io::{Read, Write};

static DEFAULT_SETTINGS: Settings = Settings {
    pull_delay: 50,
    push_delay: 40,
    div_delay: 100,
    inv_colors: None,
    screen_height: None,
};

static SETTINGS: Lazy<RwLock<Settings>> = Lazy::new(|| RwLock::new(DEFAULT_SETTINGS.clone()));

static CONFIG_PATH: &str = "./config.json";

fn save_config<T: Serialize>(path: &str, set: &T) -> Result<(), std::io::Error> {
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&set).unwrap().as_bytes())?;

    Ok(())
}

fn load_config<T>(path: &str, default: Option<&T>) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
{
    match fs::File::open(&path) {
        Ok(mut f) => {
            let mut config_text = String::new();
            if let Err(msg) = f.read_to_string(&mut config_text) {
                return Err(format!("Could not read settings: {}", msg));
            }

            let x = serde_json::from_str(&config_text);

            match x {
                Ok(settings) => Ok(settings),
                Err(msg) => Err(format!("Could not parse settings: {}", msg)),
            }
        }
        Err(_f) => match default {
            Some(obj) => match save_config(&path, &obj) {
                Ok(_) => Ok(obj.clone()),
                Err(e) => Err(format!("Could not write defualt settings: {}", e)),
            },
            None => Err(format!("File not found and no default given")),
        },
    }
}

fn main() {
    let mut _rand = rand::thread_rng();
    let set = match load_config(CONFIG_PATH, Some(&DEFAULT_SETTINGS)) {
        Ok(s) => s,
        Err(s) => {
            println!("Config load failed {}", s);
            return;
        }
    };

    *SETTINGS.write().unwrap() = set;

    println!("got config: {:?}", SETTINGS.read().unwrap());
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.get(0).map(|x| &**x) {
        Some("sort") => {
            sort_quad();
            return;
        }
        Some("empty") => {
            empty_inv();
            return;
        }
        Some("roll") => {
            let file = args.get(1).expect("missing name to roll");
            let times = args
                .get(2)
                .expect("missing number of times to roll")
                .parse()
                .expect("invalid number");
            auto_roll(&file, times);
            return;
        }
        Some("reset_inv") => {
            reset_inv_colors();
            return;
        }
        Some("chance") => {
            chance();
            return;
        }
        Some("get") => {
            move_mouse(1920 + 100, 100);
            return;
        }
        Some(n) => {
            println!("Invalid command: {}", n);
            return;
        }

        None => {}
    }

    println!("starting in inputbot mode");

    KeybdKey::HomeKey.bind(move || {
        sort_quad();
    });
    KeybdKey::AKey.bind(move || {
        empty_inv();
    });
    KeybdKey::RControlKey.bind(move || {
        println!("Resetting inv colors");
        reset_inv_colors();
    });
    KeybdKey::F7Key.bind(move || {
        chance();
    });

    let inputs = std::thread::spawn(|| inputbot::handle_input_events());

    let cmdline = std::thread::spawn(move || {
        command_line();
    });

    inputs.join().unwrap();
    cmdline.join().unwrap();
}

fn split_space(input: &str) -> (&str, &str) {
    for (i, c) in input.chars().enumerate() {
        if c == ' ' {
            return (&input[0..i], &input[i + 1..]);
        }
    }
    return (input, "");
}

#[test]
fn test_auto_roll() {
    auto_roll("test.json", 1);
}

use clipboard::ClipboardContext;
use clipboard::ClipboardProvider;

#[derive(Debug)]
struct RollResult {
    has_prefix: bool,
    has_suffix: bool,
    has_mod: bool,
}

fn check_roll(item_text: &str, config: &AutoRollConfig) -> RollResult {
    let maybe_name = item_text
        .lines()
        .filter(|s| s.contains(&config.item_name))
        .nth(0)
        .unwrap();

    RollResult {
        has_prefix: !maybe_name.starts_with(&config.item_name),
        has_suffix: !maybe_name.ends_with(&config.item_name),
        has_mod: config
            .mods
            .iter()
            .map(|x| x.name.as_str())
            .any(|x| item_text.to_lowercase().contains(&x)),
    }
}

fn read_item_on_cursor() -> String {
    static mut ctx: Option<ClipboardContext> = None;

    let safectx =
        unsafe { ctx.get_or_insert_with(|| clipboard::ClipboardProvider::new().unwrap()) };
    safectx.set_contents("".into()).unwrap();

    loop {
        KeybdKey::LControlKey.press();
        std::thread::sleep(std::time::Duration::from_millis(5));
        KeybdKey::CKey.press();
        std::thread::sleep(std::time::Duration::from_millis(5));
        KeybdKey::CKey.release();
        KeybdKey::LControlKey.release();

        match safectx.get_contents() {
            Ok(s) => {
                if s != "" {
                    return s;
                }
            }
            Err(_) => {}
        }
    }
}

fn chance() {
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
}

fn auto_roll(path: &str, times: i64) -> Option<RollResult> {
    #![allow(unused_variables)]
    let alt = (155, 354);
    let aug = (300, 422);
    let reg = (572, 354);
    let slot = (444, 628);

    let config: AutoRollConfig = {
        match load_config(&path, None) {
            Ok(config) => config,
            Err(msg) => {
                println!("{}", msg);
                return None;
            }
        }
    };

    assert!(times > 0);

    let sleep_click = 20;
    let sleep_read = 200;

    let mut i = 0;
    let mut res;
    println!("rolling!");
    click(3, 3);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    loop {
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click_right(alt.0, alt.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_click * 2));
        click(slot.0, slot.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_read));

        println!("alt");
        let item = read_item_on_cursor();
        res = check_roll(&item, &config);
        if res.has_mod {
            println!("got mod");
            break;
        }

        if (!res.has_prefix && config.needs_prefix()) || (!res.has_suffix && config.needs_suffix())
        {
            println!("aug");
            std::thread::sleep(std::time::Duration::from_millis(sleep_click));
            click_right(aug.0, aug.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_click));
            click(slot.0, slot.1);
            std::thread::sleep(std::time::Duration::from_millis(sleep_read));

            res = check_roll(&read_item_on_cursor(), &config);
            if res.has_mod {
                break;
            }
        }

        i += 1;

        if i == times {
            break;
        }

        if KeybdKey::RControlKey.is_pressed() {
            return Some(res);
        }
    }

    if res.has_mod && config.auto_aug_regal {
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click_right(aug.0, aug.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click(slot.0, slot.1);

        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click_right(reg.0, reg.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_click));
        click(slot.0, slot.1);
        std::thread::sleep(std::time::Duration::from_millis(sleep_read));

        res = check_roll(&read_item_on_cursor(), &config);
    }

    Some(res)
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

                match auto_roll(&file, times.parse().unwrap()) {
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

fn click(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    MouseButton::LeftButton.press();
    std::thread::sleep(std::time::Duration::from_millis(10));
    MouseButton::LeftButton.release();
}

fn click_right(x: i32, y: i32) {
    move_mouse(x, y);
    std::thread::sleep(std::time::Duration::from_millis(30));
    MouseButton::RightButton.press();
    std::thread::sleep(std::time::Duration::from_millis(10));
    MouseButton::RightButton.release();
}

fn move_mouse(x: i32, y: i32) {
    inputbot::MouseCursor::move_abs(x + 1920, y);
}

use once_cell::sync::Lazy;
use std::sync::RwLock;

fn reset_inv_colors() {
    let settings = SETTINGS.read().unwrap();
    let height = settings.screen_height.unwrap_or(1080);

    let inv_loc = if height == 1080 {
        (1311, 626)
    } else if height == 1440 {
        (1713, 834)
    } else {
        panic!("invalid screen size");
    };

    let inv_delta = if height == 1080 {
        53
    } else if height == 1440 {
        70
    } else {
        panic!("invalid screen size");
    };

    //click(618, 618);

    let frame = match take_screenshot() {
        Ok(frame) => frame,
        Err(()) => return (),
    };

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

    save_config(CONFIG_PATH, &*settings).unwrap();
}

fn empty_inv_macro(start_slot: u32, delay: u64) {
    let settings = SETTINGS.read().unwrap();
    let height = settings.screen_height.unwrap_or(1080);

    let inv_loc = if height == 1080 {
        (1311, 626)
    } else if height == 1440 {
        (1713, 834)
    } else {
        panic!("invalid screen size");
    };

    let inv_delta = if height == 1080 {
        53
    } else if height == 1440 {
        70
    } else {
        panic!("invalid screen size");
    };

    let frame = match take_screenshot() {
        Ok(frame) => frame,
        Err(()) => return (),
    };

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

                //println!("clicking {} {}", rx, ry);

                click(rx, ry);
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
    }

    //move_mouse(655, 801);
}

fn empty_inv() {
    let delay = { SETTINGS.read().unwrap().push_delay };

    println!("empty inv (delay {})", delay);
    //let slot = if KeybdKey::NumLockKey.is_toggled() { 5 } else { 0 };
    let slot = 0;

    KeybdKey::LControlKey.press();
    empty_inv_macro(slot, delay);
    //std::thread::sleep(std::time::Duration::from_millis(delay * 2));
    //empty_inv_macro(slot, delay);
    KeybdKey::LControlKey.release();
}

struct ScreenshotData {
    height: usize,
    width: usize,
    pixels: Vec<u8>,
}

fn take_screenshot() -> Result<ScreenshotData, ()> {
    let disp = scrap::Display::primary().unwrap();
    let mut cap = scrap::Capturer::new(disp).unwrap();
    let width = cap.width();
    let height = cap.height();

    let sleep = 50;

    //max 2 seconds before fail
    let maxloops = 2000 / sleep;

    for _ in 0..maxloops {
        match cap.frame() {
            Ok(fr) => {
                return Ok(ScreenshotData {
                    height,
                    width,
                    pixels: fr.to_vec(),
                });
            }
            Err(_) => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(sleep));
    }

    Err(())
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

fn sort_quad() {
    let (delay, height) = {
        let settings = SETTINGS.read().unwrap();
        (settings.pull_delay, settings.screen_height.unwrap_or(1080))
    };

    let frame = match take_screenshot() {
        Ok(frame) => frame,
        Err(()) => return (),
    };

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
    println!("help");

    KeybdKey::LControlKey.press();

    let mut movesleft = 25;
    for y in 0..24 {
        let ry = pys[y];

        for x in 0..24 {
            let rx = x * px + left_edge;

            let col1 = frame.get_pixel(rx, ry);
            let col2 = frame.get_pixel(rx + 7, ry);
            let col3 = frame.get_pixel(rx + 15, ry);

            //let select_color = 2008344320;
            let select_color = 2008344575;

            if col1 == select_color || col2 == select_color || col3 == select_color {
                click((rx + 10) as i32, (ry - 10) as i32);
                std::thread::sleep(std::time::Duration::from_millis(delay - 10 + 100));
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

            if movesleft < 1 {
                break;
            }
        }
    }

    KeybdKey::LControlKey.release();

    image::save_buffer(
        "./image.png",
        &frame.pixels,
        frame.width.try_into().unwrap(),
        frame.height.try_into().unwrap(),
        image::ColorType::Rgba8,
    )
    .unwrap();
}
