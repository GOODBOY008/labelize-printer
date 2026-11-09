# Development guide

[README](README.md) · [Architecture](ARCHITECTURE.md) · [Acceptance record](ACCEPTANCE.md)

Run the commands below from the repository root. Build instructions describe the current source tree; published installers may contain an earlier feature set.

The product is **Labelize Studio**. Existing crate, executable, environment-variable,
service, and data-directory names retain `labelize-printer` compatibility identifiers.
See [Migration](MIGRATION.md) for the boundary between branding and persistent identity.

## Prerequisites

- Rust 1.99 or later. CI pins Rust 1.99.1; use `--locked` to keep dependency versions reproducible.
- A native C/C++ toolchain and Python 3.11 or later.
- macOS: Xcode Command Line Tools and `pkg-config`. GPUI uses runtime Metal shader compilation, so an offline shader build is not required.
- Linux: CUPS, Vulkan, X11/Wayland, font, and other native development libraries listed in [scripts/linux.Dockerfile](scripts/linux.Dockerfile). The supplied build image uses Debian 12.
- Windows: the x64 MSVC toolchain. NSIS is required for packaging.

OpenSSL and PAPPL are built by the supplied scripts on macOS and Linux. Their versions and checksums are pinned there. Windows uses the RAW receiver and does not require the PAPPL adapter.

## Build and run

### macOS and Linux

```sh
cargo build --workspace --locked
bash scripts/build-openssl.sh
bash scripts/build-native.sh
target/debug/labelize-printer
```

The native build writes `labelize-printer-ipp` beside the desktop, daemon, and `labelize-printer-updater` binaries. Keep those binaries together when running or packaging the app.

### Windows

Run in a PowerShell environment with the MSVC toolchain available:

```powershell
cargo build --workspace --locked
.\target\debug\labelize-printer.exe
```

Debug builds accept `LABELIZE_PRINTER_WINDOW_SIZE=1000x720` (or `1440x900`, `480x360`) for reproducible layout checks. This override is not compiled into release builds. Use a dedicated `LABELIZE_PRINTER_HOME` when checking recovery or failure cases.

Opening the desktop starts the background service when needed. System-queue installation is a separate action in **Settings → System printer**. The service runs as the current user and continues after the desktop closes.

### Isolate development data

Use an absolute data path to keep development settings, job history, sockets, and inputs separate from an installed copy:

```sh
LABELIZE_PRINTER_HOME="$PWD/.cache/dev-state" target/debug/labelize-printer
```

```powershell
$env:LABELIZE_PRINTER_HOME = Join-Path $PWD '.cache\dev-state'
.\target\debug\labelize-printer.exe
```

The daemon also accepts `--data-dir /absolute/path`. Use the same data directory for the desktop, daemon, and CLI commands that should communicate. The setting does not isolate OS printer names or login-service registration; avoid installing a second system queue during ordinary development.

## Command-line use

To run the daemon without the desktop, leave this command running in one terminal:

```sh
target/debug/labelize-printer-daemon serve
```

Use a second terminal for control commands. On Windows, use the corresponding `.exe` path.

```sh
target/debug/labelize-printer-daemon submit /absolute/path/to/label.zpl --wait
target/debug/labelize-printer-daemon status
target/debug/labelize-printer-daemon test-print --wait
target/debug/labelize-printer-daemon job TASK_UUID
target/debug/labelize-printer-daemon retry TASK_UUID --wait
```

Since v0.2.0, the daemon accepts ZPL, EPL, TSPL/TSC, CPCL, DPL, and `.labelize.json` templates through `submit`. The desktop executable can also take a label file as its first argument.

| Command | Behavior |
| --- | --- |
| `settings` | Print settings as JSON. |
| `settings --file config.json` | Validate and save settings for new jobs. |
| `watch` | Stream job snapshots. |
| `refresh` | Detect installed drivers, queues, and platform state again. |
| `install` | Register or repair the local printer and configured login service. |
| `uninstall` | Remove the queue and the current user's login service. |
| `stop` | Stop the service when its queue is idle. |

