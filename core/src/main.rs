// umans-harness-core: stdio JSON-RPC server. The TUI spawns this binary,
// writes commands to stdin, and reads newline-delimited events from stdout.
//
// Several core functions (stream_turn, run_turn, dispatch_*) intentionally
// carry many positional args (the seam between the request loop and the tool
// layer); refactoring each into a context struct is a larger change, so allow
// the lint here rather than obscure the call sites.
#![allow(clippy::too_many_arguments)]

mod config;
mod fetch_tool;
mod git_ctx;
mod intercom;
mod logging;
mod memory;
mod plugins;
mod protocol;
mod provider;
mod session;
mod staging;
mod subagent;
mod tools;
mod vision;
mod workspace;

use config::{Approval, Config, PermissionRule, ResolvedProvider};
use git_ctx::{git_context_injection, read_git_context};
use intercom::IntercomBus;
use logging::{estimate_message_tokens, estimate_messages_tokens, Logger, TurnTimer};
use memory::memory_injection;
use plugins::{PluginManager, PLUGIN_DOCS};
use protocol::{emit, Command, Event, ModelInfo};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use vision::VisionConfig;

use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;

#[derive(Clone)]
pub struct QueuedPrompt {
    prompt: String,
    model: String,
    effort: String,
}

/// A pending approval request the TUI must answer before the tool runs.
const SYSTEM_PROMPT_BASE: &str = r#"You are a coding agent operating inside a Rust/Go harness with native Umans model access.
You can read, edit, write, and list files, search with grep/glob, and run bash commands — all confined to the current workspace directory.

File editing uses search-and-replace, not line numbers or hashes:
- read_file returns a file's plain content. Call it before editing so you see the exact text.
- To change a file, call edit with one or more {search, replace} pairs. `search` must match the file content EXACTLY (copy it verbatim, including whitespace) and be unique in the file; `replace` is the new text (empty string deletes the search text). To insert lines, search for a unique anchor line and put it back plus the new lines in `replace`. All edits in one call apply atomically — if any `search` is not found or is ambiguous (matches multiple places) nothing is written; re-read and correct the search text.
- Use write_file only for new files or complete rewrites; prefer edit for targeted changes. Use grep to search and glob to find files by pattern.

Tool-call hygiene — keep tool arguments small and valid JSON:
- Call the dedicated tool directly (bash, read_file, edit). Use `bulk` only to batch several genuinely independent calls — never wrap a single bash command in `bulk`.
- Keep each bash `command` short. For loops, nested quotes, long `&&` chains, or multi-line logic, write a script to a file with `write_file` and run `bash script.sh` instead of inlining one long command string. Long, quote-heavy commands nested inside `bulk`'s JSON are the most common cause of malformed tool calls: the model botches the escaping, the call fails, and the broken message can then poison the whole conversation.

All paths are relative to the workspace root; absolute paths and ".." are rejected.
Work step by step: read/search before changing, make the smallest correct change, then verify with a command.
Be concise. Prefer standard tools. When done, summarize what you did in two lines.

Self-learning — you compound knowledge across sessions, so future you starts smarter:
- The `memory` tool (actions: save/append/list/forget) persists durable facts scoped to this workspace. Saved memories are injected into your standing system prompt on every future session, so anything worth remembering does not need rediscovering. Use `save` for a new note and `append` to accumulate facts onto an existing one without clobbering it.
- Before signaling done on a non-trivial task, take one reflection step: what convention, architecture fact, decision, or gotcha did you learn that future sessions should not have to rediscover? Persist only durable, reusable facts via `memory` (append if the topic already exists, else save). Do not persist transient task state, one-off details, or trivia.
- Reusable skills live as markdown + YAML frontmatter under `.umans-harness/skills/<name>/SKILL.md`. Discover them with `list_dir .umans-harness/skills/` and read the relevant SKILL.md before applying it. When you solve the same shape of problem more than twice, write a skill there with `write_file` (frontmatter: name/description; body: when-to-use, steps, examples). The pi-subagents skill is already injected for you; others are opt-in.
- `/index` bootstraps knowledge on an unfamiliar repo (walk the structure, write memories + candidate skills); `/reflect` runs a deliberate end-of-task learning pass. Use them when handed a large unfamiliar codebase or when you want to lock in what a task taught you."#;

/// Build the full system prompt by appending git context, memory context,
/// and the plugin self-bootstrapping docs.
pub fn build_system_prompt(workspace: &std::path::Path, with_skill: bool) -> String {
    let mut prompt = SYSTEM_PROMPT_BASE.to_string();
    if let Some(git) = read_git_context(workspace) {
        prompt.push_str("\n\n");
        prompt.push_str(&git_context_injection(&git));
    }
    let mem = memory_injection(workspace, "");
    if !mem.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&mem);
    }
    prompt.push_str("\n\n");
    prompt.push_str(PLUGIN_DOCS);
    // Inject the pi-subagents orchestrator skill so the parent agent knows how
    // to delegate via the `subagent` tool and how intercom coordination works.
    // This is parent-only: subagents never receive it (they'd wrongly think
    // they are the orchestrator).
    if with_skill {
        if let Some(skill) = subagent_orchestrator_skill(workspace) {
            prompt.push_str("\n\n");
            prompt.push_str(&skill);
        }
    }
    prompt
}

/// Load the bundled pi-subagents SKILL.md (project then user scope) for the
/// orchestrator's system prompt. Returns None if no skill file is found.
fn subagent_orchestrator_skill(workspace: &std::path::Path) -> Option<String> {
    let candidates: [Option<std::path::PathBuf>; 2] = [
        Some(workspace.join(".umans-harness/skills/pi-subagents/SKILL.md")),
        config::home_dir().map(|h| h.join(".umans-harness/skills/pi-subagents/SKILL.md")),
    ];
    for p in candidates.into_iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(&p) {
            let (_fm, body) = subagent::parse_frontmatter(&content);
            return Some(format!("# Skill: pi-subagents\n\n{body}"));
        }
    }
    None
}

/// A pending approval request the TUI must answer before the tool runs.
#[allow(dead_code)]
pub struct PendingApproval {
    request_id: String,
    tool: String,
    args: Value,
    notify: Arc<Notify>,
    granted: Mutex<Option<bool>>, // Some(true)=approved, Some(false)=denied, None=awaiting
    escalated: Mutex<bool>,       // "always" was chosen → upgrade session mode
}

pub struct State {
    pub cfg: RwLock<Config>,
    /// Per-provider runtime API keys (set via `set_key {provider,api_key}`).
    /// Keyed by provider name; the active provider's key (if present) wins over
    /// config literals/env vars during resolution. The "default" slot holds the
    /// legacy single key when no providers are configured.
    pub api_keys: RwLock<HashMap<String, String>>,
    /// Runtime override of the active provider name (set via `set_provider`).
    /// Wins over `cfg.active_provider`; None => use config's active provider.
    pub active_provider: RwLock<Option<String>>,
    pub conversation: Mutex<Vec<Value>>,
    pub models: RwLock<Vec<ModelInfo>>,
    pub current: Mutex<Option<CancellationToken>>,
    pub handle: Mutex<Option<JoinHandle<()>>>,
    /// Pending approval requests keyed by their unique approval id (see
    /// APPROVAL_SEQ) so parallel subagents can't clobber each other's request.
    pub pending: Mutex<std::collections::HashMap<String, Arc<PendingApproval>>>,
    pub logger: Logger,
    /// Token counts accumulated across the session (for the status bar).
    pub tokens_in: Mutex<u64>,
    pub tokens_out: Mutex<u64>,
    /// Cumulative prefix-cache hits across the session (from
    /// usage.prompt_tokens_details.cached_tokens). Surfaces whether the
    /// stable-prefix strategy is actually landing cache hits.
    pub cached_tokens: Mutex<u64>,
    /// Tool kinds ("destructive"/"readonly") the user said "always" to,
    /// so subsequent calls of that kind skip the gate without escalating all.
    pub escalated_kinds: Mutex<std::collections::HashSet<&'static str>>,
    /// Prompt queued while a turn was running (one-deep buffer).
    pub queued: Mutex<Option<QueuedPrompt>>,
    /// Plugin manager — scans, loads, and executes hooks.
    pub plugin_manager: PluginManager,
    /// Vision-handoff config (curated vision models + preferred target), persisted
    /// to .umans-harness/vision.json; merged into the pre_turn hook context.
    pub vision: RwLock<VisionConfig>,
    /// Last time a turn completed (for idle compaction).
    pub last_turn_time: Mutex<std::time::Instant>,
    /// Incrementally maintained token estimate for the main conversation,
    /// updated on every push + recalculated after compaction.
    pub estimated_tokens: Mutex<u64>,
    /// True after compaction until the next sanitization pass.
    pub needs_sanitize: Mutex<bool>,
    /// Intercom bus: in-process mailboxes for subagent ↔ orchestrator and
    /// subagent ↔ subagent coordination.
    pub intercom: IntercomBus,
    /// Tracked subagent runs for status/interrupt/resume (keyed by run id).
    pub subagent_runs: Mutex<std::collections::HashMap<String, subagent::SubagentRun>>,
}

impl State {
    /// Resolve the active provider for an API call: kind, base URL, effective
    /// API key (runtime override -> config literal -> config env var -> global
    /// env), and extra headers. Combines the config snapshot with the runtime
    /// active-provider override and per-provider keys. This is the single
    /// source of truth every provider call site uses, so switching providers
    /// (or setting a key) takes effect on the next call with no other wiring.
    pub async fn resolved_provider(&self) -> ResolvedProvider {
        let cfg = self.cfg.read().await;
        let active = self.active_provider.read().await.clone();
        let keys = self.api_keys.read().await.clone();
        cfg.resolve_provider_with(&keys, active.as_deref())
    }
}

/// Generate a unique filename for a new session file. std has no date
/// formatting; the picker shows the derived title, not the filename, so a
/// monotonic nanos id is fine. Used only as a fallback when the TUI does not
/// supply a name.
fn new_session_filename() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.jsonl", now.as_nanos())
}

