# Self-Learning AI Software Development System — Design & Implementation

This document is the design for the self-learning layer built into the
catalyst-code. It is **opinionated and minimal**: the harness already had most of
the bones (memory store, skills, subagents, plugin hooks, telemetry). This layer
welds them into a learning loop and documents the seams.

> Design principle (ponytail): reach for the standard mechanism that already
> exists before building new infrastructure. A markdown file on disk beats a
> vector database until retrieval measurably fails. The first lazy solution
> that works is the right one — every "fancy" layer below has a named ceiling
> and a named upgrade path.

---

## 0. What was already there (do not rebuild)

Before designing, audit what exists. The harness shipped with:

- **`core/src/memory.rs`** — a persistent memory store: markdown files with
  YAML frontmatter, per-workspace scoped (hashed canonical path). `save_memory`,
  `append_memory`, `forget_memory`, `scan_memories`, `memory_injection`,
  `project_hash`, a `MEMORY.md` auto-index, and a rolling-byte cap on append.
- **`core/src/tools.rs`** — a `memory` AI tool (actions `save`/`append`/`list`/
  `forget`) reusing `memory.rs`. Classified `ReadOnly` (no approval gate).
  Reaches the main orchestrator.
- **`.catalyst-code/skills/`** — skills as markdown + YAML frontmatter. The
  `pi-subagents` skill is injected into the orchestrator's system prompt.
- **`.catalyst-code/agents/`** — 8 built-in subagents (scout, researcher,
  planner, worker, reviewer, context-builder, oracle, delegate) + an in-process
  intercom bus for orchestrator↔child and child↔child coordination.
- **`core/src/plugins.rs`** — a plugin/hook system: `session_start`,
  `session_stop`, `pre_compact`, `pre_turn`, and `pre_/post_` per-tool hooks.
- **Telemetry** — per-turn `metrics` event (TTFT, elapsed, tokens in/out,
  cached tokens, TPS) + a JSONL debug log (`init`, `tool`, `turn_done`, …).
- **Slash commands** — `/remember`, `/memory`, `/forget` already map to core
  memory commands.

So the gap was never "build a learning system from scratch." The gap was that
the pieces were inert. The implementation work (see §15) was:

1. **Fix the critical injection bug** — `memory_injection(workspace, "")` was
   called with an empty prompt everywhere; keyword relevance matched nothing, so
   **memories were never actually injected**. The whole subsystem was dead.
2. **Add `append`** to the `memory` tool (the accumulation primitive existed in
   `memory.rs` but was not exposed to the model).
3. **Teach the model to learn** — a self-learning protocol in the system prompt
   (reflect before finishing; persist durable facts; write skills when a
   pattern repeats).
4. **Bootstrap + reflection commands** — `/index` and `/reflect`.

Everything below is the design around that minimal core, with explicit notes on
what is deliberately deferred.

---

## 1. High-level architecture

```
                        ┌───────────────────────────────────────────┐
                        │              TUI (Go / Bubble Tea)          │
                        │  /index  /reflect  /remember  /memory ...  │
                        └───────────────┬───────────────────────────┘
                                        │  newline-delimited JSON (stdio)
                                        ▼
┌──────────────────────────────────────────────────────────────────────┐
│                         CORE (Rust, async)                           │
│                                                                      │
│   ┌─────────┐   ┌────────────┐   ┌───────────┐   ┌───────────────┐  │
│   │  turn   │──▶│ tool loop   │──▶│  memory   │──▶│  ~/.config/   │  │
│   │  loop   │   │ (memory,   │   │  store ◀──│───│  catalyst-code│  │
│   │         │   │  read/edit/│   │ (markdown │   │  /memory/     │  │
│   │ pre_turn │   │  bash/sub- │   │  + front- │   │  <hash>/      │  │
│   │  hook    │   │  agent…)   │   │  matter)  │   │  *.md         │  │
│   └────┬────┘   └─────┬──────┘   └─────┬─────┘   └───────────────┘  │
│        │              │                │ injection (standing prompt)   │
│        ▼              ▼                ▼                               │
│   ┌──────────┐  ┌──────────────┐  ┌──────────────┐                     │
│   │ metrics  │  │ subagents +  │  │ system prompt│  ◀── skills manifest│
│   │ + JSONL  │  │ intercom     │  │ (built once, │      (pi-subagents) │
│   │ telemetry│  │ (scout/rev/  │  │  refreshed   │                     │
│   │          │  │  worker…)    │  │  on change)  │                     │
│   └──────────┘  └──────────────┘  └──────────────┘                     │
│        │                                                              │
│        ▼           ┌──────────────────────────────────────┐           │
│   session_stop ───▶│  plugin hooks (reflection/telemetry   │           │
│   (per-turn)       │   seam — write your own learning      │           │
│                    │   plugin here)                        │           │
│                    └──────────────────────────────────────┘           │
└──────────────────────────────────────────────────────────────────────┘
```

