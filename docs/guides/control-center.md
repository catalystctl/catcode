# Control Center

Control Center is the **autonomous CEO / orchestrator** surface in the web UI.
You give it a mission prompt; the parent model (the CEO) plans, self-reviews,
deploys employee subagents, verifies evidence, and replans until the request is
certified complete — or until iteration budgets are exhausted.

It never prompts you mid-mission. The only user control while a mission is
active is **Abort** (`cancel_goal`).

Classic single-pass `/goal` is unchanged: leave `ceo_mode` unset/false and the
loop still ends at synthesize → Done. See [Goal Mode](goal-mode.md).

**Sources:** `core/src/goal.rs`, `core/src/goal_ceo.rs`, `core/src/protocol.rs`
(`StartGoal`), `web/src/components/control-center.tsx`.

---

## Opening Control Center

Control Center is a **web UI flyout panel** (not a separate App Router page):

| Entry | What happens |
|-------|----------------|
| Slash `/control` | Opens the Control Center panel (`web/src/lib/commands.ts` → `action: "control"`) |
| Sidebar **Control** / header bolt | Same panel (`ControlCenterPanel`) |
| Abort / `/cancel-goal` | Sends `{ "type": "cancel_goal" }` |

The panel shows: status pill (phase + certified), iteration / plan-revision
budget, plan steps with per-agent run status, review / verify verdicts,
remaining gaps, and **Abort mission**.

---

## Starting a mission

Submitting the panel always sends `start_goal` with CEO autonomy pinned:

```json
{
  "type": "start_goal",
  "goal": "…",
  "model": "<selected model>",
  "auto_deploy": true,
  "ceo_mode": true,
  "max_iterations": 3,
  "max_plan_revisions": 2,
  "concurrency": 4,
  "max_tasks": 8
}
```

Optional Advanced fields (`planner_model`, `worker_model`, `reviewer_model`,
allowlists) match classic `/goal`. Core defaults when `ceo_mode=true`:
`max_iterations=3`, `max_plan_revisions=2` (`DEFAULT_CEO_MAX_ITERATIONS` /
`DEFAULT_CEO_MAX_PLAN_REVISIONS` in `goal.rs`). Classic `/goal` keeps
`ceo_mode=false` and both caps at `0` unless explicitly set.

---

## Mission lifecycle (CEO loop)

```
Planning → Reviewing → (revise → Planning, plan_revision-capped) | Deploying
  → Running → Verifying → (Certified → Done)
                       └→ Replanning → Planning  (iteration-capped → Failed)
```

| Phase | Who | Behavior |
|-------|-----|----------|
| `Planning` | Parent (CEO) | Writes a structured plan via `goal_write_plan` once. CEO prompts forbid `ask` / user questions. |
| `Reviewing` | Parent (CEO) | Self-reviews the plan. FAIL → revise feedback + re-enter `Planning` while `plan_revision < max_plan_revisions`. PASS → deploy. |
| `PlanReady` | — | Brief; Control Center always uses `auto_deploy=true`, so it does not wait on `approve_goal_plan`. |
| `Deploying` / `Running` | Employees | Waves of scout / worker / reviewer / … subagents under concurrency caps. Failed deps skip transitively; the loop does not fail-fast the whole mission. |
| `Verifying` | Parent (CEO) | Reads the goal-scoped summary (not 400-char wrap-up truncation). PASS → `certified` + `Done`. FAIL → `Replanning` if `iteration < max_iterations`, else `Failed`. |
| `Replanning` | Parent (CEO) | Transitional; re-enters `Planning` with `remaining_gaps` / revise feedback. |
| `Done` | — | Success; `certified=true` when verify certified. |
| `Failed` | — | Cancelled, plan-revision capped before deploy, or verify→replan iteration capped. |

`cancel_goal` (Abort) aborts from **any** active phase: cancels the deploy
token / in-flight turn and marks the goal failed.

---

## Autonomy guarantee

1. **No mid-mission user prompts.** CEO planning / review / verify / replan
   templates instruct the model not to call `ask` or wait on the user.
2. **`contact_supervisor` auto-resolves** when `GoalPhase::auto_resolves_supervisor()`
   is true — phases: `Planning`, `Reviewing`, `Deploying`, `Running`,
   `Verifying`, `Replanning`. A `need_decision` call returns immediately with
   “proceed with best judgment…”, emits `intercom_message` with
   `auto_resolved: true`, and must not re-ask.
3. **No approve / revise banners** in Control Center. Classic `/goal` may still
   use `auto_deploy=false` + `approve_goal_plan` / user `revise_goal`.
4. **Budgets end the loop.** Exhausted `max_plan_revisions` or `max_iterations`
   → `Failed` with an error message — not an indefinite ask loop.

---

## Evidence for verify

After deploy, the harness writes a goal-scoped summary:

`.catalyst-code/goal-ux/artifacts/<goal_id>/SUMMARY.md`

The CEO verify prompt (`build_verify_prompt` / `goal_ceo.rs`) requires reading
that file (and per-step artifacts under the same directory) so certification is
based on full step outputs, not the truncated synthesizing wrap-up used by
classic `/goal`.

Per-step evidence also lands at
`.catalyst-code/goal-ux/artifacts/<goal_id>/<step_id>.md`.

---

## Wire surface (CEO extras)

`goal_state` includes (among classic fields):

| Field | Meaning |
|-------|---------|
| `ceo_mode` | `true` for Control Center |
| `mode` | `"ceo"` or `"single_pass"` |
| `iteration` / `max_iterations` | Verify→replan budget |
| `plan_revision` / `max_plan_revisions` | Pre-deploy self-review budget |
| `review_verdict` / `verify_verdict` | `{ ok, summary, evidence_paths, at }` |
| `remaining_gaps` | Gaps from last failed verify |
| `self_review_feedback` | Last plan-review feedback |
| `certified` | Set when verify certifies |

Discrete events: `goal_iteration`, `goal_review_verdict`, `goal_verify_verdict`,
`goal_certified`. Full field lists: [Wire Protocol](../architecture/protocol.md#goal-events)
and [Goal Mode](goal-mode.md#events).

---

## UI checklist

The Control Center panel is expected to show:

- Phase status pill (including certified)
- Iteration and plan-revision counters
- Plan / step list with per-agent run status
- Review verdict and verify verdict (and remaining gaps)
- Abort → `cancel_goal`

It must **not** show plan-approval or user-revise flows for CEO missions.

---

## Related

- [Goal Mode](goal-mode.md) — classic `/goal` and shared phase/deploy mechanics
- [Slash commands](../commands/slash-commands.md#control) — `/control`, `/cancel-goal`
- [Wire protocol](../architecture/protocol.md) — `start_goal`, `cancel_goal`, goal events
- [Subagents](subagents.md) — employee agent roles
