---
name: pi-subagents
description: Delegate work to builtin or custom subagents with single-agent, chain, parallel, and intercom-coordinated workflows. Use for advisory review, implementation handoffs, recon, and multi-step tasks where one agent stays in control while others contribute context, planning, or execution.
---

# Subagents

You can delegate work to focused child agents via the `subagent` tool. A subagent is a nested agent run with its own system prompt and tool allowlist, sharing the same workspace and API key.

## Builtin agents

| Agent | Use when |
|-------|---------|
| `scout` | Fast codebase recon → compressed context for handoff |
| `researcher` | Web/docs research with sources and a brief |
| `planner` | Concrete implementation plan from context (reads, doesn't edit) |
| `worker` | Implementation; the single writer thread |
| `reviewer` | Adversarial code review with evidence |
| `context-builder` | Stronger setup pass; writes handoff material |
| `oracle` | Second opinion / drift check before acting (doesn't edit) |
| `delegate` | Lightweight general delegate |

## When to use

- **Advisory review**: fresh-context `reviewer` for adversarial review, or `oracle` (fork) when inherited decisions matter.
- **Implementation handoff**: `oracle` advises → `worker` implements only after approval.
- **Recon + plan**: `scout`/`context-builder` → `planner`.
- **Parallel exploration**: multiple non-conflicting tasks concurrently.

## Execution modes

```ts
// single
{ agent: "worker", task: "refactor auth" }
// forked context (branched from the parent conversation)
{ agent: "worker", task: "continue this thread", context: "fork" }
// parallel
{ tasks: [{ agent: "scout", task: "a" }, { agent: "reviewer", task: "b" }], concurrency: 2 }
// chain (sequential; {previous} = prior step output)
{ chain: [{ agent: "scout", task: "Gather context" }, { agent: "planner" }, { agent: "worker" }] }
```

## Coordination: intercom (key feature)

Subagents can prompt you (the orchestrator) for decisions, and talk to each other, when the setup allows it (`intercomBridge` not `off`; an agent's `tools` include `contact_supervisor`/`intercom`).

- `contact_supervisor({ reason: "need_decision", message })` — the subagent asks YOU a blocking question; you reply via the TUI prompt (or `intercom_reply`). Use this when a child is blocked on an unapproved decision.
- `contact_supervisor({ reason: "progress_update", message })` — non-blocking update.
- `intercom({ action: "ask", to, message })` — a subagent asks a peer subagent (parallel runs) and blocks for a reply.
- `intercom({ action: "send", to, message })` / `"receive"` / `"reply"` / `"targets"` — peer plumbing.

If a subagent asks you a question, answer it directly — it is waiting. Do not ignore `contact_supervisor` need_decision asks.

## Safety boundaries

- Children do not run subagents unless their `tools` include `subagent` (delegated fanout) and the depth cap allows it.
- Children must not invent intercom targets; use `action:"targets"` to discover peers.
- Escalate unapproved decisions instead of guessing.

## Management & control

```ts
{ action: "list" }                       // discover agents
{ action: "get", agent: "scout" }        // inspect an agent
{ action: "create", config: { name, systemPrompt, ... } }
{ action: "status" } / { action: "status", id: "run-1" }
{ action: "interrupt", id: "run-1" }
{ action: "resume", id: "run-1", message: "follow-up" }
{ action: "peek", id: "run-1" }          // watch: inspect conversation state
{ action: "steer", id: "run-1", message: "..." }  // bump direction mid-run
{ action: "doctor" }                      // setup diagnostics
```

## Watching & steering running subagents

You can inspect and intervene in a running subagent without interrupting it:

- **`peek`** — returns the subagent's current conversation state: message
  count, estimated tokens, the last 3 assistant turns, the last 3 tool
  results, and any pending intercom asks. Use this to watch what a subagent
  is doing, diagnose when it seems stuck, or decide whether to intervene.
  Works on both running and completed runs (completed runs retain their
  final conversation snapshot).

- **`steer`** — injects a directional message into the subagent's
  conversation. The message is appended as a user message
  ("Orchestrator: …") and the subagent picks it up on its next turn —
  between model calls, not mid-stream. Use this to:
  - **Bump direction** when the subagent is going down the wrong path
  - **Fix** a misunderstanding ("actually, use the Postgres driver, not MySQL")
  - **Unstick** by adding a hint or constraint the subagent is missing
  - **Refine scope** mid-flight ("also handle the error case in that function")

  Steer is non-destructive: it does not cancel the current turn. If the
  subagent is mid-model-call, the steer message waits in its mailbox and is
  picked up when the call completes. To stop a subagent entirely, use
  `interrupt`.

**Pattern — monitor and course-correct:**
```
1. Dispatch: { agent: "worker", task: "refactor the auth module", async: true }
2. Check in:  { action: "peek", id: "run-1" }
3. If off-track: { action: "steer", id: "run-1", message: "Focus on the JWT validation first, leave the session store for later." }
4. Check again: { action: "peek", id: "run-1" }
5. If recovered: let it continue. If still wrong: { action: "interrupt", id: "run-1" }
```

## Patterns

- **Implement then review**: implement → run fresh-context reviewers → synthesize fixes.
- **Review loop**: worker → reviewer → fix-worker, until clean (cap rounds).
- **Scout before planning**: `scout` → `planner` for unfamiliar code.

Prefer the `subagent` tool for delegation. Keep children focused: give each a concrete task and only the context it needs.
