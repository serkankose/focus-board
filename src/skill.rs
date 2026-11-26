//! The `focus-board` skill for each tool (owner, 2026-11-26, B44 "a"): one template,
//! `install/skill.md`, with `<!-- only: TOOLS -->` … `<!-- end -->` blocks for the parts that
//! differ, so the Claude Code, Copilot and opencode copies never drift. `install/linux.sh` writes
//! each tool's copy with `focus-board skill --tool TOOL`.

const TEMPLATE: &str = include_str!("../install/skill.md");

/// The tools the skill is written for.
pub const TOOLS: [&str; 3] = ["claude", "copilot", "opencode"];

/// The skill as `tool` reads it, or `None` for a tool it is not written for. A marker line starts
/// (or switches) a block kept only for the tools it names; `<!-- end -->` closes it.
pub fn skill_for(tool: &str) -> Option<String> {
    if !TOOLS.contains(&tool) {
        return None;
    }
    let mut out = Vec::new();
    let mut keep = true;
    for line in TEMPLATE.split('\n') {
        if let Some(names) = line
            .strip_prefix("<!-- only:")
            .and_then(|rest| rest.strip_suffix("-->"))
        {
            keep = names.split_whitespace().any(|name| name == tool);
        } else if line == "<!-- end -->" {
            keep = true;
        } else if keep {
            out.push(line);
        }
    }
    Some(out.join("\n"))
}
