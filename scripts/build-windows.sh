#!/usr/bin/env bash
# Build KaraokeMachine for Windows x86_64 and produce a portable zip.
#
# Run this yourself when you want a Windows package:
#     ./scripts/build-windows.sh
#
# It cross-compiles from Linux, stages the exe with only the generated warm-up
# library (never the commercial local library), and writes:
#     dist-windows/KaraokeMachine-Windows-x64.zip
#
# Prerequisites:
#   rustup target add x86_64-pc-windows-gnu
#   sudo apt install mingw-w64 zip
#
# The script never uploads or publishes anything. It only writes under
# dist-windows/.
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/.." && pwd)"
TARGET="x86_64-pc-windows-gnu"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
BUILT="$ROOT/rust/target/$TARGET/release/karaokemachine.exe"
DIST_ROOT="$ROOT/dist-windows"
DIST_DIR="$DIST_ROOT/KaraokeMachine"
DIST_EXE="$DIST_DIR/karaokemachine.exe"
ZIP="$DIST_ROOT/KaraokeMachine-Windows-x64.zip"
PACKAGING_README="$ROOT/packaging/README-WINDOWS.txt"

cd "$ROOT"

if ! "$CARGO" --version >/dev/null 2>&1; then
  echo "error: cargo not found at $CARGO" >&2
  echo "Set CARGO=/path/to/cargo or install rustup." >&2
  exit 1
fi

if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
  echo "error: Rust target $TARGET is not installed." >&2
  echo "Run: rustup target add $TARGET" >&2
  exit 1
fi

if ! command -v zip >/dev/null 2>&1; then
  echo "error: zip is not installed. Run: sudo apt install zip" >&2
  exit 1
fi

if [[ ! -d "$ROOT/assets/songs" ]]; then
  echo "error: assets/songs is missing. Refusing to build an empty package." >&2
  exit 1
fi

if [[ ! -f "$PACKAGING_README" ]]; then
  echo "error: packaging/README-WINDOWS.txt is missing." >&2
  exit 1
fi

echo "Building $TARGET..."
"$CARGO" build --release --manifest-path rust/Cargo.toml --target "$TARGET"

if [[ ! -f "$BUILT" ]]; then
  echo "error: build succeeded but $BUILT is missing." >&2
  exit 1
fi

echo "Staging $DIST_DIR..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

install -m 755 "$BUILT" "$DIST_EXE"
cp "$PACKAGING_README" "$DIST_DIR/README-WINDOWS.txt"

# Ship the app and the generated warm-ups only. Commercial audio and the local
# library must never be included in a distributed package. Users obtain songs
# through the map browser or the upload flow, with audio fetched from its source.
mkdir -p "$DIST_DIR/assets/songs"
if [[ -f "$ROOT/assets/icon.png" ]]; then
  cp "$ROOT/assets/icon.png" "$DIST_DIR/assets/icon.png"
  cp "$ROOT/assets/icon.png" "$DIST_DIR/icon.png"
fi
if [[ -f "$ROOT/assets/songs/scales.json" ]]; then
  cp "$ROOT/assets/songs/scales.json" "$DIST_DIR/assets/songs/scales.json"
fi
shopt -s nullglob
for warmup in "$ROOT"/assets/songs/scale-*; do
  [[ -d "$warmup" ]] || continue
  cp -a "$warmup" "$DIST_DIR/assets/songs/"
done
shopt -u nullglob

SONGS=$(find "$DIST_DIR/assets/songs" -name song.json | wc -l | tr -d ' ')
if [[ "$SONGS" -eq 0 ]]; then
  echo "error: the staged library has no song.json files." >&2
  exit 1
fi

echo "Packaging $ZIP..."
rm -f "$ZIP"
(
  cd "$DIST_ROOT"
  zip -qr "$(basename "$ZIP")" "$(basename "$DIST_DIR")"
)

echo
echo "Done."
echo "  folder:  $DIST_DIR"
echo "  zip:     $ZIP"
echo "  songs:   $SONGS"
stat -c '  size:    %s bytes' "$ZIP"
sha256sum "$ZIP"
echo
echo "Next: copy the zip wherever you want. The app is unsigned, so Windows"
echo "SmartScreen may ask you to choose More info -> Run anyway."
