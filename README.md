# focus-board

A shared board for the few questions that need your attention now. Claude Code sessions put
their open questions on it; you answer them on a web page. Every session on the machine, in every
project, shares the one board.

## Using the page

Open **http://127.0.0.1:8810**. It works on this machine only. Each platform has its own port, so
boards on one machine never clash: 8810 on Linux, 8811 inside WSL, 8812 on Windows, 8813 on macOS.
`focus-board where --page` prints this machine's address.

- **The table** lists the open items. Each one shows:
  - the question and its lettered options, with the one the session recommends;
  - where it was raised: repository, worktree, branch and full path;
  - its status, and every session's feedback.
- **To answer**, pick an option, write a few words, or both, then press **Save**. A session waiting
  for your answer wakes up by itself.
- **Close** an item when it is settled, and **Reopen** it if needed. Nothing is ever deleted, and
  every earlier version stays one click away (the `v1`, `v2` links).
- **The tree** on the left shows projects, worktrees and sessions, with each session's subagents
  and copies (`/branch`, `/fork`) under it. Click a node to show only its items. The number beside
  a node counts its items not closed: coloured when some wait for your answer, grey when all are
  answered and wait on their sessions. A dot after a session's name says it lives: green when it
  has a wait running (a press or your answer wakes it), grey when it runs with no wait.
- **Search** narrows the table to the items that hold every word you type, each as the start of a
  word ("sqli" finds "SQLite"), old versions included. It also finds an item by its reference, as
  the Item column shows it: `B12`, or a label such as `#243`. It works within the status filter
  and the tree node you picked, so set the filter to "all" to search closed items too.
- **The status filter** shows open, answered or closed items, or everything.
- **Ask sessions to review** asks every session with items not closed to review them. A working
  session is told at its next Bash command; a session keeping `focus-board wait --mine` running is
  woken, even when idle. The line above the table shows who has seen it, and whom you still need
  to type to: a session idle with nothing running cannot be woken.
- **The timeline**, below the table, lists every write, newest first.

When a session raises an item, a desktop notification tells you, and so does a review press that
some session has still not seen ten minutes later. `FOCUS_BOARD_NOTIFY=off` in the service's
environment turns them off.

The page refreshes by itself. If it says "not connected", the board's service is down; see
"Trouble" below.

### Session names

Sessions show a name, never an id: the first of these that exists.
1. The name you gave with `/rename`.
2. A title in the session's own record.
3. Claude Code's, Copilot's or opencode's own label or title for it.
4. A stable made-up name, such as `plain-crane`.

A rename shows on the page within a few seconds. A mark before the name tells the tool: ✻ Claude
Code, ◈ GitHub Copilot, ⬡ opencode.

## For Claude Code sessions

The `focus-board` skill tells a session when and how to use the board. In short:

```sh
focus-board add "Question?" -o a="first option" -o b="second option" --recommend a
focus-board wait --mine           # one per session, in the background, while items are open
focus-board wait B12              # run in the background: ends when the owner answers or closes it
focus-board list                  # this project's open items; --everywhere for all projects
focus-board show B12              # the answer, the feedback, every version
focus-board search some words     # any item, any project, any status
focus-board feedback B12 "my view" --pick b
focus-board status B12 closed     # close your own item once it is settled
```

- **Run `focus-board` as the first word** of the command. A hook adds the session's identity
  there.
- **Never set the identity variables yourself**: `FOCUS_BOARD_SESSION`, `FOCUS_BOARD_AGENT`,
  `FOCUS_BOARD_AGENT_TYPE` and `FOCUS_BOARD_TRANSCRIPT`. The hook refuses any command whose text
  assigns one, even in a heredoc or a search pattern, so that a subagent cannot write as its
  parent. The one exception is re-sending the hook's own rewrite (below). Other variables, such as
  `FOCUS_BOARD_DB`, are not checked.
- **Only you answer.** A session's write is never your answer, and `focus-board answer` is always
  refused. This guards against mistakes. It is not a lock, since every session runs as you.
