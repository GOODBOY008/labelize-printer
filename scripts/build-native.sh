#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
project_root="$PWD"
ssl_prefix="$project_root/.cache/openssl"
if [[ -n "${LABELIZE_ARCH:-}" && "$LABELIZE_ARCH" != "$(uname -m)" ]]; then ssl_prefix="$ssl_prefix-$LABELIZE_ARCH"; fi
if [[ -f "$ssl_prefix/lib/pkgconfig/openssl.pc" ]]; then
  export PKG_CONFIG_PATH="$ssl_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi
profile="${1:-debug}"
mkdir -p .cache "target/$profile"
version=1.4.12
archive=".cache/pappl-$version.tar.gz"
if [[ ! -f "$archive" ]]; then
  curl -fL --retry 3 "https://github.com/michaelrsweet/pappl/releases/download/v$version/pappl-$version.tar.gz" -o "$archive"
fi
python3 - "$archive" <<'PY'
import hashlib, sys
assert hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest() == '1684c4e06446e9f7d93a39729fa0ba56f07a4007560080fdad7d0e2076a3615f', 'PAPPL checksum mismatch'
PY
source_dir="$project_root/.cache/pappl-$version"
if [[ ! -f "$source_dir/configure" ]]; then tar xzf "$archive" -C .cache; fi
# PAPPL normally selects a printer canvas for 1-bit data. This virtual printer
# must preserve the caller's raster header, including per-page size/resolution.
# The adapter, unlike a physical printer driver, accepts and expands both 1/8-bit.
python3 - "$source_dir/pappl/job-process.c" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); s = p.read_text()
old = 'if (options->header.cupsBitsPerPixel >= 8 && header.cupsBitsPerPixel >= 8)'
new = 'if (header.cupsBitsPerPixel == 1 || (options->header.cupsBitsPerPixel >= 8 && header.cupsBitsPerPixel >= 8))'
assert old in s or new in s, 'PAPPL header patch no longer applies'
if new not in s: p.write_text(s.replace(old, new))
PY
native_flags="-O2"
if [[ "$(uname -s)" == Darwin ]]; then
  native_flags="$native_flags -arch ${LABELIZE_ARCH:-$(uname -m)} -mmacosx-version-min=13.0"
fi
(
  cd "$source_dir"
  # Reconfiguration can change OpenSSL headers or architecture. Never reuse
  # objects from a previous build configuration (make does not track CFLAGS).
  if [[ -f Makedefs ]]; then make -C pappl clean >/dev/null; fi
  ./configure --prefix="$project_root/.cache/native" --disable-shared --disable-libusb \
    --disable-libpam --disable-libjpeg --disable-libpng --with-tls=openssl CFLAGS="$native_flags" > "$project_root/.cache/pappl-configure.log" 2>&1
  make -C pappl -j4 libpappl.a > "$project_root/.cache/pappl-build.log" 2>&1
)
read -r -a ssl_cflags <<< "$(pkg-config --cflags openssl)"
read -r -a ssl_libs <<< "$(pkg-config --libs-only-L openssl)"
ssl_dir="$(pkg-config --variable=libdir openssl)"
extra=()
if [[ "$(uname -s)" == Darwin ]]; then
  extra=(-framework CoreFoundation -framework SystemConfiguration -framework AppKit)
else
  extra=(-ldl -lavahi-client -lavahi-common)
fi
read -r -a cc_flags <<< "$native_flags"
${CC:-cc} -std=c11 -D_DEFAULT_SOURCE -Wall -Wextra -Werror "${cc_flags[@]}" "${ssl_cflags[@]}" \
  -I"$source_dir" native/ipp/adapter.c "$source_dir/pappl/libpappl.a" \
  "$ssl_dir/libssl.a" "$ssl_dir/libcrypto.a" -lcups -lz -lm -lpthread "${extra[@]}" \
  -o "target/$profile/labelize-printer-ipp"
printf 'Built %s\n' "target/$profile/labelize-printer-ipp"
