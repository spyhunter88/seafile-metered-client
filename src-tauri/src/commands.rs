use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::activity::EVT_HISTORY;
use crate::data_location::{self, LocationKind, LocationSetting, StorageSettings};
use crate::explorer_integration;
use crate::seaf_rpc::obj_field;
use crate::state::{
    AppState, HistoryEntry, HistorySettings, LibrarySyncConfig, NetworkCost, Preferences, Session, SyncStatus,
    TransferRate,
};
use crate::sync_controller::{self, EVT_LIBRARIES};

type AppStateRef<'a> = State<'a, Arc<AppState>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRepo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub permission: String,
    #[serde(default)]
    pub mtime: i64,
    #[serde(default)]
    pub owner: String,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub server_url: String,
    pub email: String,
}

// ---- Helpers --------------------------------------------------------------

fn normalize_server_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Server URL must start with http:// or https://".into());
    }
    Ok(url.to_string())
}

fn validate_repo_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if ok { Ok(()) } else { Err("Invalid library id".into()) }
}

async fn current_session(state: &AppState) -> Result<Session, String> {
    state
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| "Not logged in".to_string())
}

async fn api_get(state: &AppState, path: &str) -> Result<Value, String> {
    let session = current_session(state).await?;
    let resp = state
        .http
        .get(format!("{}{}", session.server_url, path))
        .header("Authorization", format!("Token {}", session.token))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err("Session expired or access denied, please log in again".into());
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Server returned {status}: {}", body.chars().take(200).collect::<String>()));
    }
    resp.json().await.map_err(|e| format!("Invalid server response: {e}"))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false"),
        _ => false,
    }
}

fn non_empty_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn notify_libraries(app: &AppHandle) {
    let _ = app.emit(EVT_LIBRARIES, ());
}

// ---- Account --------------------------------------------------------------

