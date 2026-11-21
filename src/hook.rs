//! The Claude Code hook that tells a `focus-board` command who is running it.
//!
//! A hook runs as its own process and cannot set variables in the session's shell, and a
//! subagent shares its parent's process. So the `PreToolUse` hook rewrites the Bash command
//! itself: when the command runs `focus-board`, the session's ids are exported in front of it.
//! The hook returns only `updatedInput`, never a permission decision, so the normal permission
//! checks still run on the rewritten command.

use serde_json::{Value, json};

use crate::identity::{AGENT_TYPE_VAR, AGENT_VAR, SESSION_VAR, is_plain_id};

/// The hook's answer for one `PreToolUse` input, or `None` to leave the call alone. It never
/// fails: whatever goes wrong, the tool call runs as it was, and an unidentified write is then
/// refused by the command.
pub fn pre_tool_use(input: &str) -> Option<String> {
    let data: Value = serde_json::from_str(input).ok()?;
    if data.get("tool_name")?.as_str()? != "Bash" {
        return None;
    }
    let tool_input = data.get("tool_input")?.as_object()?;
    let command = tool_input.get("command")?.as_str()?;
    if !runs_board(command) {
        return None;
    }
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or("");
    let (session, agent, agent_type) = (text("session_id"), text("agent_id"), text("agent_type"));
    if session.is_empty() || ![session, agent, agent_type].iter().all(|v| is_plain_id(v)) {
        return None;
    }
    // Plain ids only (checked above), so single quotes cannot be broken out of.
    let prefix = format!(
        "export {SESSION_VAR}='{session}' {AGENT_VAR}='{agent}' {AGENT_TYPE_VAR}='{agent_type}'; "
    );
    let mut updated = tool_input.clone();
    updated.insert("command".into(), json!(format!("{prefix}{command}")));
    Some(
        json!({
            "hookSpecificOutput": { "hookEventName": "PreToolUse", "updatedInput": updated }
        })
        .to_string(),
    )
}

/// Whether a shell command runs `focus-board` (by name or by a path ending in it) as a command,
/// not merely mentions it. A miss only means a write is refused for want of ids; a false match
/// only adds three harmless variables.
pub fn runs_board(command: &str) -> bool {
    const PREFIXES: [&str; 5] = ["env", "exec", "command", "time", "nohup"];
    command
        .split([';', '&', '|', '(', ')', '\n', '`', '{', '}'])
        .any(|segment| {
            let mut words = segment
                .split_whitespace()
                .map(|w| w.trim_matches(|c| c == '"' || c == '\''))
                .skip_while(|w| is_assignment(w) || PREFIXES.contains(w));
            words
                .next()
                .is_some_and(|first| first.rsplit('/').next() == Some("focus-board"))
        })
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_board_finds_the_command_word_only() {
        for yes in [
            "focus-board list",
            "  focus-board add 'q'",
            "cd /x && ./target/debug/focus-board whoami",
            "FOO=1 focus-board list",
            "env FOO=1 focus-board list",
            "true; focus-board show B1",
            "echo $(focus-board list)",
            "~/.cargo/bin/focus-board list | head",
        ] {
            assert!(runs_board(yes), "should match: {yes}");
        }
        for no in [
            "ls /x/focus-board/y",
            "cd ~/dev/focus-board && cargo test",
            "grep focus-board README.md",
            "echo focus-board",
            "focus-boarding",
            "",
        ] {
            assert!(!runs_board(no), "should not match: {no}");
        }
    }
}