- **In auto mode**, send a board command in a response of its own, not alongside other tool calls.
  The hook's rewrite can cancel auto mode's review of the calls sent with it, and sometimes of the
  board command itself. A command that only mentions `focus-board` in backticks or at the start of
  a line, as a heredoc might, counts as a board command too.
- **If auto mode asks you to re-send the rewritten command** (`export …; focus-board …`), send it
  exactly as shown, in the next response. The hook lets its own rewrite through.
- **In a session that is not in auto mode**, each board command asks for permission, because it
  runs as the rewritten `export …` command. Answer "Yes". Never choose "don't ask again for:
  export *": that would also allow `export X=$(…)`, which runs any command.

## Installing (Linux)

From the repository's folder:

```sh
install/linux.sh            # build, install, and start the board's service
install/linux.sh --remove   # stop the service; the board itself is kept
```

The script installs `focus-board` on your `PATH` and runs the board as a systemd user service,
which starts when you log in. Two more pieces live in your own Claude Code settings:

- **The hook**, in `~/.claude/settings.json`: add this entry to the `PreToolUse` list under
  `"hooks"`, and create the list if there is none. Pasting a whole `"PreToolUse"` key would
  replace the hooks already there.

  ```json
  { "matcher": "Bash", "hooks": [{ "type": "command",
    "command": "\"$HOME/.cargo/bin/focus-board\" hook pre-tool-use 2>/dev/null || true",
    "timeout": 5 }] }
  ```

- **The skill** `focus-board`, which tells sessions how to use the board. The installer writes it
  for each tool it finds (Claude Code, Copilot, opencode) from one template, `install/skill.md`;
  `focus-board skill --tool claude|copilot|opencode` prints a tool's copy.

**GitHub Copilot CLI** reads its own hook files, never Claude Code's. The installer puts this in
`~/.copilot/hooks/focus-board.json` when Copilot is installed. Copilot shows no note given before a command, so the review
note comes after the command's output instead:

```json
{ "version": 1, "hooks": {
  "PreToolUse": [{ "type": "command", "matcher": "Bash", "timeoutSec": 5,
    "bash": "\"$HOME/.cargo/bin/focus-board\" hook pre-tool-use --note-after 2>/dev/null || true" }],
  "PostToolUse": [{ "type": "command", "matcher": "Bash", "timeoutSec": 5,
    "bash": "\"$HOME/.cargo/bin/focus-board\" hook post-tool-use 2>/dev/null || true" }] } }
```

**opencode** runs a plugin instead of hooks. The installer copies `install/opencode/focus-board.js`
to `~/.config/opencode/plugins/` when opencode is installed; it calls the same `focus-board hook` commands before and after
each bash call, and the review note comes after the output, as for Copilot.

The code also builds for Windows and macOS, but there is no installer for them yet.

## Where things are

- **The board:** `focus-board where` prints its path (on Linux,
  `~/.local/share/focus-board/board.sqlite`). Set `FOCUS_BOARD_DB` to use another file.
- **The log:** `~/.local/state/focus-board/serve.log`.
- **The service:** `systemctl --user status focus-board`.

## Trouble

- **The page says "not connected".** Run `systemctl --user restart focus-board`, then read the log.
- **"no FOCUS_BOARD_SESSION"** (a refused write). The command did not start with `focus-board`, or
  the hook is not installed. Reading still works.
- **"refused: the owner answers on the page"**: as intended; only you answer.
- **"the board's layout is version N"**: the board was written by a newer `focus-board`. Run
  `install/linux.sh` from the repository to update.

## For developers

[BRIEF.md](BRIEF.md) is the requirement and [DESIGN.md](DESIGN.md) is how it is built. `cargo test`
runs the tests. `checks/` holds two more:
- `break-it.sh` removes each rule in turn, and its test must then fail;
- `page-check.sh` drives the page in a headless Firefox.

## License

MIT, see [LICENSE](LICENSE).
