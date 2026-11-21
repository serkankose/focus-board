//! The brief's tests that the store alone can prove. Each test names the brief's line it proves.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use focus_board::identity::{Identity, IdentityError, Place, fork_parent, made_up_name};
use focus_board::store::{
    Field, Filter, LAYOUT, NewItem, OWNER, OpenError, Store, WriteError, label_in,
};
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
        ..Default::default()
    }
}

fn agent(session_id: &str, agent_id: &str, agent_type: &str) -> Identity {
    Identity {
        session_id: session_id.into(),
        agent_id: agent_id.into(),
        agent_type: agent_type.into(),
        ..Default::default()
    }
}

fn place(project: &str, workspace: &str) -> Place {
    Place {
        project: project.into(),
        workspace: workspace.into(),
        branch: String::new(),
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
        .timeline(&branch, 0, None)
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
    let fork_rows = store.timeline(&fork_only, 0, None).unwrap();
    assert!(fork_rows.iter().all(|c| c.writer == f) && fork_rows.len() == 1);

    // The owner's answer on the parent's item belongs to the parent's branch, not the fork's.
    store
        .owner_answer(item, &json!({ "pick": "a" }), None)
        .unwrap();
    let in_branch = |filter: &Filter| {
        store
            .timeline(filter, 0, None)
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
    // Each checkout knows its branch: `git worktree add` names the new branch after the folder.
    assert_eq!(
        (
            p_alpha.branch.as_str(),
            p_wt.branch.as_str(),
            p_loose.branch.as_str()
        ),
        ("master", "alpha-wt", ""),
        "branches of the main checkout, the worktree, and a folder with no repository"
    );
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

/// Review R10: options become a map, so a repeated letter would silently keep only its last text.
#[test]
fn a_repeated_option_letter_is_refused() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let twice = NewItem {
        question: "q".into(),
        options: vec![("a".into(), "keep".into()), ("a".into(), "drop".into())],
        ..NewItem::default()
    };
    let refused = store.add_item(&session("s"), &place("p", "p"), &twice);
    assert!(
        matches!(&refused, Err(WriteError::BadValue(why)) if why.contains("twice")),
        "{refused:?}"
    );
    assert!(
        store.items(&Filter::default()).unwrap().is_empty(),
        "nothing was raised"
    );
}

/// Review G8: only the item's author writes its question, options and recommendation.
#[test]
fn only_the_author_writes_the_question() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let item = store
        .add_item(&session("author"), &here, &question("q"))
        .unwrap();
    for (field, value) in [
        (Field::Question, json!("rewritten")),
        (Field::Options, json!({ "a": "other" })),
        (Field::Recommended, json!("b")),
    ] {
        let wrote =
            store.write_as_session(&session("someone-else"), &here, item, field, &value, None);
        assert!(
            matches!(wrote, Err(WriteError::NotYours(_))),
            "{field:?}: {wrote:?}"
        );
    }
    let agent_of_author = agent("author", "a1", "general-purpose");
    let wrote = store.write_as_session(
        &agent_of_author,
        &here,
        item,
        Field::Question,
        &json!("x"),
        None,
    );
    assert!(
        matches!(wrote, Err(WriteError::NotYours(_))),
        "even the author's own subagent: {wrote:?}"
    );
}

/// Review R11: the board's layout version is kept, and a newer one is refused, not misread.
#[test]
fn a_newer_layout_is_refused() {
    let (_dir, db) = board();
    drop(Store::open(&db).unwrap());
    let raw = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = raw
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, LAYOUT, "a new board records its layout");
    raw.pragma_update(None, "user_version", LAYOUT + 1).unwrap();
    drop(raw);
    match Store::open(&db) {
        Err(OpenError::NewerLayout(v)) => assert_eq!(v, LAYOUT + 1),
        Err(other) => panic!("wrong refusal: {other}"),
        Ok(_) => panic!("a newer layout was opened"),
    }
}

/// Review R8: the timeline can keep only its newest rows, still in the order written, and says
/// how many there are; the items can be narrowed to the ones not closed.
#[test]
fn timeline_cut_and_active_filter() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let item = store.add_item(&me, &here, &question("q")).unwrap();
    for n in 1..=5 {
        store
            .write_as_session(
                &me,
                &here,
                item,
                Field::Feedback,
                &feedback(&n.to_string()),
                None,
            )
            .unwrap();
    }
    let all = store.timeline(&Filter::default(), 0, None).unwrap();
    let newest = store.timeline(&Filter::default(), 0, Some(2)).unwrap();
    assert_eq!(
        newest,
        all[all.len() - 2..].to_vec(),
        "the newest two, oldest of them first"
    );
    assert_eq!(
        store.timeline_len(&Filter::default()).unwrap(),
        all.len() as i64
    );

    let closed = store.add_item(&me, &here, &question("done")).unwrap();
    store
        .write_as_session(&me, &here, closed, Field::Status, &json!("closed"), None)
        .unwrap();
    let active = Filter {
        status: Some("active".into()),
        ..Filter::default()
    };
    let ids: Vec<i64> = store.items(&active).unwrap().iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![item]);
}