#[tokio::main]
async fn main() {
    // Stage the harness's global defaults (agents, orchestrator skill,
    // vision-handoff plugin) into ~/.umans-harness/ on first run — shared
    // across every project, editable once, never per-project by default. Done
    // before config/plugin loading so staged files are picked up this run.
    let stage = staging::stage_if_needed();
    if stage.first_run {
        eprintln!(
            "[staging] first run: staged {} default file(s) into {}",
            stage.written.len(),
            stage.home.display()
        );
    }
    let cfg = config::load();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("client");

    // Discover models up front for the active provider (live endpoint, snapshot
    // fallback). At init there are no runtime keys yet, so resolve from config.
    let init_provider = cfg.resolve_provider(&HashMap::new());
    let models = provider::discover_models(&client, &init_provider).await;
    let logger = Logger::new(cfg.debug_log.as_deref());
    logger.log("init", json!({ "workspace": cfg.workspace.display().to_string(), "provider": init_provider.name, "kind": init_provider.kind.as_str(), "base_url": init_provider.base_url, "approval": cfg.approval.as_str() }));

    // Resume session if configured and present. A future-version session file
    // returns Err (surfaced to the user via an `error` event at Init) rather
    // than silently starting blank.
    let (resumed, session_error): (Vec<Value>, Option<String>) = match cfg.session_file.as_ref() {
        Some(p) => match session::load(p.as_path()) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(e)),
        },
        None => (Vec::new(), None),
    };
    // Persisted "always" approval escalations travel with the session file
    // (sidecar <session>.escalations) so a restart doesn't un-gate kinds the
    // user already approved.
    let init_escalations: HashSet<&'static str> = cfg
        .session_file
        .as_ref()
        .map(|p| session::load_escalations(p.as_path()))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| match s.as_str() {
            "destructive" => Some("destructive"),
            "readonly" => Some("readonly"),
            _ => None,
        })
        .collect();
    // Pre-clone values State::new needs before `cfg` is moved into the lock.
    let plugin_dir = cfg.plugin_dir.clone();
    let pm_workspace = cfg.workspace.clone();
    let trust_project = cfg.trust_project_plugins;

    // Ensure the session file exists (header only) so the active session is
    // always listed by `list_sessions`, even before the first message lands.
    if let Some(p) = cfg.session_file.as_ref() {
        session::ensure(p.as_path());
    }

    // Pre-compute token estimate for resumed conversation.
    let init_est = estimate_messages_tokens(&resumed);
    // Sanitize a resumed conversation before its first request: a prior crash
    // may have left a malformed tool-call `arguments` in the history, which the
    // API would reject with "function.arguments must be valid JSON".
    let sanitize_on_resume = !resumed.is_empty();

    let vision_cfg = VisionConfig::load(&cfg.workspace);
    let state = Arc::new(State {
        cfg: RwLock::new(cfg),
        api_keys: RwLock::new(HashMap::new()),
        active_provider: RwLock::new(None),
        conversation: Mutex::new(resumed),
        models: RwLock::new(models),
        current: Mutex::new(None),
        handle: Mutex::new(None),
        pending: Mutex::new(std::collections::HashMap::new()),
        logger,
        tokens_in: Mutex::new(0),
        tokens_out: Mutex::new(0),
        cached_tokens: Mutex::new(0),
        escalated_kinds: Mutex::new(init_escalations),
        queued: Mutex::new(None),
        plugin_manager: PluginManager::new_with_global_plugins(plugin_dir, pm_workspace, trust_project),
        vision: RwLock::new(vision_cfg),
        last_turn_time: Mutex::new(std::time::Instant::now()),
        estimated_tokens: Mutex::new(init_est),
        needs_sanitize: Mutex::new(sanitize_on_resume),
        intercom: IntercomBus::new(),
        subagent_runs: Mutex::new(std::collections::HashMap::new()),
    });

    // Apply disabled plugin list from config.
    {
        let cfg = state.cfg.read().await;
        for name in &cfg.plugins_disabled {
            let _ = state.plugin_manager.disable(name);
        }
    }

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let cmd = match serde_json::from_str::<Command>(&line) {
            Ok(c) => c,
            Err(e) => {
                emit(&Event::new("error").with("message", json!(format!("bad command: {e}"))));
                continue;
            }
        };
        match cmd {
            Command::Init => {
                let models = state.models.read().await.clone();
                let rp = state.resolved_provider().await;
                let authed = rp.api_key.is_some();
                let cfg = state.cfg.read().await;
                let conv_len = state.conversation.lock().await.len();
                emit(
                    &Event::new("ready")
                        .with("models", json!(models))
                        .with("authed", json!(authed))
                        .with("workspace", json!(cfg.workspace.display().to_string()))
                        .with("approval", json!(cfg.approval.as_str()))
                        .with("base_url", json!(rp.base_url))
                        .with("provider", json!(rp.name))
                        .with("providerKind", json!(rp.kind.as_str()))
                        .with("providers", json!(cfg.provider_names()))
                        .with("bash_timeout_secs", json!(cfg.bash_timeout_secs))
                        .with("resumed_messages", json!(conv_len)),
                );
                // Tell the user when the harness staged its global defaults
                // (first run) so the global ~/.umans-harness/ layout is
                // discoverable.
                if stage.first_run {
                    emit(
                        &Event::new("info").with(
                            "message",
                            json!(format!(
                                "First run: staged {} default file(s) into {} — agents, the pi-subagents skill, and the vision-handoff plugin now live globally and are shared across all projects. Edit them there to customize; drop a file in a project's own .umans-harness/ to override for that project only.",
                                stage.written.len(),
                                stage.home.display()
                            )),
                        ),
                    );
                }
                // Surface a future-version session-load error to the user.
                if let Some(e) = session_error.as_ref() {
                    emit(&Event::new("error").with("message", json!(e)));
                }
                // Replay any resumed conversation so the TUI shows prior history
                // on launch instead of starting from an empty transcript.
                if conv_len > 0 {
                    let conv = state.conversation.lock().await;
                    let visible: Vec<&Value> = conv
                        .iter()
                        .filter(|m| m.get("role").and_then(|v| v.as_str()) != Some("system"))
                        .collect();
                    let est = estimate_messages_tokens(&conv);
                    emit(
                        &Event::new("history")
                            .with("messages", json!(visible))
                            .with("tokens_in", json!(est)),
                    );
                }
            }
            Command::SetKey { api_key, provider } => {
                // Apply the key to a named provider, or to the active provider
                // when no name is given (backward-compatible with the pre-provider
                // single-key flow, which lands in the "default" slot).
                let name = match provider {
                    Some(p) => p,
                    None => state.resolved_provider().await.name,
                };
                state.api_keys.write().await.insert(name.clone(), api_key);
                state.logger.log("set_key", json!({ "provider": name }));
                emit(&Event::new("authed").with("ok", json!(true)).with("provider", json!(name)));
            }
            Command::SetProvider { name } => {
                // Switch the active provider at runtime, then re-discover models
                // for the new endpoint. Unknown names are ignored (stays put).
                {
                    let cfg = state.cfg.read().await;
                    if cfg.find_provider(&name).is_none() {
                        emit(
                            &Event::new("error").with(
                                "message",
                                json!(format!("unknown provider '{name}'; not switching")),
                            ),
                        );
                        return;
                    }
                }
                *state.active_provider.write().await = Some(name.clone());
                let rp = state.resolved_provider().await;
                // Re-discover models for the new provider (bypass cache freshness
                // by relying on the 8h TTL; switching is rare so a cache hit is fine).
                let models = provider::discover_models(&client, &rp).await;
                *state.models.write().await = models.clone();
                state.logger.log(
                    "set_provider",
                    json!({ "provider": rp.name, "kind": rp.kind.as_str(), "base_url": rp.base_url }),
                );
                emit(
                    &Event::new("provider_changed")
                        .with("provider", json!(rp.name))
                        .with("kind", json!(rp.kind.as_str()))
                        .with("base_url", json!(rp.base_url))
                        .with("has_key", json!(rp.api_key.is_some())),
                );
                emit(&Event::new("models").with("models", json!(models)));
            }
            Command::SetApproval { mode } => {
                let new = Approval::parse(&mode);
                state.cfg.write().await.approval = new.clone();
                state
                    .logger
                    .log("set_approval", json!({ "mode": new.as_str() }));
                emit(&Event::new("approval_changed").with("mode", json!(new.as_str())));
            }
            Command::SetConfig { key, value } => {
                // ponytail: minimal runtime knob setter for the two values the
                // TUI settings modal edits. Coerce string-or-number to u64.
                let as_u64 = |v: &Value| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                };
                let mut cfg = state.cfg.write().await;
                let out_key = key.clone();
                let mut out_val = value.clone();
                match key.as_str() {
                    "bash_timeout_secs" => {
                        if let Some(n) = as_u64(&value) {
                            cfg.bash_timeout_secs = n;
                            out_val = json!(n);
                        }
                    }
                    _ => {
                        drop(cfg);
                        emit(
                            &Event::new("error")
                                .with("message", json!(format!("unknown config key: {key}"))),
                        );
                        return;
                    }
                }
                state
                    .logger
                    .log("set_config", json!({ "key": out_key, "value": out_val }));
                drop(cfg);
                emit(
                    &Event::new("config_changed")
                        .with("key", json!(out_key))
                        .with("value", json!(out_val)),
                );
            }
            Command::Reset => {
                state.conversation.lock().await.clear();
                let cfg = state.cfg.read().await;
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &[]);
                }
                emit(&Event::new("reset"));
            }
            Command::Clear => {
                // In-memory only: keep the session file so a restart can still resume.
                state.conversation.lock().await.clear();
                emit(&Event::new("reset"));
            }
            Command::Undo => {
                // Drop the last turn: a user msg + everything after it (assistant, tool msgs).
                let mut conv = state.conversation.lock().await;
                // Walk back past trailing tool/assistant messages to the last user message.
                while let Some(last) = conv.last() {
                    let role = last.get("role").and_then(|v| v.as_str()).unwrap_or("");
                    if role == "user" {
                        conv.pop();
                        break;
                    }
                    conv.pop();
                }
                if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                    session::rewrite(p, &conv);
                }
                drop(conv);
                emit(&Event::new("reset")); // TUI clears blocks; core keeps the trimmed conv
            }
            Command::Compact => {
                // Force compaction now, then emit a compacted event.
                let mut messages = state.conversation.lock().await.clone();
                if messages.len() > 2 {
                    dispatch_lifecycle(&state, "pre_compact").await;
                    let before_est = estimate_messages_tokens(&messages);
                    compact_conversation(&mut messages, 200_000);
                    *state.conversation.lock().await = messages.clone();
                    let after_est = estimate_messages_tokens(&messages);
                    *state.estimated_tokens.lock().await = after_est;
                    *state.needs_sanitize.lock().await = true;
                    if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                        session::rewrite(p, &messages);
                    }
                    emit(
                        &Event::new("compacted")
                            .with("before_tokens", json!(before_est))
                            .with("after_tokens", json!(after_est)),
                    );
                } else {
                    emit(&Event::new("info").with("message", json!("nothing to compact yet")));
                }
            }
            Command::ListSessions => {
                let (dir, current_name) = {
                    let cfg = state.cfg.read().await;
                    let sf = cfg.session_file.as_ref();
                    let dir = sf
                        .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let cur = sf.and_then(|p| p.file_name()).map(|n| n.to_os_string());
                    (dir, cur)
                };
                let mut entries: Vec<Value> = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let path = e.path();
                        if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                            continue;
                        }
                        let name = e.file_name().to_string_lossy().to_string();
                        let info = session::describe(&path);
                        let mtime = e
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let current = current_name
                            .as_ref()
                            .map(|n| *n == e.file_name())
                            .unwrap_or(false);
                        let title = info
                            .title
                            .unwrap_or_else(|| "(no messages yet)".to_string());
                        entries.push(json!({
                            "name": name,
                            "path": path.display().to_string(),
                            "title": title,
                            "messages": info.messages,
                            "mtime": mtime,
                            "current": current,
                        }));
                    }
                }
                // Most recently modified first.
                entries.sort_by(|a, b| {
                    b["mtime"]
                        .as_u64()
                        .unwrap_or(0)
                        .cmp(&a["mtime"].as_u64().unwrap_or(0))
                });
                let files: Vec<String> = entries
                    .iter()
                    .filter_map(|e| e["name"].as_str().map(|s| s.to_string()))
                    .collect();
                emit(
                    &Event::new("sessions")
                        .with("sessions", json!(entries))
                        .with("files", json!(files)),
                );
            }
            Command::LoadSession { path } => {
                let mut p = std::path::PathBuf::from(&path);
                // Resolve relative paths against the sessions dir so the picker
                // (which may send a bare filename) works.
                if !p.is_absolute() {
                    if let Some(sess_dir) = state
                        .cfg
                        .read()
                        .await
                        .session_file
                        .as_ref()
                        .and_then(|sf| sf.parent())
                    {
                        p = sess_dir.join(&p);
                    }
                }
                let loaded = match session::load(&p) {
                    Ok(v) => v,
                    Err(e) => {
                        emit(&Event::new("error").with("message", json!(e)));
                        continue;
                    }
                };
                *state.conversation.lock().await = loaded.clone();
                // Point the session_file at the loaded path so future appends go there.
                state.cfg.write().await.session_file = Some(p);
                emit(&Event::new("reset"));
                // Replay the loaded transcript so the TUI shows prior turns
                // instead of an empty view after switching/resuming a session.
                let visible: Vec<&Value> = loaded
                    .iter()
                    .filter(|m| m.get("role").and_then(|v| v.as_str()) != Some("system"))
                    .collect();
                let est = estimate_messages_tokens(&loaded);
                *state.estimated_tokens.lock().await = est;
                emit(
                    &Event::new("history")
                        .with("messages", json!(visible))
                        .with("tokens_in", json!(est)),
                );
                emit(&Event::new("info").with(
                    "message",
                    json!(format!("loaded {} messages from {}", loaded.len(), path)),
                ));
            }
            Command::NewSession { path } => {
                // Start a fresh session file in the same project dir. The old
                // file is left on disk so sessions accumulate per project.
                let new_path = match path {
                    Some(name) => {
                        let mut p = std::path::PathBuf::from(name);
                        if !p.is_absolute() {
                            if let Some(sess_dir) = state
                                .cfg
                                .read()
                                .await
                                .session_file
                                .as_ref()
                                .and_then(|sf| sf.parent())
                            {
                                p = sess_dir.join(&p);
                            }
                        }
                        p
                    }
                    None => {
                        let dir = state
                            .cfg
                            .read()
                            .await
                            .session_file
                            .as_ref()
                            .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        dir.join(new_session_filename())
                    }
                };
                session::ensure(&new_path);
                *state.conversation.lock().await = Vec::new();
                state.cfg.write().await.session_file = Some(new_path.clone());
                state.logger.log(
                    "new_session",
                    json!({ "path": new_path.display().to_string() }),
                );
                emit(&Event::new("reset"));
                emit(&Event::new("info").with(
                    "message",
                    json!(format!("started new session: {}", new_path.display())),
                ));
            }
            Command::Stats => {
                let ti = *state.tokens_in.lock().await;
                let to = *state.tokens_out.lock().await;
                let cached = *state.cached_tokens.lock().await;
                let turns = state.logger.turn_count();
                let session_file = state
                    .cfg
                    .read()
                    .await
                    .session_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                emit(
                    &Event::new("stats")
                        .with("tokens_in", json!(ti))
                        .with("tokens_out", json!(to))
                        .with("tokens_total", json!(ti + to))
                        .with("cached_tokens", json!(cached))
                        .with("turns", json!(turns))
                        .with("messages", json!(state.conversation.lock().await.len()))
                        .with("session_file", json!(session_file)),
                );
            }
            Command::InstallPlugin { path } => {
                let dir = std::path::PathBuf::from(&path);
                match state.plugin_manager.install(&dir) {
                    Ok(plugin) => {
                        let hooks_list: Vec<String> = plugin.hooks.keys().cloned().collect();
                        emit(
                            &Event::new("plugin_installed")
                                .with("name", json!(plugin.name))
                                .with("version", json!(plugin.version))
                                .with("description", json!(plugin.description))
                                .with("hooks", json!(hooks_list))
                                .with("path", json!(plugin.source_path.display().to_string())),
                        );
                    }
                    Err(e) => {
                        emit(
                            &Event::new("plugin_error")
                                .with("name", json!(path))
                                .with("message", json!(e)),
                        );
                    }
                }
            }
            Command::RemovePlugin { name } => {
                let _ = state.plugin_manager.remove(&name);
                emit(&Event::new("plugin_removed").with("name", json!(name)));
            }
            Command::EnablePlugin { name } => {
                let _ = state.plugin_manager.enable(&name);
                emit(&Event::new("plugin_enabled").with("name", json!(name)));
            }
            Command::DisablePlugin { name } => {
                let _ = state.plugin_manager.disable(&name);
                emit(&Event::new("plugin_disabled").with("name", json!(name)));
            }
            Command::ListPlugins => {
                let plugins = state.plugin_manager.list();
                let entries: Vec<Value> = plugins
                    .values()
                    .map(|p| {
                        let hooks: Vec<String> = p.hooks.keys().cloned().collect();
                        json!({
                            "name": p.name,
                            "version": p.version,
                            "enabled": p.enabled,
                            "description": p.description,
                            "hooks": hooks,
                        })
                    })
                    .collect();
                emit(&Event::new("plugins_list").with("plugins", json!(entries)));
            }
            Command::GetVisionConfig => {
                let vc = state.vision.read().await.clone();
                let models = state.models.read().await.clone();
                let models_json: Vec<Value> = models
                    .iter()
                    .map(|m| {
                        json!({
                            "id": m.id.clone(), "vision": m.vision || vc.has_vision(m.id.as_str()),
                        })
                    })
                    .collect();
                emit(
                    &Event::new("vision_config")
                        .with("vision_models", json!(vc.vision_models.clone()))
                        .with("vision_model", json!(vc.vision_model.clone()))
                        .with("models", json!(models_json)),
                );
            }
            Command::SetVisionConfig {
                vision_models,
                vision_model,
            } => {
                let vc = VisionConfig {
                    vision_models,
                    vision_model: vision_model.filter(|s| !s.is_empty()),
                };
                let workspace = state.cfg.read().await.workspace.clone();
                vc.save(&workspace);
                *state.vision.write().await = vc.clone();
                let models = state.models.read().await.clone();
                let models_json: Vec<Value> = models
                    .iter()
                    .map(|m| {
                        json!({
                            "id": m.id.clone(), "vision": m.vision || vc.has_vision(m.id.as_str()),
                        })
                    })
                    .collect();
                emit(
                    &Event::new("vision_config")
                        .with("vision_models", json!(vc.vision_models.clone()))
                        .with("vision_model", json!(vc.vision_model.clone()))
                        .with("models", json!(models_json)),
                );
            }
            Command::RefreshMemory => {
                let msg = refresh_memory_injection(&state).await;
                emit(&Event::new("info").with("message", json!(msg)));
            }
            Command::SaveMemory { text, tags } => {
                if text.trim().is_empty() {
                    emit(&Event::new("error").with("message", json!("save_memory: 'text' must not be empty")));
                } else {
                    // Derive a name from the text (first words + timestamp) so
                    // the slug/filename is unique and human-readable.
                    let name = {
                        let stem: String = text
                            .split_whitespace()
                            .take(5)
                            .collect::<Vec<_>>()
                            .join(" ");
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        format!("{stem} [{ts}]")
                    };
                    let mem_type = tags
                        .as_ref()
                        .and_then(|t| t.first().cloned())
                        .unwrap_or_else(|| "note".to_string());
                    let ws = state.cfg.read().await.workspace.clone();
                    match memory::save_memory(&ws, &name, &text, &mem_type, "") {
                        Ok(p) => {
                            let id = p
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            // Refresh the injection so the next turn sees the new memory.
                            let _ = refresh_memory_injection(&state).await;
                            emit(
                                &Event::new("memory_saved")
                                    .with("id", json!(id))
                                    .with("message", json!("memory saved")),
                            );
                        }
                        Err(e) => {
                            emit(&Event::new("error").with("message", json!(format!("save_memory failed: {e}"))));
                        }
                    }
                }
            }
            Command::ListMemory => {
                let ws = state.cfg.read().await.workspace.clone();
                let entries = memory::scan_memories(&ws);
                let arr: Vec<Value> = entries
                    .iter()
                    .map(|m| {
                        let id = m
                            .path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        json!({
                            "id": id,
                            "name": m.name,
                            "type": m.mem_type,
                            "description": m.description,
                            "content": m.content,
                            // Display fields consumed by the TUI's /memory list:
                            // `text` is the scannable label (the memory name),
                            // `tags` surfaces the type as a single tag.
                            "text": m.name,
                            "tags": [m.mem_type],
                        })
                    })
                    .collect();
                emit(
                    &Event::new("memory_list")
                        .with("entries", json!(arr))
                        .with("count", json!(arr.len())),
                );
            }
            Command::ForgetMemory { id } => {
                let ws = state.cfg.read().await.workspace.clone();
                match memory::forget_memory(&ws, &id) {
                    Ok(()) => {
                        let _ = refresh_memory_injection(&state).await;
                        emit(
                            &Event::new("memory_saved")
                                .with("message", json!(format!("forgot memory '{id}'"))),
                        );
                    }
                    Err(e) => {
                        emit(&Event::new("error").with("message", json!(format!("forget_memory failed: {e}"))));
                    }
                }
            }
            Command::Approve {
                request_id,
                decision,
            } => {
                // Look up by the unique approval id (the request_id the TUI
                // echoes back), not the tool-call id — concurrent approvals from
                // parallel subagents (which may each use `call_1`) can't resolve
                // to the wrong request.
                let p = state.pending.lock().await.get(&request_id).cloned();
                if let Some(p) = p {
                    match decision.as_str() {
                        "yes" => *p.granted.lock().await = Some(true),
                        "always" => {
                            *p.granted.lock().await = Some(true);
                            *p.escalated.lock().await = true;
                        }
                        _ => *p.granted.lock().await = Some(false),
                    }
                    p.notify.notify_one();
                }
            }
            Command::IntercomReply { request_id, reply } => {
                // The orchestrator (user, via the TUI) replies to a subagent's
                // contact_supervisor need_decision ask. Resolves the pending ask
                // so the awaiting subagent loop wakes and continues.
                let ok = state.intercom.resolve_ask(&request_id, &reply);
                if ok {
                    emit(&Event::new("info").with("message", json!("reply delivered to subagent")));
                } else {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!("no pending intercom ask for id {request_id}")),
                    ));
                }
            }
            Command::Abort => {
                // Cancel the running turn AND drop any queued follow-up/steer so a
                // single abort fully stops the loop (not just the current turn).
                *state.queued.lock().await = None;
                if let Some(tok) = state.current.lock().await.take() {
                    tok.cancel();
                }
            }
            Command::Send {
                prompt,
                model,
                reasoning_effort,
                images,
            } => {
                let st = state.clone();
                let client = client.clone();
                let models = st.models.read().await.clone();
                let valid = models.iter().any(|m| m.id == model);
                if !valid {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("unknown model: {model}"))),
                    );
                    continue;
                }
                let effort = reasoning_effort.unwrap_or_else(|| "medium".into());
                // If a turn is already running, buffer this prompt (one-deep) instead
                // of dropping it. It drains when the running turn emits `done`.
                let already = st.current.lock().await.is_some();
                if already {
                    let mut q = st.queued.lock().await;
                    if q.is_some() {
                        emit(&Event::new("error").with(
                            "message",
                            json!("a prompt is already queued; send abort first or wait"),
                        ));
                    } else {
                        *q = Some(QueuedPrompt {
                            prompt,
                            model,
                            effort,
                        });
                        emit(&Event::new("info").with(
                            "message",
                            json!("prompt queued; will run after the current turn"),
                        ));
                    }
                    continue;
                }
                let tok = CancellationToken::new();
                *st.current.lock().await = Some(tok.clone());
                let handle = tokio::spawn(run_turn_and_drain(
                    st.clone(),
                    client.clone(),
                    model,
                    prompt,
                    effort,
                    images,
                    tok,
                ));
                *st.handle.lock().await = Some(handle);
            }
            Command::Steer {
                prompt,
                model,
                reasoning_effort,
            } => {
                let st = state.clone();
                let client_c = client.clone();
                let models = st.models.read().await.clone();
                if !models.iter().any(|m| m.id == model) {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("unknown model: {model}"))),
                    );
                    continue;
                }
                let effort = reasoning_effort.unwrap_or_else(|| "medium".into());
                // Steer = interrupt the running turn and redirect it. Cancel the
                // in-flight token and set the steer as the next queued prompt
                // (superseding any queued follow-up); the run_turn drain then runs
                // it, so the `current` token hand-off stays clean. With nothing
                // in flight, steer degrades to a normal turn.
                emit(&Event::new("steer").with("prompt", json!(prompt)));
                if st.current.lock().await.is_some() {
                    *st.queued.lock().await = Some(QueuedPrompt {
                        prompt,
                        model,
                        effort,
                    });
                    if let Some(tok) = st.current.lock().await.take() {
                        tok.cancel();
                    }
                } else {
                    let tok = CancellationToken::new();
                    *st.current.lock().await = Some(tok.clone());
                    let handle = tokio::spawn(run_turn_and_drain(
                        st.clone(),
                        client_c,
                        model,
                        prompt,
                        effort,
                        None,
                        tok,
                    ));
                    *st.handle.lock().await = Some(handle);
                }
            }
        }
    }
    // stdin EOF: don't tear down the runtime while a turn is still running.
    let h = state.handle.lock().await.take();
    if let Some(h) = h {
        let _ = h.await;
    }
}

