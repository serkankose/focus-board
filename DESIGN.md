# focus-board — design

Approved by the owner on 2026-11-21, before any code, and kept in step with it since.
[BRIEF.md](BRIEF.md) is the requirement; this says how. Items marked **unverified** have not been
seen to work yet.

## One program

- One Rust binary, `focus-board`, installed with `cargo install` onto `PATH`.
- `focus-board serve`: the web page and its API, on `127.0.0.1:8810`.
- Every other subcommand is the sessions' command. It opens the database itself, in its own
  transaction, so it works with the server down.
- Crates: `rusqlite` (SQLite bundled), `axum` + `tokio` for the server, `clap`, `serde_json`.
- The page is one HTML file with plain JavaScript, compiled into the binary. No build step.
- Data: `$XDG_DATA_HOME/focus-board/board.sqlite` (default `~/.local/share/focus-board/`).
  `FOCUS_BOARD_DB` overrides it; tests use a temp folder.
- Logs: `$XDG_STATE_HOME/focus-board/`.

## Tables

All writes go through one function in the store, used by both the command and the server.

- **`writers`**: one row per writer.
  - Fields: kind (`owner` or `session`), Claude's session id, agent id (empty for the session
    itself), agent type, parent writer, project, workspace, name, first write.
  - Unique on (session id, agent id). A session or agent that has never written gets a row on
    its first write, with no setup.
  - Exactly one `owner` row.
