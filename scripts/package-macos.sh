#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
profile="${1:-release}"
stage="dist/macos-$profile"
architecture="${2:-$(uname -m)}"
binary_dir="target/$profile"
if [[ "$architecture" != "$(uname -m)" ]]; then
  rust_arch="$architecture"
  if [[ "$rust_arch" == arm64 ]]; then rust_arch=aarch64; fi
  binary_dir="target/$rust_arch-apple-darwin/$profile"
  stage="dist/macos-$architecture-$profile"
fi
app="$stage/Labelize Printer.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/platform"
cp "$binary_dir/labelize-printer" "$binary_dir/labelize-printer-daemon" "$binary_dir/labelize-printer-ipp" "$app/Contents/MacOS/"
cp platform/macos/Info.plist "$app/Contents/Info.plist"
cp -R platform/macos "$app/Contents/Resources/platform/"
cp LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
cp -R licenses "$app/Contents/Resources/"
for name in labelize-printer-ipp labelize-printer-daemon labelize-printer; do
  binary="$app/Contents/MacOS/$name"
  if otool -L "$binary" | grep -E '/opt/homebrew|/usr/local|/Volumes' >/dev/null; then
    echo "Unbundled build-machine dependency: $binary" >&2; exit 1
  fi
  codesign --force --sign - "$binary"
done
codesign --force --sign - "$app"
pkgbuild --component "$app" --install-location /Applications --identifier com.labelize.printer \
  --version 0.1.0 "dist/LabelizePrinter-0.1.0-macos-$architecture-$profile.pkg"
echo 'Unsigned internal trial package. Open the app to finish per-user queue setup.'
