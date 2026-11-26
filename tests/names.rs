//! The rename watch (owner, 2026-11-21: "when a session is renamed the board should be updated").

use std::io::Write;
use std::path::{Path, PathBuf};

use focus_board::names::NameWatcher;
use serde_json::json;

fn transcript(projects: &Path, session: &str) -> PathBuf {
    let dir = projects.join("-home-me-dev-p");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{session}.jsonl"))
}

fn append(path: &Path, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(line.as_bytes()).unwrap();
}

fn title(t: &str) -> String {
    format!(
        "{}\n",
        json!({ "type": "custom-title", "customTitle": t, "sessionId": "s1" })
    )
}

fn label(l: &str) -> String {
    format!(
        "{}\n",
        json!({ "type": "agent-name", "agentName": l, "sessionId": "s1" })
    )
}

/// The watcher reads a transcript in full once, then only what was added: a new title is
/// reported, an unchanged one is not, a later label does not hide an earlier title, and a line
/// still being written waits until it is whole.
#[test]
fn the_watch_follows_a_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = transcript(dir.path(), "s1");
    append(&path, &label("check branch activity"));
    append(&path, &title("Smith"));
    let mut watcher = NameWatcher::new(dir.path().to_path_buf(), None);
    let on_board = |name: &str| vec![("s1".to_string(), name.to_string())];

    assert_eq!(
        watcher.changed(&on_board("plain-crane")),
        vec![("s1".into(), "Smith".into())]
    );
    assert!(
        watcher.changed(&on_board("Smith")).is_empty(),
        "nothing new"
    );

    append(
        &path,
        &format!("{}\n", json!({ "type": "assistant", "text": "work" })),
    );
    append(&path, &label("some new label"));
    assert!(
        watcher.changed(&on_board("Smith")).is_empty(),
        "a label does not hide a title"
    );

    let renamed = title("Smith - the genius coder");
    let (start, end) = renamed.split_at(20);
    append(&path, start);
    assert!(
        watcher.changed(&on_board("Smith")).is_empty(),
        "half a line is not read yet"
    );
    append(&path, end);
    assert_eq!(
        watcher.changed(&on_board("Smith")),
        vec![("s1".into(), "Smith - the genius coder".into())]
    );

    assert!(
        watcher
            .changed(&[("not-there".into(), "x".into())])
            .is_empty(),
        "a session without a transcript is left alone"
    );
}

/// A transcript that becomes shorter than what was read is read again from the start.
#[test]
fn a_rewritten_transcript_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = transcript(dir.path(), "s1");
    append(&path, &title(&"long title ".repeat(5)));
    let mut watcher = NameWatcher::new(dir.path().to_path_buf(), None);
    let on_board = vec![("s1".to_string(), "x".to_string())];
    assert_eq!(watcher.changed(&on_board).len(), 1);
    std::fs::write(&path, title("Lens")).unwrap();
    assert_eq!(
        watcher.changed(&on_board),
        vec![("s1".into(), "Lens".into())]
    );
}

/// The owner's ask (2026-11-21): sessions renamed in Claude Code show their name. Claude Code's
/// live registry (`~/.claude/sessions/<pid>.json`) holds it, with how it came: the owner's
/// `/rename` (`user`) beats a transcript title, which beats Claude's label (`auto`); a `derived`
/// name (folder plus a short id) is never used. The newer of two files wins; `.key` files are never
/// read.
#[test]
fn the_registry_names_running_sessions() {
    let projects = tempfile::tempdir().unwrap();
    let registry = tempfile::tempdir().unwrap();
    let reg = |file: &str, session: &str, name: &str, source: &str, since: i64| {
        let body =
            json!({ "sessionId": session, "name": name, "nameSource": source, "nameSince": since });
        std::fs::write(registry.path().join(file), body.to_string()).unwrap();
    };
    reg("101.json", "s1", "claude", "user", 1);
    reg("102.json", "s1", "Lens", "user", 2);
    reg("103.json", "s2", "switch session lens", "auto", 1);
    reg("104.json", "s3", "pulceed-2b", "derived", 1);
    reg("105.json", "s4", "an auto label", "auto", 1);
    std::fs::write(
        registry.path().join("101.secretkey.key"),
        "not json, never read",
    )
    .unwrap();
    append(&transcript(projects.path(), "s1"), &title("Smith"));
    append(
        &transcript(projects.path(), "s4"),
        &title("Title from the transcript"),
    );

    let mut watcher = NameWatcher::new(
        projects.path().to_path_buf(),
        Some(registry.path().to_path_buf()),
    );
    let on_board: Vec<(String, String)> = ["s1", "s2", "s3", "s4"]
        .iter()
        .map(|s| (s.to_string(), "made-up".to_string()))
        .collect();
    let mut changed = watcher.changed(&on_board);
    changed.sort();
    assert_eq!(
        changed,
        vec![
            ("s1".into(), "Lens".into()),
            ("s2".into(), "switch session lens".into()),
            ("s4".into(), "Title from the transcript".into()),
        ],
        "s3's derived name is not used"
    );
}

