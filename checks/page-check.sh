#!/usr/bin/env bash
# The page in a real browser: a scratch board, `focus-board serve` on a free loopback port, and
# browser-check.py driving a headless Firefox through Marionette. Everything it writes is in temp
# folders; the owner's Firefox profile and the real board are never touched.
#
# Usage: checks/page-check.sh [BIN] [OUT_DIR]
#   BIN      the focus-board to check (default: target/debug/focus-board)
#   OUT_DIR  where the screenshots go (default: a new temp folder, printed at the end)
# Exit: 0 when every browser check passed.
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin=${1:-$here/../target/debug/focus-board}
case $bin in # absolute, since the setup below changes folders
    */*) bin=$(realpath "$bin") ;;
    *) bin=$(command -v "$bin") ;;
esac
[ -x "$bin" ] || { echo "no focus-board at ${1:-target/debug/focus-board}"; exit 1; }
out=${2:-$(mktemp -d)}
work=$(mktemp -d)
server=
cleanup() {
    [ -n "$server" ] && kill "$server" 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT

"$here/demo-board.sh" "$bin" "$work" || exit 1
export FOCUS_BOARD_DB=$work/board.sqlite

"$bin" serve --port 0 2>"$work/serve.log" &
server=$!
for _ in $(seq 50); do
    grep -q 'http://' "$work/serve.log" && break
    sleep 0.1
done
addr=$(grep -o 'http://[0-9.:]*' "$work/serve.log" | head -1)
[ -n "$addr" ] || { echo "the server did not start:"; cat "$work/serve.log"; exit 1; }

python3 -I "$here/browser-check.py" "$addr/" "$out"
status=$?
echo "screenshots: $out"
grep -i refused "$work/serve.log" | head -5
exit $status
