//! The Claude Code hook that tells a `focus-board` command who is running it.
//!
//! A hook runs as its own process and cannot set variables in the session's shell, and a
//! subagent shares its parent's process. So the `PreToolUse` hook rewrites the Bash command
//! itself: when the command runs `focus-board`, the session's ids are exported in front of it.
//! The rewrite returns only `updatedInput`, never an `allow`, so the normal permission checks
//! still run on the rewritten command. The one decision the hook makes only tightens: a command
//! that sets the identity variables itself is denied (see [`sets_identity`]).

use serde_json::{Value, json};

use crate::identity::{
    AGENT_TYPE_VAR, AGENT_VAR, SESSION_VAR, TRANSCRIPT_VAR, is_plain_id, is_safe_path,
};

/// Why a command that sets the identity variables itself is denied.
pub const DENY_REASON: &str = "focus-board: the hook sets FOCUS_BOARD_SESSION, FOCUS_BOARD_AGENT, \
     FOCUS_BOARD_AGENT_TYPE and FOCUS_BOARD_TRANSCRIPT; a command never sets them itself, or a \
     subagent would write as its parent. Run focus-board as the first word of the command and let \
     the hook add them.";

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
    if sets_identity(command) {
        return Some(
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": DENY_REASON,
                }
            })
            .to_string(),
        );
    }
    if !runs_board(command) {
        return None;
    }
    let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or("");
    let (session, agent, agent_type) = (text("session_id"), text("agent_id"), text("agent_type"));
    if session.is_empty() || ![session, agent, agent_type].iter().all(|v| is_plain_id(v)) {
        return None;
    }
    // Plain ids only (checked above), so single quotes cannot be broken out of; the transcript's
    // path only when it is safe too. A copied session finds its parent there (see fork_parent).
    let transcript = text("transcript_path");
    let transcript = if is_safe_path(transcript) {
        transcript
    } else {
        ""
    };
    let prefix = format!(
        "export {SESSION_VAR}='{session}' {AGENT_VAR}='{agent}' {AGENT_TYPE_VAR}='{agent_type}' \
         {TRANSCRIPT_VAR}='{transcript}'; "
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
    // Words that may stand before the command itself: prefixes and shell keywords.
    const SKIP: [&str; 14] = [
        "env", "exec", "command", "time", "nohup", "nice", "!", "if", "then", "else", "elif", "do",
        "while", "until",
    ];
    command
        .split([';', '&', '|', '(', ')', '\n', '`', '{', '}'])
        .any(|segment| {
            let mut words = segment
                .split_whitespace()
                .map(|w| w.trim_matches(|c| c == '"' || c == '\''))
                .peekable();
            while let Some(word) = words.next() {
                if is_assignment(word) || SKIP.contains(&word) {
                    continue;
                }
                if word == "timeout" {
                    // Its options (two of them take a value), then its duration.
                    while let Some(option) = words.next_if(|w| w.starts_with('-')) {
                        if matches!(option, "-k" | "-s" | "--kill-after" | "--signal") {
                            words.next();
                        }
                    }
                    words.next();
                    continue;
                }
                return word.rsplit('/').next() == Some("focus-board");
            }
            false
        })
}

/// Whether a shell command assigns one of the identity variables itself (`NAME=`, `NAME+=`, in
/// any form: inline, `export`, `env`, `declare`). It would override the hook's export, and the
/// value at hand, `$CLAUDE_CODE_SESSION_ID`, is a subagent's parent's id. Mentioning a variable,
/// as `grep` or `echo $NAME` do, is fine. A guard against mistakes, not against an adversary.
pub fn sets_identity(command: &str) -> bool {
    [SESSION_VAR, AGENT_VAR, AGENT_TYPE_VAR, TRANSCRIPT_VAR]
        .iter()
        .any(|var| {
            command.match_indices(var).any(|(at, _)| {
                let before = command[..at].chars().next_back();
                let starts_a_name = before.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
                let rest = &command[at + var.len()..];
                starts_a_name && (rest.starts_with('=') || rest.starts_with("+="))
            })
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
            "timeout 5 focus-board list",
            "timeout -k 2 9 focus-board list",
            "if focus-board list; then echo ok; fi",
            "! focus-board add q",
            "while true; do focus-board list; done",
        ] {
            assert!(runs_board(yes), "should match: {yes}");
        }
        for no in [
            "ls /x/focus-board/y",
            "cd ~/dev/focus-board && cargo test",
            "grep focus-board README.md",
            "echo focus-board",
            "focus-boarding",
            "timeout 5 sleep 1",
            "",
        ] {
            assert!(!runs_board(no), "should not match: {no}");
        }
    }

    #[test]
    fn sets_identity_finds_assignments_only() {
        for yes in [
            "FOCUS_BOARD_SESSION=x focus-board add q",
            "export FOCUS_BOARD_AGENT=; focus-board list",
            "env FOCUS_BOARD_AGENT_TYPE=fork focus-board feedback B1 x",
            "FOCUS_BOARD_SESSION+=x true",
            "timeout 5 env FOCUS_BOARD_SESSION=$CLAUDE_CODE_SESSION_ID focus-board add q",
            "declare -x FOCUS_BOARD_SESSION=x",
            "FOCUS_BOARD_TRANSCRIPT=/tmp/forged.jsonl focus-board list",
        ] {
            assert!(sets_identity(yes), "should be caught: {yes}");
        }
        for no in [
            "grep -rn FOCUS_BOARD_SESSION src/",
            "echo $FOCUS_BOARD_SESSION $FOCUS_BOARD_AGENT",
            "MY_FOCUS_BOARD_SESSION=1 true",
            "env | grep FOCUS_BOARD_",
            "unset FOCUS_BOARD_SESSION",
            "focus-board list",
        ] {
            assert!(!sets_identity(no), "should pass: {no}");
        }
    }
}
