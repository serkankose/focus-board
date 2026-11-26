//! Which sessions are alive (owner, 2026-11-26, B45 "b"), so the page can show whom a review
//! press can wake and whom the owner can still type to.
//!
//! - **Waiting**: every `focus-board wait` announces itself in `<waits>/<pid>.json` with its
//!   session and agent, and removes the file when it ends. A file whose process is gone (a wait
//!   killed hard) is skipped and removed.
//! - **Running**: Claude Code lists its running sessions in `~/.claude/sessions/<pid>.json`; one
//!   whose process is alive is running. Copilot and opencode keep no such list.
//!
//! Nothing reads another program's environment: it can hold secrets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::process_alive;

/// Where waits announce themselves: `FOCUS_BOARD_WAITS`, else `focus-board/waits` in
/// `$XDG_STATE_HOME` or `~/.local/state`.
pub fn waits_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FOCUS_BOARD_WAITS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::home_dir().map(|h| h.join(".local").join("state")))?;
    Some(state.join("focus-board").join("waits"))
}

/// A wait's announcement; dropping it removes the file.
pub struct WaitMark(PathBuf);

impl Drop for WaitMark {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Announces that this process waits for `session_id`'s (and `agent_id`'s) news. `None` when it
/// cannot be written; the wait works the same, only the page cannot show it.
pub fn announce_wait(dir: &Path, session_id: &str, agent_id: &str) -> Option<WaitMark> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{}.json", std::process::id()));
    let body = json!({ "session_id": session_id, "agent_id": agent_id });
    std::fs::write(&path, body.to_string()).ok()?;
    Some(WaitMark(path))
}

/// The `(session id, agent id)` of every wait still running. A file whose process is gone is
/// removed.
pub fn waiting(dir: &Path) -> HashSet<(String, String)> {
    let mut out = HashSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(pid) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if !process_alive(pid) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Ok(value) = std::fs::read_to_string(&path)
            .map_err(|_| ())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ()))
        else {
            continue;
        };
        let text = |k: &str| {
            value
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        out.insert((text("session_id"), text("agent_id")));
    }
    out
}

/// The ids of Claude Code sessions whose process is alive, from its running-session list (only
/// the `.json` files; the `.key` files beside them hold secrets).
pub fn running_claude(registry: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(entries) = std::fs::read_dir(registry) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(value) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let session = value.get("sessionId").and_then(Value::as_str).unwrap_or("");
        let pid = value.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32;
        if !session.is_empty() && pid > 0 && process_alive(pid) {
            out.insert(session.to_string());
        }
    }
    out
}
