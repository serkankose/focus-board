//! Following sessions' renames (owner, 2026-11-21: "when a session is renamed the board should be
//! updated").
//!
//! A session learns its name from its transcript when it writes (see
//! [`crate::identity::session_name`]). A rename between writes would wait for the next write, so
//! the server also watches each session's Claude Code transcript. It reads a transcript in full
//! once, then only the bytes added since, so a long session costs little. A new `/rename` title, or
//! a new label, becomes the session's name on the board within one watch period.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::identity::{Names, clean_name, is_plain_id, scan_names};

/// Where Claude Code keeps transcripts: `FOCUS_BOARD_PROJECTS`, else `~/.claude/projects`.
pub fn projects_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FOCUS_BOARD_PROJECTS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// Claude Code's live session registry: `FOCUS_BOARD_SESSIONS`, else `~/.claude/sessions`.
pub fn registry_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FOCUS_BOARD_SESSIONS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::home_dir().map(|h| h.join(".claude").join("sessions"))
}

/// Where GitHub Copilot keeps its sessions: `FOCUS_BOARD_COPILOT`, else `~/.copilot/session-state`.
pub fn copilot_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FOCUS_BOARD_COPILOT").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::home_dir().map(|h| h.join(".copilot").join("session-state"))
}

/// A Copilot session's name (owner, 2026-11-22, #353), from `<dir>/<session id>/workspace.yaml`:
/// `user` when the owner gave it with `/rename` (`user_named: true`), else `auto`. Only a plain
/// session id is looked up, so an id can never lead out of the folder.
pub fn copilot_name(dir: &Path, session_id: &str) -> Option<Registered> {
    if !is_plain_id(session_id) || session_id.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(session_id).join("workspace.yaml")).ok()?;
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
            .map(|v| yaml_text(v.trim()))
    };
    let name = clean_name(&field("name")?);
    if name.is_empty() {
        return None;
    }
    let source = if field("user_named").as_deref() == Some("true") {
        "user"
    } else {
        "auto"
    };
    Some(Registered {
        name,
        source: source.into(),
        since: 0,
    })
}

/// A one-line YAML scalar as text: 'single' (with '' for '), "double" (JSON escapes), or plain.
fn yaml_text(v: &str) -> String {
    if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    if v.starts_with('"') && v.ends_with('"') && v.len() > 1 {
        return serde_json::from_str(v).unwrap_or_else(|_| v[1..v.len() - 1].to_string());
    }
    v.to_string()
}

/// A running session's name in the registry, and how it came: `user` (the owner's `/rename`),
/// `auto` (Claude's label from the talk) or `derived` (the folder and a short id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub name: String,
    pub source: String,
    since: i64,
}

/// Every session the registry names, by session id: `<pid>.json` files with `sessionId`,
/// `name`, `nameSource` and `nameSince`. Only `.json` files are read; the `.key` files beside them
/// hold secrets and are never opened. When two files name one session, the newer name wins.
pub fn registered_names(dir: &Path) -> HashMap<String, Registered> {
    let mut out: HashMap<String, Registered> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(d) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (session, name) = (text("sessionId"), clean_name(&text("name")));
        if session.is_empty() || name.is_empty() {
            continue;
        }
        let since = d.get("nameSince").and_then(|v| v.as_i64()).unwrap_or(0);
        let found = Registered {
            name,
            source: text("nameSource"),
            since,
        };
        match out.get(&session) {
            Some(known) if known.since >= since => {}
            _ => {
                out.insert(session, found);
            }
        }
    }
    out
}

/// The name to show (owner, 2026-11-21): the owner's own `/rename` first (the registry's `user`
/// name), then a title in the transcript, then Claude's label (registry `auto`, else the
/// transcript's). Never a `derived` name, which is half a hash. `None` leaves the session its
/// made-up name.
pub fn best_name(registered: Option<&Registered>, names: &Names) -> Option<String> {
    let from = |source: &str| {
        registered
            .filter(|r| r.source == source)
            .map(|r| r.name.clone())
    };
    from("user")
        .or_else(|| names.title.clone())
        .or_else(|| from("auto"))
        .or_else(|| names.label.clone())
}

/// What the watcher knows of one session's transcript.
#[derive(Default)]
struct Watched {
    path: Option<PathBuf>,
    /// Bytes read so far: up to the end of the last whole line.
    offset: u64,
    names: Names,
}

pub struct NameWatcher {
    projects: PathBuf,
    registry: Option<PathBuf>,
    copilot: Option<PathBuf>,
    sessions: HashMap<String, Watched>,
}

impl NameWatcher {
    /// Watches transcripts under `projects` and, when given, the session registry `registry`.
    pub fn new(projects: PathBuf, registry: Option<PathBuf>) -> NameWatcher {
        NameWatcher {
            projects,
            registry,
            copilot: None,
            sessions: HashMap::new(),
        }
    }

    /// Also names Copilot sessions from Copilot's own folder (see [`copilot_name`]).
    pub fn with_copilot(mut self, dir: Option<PathBuf>) -> NameWatcher {
        self.copilot = dir;
        self
    }

    /// The watcher for this machine's Claude Code folders.
    pub fn from_env() -> Option<NameWatcher> {
        Some(NameWatcher::new(projects_dir()?, registry_dir()).with_copilot(copilot_dir()))
    }

    /// The sessions whose name in their transcript differs from the board's, as
    /// `(session id, name)`. `sessions` holds each session's id and its name on the board.
    pub fn changed(&mut self, sessions: &[(String, String)]) -> Vec<(String, String)> {
        let registered = self
            .registry
            .as_deref()
            .map(registered_names)
            .unwrap_or_default();
        let mut out = Vec::new();
        for (session_id, on_board) in sessions {
            let watched = self.sessions.entry(session_id.clone()).or_default();
            if watched.path.is_none() {
                watched.path = find_transcript(&self.projects, session_id);
            }
            if let Some(path) = watched.path.clone()
                && let Ok(len) = path.metadata().map(|m| m.len())
            {
                if len < watched.offset {
                    // Shorter than what was read: rewritten; read it again from the start.
                    *watched = Watched {
                        path: Some(path.clone()),
                        ..Watched::default()
                    };
                }
                if len > watched.offset
                    && let Some((names, read)) = read_from(&path, watched.offset, len)
                {
                    watched.names = std::mem::take(&mut watched.names).then(names);
                    watched.offset += read;
                }
            }
            let copilot = self
                .copilot
                .as_deref()
                .and_then(|dir| copilot_name(dir, session_id));
            if let Some(name) = best_name(
                registered.get(session_id).or(copilot.as_ref()),
                &watched.names,
            ) && &name != on_board
            {
                out.push((session_id.clone(), name));
            }
        }
        out
    }
}

/// A session's transcript: `<projects>/<any project folder>/<session id>.jsonl`.
fn find_transcript(projects: &Path, session_id: &str) -> Option<PathBuf> {
    let file = format!("{session_id}.jsonl");
    std::fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|dir| dir.path().join(&file))
        .find(|candidate| candidate.is_file())
}

/// The names in bytes `[from, to)` of a transcript, up to the last whole line, and how many bytes
/// that was. A line still being written is left for the next read.
fn read_from(path: &Path, from: u64, to: u64) -> Option<(Names, u64)> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = Vec::new();
    BufReader::new(file)
        .take(to - from)
        .read_to_end(&mut bytes)
        .ok()?;
    let whole = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    Some((scan_names(&bytes[..whole]), whole as u64))
}
