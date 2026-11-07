<img src="platform/icon.svg" width="72" height="72" alt="Labelize Printer icon">

# Labelize Printer

[![Build and package](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml)
[简体中文](README.zh-CN.md)

Labelize Printer is a local virtual printer built with Rust and GPUI. Select it in the system print dialog or submit an existing ZPL file. Each job saves as one multipage PDF or one PNG per page.

The desktop app sets up the print queue, manages output settings, and shows recent jobs. A per-user background process keeps accepting jobs after you close the window.

## Labelize rendering engine

Labelize is an open-source Rust engine that renders ZPL, EPL, TSPL/TSC, CPCL, and DPL to PNG and PDF. Labelize Printer embeds its Rust library for ZPL jobs. Labelize is also available as a CLI, HTTP service, WebAssembly package, and Android library. [Labelize source and documentation](https://github.com/GOODBOY008/labelize)

## Try the preview

Download your platform's unsigned installer and `SHA256SUMS` from GitHub Releases. Each preview lists the environments it was tested on. [Preview downloads](https://github.com/GOODBOY008/labelize-printer/releases)

On macOS, Homebrew selects the PKG for Apple Silicon or Intel.

```sh
brew install --cask goodboy008/labelize-printer/labelize-printer
```

The tap follows published Printer releases, including previews. [Homebrew tap and upgrade instructions](https://github.com/GOODBOY008/homebrew-labelize-printer)

| Platform | Installer | Validation |
| --- | --- | --- |
| Windows x64 | NSIS EXE | Upgrade and direct ZPL verified on Windows 11 ARM64 with x64 emulation. ZDesigner system printing is pending. |
| macOS Apple Silicon | ARM64 PKG | Native system printing verified. |
| macOS Intel | x86_64 PKG | Package payload and CUPS printing tested with Rosetta. Intel desktop testing is pending. |
| Debian 12 / Ubuntu 24.04 x64 | amd64 DEB | CUPS printing verified in containers on both distributions. Desktop testing is pending. |

See [ACCEPTANCE.md](ACCEPTANCE.md) for the test environments and outstanding checks.

1. Install with Homebrew on macOS or use the downloaded package. On Linux, pass the downloaded `.deb` path to `sudo apt install`.
2. Open Labelize Printer, choose an output directory and format, and add the printer from the setup page.
3. Select **Labelize Printer** in the system print dialog. Use the setup page's test output to verify file saving and open the result in Recent Jobs.

Windows system printing requires the official ZDesigner v10 ZD421 ZPL driver at 203 or 300 dpi. Install it separately and select it in the setup page. [Zebra driver downloads](https://www.zebra.com/us/en/support-downloads/printers/printer-drivers.html)

On Linux, the setup page uses CUPS and the desktop PolicyKit agent to register the queue. The Wayland directory picker uses an XDG desktop portal.

Development builds are available through GitHub Actions. [Build and package](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml)

## How printing works

- **ZPL** from ZDesigner or a local submission is parsed and rendered by Labelize. The renderer handles label sizes and copy counts in the job.
- **macOS and Linux system printing** delivers PWG or Apple Raster pages over local IPP. PAPPL receives them and the output pipeline preserves their physical size and page order.
- **Files** are committed before the app marks a job as saved. The default is PDF in the `Labelize` folder under Documents.

The [architecture guide](ARCHITECTURE.md) describes the job lifecycle, storage, recovery, and platform integration.

## Build from source

Use Rust 1.99 or later and a native C toolchain. On macOS, install Xcode Command Line Tools and `pkg-config`. Linux build dependencies are listed in [scripts/linux.Dockerfile](scripts/linux.Dockerfile). Windows uses the MSVC toolchain.

```sh
cargo build --workspace --locked
cargo fmt --all --check
cargo test -p labelize-printer-core --locked
```

Build the native adapter on macOS and Linux, then run `target/debug/labelize-printer`. On Windows, run `target\debug\labelize-printer.exe` after the Cargo build.

```sh
bash scripts/build-openssl.sh
bash scripts/build-native.sh
```

The daemon also accepts self-contained ZPL jobs from the command line while the background process is running.

```sh
target/debug/labelize-printer-daemon submit /absolute/path/to/label.zpl --wait
```

See the [Chinese guide](README.zh-CN.md) for packaging and additional smoke-test commands.

## Issues and license

For a bug report, include your platform, package or commit version, and whether the job came through a system queue or direct ZPL. [GitHub Issues](https://github.com/GOODBOY008/labelize-printer/issues)

Released under the MIT license. [License](LICENSE) · [Third-party notices](THIRD_PARTY_NOTICES.md)
