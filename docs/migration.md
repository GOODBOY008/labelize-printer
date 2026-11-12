# Moving to Labelize Studio

Labelize Studio v0.4.0 is the next version of Labelize Printer. It combines the
label workspace, visual designer, PDF/PNG export, and local virtual printer
under one application name. The system print dialog still lists **Labelize Printer**.

## Upgrade

Wait for accepted print jobs to finish and quit the desktop so its workspace is
saved. Versions through v0.3.0 have no in-app updater; use an installer or Homebrew
for this first upgrade. Keep using the same installation method.

### macOS

After quitting the old app, stop its background service:

```sh
"/Applications/Labelize Printer.app/Contents/MacOS/labelize-printer-daemon" stop
```

If this reports active jobs, let them finish and retry. Do not remove the system
queue or delete application data. Then install the matching Studio PKG, or, for
an existing Homebrew installation, run:

```sh
brew tap goodboy008/labelize-studio
brew update
brew upgrade --cask goodboy008/labelize-studio/labelize-studio
```

The tap supplies `cask_renames.json` so Homebrew can migrate the installed
`labelize-printer` cask to `labelize-studio`. Existing clones of the old tap can
continue fetching through GitHub's repository redirect.

Open **/Applications/Labelize Studio.app** once after installation. The PKG installs
at the new path, then removes the old default-location bundle only if it belongs
to this application. The first Studio launch updates an existing LaunchAgent
that points to the old default app and the same user's data directory. This
does not enable autostart for users who had it disabled. Open Studio before the
next login to refresh that path. Re-pin the new app if an old Dock shortcut no
longer resolves.

Copies installed outside `/Applications` are left alone. Quit those copies and
open the new canonical application; remove an extra copy only after checking
that your data and queue are available. Custom login scripts must use the new
application path.

### Windows

Quit the desktop and run the Studio EXE using the existing installation directory.
The installer stops the idle background service, replaces the programs, updates
the existing uninstall entry, and replaces the old Start menu shortcut. The
default folder remains `%LOCALAPPDATA%\Programs\Labelize Printer` for compatibility.
Open Labelize Studio afterward; the existing ZDesigner queue is retained.

### Debian / Ubuntu

Quit the desktop and stop the idle service before installing:

```sh
/opt/labelize-printer/labelize-printer-daemon stop
sudo apt install ./labelize-studio_0.4.0_amd64.deb
```

The DEB still uses the package ID `labelize-printer`. Apt upgrades the existing
package instead of installing a second copy. The application launcher displays
**Labelize Studio**. Existing systemd and CUPS registrations keep their paths.

## What stays compatible

| Item | Retained behavior |
| --- | --- |
| User data | Same `LabelizePrinter` directory, SQLite database, settings, history, drafts, and session files; no data move or conversion. |
| Output | Existing output folder and saved PDF/PNG files remain in place. |
| System printer | Same queue identifier `LabelizePrinter` and display name **Labelize Printer**. |
| Templates | Existing `.labelize.json` files, printer-language files, and embedded assets remain readable. |
| Integration | Existing `labelize-printer*` executable names, `LABELIZE_PRINTER_HOME`, IPC endpoints, and service identifiers stay valid. |
| OS identity | `com.labelize.printer` bundle/receipt, Windows `LabelizePrinter` uninstall key, and Debian `labelize-printer` package ID are retained. |
| Downloads | Old release tags and assets keep their original names. GitHub redirects the former repository URLs. |

The public repositories are now [labelize-studio](https://github.com/GOODBOY008/labelize-studio)
and [homebrew-labelize-studio](https://github.com/GOODBOY008/homebrew-labelize-studio).
Update a development checkout with
`git remote set-url origin https://github.com/GOODBOY008/labelize-studio.git`.
Local checkout folder names do not affect the product.

## Release filenames and updates

From v0.4.0, releases contain these four installers and `SHA256SUMS`:

- `LabelizeStudio-<version>-macos-arm64-release.pkg`
- `LabelizeStudio-<version>-macos-x86_64-release.pkg`
- `LabelizeStudio-<version>-windows-x64.exe`
- `labelize-studio_<version>_amd64.deb`

The updater uses the Studio repository and recognizes installed Homebrew records
under either cask name. Historical releases before v0.4.0 retain the Printer
asset naming scheme. New releases require Studio asset names and verified
checksums. Installers remain unsigned; see [Software updates](updates.md) and
[Acceptance](acceptance.md) for verification boundaries.
