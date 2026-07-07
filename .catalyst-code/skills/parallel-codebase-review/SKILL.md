---
name: parallel-codebase-review
description: Fan out many parallel reviewer subagents to comprehensively review a whole codebase, then synthesize findings
---

# Parallel Whole-Codebase Review

Use when asked to "review the whole codebase" / "audit everything" / find bugs across all components, and you want maximum parallelism via subagents.

## When to use
- Full-codebase review/audit request.
- You want N focused reviewers (e.g. one per module) rather than one giant review.

## When NOT to use
- A targeted review of one file/feature — just read it and review directly.
- Fewer than ~3 review units — single `reviewer` subagent suffices.

## Steps

1. **Map structure + sizes first.** `list_dir` each component, then `wc -l` the files sorted descending. Split the largest component (often Rust core) into multiple reviewers by file/group so no single reviewer drowns. Rule of thumb: ≤1.5k LOC per reviewer for depth.

2. **The hard cap gotcha.** The `subagent` tool's parallel `tasks` mode rejects > `parallel_max_tasks` (default **8**) instantly with `"parallel has N tasks (max M)"` and registers NO runs. So **split reviewers into batches of ≤8**. (Single-mode has no cap but blocks one-at-a-time.) Within a batch, set `concurrency` = batch size to run all at once.

3. **Each task = fresh context, so be self-contained.** Give every reviewer: the exact files to read (with approx LOC), the focus areas, the output contract (`file:line` evidence + severity critical/high/medium/low + suggested fix), and "be thorough and evidence-based." Don't assume it inherited anything from your conversation.

4. **Pick the model per the request.** Pass `model: "<id>"` on each task (per-task model override is supported: `{agent, task, model?}`). Verify the model resolves first with ONE cheap single-mode test (`task: "Reply with the single word OK"`) — if it fails, the model likely isn't in the discovered-models cache; the failure is instant (0.0s, no runs).

5. **Dispatch batch 1 (≤8), await aggregated result**, then batch 2, etc. Each parallel batch returns concatenated `=== Parallel Task N (agent) ===` blocks.

6. **Synthesize, don't dump.** Dedupe related findings across reviewers, group by severity (not by reviewer), and rank. Keep full detail in a written report file (e.g. `REVIEW.md`); give the user a tight exec summary + pointer.

7. **Verify surprising Criticals before reporting.** Small models (e.g. `deepseek-v4-flash`) are accurate on spot-checks but line numbers drift. For any Critical claim that is surprising/high-stakes, re-read the cited lines yourself before asserting it. This converts "the reviewer said" into "verified."

8. **Save the reusable dispatch gotcha** to memory once (`parallel-subagent-cap`) — don't rediscover the 8-task limit next time.

## Example shape (this repo)
12 reviewers → 2 batches of 8 + 4, all `model: "deepseek-v4-flash"`:
- 6 Rust-core reviewers (main.rs / tools.rs / provider.rs / subagent+intercom / plugins+config / smaller modules)
- 3 Go-TUI reviewers (dispatch+lifecycle / modal+keybinds / rendering)
- 1 SDK, 1 web, 1 build/CI/Docker
Output → `REVIEW.md` with Critical/High/Medium/Low + a prioritized fix list; exec summary in chat.
