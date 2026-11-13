//! Client for the libsearpc named-pipe transport exposed by `seaf-daemon.exe`.
//!
//! Wire format (see libsearpc `searpc-named-pipe-transport.c`):
//!   request  = u32 length (native endian) + JSON `{"service": "...", "request": "[\"fname\", args...]"}`
//!   response = u32 length (native endian) + JSON `{"ret": ...}` or `{"err_code": n, "err_msg": "..."}`
//!
//! On Windows the daemon listens on `\\.\pipe\seafile_<base64(GetUserNameA)>`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::Mutex;

const SERVICE: &str = "seafile-rpcserver";
/// Service of slow calls (e.g. `seafile_diff`) that the daemon runs off its main loop.
const THREADED_SERVICE: &str = "seafile-threaded-rpcserver";
const ERROR_PIPE_BUSY: i32 = 231;
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE: u32 = 64 * 1024 * 1024;

pub struct SeafRpc {
    pipe_name: String,
    conn: Mutex<Option<NamedPipeClient>>,
    /// Only true once the pipe is confirmed to belong to *our* daemon. The pipe
    /// name is per Windows user, so the official Seafile client would answer on
    /// the same pipe; we must never send commands to it.
    enabled: AtomicBool,
}

impl SeafRpc {
    pub fn new() -> Self {
        Self {
            pipe_name: pipe_name(),
            conn: Mutex::new(None),
            enabled: AtomicBool::new(false),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    /// False while our daemon is starting, stopped, or the data folder is being moved.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Drops the cached pipe connection (used when the daemon is stopped).
    pub async fn disconnect(&self) {
        *self.conn.lock().await = None;
    }

    /// Checks whether any daemon answers on the pipe, regardless of `enabled`.
    pub async fn probe(&self) -> bool {
        self.call_raw(SERVICE, "seafile_is_auto_sync_enabled", &[]).await.is_ok()
    }

    async fn connect(&self) -> Result<NamedPipeClient, String> {
        for _ in 0..40 {
            match ClientOptions::new().open(&self.pipe_name) {
                Ok(c) => return Ok(c),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(format!("seaf-daemon is not reachable: {e}")),
            }
        }
        Err("seaf-daemon pipe is busy".into())
    }

    /// Calls an RPC function and returns the `ret` value.
    pub async fn call(&self, fname: &str, args: &[Value]) -> Result<Value, String> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Err("Sync engine (seaf-daemon) is not running".into());
        }
        self.call_raw(SERVICE, fname, args).await
    }

