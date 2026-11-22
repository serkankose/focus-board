//! The board's database: writers, items, and every version of every cell.
//!
//! All writes go through this module, from the command and from the server alike. Each write is
//! one `BEGIN IMMEDIATE` transaction; SQLite triggers refuse every change or delete of stored
//! history, so nothing is lost even if code here goes wrong.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::identity::{Identity, Place, made_up_name};

pub type WriterId = i64;
pub type ItemId = i64;

/// The owner's writer row. There is exactly one, created with the schema.
pub const OWNER: WriterId = 1;

const SCHEMA: &str = r#"
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS writers (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('owner', 'session')),
    session_id  TEXT NOT NULL,
    agent_id    TEXT NOT NULL DEFAULT '',
    agent_type  TEXT NOT NULL DEFAULT '',
    parent      INTEGER REFERENCES writers(id),
    project     TEXT NOT NULL,
    workspace   TEXT NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    UNIQUE (session_id, agent_id)
);
INSERT INTO writers (id, kind, session_id, project, workspace, created_at)
    SELECT 1, 'owner', '', '', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE NOT EXISTS (SELECT 1 FROM writers WHERE id = 1);
CREATE TABLE IF NOT EXISTS items (
    id         INTEGER PRIMARY KEY,
    parent     INTEGER REFERENCES items(id),
    author     INTEGER NOT NULL REFERENCES writers(id),
    label      TEXT NOT NULL DEFAULT '',
    -- Where the item was raised, as it was then: the repository, the checkout, its branch.
    project    TEXT NOT NULL DEFAULT '',
    workspace  TEXT NOT NULL DEFAULT '',
    branch     TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS cells (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    item        INTEGER NOT NULL REFERENCES items(id),
    writer      INTEGER NOT NULL REFERENCES writers(id),
    field       TEXT NOT NULL CHECK (field IN
                    ('question', 'options', 'recommended', 'status', 'answer', 'feedback')),
    version     INTEGER NOT NULL CHECK (version >= 1),
    value       TEXT NOT NULL,
    client      TEXT NOT NULL CHECK (client IN ('page', 'cli')),
    written_at  TEXT NOT NULL,
    UNIQUE (item, writer, field, version)
);
CREATE TRIGGER IF NOT EXISTS cells_never_changed BEFORE UPDATE ON cells
    BEGIN SELECT RAISE(ABORT, 'cells are never changed'); END;
CREATE TRIGGER IF NOT EXISTS cells_never_deleted BEFORE DELETE ON cells
    BEGIN SELECT RAISE(ABORT, 'cells are never deleted'); END;
CREATE TRIGGER IF NOT EXISTS items_never_changed BEFORE UPDATE ON items
    BEGIN SELECT RAISE(ABORT, 'items are never changed'); END;
CREATE TRIGGER IF NOT EXISTS items_never_deleted BEFORE DELETE ON items
    BEGIN SELECT RAISE(ABORT, 'items are never deleted'); END;
CREATE TRIGGER IF NOT EXISTS writers_keep_identity
    BEFORE UPDATE OF id, kind, session_id, agent_id, agent_type, parent, project, workspace,
                     created_at ON writers
    BEGIN SELECT RAISE(ABORT, 'a writer keeps its identity; only its name changes'); END;
CREATE TRIGGER IF NOT EXISTS writers_never_deleted BEFORE DELETE ON writers
    BEGIN SELECT RAISE(ABORT, 'writers are never deleted'); END;
-- REPLACE removes a clashing row without firing DELETE triggers, so inserts are checked too: a
-- cell only gains its next version, and no id or identity is ever taken over. An auto-assigned
-- id is -1 inside a BEFORE INSERT trigger, so it never matches a stored row.
CREATE TRIGGER IF NOT EXISTS cells_only_append BEFORE INSERT ON cells
    WHEN EXISTS (SELECT 1 FROM cells WHERE id = NEW.id)
      OR NEW.version != (SELECT COALESCE(MAX(version), 0) + 1 FROM cells
                         WHERE item = NEW.item AND writer = NEW.writer AND field = NEW.field)
    BEGIN SELECT RAISE(ABORT, 'a cell only gains its next version'); END;
CREATE TRIGGER IF NOT EXISTS items_only_append BEFORE INSERT ON items
    WHEN EXISTS (SELECT 1 FROM items WHERE id = NEW.id)
    BEGIN SELECT RAISE(ABORT, 'an item is never replaced'); END;
CREATE TRIGGER IF NOT EXISTS writers_only_append BEFORE INSERT ON writers
    WHEN EXISTS (SELECT 1 FROM writers WHERE id = NEW.id
                 OR (session_id = NEW.session_id AND agent_id = NEW.agent_id))
    BEGIN SELECT RAISE(ABORT, 'a writer is never replaced'); END;
COMMIT;
"#;

/// The board's layout version, kept in SQLite's `user_version` and set by [`SCHEMA`]. A board
/// written by a newer focus-board is refused rather than read with the wrong layout. Since the
/// triggers forbid changing rows in place, a layout change is a new version (review R11).
pub const LAYOUT: i64 = 4;

/// The rows a timeline draws from: `?1` after this row, `?2` project, `?3` workspace, `?4` node.
/// A node's timeline holds the writes on the items raised in that branch, whoever made them (the
/// owner's answers included), and the branch's own writes on other items.
const TIMELINE_ROWS: &str = "FROM cells c JOIN items i ON i.id = c.item
     JOIN writers a ON a.id = i.author JOIN writers w ON w.id = c.writer
     WHERE c.id > ?1
       AND (?2 IS NULL OR i.project = ?2)
       AND (?3 IS NULL OR i.workspace = ?3)
       AND (?4 IS NULL OR a.id IN branch OR w.id IN branch)";

/// A tree node's branch, at any depth: the writer `?4`, its agents and the sessions copied from
/// it, their agents, and so on. Goes in front of a query that uses [`TIMELINE_ROWS`].
const BRANCH_OF_4: &str = "WITH RECURSIVE branch(id) AS (
     SELECT ?4 UNION SELECT w2.id FROM writers w2 JOIN branch b ON w2.parent = b.id)";

