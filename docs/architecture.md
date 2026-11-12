# ARCHITECTURE SPECIFICATION: Seafile Smart Metered Client

## 1. System Architecture Overview

```
+------------------------------------------------------------------+
|                   TAURI FRONTEND (React / TS)                     |
|  - Dashboard UI           - Library Config Grid                 |
|  - Network Status Badge   - Tray Icon & Event Listeners         |
+------------------------------------------------------------------+
                                 | (Tauri IPC Commands & Events)
+------------------------------------------------------------------+
|                    TAURI BACKEND (Rust Core)                     |
|                                                                  |
|  [Network Watcher] ---> (Tokio mpsc) ---> [Sync Controller]     |
|   (Windows COM API)                            |                 |
|                                                v                 |
|  [Config DB (sled)] <---------------> [SeafDaemon RPC Client]    |
+------------------------------------------------------------------+
                                                 | (Local RPC / Pipe)
                                                 v
+------------------------------------------------------------------+
|                 SIDECAR PROCESS (seaf-daemon.exe)                |
|  - Handles SHA-1 Delta Sync & Direct Disk Watcher               |
+------------------------------------------------------------------+
```

---

## 2. Rust Backend Component Design (`src-tauri/src/`)

### 2.1 File Structure
```
src-tauri/
├── binaries/
│   ├── seaf-daemon-x86_64-pc-windows-msvc.exe
│   └── *.dll (libsearpc.dll, libseafile.dll, etc.)
├── icons/                    # Generated from public/app-icon.svg
├── src/
│   ├── main.rs / lib.rs      # Entry point & Tauri Builder, plugins, startup visibility
│   ├── state.rs              # AppState & Sled DB wrapper (libraries, settings, history)
│   ├── network_watcher.rs    # Windows COM NLM Event Listener
│   ├── sync_controller.rs    # Core logic matrix for pause/resume, live status
│   ├── seaf_rpc.rs           # RPC wrapper to communicate with seaf-daemon
│   ├── activity.rs           # Poller: transfer rates, tray status, notification queue -> history
│   ├── ignore_rules.rs       # Managed temp-file block in seafile-ignore.txt
│   ├── tray.rs               # Tray menu, tooltip, status badge rendering
│   ├── daemon.rs             # seaf-daemon spawn/stop (Job Object)
│   ├── data_location.rs      # Data folder selection and move
│   ├── shell_ext.rs          # Explorer overlay icon server
│   └── commands.rs           # Tauri IPC commands exposed to UI
├── build.rs                  # tauri-build + rerun when icons/ changes
└── tauri.conf.json
```

Windows: `main` (declared in `tauri.conf.json` with `create: false`; `tray::show_main_window` builds it from that
config on demand: at startup unless it is a minimized autostart launch, from the tray, or from a second launch).
Closing `main` destroys it, so no WebView2 process stays alive while the app sits in the tray; `RunEvent::ExitRequested`
with `code: None` (last window closed) is prevented, and only the tray "Exit" (`app.exit(0)`) quits.
`settings` is created on demand by `open_settings_window`, parent = `main`, and is destroyed with it.
Both load `index.html`; `src/main.tsx` renders `SettingsWindow` or `App` based on the window label.

### 2.2 Data Models (`state.rs`)
```rust
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySyncConfig {
    pub repo_id: String,
    pub repo_name: String,
    pub local_path: String,
    pub is_user_paused: bool,
    pub bypass_metered: bool,
    pub current_state: SyncStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SyncStatus {
    Synced,
    Syncing,
    PausedMetered,
    PausedManual,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetworkCost {
    Unmetered,
    Metered,
}

pub struct TransferRate { pub upload: u64, pub download: u64 }      // bytes/s

pub struct HistoryEntry {
    pub id: u64, pub time: i64,                                        // unix seconds
    pub repo_id: String, pub repo_name: String,
    pub kind: HistoryKind,                                             // Synced | Error
    pub detail: String,                                                // commit desc / error text
    pub path: Option<String>,
}

pub struct HistorySettings { pub max_age_days: u32, pub max_records: u32 }   // default 7 / 200

pub struct Preferences { pub start_minimized: bool, pub ignore_temp_files: bool }  // default true / false
```

Sled trees: `libraries` (key = repo id), `settings` (`session` DPAPI-encrypted, `history_settings`, `preferences`),
`history` (key = big-endian id, oldest first; pruned by age and count on every insert).

### 2.5 Activity Poller (`activity.rs`)
Loop: every 1 s while data moves, 3 s when idle.
1. `seafile_get_upload_rate` / `seafile_get_download_rate` → `transfer-rate` event + tray tooltip (only on change).
2. Overall tray status: `daemon_error` → Error; RPC disabled (starting / moving data) → Normal;
   otherwise `live_status` of every library (Error wins, any Syncing or rate > 0 → Syncing).
   The icon is redrawn only when the status changes.
