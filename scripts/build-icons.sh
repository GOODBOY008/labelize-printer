#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
logo_master="platform/branding/labelize-studio.png"
read -r width height <<< "$(magick identify -format '%w %h' "$logo_master")"
if [[ "$width" != "$height" || "$width" -lt 1024 ]]; then
  echo 'The logo master must be square and at least 1024 pixels wide.' >&2
  exit 1
fi
# Keep the master alpha channel; every platform derives from this same image.
magick "$logo_master" -filter Lanczos -resize 1024x1024 -strip "PNG32:$work/icon.png"
magick "$work/icon.png" -resize 256x256 -strip PNG32:platform/icon.png
magick "$work/icon.png" -resize 512x512 -strip PNG32:platform/linux/labelize-printer.png
magick "$work/icon.png" -define icon:auto-resize=256,128,64,48,32,24,16 platform/windows/labelize-printer.ico

if command -v iconutil >/dev/null; then
  iconset="$work/LabelizePrinter.iconset"
  mkdir "$iconset"
  for size in 16 32 128 256 512; do
    magick "$work/icon.png" -resize "${size}x${size}" -strip "PNG32:$iconset/icon_${size}x${size}.png"
    retina=$((size * 2))
    magick "$work/icon.png" -resize "${retina}x${retina}" -strip "PNG32:$iconset/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns "$iconset" -o platform/macos/LabelizePrinter.icns
fi
