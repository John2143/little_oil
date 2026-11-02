# little_oil

Path of Exile automation: calibrated clicking, item copying, rolling, inventory
emptying, quad-tab sorting and copying, map clicking and the chaos recipe.

Every feature is reachable three ways:

- **GUI** — `little_oil gui`. A six-tab egui panel: **Actions** (one button per
  macro), **Setup** (guided first run), **Calibrate** (drag-select regions and
  grids, right-click the preview for a whole window/monitor), **Inventory**
  (overlay preview), **Health** (the same checks as `doctor`) and **Settings**
  (delays, focus clicks, pointer scale, config path, keybinds/hotkeys).
- **CLI** — `little_oil <action>`. One command per action, meant for binding in
  a window manager or scripting. `little_oil keybinds` prints a ready-to-paste
  snippet for the desktop it detects.
- **Global hotkeys + tray** — Windows only. F10/F11/F12 work out of the box,
  every action can be rebound in the GUI, and the tray menu lists every action.
  Wayland compositors forbid global key grabs, so on Linux you bind the CLI
  commands in your compositor instead.

**Supported today:** Windows 10 2004+, Hyprland, niri, sway.
**Detected but not fully working:** KDE Plasma 6 and GNOME 46+ — every macro
depends on screen capture, and neither KWin nor Mutter implements the
wlr-screencopy protocol grim needs. The planned fix is an xdg-desktop-portal
RemoteDesktop/ScreenCast backend, which is not implemented yet (see
[KDE Plasma 6 / GNOME 46+](#kde-plasma-6--gnome-46)).
**Not supported:** X11 sessions (the shim was removed; a leftover
`"platform": "x11"` config key is rejected with an explanatory error).

## Actions

The registry in `src/actions.rs` is the single source of truth for the CLI
subcommands, the GUI Actions tab, the tray menu and the Windows hotkeys.

| Id | GUI label | What it does | Windows hotkey |
|---|---|---|---|
| `empty` | Empty inventory (left) | Ctrl+click every occupied inventory slot, up to 3 verify passes | F11 |
| `emptyr` | Empty inventory (right) | Same, with the right button | F10 |
| `reset_inv` | Recalibrate inventory colors | Re-sample the 60 empty-slot probe colours | F12 |
| `sort` | Sort quad tab | Click up to 40 highlighted quad-tab cells | — |
| `stash-copy` | Copy stash items | Ctrl+Alt+C every highlighted cell, print the unique items | — |
| `chance` | Chance an item | Alternate chance and scour orbs on the calibrated slot | — |
| `chaos` | Chaos recipe (1 set) | Run one chaos recipe set from the trade API | — |
| `tally` | Tally chaos recipe | Count the items already collected | — |

`little_oil roll <file> <times>` and the inventory/stash/map calibration
commands are CLI/GUI only (no registry entry).

## Support matrix

| | Windows | Hyprland | niri | sway | KDE / GNOME |
|---|---|---|---|---|---|
| GUI panel and every tab | yes | yes | yes | yes | GUI opens; capture checks fail |
| Screen capture | WGC (game window) + GDI (desktop) | grim | grim | grim | no — needs the portal backend |
| Region picker (`set-region`, calibration) | GUI picker | GUI picker | GUI picker | GUI picker | GUI picker (no capture to drag over) |
| All clicking macros (empty, sort, map, chance, roll, chaos) | yes | yes | yes | yes | no — they capture the screen first |
| Tooltip reading (copy, roll, chaos) | Win32 clipboard | wl-clipboard-rs | wl-clipboard-rs | wl-clipboard-rs | portal clipboard (not implemented) |
| Window / monitor auto-detect | Win32 | `hyprctl clients -j` / `monitors -j` | `niri msg -j windows/workspaces/outputs` ¹ | `swaymsg -t get_tree` / `-t get_outputs` | never — drag the box by hand |
| Notifications | console + GUI status line | notify-rust (needs a notification daemon) | same | same | same |
| Global hotkeys and tray | built in | compositor bindings | compositor bindings | compositor bindings | desktop shortcut settings |
| `little_oil keybinds` snippet | "not needed" note | `hyprland.conf` lines | `config.kdl` block | sway config lines | shortcut/command table |

¹ niri tiles report their viewport position only once the scrolling layout has
placed them. When it is missing, window detection returns nothing and the Setup
wizard asks you to drag the box around the game window yourself.

### KDE Plasma 6 / GNOME 46+

`little_oil doctor` reports `desktop wlr:kde` / `wlr:gnome`, the GUI opens, and
Health explains which checks fail: `desktop-capture` errors because grim cannot
talk to KWin or Mutter. Until the portal backend lands, use a wlroots session
(Hyprland, niri, sway) or Windows.

## Install

### Windows

Every push to `master` builds the exe — **Actions → build-windows-release →
latest run → Artifacts → `little_oil-windows-x86_64`**. Unzip anywhere and run
`little_oil.exe`: no installer, no extra DLLs, no admin rights. Tagged releases
(`v*`) also attach the zip to the GitHub Release page.

Build your own on a Windows machine with the MSVC toolchain:

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

### Linux

AppImage (x86_64), attached to tagged releases and built on every push as the
`little_oil-appimage` artifact:

```sh
chmod +x little_oil-<version>-x86_64.AppImage
./little_oil-<version>-x86_64.AppImage            # needs libfuse2 on some distros
./little_oil-<version>-x86_64.AppImage --appimage-extract-and-run   # without FUSE
```

Built inside `ubuntu:22.04`, so it needs glibc 2.35 or newer (Ubuntu 22.04+,
Debian 12+, Fedora 37+, Arch) plus the system tools under
[Linux requirements](#linux-requirements).

With Nix: `nix run .` runs the packaged binary, `nix develop` opens the dev
shell (which also carries `sway`, `grim`, `wtype` and `python3` for the
integration test). From source: `nix develop -c cargo build --release`.

### Linux requirements

- `grim` — screen capture (wlr-screencopy). Only checked when the desktop
  speaks it; there is no slurp/portal fallback yet.
- write access to `/dev/uinput` — pointer and keyboard injection. Add your user
  to the `input` group or install a udev rule.
- a notification daemon — status messages go through `notify-rust` (the
  clipboard is handled in-process by `wl-clipboard-rs`, no `wl-copy` needed).
- window auto-detection needs the matching tool: `hyprctl` on Hyprland, `niri`
  on niri, `swaymsg` on sway.
- the GUI's Wayland/GL client libraries (bundled in the AppImage; provided by
  the Nix package and the dev shell).

`little_oil doctor` checks all of these plus regions, colour samples, pointer
scale and the backend in use.

## First run

1. Start Path of Exile (windowed, borderless or fullscreen; the game window is
   found by title/class where the desktop reports window geometry).
2. `little_oil gui` (or `little_oil.exe gui`) opens the **Setup** tab on a
   fresh config:
   - step 1 — detect the game window, or drag a box around it;
   - step 2 — inventory region;
   - step 3 — sample the 60 empty inventory slots (*Recalibrate inventory
     colors*);
   - step 4 — pointer calibration, only when the desktop needs it
     (Hyprland/sway relative fallback; niri and Windows position absolutely, so
     the step says so and skips).
   Stash and map regions/grids are optional and only needed by the features
   that use them (`sort`, `stash-copy`, `click map`, `chaos`).
3. `little_oil doctor` — every line `[ OK ]`, or a warning you accept.
4. Bind the actions (below) or use the GUI Actions tab / Windows tray.

GUI equivalents of the CLI calibrators: `set-region window|inventory|stash|map`,
`calibrate-stash`, `calibrate-map`, `calibrate-currency`, `calibrate-point`,
`reset_inv`. Every region selection opens the same GUI picker: drag a box over
the captured desktop, confirm to save, press Esc to cancel
(`region selection cancelled`).

## Bind keys

`little_oil keybinds` prints the paste target and the snippet for the detected
desktop, using the running executable's path. The GUI shows the same snippet in
**Settings → Keybinds** with a *Copy keybinds* button.

```sh
# Hyprland — paste into ~/.config/hypr/hyprland.conf, then: hyprctl reload
bind = , F10, exec, /usr/bin/little_oil emptyr
bind = , F11, exec, /usr/bin/little_oil empty
bind = , F12, exec, /usr/bin/little_oil reset_inv
```

```kdl
// niri — paste inside binds { } in ~/.config/niri/config.kdl, then:
// niri msg action reload-config
binds {
    F10 { spawn "/usr/bin/little_oil" "emptyr"; }
    F11 { spawn "/usr/bin/little_oil" "empty"; }
    F12 { spawn "/usr/bin/little_oil" "reset_inv"; }
}
```

```sh
# sway — paste into ~/.config/sway/config, then: swaymsg reload
bindsym F10 exec /usr/bin/little_oil emptyr
bindsym F11 exec /usr/bin/little_oil empty
bindsym F12 exec /usr/bin/little_oil reset_inv
```

On KDE: *System Settings → Keyboard → Shortcuts → Add → Command or Script…*;
on GNOME: *Settings → Keyboard → Custom Shortcuts → +*. `keybinds` prints the
shortcut/command table for those desktops.

On Windows there is nothing to paste: the app registers its own hotkeys. Rebind
them in **Settings → Hotkeys** (e.g. `F9`, `ctrl+shift+E`; clear a field to
unbind). Edits apply on the next frame and land in the config key `hotkeys`;
unparsable values and duplicate keys are logged as `hotkey <id>: <err>` and
skipped (the first action in registry order keeps the key).

## Platform notes

### Windows

- Screenshots use **Windows.Graphics.Capture** for the game window (BitBlt
  reads black from the DX11 surface) and GDI for the desktop preview.
- Pointer and keyboard injection use **SendInput** with absolute positioning, so
  there is no `pointer_scale` and `calibrate-pointer` declines to run.
- DPI awareness is set per monitor at startup; all coordinates are physical
  pixels.
- Status messages go to the console and the GUI status line (no toast).

### Hyprland

- Capture is `grim`; window/monitor detection is `hyprctl clients -j` /
  `monitors -j`.
- Pointer control tries the wlr virtual pointer (`motion_absolute`) first and
  falls back to uinput relative motion scaled by `pointer_scale`. If clicks land
  short or overshoot, run `little_oil calibrate-pointer` on a static screen or
  set `pointer_scale` by hand.
- `focus_game_window` sends `focus_clicks` clicks (default 2) because
  click-to-focus consumes the first one. Raise it if the game keeps stealing the
  first click; lower it to 1 on setups that pass the first click through.

### niri

- `grim` talks to niri over wlr-screencopy — no portal or pipewire setup.
- **Scrolling invalidates calibrations.** niri is scrollable-tiling: scrolling
  the workspace moves the game window and stale regions silently point at the
  wrong pixels. Run the game **fullscreen** (niri pins fullscreen windows so the
  viewport cannot scroll them) and re-drag regions after any layout change.
- **Window detection can be unavailable.** niri reports a tile's viewport
  position only once the scrolling layout has placed it; when it is missing the
  wizard says so and you drag the box yourself.
- **Pointer control is absolute** through the wlr virtual pointer;
  `calibrate-pointer` explains that and exits.
- If the first focus click passes through and grabs an item, set
  `"focus_clicks": 1`.
- PoE runs under Xwayland; grim captures the composited output and injected
  clicks land normally.

### sway

- Capture is `grim`; window/monitor detection is `swaymsg -t get_tree` /
  `-t get_outputs`.
- Pointer control uses the wlr virtual pointer with the uinput relative
  fallback, so `little_oil calibrate-pointer` applies here too.
- Bind with the snippet `little_oil keybinds` prints, then `swaymsg reload`.

## CLI reference

```
Usage: little_oil [COMMAND]

  gui                     Open the GUI panel (settings, calibration, macros)
  config                  Print the active config
  version                 Print the version and exit
  doctor                  Diagnose why something isn't working
  sort [TIMES=40]         Click every highlighted quad-tab cell
  roll <FILE> <TIMES>     Roll an item from an auto-roll file
  click <NAME>            Click a calibrated point (filter, chaos, …)
  click map <COL> <ROW>   Click a cell of the calibrated 12x7 map grid (0-based)
  stash click [TIMES=40]  Left-click every highlighted quad-tab cell
  stash copy              Ctrl+Alt+C every highlighted cell and print unique items
  chaos [AMOUNT=1]        Run the chaos recipe
  tally                   Count items already collected for the recipe
  calibrate-pointer       Measure the pointer scale (run once per machine)
  calibrate-stash         Calibrate the 24x24 quad tab grid
  calibrate-map           Calibrate the 12x7 map tab grid
  calibrate-point <NAME>  Pick and save one named clickable point
  calibrate-currency      Calibrate the ten currency slots
  set-region <inventory|stash|window|map>   Pick and save a screen region
  keybinds                Print ready-to-paste keybind snippets
  empty                   Ctrl+click every occupied slot, up to 3 verify passes
  emptyr                  Same as empty, but right-click every occupied slot
  reset_inv               Re-sample the 60 empty-slot probe colors
  stash-copy              Ctrl+Alt+C every highlighted cell and print unique items
  chance                  Alternate chance and scour orbs on the calibrated slot
  help                    Print help or the help of a subcommand

  -h, --help              Print help
  -V, --version           Print version
```

`empty`, `emptyr`, `reset_inv`, `stash-copy`, `chance`, `chaos` and `tally` are
generated from the action registry, so their CLI help is the same text the GUI
shows as button tooltips. `sort` and `chaos` also exist as explicit subcommands
with optional counts; their defaults match the registry.

**No subcommand** starts the historical stdin REPL. Its commands are the
original set, not the clap surface:

```
help, version
empty, emptyr
set-region <inventory|stash|window|map>
calibrate-pointer, calibrate-stash, calibrate-map, calibrate-point <name>, calibrate-currency
click <name>, click map <col> <row>
stash <click|copy> [times]
pull <delay>, div <delay>
chrome <file> <times>, mchrome <file>
```

## Auto-roll files

`roll <file> <times>` (or the `chrome`/`mchrome` REPL commands) reads a JSON
auto-roll config. Example configs live in `rolls/`; `mchrome <path>` writes a
template.

```json
{
  "item_name": "Warlock Boots",
  "mods": [{ "name": "tailwind", "is_prefix": false }],
  "auto_aug_regal": false,
  "any_two_t1": true
}
```

Orb and item-slot positions come from the calibrated points `alt`,
`augment`/`aug`, `regal` and `slot`. Every rolled tooltip is appended to
`$XDG_CONFIG_HOME/little_oil/rolls.log` as JSONL. `roll_click_delay` and
`roll_read_delay` tune the settle times.

## Chaos recipe

Configure the trade-API credentials under the config key
`chaos_recipe_settings`, then `little_oil chaos [amount]` (or `tally` to dry
run the count). It reads the stash tab over the Path of Exile trade API, picks
one item per recipe slot and clicks them:

```json
"chaos_recipe_settings": {
  "session_id": "POESESSID cookie value",
  "account_name": "your account name",
  "league": "Settlers",
  "tab_name": "Chaos",
  "tab_index": null
}
```

`tab_index` pins the tab when you have duplicate names.

## Configuration

`little_oil config` prints the active settings. The file lives at
`$XDG_CONFIG_HOME/little_oil/config.json` on Linux (usually
`~/.config/little_oil/config.json`) and `%APPDATA%\little_oil\config.json` on
Windows; a missing file is created from the defaults. GUI Settings → *Open
config folder* reveals it. Parse errors are reported, never silently replaced
with defaults.

| Key | Meaning |
|---|---|
| `desktop` | Force a desktop (`windows`, `hyprland`, `niri`, `sway`, `kde`, `gnome`, `other`) instead of auto-detecting it from the environment. A `windows` override on Linux (and vice versa) is rejected at startup. |
| `game_window_region`, `inv_region`, `stash_region`, `map_region` | Calibrated screen regions |
| `stash_grid`, `map_grid` | Calibrated grid origins, cell sizes and highlight colour |
| `points` | Named clickable points (currency slots, filter button, …) |
| `inv_samples` | 60 × 3 empty-slot probe colours (`reset_inv`) |
| `pointer_scale` | uinput device units per screen pixel (unused on Windows and when the virtual pointer is available) |
| `focus_clicks` | Clicks sent to hand keyboard focus to the game (default 2) |
| `pull_delay`, `div_delay` | Settle delays for stash pulls and divination-card macros |
| `roll_click_delay`, `roll_read_delay` | Settle delays for `roll` |
| `hotkeys` | Windows only: action id → accelerator, e.g. `{"empty": "F9"}`; `""` unbinds |
| `chaos_recipe_settings` | Chaos recipe credentials and tab (see above) |
| `setup_complete` | Whether the first-run wizard finished (decides the GUI's opening tab) |

The retired `platform` key is dropped on load: `"wayland"` and `"windows"`
become auto-detection, `"x11"` is a hard error naming the config path. A stale
`platform` key is not written back.

## Architecture

```
src/main.rs        config load/save, Settings, main(); hands argv to cli.rs
src/cli.rs         clap surface; explicit subcommands + one per registry action
src/actions.rs     the action registry (id, label, help, default hotkey, fn)
src/keybinds.rs    per-desktop keybind snippets for `keybinds` and the GUI
src/app/           App: the injected context, split by feature
  mod.rs             settings, backend, pointer/keyboard primitives, screenshots
  inventory.rs       reset_inv, empty/emptyr
  stash.rs           sort, stash-copy
  map.rs             click map <col> <row>
  roll.rs            chance, roll log
  calibrate.rs       regions, grids, named points, set-region
  repl.rs            the stdin REPL
src/gui.rs         eframe/egui panel, region picker, tray and hotkeys (Windows)
src/health.rs      the checks behind `doctor` and the Health tab
src/platform/      Backend trait and per-desktop implementations
  backend.rs         Backend trait, WlrBackend, WindowsBackend, detect_backend
  input.rs           uinput/SendInput primitives, virtual-pointer fallback
  virtual_pointer.rs wlr virtual pointer over the Wayland socket
  desktops.rs        window/monitor lookup: hyprctl, swaymsg, niri
  wayland.rs         grim capture helpers
  windows.rs         WGC/GDI capture, SendInput, Win32 clipboard and lookups
src/auto_roll.rs   auto-roll files
src/chaos_recipe.rs trade-API recipe collection
src/item.rs        tooltip parser
src/stash_grid.rs  quad/map grid geometry
src/test_support.rs FakeBackend + synthetic frames (test builds only)
examples/fake_poe.rs synthetic "Path of Exile" window for integration tests
tests/integration/  headless sway and Windows end-to-end scripts
tests/fixtures/     desktop JSON fixtures and inventory captures
tests/example_items/tooltip corpus (every .txt must parse)
```

`App` owns the settings (`RwLock<Settings>`) and one `Box<dyn Backend>`.
`platform::detect_backend` picks `WlrBackend` or `WindowsBackend` from the
`Desktop` enum, which comes from `HYPRLAND_INSTANCE_SIGNATURE`, `NIRI_SOCKET`,
`SWAYSOCK`, then `XDG_CURRENT_DESKTOP`, unless the config overrides it.
Everything that touches the screen, the pointer, the clipboard, notifications
or the clock goes through the `Backend` trait, so tests swap in
`test_support::FakeBackend`, which replays synthetic frames and records input
events instead of performing them.

## Tests and CI

- **Unit tests** (`nix develop -c cargo test`): macro behaviour against
  `FakeBackend` (click order, modifiers, verify passes, clipboard retries), the
  tooltip parser over `tests/example_items`, desktop detection and config
  migration, the CLI parse matrix, action-registry invariants, keybind snippets,
  region-picker geometry, and desktop JSON parsers from `tests/fixtures/`.
- **Integration** (`tests/integration/run_sway.sh`): starts sway with the
  headless backend, opens `examples/fake_poe.rs` as a "Path of Exile" window
  with a synthetic 12×5 inventory, runs `reset_inv` then `empty` with the real
  binary, and asserts five Ctrl-held clicks on the item cells plus an empty
  captured inventory. Needs `sway`, `swaymsg`, `grim`, `wtype`, `python3` and a
  writable `/dev/uinput` (all in `nix develop`).
- **Windows integration** (`tests/integration/run_windows.ps1`): same flow with
  Win32 lookup and GDI capture. Needs an interactive desktop session and
  OpenGL 3.3; it exits `77` (skip) when the environment cannot host the fake
  window — GitHub's Windows runner has no GL 2.0+, so the CI job reports a
  warning there rather than a failure.
- **CI** (`.github/workflows/`): `check` (fmt, clippy `-D warnings`, tests),
  `clippy-windows` (the only place Windows-only code is linted),
  `integration-sway`, `integration-windows`, `nix-build`,
  `build-windows-release`, `build-linux-appimage`.

## Troubleshooting

- **Run `little_oil doctor` first.** It prints one line per check: `desktop`,
  `config`, `desktop-capture`, `game-window`, `region-game`/`region-inv`/
  `region-stash`/`region-map`, `region-bounds`, `inv-samples`, `focus-clicks`,
  `pointer-scale`, the backend's helper binaries (`bin-grim`,
  `bin-hyprctl`/`bin-niri`/`bin-swaymsg`) and `uinput`.
- **"Game window region not set / Inventory region not calibrated"** — the
  message names both fixes: open the GUI Setup tab, or run
  `little_oil set-region window` / `set-region inventory`.
- **"Path of Exile window not found"** — launch the game, or drag the window
  region by hand; KDE and GNOME never report window geometry, and niri can lack
  it until the scrolling layout places the tile.
- **"X11 is no longer supported"** — remove the `platform` key from the config;
  the app looks under `$XDG_CONFIG_HOME`/`%APPDATA%` and prints the exact path.
- **Clicks land short or overshoot** (relative-pointer desktops only) — re-run
  `little_oil calibrate-pointer` on a static screen, or set `pointer_scale`
  manually.
- **`calibrate-pointer` says "found N changed regions"** — something on screen
  is animating (clock, terminal, video); close it and retry.
- **The game never receives Ctrl/keys** — increase `focus_clicks`; on niri,
  lower it if the focus click grabs an item.
- **Regions point at stale pixels** — the layout moved (especially niri
  scrolling). Re-drag the regions; keep the game fullscreen on niri.
- **No notifications on Linux** — `notify-rust` needs a running notification
  daemon; the same messages also appear in the GUI status line.
- **A Windows hotkey stopped working** — a value in `hotkeys` failed to parse,
  duplicated another action, or names an unknown action. The app logs
  `hotkey <id>: <err>`; clear the field in Settings → Hotkeys.
- **Roll log** — every rolled tooltip is appended to
  `$XDG_CONFIG_HOME/little_oil/rolls.log` as JSONL.
