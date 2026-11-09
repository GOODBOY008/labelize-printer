# Software updates

[README](README.md) · [Architecture](ARCHITECTURE.md) · [Development](DEVELOPMENT.md)

This describes the current source tree. The first updater-enabled release needs a one-time manual installer or Homebrew upgrade; the published v0.3.0 app has no update UI. After that first manual upgrade, sign out and back in once to ensure an older per-user background process is replaced along with the desktop.

## User flow

Settings → Software update shows the application version, last check time, release notes, download progress, and recovery actions. Startup checks are asynchronous, stable-only, and limited to one attempt per 24 hours. The setting defaults to enabled. Manual checks bypass that interval. Errors, rate limits, and offline operation do not block the workspace or daemon polling. No labels, settings, or job contents are uploaded to GitHub.

Downloads and installation require separate user actions. Downloads use the fixed `GOODBOY008/labelize-printer` repository over HTTPS and select a strictly newer semantic version. The updater rejects drafts, prereleases, missing/duplicate assets, invalid asset URLs, missing SHA256 digests, oversized metadata, truncated downloads, and hash mismatches. It verifies the checksum manifest's own GitHub digest, then requires the installer's GitHub digest and manifest entry to agree. These are integrity checks, not code signing; current installers remain unsigned.

Partial files use random names under the user's private `updates/` directory. A per-user download lock prevents simultaneous writers. Cancellation removes the partial file; startup removes orphaned UUID `.part` files only when no download owns the lock. Previously verified installers are hashed again before reuse and again by the installation helper.

## Platform routes

| Source | Handoff |
| --- | --- |
| Windows x64 | Launch the verified NSIS EXE after the desktop, previews, daemon and adapter have stopped. Preserve the current installation directory. Reopen after closing the installer. |
| macOS PKG | Query hardware support, choosing ARM64 on Apple Silicon even under Rosetta. Open a new system Installer instance and wait for it to close. |
| macOS Homebrew | Read installed cask inventory via `brew info --json=v2 --installed`, with automatic Homebrew updates disabled during discovery. Copy the specific tap's upgrade command, then quit before the user executes it. Never run Homebrew automatically. |
| Debian / Ubuntu | Open the verified DEB with the registered desktop handler. If no handler exists, show the containing directory and copy a shell-quoted `sudo apt install` command. |

Failed Homebrew detection asks the user to choose the installation source. The persisted choice is retained across language changes. A system installer opening successfully does not mean an update has completed. In particular, Linux desktop file handlers can return before their package transaction finishes.

## Admission and process boundaries

`crates/updater` contains discovery, verification, platform integration and the headless `labelize-printer-updater` helper. The desktop controller has its own worker threads and cancellation state; it does not occupy the print-status worker. Preferences are backward compatible with the original language-only `desktop.json`.

Authenticated IPC adds `prepare_update`, `cancel_update`, and `commit_update`, keyed by a request UUID. Preparation locks all admission paths and checks both persisted active jobs and RAW sockets reserved before database insertion. A pending TCP backlog connection is spooled and makes preparation fail as busy. Successful preparation closes the RAW listener and rejects new IPP/local/retry requests. Existing tasks are never killed for an update. Preparation expires after 120 seconds if not committed; cancellation resumes reception on the same port.

The desktop first flushes drafts and handles unsaved settings, then pauses and reaps previews. It copies the helper into a private handoff directory outside the installation tree. The helper acquires an update lock, prepares the daemon and persists `update-pending.json`. Only after readiness does the desktop flush the latest session again and authorize exit. Without that authorization, on timeout, or if preparation fails, the helper cancels the admission lease and removes its pending marker.

After authorization, the helper waits for the desktop process to exit, commits daemon shutdown and waits for the daemon lock to be released. The daemon reaps IPP before acknowledging shutdown. Login supervisors exit successfully without starting a service while a pending marker exists. A second desktop opened while the helper owns its lock shows a notice from the cached helper and exits instead of holding installed executable files open.

Normal RAW EOF semantics and IPP job completion semantics are unchanged. Printing is unavailable during the installation handoff; an application attempting to print then must retry through its system queue. Files already committed remain saved.

## Completion and recovery

After completing or cancelling the installer, reopen Labelize Printer. A matching target version is verified automatically; cancellation requires the explicit **Installation cancelled; resume current version** action. Verification checks the daemon, updater, Unix IPP component, and system package receipt against the running desktop version. Linux also checks dpkg's POSIX record locks before resuming. A mixed or partially configured installation remains blocked and instructs the user to rerun the full installer.

The handoff record is removed only after verification, then the existing service and workspace recovery paths resume. Settings, queue registration, job history and local drafts are retained. Handoff logs and requests are stored under `updates/<request UUID>/` for diagnosis. A damaged pending record is retained for diagnosis rather than silently bypassed.

## Release contract

Keep the four installer names stable: Windows x64 EXE, macOS ARM64 and x86_64 PKG, Linux amd64 DEB. Include the updater binary and matching `--version` output from every component. Upload all installers and `SHA256SUMS` before marking a stable release latest. Every asset, including `SHA256SUMS`, must have a GitHub `sha256:` digest. Update the Homebrew tap with the same PKG hashes.

The updater does not require a custom release server, delta packages, GitHub login, a background GUI, or an automatic rollback mechanism. Native installer success/cancellation, system authorization, and actual upgrades need platform acceptance in addition to CI builds and isolated handoff tests.
