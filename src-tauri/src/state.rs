use std::path::PathBuf;
use std::process::Child;
use std::sync::{Mutex, RwLock as StdRwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::data_location::LocationSetting;
use crate::seaf_rpc::SeafRpc;

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum NetworkCost {
    Unmetered,
    Metered,
}

/// Current transfer rates reported by seaf-daemon, in bytes per second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct TransferRate {
    pub upload: u64,
    pub download: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum HistoryKind {
    Synced,
    Error,
}

/// One entry of the recent sync history (persisted in the config DB).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    /// Unix time in seconds.
    pub time: i64,
    pub repo_id: String,
    pub repo_name: String,
    pub kind: HistoryKind,
    /// Commit description for `Synced`, error text for `Error`.
    pub detail: String,
    /// File/folder inside the library, when the daemon reports one.
    #[serde(default)]
    pub path: Option<String>,
}

/// How much sync history is kept.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HistorySettings {
    pub max_age_days: u32,
    pub max_records: u32,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { max_age_days: 7, max_records: 200 }
    }
}

impl HistorySettings {
    pub const MAX_AGE_DAYS: u32 = 365;
    pub const MAX_RECORDS: u32 = 5000;

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=Self::MAX_AGE_DAYS).contains(&self.max_age_days) {
            return Err(format!("Keep history between 1 and {} days", Self::MAX_AGE_DAYS));
        }
        if !(10..=Self::MAX_RECORDS).contains(&self.max_records) {
            return Err(format!("Keep between 10 and {} records", Self::MAX_RECORDS));
        }
        Ok(())
    }
}

/// App preferences stored in the config DB. "Start with Windows" itself lives
/// in the registry Run key (autostart plugin), not here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// When launched by Windows at sign-in, stay in the tray instead of showing the window.
    pub start_minimized: bool,
    /// Maintain temp-file rules in each library's `seafile-ignore.txt`.
    pub ignore_temp_files: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        // Off by default: the ignore file is synced to the server like any other file.
        Self { start_minimized: true, ignore_temp_files: false }
    }
}

/// Account session. The whole struct is stored DPAPI-encrypted on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub server_url: String,
    pub email: String,
    pub token: String,
}

/// Data directories used by the app and the seaf-daemon sidecar.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub base: PathBuf,
    pub ccnet_dir: PathBuf,
    pub seafile_data_dir: PathBuf,
    pub worktree_dir: PathBuf,
    pub db_dir: PathBuf,
}

impl AppPaths {
    pub fn at(base: PathBuf) -> Self {
        Self {
            ccnet_dir: base.join("ccnet"),
            seafile_data_dir: base.join("seafile-data"),
            worktree_dir: base.join("Seafile"),
            db_dir: base.join("config.db"),
            base,
        }
    }
}

/// Open handles of the sled DB. Dropping this closes the DB files.
struct Storage {
    db: sled::Db,
    libraries: sled::Tree,
    settings: sled::Tree,
    /// Key = big-endian entry id, so iteration order is oldest first.
    history: sled::Tree,
}

impl Storage {
    fn open(paths: &AppPaths) -> Result<Self, String> {
        std::fs::create_dir_all(&paths.base).map_err(|e| e.to_string())?;
        let db = sled::open(&paths.db_dir).map_err(|e| format!("Cannot open config DB: {e}"))?;
        let libraries = db.open_tree("libraries").map_err(|e| e.to_string())?;
        let settings = db.open_tree("settings").map_err(|e| e.to_string())?;
        let history = db.open_tree("history").map_err(|e| e.to_string())?;
        Ok(Self { db, libraries, settings, history })
    }
}

pub struct AppState {
    location: StdRwLock<(LocationSetting, AppPaths)>,
    /// `None` only while the data folder is being moved.
    storage: StdRwLock<Option<Storage>>,
    /// Set when the configured data location was unavailable at startup.
    pub storage_warning: Mutex<Option<String>>,
    pub session: RwLock<Option<Session>>,
    pub network: Mutex<NetworkCost>,
    pub http: reqwest::Client,
    pub rpc: SeafRpc,
    pub daemon: Mutex<Option<Child>>,
    /// Set when seaf-daemon failed to start; surfaced in the UI.
    pub daemon_error: Mutex<Option<String>>,
    /// Serializes read-modify-write cycles on library configs
    /// (network events, tray actions and UI commands can race otherwise).
    pub lib_lock: tokio::sync::Mutex<()>,
    pub transfer: Mutex<TransferRate>,
    /// Libraries whose `seafile-ignore.txt` already matches the preference this session.
    pub ignore_applied: Mutex<std::collections::HashSet<String>>,
}

