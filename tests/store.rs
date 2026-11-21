//! The brief's tests that the store alone can prove. Each test names the brief's line it proves.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use focus_board::identity::{Identity, IdentityError, Place};
use focus_board::store::{Field, Filter, NewItem, OWNER, Store, WriteError};
use serde_json::json;

fn board() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("board.sqlite");
    (dir, db)
}

fn session(id: &str) -> Identity {
    Identity {
        session_id: id.into(),
        agent_id: String::new(),
        agent_type: String::new(),
    }
}

fn agent(session_id: &str, agent_id: &str, agent_type: &str) -> Identity {
    Identity {
        session_id: session_id.into(),
        agent_id: agent_id.into(),
        agent_type: agent_type.into(),
    }
}

fn place(project: &str, workspace: &str) -> Place {
    Place {
        project: project.into(),
        workspace: workspace.into(),
    }
}

fn question(q: &str) -> NewItem {
    NewItem {
        question: q.into(),
        options: vec![("a".into(), "yes".into()), ("b".into(), "no".into())],
        recommended: Some("a".into()),
        ..NewItem::default()
    }
}

fn feedback(text: &str) -> serde_json::Value {
    json!({ "text": text })
}

/// Brief: "Two writers change different columns of the same row at the same moment, and both
/// edits survive." Three connections (two sessions and the owner) start together on one item.
#[test]
fn concurrent_writers_both_land() {
    let (_dir, db) = board();
    let here = place("p", "p");
    let item = Store::open(&db)
        .unwrap()
        .add_item(&session("s-a"), &here, &question("q"))
        .unwrap();
    const N: i64 = 100;
    let start = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for who in ["s-a", "s-b"] {
        let (db, start, here) = (db.clone(), start.clone(), here.clone());
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db).unwrap();
            start.wait();
            for read in 0..N {
                let text = format!("{who} {read}");
                let wrote = store
                    .write_as_session(
                        &session(who),
                        &here,
                        item,
                        Field::Feedback,
                        &feedback(&text),
                        Some(read),
                    )
                    .unwrap();
                assert_eq!(wrote, read + 1);
            }
        }));
    }
    {
        let (db, start) = (db.clone(), start.clone());
        handles.push(thread::spawn(move || {
            let mut store = Store::open(&db).unwrap();
            start.wait();
            for read in 0..N {
                store
                    .owner_answer(item, &json!({ "pick": "a" }), Some(read))
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let store = Store::open(&db).unwrap();
    let history = store.history(item).unwrap();
    let a = store.find_writer(&session("s-a")).unwrap().unwrap();
    let b = store.find_writer(&session("s-b")).unwrap().unwrap();
    for (writer, field) in [(a, "feedback"), (b, "feedback"), (OWNER, "answer")] {
        let versions: Vec<i64> = history
            .iter()
            .filter(|c| c.writer == writer && c.field == field)
            .map(|c| c.version)
            .collect();
        assert_eq!(
            versions,
            (1..=N).collect::<Vec<_>>(),
            "writer {writer} {field}"
        );
    }
}

/// Brief: "A write that names a stale version is refused, and the cell keeps the newer value."
#[test]
fn stale_version_refused() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    assert_eq!(
        store
            .write_as_session(
                &me,
                &here,
                item,
                Field::Feedback,
                &feedback("first"),
                Some(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .write_as_session(
                &me,
                &here,
                item,
                Field::Feedback,
                &feedback("second"),
                Some(1)
            )
            .unwrap(),
        2
    );
    let stale = store.write_as_session(
        &me,
        &here,
        item,
        Field::Feedback,
        &feedback("stale"),
        Some(1),
    );
    assert!(
        matches!(stale, Err(WriteError::Stale { current: 2 })),
        "{stale:?}"
    );

    store
        .owner_answer(item, &json!({ "pick": "b" }), Some(0))
        .unwrap();
    let stale = store.owner_answer(item, &json!({ "pick": "a" }), Some(0));
    assert!(
        matches!(stale, Err(WriteError::Stale { current: 1 })),
        "{stale:?}"
    );

    let current = store.current(item).unwrap();
    let value = |field: &str| {
        current
            .iter()
            .find(|c| c.field == field)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value("feedback"), feedback("second"));
    assert_eq!(value("answer"), json!({ "pick": "b" }));
    assert_eq!(
        store
            .history(item)
            .unwrap()
            .iter()
            .filter(|c| c.field == "feedback")
            .count(),
        2
    );
}

/// Brief: "The command-line tool refuses the owner's column", at the store: a session's write of
/// the answer is refused whatever it claims. The command's own refusal is tested with the command.
#[test]
fn session_cannot_write_the_owners_column() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    let answer = store.write_as_session(
        &me,
        &here,
        item,
        Field::Answer,
        &json!({ "pick": "a" }),
        None,
    );
    assert!(matches!(answer, Err(WriteError::NotYours(_))), "{answer:?}");
    let answered =
        store.write_as_session(&me, &here, item, Field::Status, &json!("answered"), None);
    assert!(
        matches!(answered, Err(WriteError::BadValue(_))),
        "{answered:?}"
    );
    assert!(
        store
            .history(item)
            .unwrap()
            .iter()
            .all(|c| c.writer != OWNER)
    );
}

/// Brief: "A fork and its parent write on the same item, and each keeps its own cell."
#[test]
fn fork_and_parent_own_cells() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let (parent, fork) = (session("s"), agent("s", "f1", "fork"));
    let item = store.add_item(&parent, &here, &question("q")).unwrap();
    store
        .write_as_session(
            &parent,
            &here,
            item,
            Field::Feedback,
            &feedback("parent's"),
            None,
        )
        .unwrap();
    store
        .write_as_session(
            &fork,
            &here,
            item,
            Field::Feedback,
            &feedback("fork's"),
            None,
        )
        .unwrap();

    let p = store.find_writer(&parent).unwrap().unwrap();
    let f = store.find_writer(&fork).unwrap().unwrap();
    assert_ne!(p, f);
    let current = store.current(item).unwrap();
    let cell = |w| {
        current
            .iter()
            .find(|c| c.writer == w && c.field == "feedback")
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(cell(p), feedback("parent's"));
    assert_eq!(cell(f), feedback("fork's"));

    // The fork cannot write into its parent's cells: the item's status is the parent's.
    let status = store.write_as_session(&fork, &here, item, Field::Status, &json!("closed"), None);
    assert!(matches!(status, Err(WriteError::NotYours(_))), "{status:?}");
}

/// Brief: "A session that has never written before can write at once, with no setup."
#[test]
fn first_write_creates_writer() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let item = store
        .add_item(&session("old"), &here, &question("q"))
        .unwrap();
    let new = session("never-seen-before");
    assert_eq!(store.find_writer(&new).unwrap(), None);
    store
        .write_as_session(&new, &here, item, Field::Feedback, &feedback("hello"), None)
        .unwrap();
    assert!(store.find_writer(&new).unwrap().is_some());

    let subagent = agent("brand-new-session", "a1", "general-purpose");
    store
        .add_item(&subagent, &here, &question("raised by a new subagent"))
        .unwrap();
    let writers = store.writers().unwrap();
    let sub = writers.iter().find(|w| w.agent_id == "a1").unwrap();
    let its_session = writers
        .iter()
        .find(|w| w.session_id == "brand-new-session" && w.agent_id.is_empty());
    assert_eq!(
        sub.parent,
        its_session.map(|w| w.id),
        "the session row is made with its first agent"
    );
}

/// Brief: "A fork shows under its parent in the tree, and its writes appear in the timeline in
/// time order, between its parent's."
#[test]
fn fork_in_tree_and_timeline_order() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let (parent, fork) = (session("s"), agent("s", "f1", "fork"));
    let item = store.add_item(&parent, &here, &question("q")).unwrap();
    store
        .write_as_session(&parent, &here, item, Field::Feedback, &feedback("1"), None)
        .unwrap();
    store
        .write_as_session(&fork, &here, item, Field::Feedback, &feedback("2"), None)
        .unwrap();
    store
        .write_as_session(&parent, &here, item, Field::Feedback, &feedback("3"), None)
        .unwrap();

    let p = store.find_writer(&parent).unwrap().unwrap();
    let f = store.find_writer(&fork).unwrap().unwrap();
    let writers = store.writers().unwrap();
    assert_eq!(writers.iter().find(|w| w.id == f).unwrap().parent, Some(p));

    let branch = Filter {
        writer: Some(p),
        ..Filter::default()
    };
    let feedback_order: Vec<(i64, serde_json::Value)> = store
        .timeline(&branch, 0)
        .unwrap()
        .into_iter()
        .filter(|c| c.field == "feedback")
        .map(|c| (c.writer, c.value))
        .collect();
    assert_eq!(
        feedback_order,
        vec![(p, feedback("1")), (f, feedback("2")), (p, feedback("3"))]
    );

    let fork_only = Filter {
        writer: Some(f),
        ..Filter::default()
    };
    let fork_rows = store.timeline(&fork_only, 0).unwrap();
    assert!(fork_rows.iter().all(|c| c.writer == f) && fork_rows.len() == 1);

    // The owner's answer on the parent's item belongs to the parent's branch, not the fork's.
    store
        .owner_answer(item, &json!({ "pick": "a" }), None)
        .unwrap();
    let in_branch = |filter: &Filter| {
        store
            .timeline(filter, 0)
            .unwrap()
            .iter()
            .any(|c| c.writer == OWNER && c.field == "answer")
    };
    assert!(
        in_branch(&branch),
        "the owner's answer is missing from the parent's timeline"
    );
    assert!(
        !in_branch(&fork_only),
        "the owner's answer leaked into the fork's timeline"
    );
}

/// Brief: "Every version of a cell is still there after many edits."
#[test]
fn all_versions_kept() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    for n in 1..=1000 {
        store
            .write_as_session(
                &me,
                &here,
                item,
                Field::Feedback,
                &feedback(&n.to_string()),
                Some(n - 1),
            )
            .unwrap();
    }
    let kept: Vec<(i64, serde_json::Value)> = store
        .history(item)
        .unwrap()
        .into_iter()
        .filter(|c| c.field == "feedback")
        .map(|c| (c.version, c.value))
        .collect();
    let expected: Vec<_> = (1..=1000).map(|n| (n, feedback(&n.to_string()))).collect();
    assert_eq!(kept, expected);
}

/// Brief rule 2, "Nothing is deleted", held by the database itself: a raw connection that skips
/// this crate still cannot change or delete history.
#[test]
fn update_and_delete_refused() {
    let (_dir, db) = board();
    let item = Store::open(&db)
        .unwrap()
        .add_item(&session("s"), &place("p", "p"), &question("q"))
        .unwrap();
    let raw = rusqlite::Connection::open(&db).unwrap();
    for sql in [
        "UPDATE cells SET value = '\"x\"'",
        "DELETE FROM cells",
        "UPDATE items SET label = 'x'",
        "DELETE FROM items",
        "UPDATE writers SET project = 'elsewhere'",
        "UPDATE writers SET session_id = 'someone-else'",
        "DELETE FROM writers",
        // REPLACE deletes the clashing row without firing DELETE triggers, so it needs its own.
        "INSERT OR REPLACE INTO cells (item, writer, field, version, value, client, written_at)
             SELECT item, writer, field, version, '\"x\"', client, written_at FROM cells
             WHERE field = 'question'",
        "INSERT OR REPLACE INTO cells (id, item, writer, field, version, value, client, written_at)
             SELECT id, item, writer, field, version + 1, '\"x\"', client, written_at FROM cells
             WHERE field = 'question'",
        "REPLACE INTO items (id, author, label) SELECT id, author, 'x' FROM items",
        "INSERT OR REPLACE INTO writers (id, kind, session_id, project, workspace, created_at)
             VALUES (1, 'session', 'x', '', '', '')",
        "INSERT OR REPLACE INTO writers (kind, session_id, agent_id, project, workspace, created_at)
             SELECT kind, session_id, agent_id, 'elsewhere', workspace, created_at FROM writers
             WHERE kind = 'session'",
        // A version that skips ahead leaves a hole a later write would have to fill.
        "INSERT INTO cells (item, writer, field, version, value, client, written_at)
             SELECT item, writer, field, version + 5, '\"x\"', client, written_at FROM cells
             WHERE field = 'question'",
    ] {
        assert!(raw.execute(sql, []).is_err(), "{sql} was allowed");
    }
    let question: Vec<_> = Store::open(&db)
        .unwrap()
        .history(item)
        .unwrap()
        .into_iter()
        .filter(|c| c.field == "question")
        .map(|c| (c.version, c.value))
        .collect();
    assert_eq!(question, vec![(1, json!("q"))]);
    let writers = Store::open(&db).unwrap().writers().unwrap();
    assert_eq!((writers[0].id, writers[0].kind.as_str()), (OWNER, "owner"));
    assert!(writers.iter().all(|w| w.project != "elsewhere"));
    raw.execute(
        "UPDATE writers SET name = 'Smith' WHERE kind = 'session'",
        [],
    )
    .unwrap();
    assert!(Store::open(&db).unwrap().history(item).unwrap().len() >= 3);
}

/// Values are checked against their field: a pick must be one of the item's options.
#[test]
fn values_are_checked() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    for (field, value) in [
        (Field::Feedback, json!({ "pick": "z" })),
        (Field::Feedback, json!({})),
        (Field::Feedback, json!({ "text": "ok", "extra": 1 })),
        (Field::Recommended, json!("z")),
        (Field::Question, json!("  ")),
        (Field::Options, json!({ "A": "upper case" })),
    ] {
        let wrote = store.write_as_session(&me, &here, item, field, &value, None);
        assert!(
            matches!(wrote, Err(WriteError::BadValue(_))),
            "{field:?} {value} gave {wrote:?}"
        );
    }
    let bad_new = NewItem {
        recommended: Some("c".into()),
        ..question("q")
    };
    assert!(matches!(
        store.add_item(&me, &here, &bad_new),
        Err(WriteError::BadValue(_))
    ));
    store
        .write_as_session(
            &me,
            &here,
            item,
            Field::Feedback,
            &json!({ "pick": "b", "text": "why" }),
            None,
        )
        .unwrap();
}

