//! Which sessions are alive (owner, 2026-11-26, B45 "b").

use std::process::Command;

use focus_board::live::{running_claude, waiting};

/// Liveness comes from live processes only: a wait's announcement or a Claude Code registry entry
/// whose process has ended counts for nothing, and the ended wait's file is removed. The registry's
/// `.key` files are never read.
#[test]
fn liveness_comes_from_live_processes_only() {
    let dir = tempfile::tempdir().unwrap();
    let (waits, registry) = (dir.path().join("waits"), dir.path().join("sessions"));
    std::fs::create_dir_all(&waits).unwrap();
    std::fs::create_dir_all(&registry).unwrap();
    let me = std::process::id();
    let dead = {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    };
    let write = |path: std::path::PathBuf, body: &str| std::fs::write(path, body).unwrap();
    write(
        waits.join(format!("{me}.json")),
        r#"{"session_id":"s1","agent_id":""}"#,
    );
    write(
        waits.join(format!("{dead}.json")),
        r#"{"session_id":"s2","agent_id":""}"#,
    );
    let found = waiting(&waits);
    assert!(found.contains(&("s1".to_string(), String::new())));
    assert!(!found.contains(&("s2".to_string(), String::new())));
    assert!(
        !waits.join(format!("{dead}.json")).exists(),
        "an ended wait's file is removed"
    );

    write(
        registry.join("1.json"),
        &format!(r#"{{"pid":{me},"sessionId":"c1"}}"#),
    );
    write(
        registry.join("2.json"),
        &format!(r#"{{"pid":{dead},"sessionId":"c2"}}"#),
    );
    write(registry.join("1.secret.key"), "never read");
    let running = running_claude(&registry);
    assert!(
        running.contains("c1") && !running.contains("c2"),
        "{running:?}"
    );
}
