#!/usr/bin/env bash
# Headless sway integration test for `little_oil empty`.
#
# Starts sway with the headless backend and the pixman renderer (no GPU, no
# session), opens `examples/fake_poe.rs` as a "Path of Exile" window drawing a
# synthetic 12x5 inventory, and drives the real binaries end to end:
#
#   1. `little_oil reset_inv` samples the empty inventory,
#   2. five items appear in the fake window,
#   3. `little_oil empty` clicks every occupied cell,
#   4. the composited frame is captured and must show an empty inventory.
#
# Two headless-only accommodations, both about sway having no input devices of
# its own:
#
# * Ctrl+click is what moves an item out of the inventory, and little_oil
#   injects Ctrl through uinput — a kernel device a compositor can only read
#   with a libinput backend, which needs a real seat/session. The harness
#   therefore holds Ctrl with `wtype` (sway's virtual-keyboard protocol). The
#   pointer path — absolute moves and buttons — is still little_oil's own wlr
#   virtual pointer, and the fake reports per-click Ctrl state so the test
#   proves the modifier was down during the macro.
# * little_oil only enables absolute virtual-pointer coordinates when
#   `niri msg outputs` succeeds (see src/platform/virtual_pointer.rs). wlroots
#   assigns `motion_absolute` the same meaning as niri, so
#   `tests/integration/bin/niri` answers that command from sway's IPC. Without
#   it the backend falls back to uinput relative motion, which headless sway
#   cannot receive at all.
#
# Requires: sway, swaymsg, grim, wtype, python3, and a writable /dev/uinput
# (little_oil opens it at startup; see the CI job for the modprobe/chmod).
#
# Environment:
#   LITTLE_OIL_BIN  little_oil binary   (default target/debug/little_oil)
#   FAKE_POE_BIN    fake_poe binary     (default target/debug/examples/fake_poe)
# Missing binaries are built with `cargo build --bin little_oil --example fake_poe`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LITTLE_OIL="${LITTLE_OIL_BIN:-$ROOT/target/debug/little_oil}"
FAKE_POE="${FAKE_POE_BIN:-$ROOT/target/debug/examples/fake_poe}"

# Flat colors the fake paints: empty slot, then item. Kept in sync with
# examples/fake_poe.rs.
EMPTY_RGB="40 40 40"
ITEM_RGB="200 120 40"
# Pixels whose color is within this much of the item color count as item.
TOLERANCE=8
# Inventory panel offset inside the window and its size (fake_poe layout).
PANEL_X=40
PANEL_Y=40
PANEL_W=1200
PANEL_H=600

WORK=""
SWAY_PID=""
FAKE_PID=""
WTYPE_PID=""

cleanup() {
  local status=$?
  [ -n "$WTYPE_PID" ] && kill "$WTYPE_PID" 2>/dev/null || true
  [ -n "$FAKE_PID" ] && kill "$FAKE_PID" 2>/dev/null || true
  [ -n "$SWAY_PID" ] && kill "$SWAY_PID" 2>/dev/null || true
  if [ -n "$WORK" ] && [ "$status" -ne 0 ]; then
    echo "--- diagnostics (work dir: $WORK)" >&2
    for f in sway.log fake.out fake.err little_oil.log; do
      [ -f "$WORK/$f" ] || continue
      echo "--- $f (tail)" >&2
      tail -n 30 "$WORK/$f" >&2 || true
    done
  fi
  if [ -n "$WORK" ] && [ "$status" -eq 0 ]; then
    rm -rf "$WORK"
  fi
  exit "$status"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# wait_for <pattern> <file> <timeout-tenths>
wait_for() {
  local pattern=$1 file=$2 tries=$3
  for _ in $(seq 1 "$tries"); do
    grep -qE -- "$pattern" "$file" 2>/dev/null && return 0
    sleep 0.1
  done
  return 1
}

for tool in sway swaymsg grim wtype python3 cargo; do
  command -v "$tool" >/dev/null || fail "$tool is not on PATH"
done

if [ ! -w /dev/uinput ]; then
  fail "/dev/uinput is not writable — little_oil opens a uinput device at startup.
  Run: sudo modprobe uinput && sudo chmod 666 /dev/uinput"
fi

if [ ! -x "$LITTLE_OIL" ] || [ ! -x "$FAKE_POE" ]; then
  echo "== building little_oil and fake_poe"
  (cd "$ROOT" && cargo build --bin little_oil --example fake_poe)
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/little_oil_sway.XXXXXX")"
mkdir -p "$WORK/run" "$WORK/cfg/little_oil" "$WORK/bin"
chmod 700 "$WORK/run"
install -m 755 "$ROOT/tests/integration/bin/niri" "$WORK/bin/niri"

export XDG_RUNTIME_DIR="$WORK/run"
export XDG_CONFIG_HOME="$WORK/cfg"
export PATH="$WORK/bin:$PATH"

cat > "$WORK/sway.conf" <<'CONF'
output HEADLESS-1 resolution 1920x1080 position 0 0 bg #000000 solid_color
for_window [title="Path of Exile"] floating enable
CONF

echo "== starting headless sway"
WLR_BACKENDS=headless WLR_RENDERER=pixman \
  sway -c "$WORK/sway.conf" --unsupported-gpu >"$WORK/sway.log" 2>&1 &
SWAY_PID=$!

shopt -s nullglob
WAYLAND_SOCKET=""
for _ in $(seq 1 200); do
  for f in "$XDG_RUNTIME_DIR"/wayland-*; do
    case "$f" in *.lock) continue ;; esac
    WAYLAND_SOCKET=$f
  done
  [ -n "$WAYLAND_SOCKET" ] && break
  sleep 0.05