/// Check if a tool call matches a permission rule. Used by the approval gate
/// to skip prompting for allow-listed tools, or block deny-listed ones outright.
pub(crate) fn tool_matches_rule(tool_name: &str, args: &Value, rule: &PermissionRule) -> bool {
    if !rule.tool_name.eq_ignore_ascii_case(tool_name) && rule.tool_name != "*" {
        return false;
    }
    if rule.rule_content.is_empty() || rule.rule_content == "*" {
        return true;
    }
    // Rule content matching: check against tool args.
    // For bash: match against the command string.
    // For write_file/edit: match against the path.
    // For grep/glob: match against the search pattern.
    // For WebFetch: match against URL domain.
    // Use glob-style matching with * wildcards.
    let candidate = match tool_name {
        "bash" => args.get("command").and_then(|v| v.as_str()).unwrap_or(""),
        "write_file" | "edit" | "patch" | "read_file" | "bulk_read" | "bulk_write"
        | "bulk_edit" => args.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        "grep" => args.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
        "glob" => args.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
        _ => "",
    };
    if candidate.is_empty() {
        return false;
    }
    star_match_rule(&rule.rule_content, candidate)
}

fn star_match_rule(pattern: &str, text: &str) -> bool {
    // Simple glob: * matches any sequence, ? matches one char.
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            match p[i - 1] {
                '*' => dp[i][j] = dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i][j] = dp[i - 1][j - 1],
                c => dp[i][j] = dp[i - 1][j - 1] && c == t[j - 1],
            }
        }
    }
    dp[p.len()][t.len()]
}

