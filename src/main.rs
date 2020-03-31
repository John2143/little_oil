//extern crate image;
extern crate scrap;
extern crate inputbot;

fn get_pixel_color_frame(frame: &scrap::Frame, x: usize, y: usize, width: usize) -> u32 {
    let pos: usize = y * width + x;
    let pos = pos * 4; //pixel format ARGB8888;

    //TODO find the rust idiomatic way to do this
    unsafe {
        std::mem::transmute::<[u8; 4], u32>(
            [
                frame[pos],
                frame[pos + 1],
                frame[pos + 2],
                frame[pos + 3],
            ]
        )
    }
}

use inputbot::KeybdKey;
use inputbot::MouseButton;

use std::io::{self, BufRead};
use std::sync;

struct Settings {
    pull_delay: u64,
    push_delay: u64,
    div_delay: u64,
}

fn split_space(input: &str) -> (&str, &str){
    for (i, c) in input.chars().enumerate() {
        if c == ' ' {
            return (&input[0..i], &input[i + 1..])
        }
    }
    return (input, "")
}

fn main() {
    let set = Settings{
        pull_delay: 50,
        push_delay: 40,
        div_delay: 100,
    };

    let settings_mutex = sync::Mutex::new(set);
    let settings_arc = sync::Arc::new(settings_mutex);
    {
        let settings_arc = settings_arc.clone();
        KeybdKey::HomeKey.bind(move || {
            asdf(&settings_arc);
        });
    }

    {
        let settings_arc = settings_arc.clone();
        KeybdKey::InsertKey.bind(move || {
            empty_inv(&settings_arc);
        });
    }

    let inputs = {
        std::thread::spawn(|| {
            inputbot::handle_input_events()
        })
    };

    let cmdline = {
        let settings_arc = settings_arc.clone();
        std::thread::spawn(move || {
            command_line(&settings_arc);
        })
    };

    inputs.join().unwrap();
    cmdline.join().unwrap();
}

fn command_line(settings: &sync::Arc<sync::Mutex<Settings>>){
    println!("yep coc");
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match split_space(&line.unwrap()) {
            ("pull", rest @ _) => {
                println!("pull delay is {}", rest);
                match rest.parse() {
                    Ok(x) => settings.lock().unwrap().pull_delay = x,
                    Err(_) => println!("could not delay"),
                }
            },
            ("push", rest @ _) => {
                println!("push delay is {}", rest);
                match rest.parse() {
                    Ok(x) => settings.lock().unwrap().push_delay = x,
                    Err(_) => println!("could not delay"),
                }
            },
            ("div", rest @ _) => {
                println!("div delay is {}", rest);
                match rest.parse() {
                    Ok(x) => settings.lock().unwrap().div_delay = x,
                    Err(_) => println!("could not delay"),
                }
            },
            (_, _) => {
                println!("Unknown command")
            },
        }
    }
}

fn click(x: i32, y: i32){
    inputbot::MouseCursor.move_abs(x * 2, y);
    std::thread::sleep(std::time::Duration::from_millis(5));
    MouseButton::LeftButton.press();
    std::thread::sleep(std::time::Duration::from_millis(5));
    MouseButton::LeftButton.release();
}

fn empty_inv_macro(start_slot: u32, delay: u64) {
    let inv_loc = (1297, 618);
    let inv_delta = 53;

    for x in (start_slot/5)..12 {
        for y in (start_slot % 5)..5 {
            click((x * inv_delta + inv_loc.0) as i32, (y * inv_delta + inv_loc.1) as i32);
            std::thread::sleep(std::time::Duration::from_millis(delay - 10));
        }
    }
}

fn empty_inv(settings: &sync::Arc<sync::Mutex<Settings>>){
    let delay = {
        settings.lock().unwrap().push_delay
    };

    println!("empty inv (delay {})", delay);

    KeybdKey::LControlKey.press();
    empty_inv_macro(5, delay);
    KeybdKey::LControlKey.release();
}

fn asdf(settings: &sync::Arc<sync::Mutex<Settings>>){
    let delay = {
        settings.lock().unwrap().pull_delay
    };

    println!("take tab (delay {})", delay);

    let disp = scrap::Display::primary().unwrap();
    let mut cap = scrap::Capturer::new(disp).unwrap();
    let width = cap.width();

    let frame;
    loop{
        //try 20x a second to read
        std::thread::sleep(std::time::Duration::from_millis(50));
        match cap.frame() {
            Ok(res) => {
                frame = res;
                break
            }
            Err(_) => {
            }
        }
    }

    let px: f64 = (625f64 - 17f64) / 23f64;
    let pys = [187, 213, 240, 266, 292, 319, 345, 371, 398, 424, 450, 477, 503, 529, 556, 582, 608, 635, 661, 687, 713, 740, 766, 792];

    KeybdKey::LControlKey.press();

    let mut movesleft = 60;
    for y in 0..24 {
        let ry = pys[y];

        for x in 0..24 {
            let mut rxf = (x as f64) * px + 17f64;
            if x == 2 {
                rxf += 2f64;
            }

            let rx = rxf as u32;

            let col1 = get_pixel_color_frame(&frame, rx as usize, ry as usize, width);
            let col2 = get_pixel_color_frame(&frame, (rx + 7) as usize, ry as usize, width);
            let col3 = get_pixel_color_frame(&frame, (rx + 15) as usize, ry as usize, width);

            let select_color = 0xFFE7B477;

            if col1 == select_color || col2 == select_color || col3 == select_color {
                click((rx + 10) as i32, (ry - 10) as i32);
                std::thread::sleep(std::time::Duration::from_millis(delay - 10));
                movesleft -= 1;
            }

            //if(slotIsSelected(img, rx, ry) || slotIsSelected(img, rx + 15, ry)){
                //img.setPixelColor(Jimp.cssColorToHex("#FF0000"), rx + 1, ry);
                //await stash.click([rx + 10, ry - 10]);
                //await robot.moveMouse(654, 801);
                //await sleep(delays.grabTab);
                //movesleft--;
            //}
            //img.setPixelColor(Jimp.cssColorToHex("#FFFFFF"), rx, ry);
        }
        if movesleft < 1 {
            break;
        }
    }

    KeybdKey::LControlKey.release();

    //image::save_buffer("./image.png", &frame, 1920, 1080, image::ColorType::Rgba8).unwrap();
}
