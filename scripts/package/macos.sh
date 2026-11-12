#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
version="$(python3 scripts/package/version.py)"
# Earlier Printer bundles used build number 5; keep Apple's bundle ordering monotonic.
build_version="$(python3 -c 'import sys; a,b,c=map(int,sys.argv[1].split(".")); print(f"{a+6}.{b}.{c}")' "$version")"
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
app="${LABELIZE_PRINTER_MACOS_STAGE:-$stage}/Labelize Studio.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/platform"
cp "$binary_dir/labelize-printer" "$binary_dir/labelize-printer-daemon" "$binary_dir/labelize-printer-ipp" "$binary_dir/labelize-printer-updater" "$app/Contents/MacOS/"
cp platform/macos/Info.plist "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build_version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string $version" "$app/Contents/Info.plist"
cp platform/macos/LabelizePrinter.icns "$app/Contents/Resources/"
icon_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$app/Contents/Info.plist")"
test -s "$app/Contents/Resources/$icon_name"
cp -R platform/macos "$app/Contents/Resources/platform/"
cp LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
cp -R licenses "$app/Contents/Resources/"
for name in labelize-printer-ipp labelize-printer-daemon labelize-printer-updater labelize-printer; do
  binary="$app/Contents/MacOS/$name"
  if otool -L "$binary" | sed '1d' | grep -E '/opt/homebrew|/usr/local|/Volumes' >/dev/null; then
    echo "Unbundled build-machine dependency: $binary" >&2; exit 1
  fi
  codesign --force --sign - "$binary"
done
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
payload="$(mktemp -d "${TMPDIR:-/tmp}/labelize-studio-package.XXXXXX")"
trap 'rm -rf "$payload"' EXIT
ditto "$app" "$payload/Labelize Studio.app"
pkgbuild --root "$payload" --component-plist platform/macos/components.plist \
  --scripts platform/macos/installer-scripts --install-location /Applications --identifier com.labelize.printer \
  --version "$version" "dist/LabelizeStudio-$version-macos-$architecture-$profile.pkg"
echo 'Unsigned internal trial package. Open the app to finish per-user queue setup.'
