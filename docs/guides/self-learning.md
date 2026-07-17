# Self-Learning and Memory

Catalyst Code includes a persistent **self-learning layer** that lets the AI
remember facts, conventions, and workflows across sessions. This is not a
hidden background process — the model reflects and persists explicitly, in
band with the conversation, so every save is explainable and reviewable.

For a full design discussion, see [`docs/SELF_LEARNING.md`](../SELF_LEARNING.md).

---

## Quick Start

**Save a durable fact:**

```
That's a great convention. Let me remember it.
→ memory(name: "repository-conventions", content: "…", scope: "workspace")
```

**Recall what's already known:**

When you start a new session, every stored memory is automatically injected
into the system prompt. No manual recall needed.

**Learn about a new repository:**

```
/index
```

**Deliberately reflect:**

```
/reflect
```

---

## Memory Store

### Storage format

Memories are **markdown files with YAML frontmatter** stored on disk:

```markdown
---
name: project-conventions
description: Coding conventions for this repository
type: convention
importance: high
---

## Naming

- Rust modules use snake_case file names.
- Public API functions are documented with doc comments.

## Testing

- Integration tests go in `tests/` directory.
- Use table-driven tests for error cases.
```

### Frontmatter fields

| Field | Values | Description |
|-------|--------|-------------|
| `name` | string | Unique memory name (identifier) |
| `description` | string | One-line summary shown in the memory catalog |
| `type` | `convention` / `decision` / `gotcha` / `architecture` / `note` / `preference` / `user` / `identity` | Category |
| `importance` | `high` / `normal` / `low` | Prominence in the prompt |
| `scope` | `workspace` / `global` | Visibility boundary |
| `pinned` | `true` / `false` | Always included regardless of relevance |

### Auto-indexing

A `MEMORY.md` file is maintained automatically in each memory directory. It
lists all memories with their name, type, and description — this is what the
model sees in the system prompt catalog.

---

## Scope: Global vs Workspace

### Workspace (default)

Stored at `~/.config/catalyst-code/memory/<workspace-hash>/` — scoped to the
current project directory. Different projects have entirely separate memory
stores.

Use for: project conventions, architecture decisions, repository-specific
gotchas, local build knowledge.

### Global

Stored at `~/.config/catalyst-code/memory/global/` — shared across **all**
projects. A global fact is available regardless of which workspace is open.

Use for: the user's name, preferred tech stack, harness conventions, identity
facts that apply to every project.

Choose the scope when saving:

- `memory(name: "my-fact", content: "…", scope: "workspace")` — default
- `memory(name: "my-fact", content: "…", scope: "global")` — cross-repo

---

## Slash Commands

### `/remember <text>`

Save a quick memory from the TUI. Opens a modal where you type the fact to
store. The core saves it as a new workspace memory.

### `/memory`

List all stored memories. The TUI displays the catalog: each memory's name,
type, description, and importance.

### `/forget`

Open the memory picker to select and delete a memory. Deletion is permanent
(removes the `.md` file from disk).

---

## `/index` — Bootstrap Learning

Run `/index` when you first open an unfamiliar repository. The core:

1. Scans the repository structure (source files, tests, config, build scripts).
2. Builds a codebase index covering directories, key files, entry points,
   dependency graphs.
3. Identifies the project type, language ecosystem, and build system.
4. Discovers public APIs, commands, and configurations.
5. Persists the index as memory entries so future sessions in the same repo
   start with context.

This is a **learning pass** — it is exempt from auto-reflect because the
indexing itself IS the reflection.

### Incremental index

```
Run an incremental knowledge index of this repository
```

Updates only changed portions without re-scanning the entire codebase.

---

## `/reflect` — Deliberate Learning

Run `/reflect` at any point (not just turn end) to:

1. Review the work done so far in the current session.
2. Distill durable facts (conventions, decisions, gotchas, architecture).
3. Save them via the `memory` tool.
4. Optionally write a skill under `.catalyst-code/skills/`.

Like `/index`, this is exempt from auto-reflect.

---

## Auto-Reflect

At the end of every non-trivial turn (at least one tool call, by default), the
core injects a **reflect nudge** into the model's completion prompt:

> [auto-reflect] Before you write your completion summary, reflect on this
> turn. (1) If you learned a durable convention, architecture fact, decision,
> or gotcha, persist it with the `memory` tool… (2) If you just performed a
> reusable workflow, consider writing a skill…

### Configuration

| Config key | Default | Description |
|------------|---------|-------------|
| `auto_reflect` | `true` | Enable auto-reflect on turn completion |
| `auto_reflect_min_tool_calls` | `1` | Minimum tool calls for auto-reflect to trigger |

The nudge also includes **recurring pattern detection** — if the same file
categories (e.g. "edit|core/src/*.rs") have been modified 2+ times across
sessions, the pattern is included in the nudge so the model knows to write a
skill.

### Memory efficiency

- **Deduplication**: On each save, a memory with the same `name` is updated,
  not duplicated. Use `append` action to add to an existing topic memory.
- **Importance**: Set `importance: high` for durable facts that should survive
  compaction or pruning. The store auto-pins `convention`, `decision`, `gotcha`,
  and `identity` type memories.
- **Catalog**: The system prompt carries a capped MEMORY CATALOG (name + one-line
  description). Full text is fetched on demand via `memory` action `get`.

