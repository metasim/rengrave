#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch
#
# Renders the application icon source (an SVG) into a macOS icon set and Rengrave.icns.
# Needs rsvg-convert (brew install librsvg) and iconutil (included with macOS).
#
# Output, next to the SVG, in crates/rengrave_gui/assets/icon:
#   Rengrave.iconset/   the ten PNG sizes iconutil expects
#   Rengrave.icns       the finished icon

set -euo pipefail

cd "$(dirname "$0")/.."

DIR=crates/rengrave_gui/assets/icon
SVG=$DIR/rengrave-icon.svg
SET=$DIR/Rengrave.iconset

for tool in rsvg-convert iconutil; do
    if ! command -v "$tool" > /dev/null; then
        echo "$tool is not installed" >&2
        exit 1
    fi
done

rm -rf "$SET"
mkdir -p "$SET"

# Each entry is "<pixel size> <file name>".
while read -r px name; do
    rsvg-convert --width "$px" --height "$px" "$SVG" --output "$SET/$name"
done << 'EOF'
16 icon_16x16.png
32 icon_16x16@2x.png
32 icon_32x32.png
64 icon_32x32@2x.png
128 icon_128x128.png
256 icon_128x128@2x.png
256 icon_256x256.png
512 icon_256x256@2x.png
512 icon_512x512.png
1024 icon_512x512@2x.png
EOF

iconutil --convert icns --output "$DIR/Rengrave.icns" "$SET"
echo "Wrote $DIR/Rengrave.icns"