Removing the queue or application does not delete saved output, job history, or retained failed inputs. Retrying a failed job reuses its original ID and settings snapshot.

## Checks

### Rust checks

```sh
cargo fmt --all --check
cargo test -p labelize-printer-updater --locked
cargo test -p labelize-printer-core --locked
cargo test -p labelize-printer --lib --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The desktop library tests cover draft recovery (including extended styles and incomplete property input), write failures, source conflicts, stale preview callbacks, cancellation/priority scheduling, and preview-cache reuse, revision isolation, cancellation, and byte-budget eviction without a GPU. These run in each package CI job. The core tests cover job persistence, output commits, raster metadata, file-language previews, and portable designs. Consult the [acceptance record](ACCEPTANCE.md) for what has actually been run on each platform.

`cargo test -p labelize-printer-core --test designer --locked` exercises all ten barcode formats through an independent decoder at 8/12 dots/mm and four rotations (80 combinations). It also checks style-dependent pixels, strict barcode validation, rotated geometry and locking, template serialization, PNG/PDF embedded-pixel equality, and a version-1 fixture captured from the previous implementation. Barcode decoding uses the object rectangle, including its quiet zone and optional caption. See [fixture provenance](crates/core/tests/fixtures/README.md); do not regenerate the compatibility image to conceal a rendering regression.

On a Mac with an available Metal device, the vendored atlas regression exercises repeated eviction and reuse while a neighboring image stays live. It is ignored by default because headless CI runners may not expose a GPU. Runtime shader compilation avoids requiring the optional offline Metal toolchain:

```sh
CARGO_TARGET_DIR=target/gpui-metal-test cargo test --manifest-path vendor/gpui/Cargo.toml \
  --features runtime_shaders --lib \
  evicted_image_can_be_reused_without_freeing_its_neighbor -- --ignored
```

### Background smoke tests

Build the binaries and, on macOS/Linux, the native adapter first:

```sh
python3 scripts/smoke.py
```

On Windows:

```powershell
python scripts/smoke.py --skip-ipp
```

Add `--profile release` to test release binaries. These tests start a real daemon in a temporary directory and exercise RAW TCP, job recovery, file submission, and IPP where available. They do not register a system queue or login service.

`--disk-full-dir /path/to/test-tmpfs` exercises real `ENOSPC` handling. It requires a dedicated, empty, small test filesystem that the test can deliberately fill. The ordinary smoke suite does not fill the host disk.

### Installed-package CUPS test

This test installs a DEB, registers a queue, submits a PDF through CUPS, checks the resulting two-page output, and removes the queue and package. The script only runs inside a container.

First build the package in Debian 12:

```sh
docker build --platform linux/amd64 -f scripts/linux.Dockerfile -t labelize-printer-build .
docker run --rm --platform linux/amd64 -v "$PWD:/build" labelize-printer-build bash -ec '
  cargo build --workspace --release --locked
  bash scripts/build-openssl.sh
  bash scripts/build-native.sh release
  python3 scripts/collect-licenses.py
  bash scripts/package-linux.sh
