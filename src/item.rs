//! Item tooltip parsing and mod classification for rolling.
use std::{fmt::Display, ops::Range};

use anyhow::Context;
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use tracing::{debug, span, trace};

#[derive(Debug)]
pub struct Item<'a> {
    pub base_name: &'a str,
    pub item_name: ItemName,

    pub stats: Vec<StatLine<'a>>,

    pub ilvl: u8,
    pub sockets: &'a str,

    pub mods: Vec<ItemMod<'a>>,
}

#[derive(Debug)]
pub struct StatLine<'a> {
    pub stat_name: &'a str,
    pub stat_value: Decimal,
}

#[derive(Debug)]
pub enum ItemName {
    /// Gems, etc
    Other(String),
    Normal,
    Magic {
        prefix: String,
        suffix: String,
    },
    Rare(String),
    Unique(String),
}

impl Display for ItemName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemName::Other(name) => write!(f, "?: {name}")?,
            ItemName::Normal => write!(f, "N")?,
            ItemName::Magic { prefix, suffix } => match (prefix.as_ref(), suffix.as_ref()) {
                ("", suffix) => {
                    write!(f, "M(s): {suffix}")?;
                }
                (prefix, "") => {
                    write!(f, "M(p): {prefix}")?;
                }
                (prefix, suffix) => {
                    write!(f, "M(p+s): {prefix} : {suffix}")?;
                }
            },
            ItemName::Rare(name) => write!(f, "R: {name}")?,
            ItemName::Unique(name) => write!(f, "U: {name}")?,
        };

        Ok(())
    }
}

#[derive(Debug)]
pub struct ItemMod<'a> {
    /// prefix, suffix, unique
    pub affix_type: AffixType,

    /// Contains the tier if it's a rare mod
    pub affix_name_tier: Option<AffixNameTier<'a>>,

    pub value: Option<Decimal>,
    pub roll_range: Option<Range<Decimal>>,

    /// Raw value line under the mod header (e.g. `+29(24-29)% to Lightning Resistance (fractured)`).
    pub full_text: &'a str,

    /// Tags for catalysts: things like Defenses, Evasion, Fire
    pub tags: Vec<&'a str>,
    /// is fractured, etc
    #[allow(dead_code)] // parsed but not yet consumed; feature-modfilter branch uses these
    mod_qualifiers: &'a str,
}

#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub enum AffixType {
    Prefix,
    Suffix,
    Implicit,
    Unique,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AffixNameTier<'a> {
    pub name: &'a str,
    pub tier: i32,
}

// { Prefix Modifier "Phantasm's" (Tier: 3) — Defences, Evasion }
// Example:
//                      {     Prefix          Modifier    "Phantasm's  "  (Tier:      3        )      —  Defences, Evasion        }
//                         vvvvvvvvvvvvvvvvv               vvvvvvvvvvvv           vvvvvvvvvvvvv            vvvvvvvvvvv
const IMR_1: &str = r#"\{ (?P<affix_type>\w+) Modifier (?:"(?P<name>.+)" \(Tier: (?P<tier>\d+)\) )?(?:— (?P<affixes>.*) )?\}"#;
static ITEM_MOD_LINE_1_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(IMR_1).unwrap());

const IMR_2: &str = r#"(?P<before>[^\d]*)(?P<value>\d+(?:\.\d+)?)?(?:\((?P<bot_roll>\d+(?:\.\d+)?)-(?P<top_roll>\d+(?:\.\d+)?)\))?(?P<end>.*)"#;
static ITEM_MOD_LINE_2_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(IMR_2).unwrap());

const CR: &str = r#"(?P<left>[^:]+):(?P<right>.+)"#;
static COLON_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(CR).unwrap());

enum ItemParseSections {
    Class,
    Rarity,
    Name,

    Stats,

    //Requirements,
    //Sockets,
    //Level,
    Mods,
}

