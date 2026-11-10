# Third-party notices

Labelize Studio uses Labelize (MIT), GPUI, gpui-component and gpui-component-assets (Apache-2.0),
PAPPL 1.4.12 (Apache-2.0 with LLVM exception), and OpenSSL 3.5.9 (Apache-2.0).
Native vector PDF uses krilla (MIT / Apache-2.0); scene rasterization uses
tiny-skia (BSD-3-Clause). Their dependencies and notices are included by the
same license collection step used for every package.
Native packages link CUPS and operating-system frameworks as system libraries.
The native build patches PAPPL to preserve client raster headers and accept
sRGB input for grayscale conversion. See [the native build script](scripts/build-native.sh).
The legacy xattr 0.2.3 dependency has a Linux/Android libc compatibility patch;
see [the xattr patch notes](vendor/xattr/PATCH.md) (MIT / Apache-2.0).
GPUI 0.2.2 includes application-local patches for X11 visual selection and
embedded Windows debug shaders; see [the GPUI patch notes](vendor/gpui/PATCH.md).

[The license collection script](scripts/collect-licenses.py) creates the `licenses/` directory and an index from
the pinned Cargo.lock and locally built sources. All installation packages
include that directory. Labelize's embedded font and asset notices are also
included. ZDesigner is downloaded separately from Zebra, and is not redistributed.
