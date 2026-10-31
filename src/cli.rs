//! The command-line surface.
//!
//! clap parses the arguments; the command bodies stay where they were. The
//! zero-argument macro commands (`empty`, `emptyr`, `reset_inv`, `stash-copy`,
//! `chance`, `tally`) are generated from [`crate::actions::ACTIONS`], so the
//! CLI, the GUI and the tray can never drift apart. Commands that take
//! arguments (`sort`, `roll`, `click`, `stash`, `chaos`, `set-region`, the
//! calibrators) are declared explicitly below with their historical names.
//!
//! With no subcommand the interactive REPL starts.

use std::ffi::OsString;
use std::sync::Arc;

use anyhow::bail;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::App;
use crate::actions::{ACTIONS, find};
use crate::stash_grid::{MAP_COLS, MAP_ROWS};

/// `little_oil` — Path of Exile automation.
#[derive(Parser)]
#[command(
    name = "little_oil",
    version,
    about = "Path of Exile automation (inventory, stash, map, rolls, chaos recipe)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the GUI panel (settings, calibration, macros)
    Gui,
    /// Print the active config
    Config,
    /// Print the version and exit
    Version,
    /// Diagnose why something isn't working
    Doctor,
    /// Click every highlighted quad-tab cell
    Sort {
        /// Maximum number of cells to click
        #[arg(default_value_t = 40)]
        times: u32,
    },
    /// Roll an item from an auto-roll file
    Roll {
        /// Auto-roll (chrome) file
        file: String,
        /// Number of rolls
        times: i64,
    },
    /// Click a calibrated point, or a map cell
    Click {
        #[command(subcommand)]
        target: Option<ClickTarget>,
        /// Name of a calibrated point (e.g. filter, chaos)
        name: Option<String>,
    },
    /// Act on highlighted quad-tab cells
    Stash {
        #[command(subcommand)]
        mode: Option<StashMode>,
    },
    /// Run the chaos recipe
    Chaos {
        /// Number of recipe sets
        #[arg(default_value_t = 1)]
        amount: usize,
    },
    /// Measure the pointer scale (run once per machine)
    CalibratePointer,
    /// Calibrate the 24x24 quad tab grid
    CalibrateStash,
    /// Calibrate the 12x7 map tab grid
    CalibrateMap,
    /// Calibrate one named clickable point
    CalibratePoint {
        /// Point name (e.g. filter, chaos, augment)
        name: String,
    },
    /// Calibrate the ten currency slots
    CalibrateCurrency,
    /// Pick and save a screen region
    SetRegion {
        /// Which region to select
        #[arg(value_enum)]
        region: RegionName,
    },
    /// Print ready-to-paste keybind snippets for the current desktop
    Keybinds,
}

#[derive(Subcommand)]
enum ClickTarget {
    /// Click a cell of the calibrated map grid
    Map {
        /// Column (0-based)
        col: usize,
        /// Row (0-based)
        row: usize,
    },
}

#[derive(Subcommand)]
enum StashMode {
    /// Left-click every highlighted cell
    Click {
        /// Maximum number of cells to click
        #[arg(default_value_t = 40)]
        times: u32,
    },
    /// Hover every highlighted cell and copy its item text
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RegionName {
    Inventory,
    Stash,
    Window,
    Map,
}

impl RegionName {
    fn as_str(self) -> &'static str {
        match self {
            RegionName::Inventory => "inventory",
            RegionName::Stash => "stash",
            RegionName::Window => "window",
            RegionName::Map => "map",
        }
    }
}

/// Subcommand ids that have an explicit declaration above and therefore must
/// not be generated from the registry.
const EXPLICIT_IDS: &[&str] = &[
    "gui",
    "config",
    "version",
    "doctor",
    "sort",
    "roll",
    "click",
    "stash",
    "chaos",
    "calibrate-pointer",
    "calibrate-stash",
    "calibrate-map",
    "calibrate-point",
    "calibrate-currency",
    "set-region",
    "keybinds",
];

/// The full clap command: the explicit subcommands plus a zero-argument
/// subcommand for every registry id that does not have one.
pub(crate) fn command() -> clap::Command {
    let mut cmd = Cli::command();
    for def in ACTIONS {
        if EXPLICIT_IDS.contains(&def.id) {
            continue;
        }
        cmd = cmd.subcommand(clap::Command::new(def.id).about(def.help));
    }
    cmd
}

