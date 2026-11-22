# focus-board — the brief

The owner's request, 2026-11-21: "a shared document that we use to focus on current tasks: open
questions, my responses, your feedback. It should support writes from you and me at the same time,
maybe a table where some columns are yours and some are mine, so we don't overwrite or destroy each
other's edits." The owner chose a local database with a small web page, as a project of its own.

This brief says what is needed and why. How to build it is this project's call.

## Who writes

- **The owner**, in a browser.
- **Every Claude session on this machine**, from a terminal: in any project and any workspace, named
  or not (a project's Stormy, Lens and Smith, for example), and forks, subagents and background
  sessions too. They all run under the same user on the same machine.

## One board for everything (owner, 2026-11-21)

- **Every project, and every workspace of each project**, is on one board. A workspace is a checkout
  of the project, such as a git worktree. The page filters by project, by workspace and by session.
- **A writer is one session**, known by:
  - its project (the repository it works in);
  - its workspace (the checkout or worktree);
  - its session id;
  - its name, when it has one.
- **A fork writes as itself.** It gets its own id and its own cells, labelled with the session it
  was forked from. It never writes into its parent's cells, even though it starts with the parent's
  context and name.
- **Writers are not a fixed list.** A new session shows up the first time it writes, and needs no
  setup. The table cannot have one fixed column per session, so each item shows the owner's answer
  plus one feedback cell for each session that has written on it.

## What it holds

- **One row per item**: an open question or task. It holds the question, its lettered options
  (`a`, `b`, …), which option is recommended, a status (open, answered, closed), and the project
  and session it came from.
- **Item ids belong to the board.** Each session numbers its own items (`#9906` in one session,
  `#1111` in another), so those numbers collide across sessions. Keep the session's number as a label
  on the row, and never use it as the key.
- **Cells owned by one writer each**: the owner's answer (an option or free text), and each
  session's feedback or position. A writer can change only its own cells.

## Rules

1. **Nothing is lost when two write at once.** A write changes only its writer's cells, in one
   transaction. A write may also name the version it read. If the cell has moved since, the write is
   refused and the writer reads again. It is never merged silently.
2. **Nothing is deleted.** Every edit of every cell is kept, with who wrote it and when. Closing an
   item is a status, not a removal. The page can show a cell's history.
3. **The owner's column is written only through the web page.** The sessions' interface refuses it.
   On one user account this cannot be a hard guarantee, since a session can run anything the owner
   can. So the refusal is a guard against mistakes, and every write is logged with its client. A
   session's write is never the owner's answer.
4. **This machine only.** Bind 127.0.0.1. The page's writes are guarded against other sites the
   browser visits: the request must come from a loopback `Host`, with a matching `Origin` and a custom
   header. That rules out cross-site form posts and DNS rebinding.
5. **The data lives outside the source tree**, and nothing personal goes into the repo: no
   home-directory paths, usernames or LAN addresses.

## The two ways in

- **The page** (the owner): a table of open items, filtered by project and status. The owner edits
  their own column inline, and sees when each session last wrote. It updates without a manual reload.
- **A logical tree** (owner, 2026-11-21): project → workspace → session → that session's forks and
  subagents, each under the session it came from. Each node holds the items raised there, and an
  item can hold sub-items, such as a follow-up question. Picking a node narrows the table and the
  timeline to that branch.
- **A timeline** (owner, 2026-11-21): every write in time order: an item opened, a cell written, an
  answer given, a status changed. Each entry names its writer and its place in the tree. It covers
  the whole board or one branch, and it is built from the edit history the rules above keep, so
  nothing is stored twice.
- **A command-line tool** (the sessions): list open items, read one, add one, write the session's own
  column. **Make it a command, not only an HTTP route.** The Claude Code harness on this machine
  blocks non-GET `curl`, so a session cannot rely on posting to a URL. The command may call the
  server, or open the database directly in its own transaction.
- **A review button** (owner, 2026-11-22: "we need a single button on the board to signal all
  agents with not closed items to review the responses", then "b" on B22): one press asks every
  session with items not closed to review them. The hook tells a working session at its next Bash
  command, once per press, naming only its items by id and status. A session that keeps one
  background wait while it has items open is woken by the press, even when idle. A session idle
  with nothing running cannot be woken; the page shows who has seen the press, so the owner knows
  where to type.

## Tests that prove it

- Two writers change different columns of the same row at the same moment, and both edits survive.
- A write that names a stale version is refused, and the cell keeps the newer value.
- The command-line tool refuses the owner's column.
- A fork and its parent write on the same item, and each keeps its own cell.
- Items from two projects, and from two workspaces of one project, appear under their own filters
  and never mix.
- A session that has never written before can write at once, with no setup.
- A fork shows under its parent in the tree, and its writes appear in the timeline in time order,
  between its parent's.
- Every version of a cell is still there after many edits.
- The server refuses a write with a foreign `Origin`, a foreign `Host`, or no custom header.

## Open choices for the first session

- The stack. The machine has Rust (cargo, with sccache). The owner's net-monitor dashboard is a
  loopback web page over SQLite, a pattern known to work here.
- **How a session learns its own identity.** Claude Code passes hooks the session id, the agent id
  and type, and the working directory. Whether a command run from a session's shell can read the
  same values is not checked yet. One way would be a hook that writes them where the command finds
  them. A fork's parent id needs the same check.
- The port. Pick one that clashes with nothing already in use, and write it down. Other projects
  here use the 26xxx–29xxx ranges and ports near 8787–8798.
- How sessions find the command: on `PATH`, or by an absolute path named in each project's
  instructions.
- The new-project checklist: git first (`git init` before any other git command here, since the
  home folder is itself a git repo), `CLAUDE.md`, and whether this project uses named roles.

## Not in scope

- Anything reachable from outside this machine. No accounts, no internet.
- A general project tracker. This board holds the few items that need attention now.