/// Design: "No identity, no write." Only the hook's variables count; a session id in
/// CLAUDE_CODE_SESSION_ID alone is not an identity.
#[test]
fn unidentified_write_refused() {
    let only_claude =
        |name: &str| (name == "CLAUDE_CODE_SESSION_ID").then(|| "5d1da7e2".to_string());
    assert_eq!(
        Identity::from_lookup(only_claude),
        Err(IdentityError::Missing)
    );
    let quoted = |name: &str| (name == "FOCUS_BOARD_SESSION").then(|| "a'; rm -rf x".to_string());
    assert!(matches!(
        Identity::from_lookup(quoted),
        Err(IdentityError::Malformed(_))
    ));
    let hooked = |name: &str| match name {
        "FOCUS_BOARD_SESSION" => Some("5d1da7e2-7908".to_string()),
        "FOCUS_BOARD_AGENT" => Some("ab07".to_string()),
        "FOCUS_BOARD_AGENT_TYPE" => Some("general-purpose".to_string()),
        _ => None,
    };
    assert_eq!(
        Identity::from_lookup(hooked),
        Ok(agent("5d1da7e2-7908", "ab07", "general-purpose"))
    );
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "init.defaultBranch=master",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

/// Brief: "Items from two projects, and from two workspaces of one project, appear under their
/// own filters and never mix." Real repositories, a real worktree, and a folder with no
/// repository of its own inside a home folder that is itself a repository.
#[test]
fn projects_and_worktrees_stay_apart() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let (alpha, alpha_wt, beta, loose) = (
        home.join("dev/alpha"),
        home.join("dev/alpha-wt"),
        home.join("dev/beta"),
        home.join("dev/loose"),
    );
    for d in [&alpha, &beta, &loose] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&home, &["init", "-q"]);
    for repo in [&alpha, &beta] {
        git(repo, &["init", "-q"]);
        git(repo, &["commit", "-q", "--allow-empty", "-m", "start"]);
    }
    git(
        &alpha,
        &["worktree", "add", "-q", alpha_wt.to_str().unwrap()],
    );

    let at = |d: &PathBuf| Place::of_dir(d, Some(&home));
    let (p_alpha, p_wt, p_beta, p_loose) = (at(&alpha), at(&alpha_wt), at(&beta), at(&loose));
    assert_eq!(
        p_wt.project, p_alpha.project,
        "a worktree shares its repository's project"
    );
    assert_ne!(p_wt.workspace, p_alpha.workspace);
    assert_ne!(p_beta.project, p_alpha.project);
    let home_shown = home.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_ne!(
        p_loose.project, home_shown,
        "a folder in the home repository is not the home project"
    );

    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let mut raise =
        |sid: &str, p: &Place| store.add_item(&session(sid), p, &question(sid)).unwrap();
    let (i_alpha, i_wt, i_beta, i_loose) = (
        raise("s1", &p_alpha),
        raise("s2", &p_wt),
        raise("s3", &p_beta),
        raise("s4", &p_loose),
    );

    let ids = |f: Filter| {
        store
            .items(&f)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    };
    let project = |p: &Place| Filter {
        project: Some(p.project.clone()),
        ..Filter::default()
    };
    let workspace = |p: &Place| Filter {
        workspace: Some(p.workspace.clone()),
        ..Filter::default()
    };
    assert_eq!(ids(project(&p_alpha)), vec![i_alpha, i_wt]);
    assert_eq!(ids(workspace(&p_alpha)), vec![i_alpha]);
    assert_eq!(ids(workspace(&p_wt)), vec![i_wt]);
    assert_eq!(ids(project(&p_beta)), vec![i_beta]);
    assert_eq!(ids(project(&p_loose)), vec![i_loose]);
    assert_eq!(
        ids(Filter {
            project: Some(home_shown),
            ..Filter::default()
        }),
        Vec::<i64>::new()
    );
}

/// Design: the item shows the author's or the owner's status, whichever was written last.
#[test]
fn status_is_the_last_written_of_author_and_owner() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    let status = |store: &Store| store.items(&Filter::default()).unwrap()[0].status.clone();
    assert_eq!(status(&store), "open");
    store
        .owner_answer(item, &json!({ "pick": "a" }), None)
        .unwrap();
    assert_eq!(status(&store), "answered");
    store
        .write_as_session(&me, &here, item, Field::Status, &json!("closed"), None)
        .unwrap();
    assert_eq!(status(&store), "closed");
    let open = Filter {
        status: Some("open".into()),
        ..Filter::default()
    };
    assert!(store.items(&open).unwrap().is_empty());
}
