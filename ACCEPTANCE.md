# Acceptance record

Updated 2026-11-08. [README](README.md) · [Development guide](DEVELOPMENT.md) · [Architecture](ARCHITECTURE.md)

This record separates builds, automated protocol checks, system printing, and native desktop acceptance. Cross-compilation, a configured CI job, or a container test does not establish desktop compatibility on the corresponding target platform. Results below identify the version or development milestone to which they apply.

## Platform coverage

| Area | Recorded result | Environment and limits |
| --- | --- | --- |
| macOS Apple Silicon app and PKG | Built | Current Apple Silicon host; app signed ad hoc, PKG unsigned and not notarized. |
| macOS native system printing | Passed | Preview's print dialog selected Labelize Printer. With GPUI closed, two copies produced one two-page PDF. |
| macOS Intel app and PKG | Built; background checks passed | Earlier x86_64 cross-builds ran the daemon and PAPPL through Rosetta. Intel GPUI desktop acceptance remains pending. |
| Windows x64 app and NSIS installer | UTM trial passed | Windows 11 Pro ARM64, Build 26200, using x64 emulation. Desktop use, installation/upgrade/removal, login startup, and direct RAW output were checked. ZDesigner system printing remains pending. |
| Debian 12 amd64 app, PAPPL, and DEB | Built; background and installed-queue checks passed | Debian 12 amd64 container: DEB installation → CUPS PDF raster filter → PAPPL → two-page PDF, followed by queue and package removal. |
| Ubuntu 24.04 | Target acceptance pending | The same amd64 DEB is offered. Native dependency, login-service, and desktop-printing acceptance has not been established by this record. |
| GPUI input, file selection, and resizing | Partly verified on macOS | Chinese path pasting, native folder selection, settings persistence, resizing, job lists, and opening output. IME composition remains pending. |
| Linux Wayland | Container rendering passed | Nested Weston with Mesa software Vulkan displayed the Chinese interface and job results. Real desktop, portal, and IME checks remain pending. |
| Linux X11 | Container rendering passed | Xvfb/Openbox with Mesa software Vulkan displayed the interface on first launch. This uses the GPUI opaque-RGB visual patch and the application's deferred configure/restore handling. |

Early local trial packages used development builds, with debug symbols stripped from Windows/Linux binaries. Release-profile builds and package artifacts are available through [GitHub Actions](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml). A package must be matched to its source commit before applying these results to it.

## Published v0.1.2