done
[ -n "$WAYLAND_SOCKET" ] || fail "sway did not create a Wayland socket"
WAYLAND_DISPLAY="$(basename "$WAYLAND_SOCKET")"
export WAYLAND_DISPLAY

SWAYSOCK=""
for _ in $(seq 1 200); do
  for f in "$XDG_RUNTIME_DIR"/sway-ipc.*.sock; do
    SWAYSOCK=$f
  done
  [ -n "$SWAYSOCK" ] && break
  sleep 0.05
done
[ -n "$SWAYSOCK" ] || fail "sway did not create its IPC socket"
export SWAYSOCK
swaymsg -t get_version >/dev/null || fail "swaymsg cannot talk to sway"

echo "== starting fake_poe"
echo 0 > "$WORK/items"
LIBGL_ALWAYS_SOFTWARE=1 FAKE_POE_ITEMS="$WORK/items" \
  "$FAKE_POE" >"$WORK/fake.out" 2>"$WORK/fake.err" &
FAKE_PID=$!
wait_for '^READY$' "$WORK/fake.out" 300 || fail "fake_poe never painted its first frame"

# Real client-area geometry (the compositor may ignore the requested position
# and the window may have a border).
for _ in $(seq 1 100); do
  GEOMETRY="$(swaymsg -t get_tree -r | python3 -c '
import json, sys

def walk(node):
    yield node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        yield from walk(child)

for node in walk(json.load(sys.stdin)):
    if node.get("name") == "Path of Exile":
        rect, window_rect = node["rect"], node["window_rect"]
        print(
            rect["x"] + window_rect["x"],
            rect["y"] + window_rect["y"],
            window_rect["width"],
            window_rect["height"],
        )
        break
' 2>/dev/null)" || GEOMETRY=""
  [ -n "$GEOMETRY" ] && break
  sleep 0.05
done
[ -n "$GEOMETRY" ] || fail "the fake window never appeared in sway's tree"
read -r ORIGIN_X ORIGIN_Y CLIENT_W CLIENT_H <<<"$GEOMETRY"
echo "== fake window client area: ${CLIENT_W}x${CLIENT_H} at (${ORIGIN_X}, ${ORIGIN_Y})"
[ "$CLIENT_W" -ge $((PANEL_X + PANEL_W)) ] || fail "window too narrow (${CLIENT_W}px)"
[ "$CLIENT_H" -ge $((PANEL_Y + PANEL_H)) ] || fail "window too short (${CLIENT_H}px)"

INV_X=$((ORIGIN_X + PANEL_X))
INV_Y=$((ORIGIN_Y + PANEL_Y))

python3 - "$XDG_CONFIG_HOME/little_oil/config.json" \
  "$INV_X" "$INV_Y" "$ORIGIN_X" "$ORIGIN_Y" "$CLIENT_W" "$CLIENT_H" <<'PY'
import json
import sys

path, inv_x, inv_y, win_x, win_y, win_w, win_h = sys.argv[1], *map(int, sys.argv[2:8])
config = {
    # focus_clicks 0: the test needs exactly the macro's clicks on stdout.
    "focus_clicks": 0,
    "pointer_scale": 1.0,
    "inv_region": {"x": inv_x, "y": inv_y, "width": 1200, "height": 600},
    "game_window_region": {"x": win_x, "y": win_y, "width": win_w, "height": win_h},
}
with open(path, "w") as f:
    json.dump(config, f, indent=1)
PY

# ── helpers ─────────────────────────────────────────────────────

# count_item_pixels <ppm>: how many pixels of a P6 PPM are the item color.
count_item_pixels() {
  python3 - "$1" "$TOLERANCE" "$ITEM_RGB" <<'PY'
import sys

path, tolerance, rgb = sys.argv[1], int(sys.argv[2]), tuple(map(int, sys.argv[3].split()))
data = open(path, "rb").read()
# P6 header: magic, width, height, maxval — then raw RGB triples.
parts, i = [], 0
while len(parts) < 4:
    while data[i : i + 1].isspace():
        i += 1
    if data[i : i + 1] == b"#":
        while data[i : i + 1] != b"\n":
            i += 1
        continue
    start = i
    while not data[i : i + 1].isspace():
        i += 1
    parts.append(data[start:i])
pixels = data[i + 1 :]
count = 0
for offset in range(0, len(pixels) - 2, 3):
    if all(abs(pixels[offset + c] - rgb[c]) <= tolerance for c in range(3)):
        count += 1
print(count)
PY
}

