//! The page's server: the owner's way onto the board, on 127.0.0.1 only.
//!
//! The owner writes only here, and only two things: the answer and the owner's status. There is
//! no route that writes a session's cell. The command writes to the database past this server,
//! so the change feed reads the database, never an in-server event.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::names::{NameWatcher, tool_of};
use crate::store::{Cell, Filter, ItemId, Store, WriteError, Writer, label_in};
use std::path::PathBuf;

/// The header every write must carry. A cross-site form cannot send a custom header.
pub const WRITE_HEADER: &str = "x-focus-board";

const PAGE: &str = include_str!("page.html");

/// How many timeline rows one answer holds by default, and at most. Nothing is ever deleted, so
/// without a cut every refresh would ship the whole history (review R8). A display cut only.
pub const TIMELINE_DEFAULT: i64 = 300;
pub const TIMELINE_MAX: i64 = 5000;

#[derive(Clone)]
struct App {
    store: Arc<Mutex<Store>>,
    /// The only `Host` values accepted: this machine's loopback names with the bound port.
    hosts: Arc<[String; 2]>,
    /// Counts the renames the watch made, so the page knows to refresh (`/api/changes`).
    renames: Arc<AtomicU64>,
    /// Where each session's tool is told from (#361): Claude Code's transcripts, Copilot's
    /// session state. `None` without a name watcher.
    tools: Arc<Option<(PathBuf, Option<PathBuf>)>>,
}

/// How often the server looks for renamed sessions.
pub const RENAME_WATCH: Duration = Duration::from_secs(2);

/// Serves the board on `listener` until the process ends. The caller binds the listener, to
/// 127.0.0.1 only.
/// With a `watcher`, the server follows sessions' renames.
pub async fn serve(
    listener: TcpListener,
    store: Store,
    watcher: Option<NameWatcher>,
) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    let app = App {
        store: Arc::new(Mutex::new(store)),
        hosts: Arc::new([format!("127.0.0.1:{port}"), format!("localhost:{port}")]),
        renames: Arc::new(AtomicU64::new(0)),
        tools: Arc::new(watcher.as_ref().map(NameWatcher::dirs)),
    };
    if let Some(watcher) = watcher {
        tokio::spawn(watch_renames(app.clone(), watcher));
    }
    axum::serve(listener, router(app)).await
}

/// Every [`RENAME_WATCH`], gives each session the name in its transcript. The transcripts are
/// read without holding the board, which is locked only to list sessions and to rename.
async fn watch_renames(app: App, watcher: NameWatcher) {
    let watcher = Arc::new(Mutex::new(watcher));
    let mut tick = tokio::time::interval(RENAME_WATCH);
    loop {
        tick.tick().await;
        let (app, watcher) = (app.clone(), watcher.clone());
        let _ = tokio::task::spawn_blocking(move || {
            let sessions = match app.store.lock() {
                Ok(store) => match store.session_names() {
                    Ok(sessions) => sessions,
                    Err(_) => return,
                },
                Err(_) => return,
            };
            let changed = match watcher.lock() {
                Ok(mut watcher) => watcher.changed(&sessions),
                Err(_) => return,
            };
            if changed.is_empty() {
                return;
            }
            if let Ok(mut store) = app.store.lock() {
                let renamed = changed
                    .iter()
                    .filter(|(id, name)| store.rename_session(id, name).unwrap_or(false))
                    .count();
                if renamed > 0 {
                    app.renames.fetch_add(renamed as u64, Ordering::Relaxed);
                }
            }
        })
        .await;
    }
}

fn router(app: App) -> Router {
    Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/api/items", get(items))
        .route("/api/tree", get(tree))
        .route("/api/timeline", get(timeline))
        .route("/api/history", get(history))
        .route("/api/changes", get(changes))
        .route("/api/answer", post(answer))
        .route("/api/status", post(status))
        .route("/api/review", get(review).post(ask_review))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .layer(middleware::map_response(never_framed))
        .with_state(app)
}

/// No other page may frame the board: a framed board's own writes would pass the guard, so a
/// lured click could answer or close an item (review R2). Set on every response, refusals too.
async fn never_framed(mut res: Response) -> Response {
    let headers = res.headers_mut();
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

async fn guard(State(app): State<App>, req: Request, next: Next) -> Response {
    match check(&app.hosts[..], req.method(), req.headers()) {
        Ok(()) => next.run(req).await,
        Err(why) => {
            eprintln!(
                "focus-board serve: refused {} {}: {why}",
                req.method(),
                req.uri().path()
            );
            (StatusCode::FORBIDDEN, Json(json!({ "error": why }))).into_response()
        }
    }
}

/// The brief's rule 4. Every request, reads included, must name this machine's board in
/// `Host`; a DNS-rebinding page sends its own name there. Every write must also carry the same
/// origin and the custom header, which a cross-site form cannot.
fn check(hosts: &[String], method: &Method, headers: &HeaderMap) -> Result<(), &'static str> {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let host = text(header::HOST).ok_or("no Host")?;
    if !hosts.iter().any(|h| h == host) {
        return Err("the Host is not this machine's board");
    }
    if method == Method::GET || method == Method::HEAD {
        return Ok(());
    }
    let origin = text(header::ORIGIN).ok_or("a write needs an Origin")?;
    if origin != format!("http://{host}") {
        return Err("the Origin does not match the board");
    }
    if text(header::HeaderName::from_static(WRITE_HEADER)) != Some("1") {
        return Err("a write needs the X-Focus-Board header");
    }
    Ok(())
}

