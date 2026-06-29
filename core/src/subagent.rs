// Subagent system: a port of pi-subagents' delegation/orchestration features,
// adapted to this harness's single-process, Rust-core architecture.
//
// Subagents are nested agentic loops (see run_agent) that share the workspace,
// tools, and API key but run with a focused system prompt and an optional tool
// allowlist. They are defined as markdown files with YAML frontmatter
// (agents/*.md), discovered from builtin (embedded), user, and project scopes.
//
// Execution modes (the `subagent` tool):
//   - single:  { agent, task, ... }
//   - parallel:{ tasks: [...], concurrency, worktree }
//   - chain:   { chain: [...], async }
//   - management: { action: list|get|create|update|delete|status|interrupt|resume|doctor }
//
// Coordination (see intercom.rs) is wired in here: a subagent whose resolved
// tools include `contact_supervisor`/`intercom` gets those tools + bridge
// instructions, and can prompt the orchestrator for decisions or talk to peer
// subagents when the setup allows it.

use crate::config::{Config, SubagentConfig};
use crate::intercom::{execute_contact_supervisor, execute_intercom};
use crate::logging::{estimate_messages_tokens, TurnTimer};
use crate::protocol::{emit, Event};
use crate::tools::{self, Outcome};
use crate::State;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Frontmatter parsing (port of frontmatter.ts)
// ---------------------------------------------------------------------------

/// Parse YAML-ish frontmatter (key: value, flat only) + markdown body.
pub fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    let mut fm: HashMap<String, String> = HashMap::new();
    let normalized = content.replace("\r\n", "\n");
    if !normalized.starts_with("---") {
        return (fm, normalized.trim().to_string());
    }

    let end = match normalized.find("\n---") {
        Some(i) if i >= 3 => i,
        _ => return (fm, normalized),
    };
    let block = &normalized[4..end];
    let body = normalized[end + 4..].trim().to_string();
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let mut val = line[colon + 1..].trim().to_string();
            // strip surrounding quotes
            if val.len() >= 2
                && ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
            {
                val = val[1..val.len() - 1].to_string();
            }
            fm.insert(key, val);
        }
    }
    (fm, body)
}

