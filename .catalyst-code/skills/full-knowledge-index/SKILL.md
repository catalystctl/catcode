---
name: full-knowledge-index
description: Bootstrap durable repo learning via /index-style walk — layout, manifests, architecture, conventions, build/test, memories, and candidate skills
version: 1
---

## When to use

Use when the user asks to **index**, **bootstrap learning**, run a **full knowledge audit**, or `/index` on a repository — deeper than a quick "what is this codebase?" overview.

Prefer `/skill:codebase-overview` for a short orientation that does **not** persist memories.

## Steps

1. **Recall first.** `memory` action=list (workspace). `get` any architecture/build memories. Treat them as hypotheses — verify against disk.
2. **Top-level walk.** `list_dir` `.` then major dirs (`core/`, `tui/`, `sdk/`, `web/`, `docs/`, `.catalyst-code/`, `.github/`, `packaging/`).
3. **Read anchors.** README (purpose + architecture + install), CONTRIBUTING, primary manifests (`Cargo.toml`, `go.mod`, `package.json`), `build.sh` / release scripts, CI workflows.
4. **Map subsystems.** For each component: entry point, role, how it talks to others (here: stdio JSONL). Spot tests and deploy paths.
5. **Diff vs memory.** If module counts, versions, or editor stacks drifted — `memory` action=append on the existing name. Do not duplicate near-identical saves.
6. **Persist durables.** `memory` action=save with types `architecture` | `convention` | `api` | `gotcha` | `build`. One focused memory per concern; always pass `description`. Prefer `append` when updating.
7. **Skills gap.** `list_dir .catalyst-code/skills/`. If a workflow you have solved **2+ times** has no skill, write `.catalyst-code/skills/<name>/SKILL.md` (frontmatter name/description; body when-to-use + steps + example).
8. **Report.** List memories touched, any new skills, and **one area of least confidence**.

## Example

User: "Run a full knowledge index of this repository."

Agent: walks layout → verifies README vs `core/src` file count → appends `repo-top-level-architecture` → saves `build-test-release-flow` → writes `full-knowledge-index` skill if missing → finishes with memory names + "least confident: learning_* retrieval quality."

## Avoid

- Do not invent plugin schemas — use `/skill:plugin-authoring`.
- Do not treat README module lists as complete if `list_dir` shows more files.
- Do not create skills for one-off tasks.
