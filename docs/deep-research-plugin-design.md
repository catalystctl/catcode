# Deep Research Plugin — Design

Status: implemented. This document records the architecture and which upstream
deep-research concepts CatCode's native plugin adopts, modifies, or rejects.

## Goal

`/deep-research <request>` starts a real CatCode agent turn that conducts
extensive, citation-backed research and produces a structured report — reusing
CatCode's existing model, `web_search`, `fetch`, browser, and `subagent`
capabilities. No external agent framework, daemon, or extra language runtime.

## The enabling core change: prompt-backed commands

Before this work, plugin slash commands were **script-only**: a `commands` entry
declared a `script` that the harness spawned; its stdout (`{ok, output}`) was
*displayed* to the user. There was no way for a command to submit a prompt into
the normal agent loop.

This plugin required the opposite: a command that renders a template and submits
it as a normal agent turn (the same path as a user `send`). Rather than hardcode
Deep Research into core, a small, generic, backward-compatible **prompt-backed
command** feature was added:

- `CommandManifestEntry` now accepts `script` **or** `prompt_file` (mutually
  exclusive) plus an optional `mode` (`"script"` | `"agent_turn"`, inferred).
- `CommandConfig` carries a `CommandMode` (`Script` | `AgentTurn`), an optional
  `script`, and an optional `prompt_file` (resolved to an absolute, path-confined
  path at load time).
- `validate_prompt_file` confines the file to the plugin directory (rejects
  absolute paths and `..`), requires it to be a regular file, and enforces a
  256 KiB size limit (`MAX_PROMPT_COMMAND_BYTES`).
- `render_prompt_command` reads the file and substitutes `{{args}}`,
  `{{workspace}}`, `{{session_id}}`, `{{timestamp}}`, `{{plugin}}`, `{{command}}`
  (whitespace-tolerant; unknown tokens left verbatim).
- The dispatcher's `Command::PluginCommand` branches on mode: `Script` keeps the
  existing behavior; `AgentTurn` renders the template, emits a concise info event
  showing the original command (not the full prompt), resolves the model
  (last-used → default → first available) and effort (`"medium"`), and calls
  `start_turn` — the same function `send` uses. This preserves cancellation,
  session persistence, approval, tool use, model selection, compaction, and
  telemetry.
- `command_definitions()` now exposes `mode` so TUI/web discovery can distinguish
  the two kinds (backward-compatible: an added JSON field).

**Display decision:** the conversation is both the model input and the persisted
transcript, with no clean display/actual split. Per the task's explicit guidance,
the implementation uses the *visible prompt content* fallback (the rendered
protocol becomes the user message via `start_turn`) rather than adding fragile
hidden state. The dispatcher emits a concise `info` event with the original
command so the transcript stays readable; the rendered protocol is a coherent
directive, not raw scaffolding.

**Capability inference refinement:** a prompt-backed command executes no
subprocess, so `uses_subprocess` now flags only script-backed commands (not
prompt-backed ones). This keeps the capability set honest.

## Architecture

```
/deep-research <request>
        │  (TUI/web sends plugin_command)
        ▼
dispatcher: Command::PluginCommand
        │  mode == AgentTurn
        ▼
render_prompt_command(prompts/deep-research.md, {args,workspace,...})
        │  rendered research protocol
        ▼
start_turn(model, rendered, effort)   ← same path as user `send`
        ▼
┌──────────────────────── Supervisor agent (this turn) ────────────────────────┐
│ 1. Scope → brief.json                                                         │
│ 2. Perspectives + task graph → plan.json                                      │
│ 3. subagent(tasks=[branch…], concurrency=N)  ──► parallel researcher workers │
│       each worker: web_search → fetch → extract → compress                   │
│ 4. Evidence ledger → evidence.jsonl / sources.json                            │
│ 5. Source-quality ranking                                                     │
│ 6. Gap analysis → coverage.json → follow-up workers                          │
│ 7. Stopping rules                                                             │
│ 8. Citation verification (reviewer subagent)                                  │
│ 9. Central final synthesis → report.md + run.json                            │
└───────────────────────────────────────────────────────────────────────────────┘
```

The orchestration lives entirely in `prompts/deep-research.md` — **not** in the
global system prompt. Normal coding turns pay no token cost for the research
protocol; it is injected only when `/deep-research` is invoked.

## Upstream concepts

### Open Deep Research (langchain-ai/open_deep_research)

- **Adopted:** research scoping before browsing; a focused research brief as the
  source of truth; a supervisor that dynamically chooses subtopics; parallel
  researchers with isolated context; iterative gap analysis; compression of worker
  findings before returning to the supervisor; parallelism only during research;
  centralized final report writing after all research completes.
