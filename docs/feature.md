# FEATURE SPECIFICATION: Seafile Smart Metered Client (Windows)

## Overview
Lightweight custom Seafile desktop client for Windows built with Tauri (Rust + React). Main purpose is to automatically pause/resume file synchronization per library based on Windows Network Metered status (Mobile Hotspot) while keeping idle CPU and RAM usage minimal.

---

## 1. Core Features

### 1.1 Account & Server Management
- **Login Screen:** Input Server URL, Email, Password.
- **Authentication:** Authenticate via Seafile REST API (`POST /api2/auth-token/`) and retrieve Auth Token.
- **Credential Storage:** Securely store Auth Token and Server URL in encrypted local state.
- **Auto-login:** Automatic session restoration on app startup.

### 1.2 Library Management & Synchronization
- **Fetch Libraries:** Display all cloud libraries fetched via `GET /api2/repos/`.
- **Local Mapping:** Allow users to bind any cloud library to a specific local folder path.
- **Manual Sync Control:** Toggle Sync/Pause per library manually from the UI.
- **Status Badges:** Display current library state (`Synced`, `Syncing`, `Paused (Metered)`, `Paused (Manual)`, `Error`).

### 1.3 Smart Network Awareness (Metered Connection Handling)
- **Automatic OS Detection:** Event-driven detection of Windows Network Connection Cost (`INetworkListManager`).
- **Per-Library Metered Bypass:** Option to check/uncheck `Bypass Metered` for individual libraries.
  - If `Bypass Metered == true`: Library syncs normally regardless of network type (ideal for lightweight Word/Text document folders).
  - If `Bypass Metered == false`: Library automatically pauses upon entering a Metered Network (Hotspot) and resumes when switching back to Unmetered Network (Wi-Fi/Ethernet).
- **Force Sync Action:** Allow users to force-sync a paused library on a metered network for one-time manual sync.

### 1.4 Background Performance & System Tray
- **Background Daemon:** Runs silently in the Windows System Tray.
- **Tray Menu Options:** Open UI, Pause All, Resume All, Network Status Indicator, Exit.
- **Tray Status Badge:** A coloured dot on the tray icon shows the overall state:
  green = up to date (paused libraries count as normal), blue = syncing (data moving, initial download or sync task running),
  red = error (seaf-daemon failed to start or any library is in error). Error wins over syncing.
- **Tray Tooltip:** App name + version, sync status, network cost, current upload/download speed.
- **Zero-Idle CPU:** Event-driven architecture ensures 0% CPU consumption when no active sync or network event is occurring.
- **Sidecar Process Control:** Automatic lifecycle management (Spawn/Kill) of the underlying `seaf-daemon.exe`.

### 1.5 Library Context Menu
Right-click a library row in **Libraries**:

| Action | Synced library | Not synced |
| :--- | :--- | :--- |
| Sync now / Sync now (one-time) | Runs a sync immediately; on a paused library it is a one-off sync and the library stays paused | – |
| Sync… | – | Opens the folder setup form |
| Open local folder | Opens the folder in Explorer | disabled |
| View on cloud | Opens `<server>/library/<id>/<name>/` in the browser | same |
| Last error… | Dialog with the latest error (message, file path, time) and up to 20 earlier errors recorded by seaf-daemon | disabled |
| Unsync | Stops syncing after confirmation; local files are kept | disabled |

The row also shows the library size for synced libraries (`size · local path`).

### 1.6 Transfer Speed & Status Bar
- Status bar at the bottom of the main window: sync engine state, number of synced libraries,
  how many are syncing / in error, current `↑ upload` and `↓ download` speed.
- Rates come from seaf-daemon and are refreshed every 1 s while data moves, every 3 s when idle.

### 1.7 Sync History
- **Sync history** tab next to **Libraries**: completed syncs (with the commit description,
  e.g. "Added a.txt and 2 more files") and sync errors (message + file path).
- Time column shows relative time ("5 minutes ago", "yesterday"); hover shows the exact date/time,
  clicking switches the whole list between relative and exact time.
- Retention, configurable in the tab: **Keep last** 1–365 days (default 7) and **Max records** 10–5000 (default 200).
  **Clear** deletes all entries.
- Stored in the config DB, so it moves with the data folder.

### 1.8 Settings Window
Separate window (owned by the main window, no own taskbar button; Esc or ✕ closes it), with tabs:

- **General**
  - *Start automatically with Windows*: registers the app in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
    (no admin rights). The entry is refreshed on every start, so it follows a moved portable folder.
  - *Start minimized to the system tray* (default on, needs autostart): when launched by Windows at sign-in
    the app stays in the tray. Manual launches always show the window.
  - *Ignore temporary files* (default off): see 1.9.
- **Data location**: move the config DB and sync engine data (portable / AppData / custom folder).
  The window cannot be closed while a move is running.

### 1.9 Ignore Temporary Files
When enabled, the app writes a managed block of patterns into `seafile-ignore.txt` at the root of every
synced library. Lines written by the user outside the block are kept; disabling the option removes the block
(and the file, if nothing else is left). New libraries get the rules once their initial download has finished.

| Group | Patterns |
| :--- | :--- |
| Microsoft Office (Word, Excel, PowerPoint, Visio) | `~$*`, `*.tmp` |
| LibreOffice / OpenOffice | `.~lock.*#` |
| WPS Office | `.~*` |
| draw.io | `.$*.bkp`, `.$*.dtmp` |
| AutoCAD | `*.dwl`, `*.dwl2`, `*.sv$` |
| Unfinished browser downloads | `*.crdownload`, `*.part`, `*.partial` |
| Editor swap files | `.*.swp`, `.*.swo`, `*~` |
| Windows / macOS system files | `desktop.ini`, `Thumbs.db`, `ehthumbs.db`, `.DS_Store` |

Limitations:
- `seafile-ignore.txt` is synced like any other file, so the rules apply to every client of the library.
- Files that were already uploaded stay on the server.
- seaf-daemon already ignores `~$*`, `*.tmp`, `.~lock*#` and `.~*` by default; the option mainly adds the other groups.

---

## 2. User Interface (UI/UX) Requirements
- Clean, compact UI using TailwindCSS.
- App version shown in the header next to the app name.
- Real-time status update for network connection (`Unmetered` vs `Metered Hotspot`).
- Toast notifications when libraries are automatically paused due to network changes.
- Every enabled button, select, checkbox and radio shows a pointer cursor; disabled controls show `not-allowed`.
- App icon: white cloud with up/down arrows on a blue gradient (`public/app-icon.svg`, generated into
  `src-tauri/icons` with `pnpm tauri icon public/app-icon.svg`).