#[tauri::command]
pub async fn login(
    state: AppStateRef<'_>,
    server_url: String,
    email: String,
    password: String,
) -> Result<String, String> {
    let server_url = normalize_server_url(&server_url)?;
    let email = email.trim().to_string();
    if email.is_empty() || password.is_empty() {
        return Err("Email and password are required".into());
    }

    let resp = state
        .http
        .post(format!("{server_url}/api2/auth-token/"))
        .form(&[("username", email.as_str()), ("password", password.as_str())])
        .send()
        .await
        .map_err(|e| format!("Cannot reach server: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);

    let Some(token) = body.get("token").and_then(Value::as_str) else {
        let msg = body
            .get("non_field_errors")
            .and_then(|v| v.get(0))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Login failed ({status})"));
        return Err(msg);
    };

    let session = Session {
        server_url,
        email: email.clone(),
        token: token.to_string(),
    };
    state.store_session(&session)?;
    *state.session.write().await = Some(session);
    // The token stays in the Rust backend; the UI only needs the account name.
    Ok(email)
}

#[tauri::command]
pub async fn get_session(state: AppStateRef<'_>) -> Result<Option<SessionInfo>, String> {
    Ok(state.session.read().await.as_ref().map(|s| SessionInfo {
        server_url: s.server_url.clone(),
        email: s.email.clone(),
    }))
}

#[tauri::command]
pub async fn logout(state: AppStateRef<'_>) -> Result<(), String> {
    state.clear_session()?;
    *state.session.write().await = None;
    Ok(())
}

// ---- Libraries ------------------------------------------------------------

#[tauri::command]
pub async fn get_cloud_libraries(state: AppStateRef<'_>) -> Result<Vec<RemoteRepo>, String> {
    let v = api_get(&state, "/api2/repos/").await?;
    let items = v.as_array().cloned().unwrap_or_default();
    let mut repos: Vec<RemoteRepo> = Vec::new();
    for item in items {
        let Ok(repo) = serde_json::from_value::<RemoteRepo>(item) else { continue };
        // The same library can appear several times (own / shared / group).
        if !repos.iter().any(|r| r.id == repo.id) {
            repos.push(repo);
        }
    }
    repos.sort_by_key(|r| r.name.to_lowercase());
    Ok(repos)
}

/// Locally configured libraries with their live status.
#[tauri::command]
pub async fn get_libraries(state: AppStateRef<'_>) -> Result<Vec<LibrarySyncConfig>, String> {
    let mut libs = state.get_all_libraries();
    let clone_tasks = state.rpc.get_clone_tasks().await.unwrap_or_default();
    for lib in &mut libs {
        lib.current_state = sync_controller::live_status(&state, lib, &clone_tasks).await;
    }
    libs.sort_by_key(|l| l.repo_name.to_lowercase());
    Ok(libs)
}

#[tauri::command]
pub async fn setup_library_sync(
    app: AppHandle,
    state: AppStateRef<'_>,
    repo_id: String,
    local_path: String,
    bypass_metered: bool,
) -> Result<(), String> {
    validate_repo_id(&repo_id)?;
    let local_path = local_path.trim().to_string();
    if local_path.is_empty() || !std::path::Path::new(&local_path).is_absolute() {
        return Err("Please choose an absolute local folder path".into());
    }
    if state.get_library(&repo_id).is_some() {
        return Err("This library is already synced".into());
    }
    if state.network_cost() == NetworkCost::Metered && !bypass_metered {
        return Err(
            "You are on a metered network. Enable \"Bypass Metered\" or connect to Wi-Fi/Ethernet \
             to start the initial download."
                .into(),
        );
    }

    let session = current_session(&state).await?;
    let info = api_get(&state, &format!("/api2/repos/{repo_id}/download-info/")).await?;
    if truthy(info.get("encrypted")) {
        return Err("Encrypted libraries are not supported yet".into());
    }
    let token = non_empty_str(info.get("token")).ok_or("Server did not return a sync token")?;
    let repo_name = non_empty_str(info.get("repo_name")).unwrap_or(&repo_id).to_string();
    let email = non_empty_str(info.get("email")).unwrap_or(&session.email);
    let repo_version = info.get("repo_version").and_then(Value::as_i64).unwrap_or(1);
    let enc_version = info.get("enc_version").and_then(Value::as_i64).unwrap_or(0);
    let readonly = info.get("permission").and_then(Value::as_str) == Some("r");

    let mut more_info = json!({
        "server_url": session.server_url,
        "is_readonly": if readonly { 1 } else { 0 },
    });
    if let Some(salt) = non_empty_str(info.get("salt")) {
        more_info["repo_salt"] = json!(salt);
    }

    std::fs::create_dir_all(&local_path).map_err(|e| format!("Cannot create folder: {e}"))?;

    let _guard = state.lib_lock.lock().await;
    state
        .rpc
        .clone_repo(
            &repo_id,
            repo_version,
            &repo_name,
            &local_path,
            token,
            non_empty_str(info.get("magic")),
            email,
            non_empty_str(info.get("random_key")),
            enc_version,
            &more_info.to_string(),
        )
        .await?;

    let lib = LibrarySyncConfig {
        repo_id,
        repo_name,
        local_path,
        is_user_paused: false,
        bypass_metered,
        current_state: SyncStatus::Synced,
    };
    state.save_library(&lib)?;
    drop(_guard);
    notify_libraries(&app);
    Ok(())
}

async fn update_library(
    app: &AppHandle,
    state: &AppState,
    repo_id: &str,
    f: impl FnOnce(&mut LibrarySyncConfig),
) -> Result<(), String> {
    validate_repo_id(repo_id)?;
    let _guard = state.lib_lock.lock().await;
    let mut lib = state.get_library(repo_id).ok_or("Library is not synced locally")?;
    f(&mut lib);
    sync_controller::apply_policy(state, &mut lib, state.network_cost(), false).await;
    state.save_library(&lib)?;
    drop(_guard);
    notify_libraries(app);
    Ok(())
}

#[tauri::command]
pub async fn toggle_library_pause(
    app: AppHandle,
    state: AppStateRef<'_>,
    repo_id: String,
    pause_state: bool,
) -> Result<(), String> {
    update_library(&app, &state, &repo_id, |lib| lib.is_user_paused = pause_state).await
}

#[tauri::command]
pub async fn toggle_bypass_metered(
    app: AppHandle,
    state: AppStateRef<'_>,
    repo_id: String,
    bypass: bool,
) -> Result<(), String> {
    update_library(&app, &state, &repo_id, |lib| lib.bypass_metered = bypass).await
}

/// One-time sync of a library, even while it is paused (e.g. on a metered network).
#[tauri::command]
pub async fn force_sync(app: AppHandle, state: AppStateRef<'_>, repo_id: String) -> Result<(), String> {
    validate_repo_id(&repo_id)?;
    if state.get_library(&repo_id).is_none() {
        return Err("Library is not synced locally".into());
    }
    state.rpc.sync_now(&repo_id).await?;
    notify_libraries(&app);
    Ok(())
}

/// Stops syncing a library. Local files are kept.
#[tauri::command]
pub async fn unsync_library(app: AppHandle, state: AppStateRef<'_>, repo_id: String) -> Result<(), String> {
    validate_repo_id(&repo_id)?;
    let _guard = state.lib_lock.lock().await;
    if state.rpc.destroy_repo(&repo_id).await.is_err() {
        // Not a repo yet: probably an initial download in progress.
        let _ = state.rpc.cancel_clone_task(&repo_id).await;
    }
    state.remove_library(&repo_id)?;
    drop(_guard);
    notify_libraries(&app);
    Ok(())
}

/// Opens the library's local folder in Explorer.
#[tauri::command]
pub async fn open_library_folder(app: AppHandle, state: AppStateRef<'_>, repo_id: String) -> Result<(), String> {
    validate_repo_id(&repo_id)?;
    let lib = state.get_library(&repo_id).ok_or("Library is not synced locally")?;
    if !std::path::Path::new(&lib.local_path).is_dir() {
        return Err(format!("Folder does not exist: {}", lib.local_path));
    }
    app.opener()
        .open_path(lib.local_path, None::<&str>)
        .map_err(|e| format!("Cannot open folder: {e}"))
}

/// Opens the library in the Seafile web UI (`<server>/library/<id>/<name>/`).
#[tauri::command]
pub async fn open_library_on_cloud(
    app: AppHandle,
    state: AppStateRef<'_>,
    repo_id: String,
    repo_name: String,
) -> Result<(), String> {
    validate_repo_id(&repo_id)?;
    let session = current_session(&state).await?;
    let mut url = reqwest::Url::parse(&session.server_url).map_err(|e| format!("Invalid server URL: {e}"))?;
    url.path_segments_mut()
        .map_err(|_| "Invalid server URL".to_string())?
        .pop_if_empty()
        .extend(["library", repo_id.as_str(), repo_name.as_str(), ""]);
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| format!("Cannot open browser: {e}"))
}

#[derive(Serialize)]
pub enum RevealResult {
    /// The folder was opened.
    Opened,
    /// The file was selected in its folder.
    Revealed,
    /// The item no longer exists; its closest existing parent folder was opened.
    Missing,
}

/// Shows a file or folder of a library in Explorer. `path` is relative to the
/// library root (`None` = the root). Deleted items open their nearest parent folder.
#[tauri::command]
pub async fn reveal_library_path(
    app: AppHandle,
    state: AppStateRef<'_>,
    repo_id: String,
    path: Option<String>,
) -> Result<RevealResult, String> {
    validate_repo_id(&repo_id)?;
    let lib = state.get_library(&repo_id).ok_or("Library is not synced locally")?;
    let root = std::path::PathBuf::from(&lib.local_path);
    if !root.is_dir() {
        return Err(format!("Folder does not exist: {}", lib.local_path));
    }
    let mut target = root.clone();
    for comp in path.as_deref().unwrap_or("").split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        // Stay inside the library folder.
        if comp == ".." || comp.contains(':') {
            return Err("Invalid path".into());
        }
        target.push(comp);
    }

    let opener = app.opener();
    let open_dir = |dir: &std::path::Path| {
        opener
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("Cannot open folder: {e}"))
    };
    if target.is_dir() {
        open_dir(&target)?;
        return Ok(RevealResult::Opened);
    }
    if target.exists() {
        opener
            .reveal_item_in_dir(&target)
            .map_err(|e| format!("Cannot show file: {e}"))?;
        return Ok(RevealResult::Revealed);
    }
    let mut dir = target.as_path();
    while let Some(parent) = dir.parent() {
        dir = parent;
        if dir.is_dir() || dir == root {
            break;
        }
    }
    open_dir(dir)?;
    Ok(RevealResult::Missing)
}

