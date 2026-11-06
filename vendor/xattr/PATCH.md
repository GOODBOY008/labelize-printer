# xattr 0.2.3 compatibility patch

Source: crates.io xattr 0.2.3, checksum
`6d1526bbe5aaeb5eb06885f4d987bcdfa5e23187055de9b83fe00156a821fabc`.
Upstream: https://github.com/Stebalien/xattr (MIT / Apache-2.0).

GPUI 0.2.2's HTTP/tar dependency chain uses this release. Modern libc removed
the deprecated Linux/Android ENOATTR alias; those targets use ENODATA instead.
Only `src/sys/mod.rs` is changed. macOS and BSD retain their ENOATTR constant.
Remove the patch when the GPUI dependency chain moves to maintained xattr APIs.

Reference: https://github.com/rust-lang/libc/issues/1356