Key invariants:

- **One stable system prompt** built on turn 1 and refreshed only when memories
  or skills change. This preserves the provider prefix cache (the harness
  explicitly tracks `cached_tokens`); per-turn prompt churn would torch the
  cache. Memories live *inside* the stable prompt, not in a per-turn message.
- **Memory is the AI's notebook, not the user's codebase.** Saving a memory
  touches `~/.config/catalyst-code/memory/<hash>/`, never the workspace, so it
  is `ReadOnly` and needs no approval gate.
- **Learning is prompt-driven + tool-backed, not a hidden daemon.** The model
  reflects and persists; a `session_stop` plugin hook is the deterministic seam
  if you want side-effecting reflection without trusting the prompt.

---

## 2. The end-to-end learning lifecycle

```
NEW TASK
  │
  ▼
[recall]  standing system prompt already carries all prior memories
  │       (injection fixed: empty prompt ⇒ inject ALL memories)
  ▼
[plan + act]  model reads/searches/edits, may delegate to subagents
  │
  ▼
[reflect]  before signaling done, one reflection step:
  │        what convention / architecture fact / decision / gotcha
  │        should future sessions NOT rediscover?
  ▼
[persist]  memory tool: append (topic exists) or save (new)
  │        optional: write_file a skill under .catalyst-code/skills/
  ▼
[NEXT SESSION]  memories re-injected into the standing prompt → compounding
```

The loop is deliberately synchronous and visible: the model reflects in-band
(where the user can see and steer it) rather than in a hidden background
process. This keeps the system **explainable and reviewable** — the two
properties the brief demands.

---

## 3. Skill system

### What is a skill?

A skill is **a markdown file with YAML frontmatter** at
`.catalyst-code/skills/<name>/SKILL.md`. Concretely it is **a prompt fragment**
(the body) with **metadata** (the frontmatter). It is *not* executable code, a
workflow graph, or a tool composition — those are heavier representations that
this harness does not need because the model itself is the executor.

A skill answers: *"when you face this shape of problem, follow this approach."*

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

### Why markdown-and-prompts (not code/workflows/graphs)?

- The model already reasons over markdown natively; no interpreter needed.
- Markdown diffs are reviewable in `git`; code-skill diffs require running them.
- A prompt-fragment skill composes with the model's judgment; a rigid workflow
  graph fights the model on edge cases.
- **Upgrade path:** if a skill needs deterministic side effects (e.g. a
  scaffolder that must run), it becomes a *plugin* (executable hook), not a
  skill. Skills stay advisory.

### Versioning, testing, documentation, discovery

| Concern        | Decision                                                                 |
|----------------|--------------------------------------------------------------------------|
| Versioning     | Frontmatter `version: 1` field (semantic: bump on behavior change).    |
|                | No SemVer machinery yet — `git` is the version store.                    |
| Testing        | Skills are prompts; "testing" = applying them to a benchmark task and   |
|                | checking the result. No unit-test harness for prompts in v1.            |
| Documentation  | The skill file IS the documentation (frontmatter + body).                |
| Discovery      | `list_dir .catalyst-code/skills/` + read `SKILL.md`. The orchestrator   |
|                | skill (`pi-subagents`) is auto-injected; others are opt-in.              |
|                | **Deferred:** a one-line skill manifest in the system prompt (note §15).|

### When to create a skill vs solve ad hoc

Create a skill when the **same shape of problem has been solved more than
twice**. The system prompt encodes this rule explicitly. Solving once is ad-hoc
(isolated to the session); twice is a coincidence; three times is a pattern worth
a skill. This threshold prevents skill-spam.

### Duplicate detection & merging

No automated duplicate detection in v1. The model is told to `list_dir` the
skills folder before writing a new one, and to extend an existing skill rather
than create a parallel one. **Upgrade path:** when the skill count grows, add a
manifest + fuzzy name match at write time. Speculative now → YAGNI.

---

## 4. Knowledge system (memory)

### Storage: markdown files, not a vector/graph DB

```
~/.config/catalyst-code/memory/<workspace-hash>/
├── MEMORY.md            # auto-generated index (name → file → description)
├── architecture.md      # frontmatter + body
├── conventions.md
├── api-shape.md
└── gotchas.md
```

Each file:

```markdown
---
name: conventions
description: tabs, no unwrap in prod, error messages lowercase
type: convention
---
use tabs
--- appended ---
no unwrap in prod
…
```

### Why not a vector DB / graph DB?

