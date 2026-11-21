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

- Build step 2 of `DESIGN.md` is done: the store (`src/store.rs`), identity and place
  (`src/identity.rs`), and their tests (`tests/store.rs`). No command, server or page yet.
- **Test:** `cargo test`. Lint: `cargo clippy --all-targets`, `cargo fmt --check`.
- **Stack: Rust and SQLite, one program** (owner, 2026-11-21). The server serves the page; the
  command opens the database directly, each write in its own transaction.
  - Considered and dropped: PostgreSQL (a service to run), append-only files (version checks need
    our own locks), DuckDB and redb (one writing process at a time), replicated or CRDT SQLite
    forks (rqlite, LiteFS, libSQL, cr-sqlite). The board is on one machine, writes are tiny, and
    the CRDT forks merge clashing writes silently, which the brief's rule 1 forbids.
- **The command is on `PATH`** as `focus-board`, installed with `cargo install` (owner,
  2026-11-21). No project needs a path to it.

## Open choices — the owner's call, not yet made

- None left from the first session.

## Session identity (owner, 2026-11-21)

- **A hook supplies it.** Claude Code's hooks are given the session id, agent id and agent type; a
  small hook writes them where the command reads them. The design does not rely on what a shell
  happens to see.
- Seen in a main session's shell (2026-11-21): `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID`. Not
  seen: the session's name, an agent id, a parent (fork) id. Project and workspace come from the
  working directory's git top level.
- Not checked: what a fork's or subagent's shell or hook sees. A subagent test was blocked by the
  auto-mode check, and the owner chose the hook instead.

## Machine resources (owner, 2026-11-21)

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
  Commit only when asked, on a branch.