---

## Skills System

A **skill** is a markdown file at `.catalyst-code/skills/<name>/SKILL.md` with
YAML frontmatter and a body describing when and how to perform a reusable
workflow:

```markdown
---
name: api-endpoint-creation
description: Add a new HTTP endpoint following this repo's conventions
triggers: [add endpoint, new route, add api]
---

## When to use
You are adding a new HTTP route/handler.

## Steps
1. Read an existing endpoint in src/routes/ to mirror its structure.
2. Add the handler + route registration.
3. Add the request/response types.
4. Add a test mirroring tests/routes/.

## Example
…
```

### How skills are used

- Read from the prompt: `/skill:<name>` references a skill that the model has
  already seen. If the skill is in `.catalyst-code/skills/`, the model can also
  read it with `read_file`.
- The `pi-subagents` skill is kept as a short stub in the orchestrator system
  prompt; the full playbook is opt-in via `/skill:pi-subagents`.
- After auto-reflect, the model may write a new skill if a recurring pattern is
  detected.

### When to write a skill

A skill should be written when:

1. You performed a reusable workflow (same shape 2+ times).
2. No existing skill covers it.
3. The workflow benefits from a structured approach rather than model improvisation.

Skills are advisory prompt fragments, not executable code. If you need
deterministic side effects, write a **plugin** instead.

---

## Memory Injection

Every session starts by injecting all stored memories into the **stable system
prompt**. This prompt is built once on turn 1 and refreshed only when memories
or skills change, preserving the provider's prefix cache.

The injection includes:

- **Memory catalog**: each memory's name and one-line description (capped to
  avoid overflowing the prompt).
- **Relevant memory tails**: if the current prompt matches keywords in stored
  memories, those memories' full text is included.

After any memory mutation (save/append/forget), the injection is refreshed so
the next turn uses the updated catalog.

---

## The `memory` AI Tool

The model can call the `memory` tool directly during a turn. It is classified
**ReadOnly** (no approval gate needed) because it only touches
`~/.config/catalyst-code/memory/`, never the workspace.

### Actions

| Action | Description |
|--------|-------------|
| `save` | Create a new memory. Requires `name` and `content`. Optional: `type`, `description`, `scope`, `importance`, `force`. |
| `append` | Append content to an existing memory (same `name`). If the memory doesn't exist, creates it. |
| `list` | Show the memory catalog (all names + descriptions for the current scope). |
| `forget` | Delete a memory by `id` (name). |
| `get` | Retrieve the full text of a single memory by `id`. |
| `consolidate` | Merge near-duplicate memories. |
| `stats` | Show recall quality metrics (hit rate, cache efficiency). |

### Example

```json
{
  "action": "save",
  "name": "project-build-system",
  "content": "This repo uses Cargo workspaces with a root workspace Cargo.toml.\n`cargo build -p core` builds only the core crate.",
  "type": "architecture",
  "scope": "workspace",
  "importance": "high"
}
```

---

## The `knowledge` AI Tool

The `knowledge` tool provides read-only access to structured codebase
intelligence. It is classified **ReadOnly** and returns compact strings.

### Actions

| Action | Description |
|--------|-------------|
| `context` | Current workspace context (structure, entry points) |
| `search` | Search the codebase index for symbols or text |
| `symbol` | Resolve a symbol to its definition location |
| `related` | Find related files or modules (change coupling) |
| `episodes` | Retrieve past coding episodes relevant to the query |
| `preferences` | User's project-level preferences |
| `rejected` | Previously rejected approaches (avoid repeating mistakes) |
| `coverage` | Documentation coverage report |
| `explain` | Explain a file or function from the index |
| `tests` | Find tests for a given source file or symbol |

All actions are **fail-open** ��� they return a best-effort string even when the
index is incomplete.

---

## Plugin Hook: `session_stop`

The `session_stop` plugin hook fires at the end of each turn. This is the
**deterministic seam** for side-effecting reflection — if you want to run
custom learning logic (e.g. writing a telemetry record or a derived memory)
without trusting the model's prompt compliance, implement a plugin that
hooks `session_stop`.

---

## Design Principles

For the full design rationale, read [`docs/SELF_LEARNING.md`](../SELF_LEARNING.md).
Key decisions:

- **Memory is the AI's notebook, not the user's codebase.** Memory files live in
  `~/.config/catalyst-code/memory/`, never in the workspace, so mutations are
  ReadOnly from the workspace perspective.
- **Learning is prompt-driven, not a hidden daemon.** The model reflects in-band
  where the user can see and steer it.
- **Markdown, not a vector database.** A markdown file on disk beats a vector
  database until retrieval measurably fails. The upgrade path to embedding-based
  retrieval is documented but deferred (see [`src/embed.rs`](../../core/src/embed.rs)).
- **Skills are prompt fragments, not workflows.** A skill advises the model; a
  plugin executes deterministic side effects.

---

## Related

- [Full design document: `docs/SELF_LEARNING.md`](../SELF_LEARNING.md)
- [Memory store implementation: `core/src/memory.rs`](../../core/src/memory.rs)
- [Knowledge tool implementation: `core/src/knowledge_tool.rs`](../../core/src/knowledge_tool.rs)
- [Skills directory: `.catalyst-code/skills/`](../../.catalyst-code/skills/)
- [Plugin authoring guide](../plugins/index.md)