#[derive(Serialize)]
pub struct LibraryError {
    /// Unix time in seconds; `None` for the current task state.
    pub time: Option<i64>,
    pub path: Option<String>,
    pub message: String,
}

/// Errors of one library, newest first: the current sync/clone error (if any)
/// followed by the file- and library-level errors recorded by seaf-daemon.
#[tauri::command]
pub async fn get_library_errors(state: AppStateRef<'_>, repo_id: String) -> Result<Vec<LibraryError>, String> {
    const MAX: usize = 20;
    validate_repo_id(&repo_id)?;
    let lib = state.get_library(&repo_id).ok_or("Library is not synced locally")?;

    let mut out = Vec::new();
    let clone_tasks = state.rpc.get_clone_tasks().await.unwrap_or_default();
    if let SyncStatus::Error(msg) = sync_controller::live_status(&state, &lib, &clone_tasks).await {
        out.push(LibraryError { time: None, path: None, message: msg });
    }

    let recorded = state.rpc.get_file_sync_errors(0, 500).await?;
    let mut messages: std::collections::HashMap<i64, String> = Default::default();
    for e in recorded
        .iter()
        .filter(|e| obj_field(e, "repo_id").and_then(Value::as_str) == Some(repo_id.as_str()))
        .take(MAX)
    {
        let err_id = obj_field(e, "err_id").and_then(Value::as_i64).unwrap_or(0);
        let message = match messages.get(&err_id) {
            Some(m) => m.clone(),
            None => {
                let m = state.rpc.sync_error_to_str(err_id).await;
                messages.insert(err_id, m.clone());
                m
            }
        };
        out.push(LibraryError {
            time: obj_field(e, "timestamp").and_then(Value::as_i64),
            path: non_empty_str(obj_field(e, "path")).map(str::to_string),
            message,
        });
    }
    Ok(out)
}

