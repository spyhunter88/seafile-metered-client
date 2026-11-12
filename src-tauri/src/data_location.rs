//! Where the app keeps its data (config DB + seaf-daemon state), and moving it.
//!
//! Layout of a data directory:
//!   config.db/      sled DB (library rules, encrypted session)
//!   ccnet/          seaf-daemon config dir + logs
//!   seafile-data/   seaf-daemon internal data
//!   Seafile/        seaf-daemon default worktree dir
//!
//! The chosen location is remembered in a small pointer file at
//! `%LOCALAPPDATA%\SeafileMeteredClient\location.json`. Without that file the
//! default is `<exe dir>\data` (portable), falling back to `%LOCALAPPDATA%`
//! when the exe folder is not writable (e.g. installed under Program Files).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::state::{AppPaths, AppState};
use crate::{EVT_DAEMON_ERROR, daemon, sync_controller};

pub const EVT_STORAGE_CHANGED: &str = "storage-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocationKind {
    /// `<exe dir>\data`
    Portable,
    /// `%LOCALAPPDATA%\CustomSeafile`
    LocalAppData,
    /// Any folder chosen by the user.
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationSetting {
    pub kind: LocationKind,
    #[serde(default)]
    pub custom_path: Option<String>,
}

impl LocationSetting {
    pub fn resolve(&self) -> Result<PathBuf, String> {
        match self.kind {
            LocationKind::Portable => portable_dir(),
            LocationKind::LocalAppData => Ok(local_app_data_dir()),
            LocationKind::Custom => {
                let p = self.custom_path.as_deref().map(str::trim).unwrap_or_default();
                let path = PathBuf::from(p);
                if p.is_empty() || !path.is_absolute() {
                    return Err("Custom location must be an absolute folder path".into());
                }
                Ok(path)
            }
        }
    }
}

pub fn portable_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate the app exe: {e}"))?;
    let dir = exe.parent().ok_or("Cannot locate the app folder")?;
    Ok(dir.join("data"))
}

