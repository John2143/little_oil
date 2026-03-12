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
    #[serde(default)]
    pub any_two_t1: bool,
    #[serde(default)]
    pub needs_prefix_and_suffix: bool,
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
    let sleep_read = 150;

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

#[derive(Debug)]
#[allow(unused)]
pub struct ParsedMod {
    pub is_prefix: bool,
    pub notable_name: String,
    pub tier: i32,
    pub tags: Vec<String>,
    pub full_text: String,
}

fn check_roll(item_text: &str, config: &AutoRollConfig) -> RollResult {
    //println!("checking roll: {}", item_text);
    //println!("looking for: {}", config.item_name);


    //dbg!(&item_text.lines().collect::<Vec<_>>()[8..]);

    // { Prefix Modifier \"Notable\" (Tier: 1) — Caster, Speed }
    // or
    // { Suffix Modifier \"Notable\" (Tier: 1) }
    let regex = regex::Regex::new(r#"\{ (Prefix|Suffix) Modifier \"([^\"]*)\" \(Tier: (\d+)\) —? ?([^\}]*)\)?"#).unwrap();

    let mut modlines = vec![];
    let mut cur_mod_line = None;
    for line in item_text.lines() {
        if let Some(mod_line) = cur_mod_line {
            let parsed = regex.captures(mod_line).unwrap();
            let is_prefix = &parsed[1] == "Prefix";
            let notable_name = &parsed[2];
            let tier = parsed[3].parse::<i32>().unwrap();
            let tags = parsed
                .get(4)
                .map_or("", |m| m.as_str())
                .split(", ")
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            modlines.push(ParsedMod {
                is_prefix,
                notable_name: notable_name.to_string(),
                tier,
                tags,
                full_text: line.to_string(),
            });

            cur_mod_line = None;
        }
        if line.starts_with("{") && line.ends_with("}") && !line.starts_with("{ Implicit Modifier") {
            cur_mod_line = Some(line);
        }
    }

    let mut has_prefix = false; //has any prefix
    let mut has_suffix = false; //has any suffix
    let mut has_mod_prefix = false; //has a matching prefix
    let mut has_mod_suffix = false; //has a matching suffix
    for modline in &modlines {
        if modline.is_prefix {
            has_prefix = true;
        } else {
            has_suffix = true;
        }

        for mod_config in &config.mods {
            let mut got_match = false;
            if modline.notable_name == mod_config.name {
                println!("found notable name match: {}", mod_config.name);
                got_match = true;
            }
            if modline.full_text.to_lowercase().contains(&mod_config.name.to_lowercase()) {
                println!("found full text match: {}", mod_config.name);
                got_match = true;
            }

            if got_match {
                if mod_config.is_prefix {
                    has_mod_prefix = true;
                } else {
                    has_mod_suffix = true;
                }
            }
        }
    }

    // if we have any of the mods, then we can set this to true
    let mut has_mod = has_mod_prefix || has_mod_suffix;
    // if this config flag is set, then only set has_mod to true if we have both a prefix and
    // suffix mod matching
    if config.needs_prefix_and_suffix {
        has_mod = has_mod_prefix && has_mod_suffix;
    }

    let prefixes = modlines.iter().filter(|m| m.is_prefix);
    let suffixes = modlines.iter().filter(|m| !m.is_prefix);
    let prefixes_tiers = prefixes.clone().map(|m| m.tier).collect::<Vec<_>>();
    let suffixes_tiers = suffixes.clone().map(|m| m.tier).collect::<Vec<_>>();
    println!("Got {} mods. Tiers: {} / {}", modlines.len(), format!("{:?}", prefixes_tiers), format!("{:?}", suffixes_tiers));
    println!("Prefixes: {}", prefixes.clone().map(|m| m.notable_name.clone()).collect::<Vec<_>>().join(", "));
    println!("Suffixes: {}", suffixes.clone().map(|m| m.notable_name.clone()).collect::<Vec<_>>().join(", "));

    //println!("any two t1: {}, any t1: {}", config.any_two_t1, modlines.iter().any(|m| m.tier == 1));
    if modlines.iter().all(|m| m.tier == 1) && modlines.len() == 2 && config.any_two_t1 {
        println!("all mods are t1 and any_two_t1 is enabled");
        has_mod = true;
    }

    RollResult {
        has_prefix,
        has_suffix,
        has_mod,
    }
}

#[test]
fn test_auto_roll() {
    auto_roll("test.json", 1);
}