// ---------------------------------------------------------------------------
// Agent config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum SystemPromptMode {
    Replace,
    Append,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum ContextKind {
    Fresh,
    Fork,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum AgentSource {
    Builtin,
    User,
    Project,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AgentConfig {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub fallback_models: Vec<String>,
    pub thinking: Option<String>,
    pub system_prompt_mode: SystemPromptMode,
    pub inherit_project_context: bool,
    pub inherit_skills: bool,
    pub default_context: Option<ContextKind>,
    pub system_prompt: String,
    pub source: AgentSource,
    pub file_path: String,
    pub skills: Vec<String>,
    pub output: Option<String>,
    pub default_reads: Vec<String>,
    pub default_progress: bool,
    pub max_subagent_depth: Option<u32>,
    pub completion_guard: bool,
    pub disabled: bool,
}

impl AgentConfig {
    /// Map a pi-style tool name to this harness's tool name.
    pub fn normalize_tool(name: &str) -> &str {
        match name {
            "read" => "read_file",
            "find" => "glob",
            "ls" => "list_dir",
            "write" => "write_file",
            // bash, edit, grep, glob, list_dir, patch, diagnostics, subagent,
            // contact_supervisor, intercom, todo_* pass through unchanged.
            other => other,
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in agents (embedded fallback; .umans-harness/agents/*.md overrides)
// ---------------------------------------------------------------------------

fn builtin_agents() -> Vec<AgentConfig> {
    let mk = |name: &str,
              desc: &str,
              tools: &str,
              thinking: Option<&str>,
              append: bool,
              inherit_ctx: bool,
              default_ctx: Option<ContextKind>,
              prompt: &str| {
                AgentConfig {
                    name: name.to_string(),
                    description: desc.to_string(),
                    tools: tools.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                    model: None,
                    fallback_models: vec![],
                    thinking: thinking.map(|s| s.to_string()),
                    system_prompt_mode: if append { SystemPromptMode::Append } else { SystemPromptMode::Replace },
                    inherit_project_context: inherit_ctx,
                    inherit_skills: false,
                    default_context: default_ctx,
                    system_prompt: prompt.to_string(),
                    source: AgentSource::Builtin,
                    file_path: format!("<builtin:{name}>"),
                    skills: vec![],
                    output: if name == "scout" { Some("context.md".into()) } else { None },
                    default_reads: if name == "worker" || name == "reviewer" {
                        vec!["context.md".into(), "plan.md".into()]
                    } else {
                        vec![]
                    },
                    default_progress: name == "worker" || name == "scout",
                    max_subagent_depth: None,
                    completion_guard: false,
                    disabled: false,
                }
            };
    vec![
        mk("scout", "Fast codebase recon that returns compressed context for handoff",
           "read_file, grep, glob, list_dir, bash, write_file, intercom", Some("low"),
           false, true, None, SCOUT_PROMPT),
        mk("researcher", "Web/docs research with sources and a concise research brief",
           "read_file, grep, glob, list_dir, bash, write_file, intercom", Some("low"),
           false, true, None, RESEARCHER_PROMPT),
        mk("planner", "A concrete implementation plan from existing context; reads and plans, does not edit",
           "read_file, grep, glob, list_dir, bash, intercom", Some("high"),
           false, true, Some(ContextKind::Fork), PLANNER_PROMPT),
        mk("worker", "Implementation agent for normal tasks and approved oracle handoffs",
           "read_file, grep, glob, list_dir, bash, edit, write_file, contact_supervisor", Some("high"),
           false, true, Some(ContextKind::Fork), WORKER_PROMPT),
        mk("reviewer", "Code review and small fixes against the task/plan, tests, edge cases, simplicity",
           "read_file, grep, glob, list_dir, bash, edit, write_file, intercom", Some("high"),
           false, true, None, REVIEWER_PROMPT),
        mk("context-builder", "Stronger setup pass before planning: gathers context and writes handoff material",
           "read_file, grep, glob, list_dir, bash, write_file, intercom", Some("low"),
           false, true, None, CONTEXT_BUILDER_PROMPT),
        mk("oracle", "High-context decision-consistency oracle; challenges assumptions, prevents drift",
           "read_file, grep, glob, list_dir, bash, intercom", Some("high"),
           false, true, Some(ContextKind::Fork), ORACLE_PROMPT),
        mk("delegate", "Lightweight general delegate that behaves close to the parent session",
           "read_file, grep, glob, list_dir, bash, edit, write_file, contact_supervisor", None,
           true, true, None, DELEGATE_PROMPT),
    ]
}

const SCOUT_PROMPT: &str = "You are a scouting subagent. Move fast, but do not guess. Use targeted search and selective reading over reading whole files unless the task clearly needs broader coverage.\n\nFocus on the minimum context another agent needs to act: relevant entry points, key types/interfaces/functions, data flow and dependencies, files likely to need changes, and constraints/risks/open questions.\n\nWorking rules:\n- Use grep, glob, list_dir, and read_file to map the area before diving deeper.\n- Use bash only for non-interactive inspection.\n- Cite exact file paths and line ranges.\n- If told to write output, write it to the provided path and keep the final response short.\n- If blocked or needing a decision, use contact_supervisor with reason \"need_decision\" and wait for the reply.\n\nOutput format:\n# Code Context\n## Files Retrieved (exact paths + line ranges + why)\n## Key Code (critical types/functions/snippets)\n## Architecture (how pieces connect)\n## Start Here (first file another agent should open + why)";

const RESEARCHER_PROMPT: &str = "You are a research subagent. Gather external evidence: official docs, specs, benchmarks, recent changes. Return a concise research brief with source links, confidence level, gaps, and decision implications. Do not edit code. If blocked or needing a decision, use contact_supervisor with reason \"need_decision\" and wait for the reply.";

const PLANNER_PROMPT: &str = "You are a planning subagent. Produce a concrete, actionable implementation plan from the supplied context. Read and plan; do not edit code. Include: goals, affected files, step-by-step changes, risks, validation steps, and open questions. Treat inherited forked context as reference-only — do not continue prior conversations. If a decision is missing that blocks planning, use contact_supervisor with reason \"need_decision\" and wait.";

const WORKER_PROMPT: &str = "You are `worker`: the implementation subagent. You are the single writer thread. Execute the assigned task or approved direction with narrow, coherent edits. The main agent and user remain the decision authority.\n\nFirst understand the inherited context, supplied files, plan, and explicit task. Then implement carefully and minimally. If implementation reveals an unapproved decision required to continue safely, pause and escalate with contact_supervisor (reason \"need_decision\") and wait for the reply before continuing. Use reason \"progress_update\" only for concise non-blocking updates.\n\nWorking rules:\n- Prefer narrow, correct changes over broad rewrites.\n- Do not add speculative scaffolding.\n- Do not leave TODOs or silent scope changes.\n- Use bash for inspection, validation, and tests.\n- Read supplied context/plan first.\n- If your task expects edits and you made none, do not return a success summary.\n\nFinal response shape:\nImplemented X.\nChanged files: Y.\nValidation: Z.\nOpen risks/questions: R.\nRecommended next step: N.";

const REVIEWER_PROMPT: &str = "You are a disciplined review subagent. Inspect, evaluate, and report findings with evidence. Do not guess; verify from code, tests, docs, or requirements.\n\nReview: implementation vs intent, correctness/edge-cases, test coverage, unintended side effects/regressions, and simplicity/readability. Return concise, evidence-backed findings with file/line references. Make small fixes only if asked. If blocked or needing a decision, use contact_supervisor/intercom with reason \"need_decision\" and wait.";

const CONTEXT_BUILDER_PROMPT: &str = "You are a context-building subagent. Gather the code context another agent needs before planning or implementation. Read every relevant file, follow imports/callers/tests/docs/config, and write handoff material (e.g. context.md) plus a compact meta-prompt. Do not implement features. If blocked, use contact_supervisor with reason \"need_decision\" and wait.";

const ORACLE_PROMPT: &str = "You are the oracle: a high-context decision-consistency subagent. Prevent the main agent from making hidden, conflicting, or inconsistent decisions by treating inherited forked context as the authoritative contract. You are not the primary executor and do not edit files.\n\nReconstruct inherited decisions/constraints/open questions; identify drift between the current trajectory and those decisions; surface contradictions and hidden assumptions. Prefer narrow corrections over broad pivots. If you need clarification, use contact_supervisor with reason \"need_decision\" and wait for the reply.\n\nOutput shape:\nInherited decisions:\nDiagnosis:\nDrift / contradiction check:\nRecommendation:\nRisks:\nNeed from main agent:\nSuggested execution prompt (only if a worker handoff is warranted):";

const DELEGATE_PROMPT: &str = "You are a delegated agent. Execute the assigned task using the provided tools. Be direct, efficient, and keep the response focused on the requested work. If blocked or needing a decision, use contact_supervisor with reason \"need_decision\" and wait for the reply.";

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Resolve an agent's intercom target name (stable per run).
pub fn subagent_target(run_id: &str, agent: &str, index: Option<usize>) -> String {
    let suffix = index.map(|i| format!("-{}", i + 1)).unwrap_or_default();
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect::<String>().to_lowercase();
    let rid = run_id.replace('-', "");
    format!("subagent-{}-{}{}", clean(agent), &rid[..rid.len().min(8)], suffix)
}

/// Discover all agents: builtin (lowest) < user < project (project wins on name).
/// Applies settings overrides (model/fallback/thinking/disabled).
pub fn discover_agents(workspace: &Path, cfg: &SubagentConfig) -> Vec<AgentConfig> {
    let mut by_name: HashMap<String, AgentConfig> = HashMap::new();

    if !cfg.disable_builtins {
        for mut a in builtin_agents() {
            apply_overrides(&mut a, cfg);
            if !a.disabled {
                by_name.insert(a.name.clone(), a);
            }
        }
    }

    // user scope: ~/.umans-harness/agents/**/*.md
    if let Some(home) = crate::config::home_dir() {
        let dir = home.join(".umans-harness/agents");
        load_agent_dir(&dir, AgentSource::User, &mut by_name);
    }
    // project scope: <workspace>/.umans-harness/agents/**/*.md
    let dir = workspace.join(".umans-harness/agents");
    load_agent_dir(&dir, AgentSource::Project, &mut by_name);

    let mut v: Vec<AgentConfig> = by_name.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn load_agent_dir(dir: &Path, source: AgentSource, by_name: &mut HashMap<String, AgentConfig>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            load_agent_dir(&p, source.clone(), by_name);
            continue;
        }
        if p.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&p) else { continue };
        let (fm, body) = parse_frontmatter(&content);
        let name = match fm.get("name").and_then(|s| s.split_whitespace().next()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let tools_str = fm.get("tools").cloned().unwrap_or_default();
        let a = AgentConfig {
            name: name.clone(),
            description: fm.get("description").cloned().unwrap_or_default(),
            tools: tools_str.split(',').map(|s| AgentConfig::normalize_tool(s.trim()).to_string()).filter(|s| !s.is_empty()).collect(),
            model: fm.get("model").cloned(),
            fallback_models: fm.get("fallbackModels").map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default(),
            thinking: fm.get("thinking").cloned(),
            system_prompt_mode: match fm.get("systemPromptMode").map(|s| s.as_str()) {
                Some("append") => SystemPromptMode::Append,
                _ => SystemPromptMode::Replace,
            },
            inherit_project_context: fm.get("inheritProjectContext").map(|s| s == "true").unwrap_or(false),
            inherit_skills: fm.get("inheritSkills").map(|s| s == "true").unwrap_or(false),
            default_context: match fm.get("defaultContext").map(|s| s.as_str()) {
                Some("fork") => Some(ContextKind::Fork),
                Some("fresh") => Some(ContextKind::Fresh),
                _ => None,
            },
            system_prompt: body,
            source: source.clone(),
            file_path: p.display().to_string(),
            skills: fm.get("skills").map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default(),
            output: fm.get("output").cloned(),
            default_reads: fm.get("defaultReads").map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default(),
            default_progress: fm.get("defaultProgress").map(|s| s == "true").unwrap_or(false),
            max_subagent_depth: fm.get("maxSubagentDepth").and_then(|s| s.parse().ok()),
            completion_guard: fm.get("completionGuard").map(|s| s == "false").unwrap_or(false),
            disabled: fm.get("disabled").map(|s| s == "true").unwrap_or(false),
        };
        if !a.disabled {
            by_name.insert(name, a);
        }
    }
}

fn apply_overrides(a: &mut AgentConfig, cfg: &SubagentConfig) {
    if let Some(ov) = cfg.agent_overrides.get(&a.name) {
        if let Some(m) = &ov.model { a.model = Some(m.clone()); }
        if !ov.fallback_models.is_empty() { a.fallback_models = ov.fallback_models.clone(); }
        if let Some(t) = &ov.thinking { a.thinking = Some(t.clone()); }
        if ov.disabled { a.disabled = true; }
    }
}

pub fn find_agent<'a>(agents: &'a [AgentConfig], name: &str) -> Option<&'a AgentConfig> {
    // allow package.name syntax: code-analysis.scout → scout
    let bare = name.rsplit('.').next().unwrap_or(name);
    agents.iter().find(|a| a.name == name || a.name == bare)
}

// ---------------------------------------------------------------------------
// Skills (SKILL.md discovery + injection)
// ---------------------------------------------------------------------------

fn discover_skills(workspace: &Path) -> Vec<(String, String, String)> {
    // (name, description, location) — project first, then user.
    let mut out: Vec<(String, String, String)> = Vec::new();
    let dirs = [
        (workspace.join(".umans-harness/skills"), true),
        (crate::config::home_dir().map(|h| h.join(".umans-harness/skills")).unwrap_or_default(), false),
    ];
    for (dir, _proj) in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let skill_md = e.path().join("SKILL.md");
            if let Ok(content) = std::fs::read_to_string(&skill_md) {
                let (fm, _) = parse_frontmatter(&content);
                let name = fm.get("name").cloned().unwrap_or_else(|| e.file_name().to_string_lossy().into_owned());
                let desc = fm.get("description").cloned().unwrap_or_default();
                out.push((name, desc, skill_md.display().to_string()));
            }
        }
    }
    out
}

