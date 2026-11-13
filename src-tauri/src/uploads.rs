//! Upload queue shown in the "Uploading" tab.
//!
//! seaf-daemon has no per-file transfer API. What it does expose per library:
//! - the sync task state (`committing` while it indexes local changes,
//!   `uploading` while the new commit is sent),
//! - the HTTP transfer task with overall progress and rate,
//! - the local head commit, whose diff against its parent lists the files
//!   contained in the upload.
//!
//! So progress and speed are per library, and the file list is the content of
//! the commit being uploaded.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use crate::seaf_rpc::obj_field;
use crate::state::{AppState, LibrarySyncConfig};

pub const EVT_UPLOADS: &str = "uploads-updated";

/// Files sent to the UI per library; `file_count` keeps the real total.
const MAX_FILES: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UploadFile {
    /// add, mod, del, mov, newdir, deldir
    pub status: String,
    /// Path inside the library (new path for renames).
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum UploadPhase {
    /// The daemon is indexing local changes; the file list is not known yet.
    Preparing,
    Uploading,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UploadTask {
    pub repo_id: String,
    pub repo_name: String,
    pub phase: UploadPhase,
    /// Transfer step reported by the daemon (`fs`, `data`, `commit`, ...), empty if unknown.
    pub step: String,
    /// Progress in blocks or bytes (depends on the daemon's upload mode); 0/0 when unknown.
    pub done: u64,
    pub total: u64,
    /// Bytes per second.
    pub rate: u64,
    pub files: Vec<UploadFile>,
    pub file_count: usize,
}

/// (commit id, files, total count)
type CommitFiles = (String, Vec<UploadFile>, usize);

#[derive(Default)]
pub struct UploadQueue {
    current: Mutex<Vec<UploadTask>>,
    /// repo_id -> files of the commit being uploaded; the diff is computed once per commit.
    files: Mutex<HashMap<String, CommitFiles>>,
}

impl UploadQueue {
    pub fn snapshot(&self) -> Vec<UploadTask> {
        self.current.lock().unwrap().clone()
    }

    /// Replaces the queue; returns true if it changed.
    fn replace(&self, next: Vec<UploadTask>) -> bool {
        let mut cur = self.current.lock().unwrap();
        if *cur == next {
            return false;
        }
        *cur = next;
        true
    }
}

/// Rebuilds the queue from the libraries that are currently syncing.
/// Returns true when the queue changed.
pub async fn refresh(state: &AppState, syncing: &[LibrarySyncConfig]) -> bool {
    let mut tasks = Vec::new();
    for lib in syncing {
        if let Some(t) = upload_task(state, lib).await {
            tasks.push(t);
        }
    }
    {
        let active: Vec<&str> = tasks.iter().map(|t| t.repo_id.as_str()).collect();
        state.uploads.files.lock().unwrap().retain(|id, _| active.contains(&id.as_str()));
    }
    state.uploads.replace(tasks)
}

async fn upload_task(state: &AppState, lib: &LibrarySyncConfig) -> Option<UploadTask> {
    let rpc = &state.rpc;
    let sync = rpc.get_repo_sync_task(&lib.repo_id).await.ok()??;
    let phase = match obj_field(&sync, "state").and_then(Value::as_str) {
        Some("committing") => UploadPhase::Preparing,
        Some("uploading") => UploadPhase::Uploading,
        _ => return None,
    };
    let mut task = UploadTask {
        repo_id: lib.repo_id.clone(),
        repo_name: lib.repo_name.clone(),
        phase,
        step: String::new(),
        done: 0,
        total: 0,
        rate: 0,
        files: Vec::new(),
        file_count: 0,
    };
    if phase == UploadPhase::Preparing {
        return Some(task);
    }

    if let Ok(Some(tx)) = rpc.find_transfer_task(&lib.repo_id).await
        && obj_field(&tx, "ttype").and_then(Value::as_str) == Some("upload")
    {
        let num = |k: &str| obj_field(&tx, k).and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
        task.step = obj_field(&tx, "rt_state").and_then(Value::as_str).unwrap_or("").to_string();
        task.done = num("block_done");
        task.total = num("block_total");
        task.rate = num("rate");
    }

    let head = rpc
        .get_repo(&lib.repo_id)
        .await
        .ok()
        .flatten()
        .and_then(|r| obj_field(&r, "head_cmmt_id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    if !head.is_empty() {
        let cached = state.uploads.files.lock().unwrap().get(&lib.repo_id).cloned();
        let (files, count) = match cached {
            Some((commit, files, count)) if commit == head => (files, count),
            _ => {
                let entries = rpc.diff(&lib.repo_id, "", &head, false).await.unwrap_or_default();
                let count = entries.len();
                let files: Vec<UploadFile> = entries.iter().take(MAX_FILES).filter_map(diff_file).collect();
                state
                    .uploads
                    .files
                    .lock()
                    .unwrap()
                    .insert(lib.repo_id.clone(), (head, files.clone(), count));
                (files, count)
            }
        };
        task.files = files;
        task.file_count = count;
    }
    Some(task)
}

/// Path of a diff entry (new name for renames) with its status.
pub fn diff_file(entry: &Value) -> Option<UploadFile> {
    let status = obj_field(entry, "status").and_then(Value::as_str).unwrap_or("").to_string();
    let field = if status == "mov" { "new_name" } else { "name" };
    let path = obj_field(entry, field)
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())?
        .trim_start_matches('/')
        .to_string();
    Some(UploadFile { status, path })
}
