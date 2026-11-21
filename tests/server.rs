//! The page's server, over real TCP with hand-written requests, so every header is exactly what
//! the test says: a browser, a cross-site form and a DNS-rebinding page differ only in headers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use focus_board::identity::{Identity, Place};
use focus_board::store::{Field, NewItem, OWNER, Store};
use serde_json::{Value, json};

struct Board {
    _dir: tempfile::TempDir,
    db: PathBuf,
    port: u16,
}

impl Board {
    /// A fresh board with one item raised by session `s1`, and its server on a free port.
    fn start() -> Board {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("board.sqlite");
        let item = NewItem {
            question: "Keep SQLite?".into(),
            options: vec![("a".into(), "yes".into()), ("b".into(), "no".into())],
            recommended: Some("a".into()),
            ..NewItem::default()
        };
        Store::open(&db)
            .unwrap()
            .add_item(&session("s1"), &here(), &item)
            .unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let store = Store::open(&db).unwrap();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                focus_board::server::serve(listener, store).await.unwrap();
            });
        });
        Board {
            _dir: dir,
            db,
            port,
        }
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Headers a real page on the board sends with a write.
    fn page_headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Host", self.host()),
            ("Origin", format!("http://{}", self.host())),
            ("X-Focus-Board", "1".into()),
        ]
    }

    fn get(&self, path: &str) -> (u16, Value) {
        request(self.port, "GET", path, &[("Host", self.host())], None)
    }

    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        request(
            self.port,
            "POST",
            path,
            &self.page_headers(),
            Some(&body.to_string()),
        )
    }

    fn store(&self) -> Store {
        Store::open(&self.db).unwrap()
    }
}

fn session(id: &str) -> Identity {
    Identity {
        session_id: id.into(),
        agent_id: String::new(),
        agent_type: String::new(),
    }
}

fn here() -> Place {
    Place {
        project: "/p".into(),
        workspace: "/p-wt".into(),
        branch: "feature".into(),
    }
}

/// One HTTP/1.1 request with exactly these headers; returns the status and the body as JSON
/// (or as a JSON string when it is not JSON).
fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> (u16, Value) {
    let response = raw_request(port, method, path, headers, body);
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let text = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("");
    (
        status,
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.into())),
    )
}

/// The whole response, status line and headers included.
fn raw_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(body.unwrap_or(""));
    stream.write_all(req.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// Review R2: no other page may frame the board, or a lured click could pass the guard as the
/// board's own write. Every response says so, refusals included.
#[test]
fn page_cannot_be_framed() {
    let board = Board::start();
    let good = [("Host", board.host())];
    let foreign = [("Host", "evil.example".to_string())];
    for (path, headers) in [
        ("/", &good),
        ("/api/items", &good),
        ("/api/items", &foreign),
    ] {
        let head = raw_request(board.port, "GET", path, headers, None).to_ascii_lowercase();
        let head = head.split("\r\n\r\n").next().unwrap_or("");
        for needed in [
            "x-frame-options: deny",
            "content-security-policy: frame-ancestors 'none'",
            "x-content-type-options: nosniff",
        ] {
            assert!(
                head.contains(needed),
                "{path} {headers:?} lacks {needed}:\n{head}"
            );
        }
    }
}

fn owner_answers(store: &Store) -> usize {
    store
        .history(1)
        .unwrap()
        .iter()
        .filter(|c| c.writer == OWNER && c.field == "answer")
        .count()
}

/// Brief: "The server refuses a write with a foreign Origin, a foreign Host, or no custom
/// header." Reads need this machine's Host too, or a DNS-rebinding page could read the board.
#[test]
fn server_guards() {
    let board = Board::start();
    let port = board.port;
    let host = board.host();
    let answer = json!({ "item": 1, "answer": { "pick": "a" } }).to_string();

    for good in [host.clone(), format!("localhost:{port}")] {
        let (code, _) = request(port, "GET", "/api/items", &[("Host", good.clone())], None);
        assert_eq!(code, 200, "a read with Host {good} was refused");
    }
    for foreign in [
        &[("Host", format!("evil.example:{port}"))][..],
        &[("Host", "127.0.0.1:1".into())],
        &[],
    ] {
        let (code, body) = request(port, "GET", "/api/items", foreign, None);
        assert_ne!(code, 200, "a read with {foreign:?} was served: {body}");
    }

    let origin = |o: String| ("Origin", o);
    let write_cases: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "foreign Origin",
            vec![
                ("Host", host.clone()),
                origin("http://evil.example".into()),
                ("X-Focus-Board", "1".into()),
            ],
        ),
        (
            "no Origin",
            vec![("Host", host.clone()), ("X-Focus-Board", "1".into())],
        ),
        (
            "Origin of the other loopback name",
            vec![
                ("Host", host.clone()),
                origin(format!("http://localhost:{port}")),
                ("X-Focus-Board", "1".into()),
            ],
        ),
        (
            "no custom header",
            vec![("Host", host.clone()), origin(format!("http://{host}"))],
        ),
        (
            "custom header not 1",
            vec![
                ("Host", host.clone()),
                origin(format!("http://{host}")),
                ("X-Focus-Board", "0".into()),
            ],
        ),
        (
            "DNS rebinding",
            vec![
                ("Host", format!("evil.example:{port}")),
                origin(format!("http://evil.example:{port}")),
                ("X-Focus-Board", "1".into()),
            ],
        ),
    ];
    for (case, headers) in &write_cases {
        let (code, body) = request(port, "POST", "/api/answer", headers, Some(&answer));
        assert_eq!(code, 403, "{case}: {body}");
    }
    assert_eq!(owner_answers(&board.store()), 0, "a refused write landed");

    let (code, body) = request(
        port,
        "POST",
        "/api/answer",
        &board.page_headers(),
        Some(&answer),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(owner_answers(&board.store()), 1);
}

