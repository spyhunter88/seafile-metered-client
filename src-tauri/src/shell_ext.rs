//! Server side of the pipe used by the official Seafile Explorer extension
//! (`seafile_ext64.dll`, installed with Seafile Desktop) to draw overlay icons.
//!
//! We take the place of `seafile-applet.exe` (see seafile-client
//! `src/ext-handler.cpp`). Protocol on `\\.\pipe\seafile_ext_pipe_<base64(user)>`,
//! message-mode pipe:
//!   request  = message(u32 len) + message(UTF-8 "cmd\targ1\targ2...")
//!   response = message(u32 len) + message(UTF-8 body), body omitted when len == 0
//!
//! Supported commands:
//!   list-repos <ts> [v2]               -> one line per repo, tab separated fields
//!   get-file-status <repo> <path> <isdir> -> daemon path status ("synced", "syncing", ...)
//!   show-history <path>                -> opens the file history page in the browser
//! Other menu commands (share link, lock, private share...) get an empty reply.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::Mutex;

use crate::seaf_rpc::{obj_field, user_pipe_suffix};
use crate::state::AppState;
use crate::sync_controller::ACTIVE_SYNC_STATES;

const REPOS_CACHE_TTL: Duration = Duration::from_secs(2);
const MAX_REQUEST: u32 = 1024 * 1024;

#[derive(Clone)]
struct RepoInfo {
    id: String,
    name: String,
    /// Forward slashes, no trailing slash (as the extension expects).
    worktree: String,
    status: &'static str,
}

#[derive(Default)]
struct ReposCache {
    entry: Mutex<Option<(Instant, Vec<RepoInfo>)>>,
}

impl ReposCache {
    async fn get(&self, state: &AppState) -> Vec<RepoInfo> {
        let mut guard = self.entry.lock().await;
        if let Some((at, repos)) = guard.as_ref()
            && at.elapsed() < REPOS_CACHE_TTL
        {
            return repos.clone();
        }
        let repos = fetch_repos(state).await;
        *guard = Some((Instant::now(), repos.clone()));
        repos
    }
}

fn normalized_path(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Same rules as seafile-client `repoStatus()`.
async fn fetch_repos(state: &AppState) -> Vec<RepoInfo> {
    let Ok(list) = state.rpc.get_repo_list().await else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for repo in list {
        let s = |k: &str| obj_field(&repo, k).and_then(Value::as_str).unwrap_or_default().to_string();
        let id = s("id");
        let worktree = s("worktree");
        let invalid = obj_field(&repo, "worktree-invalid").and_then(Value::as_bool).unwrap_or(false);
        if id.is_empty() || worktree.is_empty() || invalid {
            continue;
        }
        let auto_sync = obj_field(&repo, "auto-sync").and_then(Value::as_bool).unwrap_or(true);
        let status = if !auto_sync {
            "paused"
        } else {
            match state.rpc.get_repo_sync_task(&id).await {
                Ok(Some(task)) => match obj_field(&task, "state").and_then(Value::as_str) {
                    Some(st) if ACTIVE_SYNC_STATES.contains(&st) => "syncing",
                    Some("error") => "error",
                    _ => "normal",
                },
                _ => "normal",
            }
        };
        out.push(RepoInfo {
            id,
            name: s("name"),
            worktree: normalized_path(&worktree),
            status,
        });
    }
    out
}

/// Finds the repo containing `path` and returns it with the path inside the repo ("/a/b.txt").
fn locate<'a>(repos: &'a [RepoInfo], path: &str) -> Option<(&'a RepoInfo, String)> {
    let path = normalized_path(path);
    repos.iter().find_map(|r| {
        let rest = path.strip_prefix(&r.worktree)?;
        rest.starts_with('/').then(|| (r, rest.to_string()))
    })
}

