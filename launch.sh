#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
BIN=rust/target/release/karaokemachine
newer="$(find rust/src rust/Cargo.toml rust/Cargo.lock -newer "$BIN" -print -quit 2>/dev/null || true)"
if [ ! -x "$BIN" ] || [ -n "$newer" ]; then
  cargo build --release --manifest-path rust/Cargo.toml
fi
# Native eframe app. A fixed 1x scale keeps the window at 1600x980 even on a
# HiDPI desktop, which is the difference between 16 fps and ~60 fps here.
# Set KARAOKEMACHINE_SOFTWARE_GL=1 to force Mesa software GL on a bad driver.
exec env -u WAYLAND_DISPLAY WINIT_X11_SCALE_FACTOR=1 "$BIN"