/// The owner's two writes through the page, the version check, and what the page reads back.
#[test]
fn page_answers_and_reads() {
    let board = Board::start();

    let (code, body) = board.post(
        "/api/answer",
        json!({ "item": 1, "answer": { "pick": "b", "text": "why not" }, "expect": 0 }),
    );
    assert_eq!((code, &body["version"]), (200, &json!(1)), "{body}");
    let (code, body) = board.post(
        "/api/answer",
        json!({ "item": 1, "answer": { "pick": "a" }, "expect": 0 }),
    );
    assert_eq!(
        (code, &body["current"]),
        (409, &json!(1)),
        "a stale answer was taken: {body}"
    );
    let (code, _) = board.post(
        "/api/answer",
        json!({ "item": 1, "answer": { "pick": "z" } }),
    );
    assert_eq!(code, 400, "a pick outside the options was taken");
    let (code, _) = board.post(
        "/api/answer",
        json!({ "item": 99, "answer": { "pick": "a" } }),
    );
    assert_eq!(code, 404);

    let history = board.store().history(1).unwrap();
    let answers: Vec<_> = history.iter().filter(|c| c.field == "answer").collect();
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].client, "page",
        "every write is logged with its client"
    );
    assert_eq!(answers[0].value, json!({ "pick": "b", "text": "why not" }));

    let (_, items) = board.get("/api/items");
    assert_eq!(items["items"][0]["status"], "answered");
    let item = &items["items"][0];
    assert_eq!(
        (&item["project"], &item["workspace"], &item["branch"]),
        (&json!("/p"), &json!("/p-wt"), &json!("feature")),
        "the page gets where the item was raised"
    );
    let (code, _) = board.post("/api/status", json!({ "item": 1, "status": "closed" }));
    assert_eq!(code, 200);
    let (_, items) = board.get("/api/items?status=closed");
    assert_eq!(items["items"].as_array().unwrap().len(), 1);

    // There is no route that writes a session's cell.
    for path in ["/api/feedback", "/api/question", "/api/write"] {
        let (code, _) = board.post(path, json!({ "item": 1, "text": "x" }));
        assert_eq!(code, 404, "{path} answered");
    }
}