/// Run one assistant turn, then drain a queued prompt (one-deep) into another
/// turn. Shared by `send` (idle start) and `steer` (idle fallback) so the
/// queue-drain logic and the `current` token hand-off live in one place. A
/// follow-up or steer queued while this turn ran is run next; the recursion is
/// via `tokio::spawn` so it never grows the stack. Boxed because recursive
/// async fns can't prove `Send` for `tokio::spawn` on their own.
fn run_turn_and_drain(
    st: Arc<State>,
    client: reqwest::Client,
    model: String,
    prompt: String,
    effort: String,
    images: Option<Vec<String>>,
    tok: CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        // Run the turn inside a panic guard: if run_turn panics (a bug or a
        // malformed model payload hitting an unwrap/index), we still clear
        // `current` and emit error+done so the TUI never wedges on a stuck
        // "working" footer with no turn actually running.
        let result = AssertUnwindSafe(run_turn(&st, &client, model, prompt, effort, images, tok))
            .catch_unwind()
            .await;
        // The turn ended for any reason — notify lifecycle plugins and release
        // the current-token slot unconditionally so new turns can start.
        dispatch_lifecycle(&st, "session_stop").await;
        st.current.lock().await.take();
        if let Err(_panic) = result {
            emit(&Event::new("error").with(
                "message",
                json!("turn terminated unexpectedly (panic); please retry"),
            ));
            emit(&Event::new("done"));
            return;
        }
        // Drain a queued prompt if one was buffered while we ran (follow-up/steer).
        if let Some(q) = st.queued.lock().await.take() {
            let tok2 = CancellationToken::new();
            *st.current.lock().await = Some(tok2.clone());
            tokio::spawn(run_turn_and_drain(
                st.clone(),
                client.clone(),
                q.model,
                q.prompt,
                q.effort,
                None,
                tok2,
            ));
        }
    })
}

/// Dispatch a lifecycle/session hook (session_start / session_stop /
/// pre_compact) to every enabled plugin that registered for it. Best-effort:
/// lifecycle hooks run for their side effects and never block the turn (their
/// `allow`/`deny` is ignored; a failing/timed-out/missing hook is skipped, not
/// fatal). This wires the hook points that were previously advertised in
/// HOOK_POINTS but never dispatched.
pub(crate) async fn dispatch_lifecycle(st: &Arc<State>, hook: &str) {
    let (workspace, session_id) = {
        let cfg = st.cfg.read().await;
        (
            cfg.workspace.display().to_string(),
            cfg.session_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        )
    };
    for (plugin_name, config) in &st.plugin_manager.get_hook_configs(hook) {
        let ctx = plugins::build_context(hook, "", &workspace, None, &session_id, config.pass_args);
        let _ = plugins::execute_hook(hook, plugin_name, config, &ctx).await;
    }
}

/// Token counts reported in the `metrics` event, which drive the footer's
/// context budget. The provider returns the *last request's* usage
/// (prompt/completion tokens ≈ the live context size) when the endpoint
/// includes it; when usage is absent these come back as zero, which would
/// pin the footer at "0%". Fall back to a char-based estimate of the current
/// conversation so the budget always reflects reality.
async fn reported_tokens(st: &Arc<State>, usage_in: u64, usage_out: u64) -> (u64, u64) {
    if usage_in > 0 || usage_out > 0 {
        return (usage_in, usage_out);
    }
    ({ *st.estimated_tokens.lock().await }, 0)
}