const SESSION_KEY: &[u8] = b"session";
const HISTORY_SETTINGS_KEY: &[u8] = b"history_settings";
const PREFERENCES_KEY: &[u8] = b"preferences";

impl AppState {
    pub fn new(location: LocationSetting, paths: AppPaths, warning: Option<String>) -> Result<Self, String> {
        let storage = Storage::open(&paths)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;

        // Auto-login: restore a previously stored session.
        let restored = load_session(&storage.settings);

        Ok(Self {
            location: StdRwLock::new((location, paths)),
            storage: StdRwLock::new(Some(storage)),
            storage_warning: Mutex::new(warning),
            session: RwLock::new(restored),
            network: Mutex::new(NetworkCost::Unmetered),
            http,
            rpc: SeafRpc::new(),
            daemon: Mutex::new(None),
            daemon_error: Mutex::new(None),
            lib_lock: tokio::sync::Mutex::new(()),
            transfer: Mutex::new(TransferRate::default()),
            ignore_applied: Mutex::new(Default::default()),
        })
    }

    pub fn transfer_rate(&self) -> TransferRate {
        *self.transfer.lock().unwrap()
    }

    pub fn network_cost(&self) -> NetworkCost {
        *self.network.lock().unwrap()
    }

    pub fn set_network_cost(&self, cost: NetworkCost) {
        *self.network.lock().unwrap() = cost;
    }

    // ---- Data location ---------------------------------------------------

    pub fn paths(&self) -> AppPaths {
        self.location.read().unwrap().1.clone()
    }

    pub fn location(&self) -> LocationSetting {
        self.location.read().unwrap().0.clone()
    }

    pub fn set_location(&self, location: LocationSetting, paths: AppPaths) {
        *self.location.write().unwrap() = (location, paths);
        *self.storage_warning.lock().unwrap() = None;
    }

    /// Flushes and closes the DB so its folder can be moved.
    pub fn close_storage(&self) -> Result<(), String> {
        let mut guard = self.storage.write().unwrap();
        if let Some(s) = guard.as_ref() {
            s.db.flush().map_err(|e| format!("Cannot flush config DB: {e}"))?;
        }
        *guard = None;
        Ok(())
    }

    /// (Re)opens the DB at the current location.
    pub fn open_storage(&self) -> Result<(), String> {
        let storage = Storage::open(&self.paths())?;
        *self.storage.write().unwrap() = Some(storage);
        Ok(())
    }

    fn with_storage<T>(&self, f: impl FnOnce(&Storage) -> Result<T, String>) -> Result<T, String> {
        let guard = self.storage.read().unwrap();
        let s = guard.as_ref().ok_or("Data folder is being moved, try again in a moment")?;
        f(s)
    }

    // ---- Libraries -------------------------------------------------------

    pub fn get_all_libraries(&self) -> Vec<LibrarySyncConfig> {
        self.with_storage(|s| {
            Ok(s.libraries
                .iter()
                .filter_map(|kv| kv.ok())
                .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
                .collect())
        })
        .unwrap_or_default()
    }

    pub fn get_library(&self, repo_id: &str) -> Option<LibrarySyncConfig> {
        self.with_storage(|s| Ok(s.libraries.get(repo_id.as_bytes()).ok().flatten()))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
    }

