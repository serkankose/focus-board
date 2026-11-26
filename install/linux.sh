#!/usr/bin/env bash
# Installs focus-board on Linux for the current user: the binary on PATH, and the server as a
# systemd user service that starts at login. It also writes the focus-board skill for each agent
# tool that is installed (Claude Code, Copilot, opencode) from the one template, `install/skill.md`
# (owner, 2026-11-26, B44 "a"), Copilot's hook file and opencode's plugin, and merges Claude
# Code's hook entry into ~/.claude/settings.json with `focus-board install-hook`, which changes
# nothing else there and keeps a copy of the previous file (B45 "d").
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
# The skill, and each tool's hook, for the tools on this machine.
source "$here/tools.sh"
echo "board: $(focus-board where)"
echo "page:  $(focus-board where --page)"