/// The command writes past the server, so the change feed must read the database: a write made
/// on another connection shows up, and the tree and timeline name it.
#[test]
fn page_sees_writes_made_past_it() {
    let board = Board::start();
    let (_, before) = board.get("/api/changes?after=0");
    let last = before["last"].as_i64().unwrap();
    assert!(last > 0 && before["changed"] == json!(true));
    let (_, same) = board.get(&format!("/api/changes?after={last}"));
    assert_eq!(same["changed"], json!(false));

    let fork = Identity {
        session_id: "s1".into(),
        agent_id: "f1".into(),
        agent_type: "fork".into(),
    };
    board
        .store()
        .write_as_session(
            &fork,
            &here(),
            1,
            Field::Feedback,
            &json!({ "text": "from the fork" }),
            None,
        )
        .unwrap();
    let (_, after) = board.get(&format!("/api/changes?after={last}"));
    assert_eq!(
        after["changed"],
        json!(true),
        "a write made past the server was missed"
    );

    let (_, tree) = board.get("/api/tree");
    let writers = tree["writers"].as_array().unwrap();
    let parent = writers
        .iter()
        .find(|w| w["session_id"] == "s1" && w["agent_id"] == "")
        .unwrap();
    let child = writers.iter().find(|w| w["agent_id"] == "f1").unwrap();
    assert_eq!(
        child["parent"], parent["id"],
        "the fork sits under its session"
    );
    assert_eq!(child["label"], "fork f1 in s1");
    assert!(
        child["last_seq"].as_i64().unwrap() > last,
        "the tree shows when the fork last wrote"
    );

    let (_, items) = board.get("/api/items");
    let cells = items["items"][0]["cells"].as_array().unwrap();
    assert!(
        cells
            .iter()
            .any(|c| c["field"] == "feedback" && c["writer"] == child["id"])
    );

    board.post(
        "/api/answer",
        json!({ "item": 1, "answer": { "pick": "a" } }),
    );
    let (_, timeline) = board.get(&format!("/api/timeline?writer={}", parent["id"]));
    let fields: Vec<&str> = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert!(
        fields.contains(&"feedback") && fields.contains(&"answer"),
        "branch timeline: {fields:?}"
    );
    let seqs: Vec<i64> = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "the timeline is in the order written"
    );

    let (code, history) = board.get("/api/history?item=1");
    assert_eq!(code, 200);
    assert!(history["cells"].as_array().unwrap().len() >= 5);
    assert_eq!(board.get("/api/history?item=42").0, 404);
}

/// `focus-board serve` binds 127.0.0.1 only and serves the page.
#[test]
fn serve_binds_loopback_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_focus-board"))
        .args(["serve", "--port", "0"])
        .env("FOCUS_BOARD_DB", dir.path().join("board.sqlite"))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let addr = first
        .trim()
        .strip_prefix("focus-board serve: http://")
        .unwrap_or_default()
        .to_string();
    let port: u16 = addr
        .strip_prefix("127.0.0.1:")
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let (code, page) = if port == 0 {
        (0, Value::Null)
    } else {
        request(port, "GET", "/", &[("Host", addr.clone())], None)
    };
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(port != 0, "serve printed {first:?}");
    assert_eq!(code, 200);
    assert!(page.as_str().unwrap_or("").contains("focus-board"));
}

/// Review R8: the timeline is cut to its newest rows and says how many there are; the items can be
/// narrowed to the ones not closed on the server.
#[test]
fn timeline_is_cut_and_says_so() {
    let board = Board::start();
    let (_, full) = board.get("/api/timeline");
    let total = full["total"].as_i64().unwrap();
    assert!(total >= 4, "{full}");
    assert_eq!(
        full["entries"].as_array().unwrap().len() as i64,
        total,
        "the default cut is above this board"
    );
    let (_, cut) = board.get("/api/timeline?limit=2");
    let seqs: Vec<i64> = cut["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    let all: Vec<i64> = full["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, all[all.len() - 2..].to_vec());
    assert_eq!(cut["total"], json!(total));

    board.post("/api/status", json!({ "item": 1, "status": "closed" }));
    let (_, active) = board.get("/api/items?status=active");
    assert_eq!(
        active["items"],
        json!([]),
        "a closed item left the active list"
    );
}