fn skills_injection(workspace: &Path, names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let all = discover_skills(workspace);
    let mut blocks = String::new();
    for name in names {
        if name == "false" { continue; }
        if let Some((n, d, loc)) = all.iter().find(|(n, _, _)| n == name) {
            blocks.push_str(&format!(
                "  <skill>\n    <name>{n}</name>\n    <description>{d}</description>\n    <location>{loc}</location>\n  </skill>\n"
            ));
        }
    }
    if blocks.is_empty() { return String::new(); }
    format!("The following skills are available to this subagent. Use read_file to load a skill file when the task matches its description.\n<available_skills>\n{blocks}</available_skills>")
}

// ---------------------------------------------------------------------------
// Intercom bridge instructions
// ---------------------------------------------------------------------------

pub const INTERCOM_BRIDGE_MARKER: &str = "Intercom orchestration channel:";

pub fn bridge_instruction(orchestrator_target: &str) -> String {
    format!("{INTERCOM_BRIDGE_MARKER}\nThe inherited thread is reference-only. Do not continue that conversation or send questions/status/completion handoffs to the supervisor in normal assistant text.\n\nUse contact_supervisor first. It resolves the supervisor \"{orchestrator_target}\" automatically.\n- Need a decision, blocked, approval, or scope ambiguity: contact_supervisor({{ reason: \"need_decision\", message: \"<question>\" }}). After a need_decision, stay alive and continue only after the reply arrives.\n- Meaningful progress or unexpected discoveries that change the plan: contact_supervisor({{ reason: \"progress_update\", message: \"UPDATE: <summary>\" }}).\n- Generic intercom is lower-level plumbing/fallback for peer subagents: intercom({{ action: \"ask\", to: \"<peer>\", message: \"...\" }}).\n\nDo not use contact_supervisor/intercom for routine completion handoffs. If no coordination is needed, return a focused task result.")
}

/// Decide whether intercom tools are injected for a subagent run, given the
/// bridge mode and the run's context kind.
pub fn bridge_active(mode: &crate::config::IntercomBridgeMode, ctx: Option<&ContextKind>) -> bool {
    use crate::config::IntercomBridgeMode;
    match mode {
        IntercomBridgeMode::Off => false,
        IntercomBridgeMode::ForkOnly => ctx == Some(&ContextKind::Fork),
        IntercomBridgeMode::Always => true,
    }
}

// ---------------------------------------------------------------------------
// Recursion guard
// ---------------------------------------------------------------------------

pub fn resolve_max_depth(cfg: &SubagentConfig) -> u32 {
    std::env::var("UMANS_SUBAGENT_MAX_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &u32| *n == 0 || *n >= 1)
        .unwrap_or(cfg.max_depth)
}

pub fn child_max_depth(parent: u32, agent: Option<u32>) -> u32 {
    match agent {
        Some(a) => parent.min(a),
        None => parent,
    }
}

// ---------------------------------------------------------------------------
// Tool defs available to a subagent (filtered by agent.tools + bridge)
// ---------------------------------------------------------------------------

fn all_tool_names() -> &'static [&'static str] {
    &[
        "read_file", "edit", "write_file", "list_dir", "grep", "glob", "bash",
        "bulk", "bulk_read", "bulk_write", "bulk_edit", "todo_write", "todo_read",
        "finish", "patch", "diagnostics", "subagent", "contact_supervisor", "intercom",
    ]
}

/// Build the tool-definition list a subagent may call, applying the agent's
/// allowlist and the intercom bridge. `depth`/`max_depth` gate the `subagent`
/// tool (nested fanout only when allowed and below the depth cap).
pub fn subagent_tool_defs(agent: &AgentConfig, bridge: bool, depth: u32, max_depth: u32) -> Vec<Value> {
    let all = tools::definitions();
    // name → def
    let by_name: HashMap<&str, &Value> = all
        .iter()
        .map(|d| (d.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or(""), d))
        .collect();

    // Resolve allowed tool names.
    let allow_subagent = agent.tools.iter().any(|t| t == "subagent") && depth + 1 < max_depth;
    let mut names: Vec<&str> = if agent.tools.is_empty() {
        all_tool_names().to_vec()
    } else {
        agent.tools.iter().filter_map(|t| {
            let n = AgentConfig::normalize_tool(t);
            // filter out subagent unless explicitly allowed + below depth
            if n == "subagent" && !allow_subagent { None } else { Some(n) }
        }).collect()
    };
    // bridge tools
    if bridge {
        for t in ["contact_supervisor", "intercom"] {
            if !names.contains(&t) {
                names.push(t);
            }
        }
    }
    // subagent only via explicit allowlist + depth
    if allow_subagent && !names.contains(&"subagent") {
        names.push("subagent");
    }
    // always allow finish so the subagent can end
    if !names.contains(&"finish") {
        names.push("finish");
    }

    names.iter().filter_map(|n| by_name.get(n).map(|v| (*v).clone())).collect()
}

// ---------------------------------------------------------------------------
// Run tracking (for status/interrupt/resume)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SubagentRun {
    pub id: String,
    pub mode: String, // single | parallel | chain
    pub agent: Option<String>,
    pub agents: Vec<String>,
    pub state: String, // running | completed | failed | paused
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub depth: u32,
    pub intercom_target: Option<String>,
    pub cancel: Option<Arc<CancellationToken>>,
    pub children: Vec<SubagentRun>,
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Execution entry point (the `subagent` tool body)
// ---------------------------------------------------------------------------