'
```

Then test the installed package in a separate container:

```sh
docker build --platform linux/amd64 -f scripts/linux-system.Dockerfile -t labelize-printer-system-test .
docker run --rm --platform linux/amd64 -v "$PWD:/build" labelize-printer-system-test bash scripts/linux-system-smoke.sh
```

Use a dedicated checkout for container builds: they write build artifacts into the mounted repository. Container results do not replace desktop, input-method, GPU, or login-service acceptance.

## Package installers

Installer versions come from the workspace version in `Cargo.toml`. Output goes to `dist/`. License collection must run before packaging.

### macOS

```sh
cargo build --workspace --release --locked
bash scripts/build-openssl.sh
bash scripts/build-native.sh release
python3 scripts/collect-licenses.py
bash scripts/package-macos.sh
```

This produces a PKG for the build host's architecture. The script signs the app bundle ad hoc, checks its icon resource, and rejects dependencies on build-machine library paths. The PKG has no public distribution signature or notarization.

### Linux

Build on Debian 12 x64, either natively or using the container recipe above:

```sh
cargo build --workspace --release --locked
bash scripts/build-openssl.sh
bash scripts/build-native.sh release
python3 scripts/collect-licenses.py
bash scripts/package-linux.sh
```

### Windows

With MSVC and `makensis` available in PowerShell:

```powershell
cargo build --workspace --release --locked
python scripts/collect-licenses.py
New-Item -ItemType Directory -Force dist | Out-Null
$version = python scripts/package-version.py
makensis "/DAPP_VERSION=$version" platform\windows\installer.nsi
```

The repository configures a static MSVC C runtime. CI checks both executables for unintended dynamic Visual C++ runtime dependencies before packaging. A GNU cross-build for `x86_64-pc-windows-gnu` instead requires MinGW `gcc` and `windres`; `windres` embeds the icon and application manifest. Cross-compilation alone is not Windows runtime validation.

### Icons and bundled patches

After changing `platform/icon.svg`, run `bash scripts/build-icons.sh` on macOS with ImageMagick installed to regenerate desktop, Windows, and macOS assets.

Keep the provenance of bundled sources intact:

- [Labelize snapshot](vendor/labelize/PROVENANCE.md): engine revision and upstream parser checks.
- [GPUI patches](vendor/gpui/PATCH.md): X11 visual selection, Windows debug shaders, and Metal image-atlas eviction.
- [xattr patch](vendor/xattr/PATCH.md): Linux/Android compatibility.
- [Third-party notices](THIRD_PARTY_NOTICES.md): licenses and packaged notices.

Parser fixes belong upstream in Labelize and require its rendering regression suite before refreshing the vendored snapshot. Platform integration and job management belong in this repository.

## Upgrades and diagnostics

Before replacing an installed build, let jobs finish, close the desktop, and run the installed daemon's `stop` command. Keep desktop, daemon, and native adapter versions matched. Upgrades preserve settings, input snapshots, and output. Before manually removing an application bundle, remove its queue and login service through **Settings → System printer** or the daemon's `uninstall` command.

Set `LABELIZE_PRINTER_DEBUG=1` and launch the desktop from a terminal to send startup diagnostics to stderr. For job failures, inspect **Jobs** or `job TASK_UUID`; inputs and diagnostics are retained for retry. Include the OS, architecture, source commit or installer version, and input route in an issue.

The [build workflow](.github/workflows/build.yml) produces Windows x64, macOS ARM64/x86_64, and Debian 12 amd64 installers. Its build and protocol checks are separate from the [remaining native acceptance work](ACCEPTANCE.md#remaining-acceptance-work).

## Update checks and installer handoff

See [UPDATES.md](UPDATES.md) for the protocol and recovery contract. The updater is a separate crate and binary without GPUI. The CI package matrix includes it in all four installers and runs its tests plus the real-service handoff smoke test:

```sh
python3 scripts/update-smoke.py --profile debug
cargo test -p labelize-printer-updater live_release_integrity -- --ignored --nocapture
```

The smoke test uses an isolated data directory and a manual handoff; it never executes Homebrew or installs a package. It checks admission across RAW/IPP/local imports/retry, cancellation, desktop-exit ordering, durable restart protection, and history/output preservation. The ignored live test contacts the official public release, downloads the host installer into a temporary directory, verifies both published hashes, and exercises cache reuse; it does not execute it.

For Linux builds using a separate target directory, add `--target-dir target/linux-workbench` to `update-smoke.py`. Keep native IPP and all Rust binaries in the same profile directory. Do not treat package builds or protocol smoke tests as native installer or ZDesigner acceptance.