impl<'a> Item<'a> {
    pub(crate) fn from_str(source: &'a str) -> anyhow::Result<Self> {
        let span = span!(tracing::Level::DEBUG, "Item Parser");
        let _ = span.enter();
        let mut cur_parser_state = ItemParseSections::Class;

        let mut item_type = None;
        let mut item_name = String::new();

        let mut current_parsed_modline = None;
        let mut mods = vec![];
        let mut line_iterator = source.trim().lines().peekable();
        while let Some(line) = line_iterator.next() {
            let line = line.trim();

            match cur_parser_state {
                //First state: What are we looking at?
                ItemParseSections::Class => {
                    let res = COLON_REGEX
                        .captures(line)
                        .context("should match first line")?;

                    let left = res
                        .name("left")
                        .context("left part of first line")?
                        .as_str();
                    if left != "Item Class" {
                        anyhow::bail!("expected 'Item Class', got '{left}'");
                    }

                    item_type = Some(res.name("right").context("right part of first line")?);

                    debug!(?item_type);
                    cur_parser_state = ItemParseSections::Rarity;
                }
                ItemParseSections::Rarity => {
                    let res = COLON_REGEX
                        .captures(line)
                        .context("should match first line")?;

                    let left = res
                        .name("left")
                        .context("left part of first line")?
                        .as_str();
                    if left != "Rarity" {
                        anyhow::bail!("expected 'Rarity', got '{left}'");
                    }

                    let _ = res.name("right").context("right part of rarity line")?;

                    debug!(?item_type);
                    cur_parser_state = ItemParseSections::Name;
                }
                ItemParseSections::Name => {
                    if line == "--------" {
                        trace!("Item line separator");
                        cur_parser_state = ItemParseSections::Stats;
                        continue;
                    }

                    if !item_name.is_empty() {
                        item_name.push('\n');
                    }
                    item_name.push_str(line);
                }
                ItemParseSections::Stats => {
                    if line == "--------" {
                        trace!("Item line separator");

                        // Check the next line to see if it contains a mod.
                        // If it does, advance the state.
                        match line_iterator.peek() {
                            Some(next) if next.starts_with('{') => {
                                debug!("Moving to next state");
                                cur_parser_state = ItemParseSections::Mods;
                                continue;
                            }
                            _ => {} // no next line or no mod header — stay in Stats
                        }
                    }

                    trace!(line, "Item stat line");
                }
                ItemParseSections::Mods => {
                    // If we have a mod line saved, then combine that with the current line.
                    // These two lines make up a single mod. ex:
                    //
                    // last:  { Unique Modifier — Elemental, Fire, Resistance }
                    // cur:   +49(40-50)% to Fire Resistance
                    if let Some(last_line) = current_parsed_modline {
                        debug!("... Got second modline");
                        if let Ok(item_mod) = ItemMod::from_strs(last_line, line) {
                            mods.push(item_mod);
                        }
                        current_parsed_modline = None;
                    // If the line starts with `{`, then it is a mod
                    } else if line.starts_with("{") {
                        debug!("Got first modline...");
                        current_parsed_modline = Some(line);
                        continue;
                    } else if line == "--------" {
                        trace!("Item line separator");
                    }
                }
            };
        }

        // TODO
        let item = Item {
            base_name: "",
            item_name: ItemName::Normal,
            stats: vec![],
            ilvl: 1,
            sockets: "",
            mods,
        };
        Ok(item)
    }

    pub fn num_mods(&self) -> (usize, usize) {
        let mut prefixes = 0;
        let mut suffixes = 0;

        for mo in &self.mods {
            match mo.affix_type {
                AffixType::Prefix => prefixes += 1,
                AffixType::Suffix => suffixes += 1,
                AffixType::Implicit => {}
                AffixType::Unique => {}
            }
        }

        (prefixes, suffixes)
    }
}

impl<'a> ItemMod<'a> {
    fn from_strs(top_line: &'a str, bottom_line: &'a str) -> anyhow::Result<Self> {
        debug!("Parsing {top_line:?}");
        let q = ITEM_MOD_LINE_1_REGEX
            .captures(top_line)
            .context("top mod line regex failed")?;
        trace!(?q);

        //Parsing "{ Prefix Modifier \"Phantasm's\" (Tier: 3) — Defences, Evasion }"
        //[src/item.rs:116:9] q = Captures(
        //0: "{ Prefix Modifier \"Phantasm's\" (Tier: 3) — Defences, Evasion }",
        //"affix_type": "Prefix",
        //"name": "Phantasm's",
        //"tier": "3",
        //"affixes": "Defences, Evasion",
        //)

        debug!("Parsing {bottom_line:?}");
        let e = ITEM_MOD_LINE_2_REGEX
            .captures(bottom_line)
            .context("bottom mod line regex failed")?;
        trace!(?e);

        //Parsing "79(68-79)% increased Evasion Rating"
        //[src/item.rs:119:9] e = Captures(
        //0: "79(68-79)% increased Evasion Rating",
        //"before": "",
        //"value": "79",
        //"bot_roll": "68",
        //"top_roll": "79",
        //"end": "% increased Evasion Rating",
        //)

        let at = match q.name("affix_type").map(|x| x.as_str()) {
            Some("Prefix") => AffixType::Prefix,
            Some("Suffix") => AffixType::Suffix,
            Some("Unique") => AffixType::Unique,
            Some("Implicit") => AffixType::Implicit,
            // TODO parse error
            Some(at) => anyhow::bail!("Unknown affix type {at}"),
            None => anyhow::bail!("mod header missing affix type"),
        };

        let ant = match (q.name("name").map(|x| x.as_str()), q.name("tier")) {
            (None, None) => None,
            (None, Some(_)) => None, // TODO warn on these branches, (also do below)
            (Some(_), None) => None,
            (Some(name), Some(tier)) => {
                // Parse the tier into a number
                let tier = tier.as_str().parse()?;

                Some(AffixNameTier { name, tier })
            }
        };

        // Parse the value regex into a decimal
        let value = e.name("value").map(|x| x.as_str().parse()).transpose()?;

        // Turn "73(68-79)% increased Evasion Rating" into `68..79`
        let roll_range = match (e.name("bot_roll"), e.name("top_roll")) {
            (None, None) => None,
            (None, Some(_)) => None,
            (Some(_), None) => None,
            (Some(bot), Some(top)) => {
                let bot_parsed = bot.as_str().parse()?;
                let top_parsed = top.as_str().parse()?;

                Some(bot_parsed..top_parsed)
            }
        };

        // Turn Fire, Cold, Elemental into a vec of `Fire` `Cold `Elemental`
        let tags = q
            .name("affixes")
            .map(|x| x.as_str().split(", ").collect())
            .unwrap_or_default();

        let final_item = ItemMod {
            affix_type: at,
            affix_name_tier: ant,
            value,
            roll_range,
            full_text: bottom_line,
            tags,
            mod_qualifiers: "", //TODO
        };

        debug!(?final_item, "Created item");

        Ok(final_item)
    }
}

