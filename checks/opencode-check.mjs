// Drives the opencode plugin (install/opencode/focus-board.js) against a scratch board, without
// opencode: node checks/opencode-check.mjs BIN. Checks the rewrite, the refusal, and the review
// note after the output, once.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const bin = path.resolve(process.argv[2] || "target/debug/focus-board");
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fb-opencode-"));
process.env.FOCUS_BOARD_BIN = bin;
process.env.FOCUS_BOARD_DB = path.join(dir, "board.sqlite");
for (const v of ["SESSION", "AGENT", "AGENT_TYPE", "TRANSCRIPT"]) delete process.env["FOCUS_BOARD_" + v];
const { FocusBoardPlugin } = await import(path.resolve("install/opencode/focus-board.js"));
const hooks = await FocusBoardPlugin();
const session = "ses_check0001";
let failed = 0;
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${name}${detail ? "   (" + detail + ")" : ""}`);
  if (!ok) failed++;
};

const call = { args: { command: 'focus-board add "From opencode?"' } };
await hooks["tool.execute.before"]({ tool: "bash", sessionID: session, callID: "c1" }, call);
check("a board command gets the session's ids", call.args.command.includes(`FOCUS_BOARD_SESSION='${session}'`), call.args.command.slice(0, 60));
const ran = spawnSync("bash", ["-c", call.args.command], { encoding: "utf8", cwd: dir });
check("and writes as that session", ran.status === 0 && ran.stdout.trim() === "B1", (ran.stdout + ran.stderr).trim());

const other = { args: { command: "ls" } };
await hooks["tool.execute.before"]({ tool: "bash", sessionID: session, callID: "c2" }, other);
check("other commands are left alone", other.args.command === "ls");

let refused = null;
try {
  await hooks["tool.execute.before"]({ tool: "bash", sessionID: session, callID: "c3" },
    { args: { command: "export FOCUS_BOARD_SESSION" + "=x; focus-board list" } });
} catch (e) {
  refused = e.message;
}
check("setting the ids yourself is refused", refused && refused.includes("the hook sets"), refused);

execFileSync("python3", ["-I", "-c",
  "import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); " +
  "c.execute(\"INSERT INTO review_requests (asked_at) VALUES ('t')\"); c.commit()",
  process.env.FOCUS_BOARD_DB]);
const after = { title: "", output: "hello", metadata: {} };
await hooks["tool.execute.after"]({ tool: "bash", sessionID: session, callID: "c4", args: {} }, after);
check("the review note follows the output", after.output.startsWith("hello\n\n") && after.output.includes("B1 (open)"), after.output.slice(0, 80));
const again = { title: "", output: "hello", metadata: {} };
await hooks["tool.execute.after"]({ tool: "bash", sessionID: session, callID: "c5", args: {} }, again);
check("once per press", again.output === "hello");

// opencode 2.x: the default export's setup(ctx) registers the hooks; its shell tool is "shell",
// and the model reads the result's `content` parts.
const registered = {};
const plugin = (await import(path.resolve("install/opencode/focus-board.js"))).default;
await plugin.setup({ tool: { hook: async (name, callback) => { registered[name] = callback; } } });
const event = { tool: "shell", sessionID: session, input: { command: "focus-board whoami" } };
await registered["execute.before"](event);
check("2.x: a board command gets the session's ids", event.input.command.includes(`FOCUS_BOARD_SESSION='${session}'`));
execFileSync("python3", ["-I", "-c",
  "import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); " +
  "c.execute(\"INSERT INTO review_requests (asked_at) VALUES ('t')\"); c.commit()",
  process.env.FOCUS_BOARD_DB]);
const done = { tool: "shell", sessionID: session, status: "completed",
  result: { output: { exitCode: 0, output: "hi\n" }, content: [{ type: "text", text: "hi\n" }] } };
await registered["execute.after"](done);
check("2.x: the note is a text part the model reads",
  done.result.content.length === 2 && done.result.content[1].text.includes("B1 (open)"), JSON.stringify(done.result.content).slice(0, 90));

fs.rmSync(dir, { recursive: true, force: true });
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
