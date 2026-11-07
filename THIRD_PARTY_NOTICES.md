# Third-party notices

Labelize Printer uses Labelize (MIT), GPUI and gpui-component (Apache-2.0),
PAPPL 1.4.12 (Apache-2.0 with LLVM exception), and OpenSSL 3.5.9 (Apache-2.0).
Native packages link CUPS and operating-system frameworks as system libraries.
The native build patches PAPPL to preserve client raster headers and accept
sRGB input for grayscale conversion. See `scripts/build-native.sh`.
The legacy xattr 0.2.3 dependency has a Linux/Android libc compatibility patch;
see `vendor/xattr/PATCH.md` (MIT / Apache-2.0).
GPUI 0.2.2 has an application-local X11 visual-selection patch;
see `vendor/gpui/PATCH.md`. The other platform backends are unchanged.

`scripts/collect-licenses.py` creates the `licenses/` directory and an index from
the pinned Cargo.lock and locally built sources. All installation packages
include that directory. Labelize's embedded font and asset notices are also
included. ZDesigner is downloaded separately from Zebra, and is not redistributed.
