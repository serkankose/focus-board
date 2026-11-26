---
name: focus-board
<!-- only: claude copilot -->
description: On this machine, in any project — when a question or choice waits on the owner's answer (an ❓ with lettered options) and should stay visible beyond this chat, when the owner says "put it on the board", or when you start or resume work and should check the owner's answers to your items — use the shared focus board through the `focus-board` command. Covers what belongs on the board, raising an item with options and a recommendation, reading or waiting for the owner's answer, giving feedback, closing, and the identity rules (run it as the first word; never set FOCUS_BOARD_* yourself).
<!-- only: opencode -->
description: In opencode, on this machine, in any project — when a question or choice waits on the owner's answer (an ❓ with lettered options) and should stay visible beyond this chat, when the owner says "put it on the board", or when you start or resume work and should check the owner's answers to your items — use the shared focus board through the `focus-board` command. Covers what belongs on the board, raising an item with options and a recommendation, reading or waiting for the owner's answer, giving feedback, closing, and the identity rules (run it as the first word; never set FOCUS_BOARD_* yourself).
<!-- end -->
---

# The focus board

The owner's shared board of open questions: one board for every project and session on this
<!-- only: claude -->
machine. The owner answers on the page, http://127.0.0.1:8810. Sessions write with the
`focus-board` command, and the global hook adds your identity.
<!-- only: copilot -->
machine, Copilot and Claude Code alike. The owner answers on the page, http://127.0.0.1:8810.
Sessions write with the `focus-board` command, and the hook in `~/.copilot/hooks/focus-board.json`
adds your identity.
<!-- only: opencode -->
machine, opencode, Copilot and Claude Code alike. The owner answers on the page, http://127.0.0.1:8810.
Sessions write with the `focus-board` command, and the plugin in
`~/.config/opencode/plugins/focus-board.js` adds your identity.
<!-- end -->

## When to use it

- **Raise an item** when a question waits on the owner's answer and should not be lost in the
  chat: a choice with lettered options, a decision blocking your work. Ask it in the chat as well.
- **Check your items** when you start or resume work: `focus-board list`.
- **Not for** progress reports, logs or finished work. The board holds the few items that need
  attention now.

## Commands

- `focus-board add "question" -o a="…" -o b="…" --recommend a --label "#12"` prints the item's id,
  `B<n>`. The label is your own number for it; `B<n>` is the key. `--parent B<n>` makes a
  follow-up.
- `focus-board list` shows this project's items that are not closed. `--everywhere` shows all
  projects, `--all` includes closed items, and `--json` gives machine-readable output.
- `focus-board show B12`: the owner's answer, every session's feedback, and versions.
- `focus-board search WORDS [--json]` finds the items that hold every word given, each as the
  start of a word, in every project and status. Before raising an item, search for it: it may
  already be on the board.
- `focus-board feedback B12 "text" --pick b` writes your view. Add `--expect V`, the version you
  read, and the write is refused if it has moved since.
- `focus-board status B12 closed` closes your own item once it is settled.
- `focus-board wait B12` ends when the owner answers, or closes the item, and prints it. Add
  `--after V` to wait for a newer answer than version V.
- `focus-board answer` is always refused: only the owner answers, on the page.

## Waiting for the owner's answer

<!-- only: claude -->
- While you have items that are not closed, keep one `focus-board wait --mine` running as a
  **background** Bash command (`run_in_background`). It ends when the owner answers or closes one
  of your items, or presses the page's review button, and Claude Code wakes you with what it
  printed. Act on it, close what is settled, and start it again while items stay open.
<!-- only: copilot -->
- While you have items that are not closed, keep one `focus-board wait --mine` running as a
  **detached background** shell command (async, detached mode). It ends when the owner answers or
  closes one of your items, or presses the page's review button, and Copilot wakes you when it
  completes. Read its output, act on it, close what is settled, and start it again while items
  stay open.
<!-- only: opencode -->
- While you have items that are not closed, keep one `focus-board wait --mine` running with your
  shell tool. opencode moves a command still running at its timeout to the background and wakes
  you when it ends. It ends when the owner answers or closes one of your items, or presses the
  page's review button. Read its output, act on it, close what is settled, and start it again
  while items stay open.
<!-- end -->
- Keep one such wait per session, and never poll the board yourself. `focus-board wait B<n>` still
  waits for one item.
<!-- only: claude -->
- When a Bash call shows a focus-board note that the owner asks you to review your items, read
  each with `focus-board show`, act on the answers, and close what is settled.
<!-- only: copilot opencode -->
- When a shell command's output ends with a focus-board note that the owner asks you to review
  your items, read each with `focus-board show`, act on the answers, and close what is settled.
<!-- end -->
- If the owner answers in the chat first, note the answer as your feedback.
- A wait only wakes a session that is still open. After a restart, check with `focus-board list`.

## Rules

<!-- only: claude -->
- **Run `focus-board` as the first word** of the Bash command, not after `timeout`, `if` or `!`.
<!-- only: copilot opencode -->
- **Run `focus-board` as the first word** of the shell command, not after `timeout`, `if` or `!`.
<!-- end -->
  The hook adds your identity there.
<!-- only: claude -->
- **Never set `FOCUS_BOARD_*` yourself.** The hook denies any command whose text assigns an
  identity variable (`FOCUS_BOARD_SESSION`, `_AGENT`, `_AGENT_TYPE`, `_TRANSCRIPT`), even in a
  heredoc or a grep pattern, because a subagent would otherwise write as its parent. The other
  `FOCUS_BOARD_*` variables are not checked; leave them alone too, since they point the command at
  another board.
<!-- only: copilot opencode -->
- **Never set `FOCUS_BOARD_*` yourself.** The hook sets the identity variables
  (`FOCUS_BOARD_SESSION`, `_AGENT`, `_AGENT_TYPE`, `_TRANSCRIPT`); setting them yourself would let
  a subagent write as its parent. Leave the other `FOCUS_BOARD_*` variables alone too, since they
  point the command at another board.
<!-- end -->
- A session's write is never the owner's answer. When the owner answers in the chat, close the
  item, or note the answer as your feedback.
- Nothing is ever deleted; close an item instead.
<!-- only: claude -->
- In a session not in auto mode, each board command asks permission. Answer "Yes", and never
  "don't ask again for: export *".
- **In auto mode**, send a board command in a response of its own, not alongside other tool
  calls. The hook's rewrite can void auto mode's review of calls sent with it. A command that only
  mentions `focus-board` in backticks or at the start of a line, as a heredoc might, counts as a
  board command too: send it alone, or edit such text with the file tools. If auto mode says a
  hook changed the call and asks you to re-issue the rewritten command (`export …; focus-board
  …`), re-issue it exactly as given. The hook lets its own rewrite through.
<!-- end -->

## Where things are

- The board's file: `focus-board where`. The server: `systemctl --user status focus-board`.
- The code, the brief and the design: `~/dev/focus-board` (`BRIEF.md`, `DESIGN.md`).
