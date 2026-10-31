//! Ready-to-paste keybind snippets for the detected desktop.
//!
//! Wayland compositors forbid global key grabs, so on Linux the user binds
//! each registry action with a default hotkey in their compositor's config.
//! [`snippet`] renders that config from [`crate::actions::ACTIONS`] and the
//! current executable path; `little_oil keybinds` prints it and the GUI
//! Settings tab shows it with a Copy button.
//!
//! Desktops without a keybind config file (KDE, GNOME) get a shortcut/command
//! table to enter in their system settings.

use crate::actions::ACTIONS;
use crate::platform::Desktop;

/// `(hotkey, action id)` for every registry action that has a default
/// accelerator, sorted by hotkey (F10, F11, F12).
pub(crate) fn bindings() -> Vec<(&'static str, &'static str)> {
    let mut bindings: Vec<(&'static str, &'static str)> = ACTIONS
        .iter()
        .filter_map(|def| def.default_hotkey.map(|hotkey| (hotkey, def.id)))
        .collect();
    bindings.sort_by_key(|(hotkey, _)| *hotkey);
    bindings
}

/// Path to the running executable — the command the keybinds invoke.
pub(crate) fn exe_path() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "little_oil".to_string())
}

/// One line telling the user where to put the snippet.
pub(crate) fn instructions(desktop: Desktop) -> &'static str {
    match desktop {
        Desktop::Hyprland => "Paste into ~/.config/hypr/hyprland.conf, then run: hyprctl reload",
        Desktop::Niri => {
            "Paste inside the binds { } block of ~/.config/niri/config.kdl, then run: niri msg action reload-config"
        }
        Desktop::Sway => "Paste into ~/.config/sway/config, then run: swaymsg reload",
        Desktop::Kde => {
            "System Settings → Keyboard → Shortcuts → Add → Command or Script…, then bind each shortcut"
        }
        Desktop::Gnome => "Settings → Keyboard → Custom Shortcuts → +, then bind each shortcut",
        Desktop::Windows => {
            "Global hotkeys are built in on Windows — edit them in the GUI Settings tab"
        }
        Desktop::OtherWayland => "Bind these commands in your compositor's keybind configuration",
    }
}

/// Quote `text` for config formats that need it (paths with spaces).
fn quote(text: &str) -> String {
    if text.contains(' ') || text.contains('"') {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        text.to_string()
    }
}

/// The keybind snippet for `desktop`, invoking `exe` for each action.
pub(crate) fn snippet(desktop: Desktop, exe: &str) -> String {
    let bindings = bindings();
    match desktop {
        Desktop::Hyprland => bindings
            .iter()
            .map(|(hotkey, id)| format!("bind = , {hotkey}, exec, {} {id}", quote(exe)))
            .collect::<Vec<_>>()
            .join("\n"),
        Desktop::Niri => {
            let mut lines = vec!["binds {".to_string()];
            for (hotkey, id) in &bindings {
                // niri's KDL parser needs the command as quoted strings.
                lines.push(format!(
                    "    {hotkey} {{ spawn \"{}\" \"{id}\"; }}",
                    exe.replace('"', "\\\"")
                ));
            }
            lines.push("}".to_string());
            lines.join("\n")
        }
        Desktop::Sway => bindings
            .iter()
            .map(|(hotkey, id)| format!("bindsym {hotkey} exec {} {id}", quote(exe)))
            .collect::<Vec<_>>()
            .join("\n"),
        Desktop::Kde | Desktop::Gnome | Desktop::OtherWayland => {
            let mut lines = vec!["# Shortcut  Command".to_string()];
            for (hotkey, id) in &bindings {
                lines.push(format!("{hotkey:<10} {} {id}", quote(exe)));
            }
            lines.join("\n")
        }
        Desktop::Windows => concat!(
            "# little_oil registers its own global hotkeys on Windows — no keybinds to paste.\n",
            "# Edit them in the GUI: little_oil gui → Settings → Hotkeys."
        )
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/usr/bin/little_oil";

    #[test]
    fn bindings_cover_every_default_hotkey_in_hotkey_order() {
        assert_eq!(
            bindings(),
            vec![("F10", "emptyr"), ("F11", "empty"), ("F12", "reset_inv"),]
        );
    }

    #[test]
    fn hyprland_binds_each_action() {
        assert_eq!(
            snippet(Desktop::Hyprland, EXE),
            "bind = , F10, exec, /usr/bin/little_oil emptyr\n\
             bind = , F11, exec, /usr/bin/little_oil empty\n\
             bind = , F12, exec, /usr/bin/little_oil reset_inv"
        );
    }

    #[test]
    fn niri_spawns_each_action_inside_binds() {
        assert_eq!(
            snippet(Desktop::Niri, EXE),
            "binds {\n    F10 { spawn \"/usr/bin/little_oil\" \"emptyr\"; }\n    F11 { spawn \"/usr/bin/little_oil\" \"empty\"; }\n    F12 { spawn \"/usr/bin/little_oil\" \"reset_inv\"; }\n}"
        );
    }

    #[test]
    fn sway_bindsym_execs_each_action() {
        assert_eq!(
            snippet(Desktop::Sway, EXE),
            "bindsym F10 exec /usr/bin/little_oil emptyr\n\
             bindsym F11 exec /usr/bin/little_oil empty\n\
             bindsym F12 exec /usr/bin/little_oil reset_inv"
        );
    }

    #[test]
    fn kde_gnome_and_other_render_a_shortcut_command_table() {
        for desktop in [Desktop::Kde, Desktop::Gnome, Desktop::OtherWayland] {
            let snippet = snippet(desktop, EXE);
            let lines: Vec<&str> = snippet.lines().collect();
            assert_eq!(lines[0], "# Shortcut  Command", "{desktop:?}");
            assert_eq!(lines[1], "F10        /usr/bin/little_oil emptyr");
            assert_eq!(lines[2], "F11        /usr/bin/little_oil empty");
            assert_eq!(lines[3], "F12        /usr/bin/little_oil reset_inv");
        }
    }

    /// Paths with spaces must stay one argument in every config format.
    #[test]
    fn paths_with_spaces_are_quoted() {
        let exe = "/home/john/My Programs/little_oil";
        let hypr = snippet(Desktop::Hyprland, exe);
        assert!(hypr.contains("bind = , F11, exec, \"/home/john/My Programs/little_oil\" empty"));
        let sway = snippet(Desktop::Sway, exe);
        assert!(sway.contains("bindsym F11 exec \"/home/john/My Programs/little_oil\" empty"));
        let niri = snippet(Desktop::Niri, exe);
        assert!(niri.contains("F11 { spawn \"/home/john/My Programs/little_oil\" \"empty\"; }"));
    }

    #[test]
    fn instructions_name_the_target_config() {
        assert!(instructions(Desktop::Hyprland).contains("hyprland.conf"));
        assert!(instructions(Desktop::Niri).contains("config.kdl"));
        assert!(instructions(Desktop::Sway).contains("sway/config"));
        assert!(instructions(Desktop::Kde).contains("System Settings"));
        assert!(instructions(Desktop::Gnome).contains("Custom Shortcuts"));
        assert!(instructions(Desktop::Windows).contains("built in"));
    }

    #[test]
    fn windows_has_no_pasted_bindings() {
        let snippet = snippet(Desktop::Windows, EXE);
        assert!(snippet.lines().all(|line| line.starts_with('#')));
        assert!(snippet.contains("Settings → Hotkeys"));
    }
}
