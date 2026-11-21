//! Who is writing, and where they work.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The variables the PreToolUse hook sets in front of a `focus-board` command.
pub const SESSION_VAR: &str = "FOCUS_BOARD_SESSION";
pub const AGENT_VAR: &str = "FOCUS_BOARD_AGENT";
pub const AGENT_TYPE_VAR: &str = "FOCUS_BOARD_AGENT_TYPE";
/// The session's Claude Code transcript, where a copied session finds its parent.
pub const TRANSCRIPT_VAR: &str = "FOCUS_BOARD_TRANSCRIPT";

/// One writer as Claude Code names it: a session, or an agent (subagent, fork) inside one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub session_id: String,
    /// Empty for the session itself.
    pub agent_id: String,
    pub agent_type: String,
    /// The transcript's path, as the hook passed it; empty when unknown.
    pub transcript: String,
    /// The session this one was copied from (`/branch`, `/fork`), when the command has looked it
    /// up for a session new to the board; empty otherwise. See [`fork_parent`].
    pub parent_session: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// No `FOCUS_BOARD_SESSION`: the hook did not run for this command.
    Missing,
    /// The named variable holds characters an id never has.
    Malformed(&'static str),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::Missing => write!(
                f,
                "no {SESSION_VAR}: the focus-board hook did not run for this command, so it cannot \
                 say who is writing. Run focus-board as the first word of the Bash command, so \
                 the hook can add your identity. Never set FOCUS_BOARD_* yourself: a subagent \
                 would write as its parent. Without the hook installed, ask the owner."
            ),
            IdentityError::Malformed(var) => write!(f, "{var} holds characters an id never has"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl Identity {
    pub fn from_env() -> Result<Identity, IdentityError> {
        Identity::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads only the hook's variables. It never falls back to `CLAUDE_CODE_SESSION_ID`: a
    /// subagent's shell carries its parent's id there, so a fallback would write a subagent's
    /// words into its parent's cells.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Identity, IdentityError> {
        let session_id = get(SESSION_VAR).unwrap_or_default();
        if session_id.is_empty() {
            return Err(IdentityError::Missing);
        }
        let agent_id = get(AGENT_VAR).unwrap_or_default();
        let agent_type = get(AGENT_TYPE_VAR).unwrap_or_default();
        for (var, value) in [
            (SESSION_VAR, &session_id),
            (AGENT_VAR, &agent_id),
            (AGENT_TYPE_VAR, &agent_type),
        ] {
            if !is_plain_id(value) {
                return Err(IdentityError::Malformed(var));
            }
        }
        let transcript = get(TRANSCRIPT_VAR).unwrap_or_default();
        if !transcript.is_empty() && !is_safe_path(&transcript) {
            return Err(IdentityError::Malformed(TRANSCRIPT_VAR));
        }
        Ok(Identity {
            session_id,
            agent_id,
            agent_type,
            transcript,
            parent_session: String::new(),
        })
    }
}

/// An absolute path, POSIX (`/…`) or Windows (`C:\…`, `C:/…`), with no quote and no control
/// character: safe inside the hook's single quotes. On Windows the hook's prefix runs in Git Bash,
/// where a backslash inside single quotes is a plain character (review P9).
pub fn is_safe_path(path: &str) -> bool {
    (path.starts_with('/') || is_drive_path(path))
        && path.len() <= 4096
        && !path.chars().any(|c| c == '\'' || c.is_control())
}

/// A Windows path that starts with a drive: `C:\…` or `C:/…`.
pub fn is_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// The usual form of a path. Windows' canonical paths start with `\\?\` (`\\?\C:\…`,
/// `\\?\UNC\server\share\…`), while git and people write them without it; dropping it keeps one
/// form of each path, so a project cannot appear twice (review P8). Other paths pass unchanged.
pub fn plain_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if is_drive_path(rest) => rest.to_string(),
        _ => path.to_string(),
    }
}

/// The session a copied session (`/branch`, `/fork`, `--fork-session`) was copied from, read
/// from its Claude Code transcript, or `None` for a session that is no copy.
///
/// A copy carries its parent's turns, and each copied assistant turn names the session that made
/// it (`requestSessionId`); a copy of a copy keeps the older ones and adds its parent's. So the
/// last such id other than the session's own is the immediate parent. `forkedFrom.sessionId` on
/// copied lines is the fallback: it names the root of a chain. Both fields are undocumented,
/// seen on Claude Code 2.1.329 (2026-11-21); when they are missing, there is no parent. Reads the
/// whole transcript, so the command calls it only for a session new to the board.
pub fn fork_parent(transcript: &Path, own: &str) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(transcript).ok()?;
    let (mut last_request, mut forked_from) = (None, None);
    // Raw lines: one that is not UTF-8 is skipped, not taken as the end (review C3).
    for bytes in std::io::BufReader::new(file).split(b'\n') {
        let Ok(bytes) = bytes else { break };
        let line = String::from_utf8_lossy(&bytes);
        if !line.contains("\"requestSessionId\"") && !line.contains("\"forkedFrom\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = entry.get("requestSessionId").and_then(|v| v.as_str())
            && id != own
        {
            last_request = Some(id.to_string());
        }
        if forked_from.is_none()
            && let Some(id) = entry
                .pointer("/forkedFrom/sessionId")
                .and_then(|v| v.as_str())
            && id != own
        {
            forked_from = Some(id.to_string());
        }
    }
    last_request.or(forked_from).filter(|id| is_plain_id(id))
}

/// Letters, digits and `_ . : -`, at most 256 of them: what Claude Code's ids are made of.
pub fn is_plain_id(s: &str) -> bool {
    s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// Where a writer works. The workspace is the checkout; the project is the repository's main
/// checkout, so every worktree of one repository shares a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub project: String,
    pub workspace: String,
    /// The checkout's branch at that moment, `detached at <commit>`, or empty outside git.
    pub branch: String,
}

impl Place {
    /// The place of `dir`. `home` is the home folder: on this machine it is itself a git
    /// repository, so a folder whose top level is the home folder counts as having no repository
    /// of its own, and like a folder outside any repository it is its own project and workspace.
    pub fn of_dir(dir: &Path, home: Option<&Path>) -> Place {
        let dir = canonical(dir);
        let home = home.map(canonical);
        let top = git(&dir, &["rev-parse", "--show-toplevel"]).map(|t| canonical(Path::new(&t)));
        match top {
            Some(top) if Some(&top) != home.as_ref() => {
                let common = git(
                    &dir,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .map(|c| canonical(Path::new(&c)));
                let project = match common {
                    Some(c) if c.file_name().is_some_and(|n| n == ".git") => c
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| top.clone()),
                    _ => top.clone(),
                };
                Place {
                    project: shown(&project),
                    workspace: shown(&top),
                    branch: branch_of(&dir),
                }
            }
            _ => Place {
                project: shown(&dir),
                workspace: shown(&dir),
                branch: String::new(),
            },
        }
    }
}

/// The checkout's branch (an unborn one too), `detached at <commit>`, or empty when unknown.
fn branch_of(dir: &Path) -> String {
    if let Some(branch) = git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        return branch;
    }
    git(dir, &["rev-parse", "--short", "HEAD"])
        .map(|commit| format!("detached at {commit}"))
        .unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn canonical(p: &Path) -> PathBuf {
    let full = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    PathBuf::from(plain_path(&full.to_string_lossy()))
}

fn shown(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