// ---- General settings -----------------------------------------------------

pub const SETTINGS_WINDOW: &str = "settings";

/// Opens the Settings window (or focuses it if already open). It is owned by
/// the main window: stays above it and has no separate taskbar button.
#[tauri::command]
pub async fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let mut builder = WebviewWindowBuilder::new(&app, SETTINGS_WINDOW, WebviewUrl::App("index.html".into()))
        .title("Settings - Seafile Metered Client")
        .inner_size(600.0, 560.0)
        .min_inner_size(480.0, 420.0)
        .center();
    if let Some(main) = app.get_webview_window(crate::tray::MAIN_WINDOW) {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| format!("Cannot open Settings: {e}"))?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct GeneralSettings {
    /// Registered in HKCU\...\Run (read from the registry, not cached).
    pub autostart: bool,
    pub start_minimized: bool,
    pub ignore_temp_files: bool,
}

#[tauri::command]
pub fn get_general_settings(app: AppHandle, state: AppStateRef<'_>) -> Result<GeneralSettings, String> {
    let prefs = state.preferences();
    Ok(GeneralSettings {
        autostart: app.autolaunch().is_enabled().map_err(|e| format!("Cannot read autostart: {e}"))?,
        start_minimized: prefs.start_minimized,
        ignore_temp_files: prefs.ignore_temp_files,
    })
}

