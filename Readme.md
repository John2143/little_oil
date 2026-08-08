# little_oil

PoE automation: calibrated clicking, copying, rolling, and inventory emptying
for the quad stash tab, map tab, currency slots, and named points. Linux
(Wayland) and Windows are supported; the Windows backend uses WGC capture and
SendInput, and `little_oil gui` provides a settings/calibration panel plus a
system tray and global hotkeys.

## Windows

### Install

Every push to `master` builds the Windows exe automatically — grab it from
**Actions → build-windows-release → latest run → Artifacts →
`little_oil-windows-x86_64`**. Unzip anywhere and run `little_oil.exe`.
Windows 10 2004+ is required (WGC). No extra runtime DLLs, no installer, no
admin rights.
On tagged releases (`v*`), the zip is also attached to the GitHub Release page.

Build your own on a Windows machine (MSVC toolchain, the most conventional
distribution build):

Cross-build from the nix dev shell on this machine:

```sh
./scripts/build-windows.sh
```

### First run

1. Launch the game (Path of Exile, any window mode; borderless fullscreen is
   fine). The game window is found by class/title automatically.
2. `little_oil gui` — the panel opens.
3. **Calibrate** tab → *Capture game window*, drag a rectangle around the game
   window and save as *Game window*, then around the inventory grid and save as
   *Inventory grid*. Run *Recalibrate inventory colors* from the Actions tab.
4. Use the Actions tab buttons, the tray menu, or the global hotkeys:
   **F10** empty (right-click), **F11** empty (left-click),
   **F12** recalibrate colors.

The config lives at `%APPDATA%\little_oil\config.json` (visible via the
Settings tab → *Open config folder*).

### Windows notes

- Screenshots use **Windows.Graphics.Capture** (WGC) — BitBlt would read black
  from the DX11 game surface. Capture is per-window and synchronous.
- Pointer and keyboard injection use **SendInput** with absolute positioning,
  so there is no `pointer_scale` on Windows.
- DPI awareness is set per-monitor at startup; all coordinates are physical
  pixels. If the game runs at 100% scale everything matches the calibrations.
- `set-region`/`select_region` (slurp) are Linux-only; on Windows do region
  selection from the GUI drag-select.

## Linux

### Install (AppImage)

Prebuilt AppImages are attached to tagged releases. Download
`little_oil-<version>-x86_64.AppImage` from the
[latest release](https://github.com/John2143/little_oil/releases/latest),
then:

```sh
chmod +x little_oil-0.1.0-x86_64.AppImage
./little_oil-0.1.0-x86_64.AppImage            # needs libfuse2 on some distros
./little_oil-0.1.0-x86_64.AppImage --appimage-extract-and-run   # fallback without FUSE
```

The AppImage bundles the app and its Wayland/X11 GUI libraries. It needs a
glibc of 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 37+, Arch) — and
the system tools and permissions under
[Requirements (Linux)](#requirements-linux) below are still required.

Build your own with the nix flake or `cargo build --release`.

## Requirements (Linux)

- `grim` — screenshots (uses the wlr-screencopy protocol)
- `slurp` — interactive region selection (uses the layer-shell protocol)
- `wl-clipboard` (the Rust `wl-clipboard-rs` crate) — item tooltip reading
- Linux `uinput` for pointer/keyboard injection (the `mouse_keyboard_input`
  crate; the user needs write access to `/dev/uinput`)

## Supported compositors


Everything goes through standard Wayland protocols — there is no
compositor-specific code. Tested on **Hyprland**; **niri** works out of the box
because it implements the same protocols (`wlr-screencopy` per its Screencasting
wiki page, `layer-shell`, and `zwp_primary_selection` for the clipboard read).
Any other Wayland compositor implementing those protocols should work too.

### niri notes

- Install `grim` and `slurp`; no portal/pipewire setup is needed because grim
  talks to niri directly via `wlr-screencopy`.
- **Scroll invalidates calibrations.** niri is scrollable-tiling: scrolling the
  workspace moves the game window on screen, so every calibrated region
  (window/stash/map/inventory) silently points at stale coordinates. Run the
  game **fullscreen** — niri's fullscreen pins the window so the viewport can't
  scroll it — and re-run `set-region` calibrations after any layout change.
- **Focus clicks.** `focus_game_window` sends 2 clicks (config key
  `focus_clicks`) because click-to-focus compositors consume the first click.
  If the first click *passes through* on your setup — the game grabs an item at
  the focus-click position on every command — set `"focus_clicks": 1` in
  `config.json`.
- **PoE runs under Xwayland** (it is an X11 game). That is fine: grim captures
  the composited output including Xwayland content, and injected clicks land
  normally.
- **Fractional output scaling** is handled — grim is pinned to scale 1
  (`-s 1`), so captures match screen coordinates at any scale factor.
- **Pointer control on niri is absolute.** niri ignores relative uinput motion,
  so on niri (and only niri — gated by `niri msg outputs`) pointer control goes
  through the wlr virtual-pointer protocol's `motion_absolute`; there is no
  pointer scale, and `calibrate-pointer` bails with an explanatory message.
  On every other Wayland compositor (e.g. Hyprland) the uinput relative path is
  still used, and pointer scale is per machine (input device + compositor
  accel): re-run `calibrate-pointer` if clicks land short or overshoot.

### First run on any compositor

1. `little_oil set-region window` — drag around the entire PoE window.
2. `little_oil calibrate-pointer` — run once per machine (**skip on niri**;
   pointer control there is absolute and needs no scale).
3. Per-layout calibration: `set-region inventory`, `set-region stash`,
   `set-region map`, `calibrate-stash`, `calibrate-map`, `calibrate-currency`
   (or `calibrate-point <name>` per slot), `calibrate-point filter`.
4. `little_oil roll <chrome-file> <times>` (or `chrome`/`mchrome` in the REPL)
   for item rolling; `stash click`, `stash copy`, `empty`, `emptyr` for the
   rest.
## Architecture

`App` (src/app.rs) is the single injected context: it owns the settings
(`RwLock<Settings>`) and the cross-platform input backend
(`platform::input::Input` — uinput/wlr-virtual-pointer on Linux, SendInput on
Windows). Every command — sorting, copying, calibration, clicking, rolling, the
REPL — is a method on `App`; there is no process-global mutable state. `main`
(src/main.rs) only loads config, constructs `App`, and dispatches argv.

Platform-dependent work lives behind `Platform` (src/platform/mod.rs):
screenshot capture (grim on Wayland, WGC on Windows), region selection (slurp
on Wayland, drag-select in the GUI on Windows), clipboard clear/read, and
full-desktop capture. `little_oil gui` (src/gui.rs) is an eframe/egui panel
that drives the same `App` methods from buttons, the system tray (Windows),
and global hotkeys (Windows: F10/F11/F12).

Settings live at the config path printed by `little_oil config`
(`$XDG_CONFIG_HOME/little_oil/config.json` on Linux,
`%APPDATA%\little_oil\config.json` on Windows); a missing file is created from
`default_settings()`. Pointer control uses either absolute positioning via the
wlr virtual-pointer protocol (niri, and Windows by design — no scale needed) or
relative uinput motion scaled by `pointer_scale` (other Wayland compositors).
