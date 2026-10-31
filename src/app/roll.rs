//! Rolling: the chance/scour orb loop and the persistent roll log.

use super::App;
use std::io::{self, Write};

impl App {
    pub(crate) fn chance(&self) -> anyhow::Result<()> {
        let chance = self.point_pos(&["chance"], (237, 292));
        let scour = self.point_pos(&["scour"], (169, 472));
        let slot = self.point_pos(&["slot"], (323, 522));
        let sleep_click = 30;
        let sleep_read = 250;

        for _ in 1..10 {
            self.click_right(chance.0, chance.1);
            self.backend.sleep_ms(sleep_click);
            self.click(slot.0, slot.1);
            self.backend.sleep_ms(sleep_read);

            self.click_right(scour.0, scour.1);
            self.backend.sleep_ms(sleep_click);
            self.click(slot.0, slot.1);
            self.backend.sleep_ms(sleep_read);
        }

        Ok(())
    }

    /// Append one rolled item's tooltip to the persistent roll log
    /// (`$XDG_CONFIG_HOME/little_oil/rolls.log`, JSONL). Logging never fails the
    /// roll: errors are only traced.
    pub(crate) fn log_roll_item(&self, source: &str, item_text: &str) {
        #[cfg(test)]
        let redirected = self.roll_log_override.clone();
        #[cfg(not(test))]
        let redirected: Option<std::path::PathBuf> = None;
        let path = redirected.map_or_else(crate::rolls_log_path, Ok);
        match path {
            Ok(path) => {
                if let Err(e) = append_roll_log(&path, source, item_text) {
                    tracing::warn!(?e, "could not write roll log");
                }
            }
            Err(e) => tracing::warn!(?e, "could not determine roll log path"),
        }
    }
}

/// Append one JSONL record `{"time": <unix secs>, "source": <chrome file>, "item": <tooltip>}`
/// to `path`, creating it if missing. `item_text` is multi-line; serde_json
/// escapes it, so the file stays one JSON object per line.
fn append_roll_log(path: &std::path::Path, source: &str, item_text: &str) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let rec = serde_json::json!({
        "time": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "source": source,
        "item": item_text,
    });
    let mut line = serde_json::to_string(&rec).map_err(io::Error::other)?;
    line.push('\n');
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::InputButton;
    use crate::test_support::{Ev, test_app};
    use crate::{NamedPoint, ScreenRegion};

    #[test]
    fn chance_alternates_chance_and_scour_nine_times() {
        let mut settings = crate::default_settings();
        let point = |name: &str, x: u32| NamedPoint {
            name: name.to_string(),
            region: ScreenRegion {
                x,
                y: x,
                width: 20,
                height: 20,
            },
        };
        // Centres: chance (20, 20), scour (40, 40), slot (60, 60).
        settings.points = Some(vec![
            point("chance", 10),
            point("scour", 30),
            point("slot", 50),
        ]);
        let (app, fake) = test_app(settings);

        app.chance().expect("chance an item");

        let click = |button: InputButton, at: (i32, i32), settle: u64| {
            vec![
                Ev::Move(at.0, at.1),
                Ev::Sleep(30),
                Ev::Button(button, true),
                Ev::Sleep(10),
                Ev::Button(button, false),
                Ev::Sleep(settle),
            ]
        };
        let mut expected = Vec::new();
        for _ in 1..10 {
            expected.extend(click(InputButton::Right, (20, 20), 30)); // chance
            expected.extend(click(InputButton::Left, (60, 60), 250)); // slot
            expected.extend(click(InputButton::Right, (40, 40), 30)); // scour
            expected.extend(click(InputButton::Left, (60, 60), 250)); // slot
        }
        assert_eq!(
            fake.events(),
            expected,
            "nine chance/scour passes with the configured settle times"
        );
    }

    #[test]
    fn append_roll_log_writes_jsonl_records() {
        let path =
            std::env::temp_dir().join(format!("little_oil_roll_log_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        append_roll_log(&path, "chrome_a.json", "line1\nline2").unwrap();
        append_roll_log(&path, "chrome_b.json", "second item").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON object per line");
        for line in &lines {
            assert!(!line.is_empty());
        }

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["source"], "chrome_a.json");
        assert_eq!(first["item"], "line1\nline2");
        assert!(first["time"].is_number());

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["source"], "chrome_b.json");
        assert_eq!(second["item"], "second item");
    }
}