3. `ignore_rules::sync_pending`: writes/removes the temp-file block for ready libraries not handled yet this session
   (reset when the preference changes; failures retried on the next tick).
4. Drains `seafile_get_sync_notification` (max 200/tick): `sync.done` (`name\tid\tcommit\tparent\tdesc`) and
   `sync.error` (JSON with `err_id`) become history entries → `history-updated` event.
   Draining also keeps the daemon's in-memory queue bounded.

### 2.6 Tray Badge (`tray.rs`)
The unbadged default window icon is kept as RGBA; `badged_icon` draws an anti-aliased dot with a white ring in the
bottom-right corner (radius 24 % of the icon size), green / blue / red for Normal / Syncing / Error.

### 2.7 Autostart
`tauri-plugin-autostart` registers the exe with the `--autostart` argument. In `setup`, the main window is shown
unless the process was started with `--autostart` and `Preferences.start_minimized` is true.

### 2.3 Network Watcher Thread (`network_watcher.rs`)
- Implements Windows COM `INetworkListManagerEvents` interface.
- Utilizes event callback `CostChanged` to detect `NLM_CONNECTION_COST_METERED`.
- Sends `NetworkCost` enum variants into a Tokio `mpsc::UnboundedSender` to guarantee 0% idle CPU utilization.

### 2.4 Sync Controller Logic Matrix (`sync_controller.rs`)
Listens to `NetworkCost` events and evaluates rules:

```rust
pub async fn handle_network_change(cost: NetworkCost, state: &AppState) {
    let libraries = state.get_all_libraries();
    for mut lib in libraries {
        match cost {
            NetworkCost::Metered => {
                if !lib.bypass_metered && lib.current_state != SyncStatus::PausedManual {
                    seaf_rpc::pause_repo(&lib.repo_id).await;
                    lib.current_state = SyncStatus::PausedMetered;
                }
            },
            NetworkCost::Unmetered => {
                if lib.current_state == SyncStatus::PausedMetered {
                    seaf_rpc::resume_repo(&lib.repo_id).await;
                    lib.current_state = SyncStatus::Synced;
                }
            }
        }
        state.save_library(&lib);
    }
}
```

---

## 3. Sidecar & Process Lifecycle
- **Initialization:** On app boot, `main.rs` checks if `%LOCALAPPDATA%/CustomSeafile/seafile-data` exists. If not, spawns `seaf-daemon.exe --init`.
- **Execution:** Spawns `seaf-daemon.exe` as a Tauri Sidecar process holding its `Child` process handle in Rust global state.
- **Termination Handling:** Listens to Tauri `RunEvent::ExitRequested` to issue a graceful `kill()` to `seaf-daemon.exe`, preventing orphaned background processes.

---

## 4. Tauri IPC API Specification (Commands)

| Command Name | Input Arguments | Return Type | Description |
| :--- | :--- | :--- | :--- |
| `login` | `server_url`, `email`, `password` | `Result<String, String>` | Authenticate and obtain auth token |
| `get_cloud_libraries` | None | `Result<Vec<RemoteRepo>, String>` | Fetch all repos from Seafile Web API |
| `setup_library_sync` | `repo_id`, `local_path`, `bypass_metered` | `Result<(), String>` | Mount folder to seaf-daemon & save config |
| `toggle_library_pause` | `repo_id`, `pause_state` | `Result<(), String>` | User manual pause/resume override |
| `toggle_bypass_metered` | `repo_id`, `bypass` | `Result<(), String>` | Update metered bypass rule for library |
| `get_network_status` | None | `NetworkCost` | Get current Windows network status |
| `open_library_folder` | `repo_id` | `Result<(), String>` | Open the library's local folder in Explorer |
| `open_library_on_cloud` | `repo_id`, `repo_name` | `Result<(), String>` | Open `<server>/library/<id>/<name>/` in the browser |
| `get_library_errors` | `repo_id` | `Result<Vec<LibraryError>, String>` | Current error + errors recorded by seaf-daemon (newest first) |
| `get_transfer_rate` | None | `TransferRate` | Current upload/download rate (bytes/s) |
| `get_sync_history` | None | `Vec<HistoryEntry>` | Recent sync history, newest first |
| `get_history_settings` / `set_history_settings` | `settings` | `HistorySettings` | Retention: max age (days) and max records |
| `clear_sync_history` | None | `Result<(), String>` | Delete all history entries |
| `open_settings_window` | None | `Result<(), String>` | Open or focus the Settings window |
| `get_general_settings` / `set_general_settings` | `settings` | `GeneralSettings` | Autostart, start minimized, ignore temp files |
| `get_temp_ignore_patterns` | None | `Vec<PatternGroup>` | Pattern groups written by "Ignore temporary files" |

Events: `transfer-rate` (payload `TransferRate`), `history-updated`. The `activity.rs` poller reads rates every 1 s while transferring (3 s when idle), updates the tray tooltip, and drains the daemon notification queue (`sync.done`, `sync.error`) into the `history` sled tree.
