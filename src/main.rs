use image::math::Rect;
use mouse_keyboard_input::{Button, VirtualDevice, key_codes};
use std::io::Read;
use tracing::{debug, trace};

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

mod auto_roll;
//mod chaos_recipe;
//mod dicts;
pub mod item;

#[derive(Debug, Clone)]
pub struct Settings {
    delay: u64,
    /// The filepath where your inventory color data is saved
    inv_colors_location: PathBuf,
    screenshot_method: ScreenshotMethod,
    screen_location: Rect,
    inventory_pos: Rect,
}

impl Default for Settings {
    fn default() -> Self {
        // /tmp/little_oil_inv_colors.json
        let persistent_path = std::env::temp_dir().join("little_oil_inv_colors.json");
        Self {
            delay: 50,
            inv_colors_location: persistent_path,
            screenshot_method: ScreenshotMethod::Grim,
            screen_location: Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            inventory_pos: Rect {
                x: 1683,
                y: 772,
                width: 870,
                height: 374,
            },
        }
    }
}

impl Settings {
    fn screenshot(&self, rectangle: &Rect) -> anyhow::Result<ScreenshotData> {
        match self.screenshot_method {
            ScreenshotMethod::Grim => take_screenshot_grim(rectangle),
        }
    }

    /// Must pass in array of length 60, containing the color value of each inventory slot
    fn save_inv_colors(&self, colors: &[u32]) -> anyhow::Result<()> {
        if colors.len() != 60 {
            anyhow::bail!("invalid inv colors length: must be exactly 60");
        }
        let json = serde_json::to_string(colors)?;
        std::fs::write(&self.inv_colors_location, json)?;
        Ok(())
    }

