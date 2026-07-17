# Goal Mode

Goal mode is a **plan-then-deploy orchestration pattern** for splitting a large
task across multiple subagents. Instead of the agent implementing everything in
one long turn, the agent writes a structured plan and the harness dispatches the
plan's steps as parallel subagent runs — each focused, bounded, and independently
verifiable.

---

## How It Works

```
User enters goal           Agent plans            Harness deploys          Results synthesized
       │                       │                       │                        │
       ▼                       ▼                       ▼                        ▼
   ┌────────┐   ┌──────────┐   ┌───────────┐   ┌──────────┐   ┌──────────────┐   ┌────────┐
   │  Idle  │──▶│ Planning │──▶│PlanReady  │──▶│Deploying │──▶│ Synthesizing │──▶│  Done  │
   └────────┘   └──────────┘   │(optional) │   └──────────┘   └──────────────┘   └────────┘
                               └──────────┘         │                                 │
                                                     ▼                                 ▼
                                               ┌──────────┐                      ┌────────┐
                                               │ Running  │                      │ Failed │
                                               └──────────┘                      └────────┘
```

**Classic `/goal` (single-pass):** `Planning → PlanReady → Deploying → Running → Synthesizing → Done|Failed`.

**Control Center `/control` (CEO mode, `ceo_mode=true`):**
```
Planning → Reviewing → (revise→Planning, plan_revision-capped) | Deploying
  → Running → Verifying → (Certified→Done) | (Replanning→Planning, iteration-capped→Failed)
```
The parent model owns Planning / Reviewing / Verifying / Replanning turns (the CEO);
employee subagents run the deploy steps. No user prompting — `contact_supervisor`
auto-resolves during CEO-active phases.

**Source:** `core/src/goal.rs`, `GoalPhase` enum, module docstring.

### Phase Machine

| Phase | What Happens |
|-------|-------------|
| `Idle` | No active goal. |
| `Planning` | Parent turn writes a structured plan via `goal_write_plan` (exactly once). Classic `/goal` may ask clarifying questions; CEO mode must not prompt the user. |
| `Reviewing` | CEO only: parent self-reviews the plan before deploy. FAIL writes revise feedback and re-enters `Planning` (capped by `max_plan_revisions`). |
| `PlanReady` | The plan is valid. If `auto_deploy` is `true` (default), the harness moves directly to `Deploying`. If `false`, it waits for an `approve_goal_plan` command. Control Center always uses `auto_deploy=true`. |
| `Deploying` | The harness partitions plan steps into dependency waves and acquires concurrency slots. |
| `Running` | Steps execute as subagents, respecting `depends_on`, concurrency caps, and per-model semaphores. Steps with failed dependencies are automatically skipped. |
| `Synthesizing` | Classic single-pass wrap-up: parent summarizes step results for the user. |
| `Verifying` | CEO only: parent verifies deploy artifacts against the goal (reads goal-scoped summary). PASS → certify → `Done`. FAIL → `Replanning` (or `Failed` when iteration-capped). |
| `Replanning` | CEO only: transitional; re-enters `Planning` with remaining gaps / feedback. |
| `Blocked` | A step is blocked (e.g. waiting on user input). |
| `Done` | Goal completed successfully (CEO: optionally `certified=true`). |
| `Failed` | Goal failed (cancelled, iteration-capped, or wrap-up aborted). |

At any phase, `cancel_goal` (Abort in Control Center) aborts the loop.

**Source:** `GoalPhase`, `transition`, `fail_goal`, `clear_goal` in `core/src/goal.rs`.

---

## Starting a Goal

### `/goal` Slash Command

In the TUI or web UI, enter `/goal` to open the goal modal. Fields:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| **Goal** | text | required | The objective (≥4 characters) |
| **Model** | string | current model | Model for the planning turn and default model for deploy steps |
| **Concurrency** | number | 4 (from config) | Max concurrent subagent slots (1–32) |
| **Max tasks** | number | 8 (from config) | Max steps in the plan (1–64) |
| **Allowed models** | list | all | Models the deploy steps may use; the planner must pick from this list |
| **Allowed providers** | list | all | Providers the deploy steps may use |
| **Auto-deploy** | bool | true | Deploy immediately after a valid plan; when false, stop at `PlanReady` |
| **Planner model** | string | — | Advanced: pin the model for the planning agent role |
| **Worker model** | string | — | Advanced: pin the model for worker subagent runs |
| **Reviewer model** | string | — | Advanced: pin the model for reviewer subagent runs |
| **Model concurrency** | map | — | Advanced: per-model concurrency caps (e.g. `{"gpt-4o": 2}`) |

### `/control` — Control Center (CEO mode)