capture_inventory() {
  grim -g "$INV_X,$INV_Y ${PANEL_W}x${PANEL_H}" -t ppm "$1"
}

# count_item_pixels retried for up to ~5 s, printed on success.
await_item_pixels() {
  local mode=$1 out=$2
  for _ in $(seq 1 50); do
    capture_inventory "$out"
    local count
    count="$(count_item_pixels "$out")"
    if [ "$mode" = "nonzero" ] && [ "$count" -gt 1000 ]; then
      echo "$count"
      return 0
    fi
    if [ "$mode" = "zero" ] && [ "$count" -eq 0 ]; then
      echo "$count"
      return 0
    fi
    sleep 0.1
  done
  return 1
}

# ── reset_inv on the empty inventory ────────────────────────────

echo "== little_oil reset_inv (empty inventory)"
"$LITTLE_OIL" reset_inv >"$WORK/little_oil.log" 2>&1 \
  || fail "reset_inv exited non-zero: $(cat "$WORK/little_oil.log")"

python3 - "$XDG_CONFIG_HOME/little_oil/config.json" "$EMPTY_RGB" "$TOLERANCE" <<'PY'
import json
import sys

path, empty, tolerance = sys.argv[1], tuple(map(int, sys.argv[2].split())), int(sys.argv[3])
samples = json.load(open(path)).get("inv_samples")
assert samples is not None, "reset_inv did not save inv_samples"
assert len(samples) == 60, f"expected 60 slots x 3 samples, got {len(samples)}"
for index, slot in enumerate(samples):
    assert len(slot) == 3, f"slot {index} has {len(slot)} samples"
    for value in slot:
        rgb = ((value >> 24) & 0xFF, (value >> 16) & 0xFF, (value >> 8) & 0xFF)
        assert all(
            abs(a - b) <= tolerance for a, b in zip(rgb, empty)
        ), f"slot {index} sampled {rgb}, not the empty-slot color {empty}"
PY
echo "   saved 60 slots x 3 samples of the empty inventory"

# ── five items appear ──────────────────────────────────────────

echo "== placing 5 items (diagonal pattern: both axes, non-contiguous)"
echo diagonal > "$WORK/items"
ITEM_PIXELS="$(await_item_pixels nonzero "$WORK/before.ppm")" \
  || fail "the fake window never showed the 5 placed items"
echo "   ${ITEM_PIXELS} item-colored pixels on screen"

# ── Ctrl held, then empty ──────────────────────────────────────

echo "== holding Ctrl with wtype (headless sway has no uinput seat)"
wtype -M ctrl -s 60000 &
WTYPE_PID=$!
wait_for '^MOD ctrl=1$' "$WORK/fake.out" 50 \
  || fail "the fake window never saw Ctrl held — is it focused?"

echo "== little_oil empty"
"$LITTLE_OIL" empty >>"$WORK/little_oil.log" 2>&1 \
  || fail "empty exited non-zero: $(cat "$WORK/little_oil.log")"
kill "$WTYPE_PID" 2>/dev/null || true
WTYPE_PID=""

# ── assertions ─────────────────────────────────────────────────

ALL_CLICKS="$(grep '^CLICK ' "$WORK/fake.out" || true)"
LEFT_CLICKS="$(grep -c '^CLICK left' "$WORK/fake.out" || true)"
TOTAL_CLICKS="$(grep -c '^CLICK ' "$WORK/fake.out" || true)"
[ "$LEFT_CLICKS" -eq 5 ] \
  || fail "expected 5 left clicks, got ${LEFT_CLICKS}:
${ALL_CLICKS:-<none>}"
[ "$TOTAL_CLICKS" -eq "$LEFT_CLICKS" ] \
  || fail "non-left clicks were sent:
${ALL_CLICKS:-<none>}"
if grep '^CLICK left' "$WORK/fake.out" | grep -vq 'ctrl=1'; then
  fail "a click arrived without Ctrl held:
$ALL_CLICKS"
fi
if grep '^CLICK left' "$WORK/fake.out" | grep -q 'cell=-'; then
  fail "a click landed outside the inventory panel:
$ALL_CLICKS"
fi
CELLS="$(grep '^CLICK left' "$WORK/fake.out" | sed -E 's/.* cell=//' | sort | tr '\n' ' ')"
[ "$CELLS" = "0,0 11,4 3,1 6,2 9,3 " ] \
  || fail "clicks did not cover each item cell exactly once: ${CELLS}"

FINAL_PIXELS="$(await_item_pixels zero "$WORK/after.ppm")" \
  || fail "inventory is not empty after \`little_oil empty\` ($(count_item_pixels "$WORK/after.ppm") item pixels left)"
[ "$FINAL_PIXELS" -eq 0 ] || fail "found ${FINAL_PIXELS} item pixels after empty"

echo "PASS: 5 occupied cells clicked once each with Ctrl held; inventory is empty"
