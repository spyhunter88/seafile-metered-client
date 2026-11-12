//! Lifecycle of the `seaf-daemon.exe` sidecar.
//!
//! The daemon is spawned directly from the bundled `binaries/` resource folder
//! (instead of Tauri's `externalBin`) so that its DLLs sit next to the exe and
//! are found by the Windows loader.

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const CONSOLE_LOG: &str = "seaf-daemon-console.log";
const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;

/// Spawns our daemon and waits until it answers RPC calls. RPC is only enabled
/// when the pipe is known to belong to the process we spawned.
pub async fn start(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.rpc.probe().await {
        return Err("Another Seafile client is already running for this Windows user \
                    (its seaf-daemon owns the RPC pipe). Quit the official Seafile client \
                    and restart this app."
            .into());
    }
    spawn(app, state)?;
    wait_ready(state).await?;
    state.rpc.set_enabled(true);
    Ok(())
}

fn spawn(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let paths = &state.paths();
    // seaf-daemon does not create its log folder and exits with code 1 without it.
    let logs_dir = paths.ccnet_dir.join("logs");
    for dir in [&logs_dir, &paths.seafile_data_dir, &paths.worktree_dir] {
        std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    // Messages printed before the daemon's own log is ready go to this file.
    let console_log = std::fs::File::create(logs_dir.join(CONSOLE_LOG))
        .map_err(|e| format!("Cannot create daemon console log: {e}"))?;
    let console_err = console_log.try_clone().map_err(|e| e.to_string())?;

    let bin_dir = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("binaries");
    let exe = bin_dir.join("seaf-daemon.exe");
    if !exe.exists() {
        return Err(format!("seaf-daemon.exe not found at {}", exe.display()));
    }

    let child = Command::new(&exe)
        .current_dir(&bin_dir)
        .arg("-c")
        .arg(&paths.ccnet_dir)
        .arg("-d")
        .arg(&paths.seafile_data_dir)
        .arg("-w")
        .arg(&paths.worktree_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(console_log))
        .stderr(Stdio::from(console_err))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to spawn seaf-daemon: {e}"))?;

    if let Err(e) = bind_to_app_lifetime(&child) {
        eprintln!("[daemon] could not attach to job object: {e}");
    }
    *state.daemon.lock().unwrap() = Some(child);
    Ok(())
}

/// Puts the daemon in a Job Object with KILL_ON_JOB_CLOSE. The job handle is
/// never closed explicitly; when this app exits or crashes, Windows closes it
/// and kills seaf-daemon, so no orphan keeps the RPC pipe busy.
fn bind_to_app_lifetime(child: &std::process::Child) -> windows::core::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    // Stored as usize because HANDLE is not Send/Sync.
    static JOB: OnceLock<usize> = OnceLock::new();

    let job = match JOB.get() {
        Some(h) => HANDLE(*h as _),
        None => unsafe {
            let job = CreateJobObjectW(None, windows::core::PCWSTR::null())?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            let _ = JOB.set(job.0 as usize);
            job
        },
    };
    unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as _)) }
}

/// Waits until the daemon answers RPC calls, or fails if the process exits.
async fn wait_ready(state: &AppState) -> Result<(), String> {
    for _ in 0..100 {
        // Check the process first so a foreign daemon on the same pipe is never mistaken for ours.
        let exited = state
            .daemon
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten());
        if let Some(status) = exited {
            if status.code() == Some(STATUS_DLL_NOT_FOUND) {
                return Err("seaf-daemon.exe could not start: a required DLL is missing from \
                            src-tauri/binaries (expected next to seaf-daemon.exe)."
                    .into());
            }
            let logs = state.paths().ccnet_dir.join("logs");
            let console = std::fs::read_to_string(logs.join(CONSOLE_LOG)).unwrap_or_default();
            let last = console.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            return Err(format!(
                "seaf-daemon exited during startup ({status}){}. Logs: {}",
                if last.is_empty() { String::new() } else { format!(": {last}") },
                logs.display()
            ));
        }
        if state.rpc.probe().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err("seaf-daemon did not become ready within 30s".into())
}

/// Graceful shutdown via RPC, then a hard kill so no orphan process remains.
/// RPC is disabled afterwards until `start` succeeds again.
pub async fn stop_async(state: &AppState) {
    let child = state.daemon.lock().unwrap().take();
    if child.is_some() {
        let _ = tokio::time::timeout(Duration::from_secs(2), state.rpc.shutdown()).await;
    }
    state.rpc.set_enabled(false);
    state.rpc.disconnect().await;

    let Some(mut child) = child else { return };
    for _ in 0..30 {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Blocking variant for the app exit path (not inside the async runtime).
pub fn stop(state: &AppState) {
    tauri::async_runtime::block_on(stop_async(state));
}
