//! Installs the board's hook into Claude Code's settings (owner, 2026-11-26, B45 "d"), so a fresh
//! machine needs no hand edit. Only the hook entry changes: every other setting keeps its value
//! and its place (serde_json keeps key order), the file keeps its two-space layout and its final
//! newline, a copy of the previous file is kept beside it, and the new file replaces the old in one
//! rename. A file that is not valid JSON is left alone.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The hook's command, as the README shows it.
pub const HOOK_COMMAND: &str =
    "\"$HOME/.cargo/bin/focus-board\" hook pre-tool-use 2>/dev/null || true";

/// Claude Code's user settings: `~/.claude/settings.json`.
pub fn claude_settings() -> Option<PathBuf> {
    crate::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// What `install_hook` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Added,
    AlreadyThere,
    Removed,
    NotThere,
}

/// Whether a hook command is ours.
fn ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains("focus-board") && c.contains("hook pre-tool-use"))
}

/// Adds the board's `PreToolUse` hook to the settings at `path`, or removes it with `remove`.
pub fn install_hook(path: &Path, remove: bool) -> Result<Outcome, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut root: Value = match &text {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| format!("{} is not valid JSON ({e}); left as it is", path.display()))?,
        None if remove => return Ok(Outcome::NotThere),
        None => json!({}),
    };
    let settings = root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object; left as it is", path.display()))?;
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("its \"hooks\" is not an object; left as it is")?;
    let present = hooks
        .get("PreToolUse")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|e| {
                e.get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hs| hs.iter().any(ours))
            })
        });
    let outcome = match (remove, present) {
        (false, true) => return Ok(Outcome::AlreadyThere),
        (true, false) => return Ok(Outcome::NotThere),
        (false, false) => {
            let entries = hooks.entry("PreToolUse").or_insert_with(|| json!([]));
            entries
                .as_array_mut()
                .ok_or("its \"PreToolUse\" is not a list; left as it is")?
                .push(json!({
                    "matcher": "Bash",
                    "hooks": [{ "type": "command", "command": HOOK_COMMAND, "timeout": 5 }]
                }));
            Outcome::Added
        }
        (true, true) => {
            let entries = hooks
                .get_mut("PreToolUse")
                .and_then(Value::as_array_mut)
                .ok_or("its \"PreToolUse\" is not a list; left as it is")?;
            for entry in entries.iter_mut() {
                if let Some(hs) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                    hs.retain(|h| !ours(h));
                }
            }
            entries.retain(|e| {
                e.get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|hs| !hs.is_empty())
            });
            if entries.is_empty() {
                hooks.remove("PreToolUse");
            }
            Outcome::Removed
        }
    };
    let new_text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
        + if text.as_deref().is_none_or(|t| t.ends_with('\n')) {
            "\n"
        } else {
            ""
        };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if let Some(old) = &text {
        let backup = path.with_extension("json.focus-board-backup");
        std::fs::write(&backup, old).map_err(|e| format!("{}: {e}", backup.display()))?;
    }
    let temporary = path.with_extension("json.focus-board-new");
    std::fs::write(&temporary, new_text).map_err(|e| format!("{}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(outcome)
}