struct ApiError(StatusCode, Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<WriteError> for ApiError {
    fn from(e: WriteError) -> Self {
        let code = match &e {
            WriteError::Stale { .. } => StatusCode::CONFLICT,
            WriteError::NotYours(_) => StatusCode::FORBIDDEN,
            WriteError::BadValue(_) => StatusCode::BAD_REQUEST,
            WriteError::NoSuchItem(_) => StatusCode::NOT_FOUND,
            WriteError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let mut body = json!({ "error": e.to_string() });
        if let WriteError::Stale { current } = e {
            body["current"] = json!(current);
        }
        ApiError(code, body)
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": e.to_string() }),
        )
    }
}

fn bad(why: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, json!({ "error": why }))
}

type ApiResult = Result<Json<Value>, ApiError>;

/// Runs `f` on the board off the async threads: a write elsewhere can hold SQLite's lock for a
/// moment, and the wait must not stall the server.
async fn on_store<T: Send + 'static>(
    app: &App,
    f: impl FnOnce(&mut Store) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        let mut store = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut store)
    })
    .await
    .map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": e.to_string() }),
        )
    })?
}

fn number(q: &HashMap<String, String>, key: &str) -> Result<Option<i64>, ApiError> {
    match q.get(key).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => v
            .trim_start_matches(['B', 'b'])
            .parse()
            .map(Some)
            .map_err(|_| bad("not a number")),
    }
}

fn filter(q: &HashMap<String, String>) -> Result<Filter, ApiError> {
    let text = |key: &str| q.get(key).filter(|v| !v.is_empty()).cloned();
    Ok(Filter {
        project: text("project"),
        workspace: text("workspace"),
        writer: number(q, "writer")?,
        status: text("status"),
    })
}

fn cell_json(c: &Cell) -> Value {
    json!({
        "seq": c.seq,
        "item": c.item,
        "writer": c.writer,
        "field": c.field,
        "version": c.version,
        "value": c.value,
        "client": c.client,
        "written_at": c.written_at,
    })
}

/// Items matching the filter, each with the current version of every cell.
async fn items(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let filter = filter(&q)?;
    // `q`: full text search over every version of every cell (layout 2).
    let typed = q
        .get("q")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    on_store(&app, move |store| {
        let found: Option<std::collections::HashSet<ItemId>> = match &typed {
            Some(typed) => Some(store.search(typed)?.into_iter().collect()),
            None => None,
        };
        let mut out = Vec::new();
        for i in store.items(&filter)? {
            if found.as_ref().is_some_and(|f| !f.contains(&i.id)) {
                continue;
            }
            let cells: Vec<Value> = store.current(i.id)?.iter().map(cell_json).collect();
            out.push(json!({
                "id": i.id,
                "label": i.label,
                "parent": i.parent,
                "author": i.author,
                "question": i.question,
                "status": i.status,
                "project": i.project,
                "workspace": i.workspace,
                "branch": i.branch,
                "cells": cells,
            }));
        }
        Ok(Json(json!({ "last": store.last_seq()?, "items": out })))
    })
    .await
}

/// Every writer, with its label, its parent and when it last wrote.
async fn tree(State(app): State<App>) -> ApiResult {
    let tools = app.tools.clone();
    on_store(&app, move |store| {
        let last: HashMap<i64, (i64, String)> = store
            .last_writes()?
            .into_iter()
            .map(|(w, seq, at)| (w, (seq, at)))
            .collect();
        let writers = store.writers()?;
        let rows: Vec<Value> = writers
            .iter()
            .map(|w: &Writer| {
                let (last_seq, last_at) = last.get(&w.id).cloned().unzip();
                json!({
                    "id": w.id,
                    "kind": w.kind,
                    "label": label_in(&writers, w.id),
                    "session_id": w.session_id,
                    "agent_id": w.agent_id,
                    "agent_type": w.agent_type,
                    "parent": w.parent,
                    "project": w.project,
                    "workspace": w.workspace,
                    "name": w.name,
                    "tool": tools.as_ref().as_ref().and_then(|(projects, copilot)| {
                        tool_of(projects, copilot.as_deref(), &w.session_id)
                    }),
                    "last_seq": last_seq,
                    "last_at": last_at,
                })
            })
            .collect();
        Ok(Json(json!({ "last": store.last_seq()?, "writers": rows })))
    })
    .await
}

