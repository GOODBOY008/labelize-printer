# Labelize Printer

<img src="platform/icon.svg" width="72" height="72" align="right" alt="Labelize Printer icon">

**Save labels and system print jobs as PDF or PNG.**

[![Build and package](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml)

[Download](https://github.com/GOODBOY008/labelize-printer/releases) · [Designer guide](DESIGNER.md) · [Development guide](DEVELOPMENT.md) · [Architecture](ARCHITECTURE.md) · [Report an issue](https://github.com/GOODBOY008/labelize-printer/issues)

Labelize Printer is a desktop label workspace and local virtual printer for Windows, macOS, and Linux, built with Rust and GPUI. Select **Labelize Printer** in your system print dialog to save a job as one multipage PDF or one PNG per page. A background service keeps processing jobs after you close the window. Label processing runs locally, without a cloud service. Optional software-update checks contact GitHub.

It uses the [Labelize rendering engine](https://github.com/GOODBOY008/labelize) to parse and render printer-language files. The desktop app adds printer setup, output settings, job history, and a visual label designer. Labelize itself is also available as a library, CLI, HTTP service, WebAssembly package, and Android library.

> **New in v0.3.0:** A blue-and-white, multi-tab workspace with automatic previews, local draft recovery, per-tab export options, and a reorganized designer. Open files and designs side by side, resume your workspace after a restart, and export through the existing background service. See [Acceptance](ACCEPTANCE.md) for verified environments and remaining checks.

## What you can do

| Workflow | Capabilities | Availability |
| --- | --- | --- |
| System printing | Select a local virtual printer and save PDF or PNG while the desktop window is closed. | v0.1.2 |
| Job management | Choose an output folder, inspect results, open saved files, and retry failed jobs. | v0.1.2 |
| Label files | Open ZPL, EPL, TSPL/TSC, CPCL, or DPL; edit supported text files and preview each page. | v0.2.0 |
| Visual design | Add text, Code 128, QR codes, rectangles, and images; move objects and edit their dimensions. | v0.2.0 |
| Extended designer | Ten barcode formats, lines and ellipses, text layout, rotation, image fitting, fills, and layer controls. | Current source (unreleased) |
| Multi-tab workspace | Independent edits, automatic previews, recent files, and local session/draft recovery. | v0.3.0 |
| Software updates | Stable-release checks, verified full installers, and package-manager-aware handoff. | Current source (unreleased) |
| Portable templates | Save editable `.labelize.json` templates with embedded images and imported fonts. | v0.2.0 |

## Install

Download an installer and `SHA256SUMS` from [GitHub Releases](https://github.com/GOODBOY008/labelize-printer/releases). The release notes identify the source commit, tested environments, and known limitations.

| Platform target | Package | Validation status |
| --- | --- | --- |
| Windows 11 x64 | NSIS `.exe` | Installation, upgrades, desktop use, and direct RAW ZPL tested in a Windows 11 ARM64 VM using x64 emulation. Native x64 and ZDesigner system printing remain pending. |
| macOS 13+ Apple Silicon | ARM64 `.pkg` | Native system printing tested on a newer macOS host. macOS 13 compatibility remains pending. |
| macOS 13+ Intel | x86_64 `.pkg` | Background processing tested through Rosetta. Intel desktop and macOS 13 acceptance remain pending. |
| Debian 12 / Ubuntu 24.04 x64 | amd64 `.deb` | Installed-package CUPS printing tested in Debian 12 containers. Desktop and Ubuntu acceptance remain pending. |

Windows and macOS installers are unsigned. macOS app bundles use ad-hoc signatures; Developer ID signing and notarization are pending.

On macOS, Homebrew selects the package for your architecture:

```sh
brew install --cask goodboy008/labelize-printer/labelize-printer
```

See the [Homebrew tap](https://github.com/GOODBOY008/homebrew-labelize-printer) for upgrade instructions. For Debian or Ubuntu, use `sudo apt install ./<downloaded-package>.deb` to install the downloaded package with its dependencies.

### Software updates

The current source tree adds **Settings → Software update**. It checks stable GitHub releases at startup at most once every 24 hours; you can turn this off or check manually. Downloading and installing always require your action. The first release containing this feature must be installed through an installer or Homebrew once; v0.3.0 does not include it.

Direct installations download the matching full installer and verify its size and SHA256 against both `SHA256SUMS` and GitHub's asset digest. Homebrew installations keep using Homebrew. On Linux, a missing DEB handler falls back to the downloaded file and a safely quoted `apt` command.

Before installation, the app retains your workspace drafts, waits for file operations, and requires the print queue to be idle. **Quit and install** pauses printing and hands off to the system installer. Finish or cancel installation, then reopen the app to verify the installed components and restore your workspace. Trial packages remain unsigned. See [Software updates](UPDATES.md) for recovery and release requirements.

### First print

1. Open **Labelize Printer** and choose an output folder and format in **Settings → Output defaults**.
2. In **Settings → System printer**, configure the resolution and add the printer. Use the test-output action to confirm that files can be saved.
3. Select **Labelize Printer** in another application's system print dialog and submit a job.
4. Open **Jobs** to check its status and open the result. **Saved** means the output files have been committed successfully.

Defaults are PDF, the `Labelize` folder under Documents, a 102 × 152 mm label, and 8 dots/mm. Each job gets its own timestamp-and-ID directory containing a single PDF or numbered PNG files. Use the language button above Settings in the sidebar to switch between English and Simplified Chinese; the preference persists across restarts.

**Windows setup:** Install the official [ZDesigner driver](https://www.zebra.com/us/en/support-downloads/printers/printer-drivers.html) separately, then refresh detection and select the ZDesigner v10 **ZD421 ZPL 203 dpi** or **300 dpi** driver in **System printer**. Match the queue resolution to 8 or 12 dots/mm, respectively. In ZDesigner Driver Settings, set **Status checking → Status check type** to **Disabled**. The app does not emulate hardware status responses. End-to-end ZDesigner acceptance is still outstanding.

**macOS and Linux setup:** The app registers a local IPP queue using its bundled PAPPL adapter. Linux requires CUPS and a desktop PolicyKit agent; the Wayland file picker also needs a working XDG desktop portal. Queue setup requests elevation only when needed. Everyday processing runs as the current user.

## Open and design labels

Install v0.3.0 or later to use the multi-tab **Workspace** described here. Installers are available from [GitHub Releases](https://github.com/GOODBOY008/labelize-printer/releases); you can also [build from source](DEVELOPMENT.md#build-and-run).

### Open a label file

In **Workspace**, choose **Open file**, drop one or more files into the workspace, or pass a file path to the desktop executable. Supported extensions are `.zpl`, `.epl`, `.tspl`, `.tsc`, `.cpcl`, `.dpl`, and `.prn`. Known extensions select the language; ambiguous files use content detection. You can override the selection with the language buttons.

Each file opens in its own tab; opening the same path again selects the existing tab. Switch between **Preview** and **Source**, browse page thumbnails, and zoom or fit the label to the canvas. UTF-8 edits automatically refresh the preview after 500 ms of inactivity. A failed preview keeps the last successful image with a visible warning; it cannot replace a newer result. Files containing binary data or control bytes, and files larger than 1 MiB, remain read-only so their original bytes are preserved.

Choose PDF or PNG and an output folder in the right-hand inspector, then use the single **Export PDF/PNG** button. PDF combines all pages; PNG writes one image per page. These choices belong to the current tab and do not change system-queue defaults. Export captures the content and options at the time of submission. Once accepted, the job continues in the background while you edit or close the tab; **Jobs** reports success only after the files are committed.

### Create a design

Choose **New design** to start a template. Select an object on the canvas or in the object list, drag it to move, and edit its position and size in millimetres. Property edits update the preview automatically. Duplicate or delete the selected object, and use **Undo** and **Redo** to restore design changes. Each tab keeps its own design history.

The **unreleased source** expands the designer to 15 object types: text, rectangles, ellipses, lines, images, and ten barcode formats. The inspector offers rotation, text wrapping and alignment, fills and rounded corners, image fitting, barcode parameters, and layer naming, visibility, locking, and order. A bottom-right handle resizes the selected object; arrow keys move it precisely when the canvas has focus. See the [designer guide](DESIGNER.md) for the full property list and a sample template. Released v0.3.0 has the original text, Code 128, QR, rectangle, and image tools.

**Save** writes back to the source file, asking for a path only for an untitled document. **Save as** is in the overflow menu. A design is saved as a portable `.labelize.json` template containing its objects, images, and imported font. Reopen the file to continue editing. For Chinese or other text outside the default font's coverage, import a compatible TrueType/OpenType font of up to 32 MiB. Missing glyphs, overflowing text, and undersized barcodes produce errors. Images are composited over white and rendered in monochrome.

Existing printer-language files remain source documents; they are not reconstructed as editable design objects. Designs currently export to PDF/PNG, with no direct physical-printer submission or printer-language code export. Supported commands and copy semantics follow the embedded Labelize engine; the integration expands ZPL `^PQ` copies once and does not add copy expansion for other languages.

| Shortcut | Action |
| --- | --- |
| Cmd/Ctrl+O | Open a label file or template |
| Cmd/Ctrl+N | Create a design |
| Cmd/Ctrl+S / Cmd/Ctrl+Shift+S | Save / Save as |
| Cmd/Ctrl+E | Export the active document |
| Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z | Undo / redo design changes outside text inputs (unreleased) |
| Arrow / Shift+Arrow | Move the selected object 0.1 / 1 mm with canvas focus (unreleased) |
| Delete / Backspace | Delete the selected unlocked object with canvas focus (unreleased) |
| Cmd/Ctrl+W | Close the active tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous document |
| Cmd/Ctrl+1 / 2 / 3 | Workspace / Recent files / Jobs |
| Cmd/Ctrl+, | Settings |

Closing an edited tab offers **Save / Discard / Cancel**. Closing the window saves the workspace and leaves accepted output jobs running. On reopening, tabs, raw drafts (including incomplete fields), page selection, zoom, and export options are restored. Drafts autosave locally after one second; they do not overwrite source files or clear the unsaved indicator. If the source changes externally, saving offers a conflict prompt. Missing source files can be located again without losing the draft.

The interface uses a compact sidebar below 1000 logical pixels and an inspector drawer with an overflow toolbar below 700. **Jobs** filters the latest 100 records; **Recent files** keeps 20 paths. Preview runs in one cancellable child process at a time and is bounded to 128 Mi pixels; exports retain the [job resource limits](ARCHITECTURE.md#resource-limits-and-retention).

## How it works

| Input | Processing | Output |
| --- | --- | --- |
| Windows system print job | ZDesigner generates ZPL → local RAW receiver → Labelize | PDF or PNG |
| macOS/Linux system print job | System generates PWG/Apple Raster → PAPPL → raster adapter | PDF or PNG |
| Label file | Labelize language parser → renderer | Preview, PDF, or PNG |
| Design template | Design objects → language-neutral representation → Labelize renderer | Preview, PDF, or PNG |

The background service snapshots each job's settings, persists the input and status in private local storage, and commits output files before reporting success. System raster pages retain their dimensions and order without being converted to ZPL. LPrint is not a dependency.

See [Architecture](ARCHITECTURE.md) for component responsibilities, task boundaries, recovery, and local communication.

## Development and support

- [Development guide](DEVELOPMENT.md): build, run, CLI commands, tests, packaging, and troubleshooting.
- [Acceptance record](ACCEPTANCE.md): automated checks, native desktop evidence, and outstanding platform tests.
- [GitHub Issues](https://github.com/GOODBOY008/labelize-printer/issues): include your OS and architecture, version or commit, input route, expected result, and diagnostic message. Attach a minimal label sample with private data removed when possible.

The current scope is one user, one computer, and one local queue. Jobs must be self-contained; printer resources are not shared between jobs. Network sharing, multiple-user queues, and vector PDF output are outside the current scope.

Released under the [MIT license](LICENSE). See [Third-party notices](THIRD_PARTY_NOTICES.md) for bundled dependencies and assets.
