//! Platform abstraction: desktop detection, screenshot capture, region
//! selection, input injection and clipboard I/O.
//!
//! Everything the commands need is reached through [`Backend`] (see
//! `backend.rs`); [`Desktop`] says which desktop we are on, so the backend and
//! the health checks can pick the right tools and error text.

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!("little_oil only supports Linux and Windows");

pub(crate) mod backend;
pub(crate) mod input;
#[cfg(target_os = "linux")]
pub(crate) mod virtual_pointer;
#[cfg(target_os = "linux")]
pub(crate) mod wayland;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

pub(crate) use backend::{Backend, detect_backend};
pub(crate) use input::{Input, InputButton, InputKey};

use crate::ScreenRegion;
use serde::{Deserialize, Serialize};

/// The desktop the process runs on.
///
/// Auto-detected from the environment (see [`Desktop::detect`]) unless the
/// config sets `desktop` explicitly. The serde names (`"windows"`,
/// `"hyprland"`, `"niri"`, `"sway"`, `"kde"`, `"gnome"`, `"other"`) are the
/// config override values.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Desktop {
    Windows,
    Hyprland,
    Niri,
    Sway,
    Kde,
    Gnome,
    #[serde(rename = "other")]
    OtherWayland,
}

impl Desktop {
    /// Detect the desktop from environment variables.
    ///
    /// `env` supplies the values so the whole matrix is unit-testable; pass
    /// `std::env::var(..).ok()` in production (see [`detect_desktop`]).
    /// Compositor-specific variables win over `XDG_CURRENT_DESKTOP`, which a
    /// wlroots session often inherits from the login manager.
    pub fn detect(env: &dyn Fn(&str) -> Option<String>) -> Self {
        if cfg!(windows) {
            return Desktop::Windows;
        }
        let has = |name: &str| env(name).is_some_and(|v| !v.is_empty());
        if has("HYPRLAND_INSTANCE_SIGNATURE") {
            return Desktop::Hyprland;
        }
        if has("NIRI_SOCKET") {
            return Desktop::Niri;
        }
        if has("SWAYSOCK") {
            return Desktop::Sway;
        }
        if let Some(current) = env("XDG_CURRENT_DESKTOP") {
            for token in current.split(':') {
                let token = token.trim().to_ascii_uppercase();
                if token.contains("KDE") {
                    return Desktop::Kde;
                }
                if token.contains("GNOME") {
                    return Desktop::Gnome;
                }
            }
        }
        Desktop::OtherWayland
    }

    /// True when pointer positioning is absolute (no `pointer_scale` needed):
    /// niri's wlr virtual pointer and Windows' `SetCursorPos`.
    pub fn uses_absolute_pointer(&self) -> bool {
        matches!(self, Desktop::Niri | Desktop::Windows)
    }

    /// Lowercase name used in config and diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Desktop::Windows => "windows",
            Desktop::Hyprland => "hyprland",
            Desktop::Niri => "niri",
            Desktop::Sway => "sway",
            Desktop::Kde => "kde",
            Desktop::Gnome => "gnome",
            Desktop::OtherWayland => "other",
        }
    }
}

/// Auto-detect the desktop from the real process environment.
pub(crate) fn detect_desktop() -> Desktop {
    Desktop::detect(&|name| std::env::var(name).ok())
}

