#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
[[ "$(uname -m)" == x86_64 ]] || { echo 'Build this amd64 package in an x86_64 environment.' >&2; exit 1; }
version="$(python3 scripts/package/version.py)"
profile="${1:-release}"
binary_dir="${CARGO_TARGET_DIR:-target}/$profile"
stage="dist/deb-$profile"
mkdir -p "$stage/DEBIAN" "$stage/opt/labelize-printer/platform" "$stage/usr/share/applications" \
  "$stage/usr/share/icons/hicolor/256x256/apps" "$stage/usr/share/icons/hicolor/512x512/apps" \
  "$stage/usr/share/doc/labelize-printer"
cp --reflink=never --sparse=never "$binary_dir/labelize-printer" "$binary_dir/labelize-printer-daemon" "$binary_dir/labelize-printer-ipp" "$binary_dir/labelize-printer-updater" "$stage/opt/labelize-printer/"
strip --strip-unneeded "$stage/opt/labelize-printer/labelize-printer" \
  "$stage/opt/labelize-printer/labelize-printer-daemon" "$stage/opt/labelize-printer/labelize-printer-ipp" "$stage/opt/labelize-printer/labelize-printer-updater"
cp -R platform/linux "$stage/opt/labelize-printer/platform/"
cp platform/linux/labelize-printer.desktop "$stage/usr/share/applications/"
# Clear the previous logo when reusing a packaging stage from an older build.
rm -f "$stage/usr/share/icons/hicolor/scalable/apps/labelize-printer.svg"
cp platform/icon.png "$stage/usr/share/icons/hicolor/256x256/apps/labelize-printer.png"
cp platform/linux/labelize-printer.png "$stage/usr/share/icons/hicolor/512x512/apps/labelize-printer.png"
cp LICENSE THIRD_PARTY_NOTICES.md "$stage/usr/share/doc/labelize-printer/"
cp -R licenses "$stage/usr/share/doc/labelize-printer/"
cat > "$stage/DEBIAN/control" <<EOF
Package: labelize-printer
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Labelize Contributors
Depends: libc6 (>= 2.36), libcups2, libavahi-client3, libavahi-common3, libfontconfig1, libfreetype6, libx11-6, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libasound2, libvulkan1, libstdc++6, libgcc-s1, zlib1g, cups-client, cups, cups-filters, policykit-1, xdg-desktop-portal
Recommends: xdg-desktop-portal-gtk, fonts-noto-cjk, mesa-vulkan-drivers
Homepage: https://github.com/GOODBOY008/labelize-studio
Description: Labelize Studio label designer, viewer and virtual printer
 Converts ZPL and system raster printing to PDF or PNG using a per-user daemon.
 Open Labelize Studio to register the local system queue after installation.
EOF
dpkg-deb --build --root-owner-group "$stage" "dist/labelize-studio_${version}_amd64.deb"