    async fn call_threaded(&self, fname: &str, args: &[Value]) -> Result<Value, String> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Err("Sync engine (seaf-daemon) is not running".into());
        }
        self.call_raw(THREADED_SERVICE, fname, args).await
    }

    async fn call_raw(&self, service: &str, fname: &str, args: &[Value]) -> Result<Value, String> {
        let mut fcall = Vec::with_capacity(args.len() + 1);
        fcall.push(Value::String(fname.to_string()));
        fcall.extend_from_slice(args);
        let request = json!({
            "service": service,
            "request": serde_json::to_string(&fcall).map_err(|e| e.to_string())?,
        })
        .to_string();

        let mut guard = self.conn.lock().await;
        let mut last_err = String::new();
        // One retry with a fresh connection (daemon may have restarted).
        for _ in 0..2 {
            if guard.is_none() {
                *guard = Some(self.connect().await?);
            }
            let pipe = guard.as_mut().unwrap();
            match tokio::time::timeout(CALL_TIMEOUT, exchange(pipe, request.as_bytes())).await {
                Ok(Ok(raw)) => return parse_response(&raw),
                Ok(Err(e)) => last_err = e,
                Err(_) => last_err = format!("RPC {fname} timed out"),
            }
            *guard = None;
        }
        Err(last_err)
    }

    // ---- Typed wrappers ---------------------------------------------------

    pub async fn set_repo_property(&self, repo_id: &str, key: &str, value: &str) -> Result<(), String> {
        let ret = self
            .call("seafile_set_repo_property", &[json!(repo_id), json!(key), json!(value)])
            .await?;
        match ret.as_i64() {
            Some(r) if r < 0 => Err(format!("Library {repo_id} is not known by seaf-daemon yet")),
            _ => Ok(()),
        }
    }

    pub async fn get_repo_property(&self, repo_id: &str, key: &str) -> Result<Option<String>, String> {
        let ret = self
            .call("seafile_get_repo_property", &[json!(repo_id), json!(key)])
            .await?;
        Ok(ret.as_str().map(str::to_string))
    }

    /// Disables auto-sync for a repo. The daemon also cancels any running sync task.
    pub async fn pause_repo(&self, repo_id: &str) -> Result<(), String> {
        self.set_repo_property(repo_id, "auto-sync", "false").await
    }

    pub async fn resume_repo(&self, repo_id: &str) -> Result<(), String> {
        self.set_repo_property(repo_id, "auto-sync", "true").await
    }

    /// One-off sync, works even while auto-sync is disabled for the repo.
    pub async fn sync_now(&self, repo_id: &str) -> Result<(), String> {
        self.call("seafile_sync", &[json!(repo_id), Value::Null]).await.map(|_| ())
    }

    pub async fn get_repo(&self, repo_id: &str) -> Result<Option<Value>, String> {
        let ret = self.call("seafile_get_repo", &[json!(repo_id)]).await?;
        Ok((!ret.is_null()).then_some(ret))
    }

    pub async fn get_repo_list(&self) -> Result<Vec<Value>, String> {
        let ret = self.call("seafile_get_repo_list", &[json!(-1), json!(-1)]).await?;
        Ok(match ret {
            Value::Array(a) => a,
            _ => Vec::new(),
        })
    }

    /// Per-path status used by Explorer overlay icons, e.g. "synced", "syncing", "error".
    pub async fn get_path_sync_status(&self, repo_id: &str, path_in_repo: &str, is_dir: bool) -> Result<String, String> {
        let ret = self
            .call(
                "seafile_get_path_sync_status",
                &[json!(repo_id), json!(path_in_repo), json!(if is_dir { 1 } else { 0 })],
            )
            .await?;
        Ok(ret.as_str().unwrap_or_default().to_string())
    }

    pub async fn get_repo_sync_task(&self, repo_id: &str) -> Result<Option<Value>, String> {
        let ret = self.call("seafile_get_repo_sync_task", &[json!(repo_id)]).await?;
        Ok((!ret.is_null()).then_some(ret))
    }

    /// Running HTTP transfer of a repo: `ttype` ("upload"/"download"), `rt_state`,
    /// `block_total`, `block_done`, `rate` (bytes/s). Progress fields are only set
    /// while blocks are being transferred.
    pub async fn find_transfer_task(&self, repo_id: &str) -> Result<Option<Value>, String> {
        let ret = self.call("seafile_find_transfer_task", &[json!(repo_id)]).await?;
        Ok((!ret.is_null()).then_some(ret))
    }

    /// Changes between two commits; an empty `old_commit` means "parent of
    /// `new_commit`". Objects carry `status` (add, del, mod, mov, newdir, deldir),
    /// `name` (path inside the repo) and `new_name` (for renames).
    pub async fn diff(&self, repo_id: &str, old_commit: &str, new_commit: &str, fold_dirs: bool) -> Result<Vec<Value>, String> {
        let ret = self
            .call_threaded(
                "seafile_diff",
                &[json!(repo_id), json!(old_commit), json!(new_commit), json!(if fold_dirs { 1 } else { 0 })],
            )
            .await?;
        Ok(match ret {
            Value::Array(a) => a,
            _ => Vec::new(),
        })
    }

    pub async fn get_clone_tasks(&self) -> Result<Vec<Value>, String> {
        let ret = self.call("seafile_get_clone_tasks", &[]).await?;
        Ok(match ret {
            Value::Array(a) => a,
            _ => Vec::new(),
        })
    }

    /// Current total upload rate in bytes/s.
    pub async fn get_upload_rate(&self) -> Result<u64, String> {
        let ret = self.call("seafile_get_upload_rate", &[]).await?;
        Ok(ret.as_i64().unwrap_or(0).max(0) as u64)
    }

    /// Current total download rate in bytes/s.
    pub async fn get_download_rate(&self) -> Result<u64, String> {
        let ret = self.call("seafile_get_download_rate", &[]).await?;
        Ok(ret.as_i64().unwrap_or(0).max(0) as u64)
    }

    /// File- and repo-level sync errors recorded by the daemon (newest first).
    /// Objects carry `id`, `repo_id`, `repo_name`, `path`, `err_id`, `timestamp`.
    pub async fn get_file_sync_errors(&self, offset: i64, limit: i64) -> Result<Vec<Value>, String> {
        let ret = self
            .call("seafile_get_file_sync_errors", &[json!(offset), json!(limit)])
            .await?;
        Ok(match ret {
            Value::Array(a) => a,
            _ => Vec::new(),
        })
    }

    /// Pops one message `{"type": ..., "content": ...}` from the daemon's
    /// notification queue, or `None` when the queue is empty.
    pub async fn pop_sync_notification(&self) -> Result<Option<Value>, String> {
        let ret = self.call("seafile_get_sync_notification", &[]).await?;
        Ok(match ret {
            Value::Null => None,
            // Some transports deliver json results as an encoded string.
            Value::String(s) => serde_json::from_str(&s).ok(),
            v => Some(v),
        })
    }

    pub async fn sync_error_to_str(&self, error_id: i64) -> String {
        match self.call("seafile_sync_error_id_to_str", &[json!(error_id)]).await {
            Ok(Value::String(s)) => s,
            _ => format!("sync error {error_id}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn clone_repo(
        &self,
        repo_id: &str,
        repo_version: i64,
        repo_name: &str,
        worktree: &str,
        token: &str,
        magic: Option<&str>,
        email: &str,
        random_key: Option<&str>,
        enc_version: i64,
        more_info: &str,
    ) -> Result<(), String> {
        self.call(
            "seafile_clone",
            &[
                json!(repo_id),
                json!(repo_version),
                json!(repo_name),
                json!(worktree),
                json!(token),
                Value::Null, // passwd (encrypted libraries are not supported)
                json!(magic),
                json!(email),
                json!(random_key),
                json!(enc_version),
                json!(more_info),
            ],
        )
        .await
        .map(|_| ())
    }

    pub async fn cancel_clone_task(&self, repo_id: &str) -> Result<(), String> {
        self.call("seafile_cancel_clone_task", &[json!(repo_id)]).await.map(|_| ())
    }

    pub async fn destroy_repo(&self, repo_id: &str) -> Result<(), String> {
        self.call("seafile_destroy_repo", &[json!(repo_id)]).await.map(|_| ())
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        self.call("seafile_shutdown", &[]).await.map(|_| ())
    }
}

/// Reads a string field from a searpc-serialized GObject. Property names may
/// arrive as `repo-id` (GObject canonical form) or `repo_id`.
pub fn obj_field<'a>(obj: &'a Value, name: &str) -> Option<&'a Value> {
    obj.get(name)
        .or_else(|| obj.get(name.replace('_', "-")))
        .or_else(|| obj.get(name.replace('-', "_")))
}

