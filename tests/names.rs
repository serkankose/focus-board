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
    let mut watcher = NameWatcher::new(dir.path().to_path_buf());
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
    let mut watcher = NameWatcher::new(dir.path().to_path_buf());
    let on_board = vec![("s1".to_string(), "x".to_string())];
    assert_eq!(watcher.changed(&on_board).len(), 1);
    std::fs::write(&path, title("Lens")).unwrap();
    assert_eq!(
        watcher.changed(&on_board),
        vec![("s1".into(), "Lens".into())]
    );
}