/// Copilot sessions (owner, 2026-11-22, #353) take their name from Copilot's own
/// `<session-state>/<id>/workspace.yaml`. A session id never leads out of that folder.
#[test]
fn copilot_sessions_have_their_names() {
    let root = tempfile::tempdir().unwrap();
    let file = |at: &str, body: &str| {
        let dir = root.path().join(at);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("workspace.yaml"), body).unwrap();
    };
    file(
        "session-state/c1",
        "id: c1\nname: 'It''s the probe'\nuser_named: true\n",
    );
    file(
        "session-state/c2",
        "id: c2\nname: \"Fix the build\"\nuser_named: false\n",
    );
    file("decoy", "id: decoy\nname: Decoy\nuser_named: true\n");
    let projects = tempfile::tempdir().unwrap();
    let mut watcher = NameWatcher::new(projects.path().to_path_buf(), None)
        .with_copilot(Some(root.path().join("session-state")));
    let on_board: Vec<(String, String)> = ["c1", "c2", "c3", "../decoy"]
        .iter()
        .map(|s| (s.to_string(), "made-up".to_string()))
        .collect();
    let mut changed = watcher.changed(&on_board);
    changed.sort();
    assert_eq!(
        changed,
        vec![
            ("c1".into(), "It's the probe".into()),
            ("c2".into(), "Fix the build".into()),
        ]
    );
}

/// Which tool a session runs in (owner, 2026-11-22, #361): Copilot when Copilot keeps a folder for
/// it, Claude Code when its transcript is there, none when neither is or the id is not plain.
#[test]
fn a_sessions_tool_comes_from_its_files() {
    let projects = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("session-state");
    std::fs::create_dir_all(state.join("copilot-1")).unwrap();
    std::fs::create_dir_all(root.path().join("decoy")).unwrap();
    append(&transcript(projects.path(), "claude-1"), &title("x"));
    let tool = |id: &str| focus_board::names::tool_of(projects.path(), Some(&state), None, id);
    assert_eq!(tool("claude-1"), Some("claude"));
    assert_eq!(tool("copilot-1"), Some("copilot"));
    assert_eq!(tool("gone"), None);
    assert_eq!(
        tool("../decoy"),
        None,
        "an id never leads out of the folder"
    );
    assert_eq!(
        focus_board::names::tool_of(projects.path(), None, None, "copilot-1"),
        None,
        "no Copilot folder known"
    );
}

/// opencode sessions (owner, 2026-11-26) take their name from the title in opencode's own
/// database, except its placeholder "New session - <date>", and are marked as opencode's.
#[test]
fn opencode_sessions_have_their_names_and_mark() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("opencode.db");
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute_batch(
        "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT NOT NULL);
         INSERT INTO session VALUES ('ses_a', 'Fix the parser');
         INSERT INTO session VALUES ('ses_b', 'New session - 2026-11-26T10:00:00.000Z');
         CREATE TABLE session_v2 (id TEXT PRIMARY KEY, title TEXT NOT NULL);
         INSERT INTO session_v2 VALUES ('ses_d', 'Renamed in opencode 2');",
    )
    .unwrap();
    drop(raw);
    let projects = tempfile::tempdir().unwrap();
    let mut watcher =
        NameWatcher::new(projects.path().to_path_buf(), None).with_opencode(Some(db.clone()));
    let on_board: Vec<(String, String)> = ["ses_a", "ses_b", "ses_c", "ses_d"]
        .iter()
        .map(|s| (s.to_string(), "made-up".to_string()))
        .collect();
    let mut changed = watcher.changed(&on_board);
    changed.sort();
    assert_eq!(
        changed,
        vec![
            ("ses_a".to_string(), "Fix the parser".to_string()),
            ("ses_d".to_string(), "Renamed in opencode 2".to_string()),
        ],
        "a placeholder title names nothing; opencode 2.x keeps titles in session_v2"
    );
    assert_eq!(
        focus_board::names::tool_of(projects.path(), None, Some(&db), "ses_d"),
        Some("opencode")
    );
    let tool = |id: &str| focus_board::names::tool_of(projects.path(), None, Some(&db), id);
    assert_eq!(tool("ses_b"), Some("opencode"));
    assert_eq!(tool("ses_c"), None);
}