/// Parse `args` and run the requested command. No subcommand starts the REPL.
///
/// Parse errors and `--help`/`--version` are handled by clap itself (it prints
/// and exits with the conventional status).
pub(crate) fn run(app: App, args: &[String]) -> anyhow::Result<()> {
    let mut argv: Vec<OsString> = Vec::with_capacity(args.len() + 1);
    argv.push(OsString::from("little_oil"));
    argv.extend(args.iter().map(OsString::from));
    let matches = command()
        .try_get_matches_from(argv)
        .unwrap_or_else(|err| err.exit());

    let app = Arc::new(app);
    let Some((name, sub)) = matches.subcommand() else {
        app.command_line();
        return Ok(());
    };

    match name {
        "gui" => crate::gui::run(Arc::clone(&app)),
        "config" => {
            println!("{}", serde_json::to_string_pretty(&*app.settings.read())?);
            Ok(())
        }
        "version" => {
            println!("little_oil v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "doctor" => doctor(&app),
        "sort" => {
            let times = sub.get_one::<u32>("times").copied().unwrap_or(40);
            app.sort_quad(times)
        }
        "roll" => {
            let file = sub
                .get_one::<String>("file")
                .expect("clap requires the file argument");
            let times = sub
                .get_one::<i64>("times")
                .expect("clap requires the times argument");
            let _ = crate::auto_roll::auto_roll(&app, file, *times);
            Ok(())
        }
        "click" => {
            if let Some(("map", map)) = sub.subcommand() {
                let col = map.get_one::<usize>("col").copied().unwrap_or(0);
                let row = map.get_one::<usize>("row").copied().unwrap_or(0);
                check_map_cell(col, row)?;
                app.click_map_cell(col, row)
            } else if let Some(point) = sub.get_one::<String>("name") {
                app.click_point(point)
            } else {
                println!("Usage: little_oil click <name> | click map <col> <row>");
                println!("  <name>    A calibrated point: filter, or a currency (chaos, alch, …)");
                Ok(())
            }
        }
        "stash" => match sub.subcommand() {
            Some(("click", click)) => {
                let times = click.get_one::<u32>("times").copied().unwrap_or(40);
                app.sort_quad(times)
            }
            Some(("copy", _)) => app.stash_copy(),
            _ => {
                println!("Usage: little_oil stash <click|copy> [times]");
                println!(
                    "  click <times>  Left-click every highlighted cell (hold Ctrl to pull, Shift to identify)"
                );
                println!(
                    "  copy           Hover every highlighted cell and Ctrl+Alt+C it, printing unique items"
                );
                Ok(())
            }
        },
        "chaos" => {
            let amount = sub.get_one::<usize>("amount").copied().unwrap_or(1);
            let recipe = app
                .settings
                .read()
                .chaos_recipe_settings
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No chaos recipe config found"))?;
            crate::chaos_recipe::do_recipe(&app, &recipe, amount)
        }
        "calibrate-pointer" => calibrate_pointer(&app),
        "calibrate-stash" => app.calibrate_stash(),
        "calibrate-map" => app.calibrate_map(),
        "calibrate-point" => {
            let name = sub
                .get_one::<String>("name")
                .expect("clap requires the point name");
            app.calibrate_point(name)
        }
        "calibrate-currency" => calibrate_currency(&app),
        "set-region" => {
            let region = sub
                .get_one::<RegionName>("region")
                .expect("clap validates the region name");
            app.set_region(region.as_str())
        }
        "keybinds" => {
            let desktop = app.desktop();
            println!("# {}", crate::keybinds::instructions(desktop));
            println!(
                "{}",
                crate::keybinds::snippet(desktop, &crate::keybinds::exe_path())
            );
            Ok(())
        }
        other => match find(other) {
            Some(def) => (def.run)(&app),
            None => {
                println!("Invalid command: {other}");
                Ok(())
            }
        },
    }
}

/// Reject out-of-range map cells before any clicking happens (the CLI used to
/// check this in its hand-written dispatch).
fn check_map_cell(col: usize, row: usize) -> anyhow::Result<()> {
    if col >= MAP_COLS || row >= MAP_ROWS {
        bail!("map cell ({col}, {row}) out of range — cols 0..{MAP_COLS}, rows 0..{MAP_ROWS}");
    }
    Ok(())
}

fn doctor(app: &App) -> anyhow::Result<()> {
    let checks = crate::health::run(app);
    for c in &checks {
        let tag = match c.status {
            crate::health::CheckStatus::Good => "[ OK ]",
            crate::health::CheckStatus::Warn => "[WARN]",
            crate::health::CheckStatus::Error => "[ ERR]",
        };
        println!("{tag} {:<20} {}", c.name, c.message);
    }
    let errors = checks
        .iter()
        .filter(|c| c.status == crate::health::CheckStatus::Error)
        .count();
    if errors > 0 {
        bail!("{errors} check(s) failed");
    }
    Ok(())
}

/// Platform-gated: only the Linux wlr backend supports pointer calibration.
fn calibrate_pointer(app: &App) -> anyhow::Result<()> {
    // Platforms with absolute pointer positioning (niri on Linux, Windows)
    // don't need pointer_scale calibration.
    if app.desktop().uses_absolute_pointer() {
        bail!(
            "calibrate-pointer is not needed on this platform — pointer \
             control uses absolute coordinates. Set pointer_scale manually \
             in config only if you see coordinate drift."
        );
    }
    #[cfg(target_os = "linux")]
    {
        app.calibrate_pointer()
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!(
            "calibrate-pointer is not supported on this platform — pointer control uses absolute coordinates."
        )
    }
}

fn calibrate_currency(app: &App) -> anyhow::Result<()> {
    for name in [
        "transmute",
        "alt",
        "annul",
        "chance",
        "augment",
        "regal",
        "chaos",
        "scour",
        "alchemy",
        "exalt",
    ] {
        crate::app::prompt_enter(
            app,
            &format!(
                "Open the currency tab if it isn't already. Next point: {name} — press Enter, then drag a box around its slot in the picker."
            ),
        )?;
        app.calibrate_point(name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<clap::ArgMatches, clap::Error> {
        command().try_get_matches_from(args)
    }

    /// Every registry id is a real subcommand, so `little_oil <id>` works for
    /// each action in the table.
    #[test]
    fn every_registry_id_is_a_subcommand() {
        for def in ACTIONS {
            let matches = parse(&["little_oil", def.id]).unwrap_or_else(|e| {
                panic!("registry id {:?} must parse as a subcommand: {e}", def.id)
            });
            assert_eq!(
                matches.subcommand_name(),
                Some(def.id),
                "registry id {:?} must dispatch to itself",
                def.id
            );
        }
    }

    #[test]
    fn help_lists_every_registry_id() {
        let help = command().render_help().to_string();
        for def in ACTIONS {
            assert!(
                help.contains(def.id),
                "`--help` must list registry id {:?}:\n{help}",
                def.id
            );
        }
    }

    #[test]
    fn no_subcommand_is_accepted_for_the_repl() {
        let matches = parse(&["little_oil"]).unwrap();
        assert!(matches.subcommand().is_none());
    }

    #[test]
    fn explicit_subcommands_keep_their_arguments() {
        let sort = parse(&["little_oil", "sort", "12"]).unwrap();
        assert_eq!(
            sort.subcommand_matches("sort")
                .unwrap()
                .get_one::<u32>("times"),
            Some(&12)
        );

        let click = parse(&["little_oil", "click", "map", "3", "4"]).unwrap();
        let map = click
            .subcommand_matches("click")
            .unwrap()
            .subcommand_matches("map")
            .expect("`click map` is a subcommand");
        assert_eq!(map.get_one::<usize>("col"), Some(&3));
        assert_eq!(map.get_one::<usize>("row"), Some(&4));

        let point = parse(&["little_oil", "click", "filter"]).unwrap();
        assert_eq!(
            point
                .subcommand_matches("click")
                .unwrap()
                .get_one::<String>("name")
                .map(String::as_str),
            Some("filter")
        );

        let stash = parse(&["little_oil", "stash", "click", "5"]).unwrap();
        assert_eq!(
            stash
                .subcommand_matches("stash")
                .unwrap()
                .subcommand_matches("click")
                .unwrap()
                .get_one::<u32>("times"),
            Some(&5)
        );
        parse(&["little_oil", "stash", "copy"]).unwrap();

        let region = parse(&["little_oil", "set-region", "inventory"]).unwrap();
        assert_eq!(
            region
                .subcommand_matches("set-region")
                .unwrap()
                .get_one::<RegionName>("region"),
            Some(&RegionName::Inventory)
        );

        parse(&["little_oil", "roll", "rolls/x.json", "3"]).unwrap();
        parse(&["little_oil", "chaos", "5"]).unwrap();
        parse(&["little_oil", "calibrate-point", "chaos"]).unwrap();
    }

    #[test]
    fn map_cells_are_range_checked() {
        assert!(check_map_cell(0, 0).is_ok());
        assert!(check_map_cell(MAP_COLS - 1, MAP_ROWS - 1).is_ok());
        let err = check_map_cell(MAP_COLS, 0).expect_err("column out of range");
        assert!(
            format!("{err:#}").contains("out of range"),
            "the error must explain the valid range, got: {err:#}"
        );
        assert!(check_map_cell(0, MAP_ROWS).is_err());
    }

    #[test]
    fn set_region_rejects_an_unknown_name() {
        assert!(
            parse(&["little_oil", "set-region", "x"]).is_err(),
            "`set-region x` must be a usage error"
        );
    }

    /// The explicit defaults must match the registry behavior they stand in
    /// for: `sort` defaults to 40 clicks and `chaos` to one recipe set.
    #[test]
    fn explicit_defaults_match_the_registry() {
        let sort = parse(&["little_oil", "sort"]).unwrap();
        assert_eq!(
            sort.subcommand_matches("sort")
                .unwrap()
                .get_one::<u32>("times"),
            Some(&40)
        );
        let chaos = parse(&["little_oil", "chaos"]).unwrap();
        assert_eq!(
            chaos
                .subcommand_matches("chaos")
                .unwrap()
                .get_one::<usize>("amount"),
            Some(&1)
        );
    }
}