- **Vector DB**: solves semantic retrieval at scale (thousands of memories). A
  workspace has tens. Keyword injection into a stable prompt is O(n) where n≈20.
  A vector DB adds an embedding model dependency, a daemon, and nondeterminism
  — for zero measurable gain below ~500 memories. **Ceiling:** keyword match
  misses synonyms. **Upgrade path:** swap `memory_injection` for an embedding
  ANN search when n > 500 or when synonym misses are observed.
- **Graph DB**: models relationships (module→depends-on→module). The harness
  already has `grep`/`glob`/`read_file` + the model itself, which trace
  relationships on demand far more flexibly than a frozen graph. **Ceiling:** a
  graph is stale the moment code changes. **Upgrade path:** a generated
  `architecture.md` memory *is* the human-readable graph, regenerated by
  `/index`.

### Retrieval quality

Standing-prompt injection includes every memory (empty-prompt ⇒ all), so
retrieval recall is 100% by construction — the model always sees all memories.
**Ceiling:** at high memory count this bloats the prompt. The per-memory preview
is capped at 5 lines and `append_memory` has a rolling byte cap, bounding cost.

### Long-term memory, confidence, aging, conflicts

| Concern        | Decision                                                                 |
|----------------|--------------------------------------------------------------------------|
| Long-term      | Memories persist as files across sessions. `append_memory` runs at       |
|                | compaction so durable facts survive context truncation.                  |
| Confidence     | No numeric confidence in v1. The model's reflection decides what is       |
|                | "durable." Confidence scoring is speculative until we measure what       |
|                | the model over/under-saves. **Upgrade:** add a `confidence` frontmatter  |
|                | field once a telemetry plugin (§11) records save→correction rates.       |
| Aging           | The rolling cap on `append_memory` ages out the oldest facts when a       |
|                | memory grows past 8 KB. Explicit `forget` removes stale entries.          |
| Conflict        | Last-write-wins per memory name (a `save` overwrites; an `append`        |
|                | accumulates). The model is told to `append` to existing topics, which    |
|                | avoids clobbering. No automated conflict resolution — the model resolves |
|                | by reading the existing memory before writing.                           |

---

## 5. Learning pipeline (post-task)

Automated, in-band, and visible:

1. **Detect completion** — the model calls `finish` (or stops producing tool
   calls). No new infrastructure; the existing turn loop already ends turns.
2. **Reflect (model-driven)** — the system-prompt protocol fires: the model
   takes one reflection step before done.
3. **Classify** what was learned into one bucket:
   - *durable fact* → `memory` (append or save)
   - *reusable pattern (seen ≥2×)* → `write_file` a skill
   - *neither* → nothing (do not persist trivia)
4. **Persist** via the existing `memory` tool / `write_file`.
5. **(Optional, deterministic)** a `session_stop` plugin hook can independently
   log telemetry or even call the model to extract facts — see §11.

### What was learned / reusable / doc / test / arch — decision rules

| Signal                                  | Action                                       |
|-----------------------------------------|----------------------------------------------|
| New convention discovered               | `memory save` (type: convention)            |
| Architecture understood                 | `memory save` (type: architecture)            |
| Decision made + rationale               | `memory save` (type: decision)               |
| Gotcha/surprise hit                     | `memory append` to relevant topic            |
| Same workflow solved ≥2×                | `write_file` a skill                         |
| Existing doc now wrong                  | `edit` the doc (normal tool, not memory)     |
| Untested critical path just written     | write a test (normal tool)                   |

---

## 6. Skill creation lifecycle

```
Task completed
  ↓
Reflection (system-prompt protocol)
  ↓
"Have I solved this shape ≥2×?" ── no ──▶ stop (ad-hoc is fine)
  ↓ yes
list_dir .catalyst-code/skills/   (check for an existing skill to extend)
  ↓
write_file .catalyst-code/skills/<name>/SKILL.md
  ↓
(next session) skill is discoverable via list_dir; model reads it when relevant
```

### Can this be improved?

The brief's 11-stage lifecycle (reflect → extract → generate → document → test
→ validate → benchmark → store → retrieve) is over-specified for prompts.
Stages collapse: *generate ≡ document* (the file is both), *test/benchmark* are
manual (apply the skill, judge the result), *validate* is human review at `git`
commit time. Forcing an automated validation/benchmark stage before a prompt-fragment
skill is "trusted" would add machinery that, for advisory prompts, buys nothing
— the model still exercises judgment when applying it. **Add staged validation
only when skills become executable** (i.e., when they graduate into plugins).

---

## 7. Skill validation

Before a skill is "permanent": **it is already reviewable as a `git` diff.**

- **Static analysis / linting**: not applicable to markdown prompts.
- **Unit/integration tests**: not applicable until skills become executable.
- **Benchmark tasks**: manual — apply the skill to a representative task and
  judge. No benchmark harness in v1.