pub fn execute(
    st: Arc<State>,
    client: reqwest::Client,
    api_key: String,
    parent_model: String,
    args: Value,
    cancel: CancellationToken,
    depth: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>> {
    Box::pin(async move {
    let workspace = st.cfg.read().await.workspace.clone();
    let cfg = st.cfg.read().await.clone();
    let max_depth = resolve_max_depth(&cfg.subagents);

    // Depth guard: a subagent calling subagent beyond the cap is blocked.
    if depth >= max_depth {
        return Outcome::err(format!(
            "subagent nesting blocked at depth {depth} (max {max_depth}); complete the current task directly"
        ));
    }

    // Backward-compat: a bare {prompt} (legacy spawn) runs as a delegate.
    if args.get("agent").is_none() && args.get("tasks").is_none() && args.get("chain").is_none() {
        if let Some(prompt) = args.get("prompt").and_then(|v| v.as_str()) {
            let agents = discover_agents(&workspace, &cfg.subagents);
            let agent = find_agent(&agents, "delegate").cloned().unwrap_or_else(|| {
                builtin_agents().into_iter().find(|a| a.name == "delegate").unwrap().clone()
            });
            let model_override = args.get("model").and_then(|v| v.as_str()).map(String::from);
            let run_id = next_run_id();
            return run_single(&st, &client, &api_key, &parent_model, &agent, prompt, &run_id, model_override, ContextKind::Fresh, depth, &cancel).await;
        }
    }

    // Management / control actions.
    if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
        return handle_action(action, &args, &workspace, &cfg, &st, &cancel).await;
    }

    // Single agent run.
    if let Some(agent_name) = args.get("agent").and_then(|v| v.as_str()) {
        let agents = discover_agents(&workspace, &cfg.subagents);
        let agent = match find_agent(&agents, agent_name) {
            Some(a) => a.clone(),
            None => return Outcome::err(format!("unknown agent '{agent_name}'; use action:\"list\" to discover agents")),
        };
        let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let model_override = args.get("model").and_then(|v| v.as_str()).map(String::from);
        let context = parse_context(&args, &agent);
        let run_id = next_run_id();
        return run_single(&st, &client, &api_key, &parent_model, &agent, &task, &run_id, model_override, context, depth, &cancel).await;
    }

    // Parallel run.
    if let Some(tasks) = args.get("tasks").and_then(|v| v.as_array()) {
        return run_parallel(&st, &client, &api_key, &parent_model, tasks, &args, depth, &cancel).await;
    }

    // Chain run.
    if let Some(chain) = args.get("chain").and_then(|v| v.as_array()) {
        return run_chain(&st, &client, &api_key, &parent_model, chain, &args, depth, &cancel).await;
    }

    Outcome::err("subagent requires 'agent'+'task', 'tasks', 'chain', or 'action'")
    })
}

fn parse_context(args: &Value, agent: &AgentConfig) -> ContextKind {
    match args.get("context").and_then(|v| v.as_str()) {
        Some("fork") => ContextKind::Fork,
        Some("fresh") => ContextKind::Fresh,
        _ => agent.default_context.clone().unwrap_or(ContextKind::Fresh),
    }
}

fn next_run_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::SeqCst);
    format!("run-{n:x}")
}

// ---------------------------------------------------------------------------
// Single-agent run (the nested agentic loop)
// ---------------------------------------------------------------------------

async fn run_single(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    agent: &AgentConfig,
    task: &str,
    run_id: &str,
    model_override: Option<String>,
    context: ContextKind,
    depth: u32,
    cancel: &CancellationToken,
) -> Outcome {
    let cfg = st.cfg.read().await.clone();
    let max_depth = resolve_max_depth(&cfg.subagents);
    let bridge = bridge_active(&cfg.subagents.intercom_bridge_mode, Some(&context));
    let my_target = subagent_target(run_id, &agent.name, None);
    let orchestrator = st.intercom.orchestrator_target();
    if bridge {
        st.intercom.register_target(&my_target);
    }

    // Register the run for status/interrupt.
    let run_cancel = CancellationToken::new();
    let run = SubagentRun {
        id: run_id.to_string(), mode: "single".into(), agent: Some(agent.name.clone()),
        agents: vec![agent.name.clone()], state: "running".into(),
        started_at: now_ms(), ended_at: None, depth, intercom_target: Some(my_target.clone()),
        cancel: Some(Arc::new(run_cancel.clone())), children: vec![], summary: None,
    };
    st.subagent_runs.lock().await.insert(run_id.to_string(), run);

    let result = run_agent(st, client, api_key, parent_model, agent, task, &my_target, &orchestrator, run_id, model_override, context, depth, max_depth, bridge, cancel).await;

    // Finalize run state.
    let mut runs = st.subagent_runs.lock().await;
    if let Some(r) = runs.get_mut(run_id) {
        r.state = if result.ok { "completed" } else { "failed" }.into();
        r.ended_at = Some(now_ms());
        r.summary = Some(result.output.chars().take(200).collect());
    }
    if bridge {
        st.intercom.unregister(&my_target);
    }
    result
}

/// The nested agentic loop for one agent. Mirrors run_spawn but applies the
/// agent config: filtered tools, system prompt, forked context, model fallback,
/// reads/output/progress, and intercom dispatch.
pub async fn run_agent(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    agent: &AgentConfig,
    task: &str,
    my_target: &str,
    orchestrator: &str,
    run_id: &str,
    model_override: Option<String>,
    context: ContextKind,
    depth: u32,
    max_depth: u32,
    bridge: bool,
    cancel: &CancellationToken,
) -> Outcome {
    emit_subagent_progress(run_id, agent, "start", "", 0, 0, 0, 0, true);
    let result = run_agent_inner(st, client, api_key, parent_model, agent, task, my_target, orchestrator, run_id, model_override, context, depth, max_depth, bridge, cancel).await;
    emit_subagent_progress(run_id, agent, "done", "", 0, 0, 0, 0, result.ok);
    result
}