Web UI flyout (slash `/control`, sidebar **Control**, header bolt). Always sends:

```json
{
  "type": "start_goal",
  "goal": "…",
  "auto_deploy": true,
  "ceo_mode": true,
  "max_iterations": 3,
  "max_plan_revisions": 2,
  "model": "<user selected>"
}
```

Classic `/goal` leaves `ceo_mode` unset/false (single-pass synthesize → Done).
Abort sends `{ "type": "cancel_goal" }`. Only user control while a mission is
active is Abort — no approve/revise banners.

Extra `goal_state` fields in CEO mode: `ceo_mode`, `mode` (`"ceo"`|`"single_pass"`),
`iteration`, `max_iterations`, `plan_revision`, `max_plan_revisions`,
`review_verdict`, `verify_verdict`, `remaining_gaps`, `self_review_feedback`,
`certified`. Discrete events: `goal_iteration`, `goal_review_verdict`,
`goal_verify_verdict`, `goal_certified`.

### Protocol: `start_goal`

```json
{
  "type": "start_goal",
  "goal": "Add a health-check endpoint to the API server",
  "model": "gpt-4o",
  "concurrency": 4,
  "max_tasks": 8,
  "allowed_models": ["gpt-4o", "claude-3.5-sonnet"],
  "auto_deploy": true
}
```

**Source:** `StartGoal` (protocol.rs line 349), `new_goal` (goal.rs line 378),
`StartGoalArgs` (goal.rs line 321).

### Validation

- Goal text must be non-empty and ≥4 characters.
- `concurrency` must not exceed `max_tasks`.
- Concurrency is clamped to 1–32, max_tasks to 1–64.
- Role model overrides (`planner_model`, etc.) are validated against
  `allowed_models`. Invalid overrides are silently dropped.
- Per-model concurrency is clamped to 1..=global concurrency.

**Source:** `new_goal` (goal.rs line 378).

---

## Planning Phase

When a goal starts, the harness:

1. Cancels any prior active goal.
2. Constructs a `planning_prompt` with the goal text, constraints, scheduling
   profile, and parallelism guidance.
3. Spawns a **speculative scout** (readonly reconnaissance) that runs in the
   background during planning.
4. Starts a planning turn with the selected model.

### Planning Prompt

The planning prompt is built from `GoalMode` fields and includes:

- The goal text
- Concurrency and max_tasks limits
- Allowed models and providers
- Role model pins (Advanced section)
- Per-model concurrency caps
- Revision feedback (when re-entering planning via `revise_goal`)
- Execution profile guidance (`serial`, `parallel`, or `ultra_parallel`)
- Scheduling profile with parallelism heuristics

**Source:** `planning_prompt` (goal.rs line 979).

### `goal_write_plan` Tool

This is a **deferred tool** — only available during goal-mode planning. The
agent must call it **exactly once** with a complete plan.

**Tool schema:**

```json
{
  "name": "goal_write_plan",
  "description": "GOAL MODE ONLY. Submit the structured multi-subagent plan exactly once.",
  "parameters": {
    "type": "object",
    "properties": {
      "summary": { "type": "string", "description": "High-level plan summary" },
      "steps": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "id": { "type": "string" },
            "agent": { "type": "string", "enum": ["scout", "researcher", "planner", "worker", "reviewer", "context-builder", "oracle", "delegate"] },
            "title": { "type": "string" },
            "task": { "type": "string" },
            "model": { "type": "string" },
            "depends_on": { "type": "array", "items": { "type": "string" } },
            "parallel_group": { "type": "string" }
          },
          "required": ["agent", "task"]
        }
      },
      "risks": { "type": "array", "items": { "type": "string" } },
      "validation": { "type": "array", "items": { "type": "string" } }
    },
    "required": ["summary", "steps"]
  }
}
```

**Validation rules:**
- `summary` must be non-empty.
- `steps` must have at least one entry and not exceed `max_tasks`.
- Each step requires an `agent` (non-empty) and a `task` (non-empty).
- `id`s must be unique and not collide.
- `depends_on` must reference existing step IDs and must not form cycles.
- Step models are validated against `allowed_models`; invalid models are dropped.
- The plan is partitioned into dependency waves via topological sort.

**Source:** `apply_plan` (goal.rs line 435), `topo_waves` (goal.rs line 548),
`goal_write_plan` tool definition (tools.rs line 575).

---

## Approval Gate

When `auto_deploy` is `false`, or when you want to review the plan before
deployment:

| Command | Protocol | Action |
|---------|----------|--------|
| `/approve-goal-plan` | `approve_goal_plan` | Approve the plan and start deployment |
| `/revise-goal <feedback>` | `revise_goal { feedback, model }` | Re-enter planning with user feedback |