- **Human approval**: the existing approval gate (`destructive` mode) already
  gates `write_file`, so creating a skill requires consent unless the user
  escalated `write_file` to `always`. The diff is reviewable at commit.
- **Confidence scoring**: deferred (§4).
- **Staging area**: **no separate staging area.** Skills are advisory; a bad
  skill just makes the model slightly worse at one task, recoverable by `git
  revert`. A staging area is justified only for *executable* skills (plugins),
  where a bad one can cause real damage. The plugin system already separates
  enable/disable for that case.

---

## 8. Skill evolution

| Operation      | Mechanism                                                    |
|----------------|--------------------------------------------------------------|
| Bug fix        | `edit` the SKILL.md body.                                   |
| Optimization   | `edit` the body; bump `version` frontmatter.                |
| API change     | `edit` the steps/example to match the new API.              |
| Deprecation    | `edit` to add a `deprecated: true` frontmatter note, or     |
|                | `bash rm` the skill dir.                                     |
| Replacement    | Write the new skill; add a `replaced_by:` frontmatter to the |
|                | old one (advisory).                                          |
| Merging        | `edit` one body to absorb the other; `rm` the other.         |
| Splitting      | `write_file` two skills from one; `rm` the original.        |

Semantic versioning: frontmatter `version` integer, bumped on behavior change.
No automated SemVer enforcement — `git` history is the audit log.

---

## 9. Repository structure

The harness uses **two** storage roots, by design:

### Per-workspace learning (the AI's notebook — NOT in the repo)

```
~/.config/catalyst-code/memory/<workspace-hash>/
├── MEMORY.md            # auto-index
└── *.md                 # memories (frontmatter + body)
```

This is deliberately outside the repo so the AI's scratch memory doesn't pollute
the user's commits. It is per-workspace (hashed canonical path) so different
projects don't cross-contaminate.

### In-repo, shareable artifacts (versioned with the code)

```
.catalyst-code/
├── agents/              # subagent definitions (markdown + frontmatter)
│   ├── scout.md
│   ├── reviewer.md
│   └── …
├── skills/              # reusable skills (markdown + frontmatter)
│   └── pi-subagents/SKILL.md
├── plugins/             # executable hooks (the "trusted code" tier)
│   └── vision-handoff/
└── vision.json
```

### Why not `memory/`, `knowledge/`, `workflows/`, `benchmarks/`, … in the repo?

The brief's 10-directory layout is over-engineered for v1:

- `memory/` vs `knowledge/` — the same thing (durable facts). One store, not two.
- `workflows/` — workflows are *skills* (prompts) or *subagent chains* (already
  supported via `/chain`). No new dir.
- `prompts/` — skills ARE prompts. No separate dir.
- `benchmarks/`, `evaluations/`, `telemetry/`, `experiments/` — speculative
  until a measurement program exists. The JSONL debug log + `metrics` event
  already capture the raw signal; a `telemetry/` dir is added when something
  needs to *write* derived metrics. **YAGNI now.**

The minimal layout (memory outside repo; skills/agents/plugins inside) keeps
the diff small and the boundaries clear: *outside = private notebook, inside =
shareable, reviewable artifacts.*

---

## 10. Retrieval system

When the model receives a task, retrieval is **implicit and free**: the standing
system prompt already contains all memories (recall = 100%). There is no
separate retrieval step, no embedding query, no "before/during/after planning"
ordering decision — it just works because the memories are in the prompt the
model already has.

For **skills**, retrieval is on-demand: the model `list_dir`s the skills folder
and reads the relevant `SKILL.md` when a task matches a skill's description. This
costs one tool call but keeps the prompt lean (skill bodies can be long).

| Retrieval target        | Mechanism                              | When          |
|-------------------------|----------------------------------------|---------------|
| Memories (facts)        | Standing prompt injection              | Always (free) |
| Skills (workflows)      | `list_dir` + `read_file`               | On-demand     |
| Previous implementations| `grep` / `read_file` in the workspace  | On-demand     |
| Architecture decisions  | A memory of type `decision`/`architecture` | Always (free) |
| Related bugs            | A memory of type `gotcha`              | Always (free) |

**Ceiling:** standing-prompt injection of all memories stops scaling past ~500
memories. **Upgrade path:** per-turn embedding retrieval into a *separate*
system message (not the stable prefix) so the cache survives. This is the
single largest deferred decision and it is flagged with a measured trigger.

---

## 11. Reflection engine

The reflection happens **in-band, before the model signals done**, driven by the
system-prompt protocol:

> Before signaling done on a non-trivial task, take one reflection step: what
> convention, architecture fact, decision, or gotcha did you learn that future
> sessions should not have to rediscover?