async fn handle_command(app: &AppHandle, state: &AppState, cache: &ReposCache, args: &[&str]) -> String {
    let Some((&cmd, args)) = args.split_first() else {
        return String::new();
    };
    match cmd {
        "list-repos" => {
            // Newer extensions send a second argument and expect a 7th field.
            let new_version = args.len() > 1;
            let lines: Vec<String> = cache
                .get(state)
                .await
                .iter()
                .map(|r| {
                    let mut fields = vec![
                        r.id.as_str(),
                        r.name.as_str(),
                        r.worktree.as_str(),
                        r.status,
                        "file-lock-unsupported",
                        "private-share-unsupported",
                    ];
                    if new_version {
                        fields.push("internal-link-unsupported");
                    }
                    fields.join("\t")
                })
                .collect();
            lines.join("\n")
        }
        "get-file-status" => {
            let [repo_id, path_in_repo, is_dir] = args else {
                return String::new();
            };
            if repo_id.len() != 36 {
                return String::new();
            }
            state
                .rpc
                .get_path_sync_status(repo_id, path_in_repo, *is_dir == "true")
                .await
                .unwrap_or_default()
        }
        "show-history" => {
            if let [path] = args {
                open_history(app, state, cache, path).await;
            }
            String::new()
        }
        _ => String::new(),
    }
}

async fn open_history(app: &AppHandle, state: &AppState, cache: &ReposCache, path: &str) {
    if std::path::Path::new(path).is_dir() {
        return;
    }
    let repos = cache.get(state).await;
    let Some((repo, path_in_repo)) = locate(&repos, path) else { return };
    let Some(server) = state.session.read().await.as_ref().map(|s| s.server_url.clone()) else {
        return;
    };
    let Ok(mut url) = reqwest::Url::parse(&format!("{server}/repo/file_revisions/{}/", repo.id)) else {
        return;
    };
    url.query_pairs_mut().append_pair("p", &path_in_repo);
    let _ = app.opener().open_url(url.as_str(), None::<&str>);
}

async fn read_request(pipe: &mut NamedPipeServer) -> Option<String> {
    let mut len_buf = [0u8; 4];
    pipe.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_ne_bytes(len_buf);
    if len == 0 || len > MAX_REQUEST {
        return None;
    }
    let mut body = vec![0u8; len as usize];
    pipe.read_exact(&mut body).await.ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Length and body go out as two separate pipe messages: the extension reads
/// them with two exact-size reads in message mode.
async fn write_response(pipe: &mut NamedPipeServer, resp: &str) -> std::io::Result<()> {
    let bytes = resp.as_bytes();
    pipe.write_all(&(bytes.len() as u32).to_ne_bytes()).await?;
    if !bytes.is_empty() {
        pipe.write_all(bytes).await?;
    }
    Ok(())
}

async fn serve_client(mut pipe: NamedPipeServer, app: AppHandle, state: Arc<AppState>, cache: Arc<ReposCache>) {
    while let Some(request) = read_request(&mut pipe).await {
        let args: Vec<&str> = request.split('\t').collect();
        let resp = handle_command(&app, &state, &cache, &args).await;
        if write_response(&mut pipe, &resp).await.is_err() {
            break;
        }
    }
    let _ = pipe.disconnect();
}

fn new_instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(first)
        .pipe_mode(PipeMode::Message)
        .reject_remote_clients(true)
        .in_buffer_size(1024)
        .out_buffer_size(1024)
        .create(name)
}

async fn serve(app: AppHandle, state: Arc<AppState>) -> Result<(), String> {
    let name = format!(r"\\.\pipe\seafile_ext_pipe_{}", user_pipe_suffix());
    let mut server = new_instance(&name, true).map_err(|e| {
        format!("Cannot own the Explorer extension pipe ({e}). Is Seafile Desktop still running?")
    })?;
    let cache = Arc::new(ReposCache::default());
    loop {
        server.connect().await.map_err(|e| e.to_string())?;
        let next = new_instance(&name, false).map_err(|e| e.to_string())?;
        let client = std::mem::replace(&mut server, next);
        tauri::async_runtime::spawn(serve_client(client, app.clone(), state.clone(), cache.clone()));
    }
}

/// Starts the pipe server in the background. Returns immediately.
pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = serve(app, state).await {
            eprintln!("[shell-ext] {e}");
        }
    });
}

/// Asks Explorer to redraw overlay icons (used after pause/resume and on exit).
pub fn refresh_explorer() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}
