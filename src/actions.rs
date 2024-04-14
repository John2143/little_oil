//!This file contains all the different things that little oil can do, like empty your inventory,
//!pull items out of your stash, or roll items.
use anyhow::{bail, Context};
use clap::Parser;
use crate::mouse::{click, click_right};
use rand::Rng;
use crate::screenshot::ScreenshotData;
use tracing::{debug, info, trace};
use once_cell::sync::Lazy;
use std::sync::RwLock;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::{Settings, SETTINGS};

pub fn chance() -> anyhow::Result<()> {
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

pub fn reset_inv_colors(settings: &Settings) -> anyhow::Result<()> {
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

    crate::save_config(crate::get_config_path(), &*settings)?;
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
    empty_inv_macro(settings, slot, settings.push_delay)
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
