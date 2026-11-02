#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
if [ ! -d rust/target ] || [ ! -x rust/target/release/karaokemachine ]; then
  (cd rust && cargo build --release)
fi
# Native eframe app. A fixed 1x scale keeps the window at 1600x980 even on a
# HiDPI desktop, which is the difference between 16 fps and ~60 fps here.
# Set KARAOKEMACHINE_SOFTWARE_GL=1 to force Mesa software GL on a bad driver.
exec env -u WAYLAND_DISPLAY WINIT_X11_SCALE_FACTOR=1 rust/target/release/karaokemachine
