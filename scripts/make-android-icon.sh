#!/usr/bin/env bash
# Generate the Android (and desktop/iOS) app icon set from an exported
# Phototropic Decay design.
#
#   scripts/make-android-icon.sh docs/icon-generation/phototropic-<id>.svg
#
# Expects the lab's SVG export; if the matching <id>.json params file sits
# beside it, the adaptive-icon background color is taken from its `bg` param
# (default #000000). Rasterizes via headless Chromium so the SVG bloom
# filter renders with a real browser engine, then:
#   1. writes mobile/src-tauri/icons/source.png (1024px)
#   2. runs `cargo tauri icon` (full icon set + gen/android mipmaps)
#   3. re-applies the adaptive-icon layers: background color + foreground
#      mipmaps with the mark fitted to the 72/108 safe zone, so Android 8+
#      launchers show the whole design instead of a center crop.
#
# Release flow: export SVG+params from docs/icon-generation/phototropic-decay.html,
# commit the pair into docs/icon-generation/, run this script, commit the icons.
set -euo pipefail

SVG=${1:?usage: $0 <design>.svg  (an export from docs/icon-generation/phototropic-decay.html)}
[ -f "$SVG" ] || { echo "error: $SVG not found" >&2; exit 1; }

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TAURI_DIR=$ROOT/mobile/src-tauri
RES=$TAURI_DIR/gen/android/app/src/main/res
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

BROWSER=$(command -v chromium || command -v chromium-browser || command -v google-chrome || true)
[ -n "$BROWSER" ] || { echo "error: need chromium/chrome for SVG rasterization" >&2; exit 1; }
TAURI_BIN=$(command -v cargo-tauri || echo "$HOME/.cargo/bin/cargo-tauri")
[ -x "$TAURI_BIN" ] || { echo "error: cargo-tauri not found (cargo install tauri-cli)" >&2; exit 1; }

# adaptive-icon background: the design's own bg param when the json is beside the svg
JSON=${SVG%.svg}.json
BG='#000000'
if [ -f "$JSON" ]; then
  BG=$(python3 -c "import json; d=json.load(open('$JSON')); print((d.get('params') or d).get('bg', '#000000'))")
  echo "using bg $BG from $(basename "$JSON")"
else
  echo "warning: no $(basename "$JSON") beside the svg — assuming bg $BG" >&2
fi

# 1. rasterize (SVG is 1000x1000; icon source wants 1024)
"$BROWSER" --headless=new --disable-gpu --hide-scrollbars \
  --window-size=1000,1000 --virtual-time-budget=10000 \
  --screenshot="$TMP/mark-1000.png" "file://$(realpath "$SVG")" 2>/dev/null
magick "$TMP/mark-1000.png" -resize 1024x1024 "$TAURI_DIR/icons/source.png"

# 2. full icon set — writes icons/ and gen/android res mipmaps
(cd "$TAURI_DIR" && "$TAURI_BIN" tauri icon icons/source.png)

# 3. adaptive-icon layers
sed -i "s|<color name=\"ic_launcher_background\">[^<]*</color>|<color name=\"ic_launcher_background\">$BG</color>|" \
  "$RES/values/ic_launcher_background.xml"
for spec in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
  d=${spec%%:*} S=${spec##*:}
  A=$(( S * 2 / 3 ))
  magick "$TMP/mark-1000.png" -resize ${A}x${A} -background "$BG" -gravity center -extent ${S}x${S} \
    "$RES/mipmap-$d/ic_launcher_foreground.png"
done

echo
echo "Icon set regenerated from $(basename "$SVG") (adaptive bg $BG)."
echo "Review: git status mobile/src-tauri && commit."