async fn run_turn(
    st: &Arc<State>,
    client: &reqwest::Client,
    model: String,
    prompt: String,
    effort: String,
    images: Option<Vec<String>>,
    cancel: CancellationToken,
) {
    // Lifecycle hook: notify plugins a session/turn is starting. Best-effort
    // and never blocks the turn.
    dispatch_lifecycle(st, "session_start").await;

    // Vision handoff (pre_turn) and other plugins may remap the model for
    // this turn; keep a mutable binding so a swap propagates to the request loop.
    let mut model = model;

    // Ensure system prompt is present; persist every finalized message to the session file.
    let mut init_est_add = 0u64;
    {
        let mut conv = st.conversation.lock().await;
        if conv.is_empty() {
            let workspace = st.cfg.read().await.workspace.clone();
            let sys_msg =
                json!({ "role": "system", "content": build_system_prompt(&workspace, true) });
            init_est_add += estimate_message_tokens(&sys_msg);
            conv.push(sys_msg);
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, &conv[0]);
            }
        }
        // Build the user message. If images are attached and vision is allowed,
        // emit a multimodal content array (text + image_url parts).
        let allow_vision = st.cfg.read().await.allow_vision;
        let user_msg = match (&images, allow_vision) {
            (Some(imgs), true) if !imgs.is_empty() => {
                let mut parts: Vec<Value> = vec![json!({ "type": "text", "text": prompt })];
                for img in imgs {
                    let url = image_to_data_url(img);
                    parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
                json!({ "role": "user", "content": parts })
            }
            _ => json!({ "role": "user", "content": prompt }),
        };
        init_est_add += estimate_message_tokens(&user_msg);
        conv.push(user_msg);
        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
            session::append(p, conv.last().unwrap());
        }
    }
    if init_est_add > 0 {
        *st.estimated_tokens.lock().await += init_est_add;
    }

    // Vision handoff (pre_turn hook): let plugins inspect the upcoming turn
    // (model + attached images) and optionally remap the model before the first
    // request. Advisory — a broken/missing hook or `allow:false` never blocks
    // the turn; only `modify.model` (validated against discovered models) is honored.
    {
        let has_images = images.as_ref().is_some_and(|v| !v.is_empty());
        let image_count = images.as_ref().map_or(0, |v| v.len());
        let vc = st.vision.read().await.clone();
        let models_json: Vec<Value> = st
            .models
            .read()
            .await
            .iter()
            .map(|m| {
                json!({
                    "id": m.id.clone(), "vision": m.vision || vc.has_vision(m.id.as_str()),
                })
            })
            .collect();
        let (workspace, session_id) = {
            let cfg = st.cfg.read().await;
            (
                cfg.workspace.display().to_string(),
                cfg.session_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
        };
        let original_model = model.clone();
        for (plugin_name, config) in &st.plugin_manager.get_hook_configs("pre_turn") {
            let turn_args = json!({
                "model": model.clone(),
                "has_images": has_images,
                "image_count": image_count,
                "models": models_json,
                "vision_model": vc.vision_model.clone(),
            });
            let ctx = plugins::build_context(
                "pre_turn",
                "",
                &workspace,
                Some(&turn_args),
                &session_id,
                config.pass_args,
            );
            let result = plugins::execute_hook("pre_turn", plugin_name, config, &ctx).await;
            if let Some(new_model) = result
                .modify
                .as_ref()
                .and_then(|m| m.get("model"))
                .and_then(|v| v.as_str())
            {
                if new_model != model.as_str() {
                    let valid = st
                        .models
                        .read()
                        .await
                        .iter()
                        .any(|m| m.id.as_str() == new_model);
                    if valid {
                        let why = if result.reason.is_empty() {
                            "vision handoff".to_string()
                        } else {
                            result.reason.clone()
                        };
                        emit(&Event::new("info").with(
                            "message",
                            json!(format!(
                                "vision handoff: {} → {} ({})",
                                model, new_model, why
                            )),
                        ));
                        st.logger.log("vision_handoff", json!({
                            "from": model, "to": new_model, "plugin": plugin_name.clone(), "reason": why
                        }));
                        model = new_model.to_string();
                    } else {
                        emit(&Event::new("info").with(
                            "message",
                            json!(format!(
                                "vision handoff ignored: '{}' is not a discovered model",
                                new_model
                            )),
                        ));
                    }
                }
            }
        }
        // No vision plugin handed off an image-bearing turn on a non-vision
        // model. Surface it so the user knows to configure /vision (or that
        // no vision model is available) instead of silently parsing bytes.
        if has_images && model == original_model {
            let current_has_vision = st
                .models
                .read()
                .await
                .iter()
                .find(|m| m.id == model.as_str())
                .map(|m| m.vision || vc.has_vision(m.id.as_str()))
                .unwrap_or(false);
            if !current_has_vision {
                emit(&Event::new("info").with("message", json!(format!(
                    "image attached but '{}' lacks vision and no vision model is configured to hand off to; use /vision to set one (or select a vision model with /model)",
                    model
                ))));
            }
        }
    }

    // Main agent tool list: exclude the subagent-only intercom coordination tools
    // (contact_supervisor/intercom) — those are registered only inside child runs.
    let tool_defs: Vec<Value> = tools::definitions()
        .into_iter()
        .filter(|d| {
            let n = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            n != "contact_supervisor" && n != "intercom"
        })
        .collect();
    let mut timer = TurnTimer::new();

    // Idle compaction: if 60+ minutes since the last turn completed, compact the
    // conversation so the next turn starts lean. Uses the same summarize strategy
    // as the threshold path; falls back to naive drop-oldest without an api key.
    {
        let last = *st.last_turn_time.lock().await;
        if last.elapsed().as_secs() > 3600 {
            let mut messages = st.conversation.lock().await.clone();
            if messages.len() > 4 {
                dispatch_lifecycle(st, "pre_compact").await;
                let est = { *st.estimated_tokens.lock().await };
                let cfg = st.cfg.read().await.clone();
                let rp = st.resolved_provider().await;
                let idle_ctx = st
                    .models
                    .read()
                    .await
                    .iter()
                    .find(|m| m.id == model)
                    .map(|m| m.context_window as u64)
                    .unwrap_or(200_000);
                if rp.api_key.is_some() {
                    compact_with_summary(
                        client,
                        &cfg,
                        &rp,
                        &model,
                        &mut messages,
                        &cancel,
                        false,
                        idle_ctx,
                    )
                    .await
                } else {
                    compact_conversation(&mut messages, idle_ctx)
                }
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                let after_est = estimate_messages_tokens(&messages);
                *st.estimated_tokens.lock().await = after_est;
                *st.needs_sanitize.lock().await = true;
                emit(
                    &Event::new("compacted")
                        .with("before_tokens", json!(est))
                        .with("after_tokens", json!(after_est)),
                );
            }
        }
    }

    loop {
        if cancel.is_cancelled() {
            emit(&Event::new("aborted"));
            return;
        }
        // Session token budget (hard ceiling across the whole session, not per turn).
        // 0 = unlimited. Trips before the request so we don't blow past a cost cap.
        let budget = st.cfg.read().await.max_session_tokens;
        if budget > 0 {
            let spent = *st.tokens_in.lock().await + *st.tokens_out.lock().await;
            if spent >= budget {
                emit(&Event::new("error").with(
                    "message",
                    json!(format!(
                        "session token budget exhausted ({spent} >= {budget}); start a new session"
                    )),
                ));
                emit(&Event::new("done"));
                return;
            }
        }

        // Resolve the active provider for this turn. Errors out if no API key is
        // available for it (runtime override -> config literal -> env var).
        let provider = {
            let rp = st.resolved_provider().await;
            match rp.api_key.as_ref() {
                Some(_) => rp,
                None => {
                    emit(
                        &Event::new("error")
                            .with(
                                "message",
                                json!(format!(
                                    "no API key set for provider '{}'; use set_key first",
                                    rp.name
                                )),
                            ),
                    );
                    emit(&Event::new("done"));
                    return;
                }
            }
        };

        let cfg = st.cfg.read().await.clone();
        // Context window management: compact once past the configured threshold
        // (default 70%). The 95% hard cap is a floor — compact by then even if the
        // configured threshold is higher, and force the summarize strategy even
        // when disabled (naive drop-oldest may not reclaim enough at critical capacity).
        let mut messages = st.conversation.lock().await.clone();
        let (model_ctx, thinking_levels, max_tokens) = st
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == model)
            .map(|m| (m.context_window as u64, m.thinking_levels.clone(), m.max_tokens))
            .unwrap_or((200_000, Vec::new(), 8_192));
        let mut est = { *st.estimated_tokens.lock().await };
        let threshold = (model_ctx as f32 * cfg.context_compact_at) as u64;
        let hard_cap = (model_ctx as f32 * 0.95) as u64;
        // Soft digest: collapse stale, large tool results into one-line digests
        // well before the compaction threshold so they stop being re-sent verbatim
        // on every turn. Conservative — only tool messages older than the
        // compaction tail (DIGEST_KEEP_LAST) and larger than DIGEST_MIN_BYTES are
        // touched; idempotent; tool_call_id + role preserved so the model's
        // tool-call/result pairing stays intact. This never removes information
        // compaction would keep (compaction drops these entirely), so it is
        // strictly safer than waiting for compaction to fire.
        let soft = (model_ctx as f32 * cfg.context_digest_at) as u64;
        if est > soft && messages.len() > DIGEST_KEEP_LAST {
            let before_est = est;
            let changed = digest_stale_tool_results(&mut messages, DIGEST_KEEP_LAST);
            if changed > 0 {
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                est = estimate_messages_tokens(&messages);
                *st.estimated_tokens.lock().await = est;
                st.logger.log(
                    "digested",
                    json!({ "results": changed, "before_tokens": before_est, "after_tokens": est }),
                );
                emit(
                    &Event::new("digested")
                        .with("results", json!(changed))
                        .with("before_tokens", json!(before_est))
                        .with("after_tokens", json!(est)),
                );
            }
        }
        if est > threshold.min(hard_cap) && messages.len() > 4 {
            let force_summarize = est > hard_cap;
            dispatch_lifecycle(st, "pre_compact").await;
            compact_with_summary(
                client,
                &cfg,
                &provider,
                &model,
                &mut messages,
                &cancel,
                force_summarize,
                model_ctx,
            )
            .await;
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            let after_est = estimate_messages_tokens(&messages);
            *st.estimated_tokens.lock().await = after_est;
            *st.needs_sanitize.lock().await = true;
            emit(
                &Event::new("compacted")
                    .with("before_tokens", json!(est))
                    .with("after_tokens", json!(after_est)),
            );
        }

        // Sanitize orphaned tool calls right before the request (mirrors Umans extension).
        // Only needed after compaction; skip the O(n) scan on clean turns.
        if *st.needs_sanitize.lock().await {
            provider::sanitize_orphaned_tool_calls(&mut messages);
            let fixed_args = provider::sanitize_tool_call_arguments(&mut messages);
            *st.needs_sanitize.lock().await = false;
            // Persist the sanitized history so a resumed session doesn't replay
            // orphaned tool_calls (which the API would reject) or malformed args.
            // Both sanitizers mutate `messages`; write the whole sanitized vec
            // back to the conversation + session file so the on-disk history stays
            // valid for the API and for any other client that loads it.
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            if fixed_args > 0 {
                emit(&Event::new("info").with("message", json!(format!(
                    "sanitized {fixed_args} malformed tool-call argument(s) to keep the conversation valid for the API"
                ))));
            }
        }
        let (assistant, _finish, tokens_in, tokens_out, cached_tokens) =
            match provider::stream_turn(
                client,
                &provider,
                cfg.idle_timeout_secs,
                &model,
                &messages,
                &tool_defs,
                &effort,
                &thinking_levels,
                max_tokens,
                &cancel,
                &mut timer,
                false,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    st.logger.log("turn_error", json!({ "error": e }));
                    if e == "aborted" {
                        emit(&Event::new("aborted"));
                    } else {
                        emit(&Event::new("error").with("message", json!(e)));
                    }
                    emit(&Event::new("done"));
                    return;
                }
            };

        // Accumulate token counts for /stats. When the endpoint omits usage
        // (tokens come back zero) estimate from the exchanged messages so the
        // session totals aren't stuck at zero alongside the footer budget.
        let (acc_in, acc_out) = if tokens_in > 0 || tokens_out > 0 {
            (tokens_in, tokens_out)
        } else {
            // Endpoint omitted usage: estimate THIS turn's input as the prompt we
            // sent (the whole messages array) and output as the assistant reply —
            // NOT the accumulated session total, which would double-count every
            // prior turn and trip --max-session-tokens after 1-2 turns on
            // usage-less endpoints.
            (
                estimate_messages_tokens(&messages),
                estimate_message_tokens(&assistant),
            )
        };
        *st.tokens_in.lock().await += acc_in;
        *st.tokens_out.lock().await += acc_out;
        // Accumulate prefix-cache hits so /stats can show cache effectiveness.
        *st.cached_tokens.lock().await += cached_tokens;

        // Append + persist the finalized assistant message.
        {
            let mut conv = st.conversation.lock().await;
            conv.push(assistant.clone());
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, conv.last().unwrap());
            }
        }
        *st.estimated_tokens.lock().await += estimate_message_tokens(&assistant);

        let tool_calls = assistant
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned();
        match tool_calls {
            Some(calls) if !calls.is_empty() => {
                for tc in &calls {
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let func = tc.get("function");
                    let name = func
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args_str = func
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    emit(
                        &Event::new("tool_call")
                            .with("id", json!(id))
                            .with("name", json!(name))
                            .with("args", json!(args_str)),
                    );
                    let args: Value = match serde_json::from_str(&args_str) {
                        Ok(v) => v,
                        Err(_) => {
                            // Malformed JSON arguments: the model produced an argument
                            // string that isn't valid JSON (common with long, quote-heavy
                            // commands wrapped inside `bulk`'s nested JSON). Return an
                            // actionable error so the model retries simply, and flag the
                            // conversation for argument sanitization so the malformed
                            // assistant message doesn't make the next API request fail
                            // with "function.arguments must be valid JSON" — which would
                            // repeat on every turn and brick the session.
                            *st.needs_sanitize.lock().await = true;
                            let msg = format!(
                                "tool call '{}' produced malformed JSON arguments (the argument string was not valid JSON). This usually happens with long, quote-heavy commands wrapped inside bulk's nested JSON. Re-issue it simply: call bash directly (not via bulk), and for complex logic write a script to a file with write_file then run `bash script.sh` instead of inlining one long command string.",
                                name
                            );
                            emit(
                                &Event::new("tool_result")
                                    .with("id", json!(id))
                                    .with("ok", json!(false))
                                    .with("output", json!(msg)),
                            );
                            let tool_result =
                                json!({ "role": "tool", "tool_call_id": id, "content": msg });
                            let est = estimate_message_tokens(&tool_result);
                            let mut conv = st.conversation.lock().await;
                            conv.push(tool_result);
                            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                session::append(p, conv.last().unwrap());
                            }
                            *st.estimated_tokens.lock().await += est;
                            continue;
                        }
                    };

                    // Approval gate for destructive tools.
                    let cfg = st.cfg.read().await.clone();
                    let kind = tools::classify(&name);
                    let kind_str: &'static str = match kind {
                        tools::ToolKind::ReadOnly => "readonly",
                        tools::ToolKind::Destructive => "destructive",
                    };
                    // Skip the gate if the user previously said "always" to this kind.
                    let escalated = st.escalated_kinds.lock().await.contains(kind_str);

                    // Check permission rules before the approval gate.
                    // DENY rules take precedence; ALLOW rules skip the gate entirely.
                    let mut force_allow = false;
                    let mut force_deny = false;
                    for rule in &cfg.allow_rules {
                        if tool_matches_rule(&name, &args, rule) {
                            force_allow = true;
                            break;
                        }
                    }
                    if !force_allow {
                        for rule in &cfg.deny_rules {
                            if tool_matches_rule(&name, &args, rule) {
                                force_deny = true;
                                break;
                            }
                        }
                    }

                    if force_deny {
                        let msg = format!("tool call '{}' denied by permission rule", name);
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result =
                            json!({ "role": "tool", "tool_call_id": id, "content": msg });
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    let needs_approval = if force_allow || escalated {
                        false
                    } else {
                        match cfg.approval {
                            Approval::Never => false,
                            Approval::Destructive => kind == tools::ToolKind::Destructive,
                            Approval::Always => true,
                        }
                    };
                    if needs_approval {
                        match request_approval(st, &id, &name, &args_str, kind_str, &cancel).await {
                            ApprovalResult::Granted => {}
                            ApprovalResult::Denied => {
                                let msg = format!("tool call '{}' was denied by the user", name);
                                emit(
                                    &Event::new("tool_result")
                                        .with("id", json!(id))
                                        .with("ok", json!(false))
                                        .with("output", json!(msg)),
                                );
                                let tool_result =
                                    json!({ "role": "tool", "tool_call_id": id, "content": msg });
                                let est = estimate_message_tokens(&tool_result);
                                let mut conv = st.conversation.lock().await;
                                conv.push(tool_result);
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
                                *st.estimated_tokens.lock().await += est;
                                continue;
                            }
                            ApprovalResult::Aborted => {
                                emit(&Event::new("aborted"));
                                emit(&Event::new("done"));
                                return;
                            }
                        }
                    }

                    // Check dangerous paths for write/edit tools.
                    let dangerous = if name == "write_file" || name == "edit" || name == "patch" {
                        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        workspace::check_dangerous_path(path)
                    } else {
                        None
                    };
                    if let Some(msg) = dangerous {
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result =
                            json!({ "role": "tool", "tool_call_id": id, "content": msg });
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    // Dispatch pre-execution hooks for this tool. Each enabled
                    // plugin registered for this hook point runs exactly once, in
                    // order. A hook may: allow (optionally overriding specific arg
                    // fields via `modify`, and/or posting a `reason` the model will
                    // see), or deny (the tool call is skipped and the reason is
                    // returned to the model). Hooks compose: each sees the args as
                    // amended by earlier hooks.
                    let hook_name = match name.as_str() {
                        "bash" => "pre_bash",
                        "write_file" | "edit" => "pre_write",
                        "read_file" | "grep" | "glob" => "pre_read",
                        _ => "",
                    };
                    let pre_configs = if hook_name.is_empty() {
                        Vec::new()
                    } else {
                        st.plugin_manager.get_hook_configs(hook_name)
                    };
                    // exec_args starts as the original args and is amended in
                    // place by pre-hooks. Only clone when hooks will actually run,
                    // so large write payloads aren't copied in the common case.
                    let mut exec_args = if pre_configs.is_empty() {
                        args
                    } else {
                        args.clone()
                    };
                    let mut hook_notes: Vec<String> = Vec::new();
                    let mut denied_by_hook = false;
                    for (plugin_name, config) in &pre_configs {
                        let session_id = cfg
                            .session_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let ctx = plugins::build_context(
                            hook_name,
                            &name,
                            &cfg.workspace.display().to_string(),
                            Some(&exec_args),
                            &session_id,
                            config.pass_args,
                        );
                        let result =
                            plugins::execute_hook(hook_name, plugin_name, config, &ctx).await;
                        if !result.allow {
                            // Deny: skip the tool call and tell the model why.
                            let msg = format!(
                                "tool call '{}' denied by plugin '{}' hook '{}': {}",
                                name, plugin_name, hook_name, result.reason
                            );
                            emit(
                                &Event::new("tool_result")
                                    .with("id", json!(id))
                                    .with("ok", json!(false))
                                    .with("output", json!(msg)),
                            );
                            let tool_result =
                                json!({ "role": "tool", "tool_call_id": id, "content": msg });
                            let est = estimate_message_tokens(&tool_result);
                            let mut conv = st.conversation.lock().await;
                            conv.push(tool_result);
                            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                session::append(p, conv.last().unwrap());
                            }
                            *st.estimated_tokens.lock().await += est;
                            denied_by_hook = true;
                            break;
                        }
                        // Allow: merge `modify` over the running args so a hook
                        // can override specific fields (e.g. reformatted `content`
                        // or a fixed `command`) without dropping the rest (e.g.
                        // `path`, `edits`). The contract is "return only the keys
                        // you want to change"; anything else is preserved.
                        if let Some(ref modify) = result.modify {
                            plugins::apply_modify(&mut exec_args, modify);
                        }
                        // Remember non-empty reasons so the model is told its tool
                        // call was inspected/modified (and can react accordingly).
                        if !result.reason.is_empty() {
                            hook_notes
                                .push(format!("{}/{}: {}", plugin_name, hook_name, result.reason));
                        }
                    }
                    if denied_by_hook {
                        continue;
                    }

                    // bulk inner-call gate: run the same permission deny-rules +
                    // dangerous-path + plugin pre-hook gate on EACH inner call so
                    // destructive ops can't evade the safety floor by hiding inside
                    // a single `bulk` call (the outer deny/hook loop above only sees
                    // the `bulk` call itself). Denied inner calls are recorded by
                    // index and rendered by execute_bulk.
                    let mut bulk_denied: std::collections::HashMap<usize, String> =
                        std::collections::HashMap::new();
                    if name == "bulk" {
                        if let Some(calls) =
                            exec_args.get_mut("calls").and_then(|v| v.as_array_mut())
                        {
                            for (i, c) in calls.iter_mut().enumerate() {
                                let iname = c
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let iargs = c.get("args").cloned().unwrap_or(json!({}));
                                let mut modified = iargs.clone();
                                let mut dmsg: Option<String> = None;
                                // permission deny-rules (ALLOW skips, DENY blocks)
                                let mut force_allow = false;
                                for rule in &cfg.allow_rules {
                                    if tool_matches_rule(&iname, &iargs, rule) {
                                        force_allow = true;
                                        break;
                                    }
                                }
                                if !force_allow {
                                    for rule in &cfg.deny_rules {
                                        if tool_matches_rule(&iname, &iargs, rule) {
                                            dmsg = Some("denied by permission rule".into());
                                            break;
                                        }
                                    }
                                }
                                // dangerous-path for write/edit (pre-hook)
                                if dmsg.is_none() && (iname == "write_file" || iname == "edit") {
                                    if let Some(m) = workspace::check_dangerous_path(
                                        iargs.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                                    ) {
                                        dmsg = Some(m);
                                    }
                                }
                                // plugin pre-hooks (the security-relevant ones)
                                if dmsg.is_none() {
                                    let hook_name = match iname.as_str() {
                                        "bash" => "pre_bash",
                                        "write_file" | "edit" => "pre_write",
                                        "read_file" | "grep" | "glob" => "pre_read",
                                        _ => "",
                                    };
                                    if !hook_name.is_empty() {
                                        let configs = st.plugin_manager.get_hook_configs(hook_name);
                                        for (pn, config) in &configs {
                                            let session_id = cfg
                                                .session_file
                                                .as_ref()
                                                .map(|p| p.display().to_string())
                                                .unwrap_or_default();
                                            let ctx = plugins::build_context(
                                                hook_name,
                                                &iname,
                                                &cfg.workspace.display().to_string(),
                                                Some(&modified),
                                                &session_id,
                                                config.pass_args,
                                            );
                                            let r =
                                                plugins::execute_hook(hook_name, pn, config, &ctx)
                                                    .await;
                                            if !r.allow {
                                                dmsg = Some(format!(
                                                    "denied by plugin '{}' hook '{}': {}",
                                                    pn, hook_name, r.reason
                                                ));
                                                break;
                                            }
                                            if let Some(m) = &r.modify {
                                                plugins::apply_modify(&mut modified, m);
                                            }
                                        }
                                        // re-check dangerous path after a hook may have rewritten it
                                        if dmsg.is_none()
                                            && (iname == "write_file" || iname == "edit")
                                        {
                                            if let Some(m) = workspace::check_dangerous_path(
                                                modified
                                                    .get("path")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or(""),
                                            ) {
                                                dmsg = Some(m);
                                            }
                                        }
                                    }
                                }
                                if let Some(m) = dmsg {
                                    bulk_denied.insert(i, m);
                                } else {
                                    *c = json!({ "name": iname, "args": modified });
                                }
                            }
                        }
                    }

                    // Execute. bash/bulk/diagnostics/spawn are async; others sync.
                    // The async ones are wrapped in a `select!` on the turn cancel
                    // so /abort can interrupt them mid-flight — kill_on_drop frees
                    // the spawned child when the future is dropped.
                    let mut outcome = if name == "bash" {
                        let cmd = exec_args
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let timeout_override = exec_args
                            .get("timeout")
                            .and_then(|v| v.as_u64());
                        tokio::select! {
                            o = tools::execute_bash(cmd, &cfg, timeout_override) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                        }
                    } else if name == "bulk" {
                        tokio::select! {
                            o = tools::execute_bulk(&exec_args, &cfg, &bulk_denied) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("bulk aborted"),
                        }
                    } else if name == "fetch" {
                        tokio::select! {
                            o = tools::execute_fetch(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("fetch aborted"),
                        }
                    } else if name == "diagnostics" {
                        tokio::select! {
                            o = tools::execute_diagnostics(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("diagnostics aborted"),
                        }
                    } else if name == "spawn" || name == "subagent" {
                        subagent::execute(
                            st.clone(),
                            client.clone(),
                            provider.clone(),
                            model.clone(),
                            exec_args.clone(),
                            cancel.clone(),
                            0,
                        )
                        .await
                    } else {
                        tools::execute(&name, &exec_args, &cfg)
                    };

                    // Dispatch post-execution hooks for this tool.
                    let post_hook = match name.as_str() {
                        "bash" => "post_bash",
                        "write_file" | "edit" => "post_write",
                        "read_file" | "grep" | "glob" => "post_read",
                        _ => "",
                    };
                    if !post_hook.is_empty() {
                        let configs = st.plugin_manager.get_hook_configs(post_hook);
                        for (plugin_name, config) in &configs {
                            let session_id = cfg
                                .session_file
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            let ctx = plugins::build_context(
                                post_hook,
                                &name,
                                &cfg.workspace.display().to_string(),
                                Some(&exec_args),
                                &session_id,
                                config.pass_args,
                            );
                            // Post-hooks can't block (the op already ran), but their
                            // reason is surfaced to the model as a note.
                            let result =
                                plugins::execute_hook(post_hook, plugin_name, config, &ctx).await;
                            if !result.reason.is_empty() {
                                hook_notes.push(format!(
                                    "{}/{}: {}",
                                    plugin_name, post_hook, result.reason
                                ));
                            }
                        }
                    }

                    // finish sentinel: the model signaled completion.
                    if name == "finish" && outcome.ok && outcome.output == "__finish__" {
                        *st.last_turn_time.lock().await = std::time::Instant::now();
                        let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                        let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                        emit(
                            &Event::new("metrics")
                                .with("ttft_ms", json!(metrics.ttft_ms))
                                .with("elapsed_ms", json!(metrics.elapsed_ms))
                                .with(
                                    "tokens_in",
                                    json!(metrics.tokens_in.saturating_add(metrics.tokens_out)),
                                )
                                .with("prompt_tokens", json!(metrics.tokens_in))
                                .with("tokens_out", json!(metrics.tokens_out))
                                .with("cached_tokens", json!(metrics.cached_tokens))
                                .with("tps", json!(metrics.tps))
                                .with("model", json!(metrics.model)),
                        );
                        st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "cached_tokens": metrics.cached_tokens, "ttft_ms": metrics.ttft_ms, "tps": metrics.tps, "finish_tool": true }));
                        st.logger.record_turn();
                        emit(&Event::new("done"));
                        return;
                    }
                    // Surface plugin hook feedback to the model. Any pre-hook that
                    // modified args or posted a reason, and any post-hook that
                    // observed something, is appended to the tool result so the
                    // model knows its write/edit/read/bash call was inspected.
                    if !hook_notes.is_empty() {
                        outcome.output.push_str("\n\nPlugin hooks:\n- ");
                        outcome.output.push_str(&hook_notes.join("\n- "));
                    }
                    st.logger.log("tool", json!({ "name": name, "args": args_str, "ok": outcome.ok, "output_len": outcome.output.len() }));
                    let mut ev = Event::new("tool_result")
                        .with("id", json!(id))
                        .with("ok", json!(outcome.ok))
                        .with("output", json!(outcome.output));
                    // Surface a unified-diff rendering to the TUI as a separate
                    // `diff` field (edit/patch/write_file). It is NOT added to the
                    // model-facing tool content (`output`) so the model's context
                    // stays compact — the diff is for the human approver.
                    if let Some(d) = &outcome.diff {
                        ev = ev.with("diff", json!(d));
                    }
                    emit(&ev);
                    let tool_result =
                        json!({ "role": "tool", "tool_call_id": id, "content": outcome.output });
                    let est = estimate_message_tokens(&tool_result);
                    let mut conv = st.conversation.lock().await;
                    conv.push(tool_result);
                    if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                        session::append(p, conv.last().unwrap());
                    }
                    *st.estimated_tokens.lock().await += est;
                }
                // Loop back for the model to continue.
            }
            _ => {
                // Turn complete: emit metrics + done.
                *st.last_turn_time.lock().await = std::time::Instant::now();
                let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                emit(
                    &Event::new("metrics")
                        .with("ttft_ms", json!(metrics.ttft_ms))
                        .with("elapsed_ms", json!(metrics.elapsed_ms))
                        .with(
                            "tokens_in",
                            json!(metrics.tokens_in.saturating_add(metrics.tokens_out)),
                        )
                        .with("prompt_tokens", json!(metrics.tokens_in))
                        .with("tokens_out", json!(metrics.tokens_out))
                        .with("cached_tokens", json!(metrics.cached_tokens))
                        .with("tps", json!(metrics.tps))
                        .with("model", json!(metrics.model)),
                );
                st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "cached_tokens": metrics.cached_tokens, "ttft_ms": metrics.ttft_ms, "tps": metrics.tps }));
                st.logger.record_turn();
                emit(&Event::new("done"));
                return;
            }
        }
    }
}