/// The newest `limit` writes after `after` (default [`TIMELINE_DEFAULT`]), in the order written,
/// narrowed by place and node, with the total so the page can say what it left out.
async fn timeline(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let filter = filter(&q)?;
    let after = number(&q, "after")?.unwrap_or(0);
    let limit = number(&q, "limit")?
        .unwrap_or(TIMELINE_DEFAULT)
        .clamp(1, TIMELINE_MAX);
    on_store(&app, move |store| {
        let entries: Vec<Value> = store
            .timeline(&filter, after, Some(limit))?
            .iter()
            .map(cell_json)
            .collect();
        Ok(Json(json!({
            "last": store.last_seq()?,
            "total": store.timeline_len(&filter)?,
            "entries": entries,
        })))
    })
    .await
}

/// Every version of every cell of one item.
async fn history(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let item: ItemId = number(&q, "item")?.ok_or(bad("which item?"))?;
    on_store(&app, move |store| {
        let cells = store.history(item)?;
        if cells.is_empty() {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                json!({ "error": format!("no item B{item}") }),
            ));
        }
        Ok(Json(
            json!({ "cells": cells.iter().map(cell_json).collect::<Vec<_>>() }),
        ))
    })
    .await
}

/// The change feed: the board's last row, so the page refetches only when it has moved, and the
/// count of renames, which move no row.
async fn changes(State(app): State<App>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let after = number(&q, "after")?.unwrap_or(0);
    let renames = app.renames.load(Ordering::Relaxed);
    on_store(&app, move |store| {
        let last = store.last_seq()?;
        Ok(Json(json!({
            "last": last,
            "changed": last > after,
            "renames": renames,
            "reviews": store.review_mark()?,
        })))
    })
    .await
}

/// The owner's newest review request and the sessions it is for, each with its items not closed
/// and whether, how and when it saw the request (owner, 2026-11-22, B22 "b").
async fn review(State(app): State<App>) -> ApiResult {
    on_store(&app, move |store| {
        let latest = store.latest_review()?;
        let sessions: Vec<Value> = store
            .review_targets()?
            .into_iter()
            .map(|t| {
                let items: Vec<Value> = t
                    .items
                    .iter()
                    .map(|(id, status)| json!({ "id": format!("B{id}"), "status": status }))
                    .collect();
                json!({
                    "writer": t.writer,
                    "items": items,
                    "seen": t.seen.map(|(how, at)| json!({ "how": how, "at": at })),
                })
            })
            .collect();
        Ok(Json(json!({
            "latest": latest.map(|r| json!({ "id": r.id, "asked_at": r.asked_at })),
            "sessions": sessions,
        })))
    })
    .await
}

/// One press of the page's review button: asks every session with items not closed to review
/// them. Guarded like every write.
async fn ask_review(State(app): State<App>) -> ApiResult {
    on_store(&app, move |store| {
        let request = store.ask_for_review()?;
        Ok(Json(
            json!({ "id": request.id, "asked_at": request.asked_at }),
        ))
    })
    .await
}

fn item_and_expect(body: &Value) -> Result<(ItemId, Option<i64>), ApiError> {
    let item = body
        .get("item")
        .and_then(Value::as_i64)
        .ok_or(bad("\"item\" is a number"))?;
    let expect = match body.get("expect") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or(bad("\"expect\" is a version number"))?),
    };
    Ok((item, expect))
}

/// The owner's answer: `{"item": 3, "answer": {"pick": "a", "text": "…"}, "expect": 1}`.
async fn answer(State(app): State<App>, Json(body): Json<Value>) -> ApiResult {
    let (item, expect) = item_and_expect(&body)?;
    let answer = body
        .get("answer")
        .cloned()
        .ok_or(bad("\"answer\" is missing"))?;
    on_store(&app, move |store| {
        let version = store.owner_answer(item, &answer, expect)?;
        Ok(Json(
            json!({ "version": version, "last": store.last_seq()? }),
        ))
    })
    .await
}

/// The owner's status: `{"item": 3, "status": "closed", "expect": 1}`.
async fn status(State(app): State<App>, Json(body): Json<Value>) -> ApiResult {
    let (item, expect) = item_and_expect(&body)?;
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .ok_or(bad("\"status\" is text"))?
        .to_string();
    on_store(&app, move |store| {
        let version = store.owner_status(item, &status, expect)?;
        Ok(Json(
            json!({ "version": version, "last": store.last_seq()? }),
        ))
    })
    .await
}
