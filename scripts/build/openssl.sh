#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
project_root="$PWD"
version=3.5.9
archive=".cache/openssl-$version.tar.gz"
mkdir -p .cache
if [[ ! -f "$archive" ]]; then
  curl -fL --retry 3 "https://github.com/openssl/openssl/releases/download/openssl-$version/openssl-$version.tar.gz" -o "$archive"
fi
python3 - "$archive" <<'PY'
import hashlib, sys
assert hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest() == '603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a'
PY
if [[ ! -f ".cache/openssl-$version/Configure" ]]; then tar xzf "$archive" -C .cache; fi
cd ".cache/openssl-$version"
args=()
prefix="$project_root/.cache/openssl"
if [[ "$(uname -s)" == Darwin ]]; then
  architecture="${LABELIZE_ARCH:-$(uname -m)}"
  args=("darwin64-$architecture-cc" -mmacosx-version-min=13.0)
  if [[ "$architecture" != "$(uname -m)" ]]; then prefix="$project_root/.cache/openssl-$architecture"; fi
fi
if [[ -f Makefile ]]; then make clean >/dev/null; fi
./Configure no-shared no-tests --prefix="$prefix" --libdir=lib "${args[@]}"
make -j4 build_libs
make install_dev
