#!/usr/bin/env sh
# Build a Windows release of little_oil from this machine (Linux + nix).
#
# Uses the x86_64-pc-windows-gnu target with a mingw-w64 cross toolchain from
# nixpkgs. The build links libgcc/libstdc++/mcfgthread statically, so the exe
# needs no mingw runtime DLLs — the zip runs on a stock Windows 10/11 box.
# (The GUI uses WGL/EGL via dynamic loading; no extra DLLs required.)
#
# For the most conventional distribution build (MSVC CRT) build on a Windows
# machine instead:
#   cargo build --release --target x86_64-pc-windows-msvc
#
# Usage: scripts/build-windows.sh [out-dir]
set -e

cd "$(dirname "$0")/.."
mkdir -p "${1:-dist/windows}"
OUT="$(realpath "${1:-dist/windows}")"
TARGET="x86_64-pc-windows-gnu"

# rust's windows-gnu target links `-l:libpthread.a`; this nix toolchain ships
# mcfgthread instead. mcfgthread *is* the pthread implementation here, so a
# static archive named libpthread.a pointing at it satisfies the linker.
SHIM="$(mktemp -d)"
trap 'rm -rf "$SHIM"' EXIT
MCFG="$(find /nix/store/*mcfgthread-x86_64-w64-mingw32* -name libmcfgthread.a 2>/dev/null | head -1)"
if [ -z "$MCFG" ]; then
  echo "error: libmcfgthread.a not found — is the mingw toolchain in the nix store?" >&2
  exit 1
fi
cp "$MCFG" "$SHIM/libpthread.a"

echo "==> Cross-compiling release for $TARGET"
nix shell \
  nixpkgs#rustup \
  "nixpkgs#pkgsCross.mingwW64.buildPackages.gcc" \
  nixpkgs#gcc \
  --command env "RUSTFLAGS=-C link-arg=-L$SHIM" \
    rustup run stable cargo build --release --target "$TARGET"

EXE="target/$TARGET/release/little_oil.exe"
test -f "$EXE" || { echo "build failed: $EXE missing" >&2; exit 1; }

mkdir -p "$OUT"
cp "$EXE" "$OUT/"

ZIP="$(realpath dist)/little_oil-windows.zip"
(cd "$OUT" && rm -f "$ZIP" && zip -q -r "$ZIP" .)
echo "==> Done: $OUT/little_oil.exe ($(du -h "$OUT/little_oil.exe" | cut -f1))"
echo "    Zip: $ZIP"
