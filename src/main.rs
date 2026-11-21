//! `focus-board`: the sessions' way onto the board. It opens the database directly, so it works
//! whether or not the page's server is running.

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use focus_board::identity::{Identity, Place};
use focus_board::store::{Cell, Field, Filter, ItemId, ItemSummary, NewItem, Store, Writer};
use focus_board::{db_path, home_dir, hook};

const PAGE: &str = "http://127.0.0.1:8810";

#[derive(Parser)]
#[command(
    name = "focus-board",
    version,
    about = "The shared board of open questions: sessions write here, the owner answers on the page."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show who this command writes as. --name sets your name label.
    Whoami {
        #[arg(long)]
        name: Option<String>,
    },
    /// List items. By default: this project's items that are not closed.
    List {
        /// A project, by path or folder name.
        #[arg(long, conflicts_with = "everywhere")]
        project: Option<String>,
        /// Every project.
        #[arg(long)]
        everywhere: bool,
        /// Only this status: open, answered or closed.
        #[arg(long, conflicts_with = "all")]
        status: Option<String>,
        /// Closed items too.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// One item: its question, options, the owner's answer and every session's feedback.
    Show {
        item: String,
        #[arg(long)]
        json: bool,
    },
    /// Raise an item.
    Add {
        question: String,
        /// An option, as LETTER=TEXT: -o a="keep it" -o b="drop it".
        #[arg(short = 'o', long = "option", value_name = "LETTER=TEXT")]
        options: Vec<String>,
        /// The option you recommend.
        #[arg(long, value_name = "LETTER")]
        recommend: Option<String>,
        /// Your own number for it, such as "#9906". A label, never the key.
        #[arg(long, default_value = "")]
        label: String,
        /// The item this one follows up.
        #[arg(long, value_name = "ITEM")]
        parent: Option<String>,
    },
    /// Write your feedback on an item: a text, a picked option, or both.
    Feedback {
        item: String,
        text: Option<String>,
        #[arg(long, value_name = "LETTER")]
        pick: Option<String>,
        /// The version of your feedback you read (0 for none). Refused if it has moved since.
        #[arg(long, value_name = "VERSION")]
        expect: Option<i64>,
    },
    /// Set the status of an item you raised: open or closed.
    Status {
        item: String,
        status: String,
        /// The version of your status you read. Refused if it has moved since.
        #[arg(long, value_name = "VERSION")]
        expect: Option<i64>,
    },
    /// Every version of every cell of an item, in the order written.
    History {
        item: String,
        #[arg(long)]
        json: bool,
    },
    /// Always refused: the owner answers on the page, never through this command.
    Answer {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        ignored: Vec<String>,
    },
    /// Entry points for Claude Code hooks.
    #[command(subcommand)]
    Hook(HookCmd),
}

#[derive(Subcommand)]
enum HookCmd {
    /// Reads a PreToolUse input on stdin. For a Bash command that runs focus-board, prints the
    /// command with the session's ids in front. Never fails.
    PreToolUse,
}

fn main() -> ExitCode {
    match run(Cli::parse().cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("focus-board: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Hook(HookCmd::PreToolUse) => {
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_ok()
                && let Some(out) = hook::pre_tool_use(&input)
            {
                println!("{out}");
            }
            Ok(())
        }
        Cmd::Answer { .. } => Err(format!(
            "refused: the owner answers on the page ({PAGE}). A session's write is never the \
             owner's answer; write your view with `focus-board feedback`."
        )),
        Cmd::Whoami { name } => whoami(name),
        Cmd::List {
            project,
            everywhere,
            status,
            all,
            json,
        } => list(project, everywhere, status, all, json),
        Cmd::Show { item, json } => show(&item, json),
        Cmd::Add {
            question,
            options,
            recommend,
            label,
            parent,
        } => add(question, options, recommend, label, parent),
        Cmd::Feedback {
            item,
            text,
            pick,
            expect,
        } => feedback(&item, text, pick, expect),
        Cmd::Status {
            item,
            status,
            expect,
        } => {
            let (who, place, mut store) = writer()?;
            let id = parse_item(&item)?;
            let v = store
                .write_as_session(&who, &place, id, Field::Status, &json!(status), expect)
                .map_err(|e| e.to_string())?;
            println!("B{id} status {status} (v{v})");
            Ok(())
        }
        Cmd::History { item, json } => history(&item, json),
    }
}

fn open() -> Result<Store, String> {
    let path = db_path().ok_or("no FOCUS_BOARD_DB, XDG_DATA_HOME or HOME to find the board")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    Store::open(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn here() -> Result<Place, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("working directory: {e}"))?;
    Ok(Place::of_dir(&cwd, home_dir().as_deref()))
}

