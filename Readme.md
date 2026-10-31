# little_oil

Path of Exile automation: calibrated clicking, item copying, rolling, inventory
emptying, quad-tab sorting, map clicking and the chaos recipe.

Every feature is available three ways:

- **GUI** — `little_oil gui`: setup wizard, calibration, health checks and every
  macro as a button. The friendliest path for non-technical users.
- **CLI** — `little_oil <action>`: bind the actions in your window manager or
  desktop, or script them. `little_oil keybinds` prints a ready-to-paste
  snippet for the desktop it detects.
- **Global hotkeys + tray** — Windows only: F10/F11/F12 work out of the box and
  every action can be rebound in the GUI. Wayland compositors forbid global key
  grabs, so on Linux you bind the CLI commands in your compositor instead.

Windows 10 2004+, Hyprland, niri and sway are supported today. KDE Plasma 6 and
GNOME 46+ are detected but need the xdg-desktop-portal backend, which is not
implemented yet (see [KDE Plasma 6 / GNOME 46+](#kde-plasma-6--gnome-46)).
X11 sessions are not supported.

## Support matrix

| | Windows | Hyprland | niri | Sway |
|---|---|---|---|---|
| GUI panel (`little_oil gui`) | yes | yes | yes | yes |
| Screenshots and region picker | WGC / GDI | grim | grim | grim |
| Empty inventory (`empty`, `emptyr`) | yes | yes | yes | yes |
| Quad-tab sort (`sort`) and copy (`stash-copy`) | yes | yes | yes | yes |
| Map click (`click map <col> <row>`) | yes | yes | yes | yes |
| Chance rolling (`chance`), auto-roll files (`roll`) | yes | yes | yes | yes |
| Chaos recipe (`chaos`, `tally`) | yes | yes | yes | yes |
| Tooltip reading (copy / roll / chaos) | Win32 clipboard | wl-clipboard | wl-clipboard | wl-clipboard |
| Window and monitor auto-detect | Win32 | `hyprctl` | `niri msg`¹ | `swaymsg` |
| Global hotkeys and tray | built in | WM bindings | WM bindings | WM bindings |
| `little_oil keybinds` snippet | not needed | `hyprland.conf` | `config.kdl` | sway config |

¹ niri tiles report their viewport position only when the scrolling layout has
placed them; when it is unavailable the wizard says the game window could not
be detected and you drag the box around it by hand.

### KDE Plasma 6 / GNOME 46+

`little_oil doctor` reports `desktop wlr:kde` / `wlr:gnome`, but grim needs the
wlr-screencopy protocol, which neither KWin nor Mutter implements, so capture
(and therefore every macro) does not work there yet. The planned fix is an
xdg-desktop-portal RemoteDesktop/ScreenCast backend; until it lands, use a
wlroots session (Hyprland, niri, sway) or Windows. The GUI still opens and the
Health tab explains which checks fail.

## Windows

### Install

Every push to `master` builds the Windows exe — grab it from **Actions →
build-windows-release → latest run → Artifacts → `little_oil-windows-x86_64`**.
Unzip anywhere and run `little_oil.exe`: no installer, no extra DLLs, no admin
rights. Tagged releases (`v*`) also attach the zip to the GitHub Release page.

Build your own on a Windows machine with the MSVC toolchain:

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

### Quick start

1. Launch Path of Exile (windowed, borderless or fullscreen; the window is found
   by class/title automatically).
2. Run `little_oil.exe gui`. The first run opens the **Setup** tab: drag a box
   around the game window, then around the inventory grid, and run
   *Recalibrate inventory colors*. Stash and map regions/grids are optional and
   only needed for the features that use them.
3. Use the **Actions** tab, the tray menu, or the built-in global hotkeys:
   **F10** empty inventory (right-click), **F11** empty inventory (left-click),
   **F12** recalibrate inventory colors.
4. Rebind hotkeys in **Settings → Hotkeys** (e.g. `F9`, `ctrl+shift+E`; clear a
   field to unbind). Changes apply immediately and live in the config key
   `hotkeys`.

The config lives at `%APPDATA%\little_oil\config.json` (Settings tab → *Open
config folder*).

### Windows notes

- Screenshots use **Windows.Graphics.Capture** for the game window (BitBlt
  reads black from the DX11 surface) and GDI for the desktop preview.
- Pointer and keyboard injection use **SendInput** with absolute positioning, so
  there is no `pointer_scale` and `calibrate-pointer` is not needed.
- DPI awareness is set per monitor at startup; all coordinates are physical
  pixels.
- `set-region` and `calibrate-point` open the same GUI picker as everywhere
  else: drag a box over the captured desktop and confirm.

## Linux

### Install

AppImage (x86_64), attached to tagged releases and built on every push as the
`little_oil-appimage` artifact:

```sh
chmod +x little_oil-<version>-x86_64.AppImage
./little_oil-<version>-x86_64.AppImage            # needs libfuse2 on some distros
./little_oil-<version>-x86_64.AppImage --appimage-extract-and-run   # without FUSE
```

It needs glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 37+, Arch) and
the system tools under [Requirements](#requirements) below.

With Nix: `nix run .` runs the packaged binary, `nix develop` opens the dev
shell. From source: `nix develop -c cargo build --release`.

### Requirements

- `grim` — screenshots (wlr-screencopy protocol)
- `wl-clipboard` (`wl-copy`/`wl-paste`; the Rust `wl-clipboard-rs` crate) —
  reading item tooltips
- write access to `/dev/uinput` — pointer/keyboard injection (add your user to
  the `input` group or install a udev rule)
- a notification daemon — status messages go through `notify-rust`
- window auto-detection: `hyprctl` on Hyprland, `niri` on niri, `swaymsg` on sway
- the GUI's Wayland client libraries (bundled in the AppImage; provided by the
  Nix package and the dev shell)

`little_oil doctor` checks all of these, plus regions, colour samples, pointer
scale and the backend in use.

### Quick start

1. `little_oil gui` — the Setup wizard walks through the game window and
   inventory regions, colour recalibration, the optional stash/map grids, and
   pointer calibration. The CLI equivalents are
   `little_oil set-region window|inventory|stash|map`, `calibrate-stash`,
   `calibrate-map`, `calibrate-currency`, `calibrate-point <name>` and
   `reset_inv`.
2. `little_oil doctor` — every line OK, or a warning you have accepted.
3. Bind the actions: `little_oil keybinds` prints a snippet for the detected
   desktop using the running executable's path, e.g. on Hyprland:

   ```
   # Paste into ~/.config/hypr/hyprland.conf, then run: hyprctl reload
   bind = , F10, exec, /usr/bin/little_oil emptyr
   bind = , F11, exec, /usr/bin/little_oil empty
   bind = , F12, exec, /usr/bin/little_oil reset_inv
   ```

4. Run macros from the GUI Actions tab, from the tray (Windows) or from the
   CLI: `empty`, `emptyr`, `reset_inv`, `sort`, `stash-copy`, `chance`,
   `chaos`, `tally`, `roll`.

Region selection always opens the GUI picker: drag a box over the captured
desktop, confirm to save, press Esc to cancel (`region selection cancelled`).

### Hyprland notes

- Pointer control prefers the wlr virtual pointer and falls back to uinput
  relative motion scaled by `pointer_scale`. Run `little_oil calibrate-pointer`
  once per machine (on a static screen — a clock or terminal animation makes it
  measure nothing) if clicks land short or overshoot.
- `focus_game_window` sends `focus_clicks` clicks (default 2) because
  click-to-focus consumes the first one. Raise it if the game keeps stealing
  the first click; lower it to 1 on setups that pass the first click through.
- Window/monitor auto-detect uses `hyprctl clients -j` / `monitors -j`; if the
  game is not on screen, drag the region by hand.

### niri notes

- Install `grim`; it talks to niri directly over wlr-screencopy — no portal or
  pipewire setup.
- **Scroll invalidates calibrations.** niri is scrollable-tiling, so scrolling
  the workspace moves the game window on screen and stale regions silently
  point at the wrong pixels. Run the game **fullscreen** (niri pins fullscreen
  windows so the viewport cannot scroll them) and re-drag regions after any
  layout change.
- **Window detection can be unavailable.** niri reports a tile's viewport
  position only once the scrolling layout has placed it; when it is missing, the
  wizard says so and you drag a box around the game window yourself.
- **Focus clicks.** If the first focus click passes through and grabs an item,
  set `"focus_clicks": 1` in `config.json`.
- **Pointer control is absolute** through the wlr virtual pointer, so there is
  no `pointer_scale`; `calibrate-pointer` bails with an explanation.
- PoE runs under Xwayland; that is fine — grim captures the composited output
  and injected clicks land normally.
- Fractional output scaling is handled: captures are taken at scale 1 and match
  screen coordinates at any scale factor.

### Sway notes

- `grim` for capture, `swaymsg -t get_tree` / `-t get_outputs` for window and
  monitor lookup.
- Pointer control uses the wlr virtual pointer with the uinput relative
  fallback, so `little_oil calibrate-pointer` applies here too.
- Bind with the snippet `little_oil keybinds` prints (`bindsym F11 exec …`),
  then `swaymsg reload`.

## CLI

```
Usage: little_oil [COMMAND]

  gui                 Open the GUI panel (settings, calibration, macros)
  config              Print the active config
  version             Print the version and exit
  doctor              Diagnose why something isn't working
  sort [times=40]     Click every highlighted quad-tab cell
  roll <file> <times> Roll an item from an auto-roll file
  click <name>        Click a calibrated point (filter, chaos, …)
  click map <col> <row>   Click a cell of the calibrated 12x7 map grid (0-based)
  stash click [times=40]  Left-click every highlighted quad-tab cell
  stash copy          Ctrl+Alt+C every highlighted cell and print unique items
  chaos [amount=1]    Run the chaos recipe
  tally               Count items already collected for the recipe
  calibrate-pointer   Measure the pointer scale (run once per machine)
  calibrate-stash     Calibrate the 24x24 quad tab grid
  calibrate-map       Calibrate the 12x7 map tab grid
  calibrate-point <name>  Pick and save one named clickable point
  calibrate-currency  Calibrate the ten currency slots
  set-region <inventory|stash|window|map>  Pick and save a screen region
  keybinds            Print ready-to-paste keybind snippets
  empty, emptyr       Empty the inventory (Ctrl+left / Ctrl+right click)
  reset_inv           Re-sample the 60 empty-slot probe colors
  stash-copy          Same as `stash copy`
  chance              Alternate chance and scour orbs on the calibrated slot
  help                Print this help
```

With no subcommand, `little_oil` starts the historical stdin REPL (`help`,
`pull <ms>`, `div <ms>`, `chrome <file> <times>`, `mchrome <file>`).

Auto-roll configs live in `rolls/`; `mchrome <path>` writes a template.

### Chaos recipe

Configure it under the config key `chaos_recipe_settings`, then
`little_oil chaos [amount]`. It reads the stash tab over the Path of Exile
trade API, picks one item per recipe slot and clicks them:

```json
"chaos_recipe_settings": {
  "session_id": "POESESSID cookie value",
  "account_name": "your account name",
  "league": "Settlers",
  "tab_name": "Chaos",
  "tab_index": null
}
```

`tab_index` pins the tab if you have duplicate names.

## Configuration

`little_oil config` prints the active settings. The file lives at
`$XDG_CONFIG_HOME/little_oil/config.json` on Linux (usually
`~/.config/little_oil/config.json`) and `%APPDATA%\little_oil\config.json` on
Windows; a missing file is created from the defaults.

| Key | Meaning |
|---|---|
| `desktop` | Force a desktop (`windows`, `hyprland`, `niri`, `sway`, `kde`, `gnome`, `other`) instead of auto-detecting |
| `game_window_region`, `inv_region`, `stash_region`, `map_region` | Calibrated screen regions |
| `stash_grid`, `map_grid` | Calibrated grid origins, cell sizes and highlight colour |
| `points` | Named clickable points (currency slots, filter button, …) |
| `inv_samples` | 60 × 3 empty-slot probe colours (`reset_inv`) |
| `pointer_scale` | uinput device units per screen pixel (not used on Windows or when the virtual pointer is available) |
| `focus_clicks` | Clicks sent to hand keyboard focus to the game (default 2) |
| `pull_delay`, `div_delay` | Settle delays for stash pulls and divination-card macros |
| `roll_click_delay`, `roll_read_delay` | Settle delays for `roll` |
| `hotkeys` | Windows only: action id → accelerator, e.g. `{"empty": "F9"}` |
| `chaos_recipe_settings` | Chaos recipe credentials and tab (see above) |

## Architecture

`App` (`src/app/`) is the single injected context: it owns the settings
(`RwLock<Settings>`) and one `Box<dyn Backend>` for the desktop. Commands are
methods on `App`, split by feature: `mod.rs` (context, pointer/keyboard
primitives, screenshots, focus clicks), `inventory.rs` (empty, reset colours),
`stash.rs` (sort, copy), `map.rs`, `roll.rs` (chance, roll log),
`calibrate.rs` (regions, grids, points) and `repl.rs`. `main.rs` only loads the
config, builds the `App` and hands argv to `cli.rs`.

`src/actions.rs` holds the single action registry: every macro's id, label,
help, default hotkey and function. The GUI Actions tab, the CLI subcommands,
the Windows tray and the Windows hotkeys are all generated from it, so adding
an action means adding one entry.

Platform work lives behind the `Backend` trait (`src/platform/backend.rs`):
`WlrBackend` on Linux (grim capture, wlr virtual pointer with the uinput
relative fallback, wl-clipboard, `desktop.rs` window/monitor lookup via
`hyprctl`/`niri`/`swaymsg`) and `WindowsBackend` (WGC/GDI capture, SendInput,
Win32 clipboard and window lookup). `platform::detect_backend` picks one from
the `Desktop` enum, which is auto-detected from environment variables unless
the config overrides it. `little_oil gui` (`src/gui.rs`) is an eframe/egui panel
that drives the same `App` methods; tests swap in `test_support::FakeBackend`,
which replays synthetic frames and records input events.

## Troubleshooting

- **Run `little_oil doctor` first.** It names the failing piece: backend,
  capture, regions, colour samples, focus clicks, pointer scale, or a missing
  binary/permission.
- **"Game window region not set / Inventory region not calibrated"** — the
  message now points at both fixes: open the GUI Setup tab, or run
  `little_oil set-region window` / `set-region inventory`.
- **Clicks land short or overshoot** (relative-pointer desktops only) — re-run
  `little_oil calibrate-pointer` on a static screen, or set `pointer_scale`
  manually.
- **`calibrate-pointer` says "found N changed regions"** — something on screen
  is animating; close it and retry.
- **The game never receives Ctrl/keys** — increase `focus_clicks`; on niri,
  lower it if the focus click grabs an item.
- **Regions point at stale pixels** — the layout moved (especially niri
  scrolling). Re-drag the regions; keep the game fullscreen on niri.
- **Roll log** — every rolled tooltip is appended to
  `$XDG_CONFIG_HOME/little_oil/rolls.log` as JSONL.
