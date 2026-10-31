//! The stdin REPL: `little_oil` with no subcommand.
//!
//! The clap CLI in [`crate::cli`] is the primary surface; the interactive loop
//! keeps the historical commands (`chrome`, `mchrome`, `pull`, `div`) working.

use super::App;
use crate::auto_roll::{self, AutoRollConfig, AutoRollMod};
use crate::save_config;
use std::io::{self, BufRead};
use std::path::Path;

fn split_space(input: &str) -> (&str, &str) {
    for (i, _c) in input.char_indices() {
        if input.as_bytes()[i] == b' ' {
            return (&input[0..i], &input[i + 1..]);
        }
    }
    (input, "")
}

const HELP: &str = r#"
help: Show this menu
version: Print version and exit
empty: Empty the inventory into the stash (ctrl+left click)
emptyr: Empty the inventory into the stash (ctrl+right click)
set-region <inventory|stash|window|map>: Select and save a screen region
calibrate-pointer: Measure pointer scale (run once per machine)
calibrate-stash: Calibrate the 24x24 quad tab grid (3 positions, base + search capture each)
calibrate-map: Calibrate the 12x7 map tab grid (3 positions, base + search capture each)
calibrate-point <name>: Drag a small box in the GUI picker and save it as a named clickable point
calibrate-currency: Calibrate the 10 currency slots (transmute, alt, annul, chance, augment, regal, chaos, scour, alchemy, exalt)
click <name>: Click a calibrated point (e.g. filter, chaos)
click map <col> <row>: Click a cell in the calibrated map grid
stash <click|copy> [times]: Act on highlighted quad-tab cells
pull <delay>: Change delay for pulling out of quad tab
div <delay>: Change delay for div macro
chrome <file> <times>: Open an auto-roll file, with name <file>, and roll item <times>
mchrome <file>: Create example chrome file with name <file>. To be used with chrome later.

Press CTRL + C to quit this program.
"#;

impl App {
    pub(crate) fn command_line(&self) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let (cmd, rest) = split_space(&line);
            match cmd {
                "pull" | "div" => match rest.parse::<u64>() {
                    Ok(x) => {
                        {
                            let mut s = self.settings.write();
                            match cmd {
                                "pull" => s.pull_delay = x,
                                "div" => s.div_delay = x,
                                _ => unreachable!(),
                            }
                        }
                        println!("{cmd} delay is {x}");
                        if let Err(e) = self.save_settings() {
                            println!("could not save config: {e:#}");
                        }
                    }
                    Err(_) => println!("invalid delay: {rest}"),
                },
                "chrome" => {
                    let (file, times) = split_space(rest);
                    println!("Loading chrome file {}", file);

                    let Ok(times) = times.parse::<i64>() else {
                        println!("invalid times: {times}");
                        continue;
                    };
                    match auto_roll::auto_roll(self, file, times) {
                        None => println!("failed to roll"),
                        Some(res) => {
                            println!("{:?}", res);
                        }
                    }
                }
                "mchrome" => {
                    println!("Making chrome file {}", rest);

                    if let Err(e) = save_config(
                        Path::new(rest),
                        &AutoRollConfig {
                            auto_aug_regal: false,
                            item_name: "Medium Cluster Jewel".to_string(),
                            any_two_t1: false,
                            needs_prefix_and_suffix: false,
                            mods: vec![
                                AutoRollMod {
                                    name: "heraldry".into(),
                                    is_prefix: true,
                                },
                                AutoRollMod {
                                    name: "harbinger".into(),
                                    is_prefix: true,
                                },
                                AutoRollMod {
                                    name: "endbringer".into(),
                                    is_prefix: true,
                                },
                            ],
                        },
                    ) {
                        println!("could not save chrome file: {e}");
                    }
                }
                "help" => {
                    println!("Available Commands: {}", HELP);
                }
                _ => println!("Unknown command"),
            }
        }
    }
}