#[derive(Debug)]
pub enum OpenError {
    Db(rusqlite::Error),
    /// The board's layout is newer than this program knows.
    NewerLayout(i64),
    /// The board has tables but no layout version: it was made by a draft from before versions.
    Unversioned,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Db(e) => write!(f, "database: {e}"),
            OpenError::NewerLayout(v) => write!(
                f,
                "the board's layout is version {v}, and this focus-board knows only up to \
                 version {LAYOUT}: update focus-board"
            ),
            OpenError::Unversioned => write!(
                f,
                "the board was made by a focus-board from before layout versions; start a new \
                 board (the old one is kept as it is)"
            ),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<rusqlite::Error> for OpenError {
    fn from(e: rusqlite::Error) -> Self {
        OpenError::Db(e)
    }
}

/// Which way a write came in. The owner writes only through the page, sessions only through the
/// command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Client {
    Page,
    Cli,
}

impl Client {
    fn as_str(self) -> &'static str {
        match self {
            Client::Page => "page",
            Client::Cli => "cli",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Question,
    Options,
    Recommended,
    Status,
    Answer,
    Feedback,
}

impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Field::Question => "question",
            Field::Options => "options",
            Field::Recommended => "recommended",
            Field::Status => "status",
            Field::Answer => "answer",
            Field::Feedback => "feedback",
        }
    }
}

#[derive(Debug)]
pub enum WriteError {
    /// The cell has moved since the writer read it. Read again; nothing was written.
    Stale {
        current: i64,
    },
    /// This writer may not write this cell.
    NotYours(&'static str),
    /// The value is not one this field takes.
    BadValue(String),
    NoSuchItem(ItemId),
    Db(rusqlite::Error),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Stale { current } => write!(
                f,
                "the cell has moved to version {current} since you read it; read it again"
            ),
            WriteError::NotYours(why) => write!(f, "refused: {why}"),
            WriteError::BadValue(why) => write!(f, "bad value: {why}"),
            WriteError::NoSuchItem(id) => write!(f, "no item B{id}"),
            WriteError::Db(e) => write!(f, "database: {e}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<rusqlite::Error> for WriteError {
    fn from(e: rusqlite::Error) -> Self {
        WriteError::Db(e)
    }
}

/// A new item: the question, its lettered options, and which one the author recommends.
#[derive(Debug, Clone, Default)]
pub struct NewItem {
    pub question: String,
    /// `(letter, text)`, letters `a`, `b`, … in order.
    pub options: Vec<(String, String)>,
    pub recommended: Option<String>,
    /// The session's own number for it, such as `#9906`. A label, never a key.
    pub label: String,
    pub parent: Option<ItemId>,
}

/// One version of one cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The board's clock: row ids order every write.
    pub seq: i64,
    pub item: ItemId,
    pub writer: WriterId,
    pub field: String,
    pub version: i64,
    pub value: Value,
    pub client: String,
    pub written_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Writer {
    pub id: WriterId,
    pub kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub agent_type: String,
    pub parent: Option<WriterId>,
    pub project: String,
    pub workspace: String,
    pub name: String,
}

impl Writer {
    /// How the command and the page name a writer on its own: `owner`, a session's name (from
    /// its `/rename` title, else Claude's label, else a made-up name; never a hash), or an agent
    /// as `fork ab073b5d in 5d1da7e2`. [`label_in`] names an agent's session by its name too.
    pub fn label(&self) -> String {
        if self.kind == "owner" {
            return "owner".into();
        }
        let short = |id: &str| id.chars().take(8).collect::<String>();
        if self.agent_id.is_empty() {
            return if self.name.is_empty() {
                made_up_name(&self.session_id)
            } else {
                self.name.clone()
            };
        }
        let kind = if self.agent_type.is_empty() {
            "agent"
        } else {
            &self.agent_type
        };
        format!(
            "{kind} {} in {}",
            short(&self.agent_id),
            short(&self.session_id)
        )
    }
}

/// A writer's label among all writers. An agent names its session by the session's name, and a
/// session copied from another (`/branch`, `/fork`) says which, as the brief asks ("labelled with
/// the session it was forked from").
pub fn label_in(writers: &[Writer], id: WriterId) -> String {
    let Some(w) = writers.iter().find(|w| w.id == id) else {
        return format!("writer {id}");
    };
    let parent = w.parent.and_then(|p| writers.iter().find(|x| x.id == p));
    match parent {
        Some(session) if !w.agent_id.is_empty() => {
            let kind = if w.agent_type.is_empty() {
                "agent"
            } else {
                &w.agent_type
            };
            let short: String = w.agent_id.chars().take(8).collect();
            format!("{kind} {short} in {}", session.label())
        }
        Some(source) if w.kind == "session" => {
            format!("{} (copy of {})", w.label(), source.label())
        }
        _ => w.label(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemSummary {
    pub id: ItemId,
    pub label: String,
    pub parent: Option<ItemId>,
    pub author: WriterId,
    pub question: String,
    /// The author's or the owner's status, whichever was written last.
    pub status: String,
    /// Where it was raised, as it was then: the repository (main checkout), the checkout, and
    /// the checkout's branch (empty outside git).
    pub project: String,
    pub workspace: String,
    pub branch: String,
}

/// Narrows a listing or the timeline. Every field set must match.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub project: Option<String>,
    pub workspace: Option<String>,
    /// A node of the tree: this writer and everything under it, at any depth (its agents, the
    /// sessions copied from it, theirs).
    pub writer: Option<WriterId>,
    /// `open`, `answered`, `closed`, or `active` for anything not closed.
    pub status: Option<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, OpenError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        let layout: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if layout > LAYOUT {
            return Err(OpenError::NewerLayout(layout));
        }
        let has_items: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'items')",
            [],
            |r| r.get(0),
        )?;
        if layout == 0 && has_items {
            return Err(OpenError::Unversioned);
        }
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        // In WAL mode NORMAL keeps every commit safe from a crash of the program; only a power
        // cut can lose the last few. Fine for a board, and it keeps a write to about a millisecond.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        upgrade(&conn)?;
        fill_names(&conn)?;
        Ok(Store { conn })
    }

    /// Raises an item. Its question, options, recommendation and the author's `open` status are
    /// written in the same transaction, so the item's first cell is the moment it opened.
    pub fn add_item(
        &mut self,
        who: &Identity,
        place: &Place,
        new: &NewItem,
    ) -> Result<ItemId, WriteError> {
        // Options become a map, where a repeated letter would silently keep only its last text.
        let mut letters = std::collections::HashSet::new();
        if let Some((letter, _)) = new
            .options
            .iter()
            .find(|(k, _)| !letters.insert(k.as_str()))
        {
            return Err(WriteError::BadValue(format!(
                "option {letter} is given twice"
            )));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let author = resolve_writer(&tx, who, place)?;
        if let Some(parent) = new.parent {
            item_author(&tx, parent)?;
        }
        tx.execute(
            "INSERT INTO items (parent, author, label, project, workspace, branch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                new.parent,
                author,
                new.label,
                place.project,
                place.workspace,
                place.branch
            ],
        )?;
        let item = tx.last_insert_rowid();
        write_in(
            &tx,
            author,
            Client::Cli,
            item,
            Field::Question,
            &json!(new.question),
            None,
        )?;
        if !new.options.is_empty() {
            let options: serde_json::Map<String, Value> = new
                .options
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            write_in(
                &tx,
                author,
                Client::Cli,
                item,
                Field::Options,
                &Value::Object(options),
                None,
            )?;
        }
        if let Some(rec) = &new.recommended {
            write_in(
                &tx,
                author,
                Client::Cli,
                item,
                Field::Recommended,
                &json!(rec),
                None,
            )?;
        }
        write_in(
            &tx,
            author,
            Client::Cli,
            item,
            Field::Status,
            &json!("open"),
            None,
        )?;
        tx.commit()?;
        Ok(item)
    }

    /// A session writes one of its own cells. `expected` is the version it read (0 for none);
    /// when the cell has moved since, the write is refused. Returns the new version.
    pub fn write_as_session(
        &mut self,
        who: &Identity,
        place: &Place,
        item: ItemId,
        field: Field,
        value: &Value,
        expected: Option<i64>,
    ) -> Result<i64, WriteError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let writer = resolve_writer(&tx, who, place)?;
        let version = write_in(&tx, writer, Client::Cli, item, field, value, expected)?;
        tx.commit()?;
        Ok(version)
    }

