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

use crate::identity::{Names, scan_names};

/// Where Claude Code keeps transcripts: `FOCUS_BOARD_PROJECTS`, else `~/.claude/projects`.
pub fn projects_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FOCUS_BOARD_PROJECTS").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::home_dir().map(|h| h.join(".claude").join("projects"))
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
    sessions: HashMap<String, Watched>,
}

impl NameWatcher {
    pub fn new(projects: PathBuf) -> NameWatcher {
        NameWatcher {
            projects,
            sessions: HashMap::new(),
        }
    }

    /// The sessions whose name in their transcript differs from the board's, as
    /// `(session id, name)`. `sessions` holds each session's id and its name on the board.
    pub fn changed(&mut self, sessions: &[(String, String)]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (session_id, on_board) in sessions {
            let watched = self.sessions.entry(session_id.clone()).or_default();
            if watched.path.is_none() {
                watched.path = find_transcript(&self.projects, session_id);
            }
            let Some(path) = watched.path.clone() else {
                continue;
            };
            let Ok(len) = path.metadata().map(|m| m.len()) else {
                continue;
            };
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
            if let Some(name) = watched.names.name()
                && &name != on_board
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