/// The owner's ask (2026-11-21): to decide on an item, see where it was raised. Each item keeps
/// the repository, checkout and branch of the command that raised it, as they were then, even
/// when its writer first wrote somewhere else.
#[test]
fn item_context_is_recorded_at_raise_time() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let me = session("s");
    let main = Place {
        project: "/repo".into(),
        workspace: "/repo".into(),
        branch: "master".into(),
    };
    let worktree = Place {
        project: "/repo".into(),
        workspace: "/repo/.claude/worktrees/fix".into(),
        branch: "fix-it".into(),
    };
    let first = store
        .add_item(&me, &main, &question("raised on master"))
        .unwrap();
    let second = store
        .add_item(&me, &worktree, &question("raised in the worktree"))
        .unwrap();
    let items = store.items(&Filter::default()).unwrap();
    let context = |id| {
        let i = items.iter().find(|i| i.id == id).unwrap();
        (i.project.clone(), i.workspace.clone(), i.branch.clone())
    };
    assert_eq!(
        context(first),
        ("/repo".into(), "/repo".into(), "master".into())
    );
    assert_eq!(
        context(second),
        (
            "/repo".into(),
            "/repo/.claude/worktrees/fix".into(),
            "fix-it".into()
        )
    );
    let writer = store
        .writers()
        .unwrap()
        .into_iter()
        .find(|w| w.session_id == "s")
        .unwrap();
    assert_eq!(
        writer.workspace, "/repo",
        "the writer stays where it first wrote"
    );
}

/// A board made before layout versions has tables but `user_version` 0; it is refused, not
/// read with a layout it does not have.
#[test]
fn an_unversioned_board_is_refused() {
    let (_dir, db) = board();
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, author INTEGER, label TEXT);")
        .unwrap();
    drop(raw);
    match Store::open(&db) {
        Err(OpenError::Unversioned) => {}
        Err(other) => panic!("wrong refusal: {other}"),
        Ok(_) => panic!("an unversioned board was opened"),
    }
}

/// Review R3, from the owner's /branch and /fork checks (Claude Code 2.1.329): a copied session
/// finds its parent in its transcript. Copied turns name the session that made them, so the last
/// such id other than its own is the immediate parent; `forkedFrom` is the fallback.
#[test]
fn fork_parent_reads_the_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, lines: &[serde_json::Value]| {
        let path = dir.path().join(name);
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, text).unwrap();
        path
    };
    let from_root = json!({ "type": "assistant", "sessionId": "x",
        "forkedFrom": { "sessionId": "root", "messageUuid": "m1" }, "requestSessionId": "root" });

    let root = write(
        "root.jsonl",
        &[json!({ "type": "assistant", "requestSessionId": "root" })],
    );
    assert_eq!(
        fork_parent(&root, "root"),
        None,
        "a session that is no copy"
    );

    let branch = write(
        "branch.jsonl",
        &[
            json!({ "type": "x" }),
            from_root.clone(),
            json!({ "type": "assistant" }),
        ],
    );
    assert_eq!(
        fork_parent(&branch, "br").as_deref(),
        Some("root"),
        "a /branch of root"
    );

    let copy = write(
        "copy.jsonl",
        &[
            from_root.clone(),
            json!({ "type": "assistant", "requestSessionId": "br" }),
            json!({ "type": "assistant", "requestSessionId": "cp" }),
        ],
    );
    assert_eq!(
        fork_parent(&copy, "cp").as_deref(),
        Some("br"),
        "the immediate parent, not the root"
    );

    let only = write(
        "only.jsonl",
        &[json!({ "forkedFrom": { "sessionId": "root" } })],
    );
    assert_eq!(
        fork_parent(&only, "x").as_deref(),
        Some("root"),
        "forkedFrom as the fallback"
    );

    assert_eq!(
        fork_parent(&dir.path().join("none.jsonl"), "x"),
        None,
        "no transcript"
    );
    let bad = write("bad.jsonl", &[json!({ "requestSessionId": "a'b" })]);
    assert_eq!(
        fork_parent(&bad, "x"),
        None,
        "an id with characters ids never have"
    );

    // Review C3: a line that is not UTF-8 is skipped, not taken as the end of the transcript.
    let mixed = dir.path().join("mixed.jsonl");
    let mut bytes = format!(
        "{}\n",
        json!({ "type": "assistant", "requestSessionId": "root" })
    )
    .into_bytes();
    bytes.extend_from_slice(b"{\"type\":\"assistant\",\"text\":\"\xff\xfe broken\"}\n");
    bytes.extend_from_slice(
        format!(
            "{}\n",
            json!({ "type": "assistant", "requestSessionId": "br" })
        )
        .as_bytes(),
    );
    std::fs::write(&mixed, bytes).unwrap();
    assert_eq!(
        fork_parent(&mixed, "cp").as_deref(),
        Some("br"),
        "read past a line that is not UTF-8"
    );
}