    /// The owner's answer, from the page. It also sets the owner's status to `answered`, in the
    /// same transaction.
    pub fn owner_answer(
        &mut self,
        item: ItemId,
        answer: &Value,
        expected: Option<i64>,
    ) -> Result<i64, WriteError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version = write_in(
            &tx,
            OWNER,
            Client::Page,
            item,
            Field::Answer,
            answer,
            expected,
        )?;
        write_in(
            &tx,
            OWNER,
            Client::Page,
            item,
            Field::Status,
            &json!("answered"),
            None,
        )?;
        tx.commit()?;
        Ok(version)
    }

    /// The owner's status cell, from the page.
    pub fn owner_status(
        &mut self,
        item: ItemId,
        status: &str,
        expected: Option<i64>,
    ) -> Result<i64, WriteError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version = write_in(
            &tx,
            OWNER,
            Client::Page,
            item,
            Field::Status,
            &json!(status),
            expected,
        )?;
        tx.commit()?;
        Ok(version)
    }

    /// Sets a writer's name label, creating the writer if it has never written.
    pub fn set_name(
        &mut self,
        who: &Identity,
        place: &Place,
        name: &str,
    ) -> Result<WriterId, WriteError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let writer = resolve_writer(&tx, who, place)?;
        tx.execute(
            "UPDATE writers SET name = ?1 WHERE id = ?2",
            params![name, writer],
        )?;
        tx.commit()?;
        Ok(writer)
    }

    /// The writer for this identity, if it has ever written.
    pub fn find_writer(&self, who: &Identity) -> Result<Option<WriterId>, rusqlite::Error> {
        find_writer(&self.conn, &who.session_id, &who.agent_id)
    }

    pub fn writers(&self) -> Result<Vec<Writer>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, session_id, agent_id, agent_type, parent, project, workspace, name
             FROM writers ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Writer {
                id: r.get(0)?,
                kind: r.get(1)?,
                session_id: r.get(2)?,
                agent_id: r.get(3)?,
                agent_type: r.get(4)?,
                parent: r.get(5)?,
                project: r.get(6)?,
                workspace: r.get(7)?,
                name: r.get(8)?,
            })
        })?;
        rows.collect()
    }

    /// The current version of every cell of an item: one per (writer, field).
    pub fn current(&self, item: ItemId) -> Result<Vec<Cell>, rusqlite::Error> {
        self.cells(
            "SELECT c.id, c.item, c.writer, c.field, c.version, c.value, c.client, c.written_at
             FROM cells c
             WHERE c.item = ?1 AND c.version = (SELECT MAX(version) FROM cells d
                 WHERE d.item = c.item AND d.writer = c.writer AND d.field = c.field)
             ORDER BY c.id",
            params![item],
        )
    }

    /// Every version of every cell of an item, in the order written.
    pub fn history(&self, item: ItemId) -> Result<Vec<Cell>, rusqlite::Error> {
        self.cells(
            "SELECT id, item, writer, field, version, value, client, written_at
             FROM cells WHERE item = ?1 ORDER BY id",
            params![item],
        )
    }

    /// Every write after `after`, in the order written, narrowed by the filter's place and node.
    /// A node's timeline holds the writes on the items raised in that branch, whoever made them
    /// (the owner's answers included), and the branch's own writes on other items.
    /// `newest` keeps only that many of the latest rows, still in the order written.
    pub fn timeline(
        &self,
        filter: &Filter,
        after: i64,
        newest: Option<i64>,
    ) -> Result<Vec<Cell>, rusqlite::Error> {
        let sql = format!(
            "{BRANCH_OF_4}
             SELECT c.id, c.item, c.writer, c.field, c.version, c.value, c.client, c.written_at
             {TIMELINE_ROWS} ORDER BY c.id DESC LIMIT ?5"
        );
        let mut rows = self.cells(
            &sql,
            params![
                after,
                filter.project,
                filter.workspace,
                filter.writer,
                newest.unwrap_or(-1)
            ],
        )?;
        rows.reverse();
        Ok(rows)
    }

    /// How many rows the timeline holds for this filter, before any `newest` cut.
    pub fn timeline_len(&self, filter: &Filter) -> Result<i64, rusqlite::Error> {
        let sql = format!("{BRANCH_OF_4} SELECT COUNT(*) {TIMELINE_ROWS}");
        self.conn.query_row(
            &sql,
            params![0, filter.project, filter.workspace, filter.writer],
            |r| r.get(0),
        )
    }

    /// The items that hold every word typed, each word anywhere in the item: any version of any
    /// cell (question, options, answers, feedback) or its Item column (id, label, the item it
    /// follows). Oldest first; nothing for an empty query. Each word is looked up on its own and
    /// the items intersected, because the index matches all of one query's words within one row,
    /// that is one cell.
    pub fn search(&self, typed: &str) -> Result<Vec<ItemId>, rusqlite::Error> {
        let terms = text_terms(typed);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT item FROM cell_text WHERE cell_text MATCH ?1")?;
        let mut found: Option<std::collections::BTreeSet<ItemId>> = None;
        for term in &terms {
            let items = stmt
                .query_map(params![term], |r| r.get(0))?
                .collect::<Result<std::collections::BTreeSet<ItemId>, _>>()?;
            found = Some(match found {
                None => items,
                Some(so_far) => so_far.intersection(&items).copied().collect(),
            });
        }
        Ok(found.unwrap_or_default().into_iter().collect())
    }

    /// The owner asks every session with items not closed to review them: one press of the page's
    /// button (owner, 2026-11-22, B22 "b"). Only the page's server calls it; the command cannot.
    pub fn ask_for_review(&mut self) -> Result<ReviewRequest, rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO review_requests (asked_at) VALUES (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.query_row(
            "SELECT id, asked_at FROM review_requests WHERE id = ?1",
            params![id],
            |r| {
                Ok(ReviewRequest {
                    id: r.get(0)?,
                    asked_at: r.get(1)?,
                })
            },
        )
    }

    /// The owner's newest review request, if any.
    pub fn latest_review(&self) -> Result<Option<ReviewRequest>, rusqlite::Error> {
        latest_review(&self.conn)
    }

    /// Whether `writer` has seen review request `request`.
    pub fn has_seen_review(&self, request: i64, writer: WriterId) -> Result<bool, rusqlite::Error> {
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM review_seen WHERE request = ?1 AND writer = ?2)",
            params![request, writer],
            |r| r.get(0),
        )
    }

    /// Marks review request `request` seen by `writer`, `how` (`hook` or `wait`), and says whether
    /// this call did it. Only the first marking delivers, so a press reaches a session once, even
    /// when two of its calls race.
    pub fn mark_review_seen(
        &self,
        request: i64,
        writer: WriterId,
        how: &str,
    ) -> Result<bool, rusqlite::Error> {
        mark_review_seen(&self.conn, request, writer, how)
    }

    /// The writer's own items that are not closed, oldest first, with their status.
    pub fn open_items_of(
        &self,
        writer: WriterId,
    ) -> Result<Vec<(ItemId, String)>, rusqlite::Error> {
        Ok(self
            .items(&Filter::default())?
            .into_iter()
            .filter(|i| i.author == writer && i.status != "closed")
            .map(|i| (i.id, i.status))
            .collect())
    }

    /// For the page: each session writer with items not closed, those items, and how and when it
    /// saw the owner's newest review request, if it has.
    pub fn review_targets(&self) -> Result<Vec<ReviewTarget>, rusqlite::Error> {
        let latest = self.latest_review()?;
        let mut open: std::collections::BTreeMap<WriterId, Vec<(ItemId, String)>> =
            std::collections::BTreeMap::new();
        for i in self.items(&Filter::default())? {
            if i.status != "closed" && i.author != OWNER {
                open.entry(i.author).or_default().push((i.id, i.status));
            }
        }
        let mut out = Vec::new();
        for (writer, items) in open {
            let seen = match &latest {
                Some(request) => self
                    .conn
                    .query_row(
                        "SELECT how, seen_at FROM review_seen WHERE request = ?1 AND writer = ?2",
                        params![request.id, writer],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?,
                None => None,
            };
            out.push(ReviewTarget {
                writer,
                items,
                seen,
            });
        }
        Ok(out)
    }

    /// Moves whenever the owner asks for a review or a session sees one, for the page's feed.
    pub fn review_mark(&self) -> Result<i64, rusqlite::Error> {
        self.conn.query_row(
            "SELECT (SELECT count(*) FROM review_requests) + (SELECT count(*) FROM review_seen)",
            [],
            |r| r.get(0),
        )
    }

    /// When each writer last wrote: `(writer, row, time)` of its newest cell.
    pub fn last_writes(&self) -> Result<Vec<(WriterId, i64, String)>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT writer, id, written_at FROM cells
             WHERE id IN (SELECT MAX(id) FROM cells GROUP BY writer) ORDER BY writer",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    /// Every session (not an agent) as `(session id, name on the board)`, for the rename watch.
    pub fn session_names(&self) -> Result<Vec<(String, String)>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, name FROM writers WHERE kind = 'session' AND agent_id = ''",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Gives a session its new name; `false` when it already had it or is unknown.
    pub fn rename_session(
        &mut self,
        session_id: &str,
        name: &str,
    ) -> Result<bool, rusqlite::Error> {
        let changed = self.conn.execute(
            "UPDATE writers SET name = ?1
             WHERE kind = 'session' AND agent_id = '' AND session_id = ?2 AND name != ?1",
            params![name, session_id],
        )?;
        Ok(changed > 0)
    }

    /// The board's clock now: the last row written, 0 for an empty board.
    pub fn last_seq(&self) -> Result<i64, rusqlite::Error> {
        self.conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM cells", [], |r| r.get(0))
    }

    /// Items matching the filter, oldest first. Project and workspace are the item's own, where
    /// it was raised (review X1); the node is its author's branch.
    pub fn items(&self, filter: &Filter) -> Result<Vec<ItemSummary>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE branch(id) AS (
                 SELECT ?3 UNION SELECT w2.id FROM writers w2 JOIN branch b ON w2.parent = b.id)
             SELECT i.id, i.label, i.parent, i.author,
                (SELECT value FROM cells WHERE item = i.id AND field = 'question'
                    ORDER BY id DESC LIMIT 1),
                (SELECT value FROM cells WHERE item = i.id AND field = 'status'
                    AND writer IN (i.author, ?4) ORDER BY id DESC LIMIT 1),
                i.project, i.workspace, i.branch
             FROM items i JOIN writers a ON a.id = i.author
             WHERE (?1 IS NULL OR i.project = ?1)
               AND (?2 IS NULL OR i.workspace = ?2)
               AND (?3 IS NULL OR a.id IN branch)
             ORDER BY i.id",
        )?;
        let rows = stmt.query_map(
            params![filter.project, filter.workspace, filter.writer, OWNER],
            |r| {
                let question: Option<String> = r.get(4)?;
                let status: Option<String> = r.get(5)?;
                Ok(ItemSummary {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    parent: r.get(2)?,
                    author: r.get(3)?,
                    question: json_str(question),
                    status: json_str(status),
                    project: r.get(6)?,
                    workspace: r.get(7)?,
                    branch: r.get(8)?,
                })
            },
        )?;
        let mut items: Vec<ItemSummary> = rows.collect::<Result<_, _>>()?;
        if let Some(status) = &filter.status {
            items.retain(|i| match status.as_str() {
                "active" => i.status != "closed",
                wanted => i.status == wanted,
            });
        }
        Ok(items)
    }

    fn cells(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Cell>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |r| {
            let value: String = r.get(5)?;
            Ok(Cell {
                seq: r.get(0)?,
                item: r.get(1)?,
                writer: r.get(2)?,
                field: r.get(3)?,
                version: r.get(4)?,
                value: serde_json::from_str(&value).unwrap_or(Value::String(value)),
                client: r.get(6)?,
                written_at: r.get(7)?,
            })
        })?;
        rows.collect()
    }
}