`revise_goal` re-enters `Planning` phase with the feedback injected into the
planning prompt. The agent receives context from the prior planning turn.

**Source:** `ApproveGoalPlan`, `ReviseGoal` (protocol.rs lines 362, 367), command
handlers in main.rs lines 3695 and 3738.

---

## Deploy Phase

The harness:

1. Validates that a plan exists and phase is `PlanReady`.
2. Moves to `Deploying`.
3. Computes topological waves from step `depends_on`.
4. For each wave:
   - Skips steps whose dependencies failed.
   - Acquires **global concurrency** and **per-model** semaphore slots.
   - Marks steps `Running` only after slot acquisition (no flash state).
   - Spawns each step as a `subagent` call with the step's agent and task.
   - Uses **git worktree isolation** for mutating agents (`worker`, custom agents)
     when `concurrency > 1` and the workspace is a git repo.
   - Read-focused agents (`scout`, `researcher`, `reviewer`, etc.) share the
     main tree to avoid racing `git worktree add`.
5. After the final wave, runs plan **validation criteria** via a reviewer
   subagent, if any validation checks are specified.
6. Transitions to `Synthesizing`.

**Concurrency model:**
- **Global semaphore**: at most `concurrency` steps run simultaneously.
- **Per-model semaphore**: at most `model_concurrency[model]` steps run on the
  same model (defaults to global cap).
- Steps remain `Pending` in the queue until they acquire both slots.

**Worktree isolation:**

| Agent | Worktree? | Reason |
|-------|-----------|--------|
| `scout`, `researcher`, `planner`, `reviewer`, `context-builder`, `oracle` | No | Read-only or disjoint artifact paths |
| `worker` (and unknown/custom agents) | Yes | Edit shared files, need isolation |

**Source:** `deploy_goal` (goal.rs line 1196), `goal_step_needs_worktree` (goal.rs
line 298), `cap_concurrency` (goal.rs line 584), `filter_model_candidates`
(goal.rs line 563).

---

## Synthesizing Phase

After all waves complete:

1. The parent agent receives a `wrapup_prompt` containing step results (title,
   agent, status, summary) and artifact file paths.
2. The agent reads step artifacts under `.catalyst-code/goal-ux/artifacts/<goal-id>/`
   and writes a completion summary.
3. When the agent calls `finish`, the harness transitions to `Done`.
4. If the wrap-up is aborted or produces minimal output (<200 chars), the
   harness emits a deterministic completion summary as a fallback.

**Source:** `build_wrapup_prompt` (goal.rs line 1058), `finish_synthesis` (goal.rs
line 1136), `build_deterministic_completion_summary` (goal.rs line 842).

---

## Goal State Events

The harness emits events for UI rendering and protocol consumers:

| Event | When | Key Fields |
|-------|------|------------|
| `goal_state` | Any phase change or field update | `id`, `goal`, `phase`, `concurrency`, `prompts[]`, `active_run_ids`, `version`, `error` (+ CEO: `ceo_mode`, `mode`, `iteration`, `max_iterations`, `plan_revision`, `max_plan_revisions`, `review_verdict`, `verify_verdict`, `remaining_gaps`, `self_review_feedback`, `certified`) |
| `goal_phase` | Phase transition | `from`, `to`, `message`, `wave`, `step_count`, `done_count` |
| `goal_plan` | Plan is set | `id`, `summary`, `steps[]`, `risks[]`, `validation[]`, `version` |
| `goal_step_complete` | Each step finishes | `step_id`, `title`, `agent`, `ok`, `status`, `summary`, `run_id` |
| `goal_completion_summary` | Final summary | `text` (deterministic or model-written) |
| `goal_iteration` | CEO budget progress | `id`, `iteration`, `max_iterations`, `plan_revision`, `max_plan_revisions` |
| `goal_review_verdict` | CEO plan self-review | `id`, `ok`, `summary`, `iteration`, `plan_revision`, `evidence_paths` |
| `goal_verify_verdict` | CEO post-deploy verify | `id`, `ok`, `summary`, `iteration`, `remaining_gaps`, `evidence_paths` |
| `goal_certified` | CEO verify PASS | `id`, `summary`, `iteration`, `certified: true` |

**WorkState** (for TUI status bar) is synchronized from prompts after every
step completion and phase transition.

**Source:** `emit_goal_state` (goal.rs line 599), `emit_goal_phase` (line 612),
`emit_goal_plan` (line 602), `emit_goal_step_complete` (line 788), `emit_goal_completion_summary`
(line 830), `sync_work_state_from_prompts` (goal.rs line 1743).

---

## Advanced Mode: Role Model Pins