async fn run_agent_inner(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    agent: &AgentConfig,
    task: &str,
    my_target: &str,
    orchestrator: &str,
    run_id: &str,
    model_override: Option<String>,
    context: ContextKind,
    depth: u32,
    max_depth: u32,
    bridge: bool,
    cancel: &CancellationToken,
) -> Outcome {
    let workspace = st.cfg.read().await.workspace.clone();
    let cfg = st.cfg.read().await.clone();
    let tool_defs = subagent_tool_defs(agent, bridge, depth, max_depth);

    // --- system prompt ---
    let mut sys = match agent.system_prompt_mode {
        SystemPromptMode::Replace => String::new(),
        SystemPromptMode::Append => crate::build_system_prompt(&workspace, false),
    };
    if agent.inherit_project_context && agent.system_prompt_mode == SystemPromptMode::Replace {
        // even in replace mode, inherit git context + memory if asked
        sys.push_str(&crate::build_system_prompt(&workspace, false));
        sys.push_str("\n\n");
    }
    sys.push_str(&agent.system_prompt);
    if bridge {
        sys.push_str("\n\n");
        sys.push_str(&bridge_instruction(orchestrator));
    }
    // skills
    let skill_names: Vec<String> = if !agent.skills.is_empty() { agent.skills.clone() } else { vec![] };
    let sinj = skills_injection(&workspace, &skill_names);
    if !sinj.is_empty() {
        sys.push_str("\n\n");
        sys.push_str(&sinj);
    }

    let mut sub: Vec<Value> = vec![json!({ "role": "system", "content": sys })];

    // --- forked context: parent conversation as reference ---
    if context == ContextKind::Fork {
        let parent = st.conversation.lock().await.clone();
        let fork_msgs: Vec<Value> = parent.iter()
            .filter(|m| {
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
                // drop system + subagent tool-call/result artifacts; keep prose
                if role == "system" { return false; }
                if role == "assistant" {
                    // keep assistant prose, drop pure tool_call-only messages? keep them as reference
                    return true;
                }
                if role == "tool" {
                    // drop tool results that came from the subagent/spawn tool
                    let c = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    return !c.contains("spawn done") && !c.starts_with("subagent(");
                }
                true
            })
            .take(40)
            .cloned()
            .collect();
        if !fork_msgs.is_empty() {
            sub.push(json!({ "role": "system", "content": "You are running from a fork of the parent session. Treat the following inherited conversation as reference-only context, not a live thread to continue. Do not answer prior messages.\n\n--- inherited parent context ---" }));
            sub.extend(fork_msgs);
        }
    }

    // --- default reads ---
    let reads: Vec<String> = agent.default_reads.clone();
    if !reads.is_empty() {
        let mut read_ctx = String::new();
        for r in &reads {
            let p = workspace.join(r);
            if let Ok(c) = std::fs::read_to_string(&p) {
                read_ctx.push_str(&format!("--- {r} ---\n{}\n\n", truncate(&c, 8000)));
            }
        }
        if !read_ctx.is_empty() {
            sub.push(json!({ "role": "system", "content": format!("Reference files read before your task:\n\n{read_ctx}") }));
        }
    }

    // --- task message ---
    let task_msg = if context == ContextKind::Fork {
        json!({ "role": "user", "content": format!("Task:\n{task}") })
    } else {
        json!({ "role": "user", "content": task })
    };
    sub.push(task_msg);

    // --- model candidate list (fallback) ---
    let candidates = resolve_model_candidates(agent, parent_model, model_override, st);
    if candidates.is_empty() {
        return Outcome::err("no model resolved for subagent (set agent model or a default)");
    }
    let effort = agent.thinking.clone().unwrap_or_else(|| "medium".to_string());
    let thinking_levels = st.models.read().await.iter()
        .find(|m| candidates.iter().any(|c| c == &m.id))
        .map(|m| m.thinking_levels.clone())
        .unwrap_or_default();
    let model_ctx = st.models.read().await.iter().find(|m| candidates.iter().any(|c| c == &m.id))
        .map(|m| m.context_window as u64).unwrap_or(200_000);
    let mut timer = TurnTimer::new();
    let mut sub_in: u64 = 0;
    let mut sub_out: u64 = 0;
    let mut sub_cached: u64 = 0;
    let mut last_model: Option<String> = None;
    let run_start = std::time::Instant::now();
    let mut tool_count: u32 = 0;

    loop {
        if cancel.is_cancelled() {
            return Outcome::ok("[subagent aborted]");
        }
        // compaction gate
        let est = estimate_messages_tokens(&sub);
        let threshold = (model_ctx as f32 * cfg.context_compact_at) as u64;
        let hard_cap = (model_ctx as f32 * 0.95) as u64;
        if est > threshold.min(hard_cap) && sub.len() > 4 {
            crate::compact_with_summary(client, &cfg, api_key, candidates.first().unwrap(), &mut sub, cancel, est > hard_cap).await;
            emit(&Event::new("compacted").with("scope", json!("subagent")).with("before_tokens", json!(est)).with("after_tokens", json!(estimate_messages_tokens(&sub))));
        }

        crate::provider::sanitize_orphaned_tool_calls(&mut sub);

        // stream with model fallback
        let (assistant, _finish_reason, ti, to, cached) = match stream_with_fallback(
            client, &cfg, api_key, &candidates, &sub, &tool_defs, &effort, &thinking_levels, cancel, &mut timer,
        ).await {
            Ok(v) => v,
            Err(e) => {
                if e == "aborted" { emit(&Event::new("aborted")); }
                return Outcome::err(format!("subagent stream error: {e}"));
            }
        };
        last_model = Some(candidates[0].clone());
        sub_in += ti; sub_out += to; sub_cached += cached;
        *st.tokens_in.lock().await += ti;
        *st.tokens_out.lock().await += to;
        *st.cached_tokens.lock().await += cached;
        sub.push(assistant.clone());
        emit_subagent_progress(run_id, agent, "streaming", "", tool_count, sub_in, sub_out, run_start.elapsed().as_millis() as u64, true);

        let Some(calls) = assistant.get("tool_calls").and_then(|v| v.as_array()).cloned() else {
            // done — finalize output
            let text = assistant.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            // optional output file
            if let Some(out_path) = &agent.output {
                let p = workspace.join(out_path);
                if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
                let _ = std::fs::write(&p, &text);
            }
            emit_subagent_summary(sub_in, sub_out, sub_cached, &last_model);
            return Outcome::ok(text);
        };
        if calls.is_empty() {
            let text = assistant.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            emit_subagent_summary(sub_in, sub_out, sub_cached, &last_model);
            return Outcome::ok(text);
        }

        for call in &calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let func = call.get("function");
            let name = func.and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args_str = func.and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}").to_string();
            let argsv: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
            tool_count += 1;
            emit_subagent_progress(run_id, agent, "tool", &name, tool_count, sub_in, sub_out, run_start.elapsed().as_millis() as u64, true);

            let outcome = dispatch_subagent_tool(&name, &argsv, st, client, api_key, parent_model, my_target, agent, depth, max_depth, &cfg, cancel).await;

            emit_subagent_progress(run_id, agent, "tool_end", &name, tool_count, sub_in, sub_out, run_start.elapsed().as_millis() as u64, outcome.ok);
            sub.push(json!({ "role": "tool", "tool_call_id": id, "content": outcome.output }));

            // finish sentinel
            if name == "finish" && outcome.ok && outcome.output == "__finish__" {
                let text = assistant.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if let Some(out_path) = &agent.output {
                    let p = workspace.join(out_path);
                    if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
                    let _ = std::fs::write(&p, &text);
                }
                emit_subagent_summary(sub_in, sub_out, sub_cached, &last_model);
                return Outcome::ok(text);
            }
        }
    }
}

/// Dispatch a tool call inside a subagent loop. Handles async tools (bash/bulk/
/// diagnostics/subagent) + intercom tools; others go through tools::execute.
async fn dispatch_subagent_tool(
    name: &str,
    args: &Value,
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    my_target: &str,
    agent: &AgentConfig,
    depth: u32,
    max_depth: u32,
    cfg: &Config,
    cancel: &CancellationToken,
) -> Outcome {
    match name {
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            tools::execute_bash(cmd, cfg).await
        }
        "bulk" => tools::execute_bulk(args, cfg).await,
        "diagnostics" => tools::execute_diagnostics(args, cfg).await,
        "contact_supervisor" => execute_contact_supervisor(args, &st.intercom, my_target, cancel).await,
        "intercom" => execute_intercom(args, &st.intercom, my_target, cancel).await,
        "subagent" | "spawn" => {
            if depth + 1 >= max_depth {
                Outcome::err(format!("nested subagent blocked at depth {} (max {})", depth + 1, max_depth))
            } else {
                execute(st.clone(), client.clone(), api_key.to_string(), parent_model.to_string(), args.clone(), cancel.clone(), depth + 1).await
            }
        }
        _ => tools::execute(name, args, cfg),
    }
}

fn resolve_model_candidates(agent: &AgentConfig, parent_model: &str, override_model: Option<String>, st: &Arc<State>) -> Vec<String> {
    let mut cands: Vec<String> = Vec::new();
    if let Some(m) = override_model { cands.push(m); }
    if let Some(m) = &agent.model { cands.push(m.clone()); }
    for m in &agent.fallback_models { cands.push(m.clone()); }
    if cands.is_empty() { cands.push(parent_model.to_string()); }
    // keep only models known to the registry, but keep unknowns too (they may
    // be valid ids the registry didn't list); just dedup preserving order.
    let mut seen = std::collections::HashSet::new();
    cands.retain(|c| seen.insert(c.clone()));
    cands
}

/// stream_turn with model fallback: try each candidate in order until one
/// succeeds (or all fail). Aborted errors are not retried.
async fn stream_with_fallback(
    client: &reqwest::Client,
    cfg: &Config,
    api_key: &str,
    candidates: &[String],
    messages: &[Value],
    tools: &[Value],
    effort: &str,
    thinking_levels: &[String],
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
) -> Result<(Value, String, u64, u64, u64), String> {
    let mut last_err = String::from("no model candidates");
    for (i, model) in candidates.iter().enumerate() {
        let levels = if thinking_levels.is_empty() {
            // re-resolve per model
            Vec::new()
        } else {
            thinking_levels.to_vec()
        };
        match crate::provider::stream_turn(client, cfg, api_key, model, messages, tools, effort, &levels, cancel, timer, true).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if e == "aborted" || cancel.is_cancelled() {
                    return Err(e);
                }
                last_err = format!("model {} failed: {e}", model);
                if i + 1 < candidates.len() {
                    emit(&Event::new("info").with("message", json!(format!("subagent model '{}' failed ({}); falling back to '{}'", model, e, candidates[i + 1]))));
                }
            }
        }
    }
    Err(last_err)
}

