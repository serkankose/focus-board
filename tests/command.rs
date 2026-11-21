//! The command, run as a session runs it: the real binary, a fresh board, a working directory
//! with no repository, and only the variables the hook would set.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_focus-board");

struct Board {
    dir: tempfile::TempDir,
}

type Who<'a> = Option<(&'a str, &'a str, &'a str)>;

impl Board {
    fn new() -> Board {
        let dir = tempfile::tempdir().unwrap();
        for d in ["work", "elsewhere", "one/same", "two/same"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        Board { dir }
    }

    fn command(&self, who: Who, cwd: &str) -> Command {
        let mut c = Command::new(BIN);
        c.current_dir(self.dir.path().join(cwd))
            .env("FOCUS_BOARD_DB", self.dir.path().join("board.sqlite"))
            .env("HOME", self.dir.path())
            .env_remove("FOCUS_BOARD_SESSION")
            .env_remove("FOCUS_BOARD_AGENT")
            .env_remove("FOCUS_BOARD_AGENT_TYPE")
            // Every session's shell has this; it must never count as an identity.
            .env("CLAUDE_CODE_SESSION_ID", "shell-session-id");
        if let Some((session, agent, agent_type)) = who {
            c.env("FOCUS_BOARD_SESSION", session)
                .env("FOCUS_BOARD_AGENT", agent)
                .env("FOCUS_BOARD_AGENT_TYPE", agent_type);
        }
        c
    }

    fn run_in(&self, who: Who, cwd: &str, args: &[&str]) -> Output {
        self.command(who, cwd).args(args).output().unwrap()
    }

    fn run(&self, who: Who, args: &[&str]) -> Output {
        self.run_in(who, "work", args)
    }

    fn ok(&self, who: Who, args: &[&str]) -> String {
        let out = self.run(who, args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(None, args)).unwrap()
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const S: Who = Some(("sess-1", "", ""));

/// Brief: "The command-line tool refuses the owner's column."
#[test]
fn cli_refuses_answer() {
    let board = Board::new();
    assert_eq!(
        board
            .ok(S, &["add", "q?", "-o", "a=yes", "-o", "b=no"])
            .trim(),
        "B1"
    );
    for args in [
        &["answer", "B1", "a"][..],
        &["answer", "--pick", "a", "B1"],
        &["answer"],
    ] {
        let out = board.run(S, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("the owner answers on the page"),
            "{}",
            stderr(&out)
        );
    }
    let item = board.json(&["show", "B1", "--json"]);
    let cells = item["cells"].as_array().unwrap();
    assert!(
        cells
            .iter()
            .all(|c| c["field"] != "answer" && c["writer"] != "owner"),
        "{item}"
    );
}

/// Design: "No identity, no write." The shell's own CLAUDE_CODE_SESSION_ID is not an identity;
/// reading needs none.
#[test]
fn unidentified_command_write_refused() {
    let board = Board::new();
    for args in [&["add", "q?"][..], &["whoami"], &["feedback", "B1", "x"]] {
        let out = board.run(None, args);
        assert!(!out.status.success(), "{args:?} was allowed");
        assert!(
            stderr(&out).contains("FOCUS_BOARD_SESSION"),
            "{args:?}: {}",
            stderr(&out)
        );
        // The refusal must not invite setting the variable by hand (review R1).
        assert!(
            stderr(&out).contains("Never set FOCUS_BOARD_* yourself")
                && stderr(&out).contains("first word"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(
        board.json(&["list", "--everywhere", "--all", "--json"]),
        Value::Array(vec![])
    );
}

/// Review R1: a command that sets the identity variables itself would override the hook's export,
/// and the value at hand is a subagent's parent's id. The hook denies it, whatever the command,
/// and never denies a command that only mentions a variable.
#[test]
fn hook_denies_a_hand_set_identity() {
    for command in [
        "FOCUS_BOARD_SESSION=x focus-board add q",
        "export FOCUS_BOARD_AGENT=; focus-board list",
        "timeout 5 env FOCUS_BOARD_SESSION=$CLAUDE_CODE_SESSION_ID focus-board add q",
        "FOCUS_BOARD_AGENT_TYPE=fork true",
        "FOCUS_BOARD_TRANSCRIPT=/tmp/forged.jsonl focus-board add q",
    ] {
        let out = hook(&bash_input(
            "sess-1",
            Some(("ag-1", "general-purpose")),
            command,
        ));
        assert!(out.status.success(), "the hook failed on {command}");
        let reply: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let decision = &reply["hookSpecificOutput"];
        assert_eq!(decision["permissionDecision"], "deny", "{command}: {reply}");
        assert!(
            decision["permissionDecisionReason"]
                .as_str()
                .is_some_and(|r| r.contains("first word")),
            "{reply}"
        );
        assert!(decision.get("updatedInput").is_none(), "{reply}");
    }
    for command in [
        "grep -rn FOCUS_BOARD_SESSION src/",
        "echo $FOCUS_BOARD_AGENT",
    ] {
        let out = hook(&bash_input("sess-1", None, command));
        assert!(out.stdout.is_empty(), "{command} was not left alone");
    }
}

fn hook(input: &str) -> Output {
    let mut child = Command::new(BIN)
        .args(["hook", "pre-tool-use"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn bash_input(session: &str, agent: Option<(&str, &str)>, command: &str) -> String {
    let mut v = serde_json::json!({
        "session_id": session,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command, "description": "d" },
    });
    if let Some((id, kind)) = agent {
        v["agent_id"] = id.into();
        v["agent_type"] = kind.into();
    }
    v.to_string()
}

/// Design: the hook rewrites only commands that run focus-board, never fails, and the rewritten
/// command really writes as the hook's session and agent.
#[test]
fn hook_rewrites_only_board_calls() {
    for input in [
        bash_input("sess-1", None, "ls /x/focus-board"),
        bash_input("sess-1", None, "cd ~/dev/focus-board && cargo test"),
        bash_input("bad'id", None, "focus-board list"),
        bash_input("", None, "focus-board list"),
        r#"{"tool_name":"Read","tool_input":{"file_path":"focus-board"}}"#.to_string(),
        "not json".to_string(),
        String::new(),
    ] {
        let out = hook(&input);
        assert!(out.status.success(), "the hook failed on {input}");
        assert!(
            out.stdout.is_empty(),
            "rewrote {input}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    // End to end: the hook's rewrite of a real command, run by bash, writes as the subagent.
    let board = Board::new();
    board.ok(S, &["add", "q?"]);
    let original = format!("{BIN} feedback B1 'from the fork' && {BIN} whoami");
    let out = hook(&bash_input("sess-1", Some(("ag-7", "fork")), &original));
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
    let updated = &reply["hookSpecificOutput"]["updatedInput"];
    assert_eq!(reply["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert!(
        reply["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert_eq!(updated["description"], "d", "other tool input is kept");
    let rewritten = updated["command"].as_str().unwrap();
    assert!(rewritten.ends_with(&original));

    let out = Command::new("bash")
        .arg("-c")
        .arg(rewritten)
        .current_dir(board.dir.path().join("work"))
        .env("FOCUS_BOARD_DB", board.dir.path().join("board.sqlite"))
        .env("HOME", board.dir.path())
        .env_remove("FOCUS_BOARD_SESSION")
        .env_remove("FOCUS_BOARD_AGENT")
        .env_remove("FOCUS_BOARD_AGENT_TYPE")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let whoami = String::from_utf8(out.stdout).unwrap();
    assert!(
        whoami.contains("fork ag-7 in sess-1") && whoami.contains("under      session sess-1"),
        "{whoami}"
    );
    let item = board.json(&["show", "B1", "--json"]);
    let fork_cell = item["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "feedback")
        .unwrap();
    assert_eq!(fork_cell["writer"], "fork ag-7 in sess-1");
}

/// The command end to end: raise, give feedback with a version, close, list by project and
/// status, name a writer, and keep a fork's cell apart from its parent's.
#[test]
fn command_round_trip() {
    let board = Board::new();
    let id = board.ok(
        S,
        &[
            "add",
            "Keep SQLite?",
            "-o",
            "a=yes",
            "-o",
            "b=no",
            "--recommend",
            "a",
            "--label",
            "#15",
        ],
    );
    assert_eq!(id.trim(), "B1");
    assert_eq!(
        board
            .ok(S, &["feedback", "B1", "first", "--expect", "0"])
            .trim(),
        "B1 feedback v1"
    );
    let stale = board.run(S, &["feedback", "B1", "late", "--expect", "0"]);
    assert!(
        !stale.status.success() && stderr(&stale).contains("version 1"),
        "{}",
        stderr(&stale)
    );
    board.ok(
        Some(("sess-1", "f1", "fork")),
        &["feedback", "B1", "--pick", "b"],
    );
    board.ok(S, &["whoami", "--name", "Smith"]);

    let item = board.json(&["show", "B1", "--json"]);
    assert_eq!(item["label"], "#15");
    // Raised in a folder outside git: the context says so, with the full path.
    assert_eq!(item["branch"], "");
    assert!(
        item["workspace"].as_str().unwrap().ends_with("/work"),
        "{item}"
    );
    let shown = board.ok(None, &["show", "B1"]);
    assert!(
        shown.contains("in         work · not in git") && shown.contains("path       /"),
        "{shown}"
    );
    assert_eq!(item["recommended"], "a");
    assert_eq!(item["options"]["b"], "no");
    let feedback: Vec<(String, Value)> = item["cells"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["field"] == "feedback")
        .map(|c| {
            (
                c["writer"].as_str().unwrap().to_string(),
                c["value"].clone(),
            )
        })
        .collect();
    assert_eq!(
        feedback,
        vec![
            (
                "Smith (session sess-1)".to_string(),
                serde_json::json!({ "text": "first" })
            ),
            (
                "fork f1 in sess-1".to_string(),
                serde_json::json!({ "pick": "b" })
            ),
        ]
    );
    let text = board.ok(None, &["show", "B1"]);
    assert!(
        text.contains("a) yes   (recommended)") && text.contains("answer     none yet"),
        "{text}"
    );

    // Listing: this project by default, not closed by default.
    let here = board.json(&["list", "--json"]);
    assert_eq!(here.as_array().unwrap().len(), 1);
    let elsewhere: Value =
        serde_json::from_slice(&board.run_in(None, "elsewhere", &["list", "--json"]).stdout)
            .unwrap();
    assert_eq!(
        elsewhere,
        Value::Array(vec![]),
        "another project's list stays empty"
    );
    assert_eq!(
        board
            .json(&["list", "--project", "work", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let refused = board.run(Some(("sess-2", "", "")), &["status", "B1", "closed"]);
    assert!(!refused.status.success(), "only the author closes its item");
    board.ok(S, &["status", "B1", "closed"]);
    assert_eq!(board.json(&["list", "--json"]), Value::Array(vec![]));
    assert_eq!(
        board.json(&["list", "--all", "--json"])[0]["status"],
        "closed"
    );
    assert_eq!(
        board.json(&["list", "--status", "closed", "--json"])[0]["id"],
        "B1"
    );

    let history = board.json(&["history", "B1", "--json"]);
    let seqs: Vec<i64> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["seq"].as_i64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "history is in the order written"
    );
    assert!(board.run(None, &["show", "B99"]).status.code() == Some(1));
}

/// Review R6: two projects can share a folder name, and projects never mix, so a name that fits
/// more than one is refused with the paths to choose from; a path picks exactly one.
#[test]
fn a_shared_folder_name_does_not_mix_projects() {
    let board = Board::new();
    // Two sessions: a writer's project is fixed at its first write.
    for (session, cwd) in [("sess-1", "one/same"), ("sess-2", "two/same")] {
        let out = board.run_in(
            Some((session, "", "")),
            cwd,
            &["add", &format!("raised in {cwd}")],
        );
        assert!(out.status.success(), "{}", stderr(&out));
    }
    let both = board.run(None, &["list", "--project", "same", "--json"]);
    assert_eq!(both.status.code(), Some(1), "an ambiguous name was taken");
    assert!(
        stderr(&both).contains("names 2 projects"),
        "{}",
        stderr(&both)
    );

    let one = board.dir.path().join("one/same").canonicalize().unwrap();
    let out = board.run(
        None,
        &["list", "--project", one.to_str().unwrap(), "--json"],
    );
    let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
    let questions: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["question"].as_str().unwrap())
        .collect();
    assert_eq!(questions, vec!["raised in one/same"]);
}

/// Review R3, end to end: the hook passes the transcript's path, and a copied session's first
/// write files it under the session it was copied from. An unsafe path is not passed on.
#[test]
fn a_copied_session_writes_under_its_parent() {
    let board = Board::new();
    board.ok(Some(("sess-root", "", "")), &["add", "q?"]);
    let transcript = board.dir.path().join("copy.jsonl");
    let lines = [
        serde_json::json!({ "type": "x", "sessionId": "sess-copy" }),
        serde_json::json!({ "type": "assistant", "sessionId": "sess-copy",
            "forkedFrom": { "sessionId": "sess-root", "messageUuid": "m" },
            "requestSessionId": "sess-root" }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&transcript, text).unwrap();

    let command = format!("{BIN} feedback B1 'from the copy' && {BIN} whoami");
    let mut input: Value = serde_json::from_str(&bash_input("sess-copy", None, &command)).unwrap();
    input["transcript_path"] = Value::String(transcript.to_str().unwrap().to_string());
    let reply: Value = serde_json::from_slice(&hook(&input.to_string()).stdout).unwrap();
    let rewritten = reply["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        rewritten.contains("FOCUS_BOARD_TRANSCRIPT='/"),
        "{rewritten}"
    );

    let out = Command::new("bash")
        .arg("-c")
        .arg(&rewritten)
        .current_dir(board.dir.path().join("work"))
        .env("FOCUS_BOARD_DB", board.dir.path().join("board.sqlite"))
        .env("HOME", board.dir.path())
        .env_remove("FOCUS_BOARD_SESSION")
        .env_remove("FOCUS_BOARD_AGENT")
        .env_remove("FOCUS_BOARD_AGENT_TYPE")
        .env_remove("FOCUS_BOARD_TRANSCRIPT")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let whoami = String::from_utf8(out.stdout).unwrap();
    assert!(
        whoami.contains("writes as  session sess-cop (copy of sess-roo)")
            && whoami.contains("under      session sess-roo"),
        "{whoami}"
    );

    input["transcript_path"] = Value::String("/tmp/it's.jsonl".into());
    let reply: Value = serde_json::from_slice(&hook(&input.to_string()).stdout).unwrap();
    let command = reply["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("FOCUS_BOARD_TRANSCRIPT=''"), "{command}");
}
