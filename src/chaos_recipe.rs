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

#[derive(Debug, PartialEq, Eq)]
enum TabMatch {
    /// The fetched page is the configured tab.
    Current,
    /// The configured tab lives at this index; fetch that page instead.
    Refetch(usize),
    /// No tab has the configured name.
    Missing,
}

fn match_tab(tabs: &[StashTab], tab_name: &str, fetched_index: usize) -> TabMatch {
    if tabs.iter().any(|t| t.i == fetched_index && t.n == tab_name) {
        return TabMatch::Current;
    }
    match tabs.iter().find(|t| t.n == tab_name) {
        Some(t) => TabMatch::Refetch(t.i),
        None => TabMatch::Missing,
    }
}

impl ChaosRecipe {
    fn get_url(&self, tab_index: usize) -> String {
        format!(
            "https://www.pathofexile.com/character-window/get-stash-items?accountName={}&realm=pc&league={}&tabs=1&tabIndex={}",
            self.account_name, self.league, tab_index,
        )
    }

    fn fetch(&self, tab_index: usize) -> anyhow::Result<StashAPIResult> {
        let resp = ureq::get(&self.get_url(tab_index))
            .header("Accept", "application/json")
            .header("Cookie", &format!("POESESSID={}", self.session_id))
            .call()
            .map_err(|e| anyhow::anyhow!("failed to fetch stash tab from pathofexile.com: {e}"))?;
        resp.into_body()
            .read_json()
            .map_err(|e| anyhow::anyhow!("failed to parse stash tab JSON: {e}"))
    }

    /// Fetch the configured stash tab. If it moved, find it by name, save the
    /// new index to config, and fetch once more — never more than two requests.
    fn get_json(&self, app: &crate::App) -> anyhow::Result<StashAPIResult> {
        let first = self.tab_index.unwrap_or(0);
        let apir = self.fetch(first)?;
        match match_tab(&apir.tabs, &self.tab_name, first) {
            TabMatch::Current => Ok(apir),
            TabMatch::Missing => bail!(
                "No stash tab named '{}' found — check chaos_recipe_settings (account '{}', league '{}') in config.json",
                self.tab_name,
                self.account_name,
                self.league
            ),
            TabMatch::Refetch(i) => {
                println!(
                    "Stash tab '{}' is index {i}; saving to config",
                    self.tab_name
                );
                {
                    let mut settings = app.settings.write();
                    if let Some(s) = settings.chaos_recipe_settings.as_mut() {
                        s.tab_index = Some(i);
                    }
                }
                app.save_settings()?;
                let apir = self.fetch(i)?;
                if match_tab(&apir.tabs, &self.tab_name, i) != TabMatch::Current {
                    bail!(
                        "Stash tab '{}' moved while fetching — run the command again",
                        self.tab_name
                    );
                }
                Ok(apir)
            }
        }
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
            let settings = app.settings.read().clone();
            let grid = match &settings.stash_grid {
                Some(g) => g.clone(),
                None => {
                    println!("Stash grid not calibrated — run: little_oil calibrate-stash");
                    return;
                }
            };
            match app.screenshot() {
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
            app.backend().sleep_ms(delay + 10);
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

        app.backend().sleep_ms(delay);
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
    fn take_clicks_the_chosen_item_slots() {
        use crate::platform::InputButton;
        use crate::stash_grid::CellGrid;
        use crate::test_support::{Ev, blank_frame, test_app};

        // A 300x300 game window at (100, 200) with a uniform 10-px quad grid.
        let region = crate::ScreenRegion {
            x: 100,
            y: 200,
            width: 300,
            height: 300,
        };
        let grid: CellGrid<24, 24> = CellGrid {
            cols: std::array::from_fn(|i| i as u32 * 10),
            rows: std::array::from_fn(|i| i as u32 * 10),
            cell_w: 10,
            cell_h: 10,
            highlight_color: 0,
        };
        let mut settings = crate::default_settings();
        settings.focus_clicks = 0;
        settings.game_window_region = Some(region);
        settings.stash_grid = Some(grid.clone());
        let (app, fake) = test_app(settings);
        fake.push_frame(blank_frame(region, 0));

        let mut apir = fixture();
        let list = apir.create_item_list();
        list.take(&app);

        // `take` clicks body, boots, weapon A, weapon B, ring 1, ring 2 —
        // helmet, gloves, belt and amulet are missing from the fixture.
        let mut expected = vec![Ev::Sleep(50)]; // pull_delay before the first click
        for (x, y) in [(4u32, 0u32), (0, 0), (6, 0), (8, 0), (2, 0), (3, 0)] {
            let (px, py) = grid.cell_center(x as usize, y as usize);
            expected.extend([
                Ev::Move(region.x as i32 + px as i32, region.y as i32 + py as i32),
                Ev::Sleep(30),
                Ev::Button(InputButton::Left, true),
                Ev::Sleep(10),
                Ev::Button(InputButton::Left, false),
                Ev::Sleep(60), // pull_delay + 10
            ]);
        }
        assert_eq!(
            fake.events(),
            expected,
            "one click per chosen stash slot, in take order"
        );
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

    fn tab(name: &str, index: usize) -> StashTab {
        StashTab {
            n: name.to_string(),
            i: index,
            id: format!("tab-{index}"),
            colour: Color { r: 0, g: 0, b: 0 },
        }
    }

    #[test]
    fn match_tab_current_when_fetched_index_holds_the_name() {
        let tabs = [tab("maps", 0), tab("quad", 1)];
        assert_eq!(match_tab(&tabs, "quad", 1), TabMatch::Current);
    }

    #[test]
    fn match_tab_refetches_the_index_the_named_tab_lives_at() {
        let tabs = [tab("maps", 0), tab("quad", 5)];
        assert_eq!(match_tab(&tabs, "quad", 0), TabMatch::Refetch(5));
    }

    #[test]
    fn match_tab_current_on_the_second_duplicate_name() {
        let tabs = [tab("maps", 0), tab("quad", 2), tab("quad", 7)];
        assert_eq!(match_tab(&tabs, "quad", 7), TabMatch::Current);
    }

    /// Refetch always targets the first tab in slice order, so duplicate names
    /// cannot make the caller bounce between two indices.
    #[test]
    fn match_tab_refetches_first_duplicate_by_order() {
        let tabs = [tab("maps", 0), tab("quad", 2), tab("quad", 7)];
        assert_eq!(match_tab(&tabs, "quad", 0), TabMatch::Refetch(2));
    }

    #[test]
    fn match_tab_missing_when_no_tab_has_the_name() {
        let tabs = [tab("maps", 0), tab("quad", 1)];
        assert_eq!(match_tab(&tabs, "chaos", 0), TabMatch::Missing);
    }

    /// A tab at the fetched index is not enough: the name must match too, or
    /// the bot would click into whichever tab happens to sit at that index.
    #[test]
    fn match_tab_missing_when_only_the_index_matches() {
        let tabs = [tab("maps", 0), tab("quad", 1)];
        assert_eq!(match_tab(&tabs, "chaos", 1), TabMatch::Missing);
    }

    #[test]
    fn match_tab_missing_on_empty_tab_list() {
        assert_eq!(match_tab(&[], "quad", 0), TabMatch::Missing);
    }
}
