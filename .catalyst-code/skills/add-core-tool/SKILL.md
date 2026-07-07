---
name: add-core-tool
description: Add a new built-in tool to the Rust core (schema, approval class, dispatch, guards)
version: 1
---

## When to use

You are extending `catalyst-code`'s Rust core with a new tool the agent can call
(via OpenAI function-calling). Every existing tool — `read_file`, `edit`, `bash`,
`grep`, `fetch`, `subagent`, `memory`, `git_*`, … — was added by following this
exact shape, so this is the canonical workflow when the repo needs a new
agent-callable capability.

## Where things live

- **Schema** — `core/src/tools.rs::definitions()` returns the OpenAI
  function-calling JSON for every tool (append one `json!({"type":"function","function":{...}})`).
- **Approval class** — `core/src/tools.rs::classify(name) -> ToolKind`.
  `ToolKind::ReadOnly` = never gated; `ToolKind::Destructive` = gated under
  `Approval::Destructive` (the default). Anything not listed is `Destructive`.
- **Sync dispatch** — `core/src/tools.rs::execute(name, args, cfg) -> Outcome`
  is a `match name { ... }`. Add your arm here for a pure/sync tool.
- **Async dispatch** — tools that need async I/O (bash, fetch, diagnostics,
  subagent, intercom, bulk) are NOT run from `execute()`. Instead `execute()`
  returns `Outcome::err("<name> must be dispatched through execute_<name> (async)")`
  as a sentinel, and `main.rs` routes them to a real `async fn execute_<name>`
  before calling `execute()`. Add your async path there.
- **Dangerous-path guard** — `workspace::check_dangerous_path` is applied to
  `write_file`/`edit`/`patch` (and the subagent tool dispatch) to block writes
  to `.git/**`, `**/.ssh/**`, `**/.env*`, etc. Wire it for any tool that writes.

## Steps

1. **Schema** — append to `definitions()` in `tools.rs`. Give it a precise
   `description` (the model reads this to decide when to call it) and a tight
   `parameters` JSON schema. List `required` fields.
2. **Classify** — add the tool name to the `ReadOnly` match list in
   `classify()` IF it only reads/inspects (no side effects). Leave it out
   (→ `Destructive`) if it writes, shells out, or has side effects.
3. **Implement** — write the logic. For a sync tool, add a `fn <name>(...)`
   returning `Outcome` and a match arm in `execute()`. For an async tool, add
   the sentinel err in `execute()` AND an `async fn execute_<name>` routed in
   `main.rs`'s tool loop. Use `workspace::resolve(&cfg.workspace, path)` to
   confine file paths; reject absolute paths / `..` (the resolver does this).
4. **Guards** — if the tool writes files or runs commands, apply
   `workspace::check_dangerous_path` to the target path/command exactly as
   `write_file`/`edit`/`patch` do.
5. **Approvals** — if `Destructive`, the existing approval gate +
  `approval_request` (with a `diff` preview for write/edit/patch) already
   covers it. If `ReadOnly`, no gate. `approve` with decision `"always"`
   upgrades only the matched tool *kind*.
6. **Verify** — `cd core && cargo fmt --all && cargo clippy --all-targets &&
  cargo test --locked`. Add a unit test next to the tool fn (mirrors the
  `#[cfg(test)] mod tests` style used throughout `tools.rs`/`config.rs`).

## Example (a sync read-only `count_lines` tool)

```rust
// 1) schema in definitions()
json!({"type":"function","function":{
    "name":"count_lines","description":"Count lines in a file.",
    "parameters":{"type":"object","properties":{
        "path":{"type":"string"}
    },"required":["path"]}}}),

// 2) classify — add to the ReadOnly list
"count_lines" => ToolKind::ReadOnly,

// 3) implement + dispatch
fn count_lines(input: &str, cfg: &Config) -> Outcome {
    let path = match workspace::resolve(&cfg.workspace, input) {
        Ok(p) => p, Err(e) => return Outcome::err(e) };
    match std::fs::read_to_string(&path) {
        Ok(s) => Outcome::ok(s.lines().count().to_string()),
        Err(e) => Outcome::err(format!("count_lines {input:?} failed: {e}")),
    }
}
// in execute():
"count_lines" => count_lines(s("path"), cfg),
```

## Gotchas

- An async tool that is ONLY in `execute()` (no sentinel + no `execute_<name>`
  in `main.rs`) will return its err string to the model as a "tool_result" and
  silently never run. Always wire BOTH the sentinel and the async path.
- `Outcome` has an optional `diff` field — populate it for write/edit tools so
  the `approval_request` event carries a diff preview the TUI/web can render.
- Subagents get the SAME tool set minus what their frontmatter `tools` omits
  and minus `memory` (orchestrator-owned). A tool meant for subagents must also
  be safe under their narrower context.
