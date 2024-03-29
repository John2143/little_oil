use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChaosRecipe {
    session_id: String,
    account_name: String,
    league: String,
    tab_name: String,
    tab_index: Option<usize>,
}

#[derive(Deserialize, Debug, Clone)]
struct Color {
    r: usize,
    g: usize,
    b: usize,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct StashTabAPIResult {
    num_tabs: usize,
    tabs: Vec<StashTab>,
    items: Vec<Item>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct StashTab {
    n: String,
    i: usize,
    id: String,
    colour: Color,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct StashAPIResult {
    num_tabs: usize,
    #[serde(default)]
    quad_layout: bool,
    items: Vec<Item>,
    tabs: Vec<StashTab>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Item {
    x: usize,
    y: usize,
    identified: bool,
    base_type: String,
    ilvl: usize,
    name: String,
    type_line: String,
    w: usize,
    h: usize,
    properties: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    used: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemType {
    Weapon,
    Ring,
    Amulet,
    Belt,

    Gloves,
    Boots,
    Helmet,
    Body,

    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemCount {
    weapon: usize,
    ring: usize,
    amulet: usize,
    belt: usize,
    gloves: usize,
    boots: usize,
    helmet: usize,
    body: usize,
    other: usize,
}

fn check_help(items: &[&'static str], base: &str) -> bool {
    for item in items {
        if item == &base {
            return true;
        }
    }
    false
}

impl Item {
    fn get_category(&self) -> ItemType {
        use crate::dicts::*;

        if self.is_weapon() {
            return ItemType::Weapon;
        }

        if check_help(BOOTS, &self.base_type) {
            return ItemType::Boots;
        }

        if check_help(HELMETS, &self.base_type) {
            return ItemType::Helmet;
        }

        if check_help(GLOVES, &self.base_type) {
            return ItemType::Gloves;
        }

        if check_help(BODY, &self.base_type) {
            return ItemType::Body;
        }

        if self.base_type.contains("Ring") {
            return ItemType::Ring;
        }

        if self.base_type.contains("Belt") || self.base_type == "Rustic Sash" {
            return ItemType::Belt;
        }

        if self.base_type.contains("Amulet") {
            return ItemType::Amulet;
        }

        ItemType::Unknown
    }

    fn is_weapon(&self) -> bool {
        let props = match &self.properties {
            Some(s) => s,
            None => return false,
        };

        for prop in props {
            let hasaps = prop
                .get("name")
                .map(|name| name.as_str())
                .flatten()
                .map(|name| name == "Attacks per Second");

            if hasaps == Some(true) {
                return true;
            }
        }

        false
    }
}

impl ChaosRecipe {
    fn get_url(&self) -> String {
        //let u = format!(
        //"https://www.pathofexile.com/character-window/get-stash-items?accountName={}&realm=pc&league={}&tabs=1&tabIndex={}",
        //self.account_name,
        //self.league,
        //self.tab_index,
        //);
        //let d = ureq::get(&u)
        //.set("Accept", "application/json")
        //.set("Cookie", &format!("POESESSID={}", self.session_id))
        //.call();

        //dbg!(d.unwrap().into_string());

        format!(
            "https://www.pathofexile.com/character-window/get-stash-items?accountName={}&realm=pc&league={}&tabs=1&tabIndex={}",
            self.account_name,
            self.league,
            self.tab_index.unwrap_or(0),
        )
    }

    fn get_json(&self) -> StashAPIResult {
        let d = ureq::get(&self.get_url())
            .set("Accept", "application/json")
            .set("Cookie", &format!("POESESSID={}", self.session_id))
            .call();

        //let apir2 = d.unwrap().into_string().unwrap();
        //dbg!(apir2);
        let apir: StashAPIResult = d.unwrap().into_json().unwrap();

        for tab in &apir.tabs {
            if Some(tab.i) == self.tab_index {
                println!("Chaos recipe tab name is {}", tab.n);
                println!("Config file name is {}", self.tab_name);
            } else if tab.n == self.tab_name {
                println!("closest ID is {}, {}", tab.n, tab.i);
                let mut settings = crate::SETTINGS.write().unwrap();
                settings
                    .chaos_recipe_settings
                    .as_mut()
                    .map(|s| s.tab_index = Some(tab.i));
                crate::save_config(crate::CONFIG_PATH, &*settings).unwrap();
                println!("writing config {:?}", settings);

                let mut newc = self.clone();
                newc.tab_index = Some(tab.i);
                //TODO safety
                return newc.get_json();
            }
        }

        apir
    }
}

#[derive(Default, Debug)]
struct ItemList<'a> {
    weapon1: Option<&'a Item>,
    weapon2: Option<&'a Item>,
    ring1: Option<&'a Item>,
    ring2: Option<&'a Item>,

    amulet: Option<&'a Item>,
    belt: Option<&'a Item>,
    gloves: Option<&'a Item>,
    boots: Option<&'a Item>,
    helmet: Option<&'a Item>,
    body: Option<&'a Item>,
}

impl StashAPIResult {
    fn create_item_list(&mut self) -> ItemList {
        let mut il = ItemList::default();
        for item in self.items.iter_mut() {
            let ty = item.get_category();
            if ty == ItemType::Unknown {
                continue;
            }

            if item.used {
                continue;
            }

            item.used = true;
            if ty == ItemType::Weapon && il.weapon1.is_none() {
                il.weapon1 = Some(&*item);
                continue;
            }

            if ty == ItemType::Weapon && il.weapon2.is_none() && il.weapon1.unwrap().h <= 3 {
                il.weapon2 = Some(item);
                continue;
            }

            if ty == ItemType::Ring && il.ring1.is_none() {
                il.ring1 = Some(item);
                continue;
            }

            if ty == ItemType::Ring && il.ring2.is_none() {
                il.ring2 = Some(item);
                continue;
            }

            if ty == ItemType::Amulet && il.amulet.is_none() {
                il.amulet = Some(item);
                continue;
            }

            if ty == ItemType::Belt && il.belt.is_none() {
                il.belt = Some(item);
                continue;
            }

            if ty == ItemType::Gloves && il.gloves.is_none() {
                il.gloves = Some(item);
                continue;
            }
            if ty == ItemType::Boots && il.boots.is_none() {
                il.boots = Some(item);
                continue;
            }
            if ty == ItemType::Helmet && il.helmet.is_none() {
                il.helmet = Some(item);
                continue;
            }
            if ty == ItemType::Body && il.body.is_none() {
                il.body = Some(item);
                continue;
            }

            item.used = false;
        }

        il
    }

    fn tally(&self) -> ItemCount {
        let mut ic = ItemCount {
            weapon: 0,
            ring: 0,
            amulet: 0,
            belt: 0,
            gloves: 0,
            boots: 0,
            helmet: 0,
            body: 0,
            other: 0,
        };

        for item in &self.items {
            let ty = item.get_category();
            let field = match ty {
                ItemType::Weapon => &mut ic.weapon,
                ItemType::Ring => &mut ic.ring,
                ItemType::Amulet => &mut ic.amulet,
                ItemType::Belt => &mut ic.belt,
                ItemType::Gloves => &mut ic.gloves,
                ItemType::Boots => &mut ic.boots,
                ItemType::Helmet => &mut ic.helmet,
                ItemType::Body => &mut ic.body,
                ItemType::Unknown => &mut ic.other,
            };
            *field += 1;
        }

        ic
    }
}

//TODO copy less code
impl ItemList<'_> {
    fn take(&self) {
        let (delay, height) = {
            let settings = crate::SETTINGS.read().unwrap();
            (settings.pull_delay, settings.screen_height.unwrap_or(1080))
        };

        let left_edge = if height == 1080 {
            21
        } else if height == 1440 {
            29
        } else {
            panic!("invalid screen size");
        };

        let px = if height == 1080 {
            (2573 - 1920) / 24
        } else if height == 1440 {
            830 - 795
        } else {
            panic!("invalid screen size");
        };

        let pys = if height == 1080 {
            [
                160, 186, 212, 239, 265, 291, 318, 344, 370, 397, 423, 449, 476, 502, 528, 555,
                581, 607, 634, 660, 686, 712, 739, 765, //792,
            ]
        } else if height == 1440 {
            [
                260, 295, 330, 365, 400, 436, 471, 506, 541, 576, 611, 646, 681, 716, 751, 787,
                822, 857, 892, 927, 962, 997, 1032, 1067,
            ]
        } else {
            panic!("invalid screen size");
        };

        let click_quad = |x: usize, y: usize| {
            let ry = pys[y];
            let rx = x * px + left_edge;
            crate::click((rx + 10) as i32, (ry - 10) as i32);
            std::thread::sleep(std::time::Duration::from_millis(delay + 10));
        };

        let clicks = [
            ("Body", self.body),
            ("Helmet", self.helmet),
            ("Boots", self.boots),
            ("Gloves", self.gloves),
            ("Belt", self.belt),
            ("Weapon A", self.weapon1),
            ("Weapon B", self.weapon2),
            ("Ring 1", self.ring1),
            ("Ring 2", self.ring2),
            ("Amulet", self.amulet),
        ];

        use inputbot::KeybdKey;
        KeybdKey::LControlKey.press();
        std::thread::sleep(std::time::Duration::from_millis(delay - 10));
        for (name, c) in clicks {
            match c {
                Some(s) => {
                    println!("Got item (slot {}): {}", name, s.base_type);
                    click_quad(s.x, s.y);
                }
                None => {
                    println!("No item for slot {}", name);
                }
            }
        }

        KeybdKey::LControlKey.release();
    }
}

use ureq;

pub fn get_tally(cr_config: &ChaosRecipe) {
    let apir = cr_config.get_json();
    println!("Total item counts: {:?}", apir.tally());
}

pub fn do_recipe(cr_config: &ChaosRecipe, amt: usize) {
    let mut apir = cr_config.get_json();
    for i in 0..amt {
        let item_list = apir.create_item_list();
        item_list.take();
    }
}

//curl 'https://www.pathofexile.com/character-window/get-stash-items
//?accountName=John2143658709
//&realm=pc
//&league=Kalandra
//&tabs=0
//&tabIndex=6
//'-H 'Accept: application/json, text/javascript, */*; q=0.01'
//-H 'Accept-Language: en-US,en;q=0.5'
//-H 'Accept-Encoding: gzip, deflate, br'
//-H 'Cookie: POESESSID=asdf'
//--compressed
