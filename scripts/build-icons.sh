#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
magick -background none -density 384 platform/icon.svg -resize 1024x1024 "PNG32:$work/icon.png"
magick "$work/icon.png" -resize 256x256 PNG32:platform/icon.png
magick platform/icon.png -define icon:auto-resize=256,128,64,48,32,24,16 platform/windows/labelize-printer.ico

if command -v iconutil >/dev/null; then
  iconset="$work/LabelizePrinter.iconset"
  mkdir "$iconset"
  for size in 16 32 128 256 512; do
    magick "$work/icon.png" -resize "${size}x${size}" "PNG32:$iconset/icon_${size}x${size}.png"
    retina=$((size * 2))
    magick "$work/icon.png" -resize "${retina}x${retina}" "PNG32:$iconset/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns "$iconset" -o platform/macos/LabelizePrinter.icns
fi
