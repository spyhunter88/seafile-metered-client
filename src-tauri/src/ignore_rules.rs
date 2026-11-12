//! Ignore rules for temporary files, written into each library's
//! `seafile-ignore.txt` (read by seaf-daemon on every scan).
//!
//! Only a marked block is managed; lines the user wrote outside it are kept.
//! Note: `seafile-ignore.txt` is a normal file of the library, so it is synced
//! to the server and applies to every client of that library.
//!
//! Pattern syntax (see seaf-daemon `seaf_repo_load_ignore_files`): each line
//! becomes the glob `<worktree>/<line>`, where `*` and `?` also match `/`.
//! Basename rules are therefore written twice: `P` (library root) and `*/P`
//! (any subfolder).

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::seaf_rpc::obj_field;
use crate::state::{AppState, LibrarySyncConfig};

pub const IGNORE_FILE: &str = "seafile-ignore.txt";
const BEGIN: &str = "# >>> Seafile Metered Client: temporary files (managed block, edits inside are overwritten) >>>";
const END: &str = "# <<< Seafile Metered Client <<<";

#[derive(Serialize)]
pub struct PatternGroup {
    pub name: &'static str,
    pub patterns: &'static [&'static str],
}

/// File-name patterns of temporary/lock files.
pub const TEMP_PATTERNS: &[PatternGroup] = &[
    PatternGroup { name: "Microsoft Office (Word, Excel, PowerPoint, Visio)", patterns: &["~$*", "*.tmp"] },
    PatternGroup { name: "LibreOffice / OpenOffice", patterns: &[".~lock.*#"] },
    PatternGroup { name: "WPS Office", patterns: &[".~*"] },
    PatternGroup { name: "draw.io", patterns: &[".$*.bkp", ".$*.dtmp"] },
    PatternGroup { name: "AutoCAD", patterns: &["*.dwl", "*.dwl2", "*.sv$"] },
    PatternGroup { name: "Unfinished browser downloads", patterns: &["*.crdownload", "*.part", "*.partial"] },
    PatternGroup { name: "Editor swap files", patterns: &[".*.swp", ".*.swo", "*~"] },
    PatternGroup { name: "Windows / macOS system files", patterns: &["desktop.ini", "Thumbs.db", "ehthumbs.db", ".DS_Store"] },
];

fn managed_block(nl: &str) -> String {
    let mut out = vec![BEGIN.to_string()];
    for g in TEMP_PATTERNS {
        out.push(format!("# {}", g.name));
        for p in g.patterns {
            out.push((*p).to_string());
            out.push(format!("*/{p}"));
        }
    }
    out.push(END.to_string());
    out.join(nl)
}

/// Adds, refreshes or removes the managed block. Returns true if the file changed.
pub fn apply_to_worktree(worktree: &Path, enabled: bool) -> Result<bool, String> {
    let path = worktree.join(IGNORE_FILE);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    let nl = if existing.contains("\r\n") { "\r\n" } else { "\n" };

    // User lines = everything outside our block.
    let mut user_lines = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        match line.trim() {
            l if l == BEGIN => inside = true,
            l if l == END => inside = false,
            _ if !inside => user_lines.push(line),
            _ => {}
        }
    }
    let user = user_lines.join(nl);
    let user = user.trim_end();

    let new = match (enabled, user.is_empty()) {
        (true, true) => format!("{}{nl}", managed_block(nl)),
        (true, false) => format!("{user}{nl}{nl}{}{nl}", managed_block(nl)),
        (false, true) => String::new(),
        (false, false) => format!("{user}{nl}"),
    };
    if new == existing {
        return Ok(false);
    }
    if new.is_empty() {
        // The file only contained our block.
        std::fs::remove_file(&path).map_err(|e| format!("Cannot remove {}: {e}", path.display()))?;
    } else {
        std::fs::write(&path, new).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    }
    Ok(true)
}

/// True once the initial download finished and the daemon knows the repo,
/// so writing into the folder cannot clash with files being checked out.
async fn ready_for_rules(state: &AppState, lib: &LibrarySyncConfig, clone_tasks: &[Value]) -> bool {
    let cloning = clone_tasks.iter().any(|t| {
        obj_field(t, "repo_id").and_then(Value::as_str) == Some(lib.repo_id.as_str())
            && obj_field(t, "state").and_then(Value::as_str) != Some("done")
    });
    !cloning && Path::new(&lib.local_path).is_dir() && matches!(state.rpc.get_repo(&lib.repo_id).await, Ok(Some(_)))
}

/// Applies the current preference to every ready library not handled yet in
/// this session. Returns the errors (library name: message).
pub async fn sync_pending(state: &AppState, clone_tasks: &[Value]) -> Vec<String> {
    let enabled = state.preferences().ignore_temp_files;
    let done: HashSet<String> = state.ignore_applied.lock().unwrap().clone();
    let mut errors = Vec::new();
    for lib in state.get_all_libraries() {
        if done.contains(&lib.repo_id) || !ready_for_rules(state, &lib, clone_tasks).await {
            continue;
        }
        match apply_to_worktree(Path::new(&lib.local_path), enabled) {
            Ok(changed) => {
                if changed {
                    eprintln!("[ignore] {} rules for {}", if enabled { "added" } else { "removed" }, lib.repo_name);
                }
                state.ignore_applied.lock().unwrap().insert(lib.repo_id.clone());
            }
            // Not marked as done: retried on the next poll.
            Err(e) => errors.push(format!("{}: {e}", lib.repo_name)),
        }
    }
    errors
}

