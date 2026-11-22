#!/usr/bin/env bash
# Installs focus-board on Linux for the current user: the binary on PATH, and the server as a
# systemd user service that starts at login. The Claude Code hook is installed separately, in the
# user's own settings (see CLAUDE.md, step 6), since that is the owner's file.
#
# Usage: install/linux.sh            install or update, then (re)start the service
#        install/linux.sh --remove   stop and remove the service; the binary and the board stay
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user

if [ "${1:-}" = --remove ]; then
    systemctl --user disable --now focus-board.service || true
    rm -f "$units/focus-board.service"
    systemctl --user daemon-reload
    echo "removed the service; the board itself is kept: $(focus-board where 2>/dev/null || echo '?')"
    exit 0
fi

cargo install --path "$here/.." --locked -q
# The log folder must exist before the service starts (see the unit).
mkdir -p "$units" "${XDG_STATE_HOME:-$HOME/.local/state}/focus-board"
install -m 644 "$here/focus-board.service" "$units/focus-board.service"
systemctl --user daemon-reload
systemctl --user enable focus-board.service
systemctl --user restart focus-board.service
systemctl --user --no-pager --lines=0 status focus-board.service
echo "board: $(focus-board where)"
echo "page:  $(focus-board where --page)"