fn emit_subagent_progress(run_id: &str, agent: &AgentConfig, phase: &str, tool: &str, tool_count: u32, tokens_in: u64, tokens_out: u64, elapsed_ms: u64, ok: bool) {
    emit(&Event::new("subagent_progress")
        .with("run_id", json!(run_id))
        .with("agent", json!(agent.name))
        .with("phase", json!(phase))
        .with("tool", json!(tool))
        .with("tool_count", json!(tool_count))
        .with("tokens_in", json!(tokens_in))
        .with("tokens_out", json!(tokens_out))
        .with("elapsed_ms", json!(elapsed_ms))
        .with("ok", json!(ok)));
}

fn emit_subagent_summary(sub_in: u64, sub_out: u64, sub_cached: u64, last_model: &Option<String>) {
    let m = last_model.clone().unwrap_or_else(|| "?".into());
    let pct = if sub_cached > 0 && sub_in > 0 { sub_cached * 100 / sub_in } else { 0 };
    if sub_cached > 0 && sub_in > 0 {
        emit(&Event::new("info").with("message", json!(format!("subagent done ({m}): {sub_in}+{sub_out}t ({pct}% cached)"))));
    } else {
        emit(&Event::new("info").with("message", json!(format!("subagent done ({m}): {sub_in}+{sub_out}t"))));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max]) }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Parallel run
// ---------------------------------------------------------------------------

async fn run_parallel(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    tasks: &[Value],
    args: &Value,
    depth: u32,
    cancel: &CancellationToken,
) -> Outcome {
    let workspace = st.cfg.read().await.workspace.clone();
    let cfg = st.cfg.read().await.clone();
    if tasks.is_empty() {
        return Outcome::err("parallel requires a non-empty 'tasks' array");
    }
    if tasks.len() as u32 > cfg.subagents.parallel_max_tasks {
        return Outcome::err(format!("parallel has {} tasks (max {})", tasks.len(), cfg.subagents.parallel_max_tasks));
    }
    let concurrency = args.get("concurrency").and_then(|v| v.as_u64()).unwrap_or(cfg.subagents.parallel_concurrency as u64).max(1) as usize;
    let context = args.get("context").and_then(|v| v.as_str());

    // resolve agents up front (fail fast on a bad name)
    let agents = discover_agents(&workspace, &cfg.subagents);
    let mut resolved: Vec<(AgentConfig, String, Option<String>, ContextKind)> = Vec::new();
    let mut agent_names: Vec<String> = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let an = t.get("agent").and_then(|v| v.as_str()).unwrap_or("");
        let agent = match find_agent(&agents, an) {
            Some(a) => a.clone(),
            None => return Outcome::err(format!("parallel task {i}: unknown agent '{an}'")),
        };
        agent_names.push(agent.name.clone());
        let task = t.get("task").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let model_override = t.get("model").and_then(|v| v.as_str()).map(String::from);
        let ctx = match context {
            Some("fork") => ContextKind::Fork,
            Some("fresh") => ContextKind::Fresh,
            _ => agent.default_context.clone().unwrap_or(ContextKind::Fresh),
        };
        resolved.push((agent, task, model_override, ctx));
    }

    let run_id = next_run_id();
    let run = SubagentRun {
        id: run_id.clone(), mode: "parallel".into(), agent: None, agents: agent_names.clone(),
        state: "running".into(), started_at: now_ms(), ended_at: None, depth,
        intercom_target: None, cancel: Some(Arc::new(cancel.clone())), children: vec![], summary: None,
    };
    st.subagent_runs.lock().await.insert(run_id.clone(), run);

    // run all tasks with a concurrency semaphore; collect results in order
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Outcome)>();
    for (i, (agent, task, model_override, ctx)) in resolved.into_iter().enumerate() {
        let stc = st.clone();
        let clientc = client.clone();
        let apik = api_key.to_string();
        let pm = parent_model.to_string();
        let cancelc = cancel.clone();
        let semc = sem.clone();
        let txc = tx.clone();
        let rid = format!("{}-{}", run_id, i);
        tokio::spawn(async move {
            let _permit = semc.acquire().await.ok();
            let o = run_single(&stc, &clientc, &apik, &pm, &agent, &task, &rid, model_override, ctx, depth, &cancelc).await;
            let _ = txc.send((i, o));
        });
    }
    drop(tx);
    let mut collected: Vec<(usize, Outcome)> = Vec::with_capacity(tasks.len());
    while let Some(item) = rx.recv().await {
        collected.push(item);
    }
    collected.sort_by_key(|(i, _)| *i);

    // finalize run
    let mut runs = st.subagent_runs.lock().await;
    if let Some(r) = runs.get_mut(&run_id) {
        r.state = "completed".into();
        r.ended_at = Some(now_ms());
    }

    let mut blocks = String::new();
    let mut all_ok = true;
    for (i, (_, o)) in collected.iter().enumerate() {
        if !o.ok { all_ok = false; }
        blocks.push_str(&format!("=== Parallel Task {} ({}) ===\n{}\n\n", i + 1, agent_names.get(i).cloned().unwrap_or_default(), o.output));
    }
    Outcome { ok: all_ok, output: blocks.trim().to_string() }
}

// ---------------------------------------------------------------------------
// Chain run (sequential; static parallel groups inline)
// ---------------------------------------------------------------------------

async fn run_chain(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    parent_model: &str,
    chain: &[Value],
    args: &Value,
    depth: u32,
    cancel: &CancellationToken,
) -> Outcome {
    if chain.is_empty() {
        return Outcome::err("chain requires a non-empty 'chain' array");
    }
    let workspace = st.cfg.read().await.workspace.clone();
    let cfg = st.cfg.read().await.clone();
    let agents = discover_agents(&workspace, &cfg.subagents);
    let run_id = next_run_id();
    let chain_dir = std::env::temp_dir().join(format!("umans-subagent-chain-{}", run_id));
    let _ = std::fs::create_dir_all(&chain_dir);

    let mut outputs: HashMap<String, String> = HashMap::new();
    let mut previous = String::new();

    let run = SubagentRun {
        id: run_id.clone(), mode: "chain".into(), agent: None,
        agents: chain.iter().filter_map(|s| s.get("agent").and_then(|v| v.as_str()).map(String::from)).collect(),
        state: "running".into(), started_at: now_ms(), ended_at: None, depth,
        intercom_target: None, cancel: Some(Arc::new(cancel.clone())), children: vec![], summary: None,
    };
    st.subagent_runs.lock().await.insert(run_id.clone(), run);

    for (step_i, step) in chain.iter().enumerate() {
        if cancel.is_cancelled() {
            return Outcome::ok("[chain aborted]");
        }
        // parallel group?
        if let Some(group) = step.get("parallel").and_then(|v| v.as_array()) {
            let group_args = json!({ "tasks": group, "context": args.get("context").and_then(|v| v.as_str()).unwrap_or("fresh"), "concurrency": step.get("concurrency").and_then(|v| v.as_u64()).unwrap_or(cfg.subagents.parallel_concurrency as u64) });
            let o = Box::pin(run_parallel(st, client, api_key, parent_model, group, &group_args, depth, cancel)).await;
            if !o.ok {
                return Outcome::err(format!("chain step {step_i} (parallel group) failed: {}", o.output));
            }
            previous = o.output.clone();
            if let Some(as_name) = step.get("as").and_then(|v| v.as_str()) {
                outputs.insert(as_name.to_string(), o.output.clone());
            }
            continue;
        }

        let an = step.get("agent").and_then(|v| v.as_str()).unwrap_or("");
        let agent = match find_agent(&agents, an) {
            Some(a) => a.clone(),
            None => return Outcome::err(format!("chain step {step_i}: unknown agent '{an}'")),
        };
        let task_tmpl = step.get("task").and_then(|v| v.as_str()).unwrap_or("{previous}");
        let task = render_task(task_tmpl, &previous, &outputs, &chain_dir);
        let model_override = step.get("model").and_then(|v| v.as_str()).map(String::from);
        let context = match args.get("context").and_then(|v| v.as_str()) {
            Some("fork") => ContextKind::Fork,
            Some("fresh") => ContextKind::Fresh,
            _ => agent.default_context.clone().unwrap_or(ContextKind::Fresh),
        };
        let step_id = format!("{run_id}-{step_i}");
        emit(&Event::new("info").with("message", json!(format!("chain step {step_i}+1: {} — {}", agent.name, truncate(&task, 80)))));
        let o = run_single(st, client, api_key, parent_model, &agent, &task, &step_id, model_override, context, depth, cancel).await;
        if !o.ok {
            return Outcome::err(format!("chain step {step_i} ({}) failed: {}", agent.name, o.output));
        }
        previous = o.output.clone();
        if let Some(as_name) = step.get("as").and_then(|v| v.as_str()) {
            outputs.insert(as_name.to_string(), o.output.clone());
        }
    }

    let mut runs = st.subagent_runs.lock().await;
    if let Some(r) = runs.get_mut(&run_id) {
        r.state = "completed".into();
        r.ended_at = Some(now_ms());
    }
    Outcome::ok(previous)
}

