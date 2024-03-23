use std::{fmt::Display, ops::Range, str::FromStr, thread::current};

use anyhow::Context;
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use tracing::{debug, trace};

#[derive(Debug)]
pub struct Item<'a> {
    pub base_name: &'a str,
    pub item_name: ItemName<'a>,

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
pub enum ItemName<'a> {
    /// Gems, etc
    Other(&'a str),
    Normal,
    Magic {
        prefix: &'a str,
        suffix: &'a str,
    },
    Rare(&'a str),
    Unique(&'a str),
}

impl<'a> Display for ItemName<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemName::Other(name) => write!(f, "?: {name}")?,
            ItemName::Normal => write!(f, "N")?,
            ItemName::Magic { prefix, suffix } => match (*prefix, *suffix) {
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
    affix_type: AffixType,

    /// Contains the tier if it's a rare mod
    affix_name_tier: Option<AffixNameTier<'a>>,

    value: Option<Decimal>,
    roll_range: Option<Range<Decimal>>,

    /// Tags for catalysts: things like Defenses, Evasion, Fire
    tags: Vec<&'a str>,
    /// is fractured, etc
    mod_qualifiers: &'a str,
}

#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub enum AffixType {
    Prefix,
    Suffix,
    Unique,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AffixNameTier<'a> {
    name: &'a str,
    tier: i32,
}

// { Prefix Modifier "Phantasm's" (Tier: 3) — Defences, Evasion }
// Example:
//                      {     Prefix          Modifier    "Phantasm's  "  (Tier:      3        )      —  Defences, Evasion        }
//                         vvvvvvvvvvvvvvvvv               vvvvvvvvvvvv           vvvvvvvvvvvvv            vvvvvvvvvvv
const IMR_1: &str = r#"\{ (?P<affix_type>\w+) Modifier (?:"(?P<name>.+)" \(Tier: (?P<tier>\d+)\) )?(?:— (?P<affixes>.*) )?\}"#;
const ITEM_MOD_LINE_1_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(IMR_1).unwrap());

const IMR_2: &str = r#"(?P<before>[^\d]*)(?P<value>\d+(?:\.\d+)?)?(?:\((?P<bot_roll>\d+(?:\.\d+)?)-(?P<top_roll>\d+(?:\.\d+)?)\))?(?P<end>.*)"#;
const ITEM_MOD_LINE_2_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(IMR_2).unwrap());

const CR: &str = r#"(?P<left>[^:]+):(?P<right>.+)"#;
const COLON_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(CR).unwrap());

enum ItemParseSections {
    ItemClass,
    ItemRarity,
    ItemName,

    ItemStats,

    //ItemRequirements,
    //ItemSockets,
    //ItemLevel,
    ItemMods,
}

impl<'a> Item<'a> {
    #[tracing::instrument]
    fn from_str(source: &'a str) -> anyhow::Result<Self> {
        let cur_parser_state = ItemParseSections::ItemClass;

        let mut current_parsed_modline = None;
        let mut mods = vec![];
        for line in source.trim().lines() {
            let line = line.trim();

            match cur_parser_state {
                //First state: What are we looking at?
                ItemParseSections::ItemClass => {
                    let res = COLON_REGEX
                        .captures(line)
                        .context("should match first line")?;

                    assert_eq!(
                        res.name("left")
                            .context("left part of first line")?
                            .as_str(),
                        "Item Class"
                    );
                }
                ItemParseSections::ItemRarity => todo!(),
                ItemParseSections::ItemName => todo!(),
                ItemParseSections::ItemStats => todo!(),
                ItemParseSections::ItemMods => todo!(),
            };

            // If we have a mod line saved, then combine that with the current line.
            // These two lines make up a single mod. ex:
            //
            // last:  { Unique Modifier — Elemental, Fire, Resistance }
            // cur:   +49(40-50)% to Fire Resistance
            if let Some(last_line) = current_parsed_modline {
                debug!("... Got second modline");
                let item_mod = ItemMod::from_strs(last_line, line)?;
                mods.push(item_mod);
            // If the line starts with `{`, then it is a mod
            } else if line.starts_with("{") {
                debug!("Got first modline...");
                current_parsed_modline = Some(line);
                continue;
            } else if line == "--------" {
                trace!("Item line separator");
            }
        }

        todo!()
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
            // TODO parse error
            Some(at) => anyhow::bail!("Unknown affix type {at}"),
            None => unreachable!("required in regex pattern"),
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
    fn item_mods() {
        let _ = [
            r#"
                { Prefix Modifier "Phantasm's" (Tier: 3) — Defences, Evasion }
                79(68-79)% increased Evasion Rating
            "#,
            r#"
                { Unique Modifier — Elemental, Fire, Resistance }
                +49(40-50)% to Fire Resistance
            "#,
            r#"
                { Unique Modifier — Mana }
                60% increased Mana Regeneration Rate
            "#,
            r#"
                { Unique Modifier }
                17(14-20)% increased Quantity of Items found
            "#,
            r#"
                { Unique Modifier — Speed }
                10% increased Movement Speed
            "#,
            r#"
                { Suffix Modifier "of the Thunderhead" (Tier: 5) — Elemental, Lightning, Resistance }
                +29(24-29)% to Lightning Resistance (fractured)
            "#,
        ];
    }
}