/// Review P8, P9: Windows paths are understood on any OS. A drive path is safe in the hook's
/// quotes, and Windows' `\\?\` form of a canonical path is written without the prefix.
#[test]
fn windows_paths_are_understood() {
    use focus_board::identity::{is_drive_path, is_safe_path, plain_path};
    for safe in [
        "/home/me/.claude/projects/p/s.jsonl",
        r"C:\Users\me\.claude\projects\p\s.jsonl",
        "C:/Users/me/s.jsonl",
    ] {
        assert!(is_safe_path(safe), "{safe}");
    }
    for unsafe_path in [
        r"relative\s.jsonl",
        r"C:\it's.jsonl",
        "C:",
        "s.jsonl",
        "/tmp/a\nb",
    ] {
        assert!(!is_safe_path(unsafe_path), "{unsafe_path:?}");
    }
    assert!(is_drive_path(r"d:\x") && !is_drive_path(r"\\server\share"));
    assert_eq!(plain_path(r"\\?\C:\Users\me\board"), r"C:\Users\me\board");
    assert_eq!(
        plain_path(r"\\?\UNC\server\share\board"),
        r"\\server\share\board"
    );
    assert_eq!(plain_path("/home/me/board"), "/home/me/board");
    assert_eq!(
        plain_path(r"\\?\Volume{x}\board"),
        r"\\?\Volume{x}\board",
        "only drive and UNC forms"
    );
}

/// Brief: "A fork writes as itself ... labelled with the session it was forked from" and "shows
/// under its parent in the tree". A copied session is filed under its parent, whose branch holds
/// the copy and its own agents at any depth; a parent that never wrote is made with it.
#[test]
fn a_copied_session_files_under_its_parent() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let item = store
        .add_item(&session("root"), &here, &question("q"))
        .unwrap();
    let copy = Identity {
        session_id: "copy".into(),
        parent_session: "root".into(),
        ..Identity::default()
    };
    store
        .write_as_session(
            &copy,
            &here,
            item,
            Field::Feedback,
            &feedback("from the copy"),
            None,
        )
        .unwrap();
    let copy_agent = Identity {
        session_id: "copy".into(),
        agent_id: "a1".into(),
        agent_type: "general-purpose".into(),
        ..Identity::default()
    };
    let sub = store
        .add_item(&copy_agent, &here, &question("raised by the copy's agent"))
        .unwrap();

    let writers = store.writers().unwrap();
    let id = |sid: &str, aid: &str| {
        writers
            .iter()
            .find(|w| w.session_id == sid && w.agent_id == aid)
            .unwrap()
            .id
    };
    let (r, c, a) = (id("root", ""), id("copy", ""), id("copy", "a1"));
    let parent_of = |w: i64| writers.iter().find(|x| x.id == w).unwrap().parent;
    assert_eq!((parent_of(c), parent_of(a)), (Some(r), Some(c)));
    assert_eq!(
        label_in(&writers, c),
        format!(
            "{} (copy of {})",
            made_up_name("copy"),
            made_up_name("root")
        )
    );

    let branch = Filter {
        writer: Some(r),
        ..Filter::default()
    };
    let ids: Vec<i64> = store.items(&branch).unwrap().iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        vec![item, sub],
        "the root's branch holds its copy's agent's item"
    );
    let wrote: Vec<i64> = store
        .timeline(&branch, 0, None)
        .unwrap()
        .iter()
        .map(|c| c.writer)
        .collect();
    assert!(wrote.contains(&c) && wrote.contains(&a), "{wrote:?}");

    let orphan = Identity {
        session_id: "copy2".into(),
        parent_session: "never-wrote".into(),
        ..Identity::default()
    };
    store
        .write_as_session(&orphan, &here, item, Field::Feedback, &feedback("x"), None)
        .unwrap();
    let writers = store.writers().unwrap();
    let never = writers
        .iter()
        .find(|w| w.session_id == "never-wrote")
        .unwrap()
        .id;
    let copy2 = writers.iter().find(|w| w.session_id == "copy2").unwrap();
    assert_eq!(copy2.parent, Some(never));
}