pub fn local_app_data_dir() -> PathBuf {
    local_app_data().join("CustomSeafile")
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn pointer_file() -> PathBuf {
    local_app_data().join("SeafileMeteredClient").join("location.json")
}

/// True if `dir` (or its nearest existing ancestor) accepts new files.
pub fn is_writable(dir: &Path) -> bool {
    let mut probe_dir = dir;
    while !probe_dir.exists() {
        match probe_dir.parent() {
            Some(p) => probe_dir = p,
            None => return false,
        }
    }
    let probe = probe_dir.join(format!(".write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn default_location() -> LocationSetting {
    let portable_ok = portable_dir().map(|p| is_writable(&p)).unwrap_or(false);
    LocationSetting {
        kind: if portable_ok { LocationKind::Portable } else { LocationKind::LocalAppData },
        custom_path: None,
    }
}

pub fn load_location() -> LocationSetting {
    std::fs::read(pointer_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(default_location)
}

fn save_location(loc: &LocationSetting) -> Result<(), String> {
    let file = pointer_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(loc).map_err(|e| e.to_string())?;
    std::fs::write(&file, bytes).map_err(|e| format!("Cannot save location setting: {e}"))
}

/// Resolves the startup location. If it is unusable (e.g. a removed USB drive),
/// falls back to `%LOCALAPPDATA%` and returns a warning for the UI. The pointer
/// file is left untouched so the original location is used again once available.
pub fn startup_location() -> (LocationSetting, AppPaths, Option<String>) {
    let loc = load_location();
    match loc.resolve().and_then(|p| std::fs::create_dir_all(&p).map(|_| p).map_err(|e| e.to_string())) {
        Ok(base) => (loc, AppPaths::at(base), None),
        Err(e) => {
            let fallback = LocationSetting { kind: LocationKind::LocalAppData, custom_path: None };
            let base = local_app_data_dir();
            let warning = format!(
                "Data location {:?} is not available ({e}). Using {} temporarily.",
                loc.custom_path.clone().unwrap_or_else(|| format!("{:?}", loc.kind)),
                base.display()
            );
            (fallback, AppPaths::at(base), Some(warning))
        }
    }
}

// ---- Moving ---------------------------------------------------------------

fn normalize(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    s.trim_end_matches('\\').to_string()
}

fn is_inside(child: &Path, parent: &Path) -> bool {
    let (c, p) = (normalize(child), normalize(parent));
    c == p || c.starts_with(&format!("{p}\\"))
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut it| it.next().is_none()).unwrap_or(false)
}

fn validate_target(state: &AppState, from: &Path, to: &Path) -> Result<(), String> {
    if normalize(from) == normalize(to) {
        return Err("Data is already stored in this location".into());
    }
    if is_inside(to, from) || is_inside(from, to) {
        return Err("The new location cannot be inside the current one (or vice versa)".into());
    }
    if to.exists() && !(to.is_dir() && is_empty_dir(to)) {
        return Err(format!("{} already exists and is not an empty folder", to.display()));
    }
    if !is_writable(to) {
        return Err(format!("{} is not writable", to.display()));
    }
    // Synced folders are referenced by absolute path in seaf-daemon; they must not move.
    if let Some(lib) = state
        .get_all_libraries()
        .into_iter()
        .find(|l| is_inside(Path::new(&l.local_path), from))
    {
        return Err(format!(
            "Library \"{}\" is synced to {}, which is inside the data folder. Unsync it or use \
             another folder before moving.",
            lib.repo_name, lib.local_path
        ));
    }
    // Never put app data inside a synced folder (it would be uploaded to the server).
    if let Some(lib) = state.get_all_libraries().into_iter().find(|l| {
        let lp = Path::new(&l.local_path);
        is_inside(to, lp) || is_inside(lp, to)
    }) {
        return Err(format!(
            "The new location overlaps the synced folder of library \"{}\" ({})",
            lib.repo_name, lib.local_path
        ));
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Moves a directory: rename on the same volume, copy + delete across volumes.
fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        std::fs::remove_dir(to).map_err(|e| format!("Cannot prepare {}: {e}", to.display()))?;
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if !from.exists() {
        return std::fs::create_dir_all(to).map_err(|e| e.to_string());
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = copy_dir(from, to) {
        let _ = std::fs::remove_dir_all(to);
        return Err(format!("Copy to {} failed: {e}", to.display()));
    }
    if let Err(e) = std::fs::remove_dir_all(from) {
        // Data is safe at the new place; leftovers can be deleted manually.
        eprintln!("[data-location] could not remove old folder {}: {e}", from.display());
    }
    Ok(())
}

/// Stops the daemon, closes the DB, moves the data folder, then reopens
/// everything at the new place. Rolls back to the old place on failure.
pub async fn move_data(app: &AppHandle, state: &AppState, target: LocationSetting) -> Result<(), String> {
    let new_base = target.resolve()?;
    let old_base = state.paths().base;
    validate_target(state, &old_base, &new_base)?;

    let result = {
        let _guard = state.lib_lock.lock().await;
        daemon::stop_async(state).await;
        state.close_storage()?;

        match move_dir(&old_base, &new_base) {
            Ok(()) => match save_location(&target) {
                Ok(()) => {
                    state.set_location(target, AppPaths::at(new_base));
                    Ok(())
                }
                Err(e) => {
                    // Keep pointer and data consistent.
                    let _ = move_dir(&new_base, &old_base);
                    Err(e)
                }
            },
            Err(e) => Err(e),
        }
    };

    // Reopen DB and restart the daemon at whichever location is current now.
    let reopen = state.open_storage();
    restart_daemon(app, state).await;
    let _ = app.emit(EVT_STORAGE_CHANGED, ());
    result.and(reopen)
}

async fn restart_daemon(app: &AppHandle, state: &AppState) {
    match daemon::start(app, state).await {
        Ok(()) => *state.daemon_error.lock().unwrap() = None,
        Err(e) => {
            *state.daemon_error.lock().unwrap() = Some(e.clone());
            let _ = app.emit(EVT_DAEMON_ERROR, e);
        }
    }
    // Re-apply pause rules to the restarted daemon.
    sync_controller::handle_network_change(app, state, state.network_cost(), true).await;
}

// ---- UI model ---------------------------------------------------------------

#[derive(Serialize)]
pub struct LocationOption {
    pub kind: LocationKind,
    pub path: String,
    pub available: bool,
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct StorageSettings {
    pub current_kind: LocationKind,
    pub current_path: String,
    pub options: Vec<LocationOption>,
    /// Set when the configured location was unavailable at startup.
    pub warning: Option<String>,
}

pub fn storage_settings(state: &AppState) -> StorageSettings {
    let current = state.location();
    let current_path = state.paths().base;

    let mut options = Vec::new();
    match portable_dir() {
        Ok(p) => {
            let writable = is_writable(&p);
            options.push(LocationOption {
                kind: LocationKind::Portable,
                path: p.display().to_string(),
                available: writable,
                note: (!writable).then(|| "The app folder is not writable (e.g. Program Files)".into()),
            });
        }
        Err(e) => options.push(LocationOption {
            kind: LocationKind::Portable,
            path: String::new(),
            available: false,
            note: Some(e),
        }),
    }
    options.push(LocationOption {
        kind: LocationKind::LocalAppData,
        path: local_app_data_dir().display().to_string(),
        available: true,
        note: None,
    });
    options.push(LocationOption {
        kind: LocationKind::Custom,
        path: if current.kind == LocationKind::Custom {
            current.custom_path.clone().unwrap_or_default()
        } else {
            String::new()
        },
        available: true,
        note: None,
    });

    StorageSettings {
        current_kind: current.kind,
        current_path: current_path.display().to_string(),
        options,
        warning: state.storage_warning.lock().unwrap().clone(),
    }
}
