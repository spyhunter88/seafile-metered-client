//! Background poller for live sync activity:
//! - transfer rates (UI status bar + tray tooltip),
//! - overall sync status (tray icon badge),
//! - the daemon's notification queue, turned into the persisted sync history.
//!
//! seaf-daemon pushes a "transfer" message every second while data moves and
//! never drops messages on its own, so draining the queue also keeps the
//! daemon's memory bounded.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::seaf_rpc::obj_field;
use crate::state::{AppState, HistoryEntry, HistoryKind, SyncStatus, TransferRate, now_secs};
use crate::sync_controller::live_status;
use crate::tray::TrayStatus;

pub const EVT_TRANSFER: &str = "transfer-rate";
pub const EVT_HISTORY: &str = "history-updated";

/// Poll faster while data is moving so the displayed rate stays current.
const ACTIVE_INTERVAL: Duration = Duration::from_secs(1);
const IDLE_INTERVAL: Duration = Duration::from_secs(3);
const MAX_MESSAGES_PER_TICK: usize = 200;

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let active = tick(&app, &state).await;
            tokio::time::sleep(if active { ACTIVE_INTERVAL } else { IDLE_INTERVAL }).await;
        }
    });
}

/// Returns true while a transfer is in progress.
async fn tick(app: &AppHandle, state: &AppState) -> bool {
    // Errors (daemon not running yet, data folder being moved) read as "no transfer".
    let rate = match (state.rpc.get_upload_rate().await, state.rpc.get_download_rate().await) {
        (Ok(upload), Ok(download)) => TransferRate { upload, download },
        _ => TransferRate::default(),
    };
    let changed = {
        let mut current = state.transfer.lock().unwrap();
        let changed = *current != rate;
        *current = rate;
        changed
    };
    if changed {
        crate::tray::refresh_tooltip(app);
        let _ = app.emit(EVT_TRANSFER, rate);
    }

    let transferring = rate != TransferRate::default();
    let status = overall_status(state, transferring).await;
    crate::tray::set_sync_status(app, status);

    drain_notifications(app, state).await;
    transferring
}

/// Error if the daemon failed or any library is in error, Syncing if data is
/// moving or a library is syncing, otherwise Normal (paused counts as normal).
async fn overall_status(state: &AppState, transferring: bool) -> TrayStatus {
    if state.daemon_error.lock().unwrap().is_some() {
        return TrayStatus::Error;
    }
    if !state.rpc.is_enabled() {
        // Daemon still starting or restarting after a data move.
        return TrayStatus::Normal;
    }
    let libs = state.get_all_libraries();
    let clone_tasks = if libs.is_empty() {
        Vec::new()
    } else {
        state.rpc.get_clone_tasks().await.unwrap_or_default()
    };
    let mut syncing = transferring;
    for lib in &libs {
        match live_status(state, lib, &clone_tasks).await {
            SyncStatus::Error(_) => return TrayStatus::Error,
            SyncStatus::Syncing => syncing = true,
            _ => {}
        }
    }
    if syncing { TrayStatus::Syncing } else { TrayStatus::Normal }
}

async fn drain_notifications(app: &AppHandle, state: &AppState) {
    let mut entries = Vec::new();
    for _ in 0..MAX_MESSAGES_PER_TICK {
        let Ok(Some(msg)) = state.rpc.pop_sync_notification().await else { break };
        let kind = msg.get("type").and_then(Value::as_str).unwrap_or("");
        let content = msg.get("content").and_then(Value::as_str).unwrap_or("");
        match kind {
            "sync.done" => {
                if let Some(e) = parse_sync_done(content) {
                    entries.push(e);
                }
            }
            "sync.error" => {
                if let Some(e) = parse_sync_error(state, content).await {
                    entries.push(e);
                }
            }
            _ => {}
        }
    }
    if entries.is_empty() {
        return;
    }
    match state.add_history(entries) {
        Ok(()) => {
            let _ = app.emit(EVT_HISTORY, ());
        }
        Err(e) => eprintln!("[activity] cannot save history: {e}"),
    }
}

/// Content: `repo_name \t repo_id \t commit_id \t parent_id \t commit_desc`.
fn parse_sync_done(content: &str) -> Option<HistoryEntry> {
    let mut parts = content.splitn(5, '\t');
    let repo_name = parts.next()?.to_string();
    let repo_id = parts.next()?.to_string();
    let desc = parts.nth(2).unwrap_or("").trim();
    Some(HistoryEntry {
        id: 0,
        time: now_secs(),
        repo_id,
        repo_name,
        kind: HistoryKind::Synced,
        detail: if desc.is_empty() { "Synchronized".into() } else { desc.to_string() },
        path: None,
    })
}

/// Content: JSON `{"repo_id", "repo_name", "path", "err_id"}`.
async fn parse_sync_error(state: &AppState, content: &str) -> Option<HistoryEntry> {
    let v: Value = serde_json::from_str(content).ok()?;
    let err_id = obj_field(&v, "err_id").and_then(Value::as_i64).unwrap_or(0);
    let path = obj_field(&v, "path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    Some(HistoryEntry {
        id: 0,
        time: now_secs(),
        repo_id: obj_field(&v, "repo_id").and_then(Value::as_str).unwrap_or("").to_string(),
        repo_name: obj_field(&v, "repo_name").and_then(Value::as_str).unwrap_or("").to_string(),
        kind: HistoryKind::Error,
        detail: state.rpc.sync_error_to_str(err_id).await,
        path,
    })
}

/// "1.2 MB/s" style label used by the tray tooltip.
pub fn format_rate(bytes_per_sec: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes_per_sec as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{bytes_per_sec} B/s") } else { format!("{v:.1} {}/s", UNITS[i]) }
}
