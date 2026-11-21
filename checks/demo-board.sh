#!/usr/bin/env bash
# Fills a scratch board with a few demo items, for looking at the page or checking it.
# Usage: checks/demo-board.sh BIN DIR      (writes DIR/board.sqlite and DIR/alpha, DIR/beta)
#
# The writers are set here, in a script, as the tests do. A session's own Bash command never sets
# FOCUS_BOARD_* itself: the hook denies that. Never point this at the real board.
set -uo pipefail

bin=$1
dir=$2
export FOCUS_BOARD_DB=$dir/board.sqlite
as() { # as SESSION AGENT AGENT_TYPE -- focus-board arguments...
    local session=$1 agent=$2 kind=$3
    shift 3
    FOCUS_BOARD_SESSION=$session FOCUS_BOARD_AGENT=$agent FOCUS_BOARD_AGENT_TYPE=$kind \
        "$bin" "$@" >/dev/null || { echo "board setup failed: $*"; exit 1; }
}

mkdir -p "$dir/alpha" "$dir/beta"
# alpha is a git repository on branch main, with a worktree on branch feature; beta is no repo.
g() {
    git -C "$1" -c user.name=demo -c user.email=demo@example.invalid -c init.defaultBranch=main \
        "${@:2}" >/dev/null 2>&1
}
g "$dir/alpha" init -q && g "$dir/alpha" commit -q --allow-empty -m demo &&
    g "$dir/alpha" worktree add -q -b feature "$dir/alpha-feature" ||
    { echo "git setup failed"; exit 1; }
cd "$dir/alpha" || exit 1
as demo-session-a "" "" add "Keep SQLite, or store the board another way?" \
    -o a="keep SQLite" -o b="PostgreSQL" -o c="append-only files" --recommend a --label "#15"
as demo-session-a "" "" whoami --name Smith
as demo-session-a "" "" feedback B1 "One machine, tiny writes: SQLite is enough." --pick a
as demo-session-a fork-0001 fork feedback B1 "A fork's view: <b>not bold</b> & safe" --pick a
as demo-session-a "" "" add "Which port?" -o a=8810 -o b="pick later" --recommend a --label "#8" --parent B1
cd "$dir/beta" || exit 1
as demo-session-b "" "" add "Ship the page today?" -o a=yes -o b=no --label "#3"
as demo-session-b "" "" feedback B3 "Waiting on the browser check."
cd "$dir/alpha-feature" || exit 1
as demo-session-c "" "" add "Merge the feature branch now?" \
    -o a="merge" -o b="wait for the review" --recommend b --label "#21"