/// Render a chain task template, substituting {previous}, {outputs.name},
/// {task}, {chain_dir}.
fn render_task(tmpl: &str, previous: &str, outputs: &HashMap<String, String>, chain_dir: &std::path::Path) -> String {
    let mut out = tmpl.replace("{previous}", previous);
    out = out.replace("{chain_dir}", &chain_dir.display().to_string());
    // {outputs.name}
    let mut start = 0;
    let mut res = String::new();
    while let Some(i) = out[start..].find("{outputs.") {
        res.push_str(&out[start..start + i]);
        let rest = &out[start + i + 9..]; // after "{outputs."
        if let Some(end) = rest.find('}') {
            let name = &rest[..end];
            res.push_str(outputs.get(name).map(|s| s.as_str()).unwrap_or(""));
            start = start + i + 9 + end + 1;
        } else {
            res.push_str(&out[start + i..]);
            start = out.len();
            break;
        }
    }
    res.push_str(&out[start..]);
    res
}

// ---------------------------------------------------------------------------
// Management / control actions
// ---------------------------------------------------------------------------

async fn handle_action(
    action: &str,
    args: &Value,
    workspace: &std::path::Path,
    cfg: &Config,
    st: &Arc<State>,
    cancel: &CancellationToken,
) -> Outcome {
    match action {
        "list" => {
            let agents = discover_agents(workspace, &cfg.subagents);
            let lines: Vec<String> = agents.iter().map(|a| {
                format!("- {} [{}] — {}", a.name, source_label(&a.source), a.description)
            }).collect();
            Outcome::ok(format!("{} agent(s):\n{}", agents.len(), lines.join("\n")))
        }
        "get" => {
            let agents = discover_agents(workspace, &cfg.subagents);
            let name = args.get("agent").and_then(|v| v.as_str()).unwrap_or("");
            match find_agent(&agents, name) {
                Some(a) => Outcome::ok(json!(a).to_string()),
                None => Outcome::err(format!("unknown agent '{name}'")),
            }
        }
        "models" => {
            let agents = discover_agents(workspace, &cfg.subagents);
            let models = st.models.read().await;
            let default_model = cfg.default_model.clone().or_else(|| models.first().map(|m| m.id.clone())).unwrap_or_default();
            let name = args.get("agent").and_then(|v| v.as_str());
            let lines: Vec<String> = agents.iter().filter(|a| name.map(|n| a.name == n || a.name == n.rsplit('.').next().unwrap_or(n)).unwrap_or(true)).map(|a| {
                let m = a.model.clone().unwrap_or_else(|| default_model.clone());
                format!("- {}: {} (fallback: {})", a.name, m, a.fallback_models.join(", "))
            }).collect();
            Outcome::ok(lines.join("\n"))
        }
        "create" => create_agent(args, workspace),
        "update" => update_agent(args, workspace),
        "delete" => delete_agent(args, workspace),
        "status" => status_action(args, st).await,
        "interrupt" => interrupt_action(args, st).await,
        "resume" => resume_action(args, st, cancel).await,
        "doctor" => doctor_action(workspace, cfg, st).await,
        other => Outcome::err(format!("unknown action '{other}'; use list|get|create|update|delete|status|interrupt|resume|doctor")),
    }
}

fn source_label(s: &AgentSource) -> &'static str {
    match s { AgentSource::Builtin => "builtin", AgentSource::User => "user", AgentSource::Project => "project" }
}

fn create_agent(args: &Value, workspace: &std::path::Path) -> Outcome {
    let cfg = match args.get("config") {
        Some(v) => v,
        None => return Outcome::err("create requires 'config' with name + systemPrompt"),
    };
    let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() { return Outcome::err("create config requires 'name'"); }
    let scope = cfg.get("scope").and_then(|v| v.as_str()).unwrap_or("project");
    let dir = match scope {
        "user" => crate::config::home_dir().map(|h| h.join(".umans-harness/agents")).unwrap_or_else(|| workspace.join(".umans-harness/agents")),
        _ => workspace.join(".umans-harness/agents"),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Outcome::err(format!("create mkdir failed: {e}"));
    }
    let path = dir.join(format!("{name}.md"));
    let body = cfg.get("systemPrompt").and_then(|v| v.as_str()).unwrap_or("");
    let mut fm = format!("---\nname: {name}\n");
    if let Some(d) = cfg.get("description").and_then(|v| v.as_str()) { fm.push_str(&format!("description: {d}\n")); }
    if let Some(t) = cfg.get("tools").and_then(|v| v.as_str()) { fm.push_str(&format!("tools: {t}\n")); }
    if let Some(m) = cfg.get("model").and_then(|v| v.as_str()) { fm.push_str(&format!("model: {m}\n")); }
    if let Some(t) = cfg.get("thinking").and_then(|v| v.as_str()) { fm.push_str(&format!("thinking: {t}\n")); }
    let mode = cfg.get("systemPromptMode").and_then(|v| v.as_str()).unwrap_or("replace");
    fm.push_str(&format!("systemPromptMode: {mode}\n"));
    fm.push_str("---\n\n");
    fm.push_str(body);
    match std::fs::write(&path, fm) {
        Ok(_) => Outcome::ok(format!("created agent '{name}' at {}", path.display())),
        Err(e) => Outcome::err(format!("create write failed: {e}")),
    }
}