fn json_str(stored: Option<String>) -> String {
    match stored.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(Value::String(s))) => s,
        _ => String::new(),
    }
}

fn find_writer(
    conn: &Connection,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<WriterId>, rusqlite::Error> {
    conn.query_row(
        "SELECT id FROM writers WHERE kind = 'session' AND session_id = ?1 AND agent_id = ?2",
        params![session_id, agent_id],
        |r| r.get(0),
    )
    .optional()
}

/// The writer for this identity, created on its first write with no setup. Its place is fixed
/// then, so a later `cd` does not move it. An agent's parent is its session's writer, created
/// with it when the session itself has not written yet.
fn resolve_writer(tx: &Transaction, who: &Identity, place: &Place) -> Result<WriterId, WriteError> {
    if who.session_id.is_empty() {
        return Err(WriteError::NotYours(
            "a session writes only with its session id",
        ));
    }
    if let Some(id) = find_writer(tx, &who.session_id, &who.agent_id)? {
        // A session's name follows a rename: the transcript's latest name wins (#189/a).
        if who.agent_id.is_empty() && !who.name.is_empty() {
            tx.execute(
                "UPDATE writers SET name = ?1 WHERE id = ?2 AND name != ?1",
                params![who.name, id],
            )?;
        }
        return Ok(id);
    }
    if who.agent_id.is_empty() {
        let parent = session_parent(tx, who, place)?;
        return insert_writer(tx, &who.session_id, "", "", parent, place, &who.name);
    }
    let session = match find_writer(tx, &who.session_id, "")? {
        Some(s) => s,
        None => {
            let parent = session_parent(tx, who, place)?;
            insert_writer(tx, &who.session_id, "", "", parent, place, "")?
        }
    };
    insert_writer(
        tx,
        &who.session_id,
        &who.agent_id,
        &who.agent_type,
        Some(session),
        place,
        "",
    )
}

/// Brings a board to [`LAYOUT`], in one transaction that re-reads the version, so two processes
/// opening it at once upgrade it once.
/// - Layout 2 (2026-11-21, the owner's "we need full text search"): the `cell_text` full text
///   index (FTS5) over every version of every cell, kept by a trigger on `cells`, and filled from
///   the cells already there. A cell's text is all its JSON values: a question, the options' texts,
///   a pick and a text.
/// - Layout 3 (2026-11-22, the owner's "full text search should also contain ITEM column, we can
///   search by ref number etc"): the Item column's text, the item's id, its label and the item it
///   follows (`B12 #243 follows B11`), as one more row per item in `cell_text`, with no cell. A
///   trigger on `items` keeps it (items never change), filled from the items already there.
/// - Layout 4 (2026-11-22, the owner's B22 "b"): the owner's review requests, one row per press
///   of the page's button (`review_requests`), and which session saw each, how (`review_seen`,
///   keyed by request and writer, so a press reaches each session once). Both append-only.
fn upgrade(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let layout: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if layout < 2 {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS cell_text
                     USING fts5(text, item UNINDEXED, cell UNINDEXED);
                 CREATE TRIGGER IF NOT EXISTS cells_indexed AFTER INSERT ON cells BEGIN
                     INSERT INTO cell_text (text, item, cell) VALUES
                         ((SELECT group_concat(value, ' ') FROM json_each(NEW.value)),
                          NEW.item, NEW.id);
                 END;
                 DELETE FROM cell_text;
                 INSERT INTO cell_text (text, item, cell)
                     SELECT (SELECT group_concat(value, ' ') FROM json_each(c.value)), c.item, c.id
                     FROM cells c;
                 PRAGMA user_version = 2;",
            )?;
        }
        if layout < 3 {
            conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS items_indexed AFTER INSERT ON items BEGIN
                     INSERT INTO cell_text (text, item, cell) VALUES
                         ('B' || NEW.id || ' ' || NEW.label
                              || COALESCE(' follows B' || NEW.parent, ''),
                          NEW.id, NULL);
                 END;
                 DELETE FROM cell_text WHERE cell IS NULL;
                 INSERT INTO cell_text (text, item, cell)
                     SELECT 'B' || i.id || ' ' || i.label || COALESCE(' follows B' || i.parent, ''),
                            i.id, NULL
                     FROM items i;
                 PRAGMA user_version = 3;",
            )?;
        }
        if layout < 4 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS review_requests (
                     id        INTEGER PRIMARY KEY,
                     asked_at  TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS review_seen (
                     request   INTEGER NOT NULL REFERENCES review_requests(id),
                     writer    INTEGER NOT NULL REFERENCES writers(id),
                     how       TEXT NOT NULL CHECK (how IN ('hook', 'wait')),
                     seen_at   TEXT NOT NULL,
                     PRIMARY KEY (request, writer)
                 );
                 CREATE TRIGGER IF NOT EXISTS review_requests_never_changed
                     BEFORE UPDATE ON review_requests
                     BEGIN SELECT RAISE(ABORT, 'review requests are never changed'); END;
                 CREATE TRIGGER IF NOT EXISTS review_requests_never_deleted
                     BEFORE DELETE ON review_requests
                     BEGIN SELECT RAISE(ABORT, 'review requests are never deleted'); END;
                 CREATE TRIGGER IF NOT EXISTS review_seen_never_changed
                     BEFORE UPDATE ON review_seen
                     BEGIN SELECT RAISE(ABORT, 'seen records are never changed'); END;
                 CREATE TRIGGER IF NOT EXISTS review_seen_never_deleted
                     BEFORE DELETE ON review_seen
                     BEGIN SELECT RAISE(ABORT, 'seen records are never deleted'); END;
                 PRAGMA user_version = 4;",
            )?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The full text terms for what a person typed, one per word: a word matches as the start of a