    pub fn save_library(&self, lib: &LibrarySyncConfig) -> Result<(), String> {
        let bytes = serde_json::to_vec(lib).map_err(|e| e.to_string())?;
        self.with_storage(|s| {
            s.libraries
                .insert(lib.repo_id.as_bytes(), bytes)
                .map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn remove_library(&self, repo_id: &str) -> Result<(), String> {
        self.with_storage(|s| {
            s.libraries.remove(repo_id.as_bytes()).map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    // ---- Preferences -----------------------------------------------------

    pub fn preferences(&self) -> Preferences {
        self.with_storage(|s| Ok(s.settings.get(PREFERENCES_KEY).ok().flatten()))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default()
    }

    pub fn set_preferences(&self, prefs: Preferences) -> Result<(), String> {
        let bytes = serde_json::to_vec(&prefs).map_err(|e| e.to_string())?;
        self.with_storage(|s| {
            s.settings.insert(PREFERENCES_KEY, bytes).map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    // ---- Sync history ----------------------------------------------------

    pub fn history_settings(&self) -> HistorySettings {
        self.with_storage(|s| Ok(s.settings.get(HISTORY_SETTINGS_KEY).ok().flatten()))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default()
    }

    pub fn set_history_settings(&self, settings: HistorySettings) -> Result<(), String> {
        settings.validate()?;
        let bytes = serde_json::to_vec(&settings).map_err(|e| e.to_string())?;
        self.with_storage(|s| {
            s.settings.insert(HISTORY_SETTINGS_KEY, bytes).map_err(|e| e.to_string())?;
            Ok(())
        })?;
        self.prune_history(settings)
    }

    /// Appends entries (id/time are filled in here) and applies the retention rules.
    pub fn add_history(&self, entries: Vec<HistoryEntry>) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        self.with_storage(|s| {
            for mut e in entries {
                e.id = s.db.generate_id().map_err(|e| e.to_string())?;
                let bytes = serde_json::to_vec(&e).map_err(|e| e.to_string())?;
                s.history.insert(e.id.to_be_bytes(), bytes).map_err(|e| e.to_string())?;
            }
            Ok(())
        })?;
        self.prune_history(self.history_settings())
    }

    /// Newest first.
    pub fn get_history(&self) -> Vec<HistoryEntry> {
        let settings = self.history_settings();
        let min_time = now_secs() - i64::from(settings.max_age_days) * 86_400;
        self.with_storage(|s| {
            Ok(s.history
                .iter()
                .rev()
                .filter_map(|kv| kv.ok())
                .filter_map(|(_, v)| serde_json::from_slice::<HistoryEntry>(&v).ok())
                .filter(|e| e.time >= min_time)
                .take(settings.max_records as usize)
                .collect())
        })
        .unwrap_or_default()
    }

    pub fn clear_history(&self) -> Result<(), String> {
        self.with_storage(|s| {
            s.history.clear().map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn prune_history(&self, settings: HistorySettings) -> Result<(), String> {
        let min_time = now_secs() - i64::from(settings.max_age_days) * 86_400;
        self.with_storage(|s| {
            let excess = s.history.len().saturating_sub(settings.max_records as usize);
            let mut stale = Vec::new();
            for (i, kv) in s.history.iter().enumerate() {
                let (k, v) = kv.map_err(|e| e.to_string())?;
                let too_old = serde_json::from_slice::<HistoryEntry>(&v).map_or(true, |e| e.time < min_time);
                if i < excess || too_old {
                    stale.push(k);
                } else {
                    // Entries are ordered by insertion; the rest are newer.
                    break;
                }
            }
            for k in stale {
                s.history.remove(k).map_err(|e| e.to_string())?;
            }
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    // ---- Session ---------------------------------------------------------

    pub fn store_session(&self, session: &Session) -> Result<(), String> {
        let plain = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        let enc = dpapi::protect(&plain)?;
        self.with_storage(|s| {
            s.settings.insert(SESSION_KEY, enc).map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn clear_session(&self) -> Result<(), String> {
        self.with_storage(|s| {
            s.settings.remove(SESSION_KEY).map_err(|e| e.to_string())?;
            s.db.flush().map_err(|e| e.to_string())?;
            Ok(())
        })
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load_session(settings: &sled::Tree) -> Option<Session> {
    let enc = settings.get(SESSION_KEY).ok().flatten()?;
    let plain = dpapi::unprotect(&enc).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Windows DPAPI (CryptProtectData) bound to the current user account.
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(Some(HLOCAL(out.pbData as _))) };
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| format!("DPAPI encrypt failed: {e}"))?;
            Ok(take(out))
        }
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| format!("DPAPI decrypt failed: {e}"))?;
            Ok(take(out))
        }
    }
}
