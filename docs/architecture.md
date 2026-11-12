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
├── src/
│   ├── main.rs               # Entry point & Tauri Builder
│   ├── state.rs              # AppState & Sled DB wrapper
│   ├── network_watcher.rs    # Windows COM NLM Event Listener
│   ├── sync_controller.rs    # Core logic matrix for pause/resume
│   ├── seaf_rpc.rs           # RPC wrapper to communicate with seaf-daemon
│   └── commands.rs           # Tauri IPC commands exposed to UI
└── tauri.conf.json
```

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
```

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

Events: `transfer-rate` (payload `TransferRate`), `history-updated`. The `activity.rs` poller reads rates every 1 s while transferring (3 s when idle), updates the tray tooltip, and drains the daemon notification queue (`sync.done`, `sync.error`) into the `history` sled tree.
