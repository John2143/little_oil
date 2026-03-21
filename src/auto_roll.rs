use serde::{Deserialize, Serialize};

use crate::{click, click_right, load_config, read_item_on_cursor};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Currency {
    Alt,
    Aug,
    Chance,
    Scour,
    Regal,
    Alch,
    Chaos,
    Transmute,
    Binding,
    Exalt,
    Annul,
    Cromatic,
}

impl Currency {
    fn click_coords(&self) -> (i32, i32) {
        match self {
            Currency::Alt => (155, 354),
            Currency::Aug => (300, 422),
            Currency::Chance => (297, 363),
            Currency::Scour => (580, 530),
            Currency::Regal => (572, 354),
            Currency::Alch => (655, 360),
            Currency::Chaos => (725, 360),
            Currency::Transmute => (71, 360),
            Currency::Binding => (216, 608),
            Currency::Exalt => (400, 360),
            Currency::Annul => (230, 360),
            Currency::Cromatic => (295, 532),
        }
    }

    /// The third line of the currency tooltip should contain this name
    fn get_name(&self) -> &'static str {
        match self {
            Currency::Alt => "Orb of Alteration",
            Currency::Aug => "Orb of Augmentation",
            Currency::Chance => "Orb of Chance",
            Currency::Scour => "Orb of Scouring",
            Currency::Regal => "Regal Orb",
            Currency::Alch => "Orb of Alchemy",
            Currency::Chaos => "Chaos Orb",
            Currency::Transmute => "Orb of Transmutation",
            Currency::Binding => "Orb of Binding",
            Currency::Exalt => "Exalted Orb",
            Currency::Annul => "Orb of Annulment",
            Currency::Cromatic => "Chromatic Orb",
        }
    }
}

#[test]
fn test_currency_coords() {
    super::move_mouse(1, 1);
        std::thread::sleep(std::time::Duration::from_millis(500));
    for currency in [
        Currency::Alt,
        Currency::Aug,
        Currency::Chance,
        Currency::Scour,
        Currency::Regal,
        Currency::Alch,
        Currency::Chaos,
        Currency::Transmute,
        Currency::Binding,
        Currency::Exalt,
        Currency::Annul,
        Currency::Cromatic,
    ] {
        let coords = currency.click_coords();
        let (x, y) = coords;
        println!("{:?} coords: {:?}", currency, coords);
        super::move_mouse(x, y);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let item = read_item_on_cursor();
        let third_line = item.lines().nth(2).unwrap_or("");
        assert_eq!(third_line, currency.get_name());
    }
}

