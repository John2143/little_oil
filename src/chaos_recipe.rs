//! Chaos-recipe bot: query the PoE stash API and click the matching items into
//! the quad tab.
use anyhow::bail;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChaosRecipe {
    session_id: String,
    account_name: String,
    league: String,
    tab_name: String,
    tab_index: Option<usize>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct Color {
    r: usize,
    g: usize,
    b: usize,
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

fn check_help(items: &[&str], base: &str) -> bool {
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
                .and_then(|name| name.as_str())
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

    fn get_json(&self, app: &crate::App) -> anyhow::Result<StashAPIResult> {
        let resp = ureq::get(&self.get_url())
            .header("Accept", "application/json")
            .header("Cookie", &format!("POESESSID={}", self.session_id))
            .call()
            .map_err(|e| anyhow::anyhow!("failed to fetch stash tab from pathofexile.com: {e}"))?;
        let apir: StashAPIResult = resp
            .into_body()
            .read_json()
            .map_err(|e| anyhow::anyhow!("failed to parse stash tab JSON: {e}"))?;

        let mut index_matched = false;
        for tab in &apir.tabs {
            if Some(tab.i) == self.tab_index {
                index_matched = true;
                println!("Chaos recipe tab name is {}", tab.n);
                println!("Config file name is {}", self.tab_name);
            } else if tab.n == self.tab_name {
                println!("closest ID is {}, {}", tab.n, tab.i);
                let mut settings = app.settings.write();
                if let Some(s) = settings.chaos_recipe_settings.as_mut() {
                    s.tab_index = Some(tab.i);
                }
                crate::save_config(&crate::config_path()?, &*settings)?;
                println!("writing config {:?}", settings);

                let mut newc = self.clone();
                newc.tab_index = Some(tab.i);
                //TODO safety
                return newc.get_json(app);
            }
        }

        if !index_matched {
            bail!(
                "No stash tab named '{}' found — check chaos_recipe_settings (account '{}', league '{}') in config.json",
                self.tab_name,
                self.account_name,
                self.league
            );
        }

        Ok(apir)
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
    fn create_item_list<'a>(&'a mut self) -> ItemList<'a> {
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

            if ty == ItemType::Weapon && il.weapon2.is_none() && item.h <= 3 {
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

impl ItemList<'_> {
    fn take(&self, app: &crate::App) {
        let (delay, grid, frame) = {
            let settings = app.settings.read();
            let grid = match &settings.stash_grid {
                Some(g) => g.clone(),
                None => {
                    println!("Stash grid not calibrated — run: little_oil calibrate-stash");
                    return;
                }
            };
            match settings.screenshot() {
                Ok(f) => (settings.pull_delay, grid, f),
                Err(e) => {
                    println!("Could not screenshot: {e}");
                    return;
                }
            }
        };

        let click_quad = |x: usize, y: usize| {
            if x >= 24 || y >= 24 {
                println!("Item slot ({x}, {y}) is outside the 24x24 grid, skipping");
                return;
            }
            let (px, py) = grid.cell_center(x, y);
            let (sx, sy) = frame.frame_to_screen(px, py);
            app.click(sx, sy);
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

        std::thread::sleep(std::time::Duration::from_millis(delay));
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
    }
}

pub fn get_tally(app: &crate::App, cr_config: &ChaosRecipe) -> anyhow::Result<()> {
    let apir = cr_config.get_json(app)?;
    println!("Total item counts: {:?}", apir.tally());
    Ok(())
}

pub fn do_recipe(app: &crate::App, cr_config: &ChaosRecipe, amt: usize) -> anyhow::Result<()> {
    let mut apir = cr_config.get_json(app)?;
    for _i in 0..amt {
        let item_list = apir.create_item_list();
        item_list.take(app);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inline PoE stash-API fixture (no network). Item order matters: the tall
    /// 2x4 branch comes first so it becomes weapon1, then the 1x3 sword must
    /// fit the weapon2 slot.
    const FIXTURE: &str = r#"{
        "numTabs": 1,
        "quadLayout": true,
        "tabs": [],
        "items": [
            {"x": 0, "y": 0, "identified": true, "baseType": "Iron Greaves", "ilvl": 1, "name": "", "typeLine": "", "w": 2, "h": 2, "properties": null},
            {"x": 2, "y": 0, "identified": true, "baseType": "Two-Stone Ring", "ilvl": 1, "name": "", "typeLine": "", "w": 1, "h": 1, "properties": null},
            {"x": 3, "y": 0, "identified": true, "baseType": "Iron Ring", "ilvl": 1, "name": "", "typeLine": "", "w": 1, "h": 1, "properties": null},
            {"x": 4, "y": 0, "identified": true, "baseType": "Plate Vest", "ilvl": 1, "name": "", "typeLine": "", "w": 2, "h": 3, "properties": null},
            {"x": 6, "y": 0, "identified": true, "baseType": "Gnarled Branch", "ilvl": 1, "name": "", "typeLine": "", "w": 2, "h": 4, "properties": [{"name": "Attacks per Second"}]},
            {"x": 8, "y": 0, "identified": true, "baseType": "Rusted Sword", "ilvl": 1, "name": "", "typeLine": "", "w": 1, "h": 3, "properties": [{"name": "Attacks per Second"}]}
        ]
    }"#;

    fn fixture() -> StashAPIResult {
        serde_json::from_str(FIXTURE).unwrap()
    }

    #[test]
    fn tally_counts_by_category() {
        let ic = fixture().tally();
        assert_eq!(ic.boots, 1);
        assert_eq!(ic.ring, 2);
        assert_eq!(ic.body, 1);
        assert_eq!(ic.weapon, 2);
        assert_eq!(ic.other, 0);
    }

    #[test]
    fn create_item_list_respects_weapon_height() {
        let mut apir = fixture();
        let il = apir.create_item_list();
        let w2 = il.weapon2.expect("1x3 sword must fit the weapon2 slot");
        assert_eq!(w2.base_type, "Rusted Sword");
    }

    #[test]
    fn second_recipe_pass_picks_different_items() {
        let mut apir = fixture();
        assert!(apir.create_item_list().weapon1.is_some());
        // Pass 1 marked every fixture item `used`, so pass 2 finds nothing.
        let second = apir.create_item_list();
        assert!(second.weapon1.is_none());
        assert!(second.weapon2.is_none());
        assert!(second.ring1.is_none());
        assert!(second.ring2.is_none());
        assert!(second.amulet.is_none());
        assert!(second.belt.is_none());
        assert!(second.gloves.is_none());
        assert!(second.boots.is_none());
        assert!(second.helmet.is_none());
        assert!(second.body.is_none());
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