async fn exchange(pipe: &mut NamedPipeClient, body: &[u8]) -> Result<Vec<u8>, String> {
    let io = |e: std::io::Error| format!("seaf-daemon pipe error: {e}");
    pipe.write_all(&(body.len() as u32).to_ne_bytes()).await.map_err(io)?;
    pipe.write_all(body).await.map_err(io)?;
    pipe.flush().await.map_err(io)?;

    let mut len_buf = [0u8; 4];
    pipe.read_exact(&mut len_buf).await.map_err(io)?;
    let len = u32::from_ne_bytes(len_buf);
    if len > MAX_RESPONSE {
        return Err(format!("RPC response too large ({len} bytes)"));
    }
    let mut buf = vec![0u8; len as usize];
    pipe.read_exact(&mut buf).await.map_err(io)?;
    Ok(buf)
}

fn parse_response(raw: &[u8]) -> Result<Value, String> {
    // The C side may include a trailing NUL.
    let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
    let v: Value = serde_json::from_slice(raw).map_err(|e| format!("Invalid RPC response: {e}"))?;
    if let Some(code) = v.get("err_code") {
        let msg = v.get("err_msg").and_then(Value::as_str).unwrap_or("unknown error");
        return Err(format!("seaf-daemon error {code}: {msg}"));
    }
    Ok(v.get("ret").cloned().unwrap_or(Value::Null))
}

fn pipe_name() -> String {
    format!(r"\\.\pipe\seafile_{}", user_pipe_suffix())
}

/// base64(GetUserNameA()), the per-user suffix Seafile appends to its pipe names.
pub fn user_pipe_suffix() -> String {
    use windows::Win32::System::WindowsProgramming::GetUserNameA;
    use windows::core::PSTR;

    let mut buf = vec![0u8; 512];
    let mut size = buf.len() as u32;
    let user = unsafe { GetUserNameA(Some(PSTR(buf.as_mut_ptr())), &mut size) }
        .ok()
        .map(|_| {
            // `size` includes the terminating NUL.
            buf.truncate(size.saturating_sub(1) as usize);
            buf
        })
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_default().into_bytes());
    base64::engine::general_purpose::STANDARD.encode(user)
}
