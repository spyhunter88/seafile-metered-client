//! Pause/resume policy per library, driven by network cost and user overrides.
//!
//! Rule matrix (persisted `current_state`):
//!   is_user_paused                      -> PausedManual
//!   Metered && !bypass_metered          -> PausedMetered
//!   otherwise                           -> Synced (live state is read from the daemon)

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::seaf_rpc::obj_field;
use crate::state::{AppState, LibrarySyncConfig, NetworkCost, SyncStatus};

pub const EVT_NETWORK: &str = "network-changed";
pub const EVT_LIBRARIES: &str = "libraries-updated";
pub const EVT_AUTO_PAUSED: &str = "auto-paused";
pub const EVT_AUTO_RESUMED: &str = "auto-resumed";

#[derive(Clone, Serialize)]
pub struct AutoChange {
    pub libraries: Vec<String>,
    pub network: NetworkCost,
}

fn is_paused(s: &SyncStatus) -> bool {
    matches!(s, SyncStatus::PausedManual | SyncStatus::PausedMetered)
}

pub fn desired_state(lib: &LibrarySyncConfig, cost: NetworkCost) -> SyncStatus {
    if lib.is_user_paused {
        SyncStatus::PausedManual
    } else if cost == NetworkCost::Metered && !lib.bypass_metered {
        SyncStatus::PausedMetered
    } else {
        SyncStatus::Synced
    }
}

/// Applies the policy to one library (talking to the daemon when the paused
/// state changes, or always when `force`). Returns the previous state.
/// Caller must hold `state.lib_lock`.
pub async fn apply_policy(
    state: &AppState,
    lib: &mut LibrarySyncConfig,
    cost: NetworkCost,
    force: bool,
) -> SyncStatus {
    let previous = lib.current_state.clone();
    let next = desired_state(lib, cost);
    let was_paused = is_paused(&previous);
    let now_paused = is_paused(&next);

    if force || was_paused != now_paused {
        let res = if now_paused {
            state.rpc.pause_repo(&lib.repo_id).await
        } else {
            state.rpc.resume_repo(&lib.repo_id).await
        };
        // A failure usually means the library is still being cloned. The state is
        // persisted anyway and re-applied by `live_status` once the repo exists.
        if let Err(e) = res {
            eprintln!("[sync-controller] {}: {e}", lib.repo_name);
        }
    }
    lib.current_state = next;
    previous
}

/// Reacts to a network cost change: re-evaluates every library and notifies the UI.
pub async fn handle_network_change(app: &AppHandle, state: &AppState, cost: NetworkCost, initial: bool) {
    eprintln!("[network] cost = {cost:?}");
    state.set_network_cost(cost);
    crate::tray::set_network(app, cost);
    let _ = app.emit(EVT_NETWORK, cost);

    let mut paused = Vec::new();
    let mut resumed = Vec::new();
    {
        let _guard = state.lib_lock.lock().await;
        for mut lib in state.get_all_libraries() {
            // On startup force-apply so the daemon matches our persisted rules.
            let prev = apply_policy(state, &mut lib, cost, initial).await;
            if !initial {
                if lib.current_state == SyncStatus::PausedMetered && !is_paused(&prev) {
                    paused.push(lib.repo_name.clone());
                } else if prev == SyncStatus::PausedMetered && !is_paused(&lib.current_state) {
                    resumed.push(lib.repo_name.clone());
                }
            }
            if let Err(e) = state.save_library(&lib) {
                eprintln!("[sync-controller] save failed: {e}");
            }
        }
    }

    if !paused.is_empty() || !resumed.is_empty() {
        crate::shell_ext::refresh_explorer();
    }
    if !paused.is_empty() {
        notify(app, "Sync paused (metered network)", &paused);
        let _ = app.emit(EVT_AUTO_PAUSED, AutoChange { libraries: paused, network: cost });
    }
    if !resumed.is_empty() {
        notify(app, "Sync resumed (unmetered network)", &resumed);
        let _ = app.emit(EVT_AUTO_RESUMED, AutoChange { libraries: resumed, network: cost });
    }
    let _ = app.emit(EVT_LIBRARIES, ());
}

/// Tray "Pause All" / "Resume All".
pub async fn set_all_user_paused(app: &AppHandle, state: &AppState, paused: bool) {
    let cost = state.network_cost();
    {
        let _guard = state.lib_lock.lock().await;
        for mut lib in state.get_all_libraries() {
            lib.is_user_paused = paused;
            apply_policy(state, &mut lib, cost, false).await;
            if let Err(e) = state.save_library(&lib) {
                eprintln!("[sync-controller] save failed: {e}");
            }
        }
    }
    crate::shell_ext::refresh_explorer();
    let _ = app.emit(EVT_LIBRARIES, ());
}

fn notify(app: &AppHandle, title: &str, libs: &[String]) {
    let body = if libs.len() <= 3 {
        libs.join(", ")
    } else {
        format!("{} and {} more", libs[..3].join(", "), libs.len() - 3)
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

pub const ACTIVE_SYNC_STATES: &[&str] = &["committing", "initializing", "downloading", "uploading", "merging"];

/// Computes the status shown in the UI from the persisted policy state plus
/// the daemon's live clone/sync task info. Does not persist anything.
pub async fn live_status(state: &AppState, lib: &LibrarySyncConfig, clone_tasks: &[Value]) -> SyncStatus {
    let rpc = &state.rpc;

    // 1. Initial download (clone) still in progress?
    if let Some(task) = clone_tasks.iter().find(|t| {
        obj_field(t, "repo_id").and_then(Value::as_str) == Some(lib.repo_id.as_str())
    }) {
        let st = obj_field(task, "state").and_then(Value::as_str).unwrap_or("");
        match st {
            "done" => {}
            "canceled" | "canceling" => return SyncStatus::Error("Initial download canceled".into()),
            "error" => {
                let err = obj_field(task, "error").and_then(Value::as_i64).unwrap_or(0);
                return SyncStatus::Error(rpc.sync_error_to_str(err).await);
            }
            _ => return SyncStatus::Syncing,
        }
    }

    match rpc.get_repo(&lib.repo_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return SyncStatus::Error("Library is not registered in seaf-daemon".into()),
        Err(e) => return SyncStatus::Error(e),
    }

    // 2. Paused libraries: make sure the daemon really has auto-sync off
    //    (covers repos whose clone finished while a pause was pending).
    if is_paused(&lib.current_state)
        && let Ok(v) = rpc.get_repo_property(&lib.repo_id, "auto-sync").await
            && v.as_deref() != Some("false") {
                let _ = rpc.pause_repo(&lib.repo_id).await;
            }

    // 3. Live sync task (also covers a one-off "force sync" on a paused library).
    let task = match rpc.get_repo_sync_task(&lib.repo_id).await {
        Ok(t) => t,
        Err(e) => return SyncStatus::Error(e),
    };
    if let Some(task) = task {
        let st = obj_field(&task, "state").and_then(Value::as_str).unwrap_or("");
        if ACTIVE_SYNC_STATES.contains(&st) {
            return SyncStatus::Syncing;
        }
        if st == "error" && !is_paused(&lib.current_state) {
            let err = obj_field(&task, "error").and_then(Value::as_i64).unwrap_or(0);
            return SyncStatus::Error(rpc.sync_error_to_str(err).await);
        }
    }
    lib.current_state.clone()
}
