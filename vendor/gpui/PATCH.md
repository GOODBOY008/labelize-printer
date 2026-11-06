# GPUI 0.2.2 X11 visual selection patch

Upstream: https://github.com/zed-industries/zed/tree/69e2130295c2649963eb639fc70b4f2ee8ea1624/crates/gpui
The published crate records this Git revision with a dirty working tree; the
crates.io archive checksum, rather than the Git tree, pins our source.
Crates.io checksum: `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`. License: Apache-2.0.

This application's windows paint an opaque background. The X11 backend chooses
an opaque RGB visual instead of always choosing an ARGB visual. The latter can
expose no OPAQUE Vulkan composite-alpha mode on Mesa software rendering and
produce a blank window. Only `src/platform/linux/x11/window.rs` is changed to
select the opaque visual. The application's `crates/desktop/src/main.rs` also
performs one deferred X11 configure/restore cycle after window creation to
initialize the mapped drawable without requiring a manual resize.
macOS, Windows and Wayland backends are unchanged. Revisit this application-local
patch when updating GPUI or adding transparent top-level windows.