- **`items`**: one row per item.
  - Fields: board id, parent item (for sub-items), the writer that raised it, and the session's
    own number kept as a label (`#9906`).
  - **Where it was raised, as it was then** (owner, 2026-11-21: "to make good decisions in the item
    listing I should be able to see the context"): the repository (main checkout), the checkout
    (worktree) and its branch, taken from the raising command's working directory. The writer's
    own place stays where it first wrote. The page shows repo, worktree, branch and full path on
    each item; `list` and `show` print them. The project and workspace filters use the item's own
    place too, so an item is filed where it shows it was raised (review X1).
  - Board ids are shown as `B12`, so they never look like a session's `#12`.
- **`cells`**: every version of every cell, never changed.
  - Fields: row id, item, writer (who owns the cell), field, version, value, client (`page` or
    `cli`), time written.
  - Unique on (item, writer, field, version).
  - The current value is the highest version. A cell's history is all its rows.
  - The row id is the board's one clock: the timeline and the page's change feed both order by it,
    never by timestamps.

**Who owns which cell**

| Field | Owner | Written from |
|---|---|---|
| question, options, recommended | the item's author | command |
| feedback (text, optionally a picked option) | each session, its own | command |
| answer (an option or free text) | the owner | page only |
| status (open, closed) | the item's author | command |
| status (answered, closed) | the owner | page only |

**Status** (owner, 2026-11-21): two status cells, the author's and the owner's. The item shows the
one written last, by row id. Writing the owner's answer also writes the owner's status as
`answered`, in the same transaction. Each writer still writes only its own cells. Any answer
write sets `answered`, even on a closed item: editing an answer reopens it (owner, 2026-11-21).

## The write rules

- Each write runs in `BEGIN IMMEDIATE`, with WAL mode and a busy timeout. Two writers on the same row
  queue for a few milliseconds and both land.
- The write checks that this writer owns this cell. Then it takes the current version. If the
  write named a version and the cell has moved since, it is refused with the newer version. Then it
  inserts version + 1. The unique index is a second guard against a race.
- The `answer` field is refused unless the client is `page`. The command has no path that sets
  `page`. This is a guard against mistakes, as rule 3 says, not a hard boundary.
- **Nothing is deleted:** SQLite triggers refuse every `UPDATE` and `DELETE` on `items` and
  `cells`. Closing is a status value.

## How a session knows who it is

A hook runs as its own process, so it cannot set variables in the session's shell. Subagents share
their parent's process, so the shell alone cannot tell a subagent from its parent.

- **A `PreToolUse` hook on Bash rewrites the command.** When a Bash command calls `focus-board`,
  the hook (`focus-board hook pre-tool-use`, reading the hook's JSON on stdin) returns the command
  with `FOCUS_BOARD_SESSION`, `FOCUS_BOARD_AGENT` and `FOCUS_BOARD_AGENT_TYPE` set in front.
  Claude Code passes hooks `session_id`, and `agent_id` and `agent_type` inside a subagent.
  - **Checked 2026-11-21 in a main session:** the rewrite works. The hook returns only
    `updatedInput`, with no `permissionDecision`, so the normal permission checks still run on the
    rewritten command. A new project-local hook fired without a restart.
  - The main session's hook input held `session_id`, `transcript_path`, `cwd`, `prompt_id`,
    `permission_mode`, `tool_use_id`: no agent id (as expected) and no session name.
  - **Checked 2026-11-21 in a subagent** (`general-purpose`, spawned on the owner's approval): its
    hook input held the parent's `session_id`, its own `agent_id` and its `agent_type`. The
    rewritten command saw all three. No field names a parent agent.
- **No identity, no write.** Without `FOCUS_BOARD_SESSION`, the command reads but refuses to write.
  It never falls back to `CLAUDE_CODE_SESSION_ID`: a subagent would then write into its parent's
  cells, which is exactly the brief's fork test failing silently.
- **Nobody sets the identity by hand** (review R1). A command that assigns `FOCUS_BOARD_SESSION`,
  `FOCUS_BOARD_AGENT` or `FOCUS_BOARD_AGENT_TYPE` itself would override the hook's export, and the
  value at hand is a subagent's parent's id. The hook checks every Bash command and denies such a
  one (`permissionDecision: deny`); this only tightens. The refusal for a missing identity says to
  run `focus-board` as the first word, and never to set the variables. `runs_board` sees through
  `env`, `exec`, `command`, `time`, `nohup`, `nice`, `timeout N`, `!` and shell keywords.
- **Parents.** An Agent-tool fork or subagent has its parent's session id and its own agent id, so
  its parent is that session's writer. Seen for a subagent; **unverified** for an Agent-tool fork
  (`subagent_type: fork`), which is expected to behave the same.
- **Copied sessions** (`/branch`, `/fork`, `--fork-session`; review R3, built 2026-11-21 at the
  owner's word). Such a copy gets a new session id and writes as itself. Its hooks name no parent,
  but its transcript does, and the hook passes the transcript's path as
  `FOCUS_BOARD_TRANSCRIPT`.
  - Copied assistant turns carry `requestSessionId`, the session that made them, and a copy of a
    copy keeps the older ids and adds its parent's. So the last one other than the session's own
    is the immediate parent.
  - `forkedFrom.sessionId` on copied lines is the fallback; it names the root of a chain.
  - Seen in the owner's checks (V, Claude Code 2.1.329): a `/branch` of the root, then a `/fork`
    of that branch. Both fields are undocumented; when they are missing, the copy shows as a
    session of its own.
  - The command reads the transcript only on a session's first write (`identity::fork_parent`).
    The store then files the copy under its parent, creating the parent's row if it never wrote.
    Labels say "(copy of …)". A tree node's filter covers its whole branch, at any depth.
- **Project and workspace** come from the command's working directory at the writer's first write,
  and stay fixed after that, so a later `cd` does not move a writer.
  - Workspace: `git rev-parse --show-toplevel`. Project: the main checkout, from
    `--git-common-dir`, so all worktrees of one repo share a project.
  - When there is no repo, or the top level is the home folder (which is itself a repo here), the
    project and workspace are the working directory itself.
- **Name.** No name has been seen in a session's shell or hook input. `focus-board whoami
  --name Smith` sets the writer's name label. The name is a label; the id is the key.

## The command

| Command | Does |
|---|---|
| `focus-board whoami [--name N]` | shows (or names) this writer |
| `focus-board list [--project P \| --everywhere] [--status S \| --all] [--json]` | by default this project's items that are not closed |
| `focus-board show B12 [--json]` | one item: all cells, current versions |
| `focus-board add "question" -o a="…" -o b="…" [--recommend a] [--label "#9906"] [--parent B3]` | raises an item |
| `focus-board feedback B12 ["text"] [--pick b] [--expect V]` | writes this session's feedback |
| `focus-board status B12 open\|closed [--expect V]` | the author's status of its own item |
| `focus-board history B12 [--json]` | every version of every cell |
| `focus-board answer …` | always refused: "the owner answers on the page" |
| `focus-board hook pre-tool-use` | the hook itself: reads the hook's JSON on stdin, never fails |

Reads need no identity. Every write needs the hook's `FOCUS_BOARD_SESSION`, and refuses before
opening the board without it. A refusal exits 1 with the reason on stderr.

## The server and the page

- Binds `127.0.0.1:8810` only.
- **Every route**, reads included, refuses a `Host` other than `127.0.0.1:8810` or
  `localhost:8810`. Checking writes only would still let a DNS-rebinding site read the board.
- **Every write** also needs a matching `Origin` and the header `X-Focus-Board: 1`, which a
  cross-site form cannot send.
- Routes: `GET /` (the page), `GET /api/items`, `GET /api/tree`, `GET /api/timeline`,
  `GET /api/history?item=N`, `GET /api/changes?after=N`, `POST /api/answer`, `POST /api/status`.
  The reads take `project`, `workspace`, `writer` (a tree node) and `status`. The two writes are
  the owner's only: no route writes a session's cell. A refused request is logged to stderr,
  which the systemd unit keeps.
- A stale write gets `409` with the current version; a value that does not fit gets `400`.
- **Live updates.** The command writes past the server, so the change feed reads the database: the
  page asks `/api/changes?after=<last row id>` every 2 s and refetches when it has moved.
- The page has three parts:
  - **Table**: open items, filtered by project and status. The owner's answer is edited inline;
    each session's feedback has its own cell, with when it last wrote.
  - **Tree**: project, workspace, session, then its forks and subagents. Picking a node narrows the
    table and timeline to that branch.
  - **Timeline**: the `cells` rows in row-id order, each naming its writer and its place in the tree.
    A node's timeline holds every write on the items raised in that branch, the owner's answers
    included, plus the branch's own writes on other items.

## Tests — the brief's nine, and the ones added while building

| Brief's test | Test |
|---|---|
| Two writers, different columns, same row, same moment: both survive | `concurrent_writers_both_land` (two connections, two threads, barrier-started) |
| A stale version is refused; the cell keeps the newer value | `stale_version_refused` |
| The command refuses the owner's column | `cli_refuses_answer` (runs the binary); the store's guard: `session_cannot_write_the_owners_column` |
| A fork and its parent each keep their own cell | `fork_and_parent_own_cells` |
| Two projects, two workspaces of one: never mix | `projects_and_worktrees_stay_apart`, also covering the home-folder case |
| A new session writes at once, no setup | `first_write_creates_writer` |
| A fork shows under its parent; timeline interleaves | `fork_in_tree_and_timeline_order` |
| Every version survives many edits | `all_versions_kept` (1000 edits) and `update_and_delete_refused` (triggers) |
| Foreign `Origin`, foreign `Host`, or no header: refused | `server_guards` (writes and reads, raw requests over TCP, DNS rebinding included) |
| *(added)* No identity, no write | `unidentified_write_refused` (identity), `unidentified_command_write_refused` (the binary, with only `CLAUDE_CODE_SESSION_ID` set) |
| *(added)* The hook rewrites only `focus-board` commands, never fails, and the rewrite writes as the hook's agent | `hook_rewrites_only_board_calls` (binary, end to end through bash), `runs_board_finds_the_command_word_only` (unit) |
| *(added)* The command end to end | `command_round_trip` |
| *(added)* Values fit their field | `values_are_checked` |
| *(added)* Status: the last written of the author's and the owner's | `status_is_the_last_written_of_author_and_owner` |
| *(added)* The owner never writes from the command, a session never from the page | `writer_and_client_must_match` (unit test in `src/store.rs`) |
| *(added)* The page's answer and status, the version check, no route for a session's cell | `page_answers_and_reads` |
| *(added)* The page sees writes made past the server: change feed, tree, branch timeline | `page_sees_writes_made_past_it` |
| *(added)* `focus-board serve` binds 127.0.0.1 only and serves the page | `serve_binds_loopback_only` |
| *(review R1)* A hand-set identity is denied; the refusal says not to set it | `hook_denies_a_hand_set_identity`, `sets_identity_finds_assignments_only` (unit), `unidentified_command_write_refused` |
| *(review R2)* No page can frame the board | `page_cannot_be_framed`, and `checks/page-check.sh` frames it in a real browser |
| *(review R6)* A shared folder name does not mix projects | `a_shared_folder_name_does_not_mix_projects` |
| *(review R8)* The timeline is cut and says so; active items on the server | `timeline_is_cut_and_says_so`, `timeline_cut_and_active_filter` |
| *(review R10, R11, G8)* Repeated letters, a newer layout, the author-only question | `a_repeated_option_letter_is_refused`, `a_newer_layout_is_refused`, `only_the_author_writes_the_question` |
| The page in a real browser (R7, R9, W9, W10 included) | `checks/page-check.sh`: 13 checks in headless Firefox |
| *(owner's ask)* Each item keeps where it was raised: repo, worktree, branch, path | `item_context_is_recorded_at_raise_time`, the branch asserts in `projects_and_worktrees_stay_apart`, and the page check "each item shows its repo, worktree, branch and full path" |
| A board from before layout versions is refused | `an_unversioned_board_is_refused` |
| *(review R3)* A copied session finds its parent and sits under it | `fork_parent_reads_the_transcript`, `a_copied_session_files_under_its_parent`, `a_copied_session_writes_under_its_parent` (the binary, through the hook), and the page check "a copied session sits under the session it was copied from" |
| *(review X1)* An item is filed where it was raised | `an_item_is_filed_where_it_was_raised` |
| Each rule's test fails with the rule removed | `checks/break-it.sh`: 44 checks |

`update_and_delete_refused` also tries `INSERT OR REPLACE` and `REPLACE`. Those delete the clashing
row without firing delete triggers, so insert triggers refuse them: a cell only gains its next
version, and no row id or writer identity is ever taken over.

The tests drive the board with the identity variables set directly. Those tests prove the board's
logic, not that Claude Code's hook delivers the ids. That part is checked once in the spike below.

## Build order

1. **Spike: does the hook rewrite work?** Install the `PreToolUse` hook in this project's
   gitignored `.claude/settings.local.json` first, so only sessions here get it (owner,
   2026-11-21). Run `focus-board whoami` from this session, then from one subagent (spawned only on
   the owner's explicit approval; auto mode blocked one before). If the rewrite cannot carry the
   ids, stop and bring that to the owner before anything else. Once it works, it moves to the
   global settings.
2. Store, write rules, triggers, and tests 1–8.
3. The command.
4. The server, its guards, and the guard tests.
5. The page: table, tree, timeline, cell history.
6. Install: `cargo install`, the hook in the global settings, and a systemd user unit for
   `focus-board serve` that starts at login and restarts after a crash (owner, 2026-11-21).
   Before calling it done, run one board command from a session that is not in auto mode: the
   rewritten command starts with `export …;`, so an allow rule such as `Bash(focus-board:*)` may
   stop matching and every board call may prompt. The unit sends the server's stderr (startup and
   refused requests) to `%S/focus-board/serve.log`, which is `~/.local/state/focus-board/`, the
   log folder the owner chose (review R5).

## Platforms

**Linux, Windows and macOS: the code now, the installs later** (owner, 2026-11-21: "#122/a", sent
right after "#122/b"; the later one stands). Review 04 listed what a port touches.
- **To build next, tested with Windows-style paths on Linux:**
  - P1: one data folder per OS;
  - P2: the page splits paths on both separators;
  - P3: `runs_board` takes `focus-board.exe` and `\`;
  - P4: `list --project` takes `C:\…` as a path;
  - P8: Windows `canonicalize` gives `\\?\` paths;
  - P9: `is_safe_path` takes a drive path.
- **Unverified until a run on that OS:**
  - P5: the hook's POSIX `export` prefix (Claude Code's Bash tool on Windows runs Git Bash);
  - P7: tests and checks need bash, git and Firefox.
- **Waits for a machine to try:** P6, the install recipe per OS (launchd, a scheduled task). Step 6
  is the Linux one.

## Open, the owner's call

- None.

## Accepted for now

- The hand-set-identity guard reads command text, so a Bash command that only *writes* such an
  assignment somewhere (a heredoc writing a test file) is denied too. In this repository, make
  those edits with the file tools, which the hook does not see.

- A session that moves into another worktree mid-session stays under the workspace of its first
  write.
- A subagent of a subagent is filed under the session, not under its own parent, unless the hook
  input turns out to carry the parent agent's id.
- A subagent whose session has not written yet creates the session's row with the subagent's own
  place, so a subagent in its own worktree files its parent under that worktree (review R12).
- `whoami --name` changes a writer's name in place; a name is a label, not a cell.

## Also from review 02

- Every response forbids framing (`X-Frame-Options: DENY`, `frame-ancestors 'none'`), so a framed
  board's own writes cannot be clicked from another page (R2).
- `list --project NAME` takes a path, or a folder name that fits exactly one project (R6).
- The timeline answers its newest `limit` rows (default 300) with the total; the page says what it
  left out. `/api/items?status=active` leaves closed items out on the server (R8).
- A repeated option letter is refused (R10). The board's layout version is kept in
  `user_version`, and a newer one is refused (R11).
