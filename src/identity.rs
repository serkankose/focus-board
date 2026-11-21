//! Who is writing, and where they work.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The variables the PreToolUse hook sets in front of a `focus-board` command.
pub const SESSION_VAR: &str = "FOCUS_BOARD_SESSION";
pub const AGENT_VAR: &str = "FOCUS_BOARD_AGENT";
pub const AGENT_TYPE_VAR: &str = "FOCUS_BOARD_AGENT_TYPE";

/// One writer as Claude Code names it: a session, or an agent (subagent, fork) inside one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub session_id: String,
    /// Empty for the session itself.
    pub agent_id: String,
    pub agent_type: String,
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
        Ok(Identity {
            session_id,
            agent_id,
            agent_type,
        })
    }
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
                }
            }
            _ => Place {
                project: shown(&dir),
                workspace: shown(&dir),
            },
        }
    }
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
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn shown(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
