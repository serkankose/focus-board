// focus-board for opencode (owner, 2026-11-26: "add support for opencode").
//
// Copy to ~/.config/opencode/plugins/. It runs the same `focus-board hook` commands as Claude Code
// and Copilot, so every rule lives in one place:
// - before a shell call, `hook pre-tool-use --note-after` puts the session's ids in front of a
//   board command, or refuses a command that sets them itself;
// - after it, `hook post-tool-use` appends the owner's review note to the output, once per press.
// Whatever goes wrong here, the command runs as it was, and an unidentified write is refused.
// opencode 2.x calls `setup(ctx)` (its shell tool is "shell"); 1.x calls `server()` ("bash").
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

const BIN = process.env.FOCUS_BOARD_BIN
  || path.join(os.homedir(), ".cargo", "bin", process.platform === "win32" ? "focus-board.exe" : "focus-board");
const SHELL_TOOLS = new Set(["shell", "bash"]);

function hook(kind, sessionID, command, extra) {
  const input = JSON.stringify({ tool_name: "Bash", session_id: sessionID, tool_input: { command } });
  try {
    const run = spawnSync(BIN, ["hook", kind, ...extra], { input, encoding: "utf8", timeout: 5000 });
    return run.status === 0 && run.stdout.trim() ? JSON.parse(run.stdout) : null;
  } catch {
    return null;
  }
}

/// Puts the session's ids in front of a board command in `args.command`, or throws the refusal.
function before(tool, sessionID, args) {
  if (!SHELL_TOOLS.has(tool) || typeof args?.command !== "string") return;
  const answer = hook("pre-tool-use", sessionID, args.command, ["--note-after"])?.hookSpecificOutput;
  if (answer?.permissionDecision === "deny") throw new Error(answer.permissionDecisionReason);
  if (typeof answer?.updatedInput?.command === "string") args.command = answer.updatedInput.command;
}

/// The owner's review note for this session, once per press, or null.
function noteAfter(tool, sessionID) {
  if (!SHELL_TOOLS.has(tool)) return null;
  const note = hook("post-tool-use", sessionID, "", [])?.additionalContext;
  return typeof note === "string" ? note : null;
}

// opencode 1.x.
export const FocusBoardPlugin = async () => ({
  "tool.execute.before": async (input, output) => before(input.tool, input.sessionID, output.args),
  "tool.execute.after": async (input, output) => {
    const note = noteAfter(input.tool, input.sessionID);
    if (note) output.output = `${output.output ?? ""}\n\n${note}`;
  },
});

// opencode 2.x loads the default export: an id, and `setup` registering the hooks.
export default {
  id: "focus-board",
  server: FocusBoardPlugin,
  async setup(ctx) {
    await ctx.tool.hook("execute.before", (event) => before(event.tool, event.sessionID, event.input));
    await ctx.tool.hook("execute.after", (event) => {
      if (event.status !== "completed" || !event.result) return;
      const note = noteAfter(event.tool, event.sessionID);
      if (!note) return;
      // The model reads `content` (text parts); `output` holds the shell's own result object.
      if (Array.isArray(event.result.content)) event.result.content.push({ type: "text", text: note });
      else if (typeof event.result.output === "string") event.result.output += `\n\n${note}`;
    });
  },
};