pub fn auto_roll(path: &str, times: i64) -> Option<RollResult> {
    #![allow(unused_variables)]
    let alt = Currency::Alt.click_coords();
    let aug = Currency::Aug.click_coords();
    let reg = Currency::Regal.click_coords();
    let slot = (444, 628);

    let config: AutoRollConfig = {
        match load_config(path, None) {
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


#[derive(Debug, PartialEq, Eq)]
pub enum ModType {
    Prefix,
    Suffix,
    Implicit,
    Other,
}

#[derive(Debug)]
#[allow(unused)]
pub struct ParsedMod {
    pub mod_type: ModType,
    pub is_fractured: bool,
    pub notable_name: String,
    pub tier: i32,
    pub tags: Vec<String>,
    pub full_text: String,
}

impl ParsedMod {
    fn is_prefix(&self) -> bool {
        self.mod_type == ModType::Prefix
    }

    fn is_suffix(&self) -> bool {
        self.mod_type == ModType::Suffix
    }
}

//impl ModFilter

fn check_roll(item_text: &str, config: &AutoRollConfig) -> RollResult {
    //println!("checking roll: {}", item_text);
    //println!("looking for: {}", config.item_name);

    //dbg!(&item_text.lines().collect::<Vec<_>>()[8..]);

    // { Prefix Modifier \"Notable\" (Tier: 1) — Caster, Speed }
    // or
    // { Suffix Modifier \"Notable\" (Tier: 1) }
    let regex = regex::Regex::new(
        r#"\{ ([\w ]+) Modifier \"([^\"]*)\" \(Tier: (\d+)\) —? ?([^\}]*)\)?"#,
    )
    .unwrap();

    let mut modlines = vec![];
    let mut cur_mod_line = None;
    let mut mod_text = String::new();
    for line in item_text.lines() {
        if let Some(top_mod_line) = cur_mod_line {
            if !line.starts_with("------") && !line.starts_with("{") && !line.ends_with("}") {
                mod_text += line;
                mod_text += "\n";
                continue;
            }
            cur_mod_line = None;

            let Some(parsed) = regex.captures(top_mod_line) else {
                tracing::warn!("Invalid mod line: {top_mod_line}");
                continue;
            };

            let p = &parsed[1];
            let mut mod_type = ModType::Other;
            if p.contains("Prefix") {
                mod_type = ModType::Prefix;
            } else if p.contains("Suffix") {
                mod_type = ModType::Suffix;
            } else if p.contains("Implicit") {
                mod_type = ModType::Implicit;
            }
            let is_fractured = p.contains("Fractured");

            let notable_name = &parsed[2];
            let tier = parsed[3].parse::<i32>().unwrap();
            let tags = parsed
                .get(4)
                .map_or("", |m| m.as_str())
                .split(", ")
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            modlines.push(ParsedMod {
                is_fractured,
                mod_type,
                notable_name: notable_name.to_string(),
                tier,
                tags,
                full_text: line.to_string(),
            });
        }

        if line.starts_with("{") && line.ends_with("}")
        {
            cur_mod_line = Some(line);
        }
    }

    let mut has_prefix = false; //has any prefix
    let mut has_suffix = false; //has any suffix
    let mut has_mod_prefix = false; //has a matching prefix
    let mut has_mod_suffix = false; //has a matching suffix
    for modline in &modlines {
        if modline.mod_type == ModType::Prefix {
            has_prefix = true;
        } if modline.mod_type == ModType::Suffix {
            has_suffix = true;
        }

        for mod_config in &config.mods {
            let mut got_match = false;
            if modline.notable_name == mod_config.name {
                println!("found notable name match: {}", mod_config.name);
                got_match = true;
            }
            if modline
                .full_text
                .to_lowercase()
                .contains(&mod_config.name.to_lowercase())
            {
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

    let prefixes = modlines.iter().filter(|m| m.is_prefix());
    let suffixes = modlines.iter().filter(|m| m.is_suffix());
    let prefixes_tiers = prefixes.clone().map(|m| m.tier).collect::<Vec<_>>();
    let suffixes_tiers = suffixes.clone().map(|m| m.tier).collect::<Vec<_>>();
    println!(
        "Got {} mods. Tiers: {:?} / {:?}",
        modlines.len(),
        prefixes_tiers,
        suffixes_tiers
    );
    println!(
        "Prefixes: {}",
        prefixes
            .clone()
            .map(|m| m.notable_name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "Suffixes: {}",
        suffixes
            .clone()
            .map(|m| m.notable_name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );

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

#[cfg(test)]
mod test {
    use super::*;

    fn trim_lines_start_end(item_text: &str) -> String {
        item_text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_auto_roll() {
        auto_roll("test.json", 1);
    }

    #[test]
    fn test_item_fractured() {
        let item_text = r#"
            Item Class: Gloves
            Rarity: Magic
            Phantom Mitts of Puhuarte
            --------
            Quality: +20% (augmented)
            Evasion Rating: 275 (augmented)
            Energy Shield: 55 (augmented)
            --------
            Requirements:
            Level: 84
            Dex: 80
            Int: 80
            --------
            Sockets: B-G G-G 
            --------
            Item Level: 86
            --------
            { Searing Exarch Implicit Modifier (Lesser) — Damage, Chaos }
            +7(5-7)% to Chaos Damage over Time Multiplier
            --------
            { Fractured Suffix Modifier "of Puhuarte" — Damage, Elemental, Cold, Resistance }
            +47(46-4￼% to Cold Resistance
            49(30-50)% increased Damage with Hits against Chilled Enemies
            { Prefix Modifier "Acute" (Tier: 6) — Damage }
            5(5-10)% increased Damage with Bow Skills
            Searing Exarch Item
            --------
            Fractured Item
        "#;

        let item_text = trim_lines_start_end(item_text);

        let config = AutoRollConfig {
            item_name: "Phantom Mitts".to_string(),
            mods: vec![AutoRollMod {
                name: "of Puhuarte".to_string(),
                is_prefix: false,
            }],
            auto_aug_regal: false,
            any_two_t1: false,
            needs_prefix_and_suffix: false,
        };

        let res = check_roll(&item_text, &config);
        // Ignore frac mod
        assert!(!res.has_suffix);
        assert!(!res.has_mod);
        assert!(res.has_prefix);
    }

    #[test]
    fn normal_item() {
        let item_text = r#"
            Item Class: Quivers
            Rarity: Magic
            Acute Feathered Arrow Quiver of Ire
            --------
            Requirements:
            Level: 20
            --------
            Item Level: 86
            --------
            { Implicit Modifier — Speed }
            25(20-30)% increased Projectile Speed
            --------
            { Prefix Modifier "Acute" (Tier: 6) — Damage }
            5(5-10)% increased Damage with Bow Skills
            { Suffix Modifier "of Ire" (Tier: 6) — Damage, Attack, Critical }
            +10(8-12)% to Critical Strike Multiplier with Bows
        "#;

        let item_text = trim_lines_start_end(item_text);

        let config = AutoRollConfig {
            item_name: "Feathered Arrow Quiver".to_string(),
            mods: vec![],
            auto_aug_regal: false,
            any_two_t1: false,
            needs_prefix_and_suffix: false,
        };

        let res = check_roll(&item_text, &config);
        assert!(res.has_suffix);
        assert!(res.has_prefix);
    }
}
