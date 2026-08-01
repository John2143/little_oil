# little_oil

PoE automation: calibrated clicking, copying, and rolling for the quad stash
tab, map tab, currency slots, and named points. Wayland-first — X11 and Windows
exist as stubs.

## Requirements

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
(`RwLock<Settings>`), the uinput virtual device (`Mutex<VirtualDevice>`), and
the Wayland virtual-pointer connection (`Mutex<VirtualPointer>`). Every
command — sorting, copying, calibration, clicking, rolling, the REPL — is a
method on `App`; there is no process-global mutable state. `main` (src/main.rs)
only loads config, constructs `App`, and dispatches argv.

Settings live at the config path printed by `little_oil config`
(`$XDG_CONFIG_HOME/little_oil/config.json`); a missing file is created from
`default_settings()`. Pointer control uses either absolute positioning via the
wlr virtual-pointer protocol (niri — no scale needed) or relative uinput
motion scaled by `pointer_scale` (all other Wayland compositors).
