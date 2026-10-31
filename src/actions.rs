//! The single action registry.
//!
//! Every user-facing macro is defined once here and drives all surfaces: the
//! GUI Actions tab, the Windows tray menu and global hotkeys, and the CLI
//! (`little_oil <id>`). Adding an action means adding one entry — the CLI
//! subcommand, the button, the tray item and the default hotkey follow.
//!
//! Explicit CLI subcommands (`sort`, `chaos`, …) keep their own argument
//! parsing (see [`crate::cli`]); the ids that only need a zero-argument
//! subcommand are generated from this table.

use crate::App;

/// A macro action callable from every surface.
pub(crate) type Action = fn(&App) -> anyhow::Result<()>;

/// One registry entry.
pub(crate) struct ActionDef {
    /// CLI subcommand name and tray/menu id (`^[a-z][a-z_-]*$`).
    pub id: &'static str,
    /// Human label shown on the GUI button and tray item.
    pub label: &'static str,
    /// One-line help: GUI hover text and CLI `--help` text.
    pub help: &'static str,
    /// Default global accelerator (Windows), configurable in Settings.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub default_hotkey: Option<&'static str>,
    pub run: Action,
}

fn run_empty(app: &App) -> anyhow::Result<()> {
    app.empty_inv()
}

fn run_emptyr(app: &App) -> anyhow::Result<()> {
    app.empty_inv_right()
}

fn run_reset_inv(app: &App) -> anyhow::Result<()> {
    app.reset_inv_colors()
}

fn run_sort(app: &App) -> anyhow::Result<()> {
    app.sort_quad(40)
}

fn run_stash_copy(app: &App) -> anyhow::Result<()> {
    app.stash_copy()
}

fn run_chance(app: &App) -> anyhow::Result<()> {
    app.chance()
}

fn run_chaos(app: &App) -> anyhow::Result<()> {
    let recipe = app
        .settings
        .read()
        .chaos_recipe_settings
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No chaos recipe config found"))?;
    crate::chaos_recipe::do_recipe(app, &recipe, 1)
}

fn run_tally(app: &App) -> anyhow::Result<()> {
    let recipe = app
        .settings
        .read()
        .chaos_recipe_settings
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No chaos recipe config found"))?;
    crate::chaos_recipe::get_tally(app, &recipe)
}

/// Every action, in the order the GUI and tray show them and the order in
/// which duplicate hotkeys are resolved (first wins).
pub(crate) const ACTIONS: &[ActionDef] = &[
    ActionDef {
        id: "empty",
        label: "Empty inventory (left)",
        help: "Ctrl+click every occupied slot, up to 3 verify passes",
        default_hotkey: Some("F11"),
        run: run_empty,
    },
    ActionDef {
        id: "emptyr",
        label: "Empty inventory (right)",
        help: "Same as empty, but right-click every occupied slot",
        default_hotkey: Some("F10"),
        run: run_emptyr,
    },
    ActionDef {
        id: "reset_inv",
        label: "Recalibrate inventory colors",
        help: "Re-sample the 60 empty-slot probe colors",
        default_hotkey: Some("F12"),
        run: run_reset_inv,
    },
    ActionDef {
        id: "sort",
        label: "Sort quad tab",
        help: "Click up to 40 highlighted quad-tab cells",
        default_hotkey: None,
        run: run_sort,
    },
    ActionDef {
        id: "stash-copy",
        label: "Copy stash items",
        help: "Ctrl+Alt+C every highlighted cell and print the unique items",
        default_hotkey: None,
        run: run_stash_copy,
    },
    ActionDef {
        id: "chance",
        label: "Chance an item",
        help: "Alternate chance and scour orbs on the calibrated slot",
        default_hotkey: None,
        run: run_chance,
    },
    ActionDef {
        id: "chaos",
        label: "Chaos recipe (1 set)",
        help: "Run one chaos recipe set",
        default_hotkey: None,
        run: run_chaos,
    },
    ActionDef {
        id: "tally",
        label: "Tally chaos recipe",
        help: "Count the items already collected for the chaos recipe",
        default_hotkey: None,
        run: run_tally,
    },
];

/// Look up a registry entry by id.
pub(crate) fn find(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|def| def.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids are CLI subcommand names and tray menu ids, so they must stay in the
    /// conservative shape every surface accepts.
    #[test]
    fn ids_are_unique_and_well_formed() {
        let id_re = regex::Regex::new(r"^[a-z][a-z_-]*$").unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for def in ACTIONS {
            assert!(
                id_re.is_match(def.id),
                "action id {:?} must match ^[a-z][a-z_-]*$",
                def.id
            );
            assert!(seen.insert(def.id), "duplicate action id {:?}", def.id);
        }
    }

    #[test]
    fn default_hotkeys_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for def in ACTIONS {
            if let Some(hotkey) = def.default_hotkey {
                assert!(
                    seen.insert(hotkey),
                    "default hotkey {hotkey:?} is used by more than one action"
                );
            }
        }
    }

    #[test]
    fn registry_contains_the_documented_actions_in_order() {
        let ids: Vec<&str> = ACTIONS.iter().map(|def| def.id).collect();
        assert_eq!(
            ids,
            vec![
                "empty",
                "emptyr",
                "reset_inv",
                "sort",
                "stash-copy",
                "chance",
                "chaos",
                "tally",
            ]
        );
        let f10 = find("emptyr").unwrap();
        assert_eq!(f10.default_hotkey, Some("F10"));
        assert_eq!(find("empty").unwrap().default_hotkey, Some("F11"));
        assert_eq!(find("reset_inv").unwrap().default_hotkey, Some("F12"));
        assert!(find("sort").unwrap().default_hotkey.is_none());
        assert!(find("nope").is_none());
    }

    /// The default accelerators must be accepted by the hotkey parser Windows
    /// uses. The crate is a Windows-only dependency, so the parse check only
    /// exists there; the shape check above runs everywhere.
    #[cfg(target_os = "windows")]
    #[test]
    fn default_hotkeys_parse() {
        use std::str::FromStr;
        for def in ACTIONS {
            if let Some(hotkey) = def.default_hotkey {
                global_hotkey::hotkey::HotKey::from_str(hotkey)
                    .unwrap_or_else(|e| panic!("default hotkey {hotkey:?} must parse: {e}"));
            }
        }
    }
}
