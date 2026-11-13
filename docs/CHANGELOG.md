# Changelog

Versions follow `version` in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`
(keep the three in sync). Feature details: [feature.md](feature.md).

## Unreleased

### Added
- **Uploading** tab with a badge counting items waiting or being uploaded. Per library: phase (scanning local
  changes / uploading), progress, speed and the files of the commit being uploaded (up to 500 listed).
  seaf-daemon reports progress and speed per library, not per file.
- **Sync history**: right-click an entry to show the file or folder in Explorer (opens the closest folder if
  the item was deleted), open the library folder, or copy the path. 20 entries per page.
- Completed syncs in the history now record the changed file, or the folder containing all changes.

## 0.2.0 – 2026-11-12

### Added
- **Library context menu** (right-click): Sync now, Open local folder, View on cloud, Last error…, Unsync.
- **Last error dialog**: latest error with file path and time, plus earlier errors recorded by seaf-daemon.
- **Status bar**: sync engine state, synced / syncing / error counts, current upload and download speed.
- **Tray**: status badge on the icon (green = up to date, blue = syncing, red = error); tooltip shows version,
  status, network and transfer speed.
- **Sync history** tab: completed syncs and errors, relative time ("5 minutes ago") with exact date/time on hover
  or click; retention by days and record count; Clear.
- **Settings window** (separate window) with tabs *General* and *Data location*.
- **Start automatically with Windows** and **Start minimized to the system tray**.
- **Ignore temporary files** option: managed rules in each library's `seafile-ignore.txt`
  (Office, LibreOffice, WPS, draw.io, AutoCAD, partial downloads, editor swap files, system files).
- App version in the main window header.

### Changed
- Closing the main window now destroys it instead of hiding it, and a minimized start never creates it, so
  WebView2 is not kept in memory while the app runs in the tray. The window is recreated when opened again.
- Synced libraries also show their size.
- New app icon (cloud with up/down arrows).
- Pointer cursor on all enabled buttons and form controls.
- Main window starts hidden and is shown during startup (no flash on a minimized start).

### Fixed
- Changed icons were not embedded in the exe because the build script did not rerun; `build.rs` now watches `icons/`.
- seaf-daemon's notification queue was never read and grew while transferring; it is now drained.

## 0.1.0

Initial version: login with auto-login, library list and sync setup, per-library pause/resume and
Bypass Metered, automatic pause/resume on metered networks, force sync, tray menu, Explorer overlay icons,
movable data location.