/// Everything a write needs. Refuses before opening anything when the hook gave no identity.
fn writer() -> Result<(Identity, Place, Store), String> {
    let who = Identity::from_env().map_err(|e| e.to_string())?;
    Ok((who, here()?, open()?))
}

fn parse_item(s: &str) -> Result<ItemId, String> {
    s.trim_start_matches(['B', 'b'])
        .parse()
        .map_err(|_| format!("{s:?} is not an item id such as B12"))
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn folder(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

fn label(writers: &[Writer], id: i64) -> String {
    let Some(w) = writers.iter().find(|w| w.id == id) else {
        return format!("writer {id}");
    };
    if w.kind == "owner" {
        return "owner".into();
    }
    let base = if w.agent_id.is_empty() {
        format!("session {}", short(&w.session_id))
    } else {
        let kind = if w.agent_type.is_empty() {
            "agent"
        } else {
            &w.agent_type
        };
        format!("{kind} {} in {}", short(&w.agent_id), short(&w.session_id))
    };
    if w.name.is_empty() {
        base
    } else {
        format!("{} ({base})", w.name)
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(o) if o.keys().all(|k| k == "pick" || k == "text") => {
            let pick = o.get("pick").and_then(Value::as_str);
            let text = o.get("text").and_then(Value::as_str);
            match (pick, text) {
                (Some(p), Some(t)) => format!("{p}: {t}"),
                (Some(p), None) => p.to_string(),
                (None, Some(t)) => t.to_string(),
                (None, None) => String::new(),
            }
        }
        other => other.to_string(),
    }
}

fn whoami(name: Option<String>) -> Result<(), String> {
    let (who, place, mut store) = writer()?;
    if let Some(name) = name {
        store
            .set_name(&who, &place, &name)
            .map_err(|e| e.to_string())?;
    }
    let writers = store.writers().map_err(|e| e.to_string())?;
    println!("session  {}", who.session_id);
    if !who.agent_id.is_empty() {
        println!("agent    {} ({})", who.agent_id, who.agent_type);
    }
    match store.find_writer(&who).map_err(|e| e.to_string())? {
        Some(id) => {
            let w = writers
                .iter()
                .find(|w| w.id == id)
                .expect("found writer is listed");
            println!("writes as  {}", label(&writers, id));
            if let Some(parent) = w.parent {
                println!("under      {}", label(&writers, parent));
            }
            println!("project    {}", w.project);
            println!("workspace  {}", w.workspace);
        }
        None => {
            println!("not on the board yet: you appear on your first write, at");
            println!("project    {}", place.project);
            println!("workspace  {}", place.workspace);
        }
    }
    Ok(())
}

fn item_json(i: &ItemSummary, writers: &[Writer]) -> Value {
    let author = writers.iter().find(|w| w.id == i.author);
    json!({
        "id": format!("B{}", i.id),
        "label": i.label,
        "status": i.status,
        "question": i.question,
        "parent": i.parent.map(|p| format!("B{p}")),
        "author": label(writers, i.author),
        "project": author.map(|w| w.project.clone()),
        "workspace": author.map(|w| w.workspace.clone()),
    })
}

fn list(
    project: Option<String>,
    everywhere: bool,
    status: Option<String>,
    all: bool,
    as_json: bool,
) -> Result<(), String> {
    let store = open()?;
    let writers = store.writers().map_err(|e| e.to_string())?;
    let project_of = |i: &ItemSummary| {
        writers
            .iter()
            .find(|w| w.id == i.author)
            .map(|w| w.project.clone())
            .unwrap_or_default()
    };
    let mut items = store.items(&Filter::default()).map_err(|e| e.to_string())?;
    if !everywhere {
        let wanted = match project {
            Some(p) if p.contains('/') || p == "." => Path::new(&p)
                .canonicalize()
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or(p),
            Some(p) => p,
            None => here()?.project,
        };
        items.retain(|i| {
            let p = project_of(i);
            p == wanted || folder(&p) == wanted
        });
    }
    match status {
        Some(s) => items.retain(|i| i.status == s),
        None if !all => items.retain(|i| i.status != "closed"),
        None => {}
    }
    if as_json {
        let out: Vec<Value> = items.iter().map(|i| item_json(i, &writers)).collect();
        println!("{}", Value::Array(out));
        return Ok(());
    }
    if items.is_empty() {
        println!("no items");
    }
    for i in &items {
        let label_part = if i.label.is_empty() {
            String::new()
        } else {
            format!("{}  ", i.label)
        };
        println!(
            "B{:<4} {:<9} {label_part}{}   ({} · {})",
            i.id,
            i.status,
            i.question,
            label(&writers, i.author),
            folder(&project_of(i))
        );
    }
    Ok(())
}

fn cell_json(c: &Cell, writers: &[Writer]) -> Value {
    json!({
        "seq": c.seq,
        "writer": label(writers, c.writer),
        "writer_id": c.writer,
        "field": c.field,
        "version": c.version,
        "value": c.value,
        "client": c.client,
        "written_at": c.written_at,
    })
}

fn show(item: &str, as_json: bool) -> Result<(), String> {
    let store = open()?;
    let id = parse_item(item)?;
    let summary = store
        .items(&Filter::default())
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or(format!("no item B{id}"))?;
    let writers = store.writers().map_err(|e| e.to_string())?;
    let cells = store.current(id).map_err(|e| e.to_string())?;
    let author_cell = |field: &str| {
        cells
            .iter()
            .find(|c| c.writer == summary.author && c.field == field)
            .map(|c| &c.value)
    };
    let options = author_cell("options").cloned().unwrap_or(json!({}));
    let recommended = author_cell("recommended")
        .and_then(Value::as_str)
        .map(str::to_string);
    if as_json {
        let mut out = item_json(&summary, &writers);
        out["options"] = options;
        out["recommended"] = json!(recommended);
        out["cells"] = Value::Array(cells.iter().map(|c| cell_json(c, &writers)).collect());
        println!("{out}");
        return Ok(());
    }
    let label_part = if summary.label.is_empty() {
        String::new()
    } else {
        format!("  {}", summary.label)
    };
    println!("B{}  [{}]{label_part}", summary.id, summary.status);
    println!("{}", summary.question);
    if let Some(opts) = options.as_object() {
        for (k, v) in opts {
            let rec = if recommended.as_deref() == Some(k) {
                "   (recommended)"
            } else {
                ""
            };
            println!("  {k}) {}{rec}", v.as_str().unwrap_or(""));
        }
    }
    println!("raised by  {}", label(&writers, summary.author));
    if let Some(p) = summary.parent {
        println!("follows    B{p}");
    }
    let line = |c: &Cell| format!("{}  (v{}, {})", shown(&c.value), c.version, c.written_at);
    match cells.iter().find(|c| c.field == "answer") {
        Some(c) => println!("answer     {}", line(c)),
        None => println!("answer     none yet"),
    }
    for c in cells.iter().filter(|c| c.field == "status") {
        println!("status     {}: {}", label(&writers, c.writer), line(c));
    }
    for c in cells.iter().filter(|c| c.field == "feedback") {
        println!("feedback   {}: {}", label(&writers, c.writer), line(c));
    }
    Ok(())
}

fn add(
    question: String,
    options: Vec<String>,
    recommend: Option<String>,
    label: String,
    parent: Option<String>,
) -> Result<(), String> {
    let options = options
        .iter()
        .map(|o| {
            o.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.to_string()))
                .ok_or(format!("option {o:?} is not LETTER=TEXT"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parent = parent.as_deref().map(parse_item).transpose()?;
    let (who, place, mut store) = writer()?;
    let new = NewItem {
        question,
        options,
        recommended: recommend,
        label,
        parent,
    };
    let id = store
        .add_item(&who, &place, &new)
        .map_err(|e| e.to_string())?;
    println!("B{id}");
    Ok(())
}

fn feedback(
    item: &str,
    text: Option<String>,
    pick: Option<String>,
    expect: Option<i64>,
) -> Result<(), String> {
    let mut value = serde_json::Map::new();
    if let Some(t) = text {
        value.insert("text".into(), json!(t));
    }
    if let Some(p) = pick {
        value.insert("pick".into(), json!(p));
    }
    let id = parse_item(item)?;
    let (who, place, mut store) = writer()?;
    let v = store
        .write_as_session(
            &who,
            &place,
            id,
            Field::Feedback,
            &Value::Object(value),
            expect,
        )
        .map_err(|e| e.to_string())?;
    println!("B{id} feedback v{v}");
    Ok(())
}

fn history(item: &str, as_json: bool) -> Result<(), String> {
    let store = open()?;
    let id = parse_item(item)?;
    let writers = store.writers().map_err(|e| e.to_string())?;
    let cells = store.history(id).map_err(|e| e.to_string())?;
    if cells.is_empty() {
        return Err(format!("no item B{id}"));
    }
    if as_json {
        let out: Vec<Value> = cells.iter().map(|c| cell_json(c, &writers)).collect();
        println!("{}", Value::Array(out));
        return Ok(());
    }
    for c in &cells {
        println!(
            "#{:<5} {}  {}  {} v{}  {}",
            c.seq,
            c.written_at,
            label(&writers, c.writer),
            c.field,
            c.version,
            shown(&c.value)
        );
    }
    Ok(())
}