Structured reflections (the brief's "what went well / failed / surprised /
repetitive"):

| Question                | Captured as                                 |
|-------------------------|---------------------------------------------|
| What went well?         | (not persisted — no durable value)          |
| What failed?            | `memory` gotcha, if reusable                |
| What became repetitive? | candidate skill (seen ≥2×)                  |
| What should be stored?  | `memory` save/append                        |
| What surprised the model?| `memory` gotcha                            |
| What assumptions wrong? | (corrects the memory via append/overwrite) |

Why in-band and not a separate reflection daemon?

- **Explainable**: the user sees the reflection in the transcript and can steer it.
- **No extra model calls**: a background reflection daemon doubles token cost.
- **Deterministic seam — now WIRED (auto-reflect):** the `session_stop` hook
  fires after *every* turn (despite the "session" name, it is turn-scoped —
  `session_start` fires at `run_turn` entry, `session_stop` at exit). On top
  of the prompt-driven protocol, the core now injects an **auto-reflect
  continuation** at the first `finish`/natural completion of any non-trivial
  turn (≥ `auto_reflect_min_tool_calls` tool calls): instead of exiting, it
  nudges the model with a reflect prompt (persist durable facts → `memory`;
  write a skill if a pattern recurs) and re-streams. `reflected` prevents
  re-entry (the reflect's own `finish` exits for real). `/reflect` and `/index`
  turns are exempt (they ARE reflections); trivial turns (no real work) are
  exempt by the tool-call gate. Disable with the `auto_reflect` config (env
  `CATALYST_CODE_AUTO_REFLECT=0`). This closes the asymmetry noted in §5:
  memories already had a deterministic compaction hook (`extract_facts`);
  skills now have a deterministic reflection hook.

### Recurrence signal (makes the "seen ≥2×" rule evaluable)

The "write a skill when you solve the same shape ≥2×" rule was previously
un-trackable — the model has no cross-session recurrence counter. The
auto-reflect seam records one entry per non-trivial turn to a per-workspace log
`~/.config/catalyst-code/patterns/<hash>.jsonl` (capped at 200 entries). The
"shape" is a signature of the action tools used (bash/edit/write/patch/bulk_*/
todo_write/spawn/subagent — recon tools excluded) plus the file areas touched
(`core/src/*.rs`, `tui/*.go`, …). On each reflect, recurring shapes (count ≥ 2)
are read back and named in the nudge, so the model can decide whether to write
a skill. `core/src/pattern_log.rs`.

`/reflect` is the explicit, user-triggered version of the same pass.

---

## 12. Benchmarking

Raw signal is already emitted — no new benchmarking infra needed to *start*:

| Metric                | Source                                  |
|-----------------------|-----------------------------------------|
| Task completion       | `finish` tool call / turn outcome        |
| Token usage           | `metrics` event (tokens_in/out, cached)  |
| Latency / TTFT / TPS  | `metrics` event                          |
| Bugs introduced       | (deferred — needs a test-run signal)     |
| Test pass rate        | (deferred — parse `bash` test output)    |
| Code reuse            | (deferred — skill-utilization counter)   |
| Skill utilization     | (deferred — count `read_file` on SKILL.md)|
| Human corrections     | (deferred — count `/undo` + re-prompts)  |
| Retrieval accuracy    | N/A (recall=100% by construction)        |

The JSONL debug log (`turn_done`, `tool`, …) is the durable record. **Upgrade
path:** a `session_stop` telemetry plugin that aggregates these into a
`telemetry/` summary is the clean place to add derived metrics without touching
core. Deferred until a measurement goal is stated.

---

## 13. Safety & governance

Prevent the AI from degrading itself, without strangling it:

| Mechanism        | Status / Decision                                                |
|------------------|------------------------------------------------------------------|
| Approval gate    | **Exists.** `write_file`/`edit`/`bash` gated under `destructive` |
|                  | mode. Creating a skill (a `write_file`) needs consent.           |
| Rollback         | **Exists.** `/undo` drops a turn; `git` is the durable rollback.|
| Audit log        | **Exists.** JSONL debug log + per-workspace session JSONL.       |
| Immutable history| Session files are append-only JSONL (crash-safe, never rewritten  |
|                  | except compaction/refresh).                                       |
| Signed skills    | **Deferred.** Skills are advisory prompts reviewed as `git`       |
|                  | diffs; signing adds a key-management burden with no threat model  |
|                  | yet. Add when skills become *executable* (plugins). Plugins can   |
|                  | already be enabled/disabled per-name.                            |
| Trust levels     | Two tiers by construction: **advisory** (skills = prompts, low    |
|                  | blast radius) vs **executable** (plugins = code, already gated). |
| Sandbox execution| **Exists.** `--sandbox firejail` + `--no-network` for bash.       |
| Canary / staged  | **Deferred** for skills (advisory → no blast radius). The plugin   |
| rollouts         | enable/disable toggle is the staged-rollout mechanism for code.  |

The key safety property: **advisory skills cannot break the system.** A bad
skill makes the model worse at one task; `git revert` fixes it. This is why the
heavy governance (signing, canary, staging) is deferred until skills graduate
to executable form, where blast radius is real.

---

## 14. Multi-agent learning

The harness already has subagents + intercom. Multi-agent learning:

| Concern                    | Decision                                                        |
|----------------------------|-----------------------------------------------------------------|
| Share skills               | Skills are files in `.catalyst-code/skills/` — shared by       |
|                            | construction (all subagents in the same workspace read them).   |
| Synchronize knowledge      | Memories are per-workspace files; subagents writing to the     |
|                            | same workspace share them. (Subagents don't currently carry the  |
|                            | `memory` tool — `/index`/`/reflect` route through the           |
|                            | orchestrator. See §15 deferred item.)                           |
| Avoid conflicts            | The orchestrator owns memory writes in v1; subagents return      |
|                            | text and the orchestrator persists. Single-writer = no          |
|                            | write conflicts.                                                |
| Competing implementations  | The orchestrator (with intercom `ask`) resolves; last-writer    |
|                            | wins per memory name.                                           |
| Specialization             | Subagent definitions (`scout`, `reviewer`, `worker`, …) ARE the |
|                            | specialization mechanism.                                       |
| Federate across repos      | **Deferred.** Cross-repo federation needs a sync protocol +     |
|                            | conflict model. YAGNI until multiple repos actually share.     |

---

## 15. `/index` bootstrap command

`/index [--full|--incremental]` is implemented as a **pure delegation** slash
command: it sends a prompt that instructs the orchestrator to analyze the repo
and persist knowledge, using tools it already has (`read_file`/`grep`/`glob`/
`bash` + the `memory` tool). **No core command, no new indexing engine, no
embeddings, no graph DB.**

### What it produces

- Knowledge memories (types: architecture / convention / api / gotcha / build).
- Candidate skills under `.catalyst-code/skills/` for workflows seen 2+ times.
- A closing summary: memories created + the area the model is least confident
  about (the "human review" hint).

### `--full` vs `--incremental`

- `--full`: walk the top-level layout, README, manifests, entry points, config,
  tests; persist fresh memories + candidate skills.
- `--incremental`: use `git status` + `git diff --name-only` to find changed
  files; `append` updates to existing memories (architecture/conventions/…)
  rather than creating duplicates; save new memories only for genuinely new
  subsystems.

### Why delegation instead of a native indexer

A native indexer (AST parse → dependency graph → embedding index → API catalog)
is a second codebase to maintain, and it produces a *frozen* snapshot that's
stale on the next commit. Delegating to the model reuses its native
code-comprehension, produces *human-readable* memories (not a opaque index),
and self-updates via `--incremental`. The model is the indexer.

**Ceiling:** model-based indexing costs tokens and isn't deterministic.
**Upgrade path:** if determinism matters (e.g. CI gates), add a deterministic
plugin that emits a structured manifest; the model still writes the prose
memories from it.

---

## Data models

### Memory (file on disk)

```yaml
# ~/.config/catalyst-code/memory/<hash>/conventions.md
---
name: conventions           # required; slugified into filename
description: <one line>     # shown in the injection + index
type: convention            # note | convention | decision | architecture |
                            # api | gotcha | build | user | project
---
<body — prose facts, "--- appended ---" separators on accumulate>
```

Rust: `MemoryEntry { name, description, mem_type, content, path }` (`memory.rs`).

### Skill (file in repo)

```yaml
# .catalyst-code/skills/<name>/SKILL.md
---
name: <slug>                # required
description: <one line>     # used for discovery
version: 1                  # bump on behavior change
triggers: [add endpoint, new route]   # optional, advisory
---
## When to use
## Steps
## Example
```

### Subagent (file in repo — already exists)

```yaml
# .catalyst-code/agents/<name>.md
---
name: context-builder
description: <…>
tools: [read_file, grep, glob, list_dir, bash, write_file, intercom]
thinking: low
systemPromptMode: replace
inheritProjectContext: true
---
<body>
```

### Memory tool call (AI → core)

```json
{"action": "save|append|list|forget",
 "name": "...", "content": "...", "type": "...", "description": "...", "id": "..."}
```

---

## Versioning strategy

- **Memories**: last-write-wins per name; `append` accumulates with a rolling
  cap; `git` (for in-repo) or file mtime (for the notebook) is the history.
- **Skills**: frontmatter `version` integer; `git` is the audit log.
- **Schema (tool/protocol)**: the `Command`/`Event` JSON wire types are
  append-only (`#[serde(default)]` on optional fields) so old TUI/core pairs
  stay compatible across minor versions.

---

## Scalability considerations

| Axis                | v1 ceiling              | Upgrade trigger               | Upgrade path                          |
|---------------------|-------------------------|-------------------------------|---------------------------------------|
| Memory count        | ~500 (prompt bloat)     | synonym misses / token cost   | per-turn embedding retrieval into a  |
|                     |                         |                               | separate (non-prefix-cached) message  |
| Skill count         | ~50 (list_dir cost)    | discovery friction            | skill manifest in system prompt       |
| Repo size           | unbounded (model reads  | read cost on `/index --full`  | deterministic plugin manifest +       |
|                     | on demand)             |                               | model-prose memories from it         |
| Concurrency         | orchestrator is single- | throughput on parallel        | per-memory-name locks + let          |
|                     | writer for memory      | `/index` across agents        | subagents carry the memory tool      |
| Cross-repo          | none                   | multiple repos share patterns | federation/sync protocol              |

---

## Trade-off analysis

| Decision              | Chose                       | Rejected               | Why                                                      |
|-----------------------|-----------------------------|------------------------|----------------------------------------------------------|
| Skill representation  | Markdown prompt fragment    | Code / workflow graph   | Model executes prompts natively; reviewable; no runtime  |
| Knowledge store       | Markdown files              | Vector/Graph DB         | Tens of memories; keyword recall=100%; no new deps      |
| Retrieval             | Standing-prompt injection   | Per-turn embedding     | Preserves prefix cache; recall=100% at small scale      |
| Reflection            | In-band prompt protocol     | Background daemon       | Explainable; no extra model calls; user-steerable        |
| Post-turn hook        | (none new; `session_stop`   | New `post_turn` hook    | `session_stop` already fires per-turn → redundant        |
|                       |  already serves)            |                        |                                                          |
| `/index`              | Delegation to model         | Native AST indexer       | Reuses model comprehension; produces readable memories   |
| Skill validation      | `git` review + approval     | Staging + benchmarking   | Advisory skills have low blast radius; gating strangles  |
| Memory in tool vs     | Both (tool for AI; slash    | —                       | AI learns autonomously; user has manual control         |
| slash command         | for user)                   |                        |                                                          |

---

## Implementation roadmap

### ✅ Milestone 0 — Make the existing system actually work (done)

- **Fix the injection bug**: `build_injection` now injects ALL memories when the
  prompt is empty (the standing-prompt case), instead of matching keywords
  against an empty string and injecting nothing. (`memory.rs`, +test)
- **Expose `append`** on the `memory` tool so the model accumulates facts
  instead of clobbering. (`tools.rs`, +test)
- **Self-learning protocol** in the system prompt: reflect before done,
  persist durable facts, write skills when a pattern repeats. (`main.rs`)
- **`/index` and `/reflect`** slash commands + palette entries. (`handlers.go`,
  `modal.go`)
- **Fix stale docs**: the `memory.rs` header claimed save/scan was "a staged
  feature not yet bound" — it was bound; corrected.

All core tests (206) and TUI tests pass; both build clean.

### ✅ Milestone 1 — Mid-session visibility (done)

- **Refresh after save/forget** (`main.rs::refresh_memory_injection`): a saved or
  forgotten memory is rebuilt into the standing system prompt and visible to
  the very next turn *within the same session*. It is a no-op when the prompt is
  unchanged, so it preserves the provider prefix cache (it does not churn on
  every save). Wired into both the `SaveMemory`/`ForgetMemory` commands and the
  `memory` AI tool.
- **`memory` on subagents** (`subagent.rs`): added `"memory"` to
  `all_tool_names()` (the read-only default) and to the explicit `tools:` of
  the scout / researcher / context-builder agents (both the `.md` override
  templates and the embedded builtin fallbacks), so recon/research subagents
  persist learnings directly instead of handing text back for the orchestrator
  to re-process. `memory` is classified `ReadOnly`, so it never trips the
  approval gate.
- **Concurrency** (`memory.rs`): a process-wide `WRITE_LOCK`
  (`std::sync::Mutex`) now serializes `save_memory` / `append_memory` /
  `forget_memory`. `append_memory` is a read-modify-write, so two parallel
  subagents appending to the same memory name could previously race and drop a
  fact; the lock makes that impossible. (`+test` spawning 8 threads.)

### ✅ Milestone 2 — Skill discovery (done)

- **Skill manifest in the system prompt** (`main.rs::skill_manifest_injection`):
  a one-line-per-skill list (name + description) discovered under
  `.catalyst-code/skills/*/SKILL.md` (project then user scope) is spliced into
  the orchestrator's standing prompt, so available opt-in skills are visible
  without a `list_dir` round-trip. It excludes `pi-subagents` (already injected
  in full) and dedups by name (project wins). It returns `""` when no opt-in
  skills exist, so a fresh install's prompt — and its prefix cache — is left
  untouched. It is gated to the orchestrator (`with_skill`) so subagent prompts
  stay lean. (`+test`)

### ✅ Milestone 3 — Measurement (done)

- **Telemetry plugin** (`.catalyst-code/plugins/telemetry/`): a `session_stop`
  lifecycle hook that aggregates per-turn metrics into a per-workspace summary
  under `~/.config/catalyst-code/telemetry/<workspace-hash>/`:
  `turns.jsonl` (one record per turn), `summary.json` (incremental aggregates —
  totals, cache-hit rate, avg/min/max TTFT, avg TPS, per-model breakdown), and
  `summary.md` (human-readable). It is robust (always exits 0, emits one JSON
  line, skips cleanly on bad/empty/malformed input and null TTFT/TPS) and writes
  atomically. It captures the signals the design marks available (token trends,
  latency, throughput, cache effectiveness, per-model); skill-utilization and
  `/undo` correction rate remain deferred (§12) — they need the JSONL debug log
  or session-file parsing.
- **Small core enhancement** (`main.rs`): the design said "no core changes —
  pure plugin", but the JSONL debug log it assumed as the source is **off by
  default** (`debug_log: None`), so a pure plugin could not capture token/TTFT/TPS
  out-of-the-box. The minimal fix exposes the already-computed metrics through
  the existing hook seam: `State.last_turn_metrics` is set at turn finalization
  (and cleared at turn entry so a panicking turn can't leak the prior turn's
  numbers), and `dispatch_lifecycle` attaches `{session, turn}` metrics to the
  `session_stop` context (consumed when a plugin sets `pass_args: true`). No
  new infrastructure — just routing existing numbers to the existing hook.
- **Global staging** (`staging.rs`): the plugin is staged into
  `~/.catalyst-code/plugins/telemetry/` on first run (added to `bundled_files()`
  + `executable_rel_paths()`; `STAGING_VERSION` bumped 1 → 2 so existing users
  backfill it). Staging is non-clobbering, so users who already have a staged
  `agents/scout.md` (etc.) keep their copy; to pick up the memory-enabled agent
  defaults globally, delete the file and it is restored on next run:
  `rm ~/.catalyst-code/agents/{scout,researcher,context-builder}.md`. Project
  copies (`.catalyst-code/agents/*.md`) always win, so this repo gets them
  immediately.

All core tests (221) and TUI tests pass; core is `cargo fmt`/`clippy` clean
(only pre-existing `config.rs` lints remain); both build clean.

### Milestone 4 — Retrieval at scale (only if triggered)

- Per-turn embedding retrieval into a separate system message when memory
  count exceeds ~500 or synonym misses are observed. Keeps the stable prefix
  cached. This is the largest deferred change and is gated on a real signal.

### Milestone 5 — Executable skills (only if needed)

- If a skill needs deterministic side effects, graduate it to a **plugin**
  (executable hook), which already has enable/disable gating. At that point
  add signing + staged rollout for *that tier* only.

---

## Long-term vision

- **1 week**: The fixed injection bug + append + self-learning prompt means the
  model now actually remembers across sessions. `/index` lets it bootstrap on an
  unfamiliar repo in one command. A developer notices the agent "already knows"
  their conventions by day 3.
- **1 month**: A small skill library accrues under `.catalyst-code/skills/`,
  reviewed via `git`. The agent reuses them instead of re-deriving. Memories
  stabilize around architecture/conventions/gotchas.
- **6 months**: Telemetry (Milestone 3) shows measurable trends — fewer
  corrections, rising skill utilization. The skill manifest (Milestone 2) makes
  discovery frictionless. The first executable skills (plugins) appear for
  high-frequency deterministic workflows.
- **1 year**: Retrieval-at-scale (Milestone 4) lands if memory count warrants
  it. The system is a genuine compounder: each task leaves durable artifacts
  (memories + skills) that make the next task cheaper. Human intervention is
  mostly review, not steering.
- **5 years**: The boundary between "agent" and "platform" blurs — the
  skill/plugin library *is* the engineering knowledge base, federated across
  repos. The system contributes net-new capabilities (novel skills, refactor
  strategies) rather than just applying existing ones. The deferred heavy
  machinery (federation, signed executables, semantic retrieval) lands *only as
  measured needs justify it*, never speculatively.

The trajectory is deliberate: start with the smallest loop that compounds
(fix the bug → teach reflection → persist → reuse), and add each layer of
sophistication only when a real ceiling is hit. The 5-year system is not a
different architecture — it is this architecture with its deferred upgrades
unlocked in order.
