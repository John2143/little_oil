use serde::{Deserialize, Serialize};

use crate::{click, click_right, load_config, read_item_on_cursor, Settings};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AutoRollMod {
    pub name: String,
    pub is_prefix: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AutoRollConfig {
    pub item_name: String,
    pub mods: Vec<AutoRollMod>,
    pub auto_aug_regal: bool,
}

impl AutoRollConfig {
    fn needs_prefix(&self) -> bool {
        self.mods.iter().any(|x| x.is_prefix)
    }

    fn needs_suffix(&self) -> bool {
        self.mods.iter().any(|x| !x.is_prefix)
    }
}

#[derive(Debug)]
pub struct RollResult {
    has_prefix: bool,
    has_suffix: bool,
    has_mod: bool,
}

pub fn auto_roll(settings: &Settings, path: &str, times: i64) -> Option<RollResult> {
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
        if true || res.has_mod {
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

        //if inputbot::KeybdKey::RControlKey.is_pressed() {
        //return Some(res);
        //}
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

fn check_roll(item_text: &str, config: &AutoRollConfig) -> RollResult {
    let maybe_name = item_text
        .lines()
        .filter(|s| s.contains(&config.item_name))
        .nth(0)
        .unwrap();

    dbg!(&item_text.lines().collect::<Vec<_>>()[8..]);

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

#[test]
fn test_auto_roll() {
    auto_roll("test.json", 1);
}