pub(crate) enum ApprovalResult {
    Granted,
    Denied,
    Aborted,
}

/// Monotonic generator for globally-unique approval ids so parallel subagents
/// (which may each emit a tool call `call_1`) never collide on the shared
/// pending-approval map. The id embeds the originating tool-call id for tracing.
static APPROVAL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Ask the TUI to approve a tool call; block until answered or aborted.
/// On "always", only the matched tool KIND is escalated (not the whole session).
pub(crate) async fn request_approval(
    st: &Arc<State>,
    id: &str,
    name: &str,
    args: &str,
    kind_str: &'static str,
    cancel: &CancellationToken,
) -> ApprovalResult {
    let request_id = format!(
        "apv-{}-{}",
        APPROVAL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        id
    );
    let notify = Arc::new(Notify::new());
    let pending = Arc::new(PendingApproval {
        request_id: request_id.clone(),
        tool: name.to_string(),
        args: serde_json::from_str(args).unwrap_or(json!({})),
        notify: notify.clone(),
        granted: Mutex::new(None),
        escalated: Mutex::new(false),
    });

    st.pending
        .lock()
        .await
        .insert(request_id.clone(), pending.clone());
    // Surface the resulting change to the human, not just the raw search/replace
    // blobs: compute the unified diff the call *would* produce (without writing)
    // and attach it to the approval_request event so the TUI can render it. Only
    // write/edit/patch produce a file diff; other destructive tools (bash, git)
    // carry no preview.
    let cfg = st.cfg.read().await.clone();
    let args_v: Value = serde_json::from_str(args).unwrap_or(json!({}));
    let diff: Option<String> = match name {
        "write_file" => {
            let path = args_v.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = args_v.get("content").and_then(|v| v.as_str()).unwrap_or("");
            tools::preview_diff_write(path, content, &cfg).ok()
        }
        "edit" => {
            let path = args_v.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match args_v.get("edits").and_then(|v| v.as_array()) {
                Some(e) if !e.is_empty() => tools::preview_diff_edit(path, e, &cfg).ok(),
                _ => None,
            }
        }
        "patch" => {
            let path = args_v.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let patch = args_v.get("patch").and_then(|v| v.as_str()).unwrap_or("");
            if !path.is_empty() && !patch.is_empty() {
                tools::preview_diff_patch(path, patch, &cfg).ok()
            } else {
                None
            }
        }
        _ => None,
    };
    let evt = Event::new("approval_request")
        .with("request_id", json!(request_id))
        .with("tool", json!(name))
        .with("args", json!(args));
    let evt = if let Some(d) = &diff {
        evt.with("diff", json!(d))
    } else {
        evt
    };
    emit(&evt);

    // Wait for the approve command or abort.
    let granted = tokio::select! {
        _ = notify.notified() => pending.granted.lock().await.unwrap_or(false),
        _ = cancel.cancelled() => {
            st.pending.lock().await.remove(&request_id);
            return ApprovalResult::Aborted;
        }
    };

    // "always" escalates: record this tool KIND so subsequent calls of the same
    // kind skip the gate, without un-gating other kinds or the whole session.
    if *pending.escalated.lock().await {
        st.escalated_kinds.lock().await.insert(kind_str);
        emit(&Event::new("approval_changed").with("mode", json!(format!("{}:always", kind_str))));
        // Persist the escalation so a restart doesn't un-gate this kind.
        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
            let set: std::collections::HashSet<String> = st
                .escalated_kinds
                .lock()
                .await
                .iter()
                .map(|s| s.to_string())
                .collect();
            session::save_escalations(p, &set);
        }
    }
    st.pending.lock().await.remove(&request_id);
    if granted {
        ApprovalResult::Granted
    } else {
        ApprovalResult::Denied
    }
}