/// word, so "sqli" finds "SQLite", and a whole reference matches exactly (see [`is_ref`]). Words
/// are quoted, so no character has a meaning to the index; a "word" with no letter or digit is
/// dropped.
pub fn text_terms(typed: &str) -> Vec<String> {
    typed
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| {
            if is_ref(w) {
                quoted(w)
            } else {
                quoted(w) + "*"
            }
        })
        .collect()
}

/// A typed word as a quoted phrase, so none of its characters means anything to the index.
fn quoted(word: &str) -> String {
    format!("\"{}\"", word.replace('"', "\"\""))
}

/// Whether a typed word is a whole reference, matched exactly rather than as a word's start: an
/// item's id (`B12`) or a label's number (`#243`), so "B1" does not also find B10 to B19.
fn is_ref(word: &str) -> bool {
    let digits = word.strip_prefix(['B', 'b', '#']).unwrap_or("");
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// One press of the page's review button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequest {
    pub id: i64,
    pub asked_at: String,
}

/// A session the owner's review request is for: its items not closed, as `(id, status)`, and
/// `(how, when)` it saw the newest request, if it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub writer: WriterId,
    pub items: Vec<(ItemId, String)>,
    pub seen: Option<(String, String)>,
}

fn latest_review(conn: &Connection) -> Result<Option<ReviewRequest>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, asked_at FROM review_requests ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            Ok(ReviewRequest {
                id: r.get(0)?,
                asked_at: r.get(1)?,
            })
        },
    )
    .optional()
}

