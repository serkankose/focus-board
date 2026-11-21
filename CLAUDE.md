# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## How to talk

The owner's global style applies (`~/.claude/CLAUDE.md`): brief, really short sentences, every
summary statement numbered and tagged, the local time first.

## What this is

A shared board for the few items that need attention now: open questions, the owner's answers, and
each Claude session's feedback. The owner writes in a browser page; sessions write through a
command. **[BRIEF.md](BRIEF.md) is the requirement**: who writes, the rules, the two ways in, and the
tests that prove it. Read it before designing anything. Don't copy it here.

- One board for every project and workspace on this machine. This project's *sessions* are still
  isolated: never message a session working in another project.
- One session works here. No named roles (owner, 2026-11-21).
- Its own git repo, trunk `master`, short-lived feature branches, squash-merged back (owner,
  2026-11-21). The home folder is also a git repo, so check `git rev-parse --show-toplevel` before
  any git command here.

## Status (2026-11-21)

- Build steps 2 to 4 of `DESIGN.md` are done: the store (`src/store.rs`), identity and place
  (`src/identity.rs`), the hook (`src/hook.rs`), the command (`src/main.rs`) and the server
  (`src/server.rs`, `focus-board serve`), with their tests (`tests/store.rs`, `tests/command.rs`,
  `tests/server.rs`). Step 5, the page (`src/page.html`), runs in a real browser:
  `checks/page-check.sh` drives a headless Firefox through it (14 checks).
- Three review rounds (`~/temp/focus-board/`, 2026-11-21) are answered and merged.
- **Installed (step 6, owner 2026-11-21 "#146/a", "#148/a"):**
  - `install/linux.sh` puts the command on `PATH` (`cargo install --path . --locked`) and runs the
    server as the systemd user service `focus-board.service`. It logs to
    `~/.local/state/focus-board/serve.log`, and the real board is at
    `~/.local/share/focus-board/board.sqlite` (`focus-board where`). `--remove` stops it.
  - The hook is in the owner's global `~/.claude/settings.json` (`PreToolUse`, matcher `Bash`), so
    every session on this machine gets it. It runs the installed copy, never the build in
    `target/`. Reinstall on purpose after a green build; that also updates what every session runs.
  - This project's `.claude/settings.local.json` holds no hook any more.
- Try the command on a scratch board: `FOCUS_BOARD_DB=<scratch path> focus-board whoami`. Without
  `FOCUS_BOARD_DB` it writes to the real board, where nothing can be deleted.
- **Test:** `cargo test`. Lint: `cargo clippy --all-targets`, `cargo fmt --check`.
- **Checks** beyond `cargo test`, in `checks/`:
  - `break-it.sh`: removes each rule in a scratch copy, and its test must then fail. Run it before
    committing a step, and add a line for every new rule.
  - `page-check.sh [BIN]`: the page in a headless Firefox, on a scratch board.
  - `demo-board.sh BIN DIR`: a scratch board with demo items, for looking at the page:
    `FOCUS_BOARD_DB=DIR/board.sqlite BIN serve`.
- Test boards set writers through `FOCUS_BOARD_*` in scripts and tests only. A session's own
  Bash command never sets them: the hook denies it (review R1).
- **Stack: Rust and SQLite, one program** (owner, 2026-11-21). The server serves the page; the
  command opens the database directly, each write in its own transaction.
  - Considered and dropped: PostgreSQL (a service to run), append-only files (version checks need
    our own locks), DuckDB and redb (one writing process at a time), replicated or CRDT SQLite
    forks (rqlite, LiteFS, libSQL, cr-sqlite). The board is on one machine, writes are tiny, and
    the CRDT forks merge clashing writes silently, which the brief's rule 1 forbids.
- **The command is on `PATH`** as `focus-board`, installed with `cargo install` (owner,
  2026-11-21). No project needs a path to it.

## Open choices — the owner's call, not yet made

- None. (Review R3, copied sessions, was built at the owner's word on 2026-11-21.)

## Session identity (owner, 2026-11-21)

- **A hook supplies it.** Claude Code's hooks are given the session id, agent id and agent type; a
  small hook writes them where the command reads them. The design does not rely on what a shell
  happens to see.
- Seen in a main session's shell (2026-11-21): `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID`. Not
  seen: the session's name, an agent id, a parent (fork) id. Project and workspace come from the
  working directory's git top level.
- Checked (2026-11-21): a subagent's hook input holds the parent's `session_id` plus its own
  `agent_id` and `agent_type`. Not checked: an Agent-tool fork.
- Checked (2026-11-21, `/branch` then `/fork` by the owner): a copied session gets a new id and
  writes as itself. Its parent is read from its transcript (`requestSessionId`, else
  `forkedFrom`); see `DESIGN.md`, "Copied sessions".
- The hook denies any Bash command whose text assigns `FOCUS_BOARD_*`, a heredoc too. Edit such
  text in this repo with the file tools.
- **A new identity variable goes into `sets_identity`'s list and into every `env_remove` list (the
  test harness, the end-to-end tests) and `checks/demo-board.sh`, in the same change** (review 08,
  F4). Anything a hooked command starts inherits the session's identity, tests included: the
  transcript variable once leaked into a test board that way.

## Machine resources (owner, 2026-11-21)

- **Linux, Windows and macOS** (owner, 2026-11-21, "#122/a"): the code is made portable,
  tested with Windows-style paths on Linux. The install recipes for macOS and Windows wait for a
  machine to try them. See `DESIGN.md`, "Platforms".
- **Port 8810**, bound to 127.0.0.1 only. It was free and clear of other projects' ranges
  (26xxx–29xxx, 8787–8798) and of the ephemeral range.
- **Logs** go to `$XDG_STATE_HOME/focus-board/` (default `~/.local/state/focus-board/`), not
  `/var/log`, so no root step is needed.
- No emulators. One long-running service: the board's server.

## Rules that already hold

- **No personal data in the repo**: no home-directory paths, usernames, emails or LAN addresses.
  Local config goes in a gitignored `.env`.
- **The board's data lives outside the source tree**, for example under the XDG data directory or
  a path set by an environment variable. The same goes for generated files.
- Read before editing. Run it, or it isn't done. A fix ships with the test that would have caught it.
- **Commits** (owner, 2026-11-21): commit each build step on its own branch, without asking, once
  its tests and its break-it checks (each rule's test fails with the rule removed) are green.
  Ask before merging anything into `master`, which takes the branches by squash or cherry-pick.