/// Number of trailing messages whose tool results are always kept verbatim.
/// Chosen >= the compaction tail (8 summarize / 10 naive) so digesting never
/// touches anything compaction would keep — it only reclaims tokens from
/// results that compaction would otherwise drop entirely.
const DIGEST_KEEP_LAST: usize = 10;
/// Minimum tool-result size (bytes) worth digesting. Small results (ok/err
/// one-liners, denial messages) stay full — they're cheap and the model may
/// need them verbatim.
const DIGEST_MIN_BYTES: usize = 256;

/// Collapse stale, large `role: "tool"` results into a one-line digest so they
/// stop being re-sent verbatim on every turn. Only tool messages older than the
/// last `keep` messages are eligible, and only if their content exceeds
/// `DIGEST_MIN_BYTES`. Already-digested results are skipped (idempotent). The
/// tool_call_id and role are preserved so orphaned-call sanitization and the
/// model's tool-call/result pairing stay intact. Returns the count digested.
#[allow(clippy::ptr_arg)]
pub fn digest_stale_tool_results(messages: &mut Vec<Value>, keep: usize) -> usize {
    if messages.len() <= keep {
        return 0;
    }
    // Build tool_call_id -> (tool_name, args_json) from assistant tool_calls so
    // the digest records WHAT was read/run, not just the size.
    let mut call_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for m in messages.iter() {
        if m.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in calls {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let func = tc.get("function");
                let name = func
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = func
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string();
                call_map.insert(id, (name, args));
            }
        }
    }
    let digest_to = messages.len().saturating_sub(keep);
    let mut changed = 0usize;
    for m in messages[..digest_to].iter_mut() {
        if m.get("role").and_then(|v| v.as_str()) != Some("tool") {
            continue;
        }
        let content = match m.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => continue,
        };
        if content.starts_with("[digested:") || content.len() <= DIGEST_MIN_BYTES {
            continue;
        }
        let id = m
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let (name, args_json) = call_map.get(&id).cloned().unwrap_or_default();
        let lines = content.lines().count();
        let digest = make_digest(&name, &args_json, content.len(), lines);
        if let Some(obj) = m.as_object_mut() {
            obj.insert("content".into(), Value::String(digest));
            changed += 1;
        }
    }
    changed
}

/// Build a one-line digest for a tool result, preserving enough to navigate
/// back to the content: the tool name, its key argument, and the size/line
/// count. The suffix tells the model how to recover the full output.
fn make_digest(tool: &str, args_json: &str, len: usize, lines: usize) -> String {
    let args: Value = serde_json::from_str(args_json).unwrap_or(json!({}));
    let get = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let what = match tool {
        "read_file" => {
            if lines > 0 {
                format!(
                    "read_file {:?} ({} lines, {} bytes)",
                    get("path"),
                    lines,
                    len
                )
            } else {
                format!("read_file {:?} ({} bytes)", get("path"), len)
            }
        }
        "bulk_read" => format!("bulk_read ({} bytes)", len),
        "bash" => format!(
            "bash {:?} ({} bytes)",
            truncate_str(get("command"), 80),
            len
        ),
        "grep" => format!(
            "grep {:?} ({} bytes)",
            truncate_str(get("pattern"), 80),
            len
        ),
        "glob" => format!(
            "glob {:?} ({} bytes)",
            truncate_str(get("pattern"), 80),
            len
        ),
        "diagnostics" => format!("diagnostics ({} bytes)", len),
        other => format!("{} ({} bytes)", other, len),
    };
    let how = if tool == "bash" {
        "re-run if needed"
    } else {
        "re-run to recover full output"
    };
    format!("[digested: {what} — {how}]")
}

/// Truncate a string to `n` chars at a char boundary, appending an ellipsis.
fn truncate_str(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

/// Compact the conversation when it nears the context window.
/// ponytail: simple strategy — drop the oldest tool results (the bulk of tokens)
/// and keep system + recent turns. A summarization call would be better but adds
/// cost+complexity; this keeps the agent unblocked. Orphaned-tool-call sanitizer
/// in provider.rs backstops any tool_call/result mismatch this creates.
/// Rebuild the system prompt with fresh memory and persist it. Returns a
/// human-readable status. No-op (preserving the provider prefix cache) when
/// the prompt is unchanged. Shared by RefreshMemory and the save/forget
/// commands so a saved/removed memory is visible to the very next turn.
async fn refresh_memory_injection(state: &State) -> String {
    let ws = state.cfg.read().await.workspace.clone();
    let mem = memory_injection(&ws, "");
    let new_system = build_system_prompt(&ws, true);
    let mut conv = state.conversation.lock().await;
    if let Some(first) = conv.first() {
        let old_content = first.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if old_content == new_system {
            return "memory unchanged; system prompt kept intact (preserving prefix cache)"
                .to_string();
        }
    }
    if let Some(first) = conv.first_mut() {
        if first.get("role").and_then(|v| v.as_str()) == Some("system") {
            *first = json!({ "role": "system", "content": new_system });
            *state.needs_sanitize.lock().await = true;
            *state.estimated_tokens.lock().await = estimate_messages_tokens(&conv);
            if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                session::rewrite(p, &conv);
            }
        }
    }
    drop(conv);
    if mem.is_empty() {
        "memory refreshed: no memories found".to_string()
    } else {
        "memory refreshed: memories injected".to_string()
    }
}

/// Index where the kept verbatim tail begins, chosen by token budget rather
/// than a fixed message count. Walks backward from the end accumulating
/// `estimate_message_tokens` until the budget (25% of the context window,
/// floored at 6k tokens) is exceeded, always keeping at least `MIN_TAIL`
/// messages. A fixed count over-keeps a quiet stretch and under-keeps when a
/// huge tool result eats the whole window; a budget keeps the live context
/// that actually fits and lets the summary reclaim the rest.
fn token_budget_tail_start(messages: &[Value], context_window: u64) -> usize {
    const MIN_TAIL: usize = 6;
    const TAIL_FRACTION: f32 = 0.25;
    let n = messages.len();
    if n <= MIN_TAIL {
        return 0;
    }
    let budget = ((context_window as f32 * TAIL_FRACTION) as u64).max(6_000);
    let mut acc: u64 = 0;
    let mut start = n;
    for i in (0..n).rev() {
        let t = estimate_message_tokens(&messages[i]);
        // Always keep the most recent MIN_TAIL messages; only enforce the
        // budget on older ones so a single giant tool result can't shrink the
        // tail to nothing.
        if i < n.saturating_sub(MIN_TAIL) && acc + t > budget {
            break;
        }
        acc += t;
        start = i;
    }
    start
}