fn mark_review_seen(
    conn: &Connection,
    request: i64,
    writer: WriterId,
    how: &str,
) -> Result<bool, rusqlite::Error> {
    let added = conn.execute(
        "INSERT OR IGNORE INTO review_seen (request, writer, how, seen_at)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![request, writer, how],
    )?;
    Ok(added == 1)
}

/// What a session is told when the owner asks it to review: fixed words and its items' ids and
/// statuses only. No question, label or feedback: those are session-written text, and a hook's
/// note reaches a session with the owner's weight.
pub fn review_note(items: &[(ItemId, String)]) -> String {
    let list = items
        .iter()
        .map(|(id, status)| format!("B{id} ({status})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "focus-board: the owner asks you to review your items on the board: {list}. Read each \
         with `focus-board show B<n>`, act on the owner's answer, and close each item once it is \
         settled."
    )
}

/// The review note for one session's Bash call (owner, 2026-11-22, B22 "b"), or `None`: when the
/// owner's newest review request is one this exact writer (session id and agent id, so a subagent
/// never gets its parent's items) has not seen, and it has items not closed. The request is then
/// marked seen (`hook`), and only the call whose marking wins gets the note.
///
/// The hook runs on every Bash call of every session, so this never goes through
/// [`Store::open`], which takes a write lock and may upgrade the board. It never creates the board,
/// never touches a board of another layout, waits at most 200 ms for a lock, and never fails:
/// any error means no note.
pub fn review_note_for(db: &Path, session_id: &str, agent_id: &str) -> Option<String> {
    use rusqlite::OpenFlags;
    if session_id.is_empty() {
        return None;
    }
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.busy_timeout(Duration::from_millis(200)).ok()?;
    let layout: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .ok()?;
    if layout != LAYOUT {
        return None;
    }
    let request = latest_review(&conn).ok()??;
    let writer: WriterId = conn
        .query_row(
            "SELECT id FROM writers WHERE kind = 'session' AND session_id = ?1 AND agent_id = ?2",
            params![session_id, agent_id],
            |r| r.get(0),
        )
        .optional()
        .ok()??;
    let store = Store { conn };
    if store.has_seen_review(request.id, writer).ok()? {
        return None;
    }
    let items = store.open_items_of(writer).ok()?;
    if items.is_empty() || !store.mark_review_seen(request.id, writer, "hook").ok()? {
        return None;
    }
    Some(review_note(&items))
}

/// Sessions written before names existed get their made-up name, once, so no session shows as a
/// hash. Cheap when there is nothing to fill: one query.
fn fill_names(conn: &Connection) -> Result<(), rusqlite::Error> {
    let unnamed: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, session_id FROM writers WHERE kind = 'session' AND agent_id = '' AND name = ''",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, session_id) in unnamed {
        conn.execute(
            "UPDATE writers SET name = ?1 WHERE id = ?2 AND name = ''",
            params![made_up_name(&session_id), id],
        )?;
    }
    Ok(())
}