    fn load_inv_colors(&self) -> anyhow::Result<[u32; 60]> {
        let json = std::fs::read_to_string(&self.inv_colors_location)?;
        let vec: Vec<u32> = serde_json::from_str(&json)?;
        if vec.len() != 60 {
            anyhow::bail!("invalid inv colors file");
        }
        let mut arr = [0; 60];
        arr.copy_from_slice(vec.as_slice());
        Ok(arr)
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
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting main loop");

    // plug in our fake input device
    let _ = FAKE_DEVICE.lock().unwrap().synchronize();

    // init wayland
    //let conn = Connection::connect_to_env().expect("Wayland not initialized");
    //let display = conn.display();
    //let mut event_queue = conn.new_event_queue();
    //let qh = event_queue.handle();

    //let _registry = display.get_registry(&qh, ());

    //let mut dat = AppData;
    //event_queue.roundtrip(&mut dat);
    //event_queue.blocking_dispatch(&mut dat);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings = Settings::default();

    match args.get(0).map(|x| &**x) {
        Some("sort") => {
            dbg!(&args);
            let times = args
                .get(1)
                .map(|x| x.parse())
                .unwrap_or(Ok(40))
                .expect("invalid number");

            return sort_quad(&settings, times);
        }
        Some("empty") => {
            return empty_inv(&settings);
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
            return reset_inv_colors(&settings);
        }

        Some(n) => {
            println!("Invalid command: {}", n);
            return Ok(());
        }

        None => {}
    }

    Ok(())
}

fn read_item_on_cursor() -> String {
    use wl_clipboard_rs::utils::{PrimarySelectionCheckError, is_primary_selection_supported};

    match is_primary_selection_supported() {
        Ok(_supported) => {
            // We have our definitive result. False means that ext/wlr-data-control is present
            // and did not signal the primary selection support, or that only wlr-data-control
            // version 1 is present (which does not support primary selection).
            //println!("primary selection supported: {}", supported);
        }
        Err(PrimarySelectionCheckError::NoSeats) => {
            // Impossible to give a definitive result. Primary selection may or may not be
            // supported.

            // The required protocol (ext-data-control, or wlr-data-control version 2) is there,
            // but there are no seats. Unfortunately, at least one seat is needed to check for the
            // primary clipboard support.
            println!("no seats, cannot check for primary selection support");
            return String::new();
        }
        Err(PrimarySelectionCheckError::MissingProtocol) => {
            // The data-control protocol (required for wl-clipboard-rs operation) is not
            // supported by the compositor.
            println!("data-control protocol not supported");
            return String::new();
        }
        Err(e) => {
            println!("error checking for primary selection support: {:?}", e);
            return String::new();
            // Some communication error occurred.
        }
    }

    // clear the clipboard
    {
        use wl_clipboard_rs::copy::{MimeType, Options, Source, copy};

        let opts = Options::new();
        copy(opts, Source::Bytes([].into()), MimeType::Autodetect).unwrap();
    }

    let mut i = 0;
    loop {
        {
            let mut device = FAKE_DEVICE.lock().unwrap();
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

            use wl_clipboard_rs::paste::{ClipboardType, Error, MimeType, Seat, get_contents};

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

fn load_config<T>(path: &str, default: Option<&T>) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
{
    use anyhow::bail;
    use std::fs;
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
            Some(obj) => Ok(obj.clone()),
            None => bail!("File not found and no default given"),
        },
    }
}

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

pub fn move_mouse(x: i32, y: i32) {
    trace!(x, y, "mouse_move");
    let mut device = FAKE_DEVICE.lock().unwrap();
    device.move_mouse(-5000, -5000).unwrap();
    device
        .move_mouse((x as f32 * 1.25) as _, (y as f32 * 1.25) as _)
        .unwrap();
    //device.synchronize().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
}

use once_cell::sync::Lazy;
use std::sync::Mutex;

fn xy_inv_to_screen(settings: &Settings, x: u32, y: u32) -> (i32, i32) {
    let inv_loc = &settings.inventory_pos;

    let inv_delta = inv_loc.height / 5;

    let mousex = x * inv_delta + inv_loc.x + inv_delta / 2;
    let mousey = y * inv_delta + inv_loc.y + inv_delta / 2;

    (mousex as i32, mousey as i32)
}

fn reset_inv_colors(settings: &Settings) -> anyhow::Result<()> {
    let frame = settings.screenshot(&settings.inventory_pos)?;

    let mut colors = Vec::with_capacity(60);
    colors.resize(60, 0);

    for x in 0..12 {
        for y in 0..5 {
            let (mousex, mousey) = xy_inv_to_screen(settings, x, y);
            let color = frame.get_pixel(mousex as usize, mousey as usize);
            move_mouse(mousex as i32, mousey as i32);

            colors[(x * 5 + y) as usize] = color;
        }
    }

    settings.save_inv_colors(&colors)?;

    Ok(())
}

fn empty_inv_macro(settings: &Settings, start_slot: u32, delay: u64) -> anyhow::Result<()> {
    let frame = settings.screenshot(&settings.inventory_pos)?;

    let our_colors = settings.load_inv_colors()?;

    for x in (start_slot / 5)..12 {
        for y in (start_slot % 5)..5 {
            let (mousex, mousey) = xy_inv_to_screen(settings, x, y);
            let color = frame.get_pixel(mousex as usize, mousey as usize);
            let is_right_color = color == our_colors[(x * 5 + y) as usize];

            if !is_right_color {
                debug!(mousex, mousey, "clicking inv");

                click(mousex, mousey);
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
        //return Ok(());
    }

    Ok(())
    //move_mouse(655, 801);
}

fn empty_inv(settings: &Settings) -> anyhow::Result<()> {
    println!("empty inv (delay {})", settings.delay);
    //let slot = if KeybdKey::NumLockKey.is_toggled() { 5 } else { 0 };
    let slot = 0;

    std::thread::sleep(std::time::Duration::from_millis(500));
    return empty_inv_macro(&settings, slot, settings.delay);
    //empty_inv_macro(slot, delay);
}

pub struct ScreenshotData {
    height: usize,
    width: usize,
    pixels: Vec<u8>,
}

pub fn take_screenshot_grim(rectangle: &Rect) -> anyhow::Result<ScreenshotData> {
    let x = rectangle.x;
    let y = rectangle.y;
    let w = rectangle.width;
    let h = rectangle.height;
    let cmd = Command::new("grim")
        // whole left screen
        .arg("-g")
        //.arg("0,0 2560x1440")
        .arg(format!("{x},{y} {w}x{h}"))
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

//pub fn take_screenshot_scrap() -> anyhow::Result<ScreenshotData> {
//println!("taking screenshot...");
//let disp = scrap::Display::primary().unwrap();
////let disps = scrap::Display::all().unwrap();
//let mut cap = scrap::Capturer::new(disp).unwrap();
////for disp in disps.into_iter().skip(2) {
////cap = scrap::Capturer::new(disp).unwrap();
////println!("doing cap");
////break;
////}

//let width = cap.width();
//let height = cap.height();

//let sleep = 50;

////max 2 seconds before fail
//let maxloops = 2000 / sleep;

//println!("trying to screenshot...");

//for _ in 0..maxloops {
//match cap.frame() {
//Ok(fr) => {
//println!("got screenshot");
//return Ok(ScreenshotData {
//height,
//width,
//pixels: fr.to_vec(),
//});
//}
//Err(e) => {
//println!("screenshot failed... {}", e);
//}
//}
//std::thread::sleep(std::time::Duration::from_millis(sleep));
//}

//bail!("was not able to take screenshot after {maxloops} tries");
//}

impl ScreenshotData {
    //return RGBA8888 pixel as u32
    fn get_pixel(&self, x: usize, y: usize) -> u32 {
        assert!(x < self.width);
        assert!(y < self.height);

        let pos: usize = y * self.width + x;
        let pos = pos * 4; //pixel format ARGB8888;

        u32::from_ne_bytes([
            self.pixels[pos + 3],
            self.pixels[pos + 2],
            self.pixels[pos + 1],
            self.pixels[pos],
        ])
    }
}

fn sort_quad(settings: &Settings, times: u32) -> anyhow::Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let frame = settings.screenshot(&settings.screen_location)?;
    let delay = settings.delay;

    println!("take tab (delay {})", delay);
    let height = settings.screen_location.height;

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
    for (y, &ry) in pys.iter().enumerate() {
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