/// Naive compaction fallback: keep the system prompt + a token-budgeted tail
/// verbatim, drop the middle with a marker. `context_window` sizes the tail
/// (0/unset → the 6k floor). Used when summarization is disabled or unavailable.
pub fn compact_conversation(messages: &mut Vec<Value>, context_window: u64) {
    if messages.len() <= 12 {
        return;
    }
    let system = messages[0].clone();
    let tail_start = token_budget_tail_start(messages, context_window).max(1);
    let tail: Vec<Value> = messages[tail_start..].to_vec();
    let mut compacted = vec![system];
    compacted.push(json!({ "role": "system", "content": "[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped.]" }));
    compacted.extend(tail);
    *messages = compacted;
}

/// Compact a conversation by summarizing older turns into one system message,
/// keeping the system prompt + a token-budgeted tail verbatim. Falls back to
/// the naive drop-oldest (`compact_conversation`) when summarization is
/// disabled and not forced, or when there's too little middle to summarize. On
/// summary failure, degrades to a drop-oldest marker so the turn still
/// proceeds. `force_summarize` overrides `summarize_on_compact=false` — used by
/// the 95% hard cap where naive drop-oldest may not reclaim enough.
pub async fn compact_with_summary(
    client: &reqwest::Client,
    cfg: &Config,
    provider: &ResolvedProvider,
    model: &str,
    messages: &mut Vec<Value>,
    cancel: &CancellationToken,
    force_summarize: bool,
    context_window: u64,
) {
    if messages.len() <= 4 {
        return;
    }
    if !cfg.summarize_on_compact && !force_summarize {
        compact_conversation(messages, context_window);
        return;
    }
    let tail_start = token_budget_tail_start(messages, context_window).max(1);
    if tail_start <= 1 {
        compact_conversation(messages, context_window);
        return;
    }
    let to_summarize: Vec<Value> = messages[1..tail_start].to_vec();
    let kept: Vec<Value> = messages[tail_start..].to_vec();
    let summary = provider::summarize(client, provider, model, &to_summarize, cancel).await;
    let mut compacted = vec![messages[0].clone()];
    if let Some(s) = summary {
        compacted.push(
            json!({ "role": "system", "content": format!("[Summary of earlier turns]\n{s}") }),
        );
    } else {
        compacted.push(json!({ "role": "system", "content": "[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped; summarization was unavailable.]" }));
    }
    // Session memory extraction: persist durable facts so future sessions inherit
    // project knowledge. Best-effort; never blocks compaction. Facts ACCUMULATE
    // across compactions (append, not overwrite) so early-session facts survive,
    // with a rolling byte cap so the file stays bounded.
    if cfg.summarize_on_compact {
        if let Some(facts) =
            provider::extract_facts(client, provider, model, &to_summarize, cancel).await
        {
            let _ = memory::append_memory(
                &cfg.workspace,
                "session-extract",
                &facts,
                "session",
                "auto-extracted durable facts (accumulated on compaction)",
                16_384,
            );
        }
    }
    compacted.extend(kept);
    *messages = compacted;
}

/// Turn an image reference into a data URL. Accepts:
/// - an existing data URL (data:image/...;base64,...) → passthrough
/// - an absolute or workspace-relative file path → read + base64-encode
///
/// Returns a placeholder data URL on failure so the model gets a clear signal.
fn image_to_data_url(img: &str) -> String {
    if img.starts_with("data:") {
        return img.to_string();
    }
    // Resolve relative to cwd (the TUI's workspace). Refuse absolute paths that
    // escape — but vision input is a trust-boundary feature, so we allow any
    // readable path the host process can see.
    let p = std::path::Path::new(img);
    match std::fs::read(p) {
        Ok(bytes) => {
            // Sniff type from magic bytes; default to png.
            let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                "image/png"
            } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF8") {
                "image/gif"
            } else if bytes.starts_with(b"<svg") {
                "image/svg+xml"
            } else if bytes.starts_with(b"RIFF") && bytes.len() > 11 && &bytes[8..12] == b"WEBP" {
                "image/webp"
            } else if bytes.starts_with(&[0x42, 0x4d]) && bytes.len() > 1 {
                // BMP (BM) — lenient; a non-image file almost never starts with BM
                // followed by plausible header fields, so this is low false-positive risk.
                "image/bmp"
            } else {
                "application/octet-stream"
            };
            if mime == "application/octet-stream" {
                // Refuse to attach a non-image: /attach must only send actual
                // images to the provider. This is the vision-trust boundary —
                // a user typing `/attach ~/.ssh/id_rsa` (or any non-image) is
                // rejected here rather than base64-encoded and leaked to the API.
                return format!(
                    "data:text/plain;base64,{}",
                    base64_encode(
                        b"refused: not a recognized image format (png/jpeg/gif/webp/bmp/svg)"
                    )
                );
            }
            let b64 = base64_encode(&bytes);
            format!("data:{mime};base64,{b64}")
        }
        Err(e) => format!(
            "data:text/plain;base64,{}",
            base64_encode(format!("image read failed: {e}").as_bytes())
        ),
    }
}

/// Minimal base64 encoder (no extra crate).
fn base64_encode(input: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(T[((n >> 18) & 0x3f) as usize] as char);
        out.push(T[((n >> 12) & 0x3f) as usize] as char);
        out.push(T[((n >> 6) & 0x3f) as usize] as char);
        out.push(T[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(T[((n >> 18) & 0x3f) as usize] as char);
        out.push(T[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 0x3f) as usize] as char);
        out.push(T[((n >> 12) & 0x3f) as usize] as char);
        out.push(T[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    fn asst_tool_call(id: &str, name: &str, args: &str) -> Value {
        json!({ "role": "assistant", "tool_calls": [ {
            "id": id, "type": "function",
            "function": { "name": name, "arguments": args }
        } ] })
    }
    fn tool_result(id: &str, content: &str) -> Value {
        json!({ "role": "tool", "tool_call_id": id, "content": content })
    }
    fn asst_text(t: &str) -> Value {
        json!({ "role": "assistant", "content": t })
    }

    fn big_content(n: usize) -> String {
        "x\n".repeat(n)
    }

    /// system + a stale large read result + padding + a recent large read result.
    fn fixture() -> Vec<Value> {
        let mut m = vec![json!({ "role": "system", "content": "sys" })];
        m.push(asst_tool_call(
            "call_1",
            "read_file",
            "{\"path\":\"src/big.rs\"}",
        ));
        m.push(tool_result("call_1", &big_content(150))); // 300 bytes, 150 lines
                                                          // padding (assistant texts) to push call_1's result out of the keep window
        for i in 1..=9 {
            m.push(asst_text(&format!("pad{i}")));
        }
        // a recent large read result that must stay verbatim (inside keep window)
        m.push(asst_tool_call(
            "call_2",
            "read_file",
            "{\"path\":\"src/recent.rs\"}",
        ));
        m.push(tool_result("call_2", &big_content(140))); // 280 bytes
        m.push(asst_text("final"));
        m
    }

    #[test]
    fn digests_old_large_tool_result_keeps_recent() {
        let mut m = fixture();
        let n = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n, 1, "only the stale large result should be digested");
        // stale result (index 2) is now a digest
        let d = m[2].get("content").and_then(|v| v.as_str()).unwrap();
        assert!(d.starts_with("[digested:"), "{}", d);
        assert!(d.contains("read_file"), "{}", d);
        assert!(d.contains("src/big.rs"), "{}", d);
        assert!(d.contains("lines"), "should report line count: {}", d);
        assert!(d.contains("re-run to recover full output"), "{}", d);
        // tool_call_id preserved so the assistant/tool pairing stays valid
        assert_eq!(
            m[2].get("tool_call_id").and_then(|v| v.as_str()),
            Some("call_1")
        );
        // recent large result (inside the keep tail) is untouched
        let r = m[m.len() - 2]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(r.len(), 280, "recent result kept full: {}", r);
        assert!(!r.starts_with("[digested:"));
        assert_eq!(
            m[m.len() - 2].get("tool_call_id").and_then(|v| v.as_str()),
            Some("call_2")
        );
    }

    #[test]
    fn digest_is_idempotent() {
        let mut m = fixture();
        let n1 = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n1, 1);
        let after = m[2]
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let n2 = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n2, 0, "second pass must find nothing to digest");
        assert_eq!(
            m[2].get("content").and_then(|v| v.as_str()),
            Some(after.as_str())
        );
    }

    #[test]
    fn digest_skips_small_results() {
        let mut m = vec![
            json!({ "role": "system", "content": "sys" }),
            asst_tool_call("c1", "edit", "{\"path\":\"a.rs\"}"),
            tool_result("c1", "applied 1 edit(s)"), // 17 bytes — under MIN_BYTES
        ];
        // pad to push it out of the keep window
        for i in 0..12 {
            m.push(asst_text(&format!("p{i}")));
        }
        let n = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n, 0, "small result must not be digested");
        assert_eq!(
            m[2].get("content").and_then(|v| v.as_str()),
            Some("applied 1 edit(s)")
        );
    }

    #[test]
    fn digest_noop_when_under_keep() {
        let mut m = vec![
            json!({ "role": "system", "content": "sys" }),
            asst_tool_call("c1", "read_file", "{\"path\":\"a.rs\"}"),
            tool_result("c1", &big_content(200)),
        ];
        // only 3 messages, keep=10 → nothing eligible
        assert_eq!(digest_stale_tool_results(&mut m, 10), 0);
        assert_eq!(
            m[2].get("content").and_then(|v| v.as_str()).unwrap().len(),
            400
        );
    }

    #[test]
    fn digest_bash_label_says_rerun_if_needed() {
        let mut m = vec![
            json!({ "role": "system", "content": "sys" }),
            asst_tool_call("c1", "bash", "{\"command\":\"cargo build\"}"),
            tool_result("c1", &big_content(150)),
        ];
        for i in 0..12 {
            m.push(asst_text(&format!("p{i}")));
        }
        digest_stale_tool_results(&mut m, 10);
        let d = m[2].get("content").and_then(|v| v.as_str()).unwrap();
        assert!(d.contains("bash"), "{}", d);
        assert!(d.contains("cargo build"), "{}", d);
        assert!(
            d.contains("re-run if needed"),
            "bash digest should not promise side-effect-free recovery: {}",
            d
        );
    }
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    fn sys() -> Value {
        json!({ "role": "system", "content": "sys" })
    }
    fn user(t: &str) -> Value {
        json!({ "role": "user", "content": t })
    }

    #[test]
    fn tail_start_keeps_min_tail_on_tiny_budget() {
        // 20 messages each big enough that the floor budget (6k tokens) is
        // exceeded, so the tail trims the middle but always keeps MIN_TAIL (6).
        let mut m = vec![sys()];
        for i in 0..20 {
            m.push(user(&format!("msg {i}: {}", "x".repeat(2000))));
        }
        // small context window -> budget hits the 6k floor
        let s = token_budget_tail_start(&m, 1000);
        assert!(s >= 1, "must not fold system into the tail: {s}");
        assert!(s <= m.len() - 6, "must keep at least the min tail: {s}");
    }

    #[test]
    fn tail_start_keeps_everything_when_small() {
        let m = vec![sys(), user("a"), user("b"), user("c")];
        assert_eq!(token_budget_tail_start(&m, 200_000), 0);
    }

    #[test]
    fn tail_start_shrinks_for_huge_recent_result() {
        // normal turns then one giant tool result: with a small window the
        // budget keeps only the giant result + the min tail, trimming the rest.
        let mut m = vec![sys()];
        for i in 0..40 {
            m.push(user(&format!("turn {i}")));
        }
        m.push(json!({ "role": "tool", "content": "x".repeat(50_000) }));
        let s = token_budget_tail_start(&m, 1000); // floor budget 6k
        assert!(s >= m.len() - 7 && s <= m.len() - 6, "kept the giant result + min tail: {s}");
    }
}