- **Rejected:** having parallel workers independently write final report sections
  (produces repetitive, disconnected reports). The supervisor writes the report.

### STORM (stanford-oval/storm)

- **Adopted:** perspective-guided question generation (stakeholders, technical,
  historical, critics, supporters, affected groups); follow-up questions based on
  discoveries rather than a fixed initial list; an outline produced from gathered
  evidence rather than model assumptions.
- **Modified:** perspectives are filtered to those relevant to the request (STORM
  tends to generate many); the "outline" is an evidence-driven report structure
  that adapts to the topic rather than a fixed article skeleton.

### GPT Researcher (assafelovic/gpt-researcher)

- **Adopted:** planner/researcher separation; recursive research with configurable
  breadth and depth (`--depth`, `--max-iterations`); source deduplication;
  workspace and web research modes (`--source`); structured reports and durable
  artifacts; explicit limits for concurrency, sources, iterations, time, and
  context.
- **Modified:** the "planner" is the supervisor agent itself (no separate planner
  process); recursion is bounded by stopping rules, not a fixed depth tree.

### BrowseComp (OpenAI)

- **Adopted:** persistent browsing; creative query reformulation; multi-hop search
  paths; verification separate from discovery; the principle that a proposed answer
  may be easier to verify than to discover.
- **Reflected in:** the separate citation-verification pass (Phase 8) and the
  worker loop's "search for missing/contradictory evidence" step.

### DeepResearch Bench (quality dimensions)

Used as the quality bar the protocol targets: completeness, research depth,
instruction following, readability, citation accuracy, effective citation coverage,
source diversity, appropriate handling of uncertainty. These are encoded in the
stopping rules, the evidence-ledger schema, the verification pass, and the report
requirements.

### smolagents (huggingface)

- **Rejected as a dependency:** no Python agent framework is introduced. The useful
  idea — small, tool-calling agents with isolated context — is realized natively by
  CatCode's `subagent` tool (parallel `researcher` workers).

## Native reuse (no parallel implementation)

The plugin adds **orchestration behavior**, not duplicate capabilities:

| Capability | Reused CatCode primitive |
|---|---|
| Model + provider | active provider/model (resolved last-used → default → first) |
| Web discovery | `web_search` (Exa/Tavily/no-key chain) |
| Source reading | `fetch` (allowlist/no-network aware) |
| JS-rendered pages | browser tools (loaded via `load_tools`) |
| Parallel research | `subagent` (`tasks` + `concurrency`, `researcher` agent) |
| Workspace research | `read_file`/`grep`/`glob`/`bash`/git |
| Artifact persistence | `write_file`/`mkdir` under `.catalyst-code/research-runs/` |
| Cancellation | normal abort/escape + subagent cancellation |
| Concurrency bounds | subagent `concurrency` (never exceeds harness limits) |
| Network/approval | existing egress, approval, sandbox rules |

## Context and token management

Workers run in isolated subagent contexts and return **compressed evidence
packets** (source records + conclusions), not raw pages. The supervisor reads
file-backed evidence (`evidence.jsonl`) by record id rather than re-inlining the
full ledger. Failed searches are excluded unless material. This prevents context
collapse on large topics.

## Persistence and resumability

Each run is saved under `.catalyst-code/research-runs/<timestamp>-<slug>/` with
`request.json`, `brief.json`, `plan.json`, `coverage.json`, `evidence.jsonl`,
`sources.json`, `worker-results/`, `report.md`, and `run.json`. `--resume
<run-id>` reloads the state, skips already-fetched sources, continues incomplete
branches, and re-verifies citations. Atomic file updates (temp + rename) are used
where practical.

## Reliability and research safety

All retrieved content is treated as untrusted data. The protocol explicitly
instructs agents to ignore source text that attempts to change the objective,
override instructions, request secrets, cause tool calls, write/delete files,
install software, execute commands, contact external parties, or suppress
competing sources. Research mode is read-only except for its own artifacts. It
obeys network allowlists, no-network mode, tool approval, workspace confinement,
browser restrictions, and sandbox settings. Blocked sources are recorded as
unavailable, never as read.

## What is deliberately not done

- No LangChain/LangGraph/DSPy/Python framework/Node service/MCP.
- No separate model provider, search daemon, or Docker runtime requirement.
- No mandatory API key beyond what CatCode already requires (no-key search works).
- No browser-automation framework when `fetch` + native browser tools suffice.
- No permanent system-prompt bloat: the protocol lives in the plugin prompt file.
- No hardcoded Deep Research behavior in core — the prompt-command feature is
  generic and reusable for `/review-pr`, `/document-repo`, `/investigate-bug`,
  `/migration-plan`, etc.
