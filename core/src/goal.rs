//! Goal mode: first-class plan-then-deploy orchestration.
//!
//! `/goal` (TUI/web) sends `start_goal`. The core owns a phase machine:
//! planning → plan_ready (optional) → deploying → running → done|failed.
//! The planning turn must call `goal_write_plan` with a structured plan;
//! deploy runs subagents under the user's concurrency and model/provider caps.

use crate::protocol::{emit, Event};
use crate::tools::Outcome;
use crate::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalPhase {
    Idle,
    Planning,
    PlanReady,
    Deploying,
    Running,
    Blocked,
    Done,
    Failed,
}

impl GoalPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalPhase::Idle => "idle",
            GoalPhase::Planning => "planning",
            GoalPhase::PlanReady => "plan_ready",
            GoalPhase::Deploying => "deploying",
            GoalPhase::Running => "running",
            GoalPhase::Blocked => "blocked",
            GoalPhase::Done => "done",
            GoalPhase::Failed => "failed",
        }
    }
}

impl Default for GoalPhase {
    fn default() -> Self {
        GoalPhase::Idle
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

impl DeployStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeployStatus::Pending => "pending",
            DeployStatus::Running => "running",
            DeployStatus::Done => "done",
            DeployStatus::Failed => "failed",
            DeployStatus::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoalStep {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub task: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub parallel_group: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoalPlan {
    pub summary: String,
    pub steps: Vec<GoalStep>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub validation: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeployPrompt {
    pub step_id: String,
    pub agent: String,
    pub task: String,
    #[serde(default)]
    pub model: Option<String>,
    pub status: DeployStatus,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub title: String,
}

/// Optional per-role model overrides from the Advanced section of `/goal`.
/// Empty string / None means "use step model or allowlist / parent default".
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoleModels {
    #[serde(default)]
    pub planner: Option<String>,
    #[serde(default)]
    pub worker: Option<String>,
    #[serde(default)]
    pub reviewer: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoalMode {
    pub id: String,
    pub goal: String,
    pub phase: GoalPhase,
    pub concurrency: u32,
    pub max_tasks: u32,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
    /// When true, deploy immediately after a valid plan. When false, stop at
    /// `plan_ready` until `approve_goal_plan`.
    pub auto_deploy: bool,
    /// Advanced: preferred model per agent role (planner / worker / reviewer).
    #[serde(default)]
    pub role_models: RoleModels,
    /// Advanced: max concurrent subagents per model id. Missing keys fall back
    /// to the global `concurrency` cap.
    #[serde(default)]
    pub model_concurrency: HashMap<String, u32>,
    pub plan: Option<GoalPlan>,
    pub prompts: Vec<DeployPrompt>,
    pub active_run_ids: Vec<String>,
    pub version: u64,
    pub error: Option<String>,
    /// Orchestrator model used for the planning turn / parent model for deploy.
    pub parent_model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    /// True when a plan was accepted and deploy should run after the planning turn.
    #[serde(default)]
    pub deploy_after_turn: bool,
    /// Optional revise feedback appended on the next planning turn.
    #[serde(default)]
    pub revise_feedback: Option<String>,
}

impl Default for GoalMode {
    fn default() -> Self {
        Self {
            id: String::new(),
            goal: String::new(),
            phase: GoalPhase::Idle,
            concurrency: 4,
            max_tasks: 8,
            allowed_models: Vec::new(),
            allowed_providers: Vec::new(),
            auto_deploy: true,
            role_models: RoleModels::default(),
            model_concurrency: HashMap::new(),
            plan: None,
            prompts: Vec::new(),
            active_run_ids: Vec::new(),
            version: 0,
            error: None,
            parent_model: String::new(),
            reasoning_effort: "medium".into(),
            deploy_after_turn: false,
            revise_feedback: None,
        }
    }
}

impl GoalMode {
    pub fn is_active(&self) -> bool {
        !matches!(
            self.phase,
            GoalPhase::Idle | GoalPhase::Done | GoalPhase::Failed
        )
    }

    pub fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    pub fn to_event_value(&self) -> Value {
        json!({
            "id": self.id,
            "goal": self.goal,
            "phase": self.phase.as_str(),
            "concurrency": self.concurrency,
            "max_tasks": self.max_tasks,
            "allowed_models": self.allowed_models,
            "allowed_providers": self.allowed_providers,
            "auto_deploy": self.auto_deploy,
            "role_models": {
                "planner": self.role_models.planner,
                "worker": self.role_models.worker,
                "reviewer": self.role_models.reviewer,
            },
            "model_concurrency": self.model_concurrency,
            "prompts": self.prompts.iter().map(|p| json!({
                "step_id": p.step_id,
                "agent": p.agent,
                "title": p.title,
                "task": p.task,
                "model": p.model,
                "status": p.status.as_str(),
                "run_id": p.run_id,
                "summary": p.summary,
            })).collect::<Vec<_>>(),
            "active_run_ids": self.active_run_ids,
            "version": self.version,
            "error": self.error,
            "parent_model": self.parent_model,
        })
    }

    /// Cap for a specific model: per-model override if set, else global concurrency.
    pub fn concurrency_for_model(&self, model: &str) -> u32 {
        self.model_concurrency
            .get(model)
            .copied()
            .unwrap_or(self.concurrency)
            .clamp(1, self.concurrency.max(1))
    }
}

/// Resolve which model a step should run with, given role overrides + allowlist.
pub fn resolve_step_model(mode: &GoalMode, agent: &str, step_model: Option<String>) -> Option<String> {
    let role = match agent {
        "planner" => mode.role_models.planner.clone(),
        "worker" => mode.role_models.worker.clone(),
        "reviewer" => mode.role_models.reviewer.clone(),
        _ => None,
    };
    // Role override wins when set (Advanced section is explicit).
    let candidate = role.or(step_model).or_else(|| mode.allowed_models.first().cloned());
    match candidate {
        Some(m) if !mode.allowed_models.is_empty() && !mode.allowed_models.iter().any(|a| a == &m) => {
            // Role/step model outside allowlist → fall back to first allowed.
            mode.allowed_models.first().cloned()
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Construction / validation
// ---------------------------------------------------------------------------

pub struct StartGoalArgs {
    pub goal: String,
    pub concurrency: Option<u32>,
    pub max_tasks: Option<u32>,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
    pub auto_deploy: Option<bool>,
    pub role_models: RoleModels,
    pub model_concurrency: HashMap<String, u32>,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub default_concurrency: u32,
    pub default_max_tasks: u32,
}

fn normalize_role_model(m: Option<String>, allowed: &[String]) -> Option<String> {
    let m = m.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())?;
    if !allowed.is_empty() && !allowed.iter().any(|a| a == &m) {
        return None; // drop invalid role model vs allowlist
    }
    Some(m)
}

pub fn new_goal(args: StartGoalArgs) -> Result<GoalMode, String> {
    let goal = args.goal.trim().to_string();
    if goal.is_empty() {
        return Err("goal text must not be empty".into());
    }
    if goal.chars().count() < 4 {
        return Err("goal text is too short".into());
    }
    let concurrency = args
        .concurrency
        .unwrap_or(args.default_concurrency)
        .clamp(1, 32);
    let max_tasks = args
        .max_tasks
        .unwrap_or(args.default_max_tasks)
        .clamp(1, 64);
    if concurrency > max_tasks {
        return Err(format!(
            "concurrency ({concurrency}) cannot exceed max_tasks ({max_tasks})"
        ));
    }
    let allowed_models = args.allowed_models;
    let role_models = RoleModels {
        planner: normalize_role_model(args.role_models.planner, &allowed_models),
        worker: normalize_role_model(args.role_models.worker, &allowed_models),
        reviewer: normalize_role_model(args.role_models.reviewer, &allowed_models),
    };
    // Clamp per-model concurrency to 1..=global concurrency.
    let mut model_concurrency: HashMap<String, u32> = HashMap::new();
    for (k, v) in args.model_concurrency {
        let key = k.trim().to_string();
        if key.is_empty() {
            continue;
        }
        if !allowed_models.is_empty() && !allowed_models.iter().any(|a| a == &key) {
            continue;
        }
        model_concurrency.insert(key, v.clamp(1, concurrency));
    }
    // Planning turn prefers planner role model when set.
    let parent_model = role_models
        .planner
        .clone()
        .unwrap_or(args.model);

    let id = format!("goal-{}", now_ms());
    Ok(GoalMode {
        id,
        goal,
        phase: GoalPhase::Planning,
        concurrency,
        max_tasks,
        allowed_models,
        allowed_providers: args.allowed_providers,
        auto_deploy: args.auto_deploy.unwrap_or(true),
        role_models,
        model_concurrency,
        plan: None,
        prompts: Vec::new(),
        active_run_ids: Vec::new(),
        version: 1,
        error: None,
        parent_model,
        reasoning_effort: args
            .reasoning_effort
            .unwrap_or_else(|| "medium".into()),
        deploy_after_turn: false,
        revise_feedback: None,
    })
}

/// Parse + validate a `goal_write_plan` payload into a GoalPlan and deploy prompts.
pub fn apply_plan(
    mode: &mut GoalMode,
    args: &Value,
    known_agents: &HashSet<String>,
) -> Result<(), String> {
    if mode.phase != GoalPhase::Planning {
        return Err(format!(
            "goal_write_plan only valid during planning (phase={})",
            mode.phase.as_str()
        ));
    }
    let summary = args
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if summary.is_empty() {
        return Err("goal_write_plan requires a non-empty 'summary'".into());
    }
    let steps_raw = args
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "goal_write_plan requires a 'steps' array".to_string())?;
    if steps_raw.is_empty() {
        return Err("goal_write_plan requires at least one step".into());
    }
    if steps_raw.len() as u32 > mode.max_tasks {
        return Err(format!(
            "plan has {} steps (max_tasks={})",
            steps_raw.len(),
            mode.max_tasks
        ));
    }

    let mut steps: Vec<GoalStep> = Vec::new();
    let mut ids: HashSet<String> = HashSet::new();
    for (i, s) in steps_raw.iter().enumerate() {
        let id = s
            .get("id")
            .and_then(|v| v.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| format!("{}", i + 1));
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate step id '{id}'"));
        }
        let agent = s
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if agent.is_empty() {
            return Err(format!("step '{id}' missing agent"));
        }
        if !known_agents.is_empty() && !known_agents.contains(&agent) {
            // Soft warning: still allow unknown custom agents the registry may
            // not have listed yet; only hard-fail empty.
        }
        let title = s
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let task = s
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if task.is_empty() {
            return Err(format!("step '{id}' missing task prompt"));
        }
        let step_model = s
            .get("model")
            .and_then(|v| v.as_str())
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        // Strip models outside the allowlist (empty allowlist = unrestricted).
        let step_model = match step_model {
            Some(m) if !mode.allowed_models.is_empty() && !mode.allowed_models.iter().any(|a| a == &m) => {
                None
            }
            other => other,
        };
        // Role models (Advanced) applied later when materializing DeployPrompt.
        let depends_on: Vec<String> = s
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let parallel_group = s
            .get("parallel_group")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        steps.push(GoalStep {
            id,
            agent,
            title: if title.is_empty() {
                format!("step {}", i + 1)
            } else {
                title
            },
            task,
            model: step_model,
            depends_on,
            parallel_group,
        });
    }

    // Validate depends_on references.
    for step in &steps {
        for dep in &step.depends_on {
            if !ids.contains(dep) {
                return Err(format!(
                    "step '{}' depends on unknown id '{}'",
                    step.id, dep
                ));
            }
            if dep == &step.id {
                return Err(format!("step '{}' cannot depend on itself", step.id));
            }
        }
    }
    // Cycle check via topo waves.
    if topo_waves(&steps).is_err() {
        return Err("step depends_on graph contains a cycle".into());
    }

    let risks: Vec<String> = args
        .get("risks")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let validation: Vec<String> = args
        .get("validation")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let plan = GoalPlan {
        summary,
        steps: steps.clone(),
        risks,
        validation,
    };

    let mut prompts = Vec::new();
    for step in &plan.steps {
        let full_task = format!(
            "# Goal\n{}\n\n# Step: {}\n{}\n\n# Validation criteria for the overall goal\n{}",
            mode.goal,
            step.title,
            step.task,
            if plan.validation.is_empty() {
                "(none specified)".into()
            } else {
                plan.validation
                    .iter()
                    .map(|v| format!("- {v}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        );
        prompts.push(DeployPrompt {
            step_id: step.id.clone(),
            agent: step.agent.clone(),
            task: full_task,
            model: resolve_step_model(mode, &step.agent, step.model.clone()),
            status: DeployStatus::Pending,
            run_id: None,
            summary: None,
            title: step.title.clone(),
        });
    }

    mode.plan = Some(plan);
    mode.prompts = prompts;
    mode.error = None;
    mode.touch();
    if mode.auto_deploy {
        mode.phase = GoalPhase::PlanReady;
        mode.deploy_after_turn = true;
    } else {
        mode.phase = GoalPhase::PlanReady;
        mode.deploy_after_turn = false;
    }
    Ok(())
}

/// Partition steps into waves respecting depends_on. Err on cycles.
pub fn topo_waves(steps: &[GoalStep]) -> Result<Vec<Vec<String>>, String> {
    let mut remaining: HashMap<String, HashSet<String>> = HashMap::new();
    for s in steps {
        remaining.insert(s.id.clone(), s.depends_on.iter().cloned().collect());
    }
    let mut done: HashSet<String> = HashSet::new();
    let mut waves: Vec<Vec<String>> = Vec::new();
    while done.len() < steps.len() {
        let mut wave: Vec<String> = remaining
            .iter()
            .filter(|(id, deps)| !done.contains(*id) && deps.iter().all(|d| done.contains(d)))
            .map(|(id, _)| id.clone())
            .collect();
        if wave.is_empty() {
            return Err("cycle".into());
        }
        wave.sort();
        for id in &wave {
            done.insert(id.clone());
        }
        waves.push(wave);
    }
    Ok(waves)
}

/// Filter a model candidate list by goal allowlists (models + providers via registry).
pub fn filter_model_candidates(
    candidates: &[String],
    mode: &GoalMode,
    model_providers: &HashMap<String, String>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|m| {
            if !mode.allowed_models.is_empty()
                && !mode.allowed_models.iter().any(|a| a == *m)
            {
                return false;
            }
            if !mode.allowed_providers.is_empty() {
                let prov = model_providers.get(*m).map(|s| s.as_str()).unwrap_or("");
                if prov.is_empty() {
                    // Unknown provider mapping: keep candidate (may still work on active).
                    return true;
                }
                if !mode
                    .allowed_providers
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(prov))
                {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// Cap parallel concurrency for subagent calls when goal mode is active.
pub fn cap_concurrency(requested: u32, mode: &GoalMode) -> u32 {
    requested.min(mode.concurrency).max(1)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub fn emit_goal_state(mode: &GoalMode) {
    let mut ev = Event::new("goal_state");
    if let Value::Object(map) = mode.to_event_value() {
        for (k, v) in map {
            ev = ev.with(&k, v);
        }
    }
    emit(&ev);
}

pub fn emit_goal_plan(mode: &GoalMode) {
    let Some(plan) = &mode.plan else {
        return;
    };
    emit(
        &Event::new("goal_plan")
            .with("id", json!(mode.id))
            .with("summary", json!(plan.summary))
            .with("steps", json!(plan.steps))
            .with("risks", json!(plan.risks))
            .with("validation", json!(plan.validation))
            .with("version", json!(mode.version)),
    );
}

pub fn emit_goal_phase(from: &GoalPhase, to: &GoalPhase, message: Option<&str>) {
    let mut ev = Event::new("goal_phase")
        .with("from", json!(from.as_str()))
        .with("to", json!(to.as_str()));
    if let Some(m) = message {
        ev = ev.with("message", json!(m));
    }
    emit(&ev);
}

pub fn transition(mode: &mut GoalMode, to: GoalPhase, message: Option<&str>) {
    let from = mode.phase.clone();
    if from == to {
        return;
    }
    mode.phase = to.clone();
    mode.touch();
    emit_goal_phase(&from, &to, message);
    emit_goal_state(mode);
}

pub fn fail_goal(mode: &mut GoalMode, err: impl Into<String>) {
    let msg = err.into();
    mode.error = Some(msg.clone());
    mode.deploy_after_turn = false;
    transition(mode, GoalPhase::Failed, Some(msg.as_str()));
}

pub fn clear_goal(mode: &mut GoalMode) {
    *mode = GoalMode::default();
    emit(
        &Event::new("goal_state")
            .with("id", json!(""))
            .with("goal", json!(""))
            .with("phase", json!("idle"))
            .with("concurrency", json!(0))
            .with("max_tasks", json!(0))
            .with("allowed_models", json!([]))
            .with("allowed_providers", json!([]))
            .with("auto_deploy", json!(true))
            .with("prompts", json!([]))
            .with("active_run_ids", json!([]))
            .with("version", json!(0))
            .with("error", Value::Null)
            .with("parent_model", json!("")),
    );
}

// ---------------------------------------------------------------------------
// Planning prompt
// ---------------------------------------------------------------------------

pub fn planning_prompt(mode: &GoalMode) -> String {
    let models = if mode.allowed_models.is_empty() {
        "(any available model)".to_string()
    } else {
        mode.allowed_models.join(", ")
    };
    let providers = if mode.allowed_providers.is_empty() {
        "(any logged-in provider)".to_string()
    } else {
        mode.allowed_providers.join(", ")
    };
    let role_line = {
        let mut parts = Vec::new();
        if let Some(m) = &mode.role_models.planner {
            parts.push(format!("planner→{m}"));
        }
        if let Some(m) = &mode.role_models.worker {
            parts.push(format!("worker→{m}"));
        }
        if let Some(m) = &mode.role_models.reviewer {
            parts.push(format!("reviewer→{m}"));
        }
        if parts.is_empty() {
            "Role models: (not pinned — omit step.model or pick from allowlist)".into()
        } else {
            format!(
                "Role models (harness will force these for matching agents; omit step.model for them): {}",
                parts.join(", ")
            )
        }
    };
    let model_conc = if mode.model_concurrency.is_empty() {
        format!("Per-model concurrency: (use global cap {c})", c = mode.concurrency)
    } else {
        let mut pairs: Vec<String> = mode
            .model_concurrency
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        pairs.sort();
        format!("Per-model concurrency: {}", pairs.join(", "))
    };
    let revise = mode
        .revise_feedback
        .as_ref()
        .map(|f| format!("\n\n## Revision feedback from the user\n{f}\n"))
        .unwrap_or_default();

    format!(
        r#"You are in GOAL MODE. Your only job this turn is to produce a structured deployment plan for subagents — do not implement the goal yourself.

## Goal
{goal}

## Constraints
- Max concurrent subagents at deploy time: {concurrency}
- Max steps/tasks: {max_tasks}
- Allowed models: {models}
- Allowed providers: {providers}
- {role_line}
- {model_conc}
{revise}
## Required action
1. Optionally use read/search tools briefly if you need repo context to plan well.
2. Call the `goal_write_plan` tool EXACTLY ONCE with a complete plan. Do not call it partially.

## Plan quality rules
- Prefer scout/context-builder before worker when the codebase area is unknown.
- Prefer independent steps that can run in parallel (same parallel_group, empty depends_on).
- Use depends_on when a step needs a prior step's output.
- Each step.task must be a self-contained prompt the subagent can execute without this chat.
- Use agents: scout, researcher, planner, worker, reviewer, context-builder, oracle, delegate (or custom agents).
- Prefer agents planner / worker / reviewer when they match the work so role model pins apply.
- Assign step.model only from the allowed models list (or omit to use role/default).
- Keep steps ≤ {max_tasks}.

After goal_write_plan succeeds, briefly confirm the plan in one short paragraph. Do not start implementing."#,
        goal = mode.goal,
        concurrency = mode.concurrency,
        max_tasks = mode.max_tasks,
        models = models,
        providers = providers,
        role_line = role_line,
        model_conc = model_conc,
        revise = revise,
    )
}

// ---------------------------------------------------------------------------
// Deploy (via subagent::execute parallel waves)
// ---------------------------------------------------------------------------

/// Run deploy for the current plan. Caller must hold no goal lock across await.
pub async fn deploy_goal(
    st: Arc<State>,
    client: reqwest::Client,
    cancel: CancellationToken,
) {
    // Snapshot config for deploy.
    let (parent_model, global_conc, model_conc_map, prompts_snapshot, goal_id) = {
        let mode = st.goal.lock().await;
        if mode.plan.is_none() || mode.prompts.is_empty() {
            return;
        }
        (
            mode.parent_model.clone(),
            mode.concurrency,
            mode.model_concurrency.clone(),
            mode.prompts.clone(),
            mode.id.clone(),
        )
    };

    {
        let mut mode = st.goal.lock().await;
        if mode.id != goal_id {
            return;
        }
        mode.deploy_after_turn = false;
        transition(&mut mode, GoalPhase::Deploying, Some("deploying plan steps"));
    }

    // Build step dependency map from plan.
    let steps: Vec<GoalStep> = {
        let mode = st.goal.lock().await;
        mode.plan
            .as_ref()
            .map(|p| p.steps.clone())
            .unwrap_or_default()
    };
    let waves = match topo_waves(&steps) {
        Ok(w) => w,
        Err(e) => {
            let mut mode = st.goal.lock().await;
            fail_goal(&mut mode, format!("deploy ordering failed: {e}"));
            return;
        }
    };

    let prompt_by_id: HashMap<String, DeployPrompt> = prompts_snapshot
        .into_iter()
        .map(|p| (p.step_id.clone(), p))
        .collect();

    // Global + per-model semaphores for concurrent deploy slots.
    use std::sync::Arc as StdArc;
    use tokio::sync::Semaphore;
    let global_sem = StdArc::new(Semaphore::new(global_conc.max(1) as usize));
    let mut model_sems: HashMap<String, StdArc<Semaphore>> = HashMap::new();
    for (m, cap) in &model_conc_map {
        let n = (*cap).clamp(1, global_conc.max(1)) as usize;
        model_sems.insert(m.clone(), StdArc::new(Semaphore::new(n)));
    }

    let mut any_failed = false;

    for wave in waves {
        if cancel.is_cancelled() {
            let mut mode = st.goal.lock().await;
            fail_goal(&mut mode, "goal cancelled");
            return;
        }

        // Mark wave running.
        {
            let mut mode = st.goal.lock().await;
            for id in &wave {
                if let Some(p) = mode.prompts.iter_mut().find(|p| &p.step_id == id) {
                    p.status = DeployStatus::Running;
                }
            }
            transition(&mut mode, GoalPhase::Running, Some("subagents running"));
            sync_work_state_from_prompts(&st, &mode).await;
        }

        // Collect wave tasks.
        let mut wave_prompts: Vec<DeployPrompt> = Vec::new();
        for id in &wave {
            if let Some(p) = prompt_by_id.get(id) {
                wave_prompts.push(p.clone());
            }
        }
        if wave_prompts.is_empty() {
            continue;
        }

        // Run each step with global + per-model concurrency caps.
        let mut handles = Vec::new();
        for p in wave_prompts {
            if cancel.is_cancelled() {
                break;
            }
            let model_key = p
                .model
                .clone()
                .unwrap_or_else(|| parent_model.clone());
            let g_sem = global_sem.clone();
            let m_sem = model_sems
                .entry(model_key.clone())
                .or_insert_with(|| StdArc::new(Semaphore::new(global_conc.max(1) as usize)))
                .clone();
            let st_c = st.clone();
            let client_c = client.clone();
            let cancel_c = cancel.clone();
            let parent = parent_model.clone();
            let step_id = p.step_id.clone();
            handles.push(tokio::spawn(async move {
                let _g = match g_sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        return (step_id, false, "global concurrency semaphore closed".into());
                    }
                };
                let _m = match m_sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        return (step_id, false, "model concurrency semaphore closed".into());
                    }
                };
                if cancel_c.is_cancelled() {
                    return (step_id, false, "cancelled".into());
                }
                let mut args = json!({
                    "agent": p.agent,
                    "task": p.task,
                    "context": "fresh",
                });
                if let Some(m) = &p.model {
                    args["model"] = json!(m);
                }
                let provider = st_c.resolve_provider_for_model(
                    p.model.as_deref().unwrap_or(&parent),
                ).await;
                let outcome = crate::subagent::execute(
                    st_c,
                    client_c,
                    provider,
                    parent,
                    args,
                    cancel_c,
                    0,
                )
                .await;
                (step_id, outcome.ok, outcome.output)
            }));
        }

        let mut wave_failed = false;
        for h in handles {
            match h.await {
                Ok((step_id, ok, output)) => {
                    let mut mode = st.goal.lock().await;
                    if let Some(p) = mode.prompts.iter_mut().find(|p| p.step_id == step_id) {
                        if ok {
                            p.status = DeployStatus::Done;
                            p.summary = Some(truncate_str(&output, 400));
                        } else {
                            p.status = DeployStatus::Failed;
                            p.summary = Some(truncate_str(&output, 400));
                            wave_failed = true;
                            any_failed = true;
                        }
                    }
                    mode.touch();
                    emit_goal_state(&mode);
                    sync_work_state_from_prompts(&st, &mode).await;
                }
                Err(e) => {
                    any_failed = true;
                    wave_failed = true;
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("goal deploy task join error: {e}"))),
                    );
                }
            }
        }

        if wave_failed {
            // Fail-fast on a wave failure (blocking dependency chain).
            break;
        }
    }

    {
        let mut mode = st.goal.lock().await;
        if mode.id != goal_id {
            return;
        }
        if any_failed || cancel.is_cancelled() {
            let msg = if cancel.is_cancelled() {
                "goal cancelled"
            } else {
                "one or more deploy steps failed"
            };
            fail_goal(&mut mode, msg);
        } else {
            transition(&mut mode, GoalPhase::Done, Some("goal complete"));
        }
        sync_work_state_from_prompts(&st, &mode).await;
    }

    emit(
        &Event::new("info").with(
            "message",
            json!(if any_failed {
                "Goal mode finished with failures — see goal_state prompts"
            } else {
                "Goal mode complete"
            }),
        ),
    );
}

/// Mirror deploy prompt statuses into WorkState done/in_progress/next.
pub async fn sync_work_state_from_prompts(st: &Arc<State>, mode: &GoalMode) {
    let mut done = Vec::new();
    let mut in_progress = Vec::new();
    let mut next = Vec::new();
    for p in &mode.prompts {
        let label = if p.title.is_empty() {
            format!("{} ({})", p.step_id, p.agent)
        } else {
            format!("{} — {}", p.title, p.agent)
        };
        match p.status {
            DeployStatus::Done => done.push(label),
            DeployStatus::Running => in_progress.push(label),
            DeployStatus::Failed => done.push(format!("FAILED: {label}")),
            DeployStatus::Skipped => {}
            DeployStatus::Pending => next.push(label),
        }
    }
    let mut ws = st.work_state.lock().await;
    if !mode.goal.is_empty() {
        ws.goal = truncate_str(&mode.goal, 240);
    }
    ws.done = done;
    ws.in_progress = in_progress;
    ws.next = next;
    ws.last_activity = format!("goal:{}", mode.phase.as_str());
    ws.touch();
    let version = ws.version;
    let goal = ws.goal.clone();
    let done = ws.done.clone();
    let in_progress = ws.in_progress.clone();
    let next = ws.next.clone();
    let recent_files = ws.recent_files.clone();
    let last_activity = ws.last_activity.clone();
    drop(ws);
    emit(
        &Event::new("work_state")
            .with("version", json!(version))
            .with("goal", json!(goal))
            .with("done", json!(done))
            .with("in_progress", json!(in_progress))
            .with("next", json!(next))
            .with("recent_files", json!(recent_files))
            .with("last_activity", json!(last_activity)),
    );
}

/// Handle goal_write_plan tool call against live GoalMode.
pub async fn handle_goal_write_plan(st: &Arc<State>, args: &Value) -> Outcome {
    let workspace = st.cfg.read().await.workspace.clone();
    let cfg = st.cfg.read().await.clone();
    let agents = crate::subagent::discover_agents(&workspace, &cfg.subagents);
    let known: HashSet<String> = agents.into_iter().map(|a| a.name).collect();

    let (msg, titles) = {
        let mut mode = st.goal.lock().await;
        if mode.phase != GoalPhase::Planning {
            return Outcome::err(format!(
                "goal_write_plan only valid during planning (phase={})",
                mode.phase.as_str()
            ));
        }
        match apply_plan(&mut mode, args, &known) {
            Ok(()) => {
                emit_goal_plan(&mode);
                emit_goal_state(&mode);
                let msg = if mode.auto_deploy {
                    format!(
                        "Plan accepted ({} steps). Deploy will start after this planning turn.",
                        mode.prompts.len()
                    )
                } else {
                    format!(
                        "Plan accepted ({} steps). Waiting for approve_goal_plan.",
                        mode.prompts.len()
                    )
                };
                let titles: Vec<String> = mode
                    .prompts
                    .iter()
                    .map(|p| {
                        if p.title.is_empty() {
                            p.step_id.clone()
                        } else {
                            p.title.clone()
                        }
                    })
                    .collect();
                (msg, titles)
            }
            Err(e) => return Outcome::err(e),
        }
    };
    // Refresh work-state next from plan titles (goal lock released).
    {
        let mut ws = st.work_state.lock().await;
        ws.next = titles;
        ws.in_progress.clear();
        ws.done.clear();
        ws.last_activity = "goal:plan_ready".into();
        ws.touch();
        let version = ws.version;
        let goal = ws.goal.clone();
        let recent_files = ws.recent_files.clone();
        let last_activity = ws.last_activity.clone();
        let next = ws.next.clone();
        drop(ws);
        emit(
            &Event::new("work_state")
                .with("version", json!(version))
                .with("goal", json!(goal))
                .with("done", json!([]))
                .with("in_progress", json!([]))
                .with("next", json!(next))
                .with("recent_files", json!(recent_files))
                .with("last_activity", json!(last_activity)),
        );
    }
    Outcome::ok(msg)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn truncate_str(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_mode() -> GoalMode {
        GoalMode {
            id: "g1".into(),
            goal: "ship feature X".into(),
            phase: GoalPhase::Planning,
            concurrency: 2,
            max_tasks: 4,
            allowed_models: vec!["m1".into()],
            allowed_providers: vec![],
            auto_deploy: true,
            parent_model: "m1".into(),
            reasoning_effort: "medium".into(),
            ..GoalMode::default()
        }
    }

    #[test]
    fn is_active_during_planning_not_done() {
        let mut m = base_mode();
        assert!(m.is_active()); // Planning
        m.phase = GoalPhase::PlanReady;
        assert!(m.is_active());
        m.phase = GoalPhase::Deploying;
        assert!(m.is_active());
        m.phase = GoalPhase::Running;
        assert!(m.is_active());
        m.phase = GoalPhase::Done;
        assert!(!m.is_active());
        m.phase = GoalPhase::Failed;
        assert!(!m.is_active());
        m.phase = GoalPhase::Idle;
        assert!(!m.is_active());
    }

    fn start_args(goal: &str) -> StartGoalArgs {
        StartGoalArgs {
            goal: goal.into(),
            concurrency: None,
            max_tasks: None,
            allowed_models: vec![],
            allowed_providers: vec![],
            auto_deploy: None,
            role_models: RoleModels::default(),
            model_concurrency: HashMap::new(),
            model: "m".into(),
            reasoning_effort: None,
            default_concurrency: 4,
            default_max_tasks: 8,
        }
    }

    #[test]
    fn new_goal_rejects_empty() {
        let err = new_goal(start_args("  ")).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn role_models_and_model_concurrency_normalized() {
        let mut args = start_args("ship the feature now");
        args.concurrency = Some(3);
        args.allowed_models = vec!["m1".into(), "m2".into()];
        args.role_models = RoleModels {
            planner: Some("m1".into()),
            worker: Some("not-allowed".into()), // dropped
            reviewer: Some("m2".into()),
        };
        args.model_concurrency.insert("m1".into(), 99); // clamp to 3
        args.model_concurrency.insert("ghost".into(), 2); // dropped vs allowlist
        let mode = new_goal(args).unwrap();
        assert_eq!(mode.role_models.planner.as_deref(), Some("m1"));
        assert!(mode.role_models.worker.is_none());
        assert_eq!(mode.role_models.reviewer.as_deref(), Some("m2"));
        assert_eq!(mode.model_concurrency.get("m1"), Some(&3));
        assert!(!mode.model_concurrency.contains_key("ghost"));
        assert_eq!(mode.parent_model, "m1"); // planner pin for planning turn
    }

    #[test]
    fn resolve_step_model_prefers_role() {
        let mut mode = base_mode();
        mode.role_models.worker = Some("m1".into());
        mode.allowed_models = vec!["m1".into()];
        let m = resolve_step_model(&mode, "worker", Some("other".into()));
        assert_eq!(m.as_deref(), Some("m1"));
        let m2 = resolve_step_model(&mode, "scout", Some("m1".into()));
        assert_eq!(m2.as_deref(), Some("m1"));
    }

    #[test]
    fn apply_plan_happy_path_auto_deploy() {
        let mut mode = base_mode();
        let args = json!({
            "summary": "two-step",
            "steps": [
                {"id": "a", "agent": "scout", "title": "recon", "task": "map auth"},
                {"id": "b", "agent": "worker", "title": "impl", "task": "implement", "depends_on": ["a"]}
            ],
            "risks": ["r1"],
            "validation": ["tests pass"]
        });
        let known = HashSet::new();
        apply_plan(&mut mode, &args, &known).unwrap();
        assert_eq!(mode.phase, GoalPhase::PlanReady);
        assert!(mode.deploy_after_turn);
        assert_eq!(mode.prompts.len(), 2);
        assert!(mode.prompts[0].task.contains("ship feature X"));
    }

    #[test]
    fn apply_plan_review_mode_no_auto() {
        let mut mode = base_mode();
        mode.auto_deploy = false;
        let args = json!({
            "summary": "one",
            "steps": [{"id": "1", "agent": "worker", "title": "do", "task": "work"}]
        });
        apply_plan(&mut mode, &args, &HashSet::new()).unwrap();
        assert!(!mode.deploy_after_turn);
        assert_eq!(mode.phase, GoalPhase::PlanReady);
    }

    #[test]
    fn apply_plan_rejects_too_many_steps() {
        let mut mode = base_mode();
        mode.max_tasks = 1;
        let args = json!({
            "summary": "x",
            "steps": [
                {"id": "1", "agent": "worker", "task": "a"},
                {"id": "2", "agent": "worker", "task": "b"}
            ]
        });
        assert!(apply_plan(&mut mode, &args, &HashSet::new()).is_err());
    }

    #[test]
    fn apply_plan_rejects_cycle() {
        let mut mode = base_mode();
        let args = json!({
            "summary": "x",
            "steps": [
                {"id": "a", "agent": "worker", "task": "a", "depends_on": ["b"]},
                {"id": "b", "agent": "worker", "task": "b", "depends_on": ["a"]}
            ]
        });
        assert!(apply_plan(&mut mode, &args, &HashSet::new()).is_err());
    }

    #[test]
    fn topo_waves_order() {
        let steps = vec![
            GoalStep {
                id: "a".into(),
                agent: "scout".into(),
                title: "a".into(),
                task: "a".into(),
                model: None,
                depends_on: vec![],
                parallel_group: None,
            },
            GoalStep {
                id: "b".into(),
                agent: "worker".into(),
                title: "b".into(),
                task: "b".into(),
                model: None,
                depends_on: vec!["a".into()],
                parallel_group: None,
            },
            GoalStep {
                id: "c".into(),
                agent: "worker".into(),
                title: "c".into(),
                task: "c".into(),
                model: None,
                depends_on: vec!["a".into()],
                parallel_group: Some("impl".into()),
            },
        ];
        let waves = topo_waves(&steps).unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0], vec!["a".to_string()]);
        assert!(waves[1].contains(&"b".to_string()) && waves[1].contains(&"c".to_string()));
    }

    #[test]
    fn filter_models_by_allowlist() {
        let mode = GoalMode {
            allowed_models: vec!["m1".into()],
            allowed_providers: vec!["openai".into()],
            ..GoalMode::default()
        };
        let mut map = HashMap::new();
        map.insert("m1".into(), "openai".into());
        map.insert("m2".into(), "anthropic".into());
        map.insert("m3".into(), "openai".into());
        let out = filter_model_candidates(
            &["m1".into(), "m2".into(), "m3".into()],
            &mode,
            &map,
        );
        assert_eq!(out, vec!["m1".to_string()]);
    }

    #[test]
    fn cap_concurrency_respects_goal() {
        let mode = GoalMode {
            concurrency: 2,
            ..GoalMode::default()
        };
        assert_eq!(cap_concurrency(8, &mode), 2);
        assert_eq!(cap_concurrency(1, &mode), 1);
    }

    #[test]
    fn strips_disallowed_step_model() {
        let mut mode = base_mode();
        mode.allowed_models = vec!["m1".into()];
        let args = json!({
            "summary": "x",
            "steps": [{
                "id": "1", "agent": "worker", "task": "t", "model": "other-model"
            }]
        });
        apply_plan(&mut mode, &args, &HashSet::new()).unwrap();
        // stripped → falls back to first allowed model
        assert_eq!(mode.prompts[0].model.as_deref(), Some("m1"));
    }
}