#[cfg(test)]
mod test {
    use tracing_test::traced_test;

    use super::*;

    fn run_item_mod(s: &str) -> anyhow::Result<ItemMod<'_>> {
        let mut parts = s.trim().lines();
        let x = parts.next().unwrap().trim();
        let y = parts.next().unwrap().trim();
        ItemMod::from_strs(x, y)
    }

    #[traced_test]
    #[test]
    fn mod_test_rare() {
        let modline = r#"
        { Prefix Modifier "Phantasm's" (Tier: 3) — Defences, Evasion }
        73(68-79)% increased Evasion Rating"#;

        let mods = run_item_mod(modline).unwrap();
        assert_eq!(mods.affix_type, AffixType::Prefix);
        assert_eq!(mods.value, Some(73.into()));
        assert_eq!(mods.roll_range, Some(68.into()..79.into()));

        let ant = mods.affix_name_tier.unwrap();
        assert_eq!(ant.name, "Phantasm's");
        assert_eq!(ant.tier, 3);

        assert_eq!(mods.tags, &["Defences", "Evasion"]);
    }

    #[traced_test]
    #[test]
    fn mod_test_unique() {
        let modline = r#"
        { Unique Modifier — Elemental, Fire, Resistance }
        +49(40-50)% to Fire Resistance"#;

        let mods = run_item_mod(modline).unwrap();
        assert_eq!(mods.affix_type, AffixType::Unique);
        assert_eq!(mods.value, Some(49.into()));
        assert_eq!(mods.roll_range, Some(40.into()..50.into()));
        assert!(mods.affix_name_tier.is_none());

        assert_eq!(mods.tags, &["Elemental", "Fire", "Resistance"]);
    }

    #[traced_test]
    #[test]
    fn mod_test_unique_no_roll() {
        let modline = r#"
        { Unique Modifier — Mana }
        60% increased Mana Regeneration Rate"#;

        let mods = run_item_mod(modline).unwrap();
        assert_eq!(mods.affix_type, AffixType::Unique);
        assert_eq!(mods.value, Some(60.into()));
        assert!(mods.roll_range.is_none());
        assert!(mods.affix_name_tier.is_none());
        assert_eq!(mods.tags, &["Mana"]);
    }

    #[traced_test]
    #[test]
    fn all_item_mod_integration_tests() {
        let item_texts = [
            (1, 3, include_str!("../tests/example_items/amulet.txt")),
            (0, 0, include_str!("../tests/example_items/unique.txt")),
            (1, 1, include_str!("../tests/example_items/magic_helm.txt")),
        ];

        for (num_pre, num_suf, text) in item_texts {
            let text = text.trim();

            let t = Item::from_str(text).unwrap();
            let (np, ns) = t.num_mods();
            assert_eq!(num_pre, np);
            assert_eq!(num_suf, ns);
        }
    }

    #[test]
    fn every_example_item_parses() {
        let files = crate::test_support::example_item_files();
        assert!(
            !files.is_empty(),
            "no .txt files found under tests/example_items — add pasted tooltips there"
        );
        let mut failures = Vec::new();
        for path in &files {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    failures.push(format!("{}: read: {e}", path.display()));
                    continue;
                }
            };
            if let Err(e) = Item::from_str(&text) {
                failures.push(format!("{}: {e}", path.display()));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} example items failed to parse:\n{}",
            failures.len(),
            files.len(),
            failures.join("\n")
        );
    }
}