The Advanced section of `/goal` allows pinning models to specific agent roles:

| Role | Field | Effect |
|------|-------|--------|
| **Planner** | `planner_model` | Overrides the model used for the planning turn. Falls back to the selected model. |
| **Worker** | `worker_model` | Applied to every step whose `agent` is `"worker"` (or any agent that is not read-only). |
| **Reviewer** | `reviewer_model` | Applied to every step whose `agent` is `"reviewer"`. |

Resolution order: role override → step-level `model` → allowlist fallback.

If a role model pin is set, the planning prompt instructs the agent to omit
`step.model` for that role. This allows the harness to consistently route all
worker steps through a cheaper model without the planner needing to specify it
on every step.

**Per-model concurrency** (`model_concurrency`) caps how many runs can execute
simultaneously on a specific model, independent of the global concurrency cap.
This prevents a fast model from overwhelming a rate-limited provider while
slower models idle.

**Source:** `RoleModels` (goal.rs line 86), `resolve_step_model` (goal.rs line 287),
`StartGoal` protocol (protocol.rs line 349).

---

## Cancelling a Goal

| Command | Protocol | Effect |
|---------|----------|--------|
| `/cancel-goal` | `cancel_goal` | Aborts planning turn (if active), interrupts deploy waves, transitions to `Failed`. |

Cancellation is propagated via `CancellationToken` to all running subagent
steps. After cancellation, the harness does not enter `Synthesizing`.

**Source:** `CancelGoal` handler (main.rs line 3670), `fail_goal` (goal.rs line
908), `deploy_goal` cancellation checks (goal.rs lines 1287, 1316).

---

## Practical Workflow Example

### Scenario: Add health-check endpoint to an API server

**Step 1: Start the goal**

```
/goal
Goal: Add a /health endpoint to the API server that returns {"status":"ok"} with a 200 status
      and a database connectivity check.
Model: claude-3.5-sonnet
Concurrency: 3
Max tasks: 5
```

The harness starts a planning turn. A speculative scout runs in the background
to map the codebase.

**Step 2: Agent produces a plan**

The agent reads the project structure, then calls `goal_write_plan`:

```json
{
  "summary": "Add a /health endpoint with DB connectivity check to the Fastify API server",
  "steps": [
    {
      "id": "1",
      "agent": "scout",
      "title": "Map existing route structure and DB connection code",
      "task": "Read src/routes/ and find the route registration pattern. Find how the DB client is initialized and used. Write context.md with findings."
    },
    {
      "id": "2",
      "agent": "worker",
      "title": "Implement /health GET endpoint",
      "task": "Following the pattern in context.md, implement a GET /health route that returns {\"status\":\"ok\"} and checks DB connectivity.",
      "depends_on": ["1"]
    },
    {
      "id": "3",
      "agent": "worker",
      "title": "Add test for /health endpoint",
      "task": "Add a test file for the new /health endpoint covering success and DB-failure cases.",
      "depends_on": ["2"]
    },
    {
      "id": "4",
      "agent": "reviewer",
      "title": "Review implementation",
      "task": "Review the implementation against the plan. Check for error handling, test coverage, and alignment with existing patterns.",
      "depends_on": ["3"]
    }
  ],
  "risks": [
    "DB connectivity check may block if the test DB is unavailable"
  ],
  "validation": [
    "GET /health returns 200",
    "GET /health returns {\"status\":\"ok\"} when DB is up",
    "GET /health returns 503 when DB is down",
    "Tests pass"
  ]
}
```

**Step 3: Deploy**

- Wave 1: step 1 (scout) runs alone.
- Wave 2: step 2 (worker) runs, reads context.md from step 1.
- Wave 3: step 3 (worker) runs, adds tests.
- Wave 4: step 4 (reviewer) runs, validates the result.
- After wave 4: a reviewer subagent runs the validation criteria.

**Step 4: Results synthesized**

The parent agent reads step artifacts, summarizes what was built, and reports
any failed steps or validation issues.

**Step 5: Done**

The goal transitions to `Done` and the harness returns to normal interaction
mode.

---

## Related Commands

| Command | Protocol | Action |
|---------|----------|--------|
| `/goal` | `start_goal` | Start a classic single-pass goal |
| `/control` | `start_goal` (`ceo_mode=true`, `auto_deploy=true`) | Open Control Center / start autonomous CEO mission |
| `/cancel-goal` | `cancel_goal` | Abort active goal |
| `/approve-goal-plan` | `approve_goal_plan` | Approve plan when `auto_deploy=false` |
| `/revise-goal <feedback>` | `revise_goal` | Re-enter planning with feedback |
| `/goal-status` | `goal_status` | Re-emit current goal state and plan |