The [v0.1.2 release](https://github.com/GOODBOY008/labelize-printer/releases/tag/v0.1.2) contains four installers built from `36d169d3787182a7c56b0d5e2b17c38afa82e01c`. Its [release build](https://github.com/GOODBOY008/labelize-printer/actions/runs/42347073744) passed core output tests and headless RAW/IPP checks on the applicable runners, plus installed-package CUPS printing in Debian 12.

The release notes record these additional desktop checks:

- macOS 27.2 Apple Silicon: switch English/Simplified Chinese in both directions, retain the preference after restart, preserve unsaved settings while switching, and use the 480 × 360 layout.
- Existing Windows 11 ARM64 VM with x64 emulation: upgrade the v0.1.2 installer, launch the desktop, switch languages, and retain the choice after restart.

The installers remain unsigned; macOS bundles use ad-hoc signatures. These results do not close the native Windows x64, ZDesigner, macOS 13, or Linux desktop acceptance items below.

## Labels workspace checks for v0.2.0

These local results cover the implementation committed as [`abe5220`](https://github.com/GOODBOY008/labelize-printer/commit/abe52203dbf88e96dcee3f626fb79993adb7e720). The workspace ships in v0.2.0 and was absent from v0.1.2. The following local checks are distinct from release CI; each release links its exact source commit and successful build in the release notes.

- **File opening and pagination:** On the current macOS Apple Silicon host, a native file dialog opened a multipage ZPL file. `^PQ2` produced two 50 × 30 mm / 400 × 240 px pages, followed by a 100 × 75 mm / 800 × 600 px page with its own dimensions.
- **Design editing:** An imported Chinese font, text, QR code, and Code 128 rendered correctly. Moving an object, saving the template, and reopening it preserved positions and the embedded font. Chinese content editing, preview refresh, undo/redo, and language switching were checked; language changes retained unsaved content.
- **Export:** PDF and PNG exports reached `Saved`. Actual output was one 102 × 152 mm PDF page and an 816 × 1216 px PNG. The tests used an isolated data directory and daemon, leaving the installed queue's settings unchanged.
- **Layout and document prompts:** Fields, actions, and pagination remained reachable by scrolling in the narrow layout. Property labels wrapping vertically and an undersized source editor were corrected. Cancel/discard behavior when replacing or closing an edited document was checked.
- **Core tests:** All 11 integration tests passed. The five new tests cover preview/output pixel equality for five file languages, per-job settings, binary TSPL, embedded-asset round trips, transparency and QR white modules, PDF dimensions, and invalid input. The original six job/raster tests still passed.
- **Background smoke:** The full local suite passed, including EPL/TSPL/CPCL/DPL submission and isolated preview processes, alongside RAW, crash/retry, restart recovery, PWG/URF, and port-conflict checks.
- **Build checks:** Local Clippy and Windows x64 GNU target checking passed. The new workspace has not been verified on Windows or Linux desktops. IME composition and file dropping are not counted as passed; OS file associations are not registered.

The vendored Labelize parsers and renderer were unchanged for this feature. Direct physical-printer submission, reconstructing design objects from printer-language source, and exporting designs as printer commands are outside this implementation milestone.

## Earlier desktop regression checks

- Windows 11 UTM at 800 × 600 showed the app, title-bar icon, and taskbar icon. Keyboard navigation selected PNG and submitted a test; the job became `Saved` and produced 816 × 1216 px output. Page Down scrolled settings while the bottom action remained available.
- An NSIS upgrade preserved the earlier four jobs, eight output files, and settings. Installed icon configuration and binary hashes were checked, and the daemon resumed listening.
- On macOS Apple Silicon, a native folder picker selected a path containing Chinese characters and spaces; saving settings and generating PNG succeeded. Bottom actions remained usable after shrinking the window. Invalid label width produced an error; reconnecting after a daemon interruption preserved unsaved form input.
- Debian 12 package checks rendered the navigation and bottom actions at 640 × 480 under both Xvfb/Openbox and nested Weston. The updated DEB also passed CUPS-to-two-page-PDF printing.

## Windows UTM evidence

- Interactive NSIS installation and upgrade succeeded. Fixes addressed a missing Common Controls v6 manifest in cross-builds and debug shaders that had depended on a build-machine source path. The corrected app displayed the Chinese setup page and a running background service at 800 × 600.
- The packaged script registered the current user's login task. After signing out and back in, the daemon listened on loopback RAW without GPUI running; named-pipe status queries worked and the two earlier jobs remained present.
- With GPUI closed, a two-label ZPL job was sent in TCP fragments. The first label used `^PQ2`. Separate jobs saved one three-page PDF and three PNGs. The first two pages were 812 × 1218 px (101.5 × 152.25 mm); the last was 600 × 400 px (75 × 50 mm). PNG `pHYs` values were 8,000 pixels/metre.
- NSIS uninstall removed application files, processes, and the login task. Reinstalling and opening GPUI started the daemon again and preserved the four jobs and their output.
- ZDesigner was not installed during this RAW validation. A temporary generic queue's submission failed at `OpenPrinter` with an invalid printer name; its queue and port were removed. The system-dialog-to-ZDesigner path remains unverified and requires installing the official driver under [Zebra's license](https://www.zebra.com/content/experience-fragments/zebra1/us/en/support/eula/end-user-license-agreement-unrestricted-software/master.html).

## macOS native system-printing evidence

1. Registered the local IPP queue as `LabelizePrinter`, displayed as **Labelize Printer**.
2. Saved a GPUI test page through ZPL → Labelize → PDF.
3. Opened the PDF in Preview and selected Labelize Printer, two copies, and 4 × 6 inch media in the native print dialog.
4. Closed GPUI and confirmed its process exited before submitting the print.
5. Background job `75d36d97-8730-42d5-b759-70663fd9777f` reached `Saved`, with output present on disk.
6. `pdfinfo` reported two pages of 288 × 432 pt (101.6 × 152.4 mm). `pdfimages -list` reported two grayscale images at 812 × 1218 px, 203 dpi.
7. Reopened GPUI and confirmed the job and its two-page output in **Recent jobs**.

This establishes system printing through PAPPL and raster output, including operation with the GUI closed, on that macOS host. It does not establish macOS 13 compatibility or coverage of every application and paper setting.

## Earlier automated checks

These are the recorded virtual-printer baseline checks. The workspace's additional tests are listed above.

- The original six core integration tests passed, covering multiple ZPL labels, `^PQ`, per-page dimensions, PDF page count and MediaBox, raster pixels and PNG `pHYs`, settings snapshots, output-commit recovery, same-ID retry, interrupted input, page/canvas limits, and path boundaries.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` passed on macOS. Older indirect GPUI dependencies still emit deprecation/future-compatibility notices.
- Background smoke tests passed on ARM macOS, Intel binaries through Rosetta, and Debian 12 amd64. Coverage includes real RAW fragmentation/EOF, no early submission at `^XZ`, truncated ZPL failure, worker termination/retry, restart, IPP manifest errors, PWG 1/8-bit and Apple Raster, page order/polarity/dimensions, copy handling, truncated rasters, and first-start port conflicts.
- Debian's `--disk-full-dir /disk-full-test` used a dedicated 64 KiB tmpfs to trigger `ENOSPC`. No output was committed; after freeing space, retry saved the job under the same ID.
- The PAPPL C adapter built with Clang and GCC under `-Wall -Wextra -Werror`.
- The installed DEB completed queue creation, native CUPS PDF filtering for two copies, a two-page PDF save, queue removal, and package removal. [scripts/linux-system-smoke.sh](scripts/linux-system-smoke.sh) reproduces this in a dedicated container.
- X11 and Wayland screenshots were inspected for the complete Chinese interface and saved-task state. These checks do not cover physical GPU drivers, IME composition, desktop portals, or user login services.
- Upstream Labelize checks for the parser API passed: 50 existing ZPL parser tests, five parser-option tests, three architecture tests, 170 golden tests, and three diff-report tests, for 231 total. Carrier, unit, and reverse reports were reviewed; golden images and reports were unchanged. See [dependency provenance](vendor/labelize/PROVENANCE.md).

The ordinary background smoke suite uses temporary data and does not create a system queue or login service. The installed-package test creates and removes those resources inside its dedicated container.

## Remaining acceptance work

Use all four installers from the same successful build. Record the source commit, workflow URL, package SHA-256, OS version, architecture, sample dimensions/page counts, and actual results. When the candidate changes, rerun affected checks. Publishing a trial does not mark unexecuted checks as passed.

| Environment | Required checks |
| --- | --- |
| Native Windows 11 x64 | Install and run the package; use official ZDesigner v10 ZD421 ZPL at 203 and 300 dpi from a system print dialog. For both resolutions and both PDF/PNG outputs, check RAW task count, normal connection closure, `^PW`/`^LL`, copies, page dimensions, Chinese graphics, barcodes, compressed graphics, and printing with GPUI closed. |
| macOS 13, Intel and Apple Silicon | PKG install, first authorization, next-login LaunchAgent startup, upgrade/removal, native print dialogs, IME composition, and display scaling. The current Apple Silicon host is newer than macOS 13. |
| Debian 12 X11 and Ubuntu 24.04 Wayland desktops | DEB install/upgrade/removal, Vulkan drivers, folder portal, Chinese input, user-level systemd startup, and native application print dialogs. |
| All desktop targets | Print two pages of different content with two collated copies to both PDF and PNG; verify four-page order, dimensions, output, Chinese-path selection, and login startup. Exercise queue repair and printing after closing the UI. |
| Upgrade and fault recovery | Preserve settings, history, and output when upgrading from earlier releases, including 0.1.0. Check unwritable folders, changed permissions, full disks, port conflicts, interrupted reception, renderer crashes, cancellation, daemon restart, and power loss during reception. Verify complete queue/login-service cleanup after repair/removal. |

Output failure and retry handling are implemented, but unexecuted fault scenarios are not marked as passed. If the database disk is completely full, failure recording may also fail; free space and restart to apply the recovery rules.