/// Review X1: an item is filed where it shows it was raised. A writer that first wrote in
/// worktree A raises an item in worktree B; the workspace filter finds it under B, not A, in the
/// listing and in the timeline alike.
#[test]
fn an_item_is_filed_where_it_was_raised() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let me = session("s");
    let a = Place {
        project: "/repo".into(),
        workspace: "/repo".into(),
        branch: "master".into(),
    };
    let b = Place {
        project: "/repo".into(),
        workspace: "/repo-b".into(),
        branch: "b".into(),
    };
    let first = store.add_item(&me, &a, &question("raised in A")).unwrap();
    let second = store.add_item(&me, &b, &question("raised in B")).unwrap();
    let in_ws = |ws: &str| Filter {
        workspace: Some(ws.into()),
        ..Filter::default()
    };
    let ids = |f: &Filter| {
        store
            .items(f)
            .unwrap()
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&in_ws("/repo")), vec![first]);
    assert_eq!(ids(&in_ws("/repo-b")), vec![second]);
    let items_in_b: Vec<i64> = store
        .timeline(&in_ws("/repo-b"), 0, None)
        .unwrap()
        .iter()
        .map(|c| c.item)
        .collect();
    assert!(
        !items_in_b.is_empty() && items_in_b.iter().all(|&i| i == second),
        "{items_in_b:?}"
    );
}

/// The owner's ask (2026-11-21, "#189/a"): a session shows its name, never a hash. The name is
/// the last `/rename` title in its transcript, else Claude's last label.
#[test]
fn session_names_read_from_the_transcript() {
    use focus_board::identity::session_name;
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, lines: &[serde_json::Value]| {
        let path = dir.path().join(name);
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, text).unwrap();
        path
    };
    let label = |n: &str| json!({ "type": "agent-name", "agentName": n, "sessionId": "s" });
    let title = |t: &str| json!({ "type": "custom-title", "customTitle": t, "sessionId": "s" });

    let renamed = write(
        "renamed.jsonl",
        &[
            label("check branch activity"),
            title("Lens"),
            title("Lens - da Reviewer"),
        ],
    );
    assert_eq!(
        session_name(&renamed).as_deref(),
        Some("Lens - da Reviewer"),
        "the last title wins"
    );
    let labelled = write("labelled.jsonl", &[label("code snippet analysis")]);
    assert_eq!(
        session_name(&labelled).as_deref(),
        Some("code snippet analysis"),
        "else the label"
    );
    let none = write("none.jsonl", &[json!({ "type": "assistant" })]);
    assert_eq!(session_name(&none), None);
    let messy = write(
        "messy.jsonl",
        &[title(&format!("two\nlines {}", "x".repeat(200)))],
    );
    let cleaned = session_name(&messy).unwrap();
    assert!(
        !cleaned.contains('\n') && cleaned.chars().count() <= 80,
        "{cleaned:?}"
    );
}

/// A session with no name of its own gets a fixed two-word name made from its id: the same id
/// gives the same name, and it never looks like a hash.
#[test]
fn made_up_names_are_fixed_and_never_hashes() {
    let one = made_up_name("5d1da7e2-7908-4583-b5a6-56e1b1b8ec86");
    assert_eq!(one, made_up_name("5d1da7e2-7908-4583-b5a6-56e1b1b8ec86"));
    let words: Vec<&str> = one.split('-').collect();
    assert!(
        words.len() == 2
            && words
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase())),
        "{one}"
    );
    let names: std::collections::HashSet<String> = (0..50)
        .map(|n| made_up_name(&format!("session-{n}")))
        .collect();
    assert!(names.len() > 40, "names spread out: {}", names.len());
}

