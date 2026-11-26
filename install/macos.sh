#!/usr/bin/env bash
# Installs focus-board on macOS for the current user (owner, 2026-11-26, B45 "e"): the binary on
# PATH, and the server as a launchd agent that starts at login and restarts after a crash, logging
# to ~/Library/Logs/focus-board/serve.log. Then the skill and each tool's hook (tools.sh).
# UNVERIFIED: written and syntax-checked on Linux; never run on a Mac.
#
# Usage: install/macos.sh            install or update, then (re)start the agent
#        install/macos.sh --remove   stop and remove the agent; the binary and the board stay
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
label=dev.focus-board.serve
plist=$HOME/Library/LaunchAgents/$label.plist
logs=$HOME/Library/Logs/focus-board
domain=gui/$(id -u)

if [ "${1:-}" = --remove ]; then
    launchctl bootout "$domain/$label" 2>/dev/null || true
    rm -f "$plist"
    echo "removed the agent; the board itself is kept: $(focus-board where 2>/dev/null || echo '?')"
    exit 0
fi

cargo install --path "$here/.." --locked -q
mkdir -p "$(dirname "$plist")" "$logs"
bin=$HOME/.cargo/bin/focus-board
cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key><array><string>$bin</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$logs/serve.log</string>
  <key>StandardErrorPath</key><string>$logs/serve.log</string>
</dict>
</plist>
PLIST
launchctl bootout "$domain/$label" 2>/dev/null || true
launchctl bootstrap "$domain" "$plist"
# The skill, and each tool's hook, for the tools on this machine.
source "$here/tools.sh"
echo "board: $(focus-board where)"
echo "page:  $(focus-board where --page)"