/// The parent of a session new to the board: the session it was copied from (`/branch`,
/// `/fork`), created with it when that one has never written. `None` for a session that is no
/// copy.
fn session_parent(
    tx: &Transaction,
    who: &Identity,
    place: &Place,
) -> Result<Option<WriterId>, WriteError> {
    let parent = &who.parent_session;
    if parent.is_empty() || parent == &who.session_id {
        return Ok(None);
    }
    Ok(Some(match find_writer(tx, parent, "")? {
        Some(p) => p,
        None => insert_writer(tx, parent, "", "", None, place, "")?,
    }))
}

/// A new writer's row. A session gets `name`, or its made-up name when it has none, so it never
/// shows as a hash; an agent is named by its type and its session.
fn insert_writer(
    tx: &Transaction,
    session_id: &str,
    agent_id: &str,
    agent_type: &str,
    parent: Option<WriterId>,
    place: &Place,
    name: &str,
) -> Result<WriterId, WriteError> {
    let name = match (agent_id.is_empty(), name.is_empty()) {
        (true, true) => made_up_name(session_id),
        (true, false) => name.to_string(),
        (false, _) => String::new(),
    };
    tx.execute(
        "INSERT INTO writers (kind, session_id, agent_id, agent_type, parent, project, workspace,
                              name, created_at)
         VALUES ('session', ?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            session_id,
            agent_id,
            agent_type,
            parent,
            place.project,
            place.workspace,
            name
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

fn item_author(tx: &Transaction, item: ItemId) -> Result<WriterId, WriteError> {
    tx.query_row(
        "SELECT author FROM items WHERE id = ?1",
        params![item],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(WriteError::NoSuchItem(item))
}

/// The one place a cell is written. Checks, in order: the item exists; this writer owns this
/// cell from this client; the value fits the field; the version the writer read is current.
fn write_in(
    tx: &Transaction,
    writer: WriterId,
    client: Client,
    item: ItemId,
    field: Field,
    value: &Value,
    expected: Option<i64>,
) -> Result<i64, WriteError> {
    let author = item_author(tx, item)?;
    check_owner(writer, client, author, field)?;
    check_value(tx, item, author, writer, field, value)?;
    let current: i64 = tx.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM cells WHERE item = ?1 AND writer = ?2 AND field = ?3",
        params![item, writer, field.as_str()],
        |r| r.get(0),
    )?;
    if let Some(read) = expected
        && read != current
    {
        return Err(WriteError::Stale { current });
    }
    tx.execute(
        "INSERT INTO cells (item, writer, field, version, value, client, written_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            item,
            writer,
            field.as_str(),
            current + 1,
            value.to_string(),
            client.as_str()
        ],
    )?;
    Ok(current + 1)
}

fn check_owner(
    writer: WriterId,
    client: Client,
    author: WriterId,
    field: Field,
) -> Result<(), WriteError> {
    let is_owner = writer == OWNER;
    if is_owner && client != Client::Page {
        return Err(WriteError::NotYours(
            "the owner writes only through the page",
        ));
    }
    if !is_owner && client != Client::Cli {
        return Err(WriteError::NotYours(
            "a session writes only through the command",
        ));
    }
    match field {
        Field::Answer if !is_owner => Err(WriteError::NotYours(
            "the owner's answer is written only on the page",
        )),
        Field::Feedback if is_owner => Err(WriteError::NotYours("feedback is a session's column")),
        Field::Question | Field::Options | Field::Recommended if writer != author => Err(
            WriteError::NotYours("only the item's author writes its question and options"),
        ),
        Field::Status if !is_owner && writer != author => Err(WriteError::NotYours(
            "a session sets the status only of its own items",
        )),
        _ => Ok(()),
    }
}

fn check_value(
    tx: &Transaction,
    item: ItemId,
    author: WriterId,
    writer: WriterId,
    field: Field,
    value: &Value,
) -> Result<(), WriteError> {
    let bad = |why: &str| Err(WriteError::BadValue(why.to_string()));
    match field {
        Field::Question => match value.as_str() {
            Some(q) if !q.trim().is_empty() => Ok(()),
            _ => bad("a question is non-empty text"),
        },
        Field::Options => {
            let Some(options) = value.as_object() else {
                return bad("options are letters mapped to text");
            };
            for (k, v) in options {
                if !is_letter(k) || !v.is_string() {
                    return bad("options are letters a–z mapped to text");
                }
            }
            Ok(())
        }
        Field::Recommended => {
            let Some(letter) = value.as_str() else {
                return bad("a recommendation is one option's letter");
            };
            if option_letters(tx, item, author)?
                .iter()
                .any(|k| k == letter)
            {
                Ok(())
            } else {
                bad("the recommendation must be one of the item's options")
            }
        }
        Field::Status => {
            let allowed: &[&str] = if writer == OWNER {
                &["open", "answered", "closed"]
            } else {
                &["open", "closed"]
            };
            match value.as_str() {
                Some(s) if allowed.contains(&s) => Ok(()),
                _ => bad("not a status this writer sets"),
            }
        }
        Field::Answer | Field::Feedback => {
            let Some(obj) = value.as_object() else {
                return bad("an answer or feedback is {\"pick\": letter} and/or {\"text\": text}");
            };
            if obj.keys().any(|k| k != "pick" && k != "text") {
                return bad("only \"pick\" and \"text\" are allowed");
            }
            let text = obj.get("text");
            let pick = obj.get("pick");
            if text.is_some_and(|t| !t.is_string()) {
                return bad("\"text\" is text");
            }
            let text_given = text
                .and_then(Value::as_str)
                .is_some_and(|t| !t.trim().is_empty());
            match pick {
                None if !text_given => bad("give a pick, a text, or both"),
                None => Ok(()),
                Some(p) => {
                    let letter = p.as_str().unwrap_or("");
                    if option_letters(tx, item, author)?
                        .iter()
                        .any(|k| k == letter)
                    {
                        Ok(())
                    } else {
                        bad("the pick must be one of the item's options")
                    }
                }
            }
        }
    }
}

fn is_letter(k: &str) -> bool {
    k.len() == 1 && k.bytes().all(|b| b.is_ascii_lowercase())
}

fn option_letters(
    tx: &Transaction,
    item: ItemId,
    author: WriterId,
) -> Result<Vec<String>, WriteError> {
    let stored: Option<String> = tx
        .query_row(
            "SELECT value FROM cells WHERE item = ?1 AND writer = ?2 AND field = 'options'
             ORDER BY version DESC LIMIT 1",
            params![item, author],
            |r| r.get(0),
        )
        .optional()?;
    let letters = stored
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();
    Ok(letters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tx: &Transaction) -> (WriterId, ItemId) {
        let place = Place {
            project: "p".into(),
            workspace: "p".into(),
            branch: String::new(),
        };
        let who = Identity {
            session_id: "s".into(),
            agent_id: String::new(),
            agent_type: String::new(),
            ..Default::default()
        };
        let author = resolve_writer(tx, &who, &place).unwrap();
        tx.execute("INSERT INTO items (author) VALUES (?1)", params![author])
            .unwrap();
        (author, tx.last_insert_rowid())
    }

    /// The public API never pairs the owner with the command or a session with the page; this
    /// checks the guard that refuses it if code ever does.
    #[test]
    fn writer_and_client_must_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("b.sqlite")).unwrap();
        let tx = store.conn.transaction().unwrap();
        let (author, item) = item(&tx);
        let answer = json!({"text": "yes"});
        let owner_from_cli = write_in(&tx, OWNER, Client::Cli, item, Field::Answer, &answer, None);
        assert!(
            matches!(owner_from_cli, Err(WriteError::NotYours(_))),
            "{owner_from_cli:?}"
        );
        let session_from_page = write_in(
            &tx,
            author,
            Client::Page,
            item,
            Field::Status,
            &json!("closed"),
            None,
        );
        assert!(
            matches!(session_from_page, Err(WriteError::NotYours(_))),
            "{session_from_page:?}"
        );
    }
}