/// A session's name follows a rename, an agent's write leaves its session's name alone, and a
/// writer from before names existed gets its made-up name when the board is opened.
#[test]
fn a_session_name_follows_a_rename() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let here = place("p", "p");
    let named = |n: &str| Identity {
        session_id: "s".into(),
        name: n.into(),
        ..Identity::default()
    };
    let item = store
        .add_item(&named("Smith"), &here, &question("q"))
        .unwrap();
    let name_of = |store: &Store| {
        store
            .writers()
            .unwrap()
            .into_iter()
            .find(|w| w.session_id == "s" && w.agent_id.is_empty())
            .unwrap()
            .name
    };
    assert_eq!(name_of(&store), "Smith");
    store
        .write_as_session(
            &named("Smith - the genius coder"),
            &here,
            item,
            Field::Feedback,
            &feedback("x"),
            None,
        )
        .unwrap();
    assert_eq!(
        name_of(&store),
        "Smith - the genius coder",
        "a rename is followed"
    );
    store
        .write_as_session(
            &named(""),
            &here,
            item,
            Field::Feedback,
            &feedback("y"),
            None,
        )
        .unwrap();
    assert_eq!(
        name_of(&store),
        "Smith - the genius coder",
        "no name read: the name stays"
    );
    let agent = Identity {
        session_id: "s".into(),
        agent_id: "a1".into(),
        agent_type: "fork".into(),
        name: "Other".into(),
        ..Identity::default()
    };
    store
        .write_as_session(&agent, &here, item, Field::Feedback, &feedback("z"), None)
        .unwrap();
    assert_eq!(
        name_of(&store),
        "Smith - the genius coder",
        "an agent does not rename its session"
    );
    let unnamed = session("never-named");
    store.add_item(&unnamed, &here, &question("q2")).unwrap();
    let writers = store.writers().unwrap();
    let w = writers
        .iter()
        .find(|w| w.session_id == "never-named")
        .unwrap();
    assert_eq!(w.name, made_up_name("never-named"));

    drop(store);
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute(
        "UPDATE writers SET name = '' WHERE session_id = 'never-named'",
        [],
    )
    .unwrap();
    drop(raw);
    let store = Store::open(&db).unwrap();
    let filled = store
        .writers()
        .unwrap()
        .into_iter()
        .find(|w| w.session_id == "never-named")
        .unwrap();
    assert_eq!(
        filled.name,
        made_up_name("never-named"),
        "an old writer gets its name on open"
    );
}

/// The owner's ask (2026-11-21): "we need full text search". Every word must match the start of a
/// word, in any version of any cell: question, options, the owner's answer, any feedback.
#[test]
fn full_text_search_finds_any_cell_text() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let (me, here) = (session("s"), place("p", "p"));
    let first = NewItem {
        question: "Keep SQLite, or store the board another way?".into(),
        options: vec![
            ("a".into(), "keep it".into()),
            ("b".into(), "append-only files".into()),
        ],
        ..NewItem::default()
    };
    let item = store.add_item(&me, &here, &first).unwrap();
    let other = store
        .add_item(&me, &here, &question("Which port?"))
        .unwrap();
    store
        .write_as_session(
            &me,
            &here,
            item,
            Field::Feedback,
            &feedback("the indexer is slow"),
            None,
        )
        .unwrap();
    store
        .write_as_session(
            &me,
            &here,
            item,
            Field::Feedback,
            &feedback("now it is quick"),
            None,
        )
        .unwrap();
    store
        .owner_answer(other, &json!({ "text": "use eighty eight ten" }), None)
        .unwrap();

    let found = |typed: &str| store.search(typed).unwrap();
    assert_eq!(found("sqli"), vec![item], "the start of a word");
    assert_eq!(
        found("append files"),
        vec![item],
        "options' text, every word"
    );
    assert_eq!(
        found("indexer slow"),
        vec![item],
        "an older version of feedback counts"
    );
    assert_eq!(found("eighty"), vec![other], "the owner's answer");
    assert!(
        found("sqlite eighty").is_empty(),
        "every word must match in one item"
    );
    assert!(
        found("\"; DROP TABLE cells; --").is_empty(),
        "odd characters mean nothing"
    );
    assert!(found("   ").is_empty() && found("--").is_empty());
}

/// A board from layout 1 is brought to layout 2 on open: the full text index is built from the
/// cells already there, and new cells are indexed as they come.
#[test]
fn an_older_board_is_brought_to_layout_2() {
    let (_dir, db) = board();
    let mut store = Store::open(&db).unwrap();
    let item = store
        .add_item(
            &session("s"),
            &place("p", "p"),
            &question("Rename the branch?"),
        )
        .unwrap();
    drop(store);
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute_batch("DROP TRIGGER cells_indexed; DROP TABLE cell_text; PRAGMA user_version = 1;")
        .unwrap();
    drop(raw);

    let mut store = Store::open(&db).unwrap();
    assert_eq!(
        store.search("rename branch").unwrap(),
        vec![item],
        "old cells were indexed"
    );
    let raw = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = raw
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, LAYOUT);
    let later = store
        .add_item(
            &session("s"),
            &place("p", "p"),
            &question("Merge the docs?"),
        )
        .unwrap();
    assert_eq!(
        store.search("docs").unwrap(),
        vec![later],
        "new cells are indexed too"
    );
}