#[tauri::command]
pub fn set_general_settings(
    app: AppHandle,
    state: AppStateRef<'_>,
    settings: GeneralSettings,
) -> Result<(), String> {
    let launcher = app.autolaunch();
    if launcher.is_enabled().ok() != Some(settings.autostart) {
        let res = if settings.autostart { launcher.enable() } else { launcher.disable() };
        res.map_err(|e| format!("Cannot change autostart: {e}"))?;
    }
    let previous = state.preferences();
    state.set_preferences(Preferences {
        start_minimized: settings.start_minimized,
        ignore_temp_files: settings.ignore_temp_files,
    })?;
    if previous.ignore_temp_files != settings.ignore_temp_files {
        // The activity poller rewrites every library's ignore file within a few seconds.
        state.ignore_applied.lock().unwrap().clear();
    }
    Ok(())
}

/// Temp-file patterns written by the "ignore temporary files" option (for the UI).
#[tauri::command]
pub fn get_temp_ignore_patterns() -> &'static [crate::ignore_rules::PatternGroup] {
    crate::ignore_rules::TEMP_PATTERNS
}

// ---- Activity & history ---------------------------------------------------

#[tauri::command]
pub fn get_transfer_rate(state: AppStateRef<'_>) -> TransferRate {
    state.transfer_rate()
}

#[tauri::command]
pub fn get_uploads(state: AppStateRef<'_>) -> Vec<crate::uploads::UploadTask> {
    state.uploads.snapshot()
}

#[tauri::command]
pub fn get_sync_history(state: AppStateRef<'_>) -> Vec<HistoryEntry> {
    state.get_history()
}

#[tauri::command]
pub fn get_history_settings(state: AppStateRef<'_>) -> HistorySettings {
    state.history_settings()
}

#[tauri::command]
pub fn set_history_settings(app: AppHandle, state: AppStateRef<'_>, settings: HistorySettings) -> Result<(), String> {
    state.set_history_settings(settings)?;
    let _ = app.emit(EVT_HISTORY, ());
    Ok(())
}

#[tauri::command]
pub fn clear_sync_history(app: AppHandle, state: AppStateRef<'_>) -> Result<(), String> {
    state.clear_history()?;
    let _ = app.emit(EVT_HISTORY, ());
    Ok(())
}

// ---- Misc -----------------------------------------------------------------

#[tauri::command]
pub fn get_network_status(state: AppStateRef<'_>) -> NetworkCost {
    state.network_cost()
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.ok().flatten();
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

/// `None` when seaf-daemon is running (or still starting), otherwise the startup error.
#[tauri::command]
pub fn get_daemon_error(state: AppStateRef<'_>) -> Option<String> {
    state.daemon_error.lock().unwrap().clone()
}

// ---- Data location --------------------------------------------------------

#[tauri::command]
pub fn get_storage_settings(state: AppStateRef<'_>) -> StorageSettings {
    data_location::storage_settings(&state)
}

/// Moves config DB + seaf-daemon data to another folder. Sync is stopped
/// for the duration of the move and restarted afterwards.
#[tauri::command]
pub async fn move_data_location(
    app: AppHandle,
    state: AppStateRef<'_>,
    kind: LocationKind,
    custom_path: Option<String>,
) -> Result<(), String> {
    let target = LocationSetting {
        kind,
        custom_path: if kind == LocationKind::Custom { custom_path } else { None },
    };
    data_location::move_data(&app, &state, target).await
}

// ---- Explorer integration (overlay icons) ----------------------------------

#[tauri::command]
pub fn get_explorer_integration(app: AppHandle) -> explorer_integration::IntegrationStatus {
    explorer_integration::status(&app)
}

/// Registers the overlay icon extension (shows a UAC prompt).
#[tauri::command]
pub async fn install_explorer_integration(app: AppHandle) -> Result<(), String> {
    explorer_integration::install(&app).await
}

#[tauri::command]
pub async fn uninstall_explorer_integration() -> Result<(), String> {
    explorer_integration::uninstall().await
}

#[tauri::command]
pub async fn restart_explorer() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(explorer_integration::restart_explorer)
        .await
        .map_err(|e| e.to_string())?
}