/// Point-and-drag region capture using the desktop's native selector.
///
/// Temporary seam: the GUI picker takes over every platform in a later step,
/// which also removes the per-platform `select_region` implementations.
pub(crate) fn select_region(prompt: &str) -> anyhow::Result<ScreenRegion> {
    #[cfg(target_os = "linux")]
    {
        wayland::select_region(prompt)
    }
    #[cfg(target_os = "windows")]
    {
        windows::select_region(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_names_are_the_config_values() {
        for (desktop, text) in [
            (Desktop::Windows, "\"windows\""),
            (Desktop::Hyprland, "\"hyprland\""),
            (Desktop::Niri, "\"niri\""),
            (Desktop::Sway, "\"sway\""),
            (Desktop::Kde, "\"kde\""),
            (Desktop::Gnome, "\"gnome\""),
            (Desktop::OtherWayland, "\"other\""),
        ] {
            assert_eq!(serde_json::to_string(&desktop).unwrap(), text);
            assert_eq!(serde_json::from_str::<Desktop>(text).unwrap(), desktop);
        }
        assert!(serde_json::from_str::<Desktop>("\"wayland\"").is_err());
        assert!(serde_json::from_str::<Desktop>("\"x11\"").is_err());
    }

    #[test]
    fn backend_names_match_the_desktop() {
        assert_eq!(Desktop::Hyprland.name(), "hyprland");
        assert_eq!(Desktop::OtherWayland.name(), "other");
    }

    #[test]
    fn absolute_pointer_only_on_niri_and_windows() {
        assert!(Desktop::Niri.uses_absolute_pointer());
        assert!(Desktop::Windows.uses_absolute_pointer());
        for desktop in [
            Desktop::Hyprland,
            Desktop::Sway,
            Desktop::Kde,
            Desktop::Gnome,
            Desktop::OtherWayland,
        ] {
            assert!(!desktop.uses_absolute_pointer(), "{desktop:?}");
        }
    }

    /// The detection matrix. `Desktop::detect` short-circuits to `Windows` on
    /// Windows hosts, so these cases only hold on the other targets.
    #[cfg(not(target_os = "windows"))]
    mod detection {
        use crate::platform::Desktop;

        fn detect_from(vars: &[(&str, &str)]) -> Desktop {
            Desktop::detect(&|name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            })
        }

        #[test]
        fn hyprland_from_its_signature() {
            assert_eq!(
                detect_from(&[("HYPRLAND_INSTANCE_SIGNATURE", "_abc123")]),
                Desktop::Hyprland
            );
        }

        #[test]
        fn niri_from_its_socket() {
            assert_eq!(
                detect_from(&[("NIRI_SOCKET", "/run/user/1000/niri.wayland-1.1234.sock")]),
                Desktop::Niri
            );
        }

        #[test]
        fn sway_from_its_socket() {
            assert_eq!(
                detect_from(&[("SWAYSOCK", "/run/user/1000/sway-ipc.1000.1.sock")]),
                Desktop::Sway
            );
        }

        #[test]
        fn kde_from_the_desktop_variable() {
            assert_eq!(detect_from(&[("XDG_CURRENT_DESKTOP", "KDE")]), Desktop::Kde);
            assert_eq!(detect_from(&[("XDG_CURRENT_DESKTOP", "kde")]), Desktop::Kde);
            assert_eq!(
                detect_from(&[("XDG_CURRENT_DESKTOP", "plasma:KDE")]),
                Desktop::Kde
            );
        }

        #[test]
        fn gnome_from_the_desktop_variable() {
            assert_eq!(
                detect_from(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]),
                Desktop::Gnome
            );
            assert_eq!(
                detect_from(&[("XDG_CURRENT_DESKTOP", "gnome")]),
                Desktop::Gnome
            );
        }

        #[test]
        fn compositor_variables_win_over_the_desktop_variable() {
            assert_eq!(
                detect_from(&[
                    ("HYPRLAND_INSTANCE_SIGNATURE", "_abc123"),
                    ("XDG_CURRENT_DESKTOP", "KDE"),
                ]),
                Desktop::Hyprland
            );
            assert_eq!(
                detect_from(&[("NIRI_SOCKET", "/run/user/1000/n"), ("SWAYSOCK", "/run/s")]),
                Desktop::Niri
            );
        }

        #[test]
        fn nothing_set_is_other_wayland() {
            assert_eq!(detect_from(&[]), Desktop::OtherWayland);
            assert_eq!(
                detect_from(&[("XDG_CURRENT_DESKTOP", "wlroots")]),
                Desktop::OtherWayland
            );
        }

        #[test]
        fn empty_variables_do_not_count() {
            assert_eq!(
                detect_from(&[
                    ("HYPRLAND_INSTANCE_SIGNATURE", ""),
                    ("NIRI_SOCKET", ""),
                    ("SWAYSOCK", ""),
                    ("XDG_CURRENT_DESKTOP", ""),
                ]),
                Desktop::OtherWayland
            );
        }
    }
}