fn update_agent(args: &Value, workspace: &std::path::Path) -> Outcome {
    let name = args.get("agent").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() { return Outcome::err("update requires 'agent'"); }
    // find the file
    let candidates = [
        workspace.join(".umans-harness/agents").join(format!("{name}.md")),
        crate::config::home_dir().map(|h| h.join(format!(".umans-harness/agents/{name}.md"))).unwrap_or_default(),
    ];
    let path = candidates.iter().find(|p| p.exists()).cloned();
    let path = match path {
        Some(p) => p,
        None => return Outcome::err(format!("agent '{name}' not found to update; create it first")),
    };
    let cfg = match args.get("config") {
        Some(v) => v,
        None => return Outcome::err("update requires 'config'"),
    };
    let (mut fm, body) = parse_frontmatter(&std::fs::read_to_string(&path).unwrap_or_default());
    if let Some(m) = cfg.get("model").and_then(|v| v.as_str()) { fm.insert("model".into(), m.into()); }
    if let Some(t) = cfg.get("thinking").and_then(|v| v.as_str()) { fm.insert("thinking".into(), t.into()); }
    if let Some(d) = cfg.get("description").and_then(|v| v.as_str()) { fm.insert("description".into(), d.into()); }
    if let Some(t) = cfg.get("tools").and_then(|v| v.as_str()) { fm.insert("tools".into(), t.into()); }
    if let Some(b) = cfg.get("systemPrompt").and_then(|v| v.as_str()) {
        let out = format!("---\n{}\n---\n\n{}", fm.iter().map(|(k,v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"), b);
        let _ = std::fs::write(&path, out);
        return Outcome::ok(format!("updated agent '{name}'"));
    }
    let out = format!("---\n{}\n---\n\n{}", fm.iter().map(|(k,v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"), body);
    let _ = std::fs::write(&path, out);
    Outcome::ok(format!("updated agent '{name}'"))
}

fn delete_agent(args: &Value, workspace: &std::path::Path) -> Outcome {
    let name = args.get("agent").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() { return Outcome::err("delete requires 'agent'"); }
    let candidates = [
        workspace.join(".umans-harness/agents").join(format!("{name}.md")),
        crate::config::home_dir().map(|h| h.join(format!(".umans-harness/agents/{name}.md"))).unwrap_or_default(),
    ];
    for p in &candidates {
        if p.exists() {
            if let Err(e) = std::fs::remove_file(p) {
                return Outcome::err(format!("delete failed: {e}"));
            }
            return Outcome::ok(format!("deleted agent '{name}'"));
        }
    }
    Outcome::err(format!("agent '{name}' not found (builtins cannot be deleted; override with disabled:true)"))
}

async fn status_action(args: &Value, st: &Arc<State>) -> Outcome {
    let runs = st.subagent_runs.lock().await;
    let id = args.get("id").and_then(|v| v.as_str());
    if let Some(id) = id {
        if let Some(r) = find_run_prefix(&runs, id) {
            return Outcome::ok(format_run(&r));
        }
        return Outcome::err(format!("no run matching '{id}'"));
    }
    if runs.is_empty() {
        return Outcome::ok("no subagent runs");
    }
    let lines: Vec<String> = runs.values().map(format_run).collect();
    Outcome::ok(lines.join("\n"))
}

async fn interrupt_action(args: &Value, st: &Arc<State>) -> Outcome {
    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let mut runs = st.subagent_runs.lock().await;
    let r = match find_run_prefix_mut(&mut runs, id) {
        Some(r) => r,
        None => return Outcome::err(format!("no run matching '{id}'")),
    };
    if let Some(c) = r.cancel.clone() {
        c.cancel();
        r.state = "paused".into();
        Outcome::ok(format!("interrupted run {}", r.id))
    } else {
        Outcome::err(format!("run {} has no cancel handle", r.id))
    }
}

async fn resume_action(args: &Value, st: &Arc<State>, _cancel: &CancellationToken) -> Outcome {
    // Resume is acknowledged: a follow-up message is delivered to the child's
    // intercom target if it is still registered, otherwise we report that the
    // run has completed and suggest a new run.
    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let runs = st.subagent_runs.lock().await;
    let r = match find_run_prefix(&runs, id) {
        Some(r) => r.clone(),
        None => return Outcome::err(format!("no run matching '{id}'")),
    };
    if let Some(target) = &r.intercom_target {
        if st.intercom.targets().iter().any(|t| t == target) {
            let msg = crate::intercom::IntercomMessage {
                id: format!("resume-{}", now_ms()),
                from: st.intercom.orchestrator_target(),
                to: target.clone(),
                message: message.to_string(),
                reason: "resume".into(),
                ts: now_ms(),
                ask_id: String::new(),
            };
            let _ = st.intercom.post(msg);
            return Outcome::ok(format!("resume message delivered to {target}"));
        }
    }
    Outcome::ok(format!("run {} is no longer live; start a new run to continue", r.id))
}

fn find_run_prefix<'a>(runs: &'a HashMap<String, SubagentRun>, id: &str) -> Option<&'a SubagentRun> {
    runs.get(id).or_else(|| runs.values().find(|r| r.id.starts_with(id) || id.starts_with(&r.id)))
}

fn find_run_prefix_mut<'a>(runs: &'a mut HashMap<String, SubagentRun>, id: &str) -> Option<&'a mut SubagentRun> {
    if runs.contains_key(id) {
        return runs.get_mut(id);
    }
    let key = runs.values().find(|r| r.id.starts_with(id) || id.starts_with(&r.id)).map(|r| r.id.clone())?;
    runs.get_mut(&key)
}

fn format_run(r: &SubagentRun) -> String {
    let dur = r.ended_at.map(|e| e.saturating_sub(r.started_at) / 1000).unwrap_or(0);
    format!("[{}] {} ({}) — {} — {}s — target: {}", r.state, r.id, r.mode, r.agents.join(","), dur, r.intercom_target.clone().unwrap_or("-".into()))
}

async fn doctor_action(workspace: &std::path::Path, cfg: &Config, st: &Arc<State>) -> Outcome {
    let agents = discover_agents(workspace, &cfg.subagents);
    let mut lines = Vec::new();
    lines.push(format!("agents discovered: {}", agents.len()));
    lines.push(format!("max subagent depth: {}", resolve_max_depth(&cfg.subagents)));
    lines.push(format!("intercom bridge mode: {}", cfg.subagents.intercom_bridge_mode.as_str()));
    lines.push(format!("intercom known targets: {}", st.intercom.targets().join(", ")));
    lines.push(format!("intercom pending asks: {}", st.intercom.pending_count()));
    lines.push(format!("parallel: maxTasks={}, concurrency={}", cfg.subagents.parallel_max_tasks, cfg.subagents.parallel_concurrency));
    let runs = st.subagent_runs.lock().await;
    lines.push(format!("tracked runs: {}", runs.len()));
    for r in runs.values() {
        lines.push(format!("  {}", format_run(r)));
    }
    Outcome::ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parses() {
        let (fm, body) = parse_frontmatter("---\nname: scout\ntools: read, grep\n---\n\nYou are a scout.");
        assert_eq!(fm.get("name").unwrap(), "scout");
        assert_eq!(fm.get("tools").unwrap(), "read, grep");
        assert!(body.starts_with("You are a scout."));
    }

    #[test]
    fn builtin_agents_present() {
        let v = builtin_agents();
        let names: Vec<&str> = v.iter().map(|a| a.name.as_str()).collect();
        for n in ["scout", "researcher", "planner", "worker", "reviewer", "context-builder", "oracle", "delegate"] {
            assert!(names.contains(&n), "missing builtin {n}");
        }
    }

    #[test]
    fn tool_normalization() {
        assert_eq!(AgentConfig::normalize_tool("read"), "read_file");
        assert_eq!(AgentConfig::normalize_tool("find"), "glob");
        assert_eq!(AgentConfig::normalize_tool("ls"), "list_dir");
        assert_eq!(AgentConfig::normalize_tool("bash"), "bash");
    }

    #[test]
    fn render_task_substitutes() {
        let mut outputs = HashMap::new();
        outputs.insert("context".into(), "CTX".into());
        let r = render_task("plan from {outputs.context} and {previous}", "PREV", &outputs, std::path::Path::new("/tmp"));
        assert_eq!(r, "plan from CTX and PREV");
    }

    #[test]
    fn target_is_stable() {
        let t = subagent_target("run-1", "worker", Some(0));
        assert!(t.starts_with("subagent-worker-"));
    }

    #[test]
    fn depth_guard_clamps() {
        assert_eq!(child_max_depth(2, Some(1)), 1);
        assert_eq!(child_max_depth(2, None), 2);
    }
}
