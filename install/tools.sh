#!/usr/bin/env bash
# Shared by install/linux.sh and install/macos.sh: writes the focus-board skill for each agent
# tool on this machine from the one template (owner, 2026-11-26, B44 "a"), and each tool's hook:
# Claude Code's entry merged into ~/.claude/settings.json (B45 "d"), Copilot's hook file and
# opencode's plugin. Needs `focus-board` on PATH, and `here`, the install folder.
skill() { mkdir -p "$1" && focus-board skill --tool "$2" > "$1/SKILL.md" && echo "skill: $1/SKILL.md"; }
if [ -d "$HOME/.claude" ]; then
    skill "$HOME/.claude/skills/focus-board" claude
    # The hook entry, merged into the settings without touching anything else (B45 "d").
    focus-board install-hook
fi
if [ -d "$HOME/.copilot" ]; then
    skill "$HOME/.copilot/skills/focus-board" copilot
    mkdir -p "$HOME/.copilot/hooks"
    install -m 644 "$here/copilot/focus-board.json" "$HOME/.copilot/hooks/focus-board.json"
    echo "hook:  $HOME/.copilot/hooks/focus-board.json"
fi
opencode=${XDG_CONFIG_HOME:-$HOME/.config}/opencode
if [ -d "$opencode" ]; then
    skill "$opencode/skills/focus-board" opencode
    mkdir -p "$opencode/plugins"
    install -m 644 "$here/opencode/focus-board.js" "$opencode/plugins/focus-board.js"
    echo "hook:  $opencode/plugins/focus-board.js"
fi
