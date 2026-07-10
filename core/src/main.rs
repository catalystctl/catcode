// catalyst-code-core: stdio JSON-RPC server. The TUI spawns this binary,
// writes commands to stdin, and reads newline-delimited events from stdout.
//
// Several core functions (stream_turn, run_turn, dispatch_*) intentionally
// carry many positional args (the seam between the request loop and the tool
// layer); refactoring each into a context struct is a larger change, so allow
// the lint here rather than obscure the call sites.
#![allow(clippy::too_many_arguments)]

mod config;
mod fetch_tool;
mod fsutil;
mod git_ctx;
mod goal;
mod intercom;
mod logging;
mod memory;
mod message;
mod oauth;
mod pattern_log;
mod plugins;
mod presence;
mod protocol;
mod provider;
mod search_tool;
mod session;
mod staging;
mod subagent;
mod tools;
mod vision;
mod workspace;

use config::{Approval, Config, PermissionRule, ResolvedProvider};
use git_ctx::{git_context_injection, read_git_context};
use intercom::IntercomBus;
use logging::{
    estimate_message_tokens, estimate_messages_tokens, grounded_estimate, Logger, TurnMetrics,
    TurnTimer,
};
use memory::memory_injection;
#[allow(unused_imports)]
use message::{ContentPart, FunctionCall, ImageUrl, Message, ToolCall};
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
- The `memory` tool (actions: save/append/list/forget) persists durable facts. By default memories are scoped to this workspace (per-codebase); pass `scope: "global"` for cross-codebase facts — the user's name, preferred tech stacks, harness conventions — that apply to every project. Saved memories are injected into your standing system prompt on every future session, so anything worth remembering does not need rediscovering. Use `save` for a new note and `append` to accumulate facts onto an existing one without clobbering it.
- Before signaling done on a non-trivial task, take one reflection step: what convention, architecture fact, decision, or gotcha did you learn that future sessions should not have to rediscover? Persist only durable, reusable facts via `memory` (append if the topic already exists, else save). Do not persist transient task state, one-off details, or trivia. The harness now enforces this deterministically: at the end of any non-trivial turn (≥1 tool call), it injects an auto-reflect continuation BEFORE you write your completion summary — you reflect (save memories, no prose), then write your summary as the final message — and surfaces any recurring work shapes — so you do NOT need to remember to reflect, but you SHOULD still call `memory` proactively mid-task the moment you learn something worth keeping rather than deferring it. Disable with the `auto_reflect` config (env `CATALYST_CODE_AUTO_REFLECT=0`).
- Reusable skills live as markdown + YAML frontmatter under `.catalyst-code/skills/<name>/SKILL.md`. Discover them with `list_dir .catalyst-code/skills/` and read the relevant SKILL.md before applying it. When you solve the same shape of problem more than twice, write a skill there with `write_file` (frontmatter: name/description; body: when-to-use, steps, examples). The pi-subagents skill is already injected for you; others are opt-in. The harness tracks the "shape" of each non-trivial turn (tool sequence + file areas) across sessions; when a shape recurs (≥2×), the auto-reflect continuation names it and asks you to write a skill if none covers it — so the "same shape twice" rule is now evaluable instead of a guess.
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
        // One-line manifest of opt-in skills (name + description) so the
        // orchestrator can discover them without a `list_dir` round-trip.
        // Excludes pi-subagents (already injected in full above). Empty (so the
        // prompt + its prefix cache are untouched) when no opt-in skills exist.
        let manifest = skill_manifest_injection(workspace);
        if !manifest.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&manifest);
        }
    }
    prompt
}

/// Build the MAIN agent's system prompt: the base prompt (git context +
/// memory + PLUGIN_DOCS + orchestrator skill manifest) PLUS any text plugins
/// inject via their `system_prompt` manifest field. Plugin injection is empty
/// (so the prompt + its prefix cache are untouched) when no enabled plugin
/// declares one — mirroring how `build_system_prompt` stays cheap in the common
/// case. Subagents do NOT get plugin injection (they use the built-in tool set
/// only), matching the plugin-tools-are-main-agent-scoped design.
fn build_main_system_prompt(
    workspace: &std::path::Path,
    pm: &plugins::PluginManager,
    auto_reflect: bool,
) -> String {
    let mut prompt = build_system_prompt(workspace, true);
    let inj = pm.system_prompt_injection();
    if !inj.is_empty() {
        prompt.push_str(&inj);
    }
    // When auto-reflect is on, defer the completion summary until AFTER the
    // reflection step so the summary is the last message the user reads, not
    // buried under reflection output. Supersedes the "summarize when done"
    // line in SYSTEM_PROMPT_BASE (kept for subagents + the auto_reflect-off
    // case). Safe to defer: when auto_reflect is on, reflect ALWAYS fires for
    // a real work turn (>=1 tool call, not a /reflect or /index learning turn,
    // not cancelled) — the only turns where it doesn't fire (pure chat with 0
    // tools, or a learning delegation) don't need a separate summary.
    if auto_reflect {
        prompt.push_str(
            "\n\nCompletion flow (auto-reflect is enabled): when your work is verified and \
             complete, call `finish` WITHOUT writing a summary first. The harness injects a \
             reflection step — save any durable memories via the `memory` tool (no user-facing \
             prose), then write your completion summary as the final message and call `finish`. \
             This supersedes the \"summarize when done\" guidance above so your summary is the \
             last thing the user reads.",
        );
    }
    prompt
}

/// Load the bundled pi-subagents SKILL.md (project then user scope) for the
/// orchestrator's system prompt. Returns None if no skill file is found.
fn subagent_orchestrator_skill(workspace: &std::path::Path) -> Option<String> {
    let candidates: [Option<std::path::PathBuf>; 2] = [
        Some(workspace.join(".catalyst-code/skills/pi-subagents/SKILL.md")),
        config::home_dir().map(|h| h.join(".catalyst-code/skills/pi-subagents/SKILL.md")),
    ];
    for p in candidates.into_iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(&p) {
            let (_fm, body) = subagent::parse_frontmatter(&content);
            return Some(format!("# Skill: pi-subagents\n\n{body}"));
        }
    }
    None
}

/// One-line manifest of opt-in skills (name + description) discovered under
/// `.catalyst-code/skills/` (project then user scope). Spliced into the
/// orchestrator's stable system prompt so available skills are visible without a
/// `list_dir` round-trip. Excludes `pi-subagents` (already injected in full) and
/// deduplicates by name (project wins). Returns an empty string when no opt-in
/// skills exist, so a fresh install's prompt — and its provider prefix cache —
/// is left untouched.
fn skill_manifest_injection(workspace: &std::path::Path) -> String {
    let skills = subagent::discover_skills(workspace);
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let lines: Vec<String> = skills
        .iter()
        .filter(|(name, _, _)| name.as_str() != "pi-subagents")
        .filter_map(|(name, desc, loc)| {
            // Use the skill DIRECTORY name (parsed from the SKILL.md path) as the
            // identifier, so the header's `read .catalyst-code/skills/<name>/SKILL.md`
            // always resolves — frontmatter `name` can drift from the dirname.
            let n = std::path::Path::new(loc)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.trim());
            if n.is_empty() || !seen.insert(n) {
                return None;
            }
            let d = desc.trim();
            if d.is_empty() {
                Some(format!("- {n}"))
            } else {
                Some(format!("- {n}: {d}"))
            }
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "Available opt-in skills — read the matching .catalyst-code/skills/<name>/SKILL.md with read_file when a task fits one:\n{}",
        lines.join("\n")
    )
}

/// Build and emit a `skills` event listing every discoverable skill (project
/// then user scope) with its name, description, location, and parsed body
/// content. The TUI/web use name+description for the `/skill:<name>`
/// autocomplete; `apply_skill` re-reads the body from disk at invocation time,
/// so the body here lets a frontend optionally inline content without a second
/// round-trip. Called on `init` and `list_skills`.
fn emit_skills_event(workspace: &std::path::Path) {
    let skills = subagent::discover_skills_full(workspace);
    let arr: Vec<Value> = skills
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "location": s.location,
                "content": s.body,
            })
        })
        .collect();
    emit(&Event::new("skills").with("skills", json!(arr)));
}

/// Build the JSON array of first-party provider presets for the `ready` and
/// `provider_presets` events. Each entry tells the client whether a key is
/// already available (env var set, or a literal key in the provider config)
/// and whether the provider is already logged in — so a picker can show
/// "log in" vs "log out" and warn when a key is missing.
fn provider_presets_json(cfg: &Config, pm: Option<&plugins::PluginManager>) -> Vec<Value> {
    let mut out: Vec<Value> = config::PROVIDER_PRESETS
        .iter()
        .map(|p| {
            let configured = cfg.find_provider(p.id).is_some();
            // Auth available = API key (env/config literal) OR reusable OAuth
            // creds. OpenAI/Codex uses only this app's OAuth store; no
            // ~/.codex/auth.json auto-detect.
            let has_oauth = match p.id {
                "openai" => oauth::has_codex_creds(),
                "gemini" => oauth::has_google_creds(),
                "anthropic" => oauth::has_claude_creds(),
                "xai" => oauth::has_xai_creds(),
                "qwen" => oauth::has_qwen_creds(),
                "github" => oauth::has_github_creds(),
                "kimi-coding" => oauth::has_kimi_coding_creds(),
                "kilocode" => oauth::has_kilocode_creds(),
                "cline" => oauth::has_cline_creds(),
                "clinepass" => oauth::has_clinepass_creds(),
                "kimchi" => oauth::has_kimchi_creds(),
                "codebuddy-cn" => oauth::has_codebuddy_creds(),
                "iflow" => oauth::has_iflow_creds(),
                _ => false,
            };
            let has_key = p.env_key().is_some()
                || cfg
                    .find_provider(p.id)
                    .and_then(|pc| pc.api_key.clone().filter(|s| !s.is_empty()))
                    .is_some()
                || has_oauth;
            let logged_in = configured && has_key;
            json!({
                "id": p.id,
                "label": p.label,
                "kind": p.kind.as_str(),
                "base_url": p.base_url,
                "envVar": p.api_key_env,
                "altEnvs": p.alt_envs,
                "description": p.description,
                "hasKey": has_key,
                "configured": configured,
                "loggedIn": logged_in,
                "supportsOauth": oauth::supports_login(p.id),
            })
        })
        .collect();
    // Append plugin-declared OAuth providers so they appear in the /login picker
    // (built-in presets win on a colliding id).
    if let Some(pm) = pm {
        for c in pm.oauth_configs() {
            if config::PROVIDER_PRESETS
                .iter()
                .any(|p| p.id == c.provider_id)
            {
                continue;
            }
            let configured = cfg.find_provider(&c.provider_id).is_some();
            let has_key = pm.has_oauth_creds(&c.provider_id);
            out.push(json!({
                "id": c.provider_id,
                "label": c.label,
                "kind": c.kind.as_str(),
                "base_url": c.base_url,
                "envVar": null,
                "altEnvs": [],
                "description": c.description,
                "hasKey": has_key,
                "configured": configured,
                "loggedIn": configured && has_key,
                "supportsOauth": true,
            }));
        }
    }
    out
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

/// A pending `ask` tool call the user must answer before the model continues.
/// Mirrors PendingApproval but carries arbitrary structured answers back.
#[allow(dead_code)]
pub struct PendingAsk {
    request_id: String,
    /// The validated questions array sent to the TUI in the `ask_request`
    /// event (and used to format the model-facing result).
    questions: Value,
    notify: Arc<Notify>,
    /// None = awaiting. Some(obj) = answered (obj maps question id → answer).
    /// Some(Value::Null) = the user skipped the whole prompt.
    answers: Mutex<Option<Value>>,
}

/// A pending sudo approval: the agent wants to run a bash command that invokes
/// `sudo`. The user must approve (supplying a password) or decline (Esc). The
/// password is used once to feed `sudo -S` on stdin and is never stored.
#[allow(dead_code)]
pub struct PendingSudo {
    request_id: String,
    /// The full command string, shown to the user so they know what they're
    /// approving.
    command: String,
    notify: Arc<Notify>,
    /// None = awaiting. Some(Some(pw)) = approved with password.
    /// Some(None) = declined (Esc). The outer Option is the "resolved" flag.
    result: Mutex<Option<Option<String>>>,
}

pub struct State {
    pub cfg: RwLock<Config>,
    /// The shared HTTP client. Held on State so per-turn resolution can do
    /// async OAuth token refresh (Gemini gcloud ADC / Claude CLI creds) without
    /// threading the client through every call site.
    pub client: reqwest::Client,
    /// Per-provider runtime API keys (set via `set_key {provider,api_key}`).
    /// Keyed by provider name; the active provider's key (if present) wins over
    /// config literals/env vars during resolution. The "default" slot holds the
    /// legacy single key when no providers are configured.
    pub api_keys: RwLock<HashMap<String, String>>,
    /// Runtime override of the active provider name (set via `set_provider`).
    /// Wins over `cfg.active_provider`; None => use config's active provider.
    pub active_provider: RwLock<Option<String>>,
    pub conversation: Mutex<Vec<Message>>,
    pub models: RwLock<Vec<ModelInfo>>,
    pub current: Mutex<Option<CancellationToken>>,
    pub handle: Mutex<Option<JoinHandle<()>>>,
    /// Pending approval requests keyed by their unique approval id (see
    /// APPROVAL_SEQ) so parallel subagents can't clobber each other's request.
    pub pending: Mutex<std::collections::HashMap<String, Arc<PendingApproval>>>,
    /// Pending `ask` tool calls keyed by their unique ask id (see ASK_SEQ).
    pub pending_asks: Mutex<std::collections::HashMap<String, Arc<PendingAsk>>>,
    /// Pending sudo approval requests keyed by their unique sudo id (see
    /// SUDO_SEQ). A bash command that invokes `sudo` blocks here until the user
    /// approves (with password) or declines (Esc).
    pub pending_sudos: Mutex<std::collections::HashMap<String, Arc<PendingSudo>>>,
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
    /// to .catalyst-code/vision.json; merged into the pre_turn hook context.
    pub vision: RwLock<VisionConfig>,
    /// Last time a turn completed (for idle compaction).
    pub last_turn_time: Mutex<std::time::Instant>,
    /// Incrementally maintained token estimate for the main conversation,
    /// updated on every push + recalculated after compaction.
    pub estimated_tokens: Mutex<u64>,
    /// Real `prompt_tokens` from the endpoint's most recent `usage` chunk — the
    /// authoritative count of the conversation exactly as the model tokenized it
    /// (system prompt + messages + tool-call framing the char/4 heuristic
    /// cannot see). Anchors `grounded_estimate` so the compaction trigger and the
    /// footer percentage reflect reality instead of a whole-history char/4 guess.
    /// `None` until the first request that reports usage, and reset whenever
    /// history is rewritten (compaction/digest/reset/undo/load/refresh) so the
    /// baseline never describes stale content.
    pub last_real_prompt_tokens: Mutex<Option<u64>>,
    /// Conversation length (message count) captured when
    /// `last_real_prompt_tokens` was recorded. `grounded_estimate` only
    /// char/4-estimates the messages added since this index, keeping the real
    /// baseline accurate across tool-use loop iterations.
    pub conv_len_at_last_real: Mutex<usize>,
    /// The model id the user last sent a turn with. Used by the manual `/compact`
    /// command to pick the right context window (instead of a hardcoded 200k)
    /// and to size the reclaim budget. `None` until the first turn.
    pub last_model: Mutex<Option<String>>,
    /// Metrics from the most recently completed turn (tokens, TTFT, TPS, cache
    /// hits). Surfaced to `session_stop` lifecycle hooks so a telemetry plugin
    /// can aggregate per-turn signal out-of-the-box (without the debug log).
    /// `None` until the first turn completes.
    pub last_turn_metrics: Mutex<Option<TurnMetrics>>,

    /// Rolling, KV-cache-aware work-state summary (goal / done / in-progress /
    /// next / recent files). Maintained incrementally from conversation signals
    /// and injected as a TRANSIENT tail system message before every request —
    /// never persisted — so it never invalidates the cached conversation prefix.
    /// See the `WorkState` block comment for the full cache strategy.
    pub work_state: Mutex<WorkState>,
    /// First-class goal mode (plan → deploy subagents). See `goal.rs`.
    pub goal: Mutex<goal::GoalMode>,
    /// Cancel token for an in-flight goal deploy task (separate from the
    /// planning turn's token so cancel_goal can stop deploy without racing
    /// the turn join handle).
    pub goal_deploy_cancel: Mutex<Option<CancellationToken>>,
    /// Intercom bus: in-process mailboxes for subagent ↔ orchestrator and
    /// subagent ↔ subagent coordination.
    pub intercom: IntercomBus,
    /// Tracked subagent runs for status/interrupt/resume (keyed by run id).
    pub subagent_runs: Mutex<std::collections::HashMap<String, subagent::SubagentRun>>,
    /// Pending no-browser OAuth login state (PKCE verifier + redirect_uri),
    /// set when `/login` picks the manual flow (SSH/headless) and consumed by
    /// the `oauth_code` command when the user pastes the code.
    pub pending_oauth: Mutex<Option<oauth::PendingOauth>>,
    /// Cached live peer sessions in this workspace, refreshed every heartbeat
    /// (~8s) by the presence task. Kept in-memory so the anomaly nudge in
    /// `run_turn` can check for concurrent activity WITHOUT a filesystem read
    /// on every tool result (the hot path). Empty when alone. See `presence`.
    pub peers: Mutex<Vec<presence::PresenceRecord>>,
    /// Last time the concurrency anomaly note was emitted, for per-session
    /// rate-limiting so a pathological tool-call loop can't nag every result.
    pub last_concurrency_note: Mutex<Option<std::time::Instant>>,
}

/// Shared tail of `login_oauth` (web flow) and `oauth_code` (manual flow):
/// ensure the provider is configured (no api_key — the token is resolved +
/// refreshed at turn time by enrich_oauth), set it active, persist, emit the
/// success + provider_changed events, and refresh the model list.
/// Uses the free `protocol::emit` so this is safe to call from a `tokio::spawn`
/// task (no non-Send `&dyn Fn` borrow).
async fn finalize_oauth(state: &State, client: &reqwest::Client, preset: &str, label: &str) {
    {
        let mut cfg = state.cfg.write().await;
        if cfg.find_provider(preset).is_none() {
            if let Some(p) = config::find_preset(preset) {
                // OAuth-created configs need the same provider-specific
                // transport headers as API-key configs (Copilot and Kimi are
                // validated against their official client identities).
                cfg.providers
                    .extend(config::preset_provider_configs(p, None));
            } else if let Some(p) = state.plugin_manager.oauth_provider_config(preset) {
                // A plugin-declared OAuth provider (no built-in preset): build
                // the config from the plugin's declared base_url/kind/headers.
                cfg.providers.push(p);
            }
        }
    }
    *state.active_provider.write().await = Some(preset.to_string());
    {
        let cfg = state.cfg.read().await;
        let _ = config::save_providers_config(&cfg.providers, Some(preset));
    }
    state
        .logger
        .log("login_oauth", json!({ "provider": preset }));
    emit(&Event::new("info").with(
        "message",
        json!(format!(
            "logged into {label} via OAuth — you're signed in. Pick a model with /models if needed."
        )),
    ));
    // TUI gates prompt send on `authed`; API-key login emits this, OAuth must too.
    emit(&Event::new("authed").with("ok", json!(true)));
    let rp = state.resolved_provider_enriched().await;
    // Always report has_key=true after a successful OAuth exchange — even if
    // a transient enrich glitch can't re-read the token yet (it's on disk).
    emit(
        &Event::new("provider_changed")
            .with("provider", json!(rp.name))
            .with("kind", json!(rp.kind.as_str()))
            .with("base_url", json!(rp.base_url))
            .with("has_key", json!(true)),
    );
    state.refresh_models(client).await;
    // Confirm models landed so the user isn't left staring at an empty list.
    let n = state.models.read().await.len();
    let mine = state
        .models
        .read()
        .await
        .iter()
        .filter(|m| m.provider == preset)
        .count();
    emit(&Event::new("info").with(
        "message",
        json!(format!(
            "OAuth ready: {mine} {label} model(s) available ({n} total across providers)."
        )),
    ));
}

impl State {
    /// Resolve the active provider for an API call: kind, base URL, effective
    /// API key (runtime override -> config literal -> config env var -> global
    /// env), and extra headers. Combines the config snapshot with the runtime
    /// active-provider override and per-provider keys. This is the single
    /// source of truth every provider call site uses, so switching providers
    /// (or setting a key) takes effect on the next call with no other wiring.
    ///
    /// Note: does **not** inject OAuth subscription tokens — use
    /// [`Self::resolved_provider_enriched`] for that (turns, ready/authed).
    pub async fn resolved_provider(&self) -> ResolvedProvider {
        let cfg = self.cfg.read().await;
        let active = self.active_provider.read().await.clone();
        let keys = self.api_keys.read().await.clone();
        cfg.resolve_provider_with(&keys, active.as_deref())
    }

    /// Like [`Self::resolved_provider`], then fill in a SuperGrok / Claude /
    /// Gemini / plugin OAuth bearer when no API key is configured. Use this
    /// for status (`authed` / `has_key`) and any call that must talk to the
    /// vendor — OAuth-only providers (xAI) have no env key and would otherwise
    /// look permanently signed-out.
    pub async fn resolved_provider_enriched(&self) -> ResolvedProvider {
        let rp = self.resolved_provider().await;
        oauth::enrich_oauth(rp, &self.client, Some(&self.plugin_manager)).await
    }

    /// Resolve a named provider into a `ResolvedProvider` (key included when
    /// available). Returns None when no configured provider matches the name.
    /// Used by per-model routing: a model carries its owning provider name, and
    /// the turn is sent to THAT provider's endpoint regardless of which is
    /// "active", so multiple providers can be logged in and used simultaneously.
    pub async fn resolve_provider_by_name(&self, name: &str) -> Option<ResolvedProvider> {
        let cfg = self.cfg.read().await;
        let p = cfg.find_provider(name)?.clone();
        let keys = self.api_keys.read().await;
        Some(resolve_provider_from_config(&p, &keys))
    }

    /// Find a configured Umans provider that has a usable API key, searching
    /// ALL configured providers (not just the active one) so the concurrency
    /// `/v1/usage` poll stays live even when a non-Umans provider is active but
    /// a Umans model is selected. Prefers the active/legacy provider when it is
    /// Umans (the common case); otherwise scans the configured providers.
    pub async fn umans_provider_with_key(&self) -> Option<ResolvedProvider> {
        // Prefer the active/legacy provider when it is Umans with a key.
        let active = self.resolved_provider().await;
        if provider::is_umans(&active.base_url) && active.api_key.is_some() {
            return Some(active);
        }
        // Otherwise scan every configured provider for a Umans one with a key.
        let names: Vec<String> = {
            let cfg = self.cfg.read().await;
            cfg.providers.iter().map(|p| p.name.clone()).collect()
        };
        for name in &names {
            if let Some(rp) = self.resolve_provider_by_name(name).await {
                if provider::is_umans(&rp.base_url) && rp.api_key.is_some() {
                    return Some(rp);
                }
            }
        }
        None
    }

    /// Resolve the provider that should serve a turn for `model`: look up the
    /// model in the aggregated list, route to its owning provider; fall back to
    /// the active/legacy provider when the model has no provider tag (legacy
    /// single-provider models) or its provider isn't configured. This is the
    /// per-model routing seam that lets `/models` mix models from several
    /// logged-in providers.
    pub async fn resolve_provider_for_model(&self, model: &str) -> ResolvedProvider {
        let provider_name = self
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == model)
            .map(|m| m.provider.clone())
            .filter(|s| !s.is_empty());
        if let Some(name) = provider_name {
            if let Some(rp) = self.resolve_provider_by_name(&name).await {
                return oauth::enrich_oauth(rp, &self.client, Some(&self.plugin_manager)).await;
            }
        }
        let rp = self.resolved_provider().await;
        oauth::enrich_oauth(rp, &self.client, Some(&self.plugin_manager)).await
    }

    /// The set of provider names that are "logged in": configured providers
    /// with a usable key (runtime key -> config literal -> env var). The
    /// aggregation layer discovers models only for these, so `/models` shows
    /// exactly the providers the user has authenticated. The legacy default
    /// (Umans, when no providers are configured) is included when it has a key.
    pub async fn logged_in_providers(&self) -> Vec<String> {
        let cfg = self.cfg.read().await;
        let keys = self.api_keys.read().await;
        logged_in_providers_for(&cfg, &keys, Some(&self.plugin_manager))
    }

    /// Aggregate models across ALL logged-in providers, tagging each model with
    /// its owning provider name so per-model routing works. Deduplicates by
    /// (provider, id). When no provider is logged in, falls back to a single
    /// discovery of the active/legacy provider (so first-run still shows a model
    /// list before logging in, and the unauthenticated Umans default keeps working).
    pub async fn aggregate_models(&self, client: &reqwest::Client) -> Vec<ModelInfo> {
        let cfg = self.cfg.read().await.clone();
        let keys = self.api_keys.read().await.clone();
        let active = self.active_provider.read().await.clone();
        aggregate_models_for(
            &cfg,
            &keys,
            active.as_deref(),
            client,
            Some(&self.plugin_manager),
        )
        .await
    }

    /// Re-aggregate models, store them, and emit a `models` event + a refreshed
    /// `provider_presets` event. Shared by login/logout/set_key/set_provider so
    /// every auth change keeps `/models` in sync across all logged-in providers.
    pub async fn refresh_models(&self, client: &reqwest::Client) {
        let models = self.aggregate_models(client).await;
        *self.models.write().await = models.clone();
        emit(&Event::new("models").with("models", json!(models)));
        let presets = {
            let cfg = self.cfg.read().await;
            provider_presets_json(&cfg, Some(&self.plugin_manager))
        };
        emit(&Event::new("provider_presets").with("presets", json!(presets)));
    }

    /// Drop the real-usage baseline so estimates fall back to a full char/4 of
    /// the conversation until the next request re-establishes it. Call this
    /// whenever history is rewritten/replaced — compaction, soft-digest, reset,
    /// clear, new-session, undo, load-session, memory refresh — because the old
    /// `prompt_tokens` baseline no longer describes the (now-changed) messages.
    pub async fn invalidate_real_token_baseline(&self) {
        *self.last_real_prompt_tokens.lock().await = None;
        *self.conv_len_at_last_real.lock().await = 0;
    }

    /// Build the `args` payload handed to `session_stop` lifecycle hooks (when
    /// `pass_args: true`): the cumulative session totals plus the
    /// just-completed turn's metrics. Lets a telemetry plugin aggregate
    /// per-turn signal without the JSONL debug log (off by default). `turn`
    /// is null until the first turn completes.
    pub async fn session_stop_hook_args(&self) -> Value {
        let session = json!({
            "turns": self.logger.turn_count(),
            "tokens_in": *self.tokens_in.lock().await,
            "tokens_out": *self.tokens_out.lock().await,
            "cached_tokens": *self.cached_tokens.lock().await,
            "model": self.last_model.lock().await.clone().unwrap_or_default(),
        });
        let turn = self
            .last_turn_metrics
            .lock()
            .await
            .as_ref()
            .map(|m| {
                json!({
                    "tokens_in": m.tokens_in,
                    "tokens_out": m.tokens_out,
                    "cached_tokens": m.cached_tokens,
                    "ttft_ms": m.ttft_ms,
                    "elapsed_ms": m.elapsed_ms,
                    "tps": m.tps,
                    "model": m.model,
                })
            })
            .unwrap_or(Value::Null);
        json!({ "session": session, "turn": turn })
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Multi-provider aggregation + per-model routing (free functions)
//
// These take a `&Config` + `&HashMap<String,String>` runtime keys (and an
// optional active-provider override) so they can run BEFORE the `State` exists
// (init discovery) as well as on the live state. The State wrappers above are
// thin facades over these. The harness keeps the conversation in OpenAI
// chat-completions shape internally; a provider's `kind` only decides the wire
// translation at the HTTP boundary, so routing a turn to a different provider
// needs no other change.
// ────────────────────────────────────────────────────────────────────────────

/// Resolve a `ProviderConfig` into a `ResolvedProvider` against the runtime key
/// map (runtime key -> config literal -> config env var). Empty keys are dropped.
pub fn resolve_provider_from_config(
    p: &config::ProviderConfig,
    keys: &HashMap<String, String>,
) -> ResolvedProvider {
    let api_key = keys
        .get(&p.name)
        .cloned()
        .or_else(|| p.api_key.clone())
        .or_else(|| p.api_key_env.as_ref().and_then(|v| std::env::var(v).ok()))
        .filter(|s| !s.is_empty());
    ResolvedProvider {
        name: p.name.clone(),
        kind: p.kind.clone(),
        base_url: p.base_url.clone(),
        api_key,
        headers: p.headers.clone(),
        oauth: false,
    }
}

/// Names of providers that are "logged in": configured providers with a usable
/// key. The aggregation layer discovers models only for these, so `/models`
/// shows exactly the providers the user has authenticated. When no providers are
/// configured (legacy single-endpoint Umans setup) this returns empty so that
/// `aggregate_models_for`'s `names.is_empty()` branch handles the legacy
/// default discovery — returning a synthetic "default" name here would break,
/// because `find_provider("default")` finds no explicit entry to resolve.
pub fn logged_in_providers_for(
    cfg: &Config,
    keys: &HashMap<String, String>,
    pm: Option<&plugins::PluginManager>,
) -> Vec<String> {
    if cfg.providers.is_empty() {
        return Vec::new();
    }
    cfg.providers
        .iter()
        .filter(|p| {
            // Logged in = has an API key (runtime/config/env), OR is an OAuth-capable
            // provider with reusable OAuth credentials. OpenAI/Codex only uses
            // this app's OAuth store; no ~/.codex/auth.json auto-detect.
            keys.get(&p.name)
                .cloned()
                .or_else(|| p.api_key.clone())
                .or_else(|| p.api_key_env.as_ref().and_then(|v| std::env::var(v).ok()))
                .is_some()
                || oauth_creds_for_provider(p, pm)
        })
        .map(|p| p.name.clone())
        .collect()
}

/// True when a provider's reusable OAuth credentials exist (cheap sync file
/// check, no refresh). Used by `logged_in_providers_for` to gate OAuth-only
/// providers into aggregation. The actual token refresh happens at turn/
/// discovery time via `oauth::enrich_oauth`.
fn oauth_creds_for_provider(
    p: &config::ProviderConfig,
    pm: Option<&plugins::PluginManager>,
) -> bool {
    // A plugin-declared OAuth provider owns its token check exclusively (so a
    // plugin anthropic-kind provider isn't wrongly gated on Claude CLI creds).
    if let Some(pm) = pm {
        if pm.oauth_config(&p.name).is_some() {
            return pm.has_oauth_creds(&p.name);
        }
    }
    if p.kind == config::ProviderKind::Anthropic {
        return oauth::has_claude_creds();
    }
    if provider::is_codex_endpoint(&p.base_url) {
        return oauth::has_codex_creds();
    }
    if provider::is_gemini_endpoint(&p.base_url) {
        return oauth::has_google_creds();
    }
    // xAI SuperGrok OAuth (OAuth-only preset — no API key). Without this,
    // login writes tokens + config but aggregation skips the provider, so
    // /models never shows Grok and the TUI looks like sign-in failed.
    if provider::is_xai_endpoint(&p.base_url) || p.name == "xai" {
        return oauth::has_xai_creds();
    }
    if provider::is_qwen_endpoint(&p.base_url) || p.name == "qwen" {
        return oauth::has_qwen_creds();
    }
    if provider::is_github_copilot_endpoint(&p.base_url) || p.name == "github" {
        return oauth::has_github_creds();
    }
    if provider::is_kimi_coding_endpoint(&p.base_url) || p.name == "kimi-coding" {
        return oauth::has_kimi_coding_creds();
    }
    if provider::is_kilocode_endpoint(&p.base_url) || p.name == "kilocode" {
        return oauth::has_kilocode_creds();
    }
    if provider::is_cline_endpoint(&p.base_url) || p.name == "cline" {
        return oauth::has_cline_creds();
    }
    if provider::is_cline_endpoint(&p.base_url) || p.name == "clinepass" {
        return oauth::has_clinepass_creds();
    }
    if provider::is_kimchi_endpoint(&p.base_url) || p.name == "kimchi" {
        return oauth::has_kimchi_creds();
    }
    if provider::is_codebuddy_endpoint(&p.base_url) || p.name == "codebuddy-cn" {
        return oauth::has_codebuddy_creds();
    }
    if provider::is_iflow_endpoint(&p.base_url) || p.name == "iflow" {
        return oauth::has_iflow_creds();
    }
    false
}

/// Aggregate models across ALL logged-in providers, tagging each model with its
/// owning provider name so per-model routing works. Deduplicates by (provider,
/// id). When no provider is logged in, falls back to a single discovery of the
/// active/legacy provider (first-run before login, unauthenticated Umans default).
pub async fn aggregate_models_for(
    cfg: &Config,
    keys: &HashMap<String, String>,
    active: Option<&str>,
    client: &reqwest::Client,
    pm: Option<&plugins::PluginManager>,
) -> Vec<ModelInfo> {
    let names = logged_in_providers_for(cfg, keys, pm);
    if names.is_empty() {
        let rp = cfg.resolve_provider_with(keys, active);
        let mut models = provider::discover_models(client, &rp).await;
        // Legacy/default models get the resolved provider tag so they route
        // correctly; models already tagged (e.g. by an earlier aggregation run
        // round-tripped through the session) keep their tag.
        for m in &mut models {
            if m.provider.is_empty() {
                m.provider = rp.name.clone();
            }
        }
        return models;
    }
    let mut merged: Vec<ModelInfo> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for name in &names {
        let Some(pc) = cfg.find_provider(name) else {
            continue;
        };
        let rp = resolve_provider_from_config(pc, keys);
        // Enrich with an OAuth subscription token (gcloud/`claude` CLI) when the
        // provider has no API key, so OAuth-only providers can discover models.
        let rp = oauth::enrich_oauth(rp, client, pm).await;
        let mut discovered = provider::discover_models(client, &rp).await;
        for m in &mut discovered {
            m.provider = rp.name.clone();
            if seen.insert((m.provider.clone(), m.id.clone())) {
                merged.push(m.clone());
            }
        }
    }
    merged
}

// ============================================================================
// Rolling, KV-cache-aware work-state summary
// ============================================================================
// A live summary of what the session is working on (goal / done / in-progress
// / next / recently-touched files), maintained incrementally from conversation
// signals — `todo_write` (the structured backbone), the user's first substantive
// message (the goal), and file-edit tool calls — with NO model call, so it is
// free and deterministic.
//
// It is injected as a TRANSIENT tail `system` message right before every model
// request, then stripped: never stored in `conversation`, never persisted to
// the session file. This is the KV-cache strategy:
//
//   * The persisted conversation is strictly append-only from the provider's
//     point of view, so its prefix `[system][u1][a1]…]` is byte-identical turn
//     to turn → the provider's prefix cache hits on everything already sent.
//   * The work-state is the LAST message each turn, so updating it invalidates
//     nothing earlier in the prefix; only the small work-state (~200-400
//     tokens) plus the new turn are prefilled. Contrast with injecting it into
//     the system prompt (position 0), which would invalidate the ENTIRE cache
//     on every change.
//   * Because it is transient, a resumed session never accumulates a trail of
//     stale summaries; the live state is rebuilt from the next signals.
//
// The compaction summary (model-generated, in the prefix) covers dropped
// history; this rolling state covers the CURRENT state. They complement each
// other: the deep summary is cached and changes only on compaction; the
// rolling state changes often but lives at the cheap tail.

/// Rolling work-state summary. See the block comment above for the cache
/// strategy. Updated by the signal helpers below and rendered into the
/// transient tail message by `work_state_message`.
#[derive(Clone, Default)]
pub struct WorkState {
    pub goal: String,
    pub done: Vec<String>,
    pub in_progress: Vec<String>,
    pub next: Vec<String>,
    pub recent_files: Vec<String>,
    pub last_activity: String,
    pub version: u64,
}

impl WorkState {
    /// Render as a compact, model-facing system block. Empty sections are
    /// omitted so the block stays minimal; each list is capped so a runaway
    /// plan can't bloat every request.
    pub fn render(&self) -> String {
        const MAX_LIST: usize = 6;
        const MAX_FILES: usize = 8;
        let mut out = String::from(
            "[Work state — ambient status the harness keeps current via todo_write \
             and file edits. Use it as context; respond to the user's latest message, \
             not to this block. Keep it accurate by updating todos as you work.]",
        );
        out.push_str("\nGoal: ");
        let goal = if self.goal.is_empty() {
            "(not yet stated)".to_string()
        } else {
            truncate_str(self.goal.as_str(), 240)
        };
        out.push_str(&goal);
        {
            let mut section = |label: &str, items: &[String]| {
                if items.is_empty() {
                    return;
                }
                out.push('\n');
                out.push_str(label);
                for it in items.iter().take(MAX_LIST) {
                    out.push_str("\n- ");
                    out.push_str(&truncate_str(it, 160));
                }
                if items.len() > MAX_LIST {
                    out.push_str(&format!("\n- … +{} more", items.len() - MAX_LIST));
                }
            };
            section("Done:", &self.done);
            section("In progress:", &self.in_progress);
            section("Next:", &self.next);
        }
        if !self.recent_files.is_empty() {
            out.push_str("\nRecently touched: ");
            let files: Vec<String> = self
                .recent_files
                .iter()
                .take(MAX_FILES)
                .map(|s| truncate_str(s, 120))
                .collect();
            out.push_str(&files.join(", "));
        }
        if !self.last_activity.is_empty() {
            out.push_str("\nLast: ");
            out.push_str(&truncate_str(&self.last_activity, 160));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.goal.is_empty()
            && self.done.is_empty()
            && self.in_progress.is_empty()
            && self.next.is_empty()
            && self.recent_files.is_empty()
            && self.last_activity.is_empty()
    }

    fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Partition a `todo_write` payload into done/in-progress/next. Pure logic
    /// (no locking/emit) so it is unit-testable; the async wrapper adds those.
    pub fn sync_from_todos(&mut self, todos: &[Value]) {
        let mut done = Vec::new();
        let mut in_progress = Vec::new();
        let mut next = Vec::new();
        for t in todos {
            let subject = t.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            if subject.is_empty() {
                continue;
            }
            match t.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                "completed" => done.push(subject.to_string()),
                "in_progress" => in_progress.push(subject.to_string()),
                _ => next.push(subject.to_string()),
            }
        }
        self.done = done;
        self.in_progress = in_progress;
        self.next = next;
        self.touch();
    }

    /// Record file paths touched (most-recent-first, deduped, capped) and a
    /// short last-activity note. Pure logic; the async wrapper extracts paths.
    pub fn record_files(&mut self, tool: &str, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        // Iterate in reverse so the FIRST-listed (primary) path lands at the
        // front of the most-recent-first list — "Recently touched: a.rs, b.rs"
        // reads naturally when a.rs was the edit's primary target.
        for p in paths.iter().rev() {
            if let Some(pos) = self.recent_files.iter().position(|x| x == p) {
                self.recent_files.remove(pos);
            }
            self.recent_files.insert(0, p.clone());
        }
        self.recent_files.truncate(8);
        let act = format!("{} {}", tool, paths.join(", "));
        self.last_activity = truncate_str(&act, 160);
        self.touch();
    }
}

/// Emit a `work_state` event with the current rolling summary so the TUI/web
/// can render a live status panel alongside the conversation.
async fn emit_work_state(st: &State) {
    let ws = st.work_state.lock().await.clone();
    emit(
        &Event::new("work_state")
            .with("version", json!(ws.version))
            .with("goal", json!(ws.goal))
            .with("done", json!(ws.done))
            .with("in_progress", json!(ws.in_progress))
            .with("next", json!(ws.next))
            .with("recent_files", json!(ws.recent_files))
            .with("last_activity", json!(ws.last_activity)),
    );
}

/// Cancel an in-flight goal deploy task (if any).
async fn cancel_goal_deploy(st: &State) {
    if let Some(tok) = st.goal_deploy_cancel.lock().await.take() {
        tok.cancel();
    }
}

/// Spawn the deterministic goal deploy loop on a child cancel token.
async fn spawn_goal_deploy(st: Arc<State>, client: reqwest::Client) {
    cancel_goal_deploy(&st).await;
    let tok = CancellationToken::new();
    *st.goal_deploy_cancel.lock().await = Some(tok.clone());
    tokio::spawn(async move {
        goal::deploy_goal(st.clone(), client, tok.clone()).await;
        // Clear cancel slot if we still own it.
        let mut slot = st.goal_deploy_cancel.lock().await;
        if slot.as_ref().is_some_and(|t| t.is_cancelled() || true) {
            // Always clear when the deploy task exits.
            *slot = None;
        }
    });
}

/// After a planning turn ends: fail if no plan, or kick deploy if requested.
async fn maybe_finish_goal_planning(st: &Arc<State>, client: &reqwest::Client, cancelled: bool) {
    let should_deploy = {
        let mut g = st.goal.lock().await;
        // Deploy path: plan was written this turn (or earlier) and auto-deploy is armed.
        if g.deploy_after_turn && g.plan.is_some() && !g.prompts.is_empty() {
            g.deploy_after_turn = false;
            true
        } else if g.phase == goal::GoalPhase::Planning {
            if cancelled {
                goal::fail_goal(&mut g, "planning aborted");
            } else if g.plan.is_none() {
                goal::fail_goal(
                    &mut g,
                    "planning turn ended without goal_write_plan — re-run /goal",
                );
            }
            false
        } else {
            false
        }
    };
    if should_deploy {
        spawn_goal_deploy(st.clone(), client.clone()).await;
    }
}

/// Seed the work-state goal from a user prompt (the first substantive message).
/// Subsequent calls are no-ops once a goal is set, so the goal reflects the
/// session's original intent rather than every follow-up. Slash commands and
/// trivially short prompts are ignored so they don't pin the goal.
async fn maybe_seed_work_state_goal(st: &State, prompt: &str) {
    let p = prompt.trim();
    if p.is_empty() || p.starts_with('/') || p.chars().count() < 8 {
        return;
    }
    let mut ws = st.work_state.lock().await;
    if !ws.goal.is_empty() {
        return;
    }
    ws.goal = truncate_str(p.lines().next().unwrap_or(p), 240);
    ws.touch();
    drop(ws);
    emit_work_state(st).await;
}

/// Mirror a `todo_write` payload into the work-state's done/in-progress/next
/// lists. The todo list IS the structured work state; this keeps the rolling
/// summary in sync so the model sees current progress every turn without a
/// `todo_read` round-trip.
async fn sync_work_state_from_todos(st: &State, args: &Value) {
    let Some(todos) = args.get("todos").and_then(|v| v.as_array()) else {
        return;
    };
    let mut ws = st.work_state.lock().await;
    ws.sync_from_todos(todos);
    drop(ws);
    emit_work_state(st).await;
}

/// Record a file touch from a write/edit/patch/bulk_* call into the work-state
/// recent-files list (most-recent-first, deduped, capped). Keeps the rolling
/// summary aware of what the session has actually changed.
async fn record_file_touch(st: &State, tool: &str, args: &Value) {
    let paths: Vec<String> = match tool {
        "bulk_write" | "bulk_edit" => args
            .get("edits")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("path").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
    };
    if paths.is_empty() {
        return;
    }
    let mut ws = st.work_state.lock().await;
    ws.record_files(tool, &paths);
    drop(ws);
    emit_work_state(st).await;
}

/// Cooldown between concurrency-anomaly notes (seconds). Prevents nagging in a
/// tight tool-call loop; one nudge per minute is enough to change behavior from
/// "fix the phantom error" to "check the neighbors".
const CONCURRENCY_NOTE_COOLDOWN: u64 = 60;

/// If another session is active in this workspace AND something looks off (a
/// tool failed, or we're touching a file a peer recently touched), surface a
/// short note appended to the tool result — so the agent doesn't assume every
/// error is its own fault and "fix" a neighbor's in-flight work. Uses the cached
/// peer snapshot (refreshed by the heartbeat) so the hot path does NO filesystem
/// read. Rate-limited per session to avoid nagging.
async fn maybe_concurrency_note(
    st: &State,
    tool_name: &str,
    args: &Value,
    outcome_ok: bool,
) -> Option<String> {
    // Cooldown: at most one note per window. Checked first (before cloning the
    // peer snapshot) so a tight loop short-circuits cheaply after the first nudge.
    {
        let last = st.last_concurrency_note.lock().await;
        if let Some(t) = *last {
            if t.elapsed() < std::time::Duration::from_secs(CONCURRENCY_NOTE_COOLDOWN) {
                return None;
            }
        }
    }
    let peers = st.peers.lock().await.clone();
    if peers.is_empty() {
        return None; // alone — nothing to surface; don't arm the cooldown
    }
    let touching = peers_touching(&peers, tool_name, args);
    // (a) a tool failed — could be a neighbor leaving the tree inconsistent.
    if !outcome_ok {
        *st.last_concurrency_note.lock().await = Some(std::time::Instant::now());
        let mut s = format!(
            "⚠ {} other agent session(s) are active in this workspace. This error may \
             not be from your changes — another session may have left the tree in an \
             inconsistent state. Consider `workspace_activity` to inspect before \
             'fixing' it.",
            peers.len()
        );
        if !touching.is_empty() {
            s.push_str(&format!(" Active sessions recently touched: {touching}."));
        }
        return Some(s);
    }
    // (b) a file tool touching a path a peer recently touched (a real conflict).
    if !touching.is_empty() {
        *st.last_concurrency_note.lock().await = Some(std::time::Instant::now());
        return Some(format!(
            "ℹ Another agent session in this workspace recently touched: {touching}. \
             You may be reading/editing in-flight work — consider `workspace_activity` \
             to coordinate."
        ));
    }
    None
}

/// Return a comma-list of "pid N" for peers whose recent_files contain the
/// current tool's target path. Exact separator-normalized match — precise to
/// avoid false-positive nagging; a miss just means no warning (safe).
fn peers_touching(peers: &[presence::PresenceRecord], tool_name: &str, args: &Value) -> String {
    let path = match tool_name {
        "read_file" | "edit" | "write_file" | "patch" | "bulk_read" | "bulk_write"
        | "bulk_edit" => args.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        _ => "",
    };
    if path.is_empty() {
        return String::new();
    }
    let target = path.replace('\\', "/");
    let hitting: Vec<_> = peers
        .iter()
        .filter(|p| {
            p.recent_files
                .iter()
                .any(|f| f.replace('\\', "/") == target)
        })
        .map(|p| format!("pid {}", p.pid))
        .collect();
    hitting.join(", ")
}

/// Build the transient work-state system message, or `None` when disabled or
/// when there is no state to show yet. The caller pushes it as the LAST message
/// before the model request and pops it right after, so it never reaches the
/// persisted conversation or the session file — the conversation prefix stays
/// byte-identical turn to turn and the provider's prefix cache is never
/// invalidated by it.
async fn work_state_message(st: &State) -> Option<Message> {
    if !st.cfg.read().await.rolling_state {
        return None;
    }
    let ws = st.work_state.lock().await;
    if ws.is_empty() {
        return None;
    }
    Some(Message::system(ws.render()))
}

/// Reset the rolling work-state (new session / reset / clear / undo / load).
/// Emits an empty `work_state` so frontends clear their panel. Also clears
/// goal mode so a new session doesn't inherit a stale plan/deploy.
async fn clear_work_state(st: &State) {
    cancel_goal_deploy(st).await;
    {
        let mut g = st.goal.lock().await;
        if g.phase != goal::GoalPhase::Idle {
            goal::clear_goal(&mut g);
        }
    }
    *st.work_state.lock().await = WorkState::default();
    emit_work_state(st).await;
}

/// Persist cumulative session stats to the `<session>.stats` sidecar so `/stats`
/// survives a restart. Called at turn completion (after `record_turn`).
async fn persist_stats(st: &State) {
    let Some(p) = st.cfg.read().await.session_file.clone() else {
        return;
    };
    let stats = session::SessionStats {
        tokens_in: *st.tokens_in.lock().await,
        tokens_out: *st.tokens_out.lock().await,
        cached_tokens: *st.cached_tokens.lock().await,
        turns: st.logger.turn_count(),
    };
    session::save_stats(&p, &stats);
}

/// Zero the cumulative stats in memory and on the sidecar (reset / clear /
/// new session) so a fresh conversation doesn't carry a prior session's totals.
async fn reset_stats(st: &State) {
    *st.tokens_in.lock().await = 0;
    *st.tokens_out.lock().await = 0;
    *st.cached_tokens.lock().await = 0;
    st.logger.set_turns(0);
    if let Some(p) = st.cfg.read().await.session_file.clone() {
        session::save_stats(&p, &session::SessionStats::default());
    }
}

/// Restore cumulative stats from a session's sidecar into memory (load_session /
/// init), so switching/resuming a session shows its real totals.
async fn restore_stats(st: &State, session_path: &std::path::Path) {
    let s = session::load_stats(session_path);
    *st.tokens_in.lock().await = s.tokens_in;
    *st.tokens_out.lock().await = s.tokens_out;
    *st.cached_tokens.lock().await = s.cached_tokens;
    st.logger.set_turns(s.turns);
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
    // One-time rename migration: move the pre-rename on-disk layout
    // (~/.config/umans-harness/, ~/.umans-harness/) to the current names
    // (~/.config/catalyst-code/, ~/.catalyst-code/), preserving sessions,
    // memory, OAuth tokens, settings, and staged agent/skill/plugin files.
    // Runs before staging + config load so this run sees the migrated data.
    staging::migrate_legacy_dirs();
    // Stage the harness's global defaults (agents, orchestrator skill,
    // vision-handoff plugin) into ~/.catalyst-code/ on first run — shared
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
    let mut cfg = config::load();
    // Auto-log-in to every first-party preset whose key is already in the
    // environment (UMANS_API_KEY, OPENAI_API_KEY, ...), so providers show as
    // logged in and their models appear in /models without a manual /login.
    let auto_logged = config::auto_login_env_presets(&mut cfg);
    if !auto_logged.is_empty() {
        eprintln!("[umans] auto-logged in: {}", auto_logged.join(", "));
    }
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("client");

    // Discover models up front. In the multi-login model, models are aggregated
    // across all logged-in providers (configured + key available) so `/models`
    // can mix providers. At init there are no runtime keys yet beyond the
    // persisted ones already in cfg, so this resolves from config/env.
    let init_provider = cfg.resolve_provider(&HashMap::new());
    let init_keys = cfg.persisted_keys.clone();
    let models = aggregate_models_for(
        &cfg,
        &init_keys,
        cfg.active_provider.as_deref(),
        &client,
        None,
    )
    .await;
    let logger = Logger::new(cfg.debug_log.as_deref());
    logger.log("init", json!({ "workspace": cfg.workspace.display().to_string(), "provider": init_provider.name, "kind": init_provider.kind.as_str(), "base_url": init_provider.base_url, "approval": cfg.approval.as_str() }));

    // Resume session if configured and present. A future-version session file
    // returns Err (surfaced to the user via an `error` event at Init) rather
    // than silently starting blank.
    let (resumed, session_error): (Vec<Message>, Option<String>) = match cfg.session_file.as_ref() {
        Some(p) => match session::load(p.as_path()) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(e)),
        },
        None => (Vec::new(), None),
    };
    // Persisted cumulative stats travel with the session file (sidecar
    // <session>.stats) so `/stats` survives a restart — previously in-memory
    // only, so reopening showed zeros for tokens/turns.
    let init_stats: session::SessionStats = cfg
        .session_file
        .as_ref()
        .map(|p| session::load_stats(p.as_path()))
        .unwrap_or_default();
    logger.set_turns(init_stats.turns);
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

    let vision_cfg = VisionConfig::load(&cfg.workspace);
    let state = Arc::new(State {
        cfg: RwLock::new(cfg),
        client: client.clone(),
        api_keys: RwLock::new(HashMap::new()),
        active_provider: RwLock::new(None),
        conversation: Mutex::new(resumed),
        models: RwLock::new(models),
        current: Mutex::new(None),
        handle: Mutex::new(None),
        pending: Mutex::new(std::collections::HashMap::new()),
        pending_asks: Mutex::new(std::collections::HashMap::new()),
        pending_sudos: Mutex::new(std::collections::HashMap::new()),
        logger,
        tokens_in: Mutex::new(init_stats.tokens_in),
        tokens_out: Mutex::new(init_stats.tokens_out),
        cached_tokens: Mutex::new(init_stats.cached_tokens),
        escalated_kinds: Mutex::new(init_escalations),
        queued: Mutex::new(None),
        plugin_manager: PluginManager::new_with_global_plugins(
            plugin_dir,
            pm_workspace,
            trust_project,
        ),
        vision: RwLock::new(vision_cfg),
        last_turn_time: Mutex::new(std::time::Instant::now()),
        estimated_tokens: Mutex::new(init_est),
        last_real_prompt_tokens: Mutex::new(None),
        conv_len_at_last_real: Mutex::new(0),
        last_model: Mutex::new(None),
        last_turn_metrics: Mutex::new(None),

        work_state: Mutex::new(WorkState::default()),
        goal: Mutex::new(goal::GoalMode::default()),
        goal_deploy_cancel: Mutex::new(None),
        intercom: IntercomBus::new(),
        subagent_runs: Mutex::new(std::collections::HashMap::new()),
        pending_oauth: Mutex::new(None),
        peers: Mutex::new(Vec::new()),
        last_concurrency_note: Mutex::new(None),
    });

    // Apply disabled plugin list from config.
    {
        let cfg = state.cfg.read().await;
        for name in &cfg.plugins_disabled {
            let _ = state.plugin_manager.disable(name);
        }
    }

    // Seed runtime API keys from the TUI-persisted `provider_keys`/`api_key`
    // (read from settings.json by Config::load). A key set via `/key` or the
    // settings modal is saved by the TUI into settings.json; loading it here
    // makes it survive a restart and take precedence over provider config/env
    // keys (runtime keys are checked first in provider resolution).
    {
        let cfg = state.cfg.read().await;
        for (name, key) in cfg.persisted_keys.iter() {
            state
                .api_keys
                .write()
                .await
                .insert(name.clone(), key.clone());
        }
    }

    // Background poll of the Umans gateway's `/v1/usage` endpoint so the footer
    // can show a LIVE, account-wide concurrency usage (used/limit) ahead of tps.
    // Updated every few seconds, independent of turns. Polls ANY configured Umans
    // provider that has a key (not just the active one) so conc stays live even
    // when a non-Umans provider is active but a Umans model is selected. Emits
    // `umans_conc { used, limit, provider }` — `provider` is the Umans provider
    // name it polled, so the UI only renders the field when the SELECTED model
    // routes to that provider (a Gemini/OpenAI model selected → hidden). Both
    // null + no provider when no Umans provider is available, to clear the UI.
    {
        let st = state.clone();
        let cl = client.clone();
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(5);
            let mut last_provider: Option<String> = None;
            loop {
                match st.umans_provider_with_key().await {
                    Some(rp) => {
                        let name = rp.name.clone();
                        let (used, limit) = match rp.api_key.as_deref() {
                            Some(k) => {
                                match provider::fetch_umans_usage(&cl, &rp.base_url, k).await {
                                    Some(u) => (u.used, u.limit),
                                    None => (None, None),
                                }
                            }
                            None => (None, None),
                        };
                        let used_v = used.map(Value::from).unwrap_or(Value::Null);
                        let limit_v = limit.map(Value::from).unwrap_or(Value::Null);
                        emit(
                            &Event::new("umans_conc")
                                .with("used", used_v)
                                .with("limit", limit_v)
                                .with("provider", json!(name)),
                        );
                        last_provider = Some(name);
                    }
                    None => {
                        if last_provider.take().is_some() {
                            emit(
                                &Event::new("umans_conc")
                                    .with("used", Value::Null)
                                    .with("limit", Value::Null),
                            );
                        }
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Cross-session presence: publish this session's rolling work-state so other
    // processes in the SAME workspace can detect concurrent activity (and stop
    // "fixing" phantom errors caused by a neighbor's in-flight edits). Per-pid
    // JSON file under ~/.config/catalyst-code/presence/<hash(cwd)>/, rewritten
    // every few seconds; stale records reaped by readers. Awareness only — no
    // coordination/locking. The `workspace_activity` tool + the anomaly nudge
    // in `run_turn` consume this; the cached peer snapshot avoids a filesystem
    // read on every tool result.
    {
        let st = state.clone();
        let pid = std::process::id();
        let started = presence::unix_now();
        let presence_ws = {
            let cfg = state.cfg.read().await;
            cfg.workspace.clone()
        };
        // Publish immediately so a peer checking right after we start sees us.
        {
            let ws = st.work_state.lock().await;
            let session_id = st
                .cfg
                .read()
                .await
                .session_file
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(String::from);
            let model = st.last_model.lock().await.clone();
            let rec =
                presence::PresenceRecord::from_work_state(&ws, pid, session_id, model, started);
            drop(ws);
            presence::write_presence(&presence_ws, pid, &rec);
        }
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(8);
            loop {
                tokio::time::sleep(interval).await;
                let ws = st.work_state.lock().await;
                let session_id = st
                    .cfg
                    .read()
                    .await
                    .session_file
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(String::from);
                let model = st.last_model.lock().await.clone();
                let rec =
                    presence::PresenceRecord::from_work_state(&ws, pid, session_id, model, started);
                drop(ws);
                presence::write_presence(&presence_ws, pid, &rec);
                // Refresh the cached peer snapshot so the anomaly nudge stays
                // current without a filesystem read on the hot path.
                *st.peers.lock().await = presence::read_peers(&presence_ws, pid);
            }
        });
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
                // Enrich OAuth so SuperGrok / Claude / Gemini look signed-in on
                // startup (they store tokens on disk, not as api_key in config).
                let rp = state.resolved_provider_enriched().await;
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
                        .with(
                            "providerPresets",
                            json!(provider_presets_json(&cfg, Some(&state.plugin_manager))),
                        )
                        .with("bash_timeout_secs", json!(cfg.bash_timeout_secs))
                        .with("auto_compact", json!(cfg.auto_compact))
                        .with("resumed_messages", json!(conv_len)),
                );
                // Tell the user when the harness staged its global defaults
                // (first run) so the global ~/.catalyst-code/ layout is
                // discoverable.
                if stage.first_run {
                    emit(
                        &Event::new("info").with(
                            "message",
                            json!(format!(
                                "First run: staged {} default file(s) into {} — agents, the pi-subagents skill, and the vision-handoff plugin now live globally and are shared across all projects. Edit them there to customize; drop a file in a project's own .catalyst-code/ to override for that project only.",
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
                // Publish the discoverable-skills list so the TUI/web can
                // populate their `/skill:<name>` autocomplete immediately.
                emit_skills_event(&cfg.workspace);
                // Replay any resumed conversation so the TUI shows prior history
                // on launch instead of starting from an empty transcript.
                if conv_len > 0 {
                    let conv = state.conversation.lock().await;
                    let visible: Vec<Value> = conv
                        .iter()
                        .filter(|m| !m.is_system())
                        .map(Value::from)
                        .collect();
                    let est = estimate_messages_tokens(&conv);
                    emit(
                        &Event::new("history")
                            .with("messages", json!(visible))
                            .with("tokens_in", json!(est)),
                    );
                    // The conversation was left mid-`ask` by a prior core
                    // restart (assistant `ask` tool_call with no tool result).
                    // Tell the user a message will re-present the question so
                    // they don't see a wedged transcript with no explanation.
                    if find_trailing_unanswered_ask(&conv[..]).is_some() {
                        emit(&Event::new("info").with(
                            "message",
                            json!(
                                "A question from the previous session was interrupted by a restart. Send any message to answer it and continue."
                            ),
                        ));
                    }
                }
            }
            Command::SetKey { api_key, provider } => {
                // Apply the key to a named provider, or to the active provider
                // when no name is given (backward-compatible with the pre-provider
                // single-key flow, which lands in the "default" slot). Setting a
                // key "logs in" that provider, so re-aggregate models so its
                // models appear in `/models` alongside any others logged in.
                let name = match provider {
                    Some(p) => p,
                    None => state.resolved_provider().await.name,
                };
                state.api_keys.write().await.insert(name.clone(), api_key);
                state.logger.log("set_key", json!({ "provider": name }));
                emit(
                    &Event::new("authed")
                        .with("ok", json!(true))
                        .with("provider", json!(name)),
                );
                state.refresh_models(&client).await;
            }
            Command::SetProvider { name } => {
                // Set the default/fallback provider. In the multi-login model a
                // turn routes to the selected model's provider; this only matters
                // for model-less operations (compaction summarize) and legacy
                // models without a provider tag. Re-aggregate (don't wipe other
                // providers' models). Unknown names are ignored (stays put).
                {
                    let cfg = state.cfg.read().await;
                    if cfg.find_provider(&name).is_none() {
                        emit(&Event::new("error").with(
                            "message",
                            json!(format!("unknown provider '{name}'; not switching")),
                        ));
                        return;
                    }
                }
                *state.active_provider.write().await = Some(name.clone());
                let rp = state.resolved_provider_enriched().await;
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
                if rp.api_key.is_some() {
                    emit(
                        &Event::new("authed")
                            .with("ok", json!(true))
                            .with("provider", json!(rp.name)),
                    );
                }
                state.refresh_models(&client).await;
            }
            Command::ListProviderPresets => {
                let cfg = state.cfg.read().await;
                emit(&Event::new("provider_presets").with(
                    "presets",
                    json!(provider_presets_json(&cfg, Some(&state.plugin_manager))),
                ));
            }
            Command::Login { preset, api_key } => {
                // Log in to a first-party provider from a preset: resolve the key
                // (explicit arg → preset env var), insert/replace into config,
                // seed the runtime key, persist, and re-aggregate models across
                // all logged-in providers so this provider's models join `/models`.
                // Multiple providers can be logged in at once. Most presets create
                // one config; OpenCode Go creates two (OpenAI-kind +
                // Anthropic-kind) sharing the base URL + key.
                let Some(p) = config::find_preset(&preset) else {
                    let available = config::PROVIDER_PRESETS
                        .iter()
                        .map(|p| p.id)
                        .collect::<Vec<_>>()
                        .join(", ");
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "unknown provider preset '{preset}'; available: {available}"
                        )),
                    ));
                    return;
                };
                // API-key path: accept an explicit key or env key. OAuth-capable
                // presets (xAI, Qwen, OpenAI/Gemini/Claude) also work with no
                // key via /login's OAuth branch when supports_login is true.
                let key = api_key.or_else(|| p.env_key());
                let configs = config::preset_provider_configs(p, key.clone());
                let name = configs[0].name.clone();
                // Insert or replace each provider config (e.g. opencode-go +
                // opencode-go-anthropic for the OpenCode Go preset).
                {
                    let mut cfg = state.cfg.write().await;
                    for pc in &configs {
                        if let Some(i) = cfg.providers.iter().position(|x| x.name == pc.name) {
                            cfg.providers[i] = pc.clone();
                        } else {
                            cfg.providers.push(pc.clone());
                        }
                    }
                }
                // Seed the runtime key for every config so the immediate turn
                // works without a restart (only when a key was actually resolved).
                if let Some(k) = &key {
                    let mut keys = state.api_keys.write().await;
                    for pc in &configs {
                        keys.insert(pc.name.clone(), k.clone());
                    }
                }
                // Make the newly logged-in provider the default/fallback (used
                // for model-less compaction and legacy models). This does NOT
                // restrict routing — the selected model still routes to its own
                // provider; it only picks the fallback.
                *state.active_provider.write().await = Some(name.clone());
                // Persist to the core-owned config.json (best-effort).
                {
                    let cfg = state.cfg.read().await;
                    if let Err(e) = config::save_providers_config(&cfg.providers, Some(&name)) {
                        emit(&Event::new("info").with(
                            "message",
                            json!(format!(
                                "logged into '{}' for this session (could not persist to config.json: {e})",
                                p.label
                            )),
                        ));
                    }
                }
                let rp = state.resolved_provider().await;
                state.logger.log(
                    "login",
                    json!({ "provider": name, "kind": p.kind.as_str(), "base_url": p.base_url, "has_key": key.is_some() }),
                );
                emit(&Event::new("info").with(
                    "message",
                    json!(if key.is_some() {
                        format!("logged into {} (key from {}).", p.label, p.resolved_env())
                    } else {
                        format!("logged into {}, but no API key found — export {} or /login again with a key.", p.label, p.resolved_env())
                    }),
                ));
                emit(
                    &Event::new("provider_changed")
                        .with("provider", json!(rp.name))
                        .with("kind", json!(rp.kind.as_str()))
                        .with("base_url", json!(rp.base_url))
                        .with("has_key", json!(rp.api_key.is_some())),
                );
                emit(
                    &Event::new("authed")
                        .with("ok", json!(key.is_some()))
                        .with("provider", json!(name)),
                );
                state.refresh_models(&client).await;
            }
            Command::Logout { provider } => {
                // Log out of a provider: drop its runtime key, remove it from the
                // configured providers, persist, and re-aggregate models so its
                // models disappear from `/models`. The persisted TUI key (in
                // settings.json) is cleared by the TUI side.
                //
                // OpenCode Go is one subscription backed by two provider configs
                // (opencode-go + opencode-go-anthropic); logging out either drops
                // both so the user doesn't strand a half-configured subscription.
                let to_remove: Vec<String> =
                    if provider == "opencode-go" || provider == "opencode-go-anthropic" {
                        vec![
                            "opencode-go".to_string(),
                            "opencode-go-anthropic".to_string(),
                        ]
                    } else {
                        vec![provider.clone()]
                    };
                let existed;
                {
                    let mut cfg = state.cfg.write().await;
                    let before = cfg.providers.len();
                    cfg.providers.retain(|p| !to_remove.contains(&p.name));
                    existed = cfg.providers.len() != before;
                }
                for n in &to_remove {
                    state.api_keys.write().await.remove(n);
                }
                if !existed && provider != "default" {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("not logged into '{provider}'"))),
                    );
                    return;
                }
                // Delete the OAuth credential files our /login created, so the
                // provider is FULLY logged out — not just its config/runtime key.
                // Without this, has_*_creds() still returns true (token file
                // remains on disk) and the provider re-appears as "logged in" on
                // the next session, with the stale token silently used for turns.
                for n in &to_remove {
                    oauth::clear_oauth_creds(n);
                    // Plugin-declared OAuth providers: delete their token file +
                    // run the plugin's clear action + drop the cached token.
                    state.plugin_manager.clear_oauth(n).await;
                }
                // If the active provider was one of those logged out, clear the
                // override so the fallback resolves to the first remaining / legacy.
                {
                    let active = state.active_provider.read().await.clone();
                    if active
                        .as_deref()
                        .map(|a| to_remove.iter().any(|n| n == a))
                        .unwrap_or(false)
                    {
                        *state.active_provider.write().await = None;
                    }
                }
                // Persist the trimmed provider list (fall back to the first
                // remaining provider, else legacy).
                {
                    let cfg = state.cfg.read().await;
                    let active = cfg.providers.first().map(|p| p.name.clone());
                    let _ = config::save_providers_config(&cfg.providers, active.as_deref());
                }
                state.logger.log("logout", json!({ "provider": provider }));
                emit(
                    &Event::new("info")
                        .with("message", json!(format!("logged out of '{}'", provider))),
                );
                let rp = state.resolved_provider_enriched().await;
                emit(
                    &Event::new("provider_changed")
                        .with("provider", json!(rp.name))
                        .with("kind", json!(rp.kind.as_str()))
                        .with("base_url", json!(rp.base_url))
                        .with("has_key", json!(rp.api_key.is_some())),
                );
                if rp.api_key.is_some() {
                    emit(
                        &Event::new("authed")
                            .with("ok", json!(true))
                            .with("provider", json!(rp.name)),
                    );
                } else {
                    emit(
                        &Event::new("authed")
                            .with("ok", json!(false))
                            .with("provider", json!(rp.name)),
                    );
                }
                state.refresh_models(&client).await;
            }
            Command::LoginOauth { preset } => {
                // Perform the interactive OAuth subscription login (no official
                // CLI needed). Runs the flow (Google device-code / Claude
                // authorize+PKCE+loopback / xAI device-code), emitting
                // `oauth_prompt` events with the URL/code to visit, stores the
                // token, ensures the provider is configured, and re-aggregates
                // so its models appear in /models.
                //
                // Spawned on a background task so a multi-minute device-code
                // poll (xAI SuperGrok) does not freeze the stdin command loop
                // — otherwise the TUI looks hung for 30s+ while waiting for
                // browser approval.
                let plugin_login = state.plugin_manager.supports_oauth_login(&preset);
                if !oauth::supports_login(&preset) && !plugin_login {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "'{}' has no OAuth login flow yet; use /login with an API key instead",
                            preset
                        )),
                    ));
                    return;
                }
                let label = config::find_preset(&preset)
                    .map(|p| p.label.to_string())
                    .or_else(|| state.plugin_manager.oauth_config(&preset).map(|c| c.label))
                    .unwrap_or_else(|| preset.clone());
                emit(&Event::new("info").with(
                    "message",
                    json!(format!("starting OAuth login for {label}…")),
                ));
                let st = state.clone();
                let cl = client.clone();
                // Generation counter: a second /login cancels applying an older
                // in-flight OAuth (user re-ran login with a new device code).
                static OAUTH_GEN: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let gen = OAUTH_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                tokio::spawn(async move {
                    // Use free protocol::emit (Send) — not a captured &dyn Fn.
                    let prompt_emit = |p: oauth::OAuthPrompt| {
                        emit(
                            &Event::new("oauth_prompt")
                                .with("url", json!(p.url))
                                .with("code", json!(p.code))
                                .with("message", json!(p.message)),
                        );
                    };
                    let outcome = if plugin_login {
                        st.plugin_manager.oauth_login(&preset, &prompt_emit).await
                    } else {
                        oauth::login(&preset, &cl, &prompt_emit).await
                    };
                    // Stale attempt (user started another login) — drop result.
                    if OAUTH_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
                        emit(&Event::new("info").with(
                            "message",
                            json!("Ignoring a superseded OAuth attempt (a newer /login is in progress)."),
                        ));
                        return;
                    }
                    match outcome {
                        Ok(oauth::LoginOutcome::Done) => {
                            finalize_oauth(&st, &cl, &preset, &label).await;
                        }
                        Ok(oauth::LoginOutcome::AwaitingCode { pending }) => {
                            // No-browser (SSH/headless) flow: the prompt was already
                            // emitted; stash the PKCE verifier + redirect_uri and
                            // wait for the user to paste the code/redirect URL via
                            // the `oauth_code` command.
                            let kind = pending.kind.clone();
                            *st.pending_oauth.lock().await = Some(pending);
                            let msg = match kind.as_str() {
                                "openai" => "OAuth login awaiting callback URL. Open the URL above locally, approve, then paste the final localhost URL with /oauth-code <url>.",
                                "anthropic" => "OAuth login awaiting a code. Open the URL above on any device, approve, then paste the code or final callback URL via /oauth-code <code-or-url>.",
                                "xai" | "grok" => "OAuth login awaiting approval. Open the URL above on any device and click Approve — it finishes automatically (no /oauth-code needed for xAI device-code).",
                                "qwen" => "OAuth login awaiting approval. Open the URL above on any device and approve — it finishes automatically (no /oauth-code needed for Qwen device-code).",
                                "github" | "kimi-coding" | "kilocode" | "codebuddy-cn" => "OAuth login awaiting approval. Open the device URL above, sign in and approve — it finishes automatically (no /oauth-code needed).",
                                "cline" | "clinepass" | "kimchi" | "iflow" => "OAuth login awaiting a code. Open the URL above, sign in, then paste the redirect URL or token via /oauth-code <value>.",
                                _ => "OAuth login awaiting a code. Open the URL above on any device, approve, then paste the code via /oauth-code <code>.",
                            };
                            emit(&Event::new("info").with("message", json!(msg)));
                        }
                        Err(e) => {
                            st.logger.log(
                                "login_oauth_error",
                                json!({ "provider": preset, "error": e }),
                            );
                            emit(
                                &Event::new("error")
                                    .with("message", json!(format!("OAuth login failed: {e}"))),
                            );
                        }
                    }
                });
            }
            Command::OauthCode { code } => {
                // Complete a pending no-browser (manual-code) OAuth login — the
                // SSH/headless path. The PKCE verifier was stashed by the
                // `login_oauth` AwaitingCode arm; exchange the pasted code,
                // store the token, then finalize exactly like the web flow.
                let pending = state.pending_oauth.lock().await.take();
                let pending = match pending {
                    Some(p) => p,
                    None => {
                        emit(&Event::new("error").with(
                            "message",
                            json!("No pending OAuth login. Run /login first — the no-browser flow prints a URL; paste its code here with /oauth-code <code>."),
                        ));
                        return;
                    }
                };
                let preset = pending.kind.clone();
                let label = config::find_preset(&preset)
                    .map(|p| p.label.to_string())
                    .or_else(|| state.plugin_manager.oauth_config(&preset).map(|c| c.label))
                    .unwrap_or_else(|| preset.clone());
                // A plugin owns the exchange when this pending login is for a
                // plugin-declared OAuth provider; otherwise the built-in flow.
                let result = if state.plugin_manager.oauth_config(&preset).is_some() {
                    state
                        .plugin_manager
                        .oauth_complete(&preset, &pending, &code)
                        .await
                } else {
                    oauth::complete_oauth(&preset, &client, &pending, &code)
                        .await
                        .map(|_| ())
                };
                match result {
                    Ok(()) => {
                        finalize_oauth(&state, &client, &preset, &label).await;
                    }
                    Err(e) => {
                        // Restore the pending state so the user can retry with a
                        // corrected code without restarting /login.
                        *state.pending_oauth.lock().await = Some(pending);
                        emit(&Event::new("error").with(
                            "message",
                            json!(format!(
                                "OAuth code exchange failed: {e} (pending login restored — try /oauth-code again with the correct code)"
                            )),
                        ));
                    }
                }
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
                // Minimal runtime knob setter for the values the TUI settings
                // modal edits. Coerce string-or-number to u64, string-or-bool
                // to bool.
                let as_u64 = |v: &Value| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                };
                let as_bool = |v: &Value| {
                    v.as_bool().or_else(|| {
                        v.as_str().and_then(|s| match s {
                            "1" | "true" | "on" => Some(true),
                            "0" | "false" | "off" => Some(false),
                            _ => None,
                        })
                    })
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
                    "auto_compact" => {
                        if let Some(b) = as_bool(&value) {
                            cfg.auto_compact = b;
                            out_val = json!(b);
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
                state.invalidate_real_token_baseline().await;
                clear_work_state(&state).await;
                reset_stats(&state).await;
                emit(&Event::new("reset"));
            }
            Command::Clear => {
                // In-memory only: keep the session file so a restart can still resume.
                state.conversation.lock().await.clear();
                state.invalidate_real_token_baseline().await;
                clear_work_state(&state).await;
                reset_stats(&state).await;
                emit(&Event::new("reset"));
            }
            Command::Undo => {
                // Drop the last turn: a user msg + everything after it (assistant, tool msgs).
                let mut conv = state.conversation.lock().await;
                // Walk back past trailing tool/assistant messages to the last user message.
                while let Some(last) = conv.last() {
                    if last.is_user() {
                        conv.pop();
                        break;
                    }
                    conv.pop();
                }
                if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                    session::rewrite(p, &conv);
                }
                drop(conv);
                // The dropped turn invalidates the real baseline's length anchor.
                state.invalidate_real_token_baseline().await;
                clear_work_state(&state).await;
                emit(&Event::new("reset")); // TUI clears blocks; core keeps the trimmed conv
            }
            Command::Compact { instructions } => {
                // Force compaction now, then emit a compacted event. Uses the
                // summarize strategy (honoring any `/compact <instructions>`
                // override or the configured `compact_instructions`) when an api
                // key is present; falls back to naive drop-oldest otherwise.
                let mut messages = state.conversation.lock().await.clone();
                if messages.len() > 2 {
                    dispatch_lifecycle(&state, "pre_compact").await;
                    let before_est = estimate_messages_tokens(&messages);
                    // Size the reclaim against the user's actual model window,
                    // not a hardcoded 200k.
                    let model_ctx = {
                        let last = state.last_model.lock().await.clone();
                        let models = state.models.read().await;
                        last.as_deref()
                            .and_then(|m| models.iter().find(|mi| mi.id == m))
                            .map(|m| m.context_window as u64)
                            .unwrap_or(200_000)
                    };
                    emit(
                        &Event::new("compacting")
                            .with("before_tokens", json!(before_est))
                            .with("trigger", json!("manual")),
                    );
                    let cfg = state.cfg.read().await.clone();
                    let model_name = state.last_model.lock().await.clone().unwrap_or_default();
                    let rp = state.resolve_provider_for_model(&model_name).await;
                    // A `/compact <instructions>` override takes precedence over
                    // the configured default; empty/whitespace falls back.
                    let instr = match instructions
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(s) => Some(s),
                        None => cfg.compact_instructions.as_deref(),
                    };
                    // Manual compact is a one-shot — a fresh (never-cancelled)
                    // token is fine; there's no in-flight turn to abort it.
                    let cancel = CancellationToken::new();
                    let summary_chars = if rp.api_key.is_some() && !model_name.is_empty() {
                        compact_with_summary(
                            &client,
                            &cfg,
                            &rp,
                            &model_name,
                            &mut messages,
                            &cancel,
                            false,
                            model_ctx,
                            instr,
                        )
                        .await
                    } else {
                        compact_conversation(&mut messages, model_ctx);
                        0
                    };
                    *state.conversation.lock().await = messages.clone();
                    let after_est = estimate_messages_tokens(&messages);
                    *state.estimated_tokens.lock().await = after_est;
                    // Manual compaction rewrote history; drop the stale baseline.
                    state.invalidate_real_token_baseline().await;
                    if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                        session::rewrite(p, &messages);
                    }
                    emit(
                        &Event::new("compacted")
                            .with("before_tokens", json!(before_est))
                            .with("after_tokens", json!(after_est))
                            .with("summary_chars", json!(summary_chars)),
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
                // Restore the loaded session's cumulative stats so `/stats` shows
                // its real totals, not the prior session's.
                restore_stats(&state, &p).await;
                // Point the session_file at the loaded path so future appends go there.
                state.cfg.write().await.session_file = Some(p);
                emit(&Event::new("reset"));
                // Replay the loaded transcript so the TUI shows prior turns
                // instead of an empty view after switching/resuming a session.
                let visible: Vec<Value> = loaded
                    .iter()
                    .filter(|m| !m.is_system())
                    .map(Value::from)
                    .collect();
                let est = estimate_messages_tokens(&loaded);
                *state.estimated_tokens.lock().await = est;
                // Loaded history has no known real token count yet; the next
                // request's `usage` will re-establish the baseline.
                state.invalidate_real_token_baseline().await;
                clear_work_state(&state).await;
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
                state.invalidate_real_token_baseline().await;
                clear_work_state(&state).await;
                state.cfg.write().await.session_file = Some(new_path.clone());
                // Fresh session: zero the cumulative stats (in memory + sidecar).
                reset_stats(&state).await;
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
                // Cumulative REAL usage (billing totals — accurate by construction:
                // each turn adds the endpoint's actual prompt/completion tokens).
                let ti = *state.tokens_in.lock().await; // cumulative prompt
                let to = *state.tokens_out.lock().await; // cumulative output
                let cached = *state.cached_tokens.lock().await;
                let turns = state.logger.turn_count();
                let cache_hit_ratio = if ti > 0 {
                    cached as f64 / ti as f64
                } else {
                    0.0
                };
                // `tokens_in` = the CURRENT real context — the SAME grounded
                // estimate the footer uses (real prompt_tokens + small delta) — so
                // /stats "in" matches the footer instead of the cumulative prompt,
                // which re-sums the whole prefix every turn and looks inflated next
                // to it. The cumulative prompt is still exposed as `total_in` for
                // billing and the cache ratio.
                let ctx = {
                    let conv = state.conversation.lock().await;
                    let last_real = *state.last_real_prompt_tokens.lock().await;
                    let len_at = *state.conv_len_at_last_real.lock().await;
                    grounded_estimate(&conv, last_real, len_at)
                };
                let msg_count = state.conversation.lock().await.len();
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
                        .with("tokens_in", json!(ctx)) // current context (footer match)
                        .with("tokens_out", json!(to)) // cumulative output
                        .with("total_in", json!(ti)) // cumulative prompt (billing)
                        .with("tokens_total", json!(ti + to)) // cumulative in+out
                        .with("cached_tokens", json!(cached))
                        .with("cache_hit_ratio", json!(cache_hit_ratio))
                        .with("turns", json!(turns))
                        .with("messages", json!(msg_count))
                        .with("session_file", json!(session_file)),
                );
            }
            Command::Usage { model } => {
                // Provider plan/rate-limit usage for the model the user is on.
                // Resolve model → owning provider → provider-specific usage
                // endpoint (Umans / Codex / Claude OAuth / …). Read-only.
                let model_name = match model.filter(|m| !m.is_empty()) {
                    Some(m) => m,
                    None => state.last_model.lock().await.clone().unwrap_or_default(),
                };
                // When we still have no model (fresh session, never sent), fall
                // back to the first discovered model so /usage still works.
                let model_name = if model_name.is_empty() {
                    state
                        .models
                        .read()
                        .await
                        .first()
                        .map(|m| m.id.clone())
                        .unwrap_or_default()
                } else {
                    model_name
                };
                let rp = if model_name.is_empty() {
                    let rp = state.resolved_provider().await;
                    oauth::enrich_oauth(rp, &client, Some(&state.plugin_manager)).await
                } else {
                    state.resolve_provider_for_model(&model_name).await
                };
                let usage = provider::fetch_provider_usage(&client, &rp).await;
                let mut ev = Event::new("usage")
                    .with("provider", json!(rp.name))
                    .with("provider_kind", json!(rp.kind.to_string()))
                    .with("model", json!(model_name))
                    .with("base_url", json!(rp.base_url));
                for (k, v) in usage.to_event_fields() {
                    ev = ev.with(&k, v);
                }
                emit(&ev);
            }
            Command::Context => {
                // Token-breakdown: where is the context window being spent?
                // Aggregates per-message token estimates (same char/4 heuristic
                // the footer uses) so the user can see the biggest consumers
                // before compaction fires. Read-only — never mutates state.
                let conv = state.conversation.lock().await.clone();
                let total = {
                    let last_real = *state.last_real_prompt_tokens.lock().await;
                    let len_at = *state.conv_len_at_last_real.lock().await;
                    grounded_estimate(&conv, last_real, len_at)
                };
                let model_ctx = {
                    let last = state.last_model.lock().await.clone();
                    let models = state.models.read().await;
                    last.as_deref()
                        .and_then(|m| models.iter().find(|mi| mi.id == m))
                        .map(|m| m.context_window as u64)
                        .unwrap_or(200_000)
                };
                let pct = if model_ctx > 0 {
                    (total as f64 / model_ctx as f64 * 100.0).round() as u64
                } else {
                    0
                };
                // Per-message estimates; role buckets are aggregated below from
                // the entries for clean u64 values.
                let mut entries: Vec<Value> = Vec::with_capacity(conv.len());
                for (i, m) in conv.iter().enumerate() {
                    let tokens = estimate_messages_tokens(std::slice::from_ref(m));
                    let role = m.role();
                    let preview: String = m
                        .content_text()
                        .map(|t| {
                            let t = t.replace('\n', " ");
                            if t.chars().count() > 100 {
                                format!("{}…", t.chars().take(100).collect::<String>())
                            } else {
                                t
                            }
                        })
                        .unwrap_or_else(|| "(no text / multimodal)".to_string());
                    entries.push(json!({
                        "index": i,
                        "role": role,
                        "tokens": tokens,
                        "preview": preview,
                    }));
                }
                // Aggregate per-role token totals.
                let role_obj: Value = {
                    let mut counts: std::collections::BTreeMap<String, u64> =
                        std::collections::BTreeMap::new();
                    for e in &entries {
                        let r = e["role"].as_str().unwrap_or("").to_string();
                        let t = e["tokens"].as_u64().unwrap_or(0);
                        *counts.entry(r).or_insert(0) += t;
                    }
                    let mut map = serde_json::Map::new();
                    for (k, v) in counts {
                        map.insert(k, json!(v));
                    }
                    Value::Object(map)
                };
                let system_tokens = entries
                    .iter()
                    .filter(|e| e["role"].as_str() == Some("system"))
                    .map(|e| e["tokens"].as_u64().unwrap_or(0))
                    .sum::<u64>();
                // Top 10 consumers by tokens (descending).
                entries.sort_by(|a, b| b["tokens"].as_u64().cmp(&a["tokens"].as_u64()));
                let top: Vec<Value> = entries.iter().take(10).cloned().collect();
                emit(
                    &Event::new("context_breakdown")
                        .with("total_tokens", json!(total))
                        .with("context_window", json!(model_ctx))
                        .with("pct", json!(pct))
                        .with("messages", json!(conv.len()))
                        .with("system_tokens", json!(system_tokens))
                        .with("by_role", role_obj)
                        .with("top_consumers", json!(top)),
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
            Command::ListSkills => {
                // Re-publish the discoverable-skills list (project then user
                // scope). The TUI/web request this after a turn ends so a skill
                // created mid-session (e.g. by /reflect or /index) shows up in
                // the `/skill:<name>` autocomplete without a restart.
                let ws = state.cfg.read().await.workspace.clone();
                emit_skills_event(&ws);
            }
            Command::ApplySkill {
                name,
                task,
                model,
                reasoning_effort,
            } => {
                let st = state.clone();
                let client = client.clone();
                let models = st.models.read().await.clone();
                if !models.iter().any(|m| m.id == model) {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("unknown model: {model}"))),
                    );
                    continue;
                }
                let ws = st.cfg.read().await.workspace.clone();
                let skills = subagent::discover_skills_full(&ws);
                let skill = skills
                    .into_iter()
                    .find(|s| s.name.eq_ignore_ascii_case(&name));
                let Some(skill) = skill else {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "unknown skill '{name}' — use /skill:<name> with a name from the autocomplete"
                        )),
                    ));
                    continue;
                };
                let effort = reasoning_effort.unwrap_or_else(|| "medium".into());
                let prompt = build_skill_prompt(&skill, task.as_deref());
                start_turn(&st, &client, model, prompt, effort, None).await;
            }
            Command::StartGoal {
                goal: goal_text,
                concurrency,
                max_tasks,
                allowed_models,
                allowed_providers,
                auto_deploy,
                planner_model,
                worker_model,
                reviewer_model,
                model_concurrency,
                model,
                reasoning_effort,
            } => {
                let models = state.models.read().await.clone();
                if !models.iter().any(|m| m.id == model) {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("unknown model: {model}"))),
                    );
                    continue;
                }
                // Cancel any prior goal deploy.
                cancel_goal_deploy(&state).await;
                let cfg = state.cfg.read().await;
                let defaults = (
                    cfg.subagents.parallel_concurrency,
                    cfg.subagents.parallel_max_tasks,
                );
                drop(cfg);
                match goal::new_goal(goal::StartGoalArgs {
                    goal: goal_text,
                    concurrency,
                    max_tasks,
                    allowed_models: allowed_models.unwrap_or_default(),
                    allowed_providers: allowed_providers.unwrap_or_default(),
                    auto_deploy,
                    role_models: goal::RoleModels {
                        planner: planner_model,
                        worker: worker_model,
                        reviewer: reviewer_model,
                    },
                    model_concurrency: model_concurrency.unwrap_or_default(),
                    model: model.clone(),
                    reasoning_effort: reasoning_effort.clone(),
                    default_concurrency: defaults.0,
                    default_max_tasks: defaults.1,
                }) {
                    Ok(mode) => {
                        let prompt = goal::planning_prompt(&mode);
                        let effort = mode.reasoning_effort.clone();
                        // Planning turn uses parent_model (may be planner role pin).
                        let plan_model = mode.parent_model.clone();
                        // Seed WorkState.goal (replace, not one-shot).
                        {
                            let mut ws = state.work_state.lock().await;
                            ws.goal = truncate_str(&mode.goal, 240);
                            ws.done.clear();
                            ws.in_progress.clear();
                            ws.next.clear();
                            ws.last_activity = "goal:planning".into();
                            ws.touch();
                        }
                        emit_work_state(&state).await;
                        {
                            let mut g = state.goal.lock().await;
                            *g = mode;
                            goal::emit_goal_state(&g);
                        }
                        emit(&Event::new("info").with("message", json!("Goal mode: planning…")));
                        // Prefer planner role model when set; else selected model.
                        let turn_model = if models.iter().any(|m| m.id == plan_model) {
                            plan_model
                        } else {
                            model
                        };
                        start_turn(&state, &client, turn_model, prompt, effort, None).await;
                    }
                    Err(e) => {
                        emit(&Event::new("error").with("message", json!(e)));
                    }
                }
            }
            Command::CancelGoal => {
                cancel_goal_deploy(&state).await;
                // Also abort a planning turn if active.
                if let Some(tok) = state.current.lock().await.take() {
                    tok.cancel();
                }
                let mut g = state.goal.lock().await;
                if g.phase == goal::GoalPhase::Idle {
                    emit(&Event::new("info").with("message", json!("no active goal to cancel")));
                } else {
                    goal::fail_goal(&mut g, "cancelled by user");
                    emit(&Event::new("info").with("message", json!("goal cancelled")));
                }
            }
            Command::GoalStatus => {
                let g = state.goal.lock().await;
                goal::emit_goal_state(&g);
                goal::emit_goal_plan(&g);
            }
            Command::ApproveGoalPlan => {
                let (should_deploy, model) = {
                    let mut g = state.goal.lock().await;
                    if g.phase != goal::GoalPhase::PlanReady {
                        emit(&Event::new("error").with(
                            "message",
                            json!(format!(
                                "approve_goal_plan requires phase plan_ready (got {})",
                                g.phase.as_str()
                            )),
                        ));
                        (false, String::new())
                    } else if g.prompts.is_empty() {
                        emit(
                            &Event::new("error")
                                .with("message", json!("no plan prompts to deploy")),
                        );
                        (false, String::new())
                    } else {
                        g.deploy_after_turn = false;
                        g.auto_deploy = true;
                        (true, g.parent_model.clone())
                    }
                };
                if should_deploy {
                    let _ = model;
                    spawn_goal_deploy(state.clone(), client.clone()).await;
                }
            }
            Command::ReviseGoal {
                feedback,
                model,
                reasoning_effort,
            } => {
                let models = state.models.read().await.clone();
                if !models.iter().any(|m| m.id == model) {
                    emit(
                        &Event::new("error")
                            .with("message", json!(format!("unknown model: {model}"))),
                    );
                    continue;
                }
                cancel_goal_deploy(&state).await;
                let prompt = {
                    let mut g = state.goal.lock().await;
                    if g.goal.is_empty() {
                        emit(
                            &Event::new("error")
                                .with("message", json!("no goal to revise — use start_goal first")),
                        );
                        None
                    } else {
                        g.revise_feedback = Some(feedback);
                        g.plan = None;
                        g.prompts.clear();
                        g.error = None;
                        g.deploy_after_turn = false;
                        g.parent_model = model.clone();
                        if let Some(e) = reasoning_effort {
                            g.reasoning_effort = e;
                        }
                        goal::transition(&mut g, goal::GoalPhase::Planning, Some("revising plan"));
                        Some((goal::planning_prompt(&g), g.reasoning_effort.clone()))
                    }
                };
                if let Some((prompt, effort)) = prompt {
                    start_turn(&state, &client, model, prompt, effort, None).await;
                }
            }
            Command::RefreshMemory => {
                let msg = refresh_memory_injection(&state).await;
                emit(&Event::new("info").with("message", json!(msg)));
            }
            Command::SaveMemory { text, tags, scope } => {
                if text.trim().is_empty() {
                    emit(
                        &Event::new("error")
                            .with("message", json!("save_memory: 'text' must not be empty")),
                    );
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
                    let mem_scope = memory::Scope::parse(scope.as_deref().unwrap_or("workspace"));
                    match memory::save_memory_scoped(&ws, mem_scope, &name, &text, &mem_type, "") {
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
                            emit(
                                &Event::new("error")
                                    .with("message", json!(format!("save_memory failed: {e}"))),
                            );
                        }
                    }
                }
            }
            Command::ListMemory => {
                let ws = state.cfg.read().await.workspace.clone();
                let entries = memory::scan_all_memories(&ws);
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
                            "scope": m.scope.as_str(),
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
            Command::ForgetMemory { id, scope } => {
                let ws = state.cfg.read().await.workspace.clone();
                let result = match scope.as_deref() {
                    Some(s) if !s.is_empty() => {
                        memory::forget_memory_scoped(&ws, memory::Scope::parse(s), &id)
                    }
                    _ => memory::forget_memory_any(&ws, &id),
                };
                match result {
                    Ok(()) => {
                        let _ = refresh_memory_injection(&state).await;
                        emit(
                            &Event::new("memory_saved")
                                .with("message", json!(format!("forgot memory '{id}'"))),
                        );
                    }
                    Err(e) => {
                        emit(
                            &Event::new("error")
                                .with("message", json!(format!("forget_memory failed: {e}"))),
                        );
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
            Command::AskReply {
                request_id,
                answers,
            } => {
                // The user answered (or skipped) a pending `ask` tool call.
                // Resolves the awaiting request_ask() so the model continues.
                let p = state.pending_asks.lock().await.get(&request_id).cloned();
                if let Some(p) = p {
                    *p.answers.lock().await = Some(answers);
                    p.notify.notify_one();
                } else {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!("no pending ask for id {request_id}")),
                    ));
                }
            }
            Command::SudoReply {
                request_id,
                approved,
                password,
            } => {
                // The user approved (with password) or declined (Esc) a pending
                // sudo_request. Resolves the awaiting request_sudo() so the
                // blocked bash call either runs with `sudo -S` or returns a
                // "declined" outcome to the agent.
                let p = state.pending_sudos.lock().await.get(&request_id).cloned();
                if let Some(p) = p {
                    *p.result.lock().await =
                        Some(if approved { password } else { None });
                    p.notify.notify_one();
                } else {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!("no pending sudo request for id {request_id}")),
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
            Command::ClearQueue => {
                // Drop a queued follow-up/steer but leave the running turn alone —
                // the TUI's Esc uses this to cancel just the queued message.
                // (If a steer already cancelled the in-flight turn, that turn's
                // `aborted` will still fire; clearing here means the steer won't
                // run and the loop winds down to idle.)
                let had = state.queued.lock().await.take().is_some();
                emit(&Event::new("info").with(
                    "message",
                    json!(if had {
                        "queue cleared"
                    } else {
                        "queue already empty"
                    }),
                ));
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
                start_turn(&st, &client, model, prompt, effort, images).await;
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
    // Clean up our presence record so peers don't see a stale session. Best
    // effort — a kill -9 / crash leaves a stale file that `read_peers` reaps
    // by mtime, so this is an optimization (instant disappearance), not a
    // correctness requirement.
    {
        let ws = state.cfg.read().await.workspace.clone();
        presence::clear_presence(&ws, std::process::id());
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

/// If this tool call targets a restricted ("dangerous") path, return the
/// blocklist reason. The approval gate uses this so that — under
/// `Destructive`/`Always` — a restricted path (`.env`, `.git/**`, `.ssh/**`,
/// `id_rsa`, …) forces an approval prompt for BOTH reads and writes, instead
/// of the old unconditional hard block. Under `Never` the gate skips this
/// entirely, so ALL file restrictions are disabled.
///
/// `root` is the workspace root used to resolve symlinks: each path is first
/// checked against the blocklist in its RAW model-supplied form (catches a
/// literal `.env`/`.git` early), then — after `workspace::resolve` follows
/// symlinks to a canonical absolute path — checked AGAIN against the
/// canonical path's components. A symlink alias such as `linkdir -> .git`
/// makes `linkdir/config` pass the raw check (no `.git` in the literal
/// string) yet resolve to `<root>/.git/config`; the canonical re-check closes
/// that bypass, since the canonical path is what actually gets read/written.
/// If `resolve` fails (e.g. the path escapes the workspace) the raw-check
/// result stands unchanged.
///
/// Covers the content-touching tools: `read_file` (read), `write_file`/
/// `edit`/`patch` (write), and the bulk variants (each inner path is checked).
/// Search/list tools (`grep`/`glob`/`list_dir`) and `bash` are intentionally
/// excluded — they don't read a single restricted file's content by path.
pub(crate) fn restricted_path_for_tool(
    name: &str,
    args: &Value,
    root: &std::path::Path,
) -> Option<String> {
    fn path_of(a: &Value) -> Option<&str> {
        a.get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
    }
    // Check one path string: raw form first, then the symlink-resolved
    // canonical path. Both use the same blocklist; the canonical pass is what
    // defeats a symlink alias (linkdir -> .git) the raw pass can't see.
    fn check(raw: &str, root: &std::path::Path) -> Option<String> {
        if let Some(reason) = workspace::check_dangerous_path(raw) {
            return Some(reason);
        }
        let canon = workspace::resolve(root, raw).ok()?;
        // Reduce to a root-relative, forward-slash form so the same
        // component-glob logic (`.git/**`, `**/.ssh/**`, …) that checks the
        // raw string applies to the canonical path, cross-platform.
        let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let rel = canon.strip_prefix(&canon_root).unwrap_or(&canon);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        workspace::check_dangerous_path(&rel_str)
    }
    match name {
        "read_file" | "write_file" | "edit" | "patch" => {
            path_of(args).and_then(|raw| check(raw, root))
        }
        "bulk_read" => args
            .get("paths")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|p| p.as_str())
                    .find_map(|raw| check(raw, root))
            }),
        "bulk_write" => args
            .get("files")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|f| f.get("path").and_then(|v| v.as_str()))
                    .find_map(|raw| check(raw, root))
            }),
        "bulk_edit" => args
            .get("edits")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|f| f.get("path").and_then(|v| v.as_str()))
                    .find_map(|raw| check(raw, root))
            }),
        // `bulk`: recurse into inner calls — if ANY inner call targets a
        // restricted path, the whole bulk prompts (then approved calls proceed).
        "bulk" => args
            .get("calls")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|c| {
                    let n = c.get("name").and_then(|v| v.as_str())?;
                    let a = c.get("args")?;
                    restricted_path_for_tool(n, a, root)
                })
            }),
        _ => None,
    }
}

/// Build the user-message prompt for an `apply_skill` invocation: instructs
/// the model to read and follow the named skill, inlining the skill body (the
/// core reads it from disk so global skills under ~/.catalyst-code/skills
/// work despite read_file's path restriction), and appending an optional task.
fn build_skill_prompt(skill: &subagent::SkillEntry, task: Option<&str>) -> String {
    let mut p = format!(
        "Apply the \"{}\" skill. Read and follow the procedure in the skill file below.\n\n<skill name=\"{}\">\n{}\n</skill>\n",
        skill.name, skill.name, skill.body
    );
    if let Some(t) = task.map(str::trim).filter(|t| !t.is_empty()) {
        p.push_str(&format!("\nTask: {}\n", t));
    }
    p
}

/// Expand `@<path>` file mentions in a prompt by inlining the referenced
/// file's contents directly, so the model sees them without a `read_file`
/// round-trip — mirroring how `apply_skill` inlines a skill body. The
/// transcript still shows the concise `@path` (the TUI/web logged the raw
/// text before the core received it); only the message the model reads is
/// expanded.
///
/// A mention is `@` followed by a non-whitespace path, where the `@` is at
/// start-of-string or preceded by whitespace (so emails / `foo@bar` and
/// inline `@param` tags without a leading space don't trigger). Paths resolve
/// relative to the workspace; absolute paths (leading `/`) and `..`/`.` paths
/// are honored as-is — the core has unrestricted FS access, so `@../` and
/// `@/abs` reach outside the workspace (matching the TUI's mention completion).
/// Directories, files larger than `max_bytes`, and unreadable paths are left
/// as-is so the model can fall back to `read_file`. Returns the expanded
/// prompt and the list of paths successfully inlined.
fn expand_file_mentions(
    prompt: &str,
    workspace: &std::path::Path,
    max_bytes: u64,
) -> (String, Vec<String>) {
    let chars: Vec<(usize, char)> = prompt.char_indices().collect();
    let mut out = String::with_capacity(prompt.len() + 256);
    let mut attached: Vec<String> = Vec::new();
    let mut k = 0;
    let mut prev_ws_or_start = true;
    while k < chars.len() {
        let (idx, ch) = chars[k];
        if ch == '@' && prev_ws_or_start {
            // Span from after '@' to the next whitespace char (or end).
            let tok_byte_start = idx + '@'.len_utf8();
            let mut m = k + 1;
            while m < chars.len() && !is_mention_ws(chars[m].1) {
                m += 1;
            }
            let tok_byte_end = if m < chars.len() {
                chars[m].0
            } else {
                prompt.len()
            };
            let raw = &prompt[tok_byte_start..tok_byte_end];
            if !raw.is_empty() {
                if let Some((path, content)) = read_mentioned_file(raw, workspace, max_bytes) {
                    out.push('@');
                    out.push_str(&path);
                    out.push_str("\n<file path=\"");
                    out.push_str(&path);
                    out.push_str("\">\n");
                    out.push_str(&content);
                    if !content.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("</file>\n");
                    attached.push(path);
                    k = m;
                    prev_ws_or_start = true; // the block ends in '\n'
                    continue;
                }
            }
            // Not an attachable mention: emit the '@' and keep scanning.
            out.push('@');
            k += 1;
            prev_ws_or_start = false;
        } else {
            out.push(ch);
            prev_ws_or_start = is_mention_ws(ch);
            k += 1;
        }
    }
    (out, attached)
}

fn is_mention_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Try to read a mentioned file. The raw token may carry trailing prose
/// punctuation ("see @file.rs." → "file.rs"); try the token verbatim first,
/// then with trailing punctuation stripped, so legitimate paths keep their
/// characters while common prose edge cases still resolve.
fn read_mentioned_file(
    token: &str,
    workspace: &std::path::Path,
    max_bytes: u64,
) -> Option<(String, String)> {
    let trimmed = token.trim_end_matches(|c: char| {
        matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\'' | '"'
        )
    });
    for cand in [token, trimmed] {
        if cand.is_empty() {
            continue;
        }
        if let Some(res) = try_read_mentioned_file(cand, workspace, max_bytes) {
            return Some(res);
        }
    }
    None
}

fn try_read_mentioned_file(
    token: &str,
    workspace: &std::path::Path,
    max_bytes: u64,
) -> Option<(String, String)> {
    let p = std::path::Path::new(token);
    let resolved = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    };
    let meta = std::fs::metadata(&resolved).ok()?;
    if meta.is_dir() {
        return None;
    }
    if meta.len() > max_bytes {
        return None;
    }
    let content = std::fs::read_to_string(&resolved).ok()?;
    Some((token.to_string(), content))
}

/// Start (or queue) an assistant turn for `prompt`. Shared by `send` and
/// `apply_skill`: if a turn is already running, buffer this prompt one-deep
/// (the running turn's drain picks it up); otherwise spawn run_turn_and_drain.
async fn start_turn(
    state: &Arc<State>,
    client: &reqwest::Client,
    model: String,
    prompt: String,
    effort: String,
    images: Option<Vec<String>>,
) {
    let already = state.current.lock().await.is_some();
    if already {
        let mut q = state.queued.lock().await;
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
        return;
    }
    let tok = CancellationToken::new();
    *state.current.lock().await = Some(tok.clone());
    let handle = tokio::spawn(run_turn_and_drain(
        state.clone(),
        client.clone(),
        model,
        prompt,
        effort,
        images,
        tok,
    ));
    *state.handle.lock().await = Some(handle);
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
        // A turn freed several conversation clones + tool-result buffers
        // (compaction alone drops the old history). glibc malloc keeps those
        // freed bytes in its arenas, so RSS creeps up and never falls — trim the
        // heap back to the OS once per turn to bound long-session growth.
        trim_heap();
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
            // Store the handle so stdin EOF (which awaits state.handle) waits for
            // this drained turn too — otherwise it may tear the runtime down
            // while a queued follow-up/steer is still running.
            *st.handle.lock().await = Some(tokio::spawn(run_turn_and_drain(
                st.clone(),
                client.clone(),
                q.model,
                q.prompt,
                q.effort,
                None,
                tok2,
            )));
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
    // For session_stop, attach cumulative + last-turn metrics so a telemetry
    // plugin can aggregate per-turn signal out-of-the-box, without the
    // (off-by-default) JSONL debug log. Built once; each plugin's pass_args
    // decides whether it is included in its context.
    let metrics_args: Option<Value> = if hook == "session_stop" {
        Some(st.session_stop_hook_args().await)
    } else {
        None
    };
    for (plugin_name, config) in &st.plugin_manager.get_hook_configs(hook) {
        let ctx = plugins::build_context(
            hook,
            "",
            &workspace,
            metrics_args.as_ref(),
            &session_id,
            config.pass_args,
        );
        let _ = plugins::execute_hook(hook, plugin_name, config, &ctx).await;
    }
}

/// Run every enabled plugin's pre-execution hook for `hook_name` against a tool
/// call, composing each hook's `modify` into `exec_args` and recording reasons
/// into `hook_notes`. Returns `Some(deny_message)` when a hook denies the call
/// (the caller emits the tool_result and skips the tool), or `None` to proceed.
/// Used for BOTH the tool-specific pre_* hook (pre_bash/pre_write/pre_read) and
/// the catch-all `pre_tool` that fires for every tool call — giving a plugin the
/// same per-call reach over `memory`/`todo_write`/`git_*`/`subagent`/… that a
/// core edit of the dispatch loop has.
async fn run_pre_hooks(
    st: &Arc<State>,
    cfg: &crate::config::Config,
    hook_name: &str,
    tool_name: &str,
    exec_args: &mut Value,
    hook_notes: &mut Vec<String>,
) -> Option<String> {
    let configs = st.plugin_manager.get_hook_configs(hook_name);
    if configs.is_empty() {
        return None;
    }
    let session_id = cfg
        .session_file
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let ws = cfg.workspace.display().to_string();
    for (plugin_name, config) in &configs {
        let ctx = plugins::build_context(
            hook_name,
            tool_name,
            &ws,
            Some(exec_args),
            &session_id,
            config.pass_args,
        );
        let result = plugins::execute_hook(hook_name, plugin_name, config, &ctx).await;
        if !result.allow {
            return Some(format!(
                "tool call '{}' denied by plugin '{}' hook '{}': {}",
                tool_name, plugin_name, hook_name, result.reason
            ));
        }
        if let Some(ref modify) = result.modify {
            plugins::apply_modify(exec_args, modify);
        }
        if !result.reason.is_empty() {
            hook_notes.push(format!("{}/{}: {}", plugin_name, hook_name, result.reason));
        }
    }
    None
}

/// Run every enabled plugin's post-execution hook for `hook_name`, handing each
/// the tool's CURRENT result (so it can read it) and letting it MODIFY that
/// result. A post hook returns `modify: { "output": "…", "ok": false }` to
/// replace the result text / flip success — e.g. redact a secret, append
/// context, or reformat. Post hooks never block (the op already ran), so
/// `allow:false` is ignored (only its `reason` is surfaced). Used for BOTH the
/// tool-specific post_* hook and the catch-all `post_tool`.
async fn run_post_hooks(
    st: &Arc<State>,
    cfg: &crate::config::Config,
    hook_name: &str,
    tool_name: &str,
    exec_args: &Value,
    outcome: &mut tools::Outcome,
    hook_notes: &mut Vec<String>,
) {
    let configs = st.plugin_manager.get_hook_configs(hook_name);
    if configs.is_empty() {
        return;
    }
    let session_id = cfg
        .session_file
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let ws = cfg.workspace.display().to_string();
    for (plugin_name, config) in &configs {
        // Give the hook the current result so it can redact/append/transform it.
        let result_json = json!({
            "ok": outcome.ok,
            "output": outcome.output,
            "diff": outcome.diff,
        });
        let mut ctx = plugins::build_context(
            hook_name,
            tool_name,
            &ws,
            Some(exec_args),
            &session_id,
            config.pass_args,
        );
        if let Some(obj) = ctx.as_object_mut() {
            obj.insert("result".to_string(), result_json);
        }
        let result = plugins::execute_hook(hook_name, plugin_name, config, &ctx).await;
        // Post hooks can't block; a deny is treated as an observed note only.
        if !result.reason.is_empty() {
            hook_notes.push(format!("{}/{}: {}", plugin_name, hook_name, result.reason));
        }
        // Apply an optional result mutation: `output` replaces the text, `ok`
        // flips success, `diff` (string) replaces / (null) clears the diff.
        if let Some(obj) = result.modify.as_ref().and_then(|m| m.as_object()) {
            if let Some(out) = obj.get("output").and_then(|v| v.as_str()) {
                outcome.output = out.to_string();
            }
            if let Some(ok) = obj.get("ok").and_then(|v| v.as_bool()) {
                outcome.ok = ok;
            }
            if let Some(diff) = obj.get("diff") {
                outcome.diff = diff.as_str().map(String::from);
            }
        }
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

/// Whether a turn's originating prompt is itself a learning delegation
/// (`/reflect` or `/index`), which is exempt from auto-reflect — it IS the
/// reflection. NB: these prefixes must stay in sync with the TUI's
/// `sendDelegation` prompts for `/reflect` and `/index` (handlers.go).
fn is_learning_turn(prompt: &str) -> bool {
    let p = prompt.trim();
    p.starts_with("Reflect on the work done in this session")
        || p.starts_with("Run a full knowledge index of this repository")
        || p.starts_with("Run an incremental knowledge index of this repository")
}

/// Build the reflection text injected before completion. Includes recurring
/// patterns (if any) so the model can decide whether to write a skill — the
/// recurrence signal that makes the "solve the same shape 2+ times" rule
/// evaluable instead of a vibes check the model can't track across sessions.
fn build_reflect_text(recurring: &[(usize, String)]) -> String {
    let mut s = String::from(
        "[auto-reflect] Before you write your completion summary, reflect on this turn. \n\
         (1) If you learned a durable convention, architecture fact, decision, or \n\
         gotcha, persist it with the `memory` tool (action: append if a topic memory \n\
         exists, else save; use scope: \"global\" for cross-codebase facts like the \n\
         user's identity, tech-stack preferences, or harness conventions) — skip \n\
         transient task state. Use ONLY tool calls here; do NOT write user-facing prose. \n\
         (2) If you just performed a reusable workflow, consider writing a skill under \n\
         `.catalyst-code/skills/<name>/SKILL.md` (run `list_dir .catalyst-code/skills/` \n\
         first to extend rather than duplicate). \n\
         After reflecting (or if nothing to save), write your final completion summary \n\
         to the user and call `finish` — it should be the last message. If you already \n\
         wrote a summary above, do not repeat it; just save memories and call `finish`.",
    );
    if !recurring.is_empty() {
        s.push_str("\n\nRecurring patterns detected (performed 2+ times across sessions):");
        for (count, label) in recurring.iter().take(5) {
            s.push_str(&format!("\n- {label} ({count} times)"));
        }
        s.push_str("\nIf no existing skill covers a recurring pattern, write one.");
    }
    s
}

/// Extract file categories from a tool call's arguments, for shape analysis.
/// Only file-writing tools contribute a stable path signal (bash etc. do not).
fn extract_file_categories(tool: &str, args_json: &str) -> Vec<String> {
    let args: Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let paths: Vec<String> = match tool {
        "write_file" | "edit" | "patch" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        "bulk_write" => args
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        "bulk_edit" => args
            .get("edits")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("path").and_then(|p| p.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    paths
        .into_iter()
        .map(|p| pattern_log::file_category(&p))
        .collect()
}

/// Auto-reflect gate + prompt builder. Returns `(reflect_text, recurrence_count)`
/// if auto-reflect should fire for this turn, else `None`. As a side effect,
/// records the turn's shape to the pattern log so recurrence is tracked.
/// Callers guard with `!reflected` (one reflection per turn).
///
/// Goal mode: skip while a goal is mid-flight (planning / plan_ready /
/// deploying / running / blocked). Planning only writes a plan — there is
/// nothing durable to reflect on until the goal finishes. Deploy is
/// core-driven after the planning turn ends, so reflecting here would also
/// delay `maybe_finish_goal_planning`.
async fn maybe_reflect_prompt(
    st: &Arc<State>,
    prompt: &str,
    turn_tool_calls: u32,
    shape_tools: &[String],
    shape_files: &[String],
    cancelled: bool,
) -> Option<(String, usize)> {
    if cancelled {
        return None;
    }
    // Skip while goal mode is still working — planning is not "task complete".
    // Check before taking cfg so we never nest cfg + goal locks.
    {
        let g = st.goal.lock().await;
        if g.is_active() {
            return None;
        }
    }
    let cfg = st.cfg.read().await;
    if !cfg.auto_reflect {
        return None;
    }
    if turn_tool_calls < cfg.auto_reflect_min_tool_calls {
        return None;
    }
    if is_learning_turn(prompt) {
        return None;
    }
    let workspace = cfg.workspace.clone();
    drop(cfg);
    let sig = pattern_log::shape_signature(shape_tools, shape_files);
    let label = prompt.lines().next().unwrap_or(prompt);
    pattern_log::append_pattern(&workspace, &sig, label);
    let recurring = pattern_log::recurring_patterns(&workspace);
    let text = build_reflect_text(&recurring);
    Some((text, recurring.len()))
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
    // Remember the model the user selected so the manual `/compact` command
    // can size its reclaim budget against the right context window.
    *st.last_model.lock().await = Some(model.clone());
    // Lifecycle hook: notify plugins a session/turn is starting. Best-effort
    // and never blocks the turn.
    dispatch_lifecycle(st, "session_start").await;

    // If the conversation was left mid-`ask` by a prior core restart (the
    // assistant `ask` tool_call is persisted but no tool result exists),
    // re-present the question before the turn proceeds. Without this the
    // session is wedged (no modal) and the orphan-sanitizer would later
    // insert a synthetic EMPTY result, losing the question. No-op on the
    // common case of no trailing unanswered ask. Idempotent: once the ask
    // has a result, `find_trailing_unanswered_ask` returns None.
    resume_pending_ask(st, &cancel).await;

    // Clear the last-turn metrics at turn entry so a panic before finalization
    // can't leak the PRIOR turn's numbers to this turn's `session_stop` hook
    // (which fires unconditionally from the panic guard). A completed turn sets
    // it fresh at finalization; a failed turn leaves it None and the telemetry
    // plugin skips rather than recording a phantom turn.
    *st.last_turn_metrics.lock().await = None;

    // Vision handoff (pre_turn) and other plugins may remap the model for
    // this turn; keep a mutable binding so a swap propagates to the request loop.
    let mut model = model;

    // Auto-reflect turn-local state (SELF_LEARNING §11 deterministic seam). The
    // shape (tool names + file categories) is accumulated as tools run; at the
    // first `finish`/natural completion of a non-trivial turn it is logged to
    // the recurrence store and a reflection continuation is injected so durable
    // facts/patterns get persisted without relying on the model remembering to.
    // `reflected` prevents re-entry: the reflect's own `finish` exits for real.
    let mut reflected = false;
    let mut turn_tool_calls: u32 = 0;
    let mut shape_tools: Vec<String> = Vec::new();
    let mut shape_files: Vec<String> = Vec::new();

    // Ensure system prompt is present; persist every finalized message to the session file.
    let mut init_est_add = 0u64;
    // Expand `@<path>` file mentions so the model sees the referenced file's
    // contents directly (no `read_file` round-trip) — mirroring how
    // `apply_skill` inlines a skill body. The transcript keeps the concise
    // `@path` the user typed (the TUI/web already logged the raw text).
    let (user_text, attached_files) = {
        let c = st.cfg.read().await;
        expand_file_mentions(&prompt, &c.workspace, c.max_read_bytes)
    };
    if !attached_files.is_empty() {
        emit(&Event::new("info").with(
            "message",
            json!(format!(
                "attached {} file(s) from @mentions: {}",
                attached_files.len(),
                attached_files.join(", ")
            )),
        ));
    }
    {
        let mut conv = st.conversation.lock().await;
        if conv.is_empty() {
            let (workspace, auto_reflect) = {
                let c = st.cfg.read().await;
                (c.workspace.clone(), c.auto_reflect)
            };
            let sys_msg = Message::system(build_main_system_prompt(
                &workspace,
                &st.plugin_manager,
                auto_reflect,
            ));
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
                let mut parts: Vec<ContentPart> = vec![ContentPart::Text {
                    text: user_text.clone(),
                }];
                for img in imgs {
                    let url = image_to_data_url(img);
                    parts.push(ContentPart::Image {
                        image_url: ImageUrl { url, detail: None },
                    });
                }
                Message::user_multimodal(parts)
            }
            _ => Message::user(user_text.clone()),
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

    // Seed the rolling work-state's goal from the user's first substantive
    // prompt. No-op once a goal is set; slash commands / tiny prompts ignored.
    maybe_seed_work_state_goal(st, &prompt).await;

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

    // Main agent tool list: built-in tools (minus the subagent-only intercom
    // coordination tools contact_supervisor/intercom, registered only inside
    // child runs) MERGED with tools declared by enabled plugins, then filtered
    // by every plugin's `disable_tools`. Three plugin capabilities converge
    // here, mirroring what a direct core edit can do to the tool list:
    //   • ADD       — a plugin `tools` entry adds a new capability.
    //   • OVERRIDE  — a plugin tool with `override:true` whose name matches a
    //                  built-in REPLACES that built-in: the plugin's declared
    //                  schema is shown to the model and calls route to the
    //                  plugin handler (see the dispatch below).
    //   • REMOVE    — `disable_tools` names are dropped from the final list
    //                  (built-in OR override). `disable_tools` is the strongest
    //                  lever: a disabled name is gone, period.
    // A plugin tool that merely collides with a built-in name (no `override`)
    // is still skipped — the built-in wins, unchanged.
    let overridden = st.plugin_manager.overridden_tool_names();
    let disabled = st.plugin_manager.disabled_tools();
    let mut reserved: std::collections::HashSet<String> = std::collections::HashSet::new();
    reserved.insert("contact_supervisor".into());
    reserved.insert("intercom".into());
    let mut tool_defs: Vec<Value> = tools::definitions()
        .into_iter()
        .filter(|d| {
            let n = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            reserved.insert(n.to_string());
            // Hide the reserved subagent-only tools, AND any built-in a plugin
            // is overriding (its plugin version is added below instead).
            n != "contact_supervisor" && n != "intercom" && !overridden.contains(n)
        })
        .collect();
    for d in st.plugin_manager.tool_definitions() {
        let n = d
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // An override tool replaces the built-in (already excluded above), so
        // it's always added and claims the name. A plain custom tool is added
        // only if its name isn't already taken (built-in or another plugin).
        let is_override = overridden.contains(n);
        if !is_override && !reserved.insert(n.to_string()) {
            eprintln!(
                "[plugins] tool '{}' collides with a built-in or already-registered tool; skipping",
                n
            );
            continue;
        }
        tool_defs.push(d);
    }
    // REMOVE: `disable_tools` is a final, composition-winning filter — a
    // disabled name vanishes whether it was a built-in, an override, or a
    // custom plugin tool. (No-op when no plugin disables anything.)
    if !disabled.is_empty() {
        tool_defs.retain(|d| {
            d.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .is_none_or(|n| !disabled.contains(n))
        });
    }
    let mut timer = TurnTimer::new();

    // Idle compaction: if 60+ minutes since the last turn completed, compact the
    // conversation so the next turn starts lean. Uses the same summarize strategy
    // as the threshold path; falls back to naive drop-oldest without an api key.
    {
        let last = *st.last_turn_time.lock().await;
        let auto_compact = st.cfg.read().await.auto_compact;
        if auto_compact && last.elapsed().as_secs() > 3600 {
            let mut messages = st.conversation.lock().await.clone();
            if messages.len() > 4 {
                let est = { *st.estimated_tokens.lock().await };
                emit(
                    &Event::new("compacting")
                        .with("before_tokens", json!(est))
                        .with("trigger", json!("idle")),
                );
                dispatch_lifecycle(st, "pre_compact").await;
                let cfg = st.cfg.read().await.clone();
                let rp = st.resolve_provider_for_model(&model).await;
                let idle_ctx = st
                    .models
                    .read()
                    .await
                    .iter()
                    .find(|m| m.id == model)
                    .map(|m| m.context_window as u64)
                    .unwrap_or(200_000);
                let summary_chars = if rp.api_key.is_some() {
                    compact_with_summary(
                        client,
                        &cfg,
                        &rp,
                        &model,
                        &mut messages,
                        &cancel,
                        false,
                        idle_ctx,
                        cfg.compact_instructions.as_deref(),
                    )
                    .await
                } else {
                    compact_conversation(&mut messages, idle_ctx);
                    0
                };
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                let after_est = estimate_messages_tokens(&messages);
                *st.estimated_tokens.lock().await = after_est;
                // Idle compaction rewrote history; the old real baseline is stale.
                st.invalidate_real_token_baseline().await;
                emit(
                    &Event::new("compacted")
                        .with("before_tokens", json!(est))
                        .with("after_tokens", json!(after_est))
                        .with("summary_chars", json!(summary_chars)),
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

        // Resolve the provider for this turn. In the multi-login model the
        // turn routes to the selected model's owning provider (so `/models`
        // can mix providers); falls back to the active/legacy provider for
        // models without a provider tag. Errors out if no API key is available
        // for the resolved provider (runtime key -> config literal -> env var).
        let provider = {
            let rp = st.resolve_provider_for_model(&model).await;
            match rp.api_key.as_ref() {
                Some(_) => rp,
                None => {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "no API key set for provider '{}'; use /login to log in",
                            rp.name
                        )),
                    ));
                    emit(&Event::new("done"));
                    return;
                }
            }
        };

        let cfg = st.cfg.read().await.clone();
        // Context window management: compact once past the configured threshold
        // (default 90%). The 95% hard cap is a floor — compact by then even if the
        // configured threshold is higher, and force the summarize strategy even
        // when disabled (naive drop-oldest may not reclaim enough at critical capacity).
        // `auto_compact` (default true) gates ALL automatic compaction (this threshold
        // path + the idle-compaction block below). When false, no compaction fires
        // automatically — the user must /compact manually (or /clear). Mirrors Claude
        // Code's autoCompactEnabled / DISABLE_AUTO_COMPACT.
        let mut messages = st.conversation.lock().await.clone();
        let (model_ctx, thinking_levels, max_tokens) = st
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == model)
            .map(|m| {
                (
                    m.context_window as u64,
                    m.thinking_levels.clone(),
                    m.max_tokens,
                )
            })
            .unwrap_or((200_000, Vec::new(), 8_192));
        // Anchor on the endpoint's REAL `prompt_tokens` from the last request
        // (the authoritative count of the conversation as the model tokenized it —
        // system + messages + tool-call framing the char/4 heuristic cannot see)
        // and only char/4-estimate the messages appended since. This is far more
        // accurate than re-estimating the whole history every loop iteration, so
        // compaction fires at the right time instead of drifting late into a
        // context-window 400. Falls back to a full char/4 estimate when no real
        // usage has been seen yet (first turn) or right after compaction.
        let last_real = *st.last_real_prompt_tokens.lock().await;
        let len_at = *st.conv_len_at_last_real.lock().await;
        let mut est = grounded_estimate(&messages, last_real, len_at);
        *st.estimated_tokens.lock().await = est;
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
                // Digest rewrote message contents, so the real prompt_tokens
                // baseline no longer matches — drop it until the next request.
                st.invalidate_real_token_baseline().await;
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
        if cfg.auto_compact && est > threshold.min(hard_cap) && messages.len() > 4 {
            let force_summarize = est > hard_cap;
            emit(
                &Event::new("compacting")
                    .with("before_tokens", json!(est))
                    .with("trigger", json!("threshold")),
            );
            dispatch_lifecycle(st, "pre_compact").await;
            let summary_chars = compact_with_summary(
                client,
                &cfg,
                &provider,
                &model,
                &mut messages,
                &cancel,
                force_summarize,
                model_ctx,
                cfg.compact_instructions.as_deref(),
            )
            .await;
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            let after_est = estimate_messages_tokens(&messages);
            *st.estimated_tokens.lock().await = after_est;
            // Compaction rewrote history; the old real baseline is stale.
            st.invalidate_real_token_baseline().await;
            emit(
                &Event::new("compacted")
                    .with("before_tokens", json!(est))
                    .with("after_tokens", json!(after_est))
                    .with("summary_chars", json!(summary_chars)),
            );
        }

        // Sanitize orphaned tool calls + malformed tool-call arguments right
        // before the request. Orphans can arise not only from context compaction
        // but from ANY turn that ended with an assistant `tool_calls` message
        // whose matching results weren't all appended — notably an aborted
        // approval, which `return`s after the assistant message (carrying ALL
        // its tool_calls) was already persisted but before results for the
        // aborted + remaining calls were appended. The next request would then
        // ship an orphaned `tool_calls` and the API rejects it with HTTP 400
        // "No tool output found for function call …", which bricks the session
        // (it repeats every turn). The scan is O(n) with tiny constants and a
        // strict no-op on clean turns; we persist back only when it actually
        // changed something, so clean turns pay just the scan (no clone, no
        // session rewrite). The subagent path already does this unconditionally
        // (subagent.rs) — this makes the main loop consistent with it.
        let orphan_fixes = provider::sanitize_orphaned_tool_calls(&mut messages);
        let fixed_args = provider::sanitize_tool_call_arguments(&mut messages);
        if orphan_fixes > 0 || fixed_args > 0 {
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            if orphan_fixes > 0 {
                emit(&Event::new("info").with("message", json!(format!(
                    "inserted {orphan_fixes} synthetic tool result(s) for tool call(s) whose result was missing (e.g. after an aborted turn) — the conversation is valid again for the API"
                ))));
            }
            if fixed_args > 0 {
                emit(&Event::new("info").with("message", json!(format!(
                    "sanitized {fixed_args} malformed tool-call argument(s) to keep the conversation valid for the API"
                ))));
            }
        }
        // Best pre-stream estimate of this request's prompt size, grounded on the
        // endpoint's last real `prompt_tokens` when available. Passed to
        // stream_turn so the live footer percentage tracks reality while output
        // streams in (the real `usage` chunk at stream end then overwrites it).
        let prompt_est = grounded_estimate(
            &messages,
            *st.last_real_prompt_tokens.lock().await,
            *st.conv_len_at_last_real.lock().await,
        );
        // KV-cache-aware rolling work-state: inject as a TRANSIENT tail system
        // message (never persisted) so the conversation prefix stays byte-identical
        // turn to turn and the provider's prefix cache is never invalidated by
        // it. It is the LAST message, so updating it invalidates nothing earlier
        // in the prefix; only the small work-state + the new turn are prefilled.
        let ws_msg = work_state_message(st).await;
        if let Some(msg) = &ws_msg {
            messages.push(msg.clone());
        }
        // messages is already Vec<Message> — pass directly.
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
                prompt_est,
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

        // Strip the transient work-state before recording the token baseline so
        // conv_len_at_last_real reflects the persisted conversation length
        // (without the transient message) and grounded_estimate's delta slice
        // stays correct. On the error path above we `return` first, so the
        // transient message is simply dropped along with `messages`.
        if ws_msg.is_some() {
            messages.pop();
        }

        // Convert the assistant from OpenAI-shaped Value to Message.
        let assistant_msg = Message::try_from(&assistant).unwrap_or_else(|e| {
            emit(&Event::new("error").with("message", json!(format!("assistant parse: {e}"))));
            Message::assistant("")
        });

        // Anchor all future estimates on the endpoint's REAL `prompt_tokens` —
        // the exact count of `messages` as the model tokenized it (system +
        // history + tool-call framing). `messages` is exactly what we sent, so its
        // length marks where the real baseline stops and the char/4 delta begins;
        // the compaction trigger and live footer then reflect reality instead of
        // a whole-history char/4 guess. Only when the endpoint actually reports
        // usage (some don't); otherwise we keep the previous baseline.
        if tokens_in > 0 {
            *st.last_real_prompt_tokens.lock().await = Some(tokens_in);
            *st.conv_len_at_last_real.lock().await = messages.len();
        }

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
                estimate_message_tokens(&assistant_msg),
            )
        };
        *st.tokens_in.lock().await += acc_in;
        *st.tokens_out.lock().await += acc_out;
        // Accumulate prefix-cache hits so /stats can show cache effectiveness.
        *st.cached_tokens.lock().await += cached_tokens;

        // Append + persist the finalized assistant message.
        {
            let mut conv = st.conversation.lock().await;
            conv.push(assistant_msg.clone());
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, conv.last().unwrap());
            }
        }
        *st.estimated_tokens.lock().await += estimate_message_tokens(&assistant_msg);

        let tool_calls = assistant_msg.tool_calls().map(|tc| tc.to_vec());
        match tool_calls {
            Some(calls) if !calls.is_empty() => {
                for tc in &calls {
                    let id = tc.id.clone();
                    let name = tc.function.name.clone();
                    let args_str = tc.function.arguments.clone();
                    emit(
                        &Event::new("tool_call")
                            .with("id", json!(id))
                            .with("name", json!(name))
                            .with("args", json!(args_str)),
                    );
                    // Accumulate the turn's work-shape for auto-reflect (skip
                    // `finish` — it signals completion, not work).
                    if name != "finish" {
                        turn_tool_calls = turn_tool_calls.saturating_add(1);
                        shape_tools.push(name.clone());
                        for cat in extract_file_categories(&name, &args_str) {
                            shape_files.push(cat);
                        }
                    }
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
                            let tool_result = Message::tool(id.clone(), msg);
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

                    // Approval gate for destructive tools. A plugin tool that
                    // dispatches (a custom name, OR an `override:true` tool that
                    // replaces a built-in) carries its own kind; everything else
                    // uses the static classify() table. A plugin tool that merely
                    // collides with a built-in name (no override) does NOT
                    // dispatch to the plugin, so it falls through to classify().
                    let cfg = st.cfg.read().await.clone();
                    let kind = match st.plugin_manager.tool_config(&name) {
                        Some(tc) if tc.override_builtin || !tools::is_builtin(&name) => tc.kind,
                        _ => tools::classify(&name),
                    };
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
                        let tool_result = Message::tool(id.clone(), msg);
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    // Restricted ("dangerous") paths (.env, .git/**, .ssh/**, id_rsa, …).
                    // Under `Never` ALL file restrictions are disabled — no
                    // prompt, no block. Under `Destructive`/`Always` a
                    // restricted path forces an approval prompt (for reads AND
                    // writes) instead of the old unconditional hard block; an
                    // approved call proceeds.
                    let restricted = if matches!(cfg.approval, Approval::Never) {
                        None
                    } else {
                        restricted_path_for_tool(&name, &args, &cfg.workspace)
                    };
                    let needs_approval = if force_allow || escalated {
                        false
                    } else {
                        match cfg.approval {
                            Approval::Never => false,
                            Approval::Destructive => {
                                kind == tools::ToolKind::Destructive || restricted.is_some()
                            }
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
                                let tool_result = Message::tool(id.clone(), msg);
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

                    // Dispatch pre-execution hooks for this tool. Two phases compose:
                    //   1. the tool-SPECIFIC pre_* hook (pre_bash/pre_write/pre_read)
                    //      — transforms/audits/denies that tool's call; and
                    //   2. the catch-all `pre_tool` hook, which fires for EVERY tool
                    //      (memory, todo_write, git_*, subagent, plugin tools, …)
                    //      so a plugin can intercept any call — the same reach a
                    //      core edit of this dispatch loop has. pre_tool runs AFTER
                    //      the specific hook so it sees the final amended args.
                    // Each hook may allow (optionally overriding arg fields via
                    // `modify`, and/or posting a `reason`), or deny (the call is
                    // skipped and the reason is returned to the model). Hooks
                    // compose: each sees the args as amended by earlier hooks.
                    let hook_name = match name.as_str() {
                        "bash" => "pre_bash",
                        "write_file" | "edit" => "pre_write",
                        "read_file" | "grep" | "glob" => "pre_read",
                        _ => "",
                    };
                    let any_pre = (!hook_name.is_empty() && st.plugin_manager.has_hook(hook_name))
                        || st.plugin_manager.has_hook("pre_tool");
                    // exec_args starts as the original args and is amended in place
                    // by pre-hooks. Only clone when a hook will actually run, so
                    // large write payloads aren't copied in the common case.
                    let mut exec_args = if any_pre { args.clone() } else { args };
                    let mut hook_notes: Vec<String> = Vec::new();
                    let mut denied: Option<String> = None;
                    if !hook_name.is_empty() {
                        denied = run_pre_hooks(
                            st,
                            &cfg,
                            hook_name,
                            &name,
                            &mut exec_args,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if denied.is_none() && name != "finish" {
                        denied = run_pre_hooks(
                            st,
                            &cfg,
                            "pre_tool",
                            &name,
                            &mut exec_args,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if let Some(msg) = denied {
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result = Message::tool(id.clone(), msg);
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
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
                    let mut outcome = if let Some(tc) = st
                        .plugin_manager
                        .tool_config(&name)
                        .filter(|tc| tc.override_builtin || !tools::is_builtin(&name))
                    {
                        // Plugin-declared tool: dispatch to its handler script
                        // (subprocess, stdin=args JSON, stdout={ok,output}).
                        // This branch covers BOTH custom plugin tools (a name no
                        // built-in owns) AND `override:true` tools that REPLACE
                        // a built-in's implementation — the filter admits a
                        // built-in name only when the plugin explicitly opted
                        // into overriding it, so a mere name collision still
                        // falls through to the built-in handler below. Wrapped in
                        // a select! on the turn cancel so /abort can interrupt it
                        // mid-flight; kill_on_drop frees the child.
                        let session_id = cfg
                            .session_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let ws = cfg.workspace.display().to_string();
                        tokio::select! {
                            o = plugins::execute_plugin_tool(&name, &tc, &exec_args, &ws, &session_id) => o,
                            _ = cancel.cancelled() => tools::Outcome::err(format!("{name} aborted")),
                        }
                    } else if name == "bash" {
                        let cmd = exec_args
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let timeout_override = exec_args.get("timeout").and_then(|v| v.as_u64());
                        // Sudo passthrough: if the command invokes `sudo`, we
                        // must NOT let it run directly — sudo opens /dev/tty to
                        // read the password, which garbles the TUI. Instead we
                        // surface a `sudo_request` to the user. On approve the
                        // password is fed via `sudo -S` on stdin; on decline
                        // (Esc) the agent is told the user declined.
                        //
                        // Approval::Never optimization: when approval is Never
                        // and the user has NOPASSWD sudo (or cached creds), we
                        // skip the prompt entirely and run with `sudo -n`. Only
                        // if a password is actually needed do we prompt — so
                        // users with NOPASSWD never see a sudo flyout.
                        if tools::command_uses_sudo(cmd) {
                            let needs_prompt = match cfg.approval {
                                crate::config::Approval::Never => {
                                    tools::sudo_needs_password(&cfg).await
                                }
                                _ => true,
                            };
                            if needs_prompt {
                                match request_sudo(st, cmd, &cancel).await {
                                    SudoResult::Approved { password } => {
                                        tokio::select! {
                                            o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::Password(password)) => o,
                                            _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                                        }
                                    }
                                    SudoResult::Declined => tools::Outcome::ok(
                                        "The user declined the sudo request — the \
                                         command was NOT run. Ask the user to run it \
                                         manually, or re-attempt without sudo."
                                    ),
                                    SudoResult::Aborted => {
                                        emit(&Event::new("aborted"));
                                        emit(&Event::new("done"));
                                        return;
                                    }
                                }
                            } else {
                                // NOPASSWD / cached — run with `sudo -n`
                                // (non-interactive, never opens /dev/tty).
                                tokio::select! {
                                    o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::NonInteractive) => o,
                                    _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                                }
                            }
                        } else {
                            tokio::select! {
                                o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::None) => o,
                                _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                            }
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
                    } else if name == "web_search" {
                        tokio::select! {
                            o = tools::execute_web_search(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("web_search aborted"),
                        }
                    } else if name == "diagnostics" {
                        tokio::select! {
                            o = tools::execute_diagnostics(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("diagnostics aborted"),
                        }
                    } else if name == "spawn" || name == "subagent" {
                        // When goal mode is active, cap concurrency on parallel
                        // fan-out to the goal's limit (defense in depth).
                        let mut sub_args = exec_args.clone();
                        {
                            let g = st.goal.lock().await;
                            if g.is_active() {
                                if let Some(c) =
                                    sub_args.get("concurrency").and_then(|v| v.as_u64())
                                {
                                    sub_args["concurrency"] =
                                        json!(goal::cap_concurrency(c as u32, &g));
                                } else if sub_args.get("tasks").is_some() {
                                    sub_args["concurrency"] = json!(g.concurrency);
                                }
                            }
                        }
                        subagent::execute(
                            st.clone(),
                            client.clone(),
                            provider.clone(),
                            model.clone(),
                            sub_args,
                            cancel.clone(),
                            0,
                        )
                        .await
                    } else if name == "goal_write_plan" {
                        goal::handle_goal_write_plan(st, &exec_args).await
                    } else if name == "ask" {
                        // Blocking user-interaction tool: surface a flyout and
                        // wait for the answer (or skip/abort). Validation errors
                        // and skips return a normal Outcome; an abort ends the
                        // turn like the approval gate does.
                        match request_ask(st, &exec_args, &cancel).await {
                            AskResult::Answered { questions, answers } => {
                                tools::Outcome::ok(format_ask_answers(&questions, &answers))
                            }
                            AskResult::Skipped => tools::Outcome::ok(
                                "The user skipped the questions. Proceed with your best judgment and note any assumptions.",
                            ),
                            AskResult::Aborted => {
                                emit(&Event::new("aborted"));
                                emit(&Event::new("done"));
                                return;
                            }
                        }
                    } else {
                        tools::execute(&name, &exec_args, &cfg)
                    };

                    // Milestone 1.1: a memory save/append/forget via the AI
                    // tool must be visible to subsequent turns in THIS session,
                    // so rebuild the memory slice of the system prompt now (no-op
                    // + prefix-cache-safe when unchanged). The /memory,
                    // /save-memory and /forget-memory commands refresh from their
                    // own handlers; this covers the model's tool path.
                    if name == "memory" {
                        let action = exec_args
                            .get("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if matches!(action, "save" | "append" | "forget") {
                            refresh_memory_injection(st).await;
                        }
                    }

                    // Rolling work-state: mirror todo_write + file edits into the
                    // KV-cache-aware summary so the model sees current work state
                    // every turn without a tool call. Only on success so a failed
                    // write doesn't pollute the recent-files list.
                    if outcome.ok {
                        match name.as_str() {
                            "todo_write" => sync_work_state_from_todos(st, &exec_args).await,
                            "write_file" | "edit" | "patch" | "bulk_write" | "bulk_edit" => {
                                record_file_touch(st, &name, &exec_args).await
                            }
                            _ => {}
                        }
                    }

                    // Dispatch post-execution hooks for this tool. Two phases,
                    // mirroring the pre-hook structure: the tool-SPECIFIC post_*
                    // hook (post_bash/post_write/post_read), then the catch-all
                    // `post_tool` that fires for EVERY tool. Each hook receives the
                    // tool's CURRENT result and may MODIFY it (return
                    // `modify: {"output":…, "ok":…, "diff":…}`) — e.g. redact a
                    // secret, append context, reformat. Post-hooks never block (the
                    // op already ran); `allow:false` is ignored, only `reason` +
                    // `modify` are honored.
                    let post_hook = match name.as_str() {
                        "bash" => "post_bash",
                        "write_file" | "edit" => "post_write",
                        "read_file" | "grep" | "glob" => "post_read",
                        _ => "",
                    };
                    if !post_hook.is_empty() {
                        run_post_hooks(
                            st,
                            &cfg,
                            post_hook,
                            &name,
                            &exec_args,
                            &mut outcome,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if name != "finish" {
                        run_post_hooks(
                            st,
                            &cfg,
                            "post_tool",
                            &name,
                            &exec_args,
                            &mut outcome,
                            &mut hook_notes,
                        )
                        .await;
                    }

                    // finish sentinel: the model signaled completion.
                    if name == "finish" && outcome.ok && outcome.output == "__finish__" {
                        // Auto-reflect gate: before the first `finish` exits a
                        // non-trivial turn, inject a reflection continuation (the
                        // deterministic seam SELF_LEARNING §11 deferred) instead
                        // of completing. Falls through to the normal tool-result
                        // push + re-stream; `reflected` prevents re-entry.
                        let mut do_reflect = false;
                        let mut recurrence = 0usize;
                        if !reflected {
                            if let Some((nudge, rec)) = maybe_reflect_prompt(
                                st,
                                &prompt,
                                turn_tool_calls,
                                &shape_tools,
                                &shape_files,
                                cancel.is_cancelled(),
                            )
                            .await
                            {
                                reflected = true;
                                outcome.output = nudge;
                                recurrence = rec;
                                do_reflect = true;
                            }
                        }
                        if do_reflect {
                            emit(&Event::new("reflecting").with("recurrence", json!(recurrence)));
                            // Fall through → the finish tool_result (carrying
                            // the nudge) is pushed below and the loop re-streams.
                        } else {
                            *st.last_turn_time.lock().await = std::time::Instant::now();
                            let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                            let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                            *st.last_turn_metrics.lock().await = Some(metrics.clone());
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
                            persist_stats(st).await;
                            maybe_finish_goal_planning(st, client, cancel.is_cancelled()).await;
                            emit(&Event::new("done"));
                            return;
                        }
                    }
                    // Surface plugin hook feedback to the model. Any pre-hook that
                    // modified args or posted a reason, and any post-hook that
                    // observed something, is appended to the tool result so the
                    // model knows its write/edit/read/bash call was inspected.
                    if !hook_notes.is_empty() {
                        outcome.output.push_str("\n\nPlugin hooks:\n- ");
                        outcome.output.push_str(&hook_notes.join("\n- "));
                    }
                    // Cross-session anomaly nudge: if another session is
                    // active in this workspace and this tool failed (or touched
                    // a file a peer is editing), append a note so the agent
                    // checks the neighbors before assuming it caused the error.
                    // Uses the cached peer snapshot — no filesystem read here.
                    if let Some(note) =
                        maybe_concurrency_note(st, &name, &exec_args, outcome.ok).await
                    {
                        outcome.output.push_str("\n\n");
                        outcome.output.push_str(&note);
                    }
                    // Debug log: records full tool args (file contents, commands) which may
                    // include secrets the model handles. Opt-in (cfg.debug_log), user-owned.
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
                    let tool_result = Message::tool(id.clone(), &outcome.output);
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
                // Turn complete — or, on a non-trivial turn, inject a reflect
                // continuation before the real completion (auto-reflect gate).
                let mut do_reflect = false;
                let mut recurrence = 0usize;
                let mut reflect_prompt = String::new();
                if !reflected {
                    if let Some((p, rec)) = maybe_reflect_prompt(
                        st,
                        &prompt,
                        turn_tool_calls,
                        &shape_tools,
                        &shape_files,
                        cancel.is_cancelled(),
                    )
                    .await
                    {
                        reflected = true;
                        reflect_prompt = p;
                        recurrence = rec;
                        do_reflect = true;
                    }
                }
                if do_reflect {
                    // Push the reflect prompt as a user message and re-stream.
                    let msg = Message::user(reflect_prompt);
                    let est = estimate_message_tokens(&msg);
                    let mut conv = st.conversation.lock().await;
                    conv.push(msg);
                    if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                        session::append(p, conv.last().unwrap());
                    }
                    *st.estimated_tokens.lock().await += est;
                    drop(conv);
                    emit(&Event::new("reflecting").with("recurrence", json!(recurrence)));
                    // Don't return → the outer loop re-streams the reflection.
                } else {
                    // Turn complete: emit metrics + done.
                    *st.last_turn_time.lock().await = std::time::Instant::now();
                    let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                    let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                    *st.last_turn_metrics.lock().await = Some(metrics.clone());
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
                    persist_stats(st).await;
                    maybe_finish_goal_planning(st, client, cancel.is_cancelled()).await;
                    emit(&Event::new("done"));
                    return;
                }
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

/// Outcome of a pending `ask` tool call.
pub(crate) enum AskResult {
    /// The user answered. Carries the validated questions array (for
    /// formatting) and the answers object (question id → answer).
    Answered { questions: Value, answers: Value },
    /// The user skipped the whole prompt (closed the flyout without answering).
    Skipped,
    /// The turn was aborted (/abort) while the ask was pending.
    Aborted,
}

/// Monotonic generator for globally-unique ask ids so concurrent asks (e.g.
/// from a parallel subagent that somehow gained the tool) never collide.
static ASK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Validate the `ask` tool args and return the normalized questions array.
/// Returns Err(message) on invalid input (sent back to the model as a tool
/// error WITHOUT blocking — the model can retry with a well-formed call).
pub(crate) fn validate_ask_questions(args: &Value) -> Result<Value, String> {
    let questions = args
        .get("questions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "ask requires a non-empty 'questions' array".to_string())?;
    if questions.is_empty() {
        return Err("ask 'questions' must not be empty".to_string());
    }
    let mut seen_ids = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(questions.len());
    for (i, q) in questions.iter().enumerate() {
        let id = q
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("question {i}: missing 'id'"))?
            .trim()
            .to_string();
        if id.is_empty() {
            return Err(format!("question {i}: 'id' must not be empty"));
        }
        if !seen_ids.insert(id.clone()) {
            return Err(format!("question {i}: duplicate id '{id}'"));
        }
        let prompt = q
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("question {i}: missing 'prompt'"))?
            .to_string();
        let typ = q
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("question {i}: missing 'type'"))?;
        let typ = match typ {
            "select" | "text" => typ,
            other => {
                return Err(format!(
                    "question {i}: invalid type '{other}' (select|text)"
                ))
            }
        };
        let options = if typ == "select" {
            let opts = q
                .get("options")
                .and_then(|v| v.as_array())
                .ok_or_else(|| format!("question {i}: type 'select' requires 'options'"))?;
            if opts.is_empty() {
                return Err(format!("question {i}: 'options' must not be empty"));
            }
            let strs: Vec<String> = opts
                .iter()
                .map(|o| o.as_str().unwrap_or("").to_string())
                .collect();
            Value::from(strs)
        } else {
            Value::Null
        };
        let required = q.get("required").and_then(|v| v.as_bool()).unwrap_or(true);
        let allow_custom = q
            .get("allowCustom")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let placeholder = q
            .get("placeholder")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(json!({
            "id": id,
            "prompt": prompt,
            "type": typ,
            "options": options,
            "allowCustom": allow_custom,
            "required": required,
            "placeholder": placeholder,
        }));
    }
    Ok(Value::from(out))
}

/// Format the user's answers into the model-facing tool-result string.
/// Skipped (unanswered optional) questions are listed as "(skipped)".
pub(crate) fn format_ask_answers(questions: &Value, answers: &Value) -> String {
    let qs = questions.as_array();
    let ans = answers.as_object();
    let mut lines = vec!["User answered:".to_string()];
    if let Some(qs) = qs {
        for q in qs {
            let id = q.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let prompt = q.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            let val = ans.and_then(|m| m.get(id)).and_then(|v| v.as_str());
            let display = match val {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => "(skipped)".to_string(),
            };
            lines.push(format!("- {id} ({prompt}): {display}"));
        }
    }
    lines.join("\n")
}

/// Ask the user one or more questions via the TUI/web flyout; block until
/// answered, skipped, or aborted. Mirrors `request_approval` but carries
/// structured answers back instead of a granted/denied bool.
pub(crate) async fn request_ask(
    st: &Arc<State>,
    args: &Value,
    cancel: &CancellationToken,
) -> AskResult {
    let questions = match validate_ask_questions(args) {
        Ok(q) => q,
        Err(e) => {
            // Validation failed BEFORE we block — surface as an info event and
            // return Skipped so the model gets a tool result it can act on.
            // (The dispatch wraps this: it formats the result.)
            emit(
                &Event::new("error").with("message", json!(format!("ask validation failed: {e}"))),
            );
            return AskResult::Skipped;
        }
    };
    let request_id = format!(
        "ask-{}",
        ASK_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let notify = Arc::new(Notify::new());
    let pending = Arc::new(PendingAsk {
        request_id: request_id.clone(),
        questions: questions.clone(),
        notify: notify.clone(),
        answers: Mutex::new(None),
    });
    st.pending_asks
        .lock()
        .await
        .insert(request_id.clone(), pending.clone());
    emit(
        &Event::new("ask_request")
            .with("request_id", json!(request_id))
            .with("questions", json!(questions)),
    );

    // Wait for the ask_reply command or abort.
    let answers = tokio::select! {
        _ = notify.notified() => pending.answers.lock().await.take(),
        _ = cancel.cancelled() => {
            st.pending_asks.lock().await.remove(&request_id);
            return AskResult::Aborted;
        }
    };
    st.pending_asks.lock().await.remove(&request_id);
    match answers {
        Some(v) if v.is_object() => AskResult::Answered {
            questions: pending.questions.clone(),
            answers: v,
        },
        // Some(Null) or Some(non-object) = user skipped the prompt.
        _ => AskResult::Skipped,
    }
}

/// Outcome of a pending sudo approval.
pub(crate) enum SudoResult {
    /// The user approved and supplied a password to feed `sudo -S` on stdin.
    Approved { password: String },
    /// The user declined (Esc) — the command was NOT run.
    Declined,
    /// The turn was aborted (/abort) while the sudo prompt was pending.
    Aborted,
}

/// Monotonic generator for globally-unique sudo-request ids.
static SUDO_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Ask the user to approve a bash command that invokes `sudo`. Blocks until the
/// user approves (with a password) or declines (Esc). Mirrors `request_ask`
/// but carries a single password string back instead of structured answers.
/// The password is used once to feed `sudo -S` on stdin and is never persisted.
pub(crate) async fn request_sudo(
    st: &Arc<State>,
    command: &str,
    cancel: &CancellationToken,
) -> SudoResult {
    let request_id = format!(
        "sudo-{}",
        SUDO_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let notify = Arc::new(Notify::new());
    let pending = Arc::new(PendingSudo {
        request_id: request_id.clone(),
        command: command.to_string(),
        notify: notify.clone(),
        result: Mutex::new(None),
    });
    st.pending_sudos
        .lock()
        .await
        .insert(request_id.clone(), pending.clone());
    emit(
        &Event::new("sudo_request")
            .with("request_id", json!(request_id))
            .with("command", json!(command)),
    );

    // Wait for the sudo_reply command or abort.
    let result = tokio::select! {
        _ = notify.notified() => pending.result.lock().await.take(),
        _ = cancel.cancelled() => {
            st.pending_sudos.lock().await.remove(&request_id);
            return SudoResult::Aborted;
        }
    };
    st.pending_sudos.lock().await.remove(&request_id);
    match result {
        Some(Some(pw)) => SudoResult::Approved { password: pw },
        // Some(None) = declined (Esc). None should not happen (notify implies resolved).
        _ => SudoResult::Declined,
    }
}

/// If the conversation ends with an assistant message carrying an unanswered
/// `ask` tool call (a prior core restart happened while blocked mid-`ask`),
/// return that call's id and its arguments as a `Value` ready for `request_ask`.
/// Only the LAST unanswered `ask` is returned (the one the prior core was blocked
/// on). A call whose arguments fail validation is skipped — the
/// orphan-sanitizer will later resolve it with a synthetic result. Returns None
/// on the common case of no trailing unanswered ask.
fn find_trailing_unanswered_ask(conv: &[Message]) -> Option<(String, Value)> {
    let last = conv.last()?;
    let calls = last.tool_calls()?;
    if calls.is_empty() {
        return None;
    }
    // Tool-call ids that already have a matching `role:"tool"` result.
    let answered: HashSet<&str> = conv
        .iter()
        .filter_map(|m| if m.is_tool() { m.tool_call_id() } else { None })
        .collect();
    // Prefer the last unanswered `ask` (most likely the one that was blocking).
    for tc in calls.iter().rev() {
        if tc.function.name != "ask" || answered.contains(tc.id.as_str()) {
            continue;
        }
        let args: Value =
            serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
        if validate_ask_questions(&args).is_ok() {
            return Some((tc.id.clone(), args));
        }
    }
    None
}

/// Re-present an `ask` question that a prior core restart left unanswered, so
/// the question is not lost and the session is not wedged. Called at the top of
/// each turn; idempotent (a no-op once the ask has a result). The assistant
/// `ask` tool_call is already persisted; this appends the matching tool result
/// (the user's answers, or a "skipped" note) so the orphan-sanitizer never has
/// to insert a synthetic EMPTY result that would silently drop the question.
async fn resume_pending_ask(st: &Arc<State>, cancel: &CancellationToken) {
    let (call_id, args) = {
        let conv = st.conversation.lock().await;
        match find_trailing_unanswered_ask(&conv[..]) {
            Some(x) => x,
            None => return,
        }
    };
    // Re-present (fresh ask request_id) and block for the reply. An abort (the
    // turn was cancelled while re-presenting) ends the turn like the in-turn
    // ask abort; a skip resolves the orphan with a best-judgment note.
    let content = match request_ask(st, &args, cancel).await {
        AskResult::Answered { questions, answers } => {
            format_ask_answers(&questions, &answers)
        }
        AskResult::Skipped => {
            "The user skipped the questions. Proceed with your best judgment and note any assumptions."
                .to_string()
        }
        AskResult::Aborted => {
            emit(&Event::new("aborted"));
            emit(&Event::new("done"));
            return;
        }
    };
    let tool_result = Message::tool(&call_id, &content);
    let est = estimate_message_tokens(&tool_result);
    {
        let mut conv = st.conversation.lock().await;
        conv.push(tool_result);
        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
            session::append(p, conv.last().unwrap());
        }
    }
    *st.estimated_tokens.lock().await += est;
    emit(
        &Event::new("tool_result")
            .with("id", json!(call_id))
            .with("ok", json!(true))
            .with("output", json!(content)),
    );
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
pub fn digest_stale_tool_results(messages: &mut Vec<Message>, keep: usize) -> usize {
    if messages.len() <= keep {
        return 0;
    }
    // Build tool_call_id -> (tool_name, args_json) from assistant tool_calls so
    // the digest records WHAT was read/run, not just the size.
    let mut call_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for m in messages.iter() {
        if !m.is_assistant() {
            continue;
        }
        if let Some(calls) = m.tool_calls() {
            for tc in calls {
                if tc.id.is_empty() {
                    continue;
                }
                call_map.insert(
                    tc.id.clone(),
                    (tc.function.name.clone(), tc.function.arguments.clone()),
                );
            }
        }
    }
    let digest_to = messages.len().saturating_sub(keep);
    let mut changed = 0usize;
    for m in messages[..digest_to].iter_mut() {
        if !m.is_tool() {
            continue;
        }
        let content = match m.content_text() {
            Some(c) => c,
            None => continue,
        };
        if content.starts_with("[digested:") || content.len() <= DIGEST_MIN_BYTES {
            continue;
        }
        let id = m.tool_call_id().unwrap_or("").to_string();
        let (name, args_json) = call_map.get(&id).cloned().unwrap_or_default();
        let lines = content.lines().count();
        let digest = make_digest(&name, &args_json, content.len(), lines);
        if let Message::Tool {
            ref mut content, ..
        } = m
        {
            *content = digest;
            changed += 1;
        }
    }
    changed
}

/// Build a one-line digest for a tool result, preserving enough to navigate
/// Last-resort token reclaim for compaction: collapse oversized `role:"tool"`
/// results into one-line digests until `messages` fits under `budget` tokens.
/// Unlike `digest_stale_tool_results` (which only touches results older than a
/// keep-window and bails on small conversations), this digests ANY eligible
/// tool result — including recent ones — oldest-first, stopping as soon as the
/// budget is met so the most recent results stay verbatim when possible.
///
/// This is what makes compaction effective when a few huge tool results (large
/// file reads, verbose command output) dominate the context: dropping old
/// turns can't reclaim enough because the bulk lives in the kept tail, but
/// collapsing those results to a one-liner (with a re-run hint) drops 100k+
/// tokens at a time. `tool_call_id` + `role` are preserved, so tool-call/result
/// pairing and orphan-sanitization stay intact. Returns the count digested.
fn digest_to_budget(messages: &mut [Message], budget: u64) -> usize {
    if estimate_messages_tokens(messages) <= budget {
        return 0;
    }
    // tool_call_id -> (tool_name, args_json) from assistant tool_calls, so the
    // digest records WHAT was read/run, not just the size.
    let mut call_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for m in messages.iter() {
        if !m.is_assistant() {
            continue;
        }
        if let Some(calls) = m.tool_calls() {
            for tc in calls {
                if tc.id.is_empty() {
                    continue;
                }
                call_map.insert(
                    tc.id.clone(),
                    (tc.function.name.clone(), tc.function.arguments.clone()),
                );
            }
        }
    }
    // Walk oldest-first, collapsing oversized tool results until the budget is
    // met. Recent results are processed last and so stay verbatim when earlier
    // digests already reached the budget.
    let mut changed = 0usize;
    for i in 0..messages.len() {
        if estimate_messages_tokens(messages) <= budget {
            break;
        }
        if !messages[i].is_tool() {
            continue;
        }
        let content = match messages[i].content_text() {
            Some(c) => c,
            None => continue,
        };
        if content.starts_with("[digested:") || content.len() <= DIGEST_MIN_BYTES {
            continue;
        }
        let id = messages[i].tool_call_id().unwrap_or("").to_string();
        let (name, args_json) = call_map.get(&id).cloned().unwrap_or_default();
        let lines = content.lines().count();
        let digest = make_digest(&name, &args_json, content.len(), lines);
        if let Message::Tool {
            ref mut content, ..
        } = messages[i]
        {
            *content = digest;
            changed += 1;
        }
    }
    changed
}

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
    let (ws, auto_reflect) = {
        let c = state.cfg.read().await;
        (c.workspace.clone(), c.auto_reflect)
    };
    let mem = memory_injection(&ws, "");
    let new_system = build_main_system_prompt(&ws, &state.plugin_manager, auto_reflect);
    let mut conv = state.conversation.lock().await;
    if let Some(first) = conv.first() {
        let old_content = first.content_text().unwrap_or("");
        if old_content == new_system {
            return "memory unchanged; system prompt kept intact (preserving prefix cache)"
                .to_string();
        }
    }
    if let Some(first) = conv.first_mut() {
        if first.is_system() {
            *first = Message::system(new_system);
            *state.estimated_tokens.lock().await = estimate_messages_tokens(&conv);
            // System prompt changed; the real baseline's system portion is stale.
            state.invalidate_real_token_baseline().await;
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
fn token_budget_tail_start(messages: &[Message], context_window: u64) -> usize {
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
/// Hint the system allocator to release freed heap pages back to the OS.
///
/// Rust's default allocator on glibc Linux does NOT eagerly return freed memory:
/// a turn clones the conversation several times (lock-across-await forces
/// clones) and compaction drops the old copies, but the freed bytes stay in
/// malloc's arenas, so RSS creeps up over a long session and never falls back
/// (the "starts at 11M, now 27M" symptom). `malloc_trim(0)` releases the free
/// top-of-arena pages. Called once per turn — negligible vs a multi-second
/// model turn. No-op on non-glibc targets (musl/macOS/Windows return freed
/// memory to the OS far more eagerly on their own).
#[cfg(all(unix, target_env = "gnu"))]
fn trim_heap() {
    extern "C" {
        fn malloc_trim(pad: usize) -> std::os::raw::c_int;
    }
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(unix, target_env = "gnu")))]
fn trim_heap() {}

pub fn compact_conversation(messages: &mut Vec<Message>, context_window: u64) {
    if messages.len() <= 2 {
        return;
    }
    let system = messages[0].clone();
    let tail_start = token_budget_tail_start(messages, context_window).max(1);
    let tail: Vec<Message> = messages[tail_start..].to_vec();
    let mut compacted = vec![system];
    compacted.push(Message::system("[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped.]"));
    compacted.extend(tail);
    // The kept tail can still hold the bulk of the tokens when a few tool
    // results are huge (large file reads, verbose command output). Dropping old
    // turns reclaims nothing there; collapse those oversized results into
    // one-line digests until the conversation fits under half the window.
    let budget = ((context_window as f32) * 0.5) as u64;
    digest_to_budget(&mut compacted, budget);
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
    messages: &mut Vec<Message>,
    cancel: &CancellationToken,
    force_summarize: bool,
    context_window: u64,
    instructions: Option<&str>,
) -> usize {
    // Returns the character count of the produced summary system message (0
    // when no summary was generated — naive drop-oldest fallback or a
    // failed/too-small summarize). Surfaced on the `compacted` event so the
    // TUI can show how big the rolling summary is.
    if messages.len() <= 2 {
        return 0;
    }
    if !cfg.summarize_on_compact && !force_summarize {
        compact_conversation(messages, context_window);
        return 0;
    }
    let tail_start = token_budget_tail_start(messages, context_window).max(1);
    if tail_start <= 1 {
        compact_conversation(messages, context_window);
        return 0;
    }
    let to_summarize: Vec<Message> = messages[1..tail_start].to_vec();
    let kept: Vec<Message> = messages[tail_start..].to_vec();
    let summary =
        provider::summarize(client, provider, model, &to_summarize, cancel, instructions).await;
    let mut summary_chars = 0usize;
    let mut compacted = vec![messages[0].clone()];
    if let Some(s) = summary {
        let content = format!("[Summary of earlier turns]\n{s}");
        summary_chars = content.chars().count();
        compacted.push(Message::system(content));
    } else {
        compacted.push(Message::system("[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped; summarization was unavailable.]"));
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
    // The kept tail can still hold the bulk of the tokens when a few recent
    // tool results are huge. Collapse them so the compacted conversation
    // actually fits the window instead of no-op'ing back to its original size.
    compacted.extend(kept);
    let budget = ((context_window as f32) * 0.5) as u64;
    digest_to_budget(&mut compacted, budget);
    *messages = compacted;
    summary_chars
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
mod skill_manifest_tests {
    use super::*;

    fn write_skill(dir: &std::path::Path, name: &str, desc: &str) {
        write_skill_raw(dir, name, &format!("name: {name}\ndescription: {desc}\n"))
    }

    /// Write a SKILL.md with arbitrary extra frontmatter lines (for deprecated, etc.).
    fn write_skill_raw(dir: &std::path::Path, name: &str, frontmatter_body: &str) {
        let p = dir
            .join(".catalyst-code/skills")
            .join(name)
            .join("SKILL.md");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("---\n{frontmatter_body}---\nbody\n")).unwrap();
    }

    fn fresh_workspace() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catalyst-code-skill-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn manifest_lists_opt_in_skills_excludes_pi_subagents() {
        let ws = fresh_workspace();
        write_skill(&ws, "foo", "Foo skill");
        write_skill(&ws, "nodesc", "");
        write_skill(&ws, "pi-subagents", "should be excluded (injected in full)");
        let m = skill_manifest_injection(&ws);
        assert!(m.contains("foo"), "manifest should list foo: {m}");
        assert!(
            m.contains("Foo skill"),
            "manifest should include description: {m}"
        );
        // A skill with no description renders without a colon-suffix.
        assert!(m.contains("- nodesc"), "manifest should list nodesc: {m}");
        assert!(
            !m.lines().any(|l| l.starts_with("- pi-subagents")),
            "pi-subagents must be excluded from the manifest: {m}"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn manifest_excludes_deprecated_skills_and_uses_dirname() {
        let ws = fresh_workspace();
        write_skill(&ws, "active", "An active skill");
        write_skill_raw(
            &ws,
            "old",
            "name: old\ndescription: A deprecated skill\ndeprecated: true\n",
        );
        // frontmatter `name` deliberately != dir name; the manifest must use the
        // DIR name (the resolvable path component), not the frontmatter name.
        write_skill_raw(
            &ws,
            "realdir",
            "name: pretty-display-name\ndescription: uses a fancy name\n",
        );
        let m = skill_manifest_injection(&ws);
        assert!(m.contains("active"), "active skill should appear: {m}");
        assert!(
            !m.lines().any(|l| l.starts_with("- old")),
            "deprecated skill must be excluded: {m}"
        );
        assert!(
            m.contains("- realdir:"),
            "manifest must use the dir name, not the frontmatter name: {m}"
        );
        assert!(
            !m.contains("pretty-display-name"),
            "frontmatter name should not leak into the manifest path: {m}"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    fn asst_tool_call(id: &str, name: &str, args: &str) -> Message {
        Message::assistant_tool_calls(vec![ToolCall {
            id: id.into(),
            typ: "function".into(),
            function: message::FunctionCall {
                name: name.into(),
                arguments: args.into(),
            },
        }])
    }
    fn tool_result(id: &str, content: &str) -> Message {
        Message::tool(id, content)
    }
    fn asst_text(t: &str) -> Message {
        Message::assistant(t)
    }

    fn big_content(n: usize) -> String {
        "x\n".repeat(n)
    }

    /// system + a stale large read result + padding + a recent large read result.
    fn fixture() -> Vec<Message> {
        let mut m = vec![Message::system("sys")];
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
        let d = m[2].content_text().unwrap();
        assert!(d.starts_with("[digested:"), "{}", d);
        assert!(d.contains("read_file"), "{}", d);
        assert!(d.contains("src/big.rs"), "{}", d);
        assert!(d.contains("lines"), "should report line count: {}", d);
        assert!(d.contains("re-run to recover full output"), "{}", d);
        // tool_call_id preserved so the assistant/tool pairing stays valid
        assert_eq!(m[2].tool_call_id(), Some("call_1"));
        // recent large result (inside the keep tail) is untouched
        let r = m[m.len() - 2].content_text().unwrap();
        assert_eq!(r.len(), 280, "recent result kept full: {}", r);
        assert!(!r.starts_with("[digested:"));
        assert_eq!(m[m.len() - 2].tool_call_id(), Some("call_2"));
    }

    #[test]
    fn digest_is_idempotent() {
        let mut m = fixture();
        let n1 = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n1, 1);
        let after = m[2].content_text().unwrap().to_string();
        let n2 = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n2, 0, "second pass must find nothing to digest");
        assert_eq!(m[2].content_text(), Some(after.as_str()));
    }

    #[test]
    fn digest_skips_small_results() {
        let mut m = vec![
            Message::system("sys"),
            asst_tool_call("c1", "edit", "{\"path\":\"a.rs\"}"),
            tool_result("c1", "applied 1 edit(s)"), // 17 bytes — under MIN_BYTES
        ];
        // pad to push it out of the keep window
        for i in 0..12 {
            m.push(asst_text(&format!("p{i}")));
        }
        let n = digest_stale_tool_results(&mut m, 10);
        assert_eq!(n, 0, "small result must not be digested");
        assert_eq!(m[2].content_text(), Some("applied 1 edit(s)"));
    }

    #[test]
    fn digest_noop_when_under_keep() {
        let mut m = vec![
            Message::system("sys"),
            asst_tool_call("c1", "read_file", "{\"path\":\"a.rs\"}"),
            tool_result("c1", &big_content(200)),
        ];
        // only 3 messages, keep=10 → nothing eligible
        assert_eq!(digest_stale_tool_results(&mut m, 10), 0);
        assert_eq!(m[2].content_text().unwrap().len(), 400);
    }

    #[test]
    fn digest_bash_label_says_rerun_if_needed() {
        let mut m = vec![
            Message::system("sys"),
            asst_tool_call("c1", "bash", "{\"command\":\"cargo build\"}"),
            tool_result("c1", &big_content(150)),
        ];
        for i in 0..12 {
            m.push(asst_text(&format!("p{i}")));
        }
        digest_stale_tool_results(&mut m, 10);
        let d = m[2].content_text().unwrap();
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

    fn sys() -> Message {
        Message::system("sys")
    }
    fn user(t: &str) -> Message {
        Message::user(t)
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
        m.push(Message::tool("x", "x".repeat(50_000)));
        let s = token_budget_tail_start(&m, 1000); // floor budget 6k
        assert!(
            s >= m.len() - 7 && s <= m.len() - 6,
            "kept the giant result + min tail: {s}"
        );
    }

    fn asst_tool_call(id: &str, name: &str, args: &str) -> Message {
        Message::assistant_tool_calls(vec![ToolCall {
            id: id.into(),
            typ: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: args.into(),
            },
        }])
    }
    fn tool_result(id: &str, content: &str) -> Message {
        Message::tool(id, content)
    }

    #[test]
    fn digest_to_budget_collapses_oversized_results() {
        // Few messages but huge tokens — the case that used to no-op compaction.
        // ~250k chars ≈ 62k tokens each; two of them exceed a 100k-token budget.
        let huge = "x\n".repeat(125_000);
        let mut m = vec![
            sys(),
            asst_tool_call("c1", "read_file", "{\"path\":\"old.rs\"}"),
            tool_result("c1", &huge),
            asst_tool_call("c2", "read_file", "{\"path\":\"recent.rs\"}"),
            tool_result("c2", &huge),
            user("go"),
        ];
        let before = estimate_messages_tokens(&m);
        assert!(before > 100_000, "fixture should be large: {before}");
        // Digest oldest-first until under budget; the recent result is processed
        // last and stays verbatim once the older one already fit the budget.
        let n = digest_to_budget(&mut m, 100_000);
        assert_eq!(n, 1, "only the older result needs digesting: {n}");
        let d = m[2].content_text().unwrap();
        assert!(d.starts_with("[digested:"), "{d}");
        assert!(d.contains("old.rs"), "{d}");
        assert_eq!(m[2].tool_call_id(), Some("c1"));
        assert!(
            !m[4].content_text().unwrap().starts_with("[digested:"),
            "recent result kept verbatim"
        );
        let after = estimate_messages_tokens(&m);
        assert!(
            after < before - 80_000,
            "must reclaim a huge result: {after} vs {before}"
        );
    }

    #[test]
    fn digest_to_budget_noop_under_budget() {
        let mut m = vec![
            sys(),
            asst_tool_call("c1", "read_file", "{\"path\":\"a.rs\"}"),
            tool_result("c1", &"x\n".repeat(125_000)),
        ];
        // ~62k tokens, budget 100k → already fits, nothing digested.
        assert_eq!(digest_to_budget(&mut m, 100_000), 0);
        assert!(!m[2].content_text().unwrap().starts_with("[digested:"));
    }

    #[test]
    fn compact_conversation_reclaims_few_huge_messages() {
        // Regression: a conversation with only 6 messages but ~125k tokens used
        // to bail out of compaction entirely (the old `len <= 12` guard left
        // before == after), so the next request hit a context-window 400.
        let huge = "x\n".repeat(125_000);
        let mut m = vec![
            sys(),
            asst_tool_call("c1", "read_file", "{\"path\":\"a.rs\"}"),
            tool_result("c1", &huge),
            asst_tool_call("c2", "read_file", "{\"path\":\"b.rs\"}"),
            tool_result("c2", &huge),
            user("continue"),
        ];
        let before = estimate_messages_tokens(&m);
        compact_conversation(&mut m, 200_000);
        let after = estimate_messages_tokens(&m);
        assert!(
            after < before,
            "compaction must reduce tokens (was a no-op): {before} -> {after}"
        );
        assert!(
            after < 100_000,
            "should be well under half the window: {after}"
        );
        // Both tool messages survive (pairing intact); the older one is digested.
        assert_eq!(m.iter().filter(|x| x.is_tool()).count(), 2);
    }
}

#[cfg(test)]
mod work_state_tests {
    use super::*;

    fn todo(subject: &str, status: &str) -> Value {
        json!({ "subject": subject, "status": status })
    }

    #[test]
    fn render_empty_shows_placeholder_and_omits_sections() {
        let ws = WorkState::default();
        assert!(ws.is_empty());
        let r = ws.render();
        assert!(r.contains("Goal: (not yet stated)"));
        // Empty lists must not emit their headers.
        assert!(!r.contains("Done:"));
        assert!(!r.contains("In progress:"));
        assert!(!r.contains("Next:"));
        assert!(!r.contains("Recently touched:"));
    }

    #[test]
    fn render_includes_all_populated_sections() {
        let ws = WorkState {
            goal: "ship context management".into(),
            done: vec!["design".into()],
            in_progress: vec!["implement".into()],
            next: vec!["test".into(), "doc".into()],
            recent_files: vec!["core/src/main.rs".into()],
            last_activity: "edit core/src/main.rs".into(),
            ..Default::default()
        };
        let r = ws.render();
        assert!(r.contains("Goal: ship context management"));
        assert!(r.contains("Done:"));
        assert!(r.contains("- design"));
        assert!(r.contains("In progress:"));
        assert!(r.contains("- implement"));
        assert!(r.contains("Next:"));
        assert!(r.contains("- test"));
        assert!(r.contains("- doc"));
        assert!(r.contains("Recently touched: core/src/main.rs"));
        assert!(r.contains("Last: edit core/src/main.rs"));
        // Framing so the model treats it as ambient, not a prompt to answer.
        assert!(r.contains("respond to the user's latest message"));
    }

    #[test]
    fn render_caps_long_lists() {
        let ws = WorkState {
            goal: "g".into(),
            done: (0..10).map(|i| format!("item {i}")).collect(),
            ..Default::default()
        };
        let r = ws.render();
        // Only the first MAX_LIST (6) entries appear verbatim ...
        assert!(r.contains("- item 0"));
        assert!(r.contains("- item 5"));
        assert!(!r.contains("- item 6"));
        // ... and the overflow is summarized.
        assert!(r.contains("… +4 more"));
    }

    #[test]
    fn sync_from_todos_partitions_by_status() {
        let mut ws = WorkState {
            goal: "g".into(),
            ..Default::default()
        };
        let todos = vec![
            todo("design", "completed"),
            todo("implement", "in_progress"),
            todo("test", "pending"),
            todo("doc", "pending"),
        ];
        ws.sync_from_todos(&todos);
        assert_eq!(ws.done, vec!["design"]);
        assert_eq!(ws.in_progress, vec!["implement"]);
        assert_eq!(ws.next, vec!["test", "doc"]);
        assert!(ws.version > 0);
    }

    #[test]
    fn sync_from_todos_skips_empty_subjects() {
        let mut ws = WorkState::default();
        let todos = vec![todo("", "completed"), todo("real", "in_progress")];
        ws.sync_from_todos(&todos);
        assert!(ws.done.is_empty());
        assert_eq!(ws.in_progress, vec!["real"]);
    }

    #[test]
    fn record_files_dedup_and_mru_order() {
        let mut ws = WorkState::default();
        ws.record_files("edit", &["a.rs".into(), "b.rs".into()]);
        assert_eq!(ws.recent_files, vec!["a.rs", "b.rs"]);
        // Touching an existing file moves it to the front (most-recent-first).
        ws.record_files("edit", &["a.rs".into()]);
        assert_eq!(ws.recent_files, vec!["a.rs", "b.rs"]);
        assert_eq!(ws.last_activity, "edit a.rs");
    }

    #[test]
    fn record_files_caps_at_eight() {
        let mut ws = WorkState::default();
        for i in 0..12 {
            ws.record_files("edit", &[format!("f{i}.rs")]);
        }
        assert_eq!(ws.recent_files.len(), 8);
        // Most-recent (f11) is at the front.
        assert_eq!(ws.recent_files[0], "f11.rs");
    }

    #[test]
    fn peers_touching_matches_exact_normalized_path() {
        let mk = |pid: u32, files: &[&str]| presence::PresenceRecord {
            pid,
            session_id: None,
            started_at: 0,
            last_heartbeat: 0,
            goal: String::new(),
            in_progress: vec![],
            next: vec![],
            recent_files: files.iter().map(|s| s.to_string()).collect(),
            last_activity: String::new(),
            model: None,
        };
        let peers = vec![mk(111, &["core/src/main.rs"]), mk(222, &["other.go"])];
        // exact match → the touching peer's pid
        assert_eq!(
            peers_touching(&peers, "edit", &json!({"path":"core/src/main.rs"})),
            "pid 111"
        );
        // separator-normalized (backslash) still matches
        assert_eq!(
            peers_touching(&peers, "write_file", &json!({"path":"core\\src\\main.rs"})),
            "pid 111"
        );
        // a path nobody is touching → empty (no false positive)
        assert_eq!(
            peers_touching(&peers, "read_file", &json!({"path":"foo.rs"})),
            ""
        );
        // a non-file tool (bash) → empty
        assert_eq!(peers_touching(&peers, "bash", &json!({"command":"ls"})), "");
        // multiple touching peers → comma-list
        let peers2 = vec![mk(111, &["shared.rs"]), mk(333, &["shared.rs"])];
        let s = peers_touching(&peers2, "edit", &json!({"path":"shared.rs"}));
        assert!(s.contains("pid 111") && s.contains("pid 333"));
    }
}

#[cfg(test)]
mod auto_reflect_tests {
    use super::*;

    #[test]
    fn learning_turns_are_exempt() {
        // The prefixes the TUI's /reflect and /index delegations produce.
        assert!(is_learning_turn(
            "Reflect on the work done in this session so far…"
        ));
        assert!(is_learning_turn(
            "Run a full knowledge index of this repository now."
        ));
        assert!(is_learning_turn(
            "  Run an incremental knowledge index of this repository"
        ));
        // A normal user task is not exempt.
        assert!(!is_learning_turn("Add a release packaging flow"));
        assert!(!is_learning_turn("fix the typo in README"));
    }

    #[test]
    fn reflect_text_mentions_memory_and_skills() {
        let txt = build_reflect_text(&[]);
        assert!(txt.contains("[auto-reflect]"));
        assert!(txt.contains("memory"));
        assert!(txt.contains("skills/<name>/SKILL.md"));
        assert!(txt.contains("finish"));
        // No recurrence → no recurring-patterns section.
        assert!(!txt.contains("Recurring patterns detected"));
    }

    #[test]
    fn reflect_text_lists_recurring_patterns() {
        let rec = vec![
            (3, "add a core tool".into()),
            (2, "add a tui renderer".into()),
        ];
        let txt = build_reflect_text(&rec);
        assert!(txt.contains("Recurring patterns detected"));
        assert!(txt.contains("add a core tool (3 times)"));
        assert!(txt.contains("add a tui renderer (2 times)"));
    }

    #[test]
    fn extract_file_categories_for_file_tools() {
        let cats = extract_file_categories("edit", r#"{"path":"core/src/main.rs"}"#);
        assert_eq!(cats, vec!["core/src/*.rs".to_string()]);
        // bulk_write: one category per file, deduped by shape_signature later.
        let cats = extract_file_categories(
            "bulk_write",
            r#"{"files":[{"path":"tui/render.go"},{"path":"tui/handlers.go"}]}"#,
        );
        assert_eq!(cats.len(), 2);
        assert!(cats.iter().all(|c| c == "tui/*.go"));
    }

    #[test]
    fn extract_file_categories_empty_for_non_file_tools() {
        assert!(extract_file_categories("bash", r#"{"command":"ls"}"#).is_empty());
        assert!(extract_file_categories("grep", r#"{"pattern":"x"}"#).is_empty());
        // Malformed JSON is tolerated (no panic, no categories).
        assert!(extract_file_categories("edit", "not json").is_empty());
    }
}

#[cfg(test)]
mod restricted_path_tests {
    use super::*;

    // A throwaway workspace root for the canonical-path re-check. Unique per
    // call so parallel `cargo test` never collides (mirrors workspace::tmp_root).
    fn root() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("umans_rpt_test_{}", n));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn read_file_restricted_path_detected() {
        let r = root();
        // A direct read of a restricted file is flagged so the gate can prompt.
        let args = json!({"path": ".env"});
        assert!(restricted_path_for_tool("read_file", &args, &r).is_some());
        // Safe path: no flag.
        let args = json!({"path": "src/main.rs"});
        assert!(restricted_path_for_tool("read_file", &args, &r).is_none());
    }

    #[test]
    fn write_edit_patch_restricted_paths_detected() {
        let r = root();
        for tool in ["write_file", "edit", "patch"] {
            let args = json!({"path": ".git/config"});
            assert!(
                restricted_path_for_tool(tool, &args, &r).is_some(),
                "{tool} should flag .git/config"
            );
        }
        let args = json!({"path": "README.md"});
        assert!(restricted_path_for_tool("write_file", &args, &r).is_none());
    }

    #[test]
    fn case_insensitive_match() {
        let r = root();
        // .ENV / .GIT/config must still match on case-insensitive filesystems.
        assert!(restricted_path_for_tool("read_file", &json!({"path": ".ENV"}), &r).is_some());
        assert!(
            restricted_path_for_tool("write_file", &json!({"path": ".GIT/config"}), &r).is_some()
        );
    }

    #[test]
    fn bulk_read_flags_any_restricted() {
        let r = root();
        let args = json!({"paths": ["ok.txt", ".env", "other.rs"]});
        assert!(restricted_path_for_tool("bulk_read", &args, &r).is_some());
        let args = json!({"paths": ["a.rs", "b.go"]});
        assert!(restricted_path_for_tool("bulk_read", &args, &r).is_none());
    }

    #[test]
    fn bulk_write_and_bulk_edit_flag_restricted() {
        let r = root();
        let args = json!({"files": [{"path": ".env", "content": "x"}]});
        assert!(restricted_path_for_tool("bulk_write", &args, &r).is_some());
        let args = json!({"edits": [{"path": ".ssh/config", "edits": []}]});
        assert!(restricted_path_for_tool("bulk_edit", &args, &r).is_some());
        let args = json!({"files": [{"path": "ok.txt", "content": "x"}]});
        assert!(restricted_path_for_tool("bulk_write", &args, &r).is_none());
    }

    #[test]
    fn bulk_recurses_into_inner_calls() {
        let r = root();
        // A bulk containing an inner write to a restricted path is flagged so
        // the whole bulk prompts (then approved inner calls proceed).
        let args = json!({"calls": [
            {"name": "read_file", "args": {"path": "ok.txt"}},
            {"name": "write_file", "args": {"path": ".env", "content": "LEAK=1"}}
        ]});
        assert!(restricted_path_for_tool("bulk", &args, &r).is_some());
        // All-safe bulk: no flag.
        let args = json!({"calls": [
            {"name": "read_file", "args": {"path": "a.rs"}},
            {"name": "write_file", "args": {"path": "b.go", "content": "x"}}
        ]});
        assert!(restricted_path_for_tool("bulk", &args, &r).is_none());
    }

    #[test]
    fn excluded_tools_never_flag() {
        let r = root();
        // bash/grep/glob/list_dir are intentionally excluded — they don't read a
        // single restricted file's content by path.
        assert!(restricted_path_for_tool("bash", &json!({"command": "cat .env"}), &r).is_none());
        assert!(
            restricted_path_for_tool("grep", &json!({"pattern": "x", "path": "src"}), &r).is_none()
        );
        assert!(restricted_path_for_tool("glob", &json!({"pattern": "**/*"}), &r).is_none());
        assert!(restricted_path_for_tool("list_dir", &json!({"path": "."}), &r).is_none());
    }

    // Regression (M1): a symlink alias to a restricted dir (linkdir -> .git)
    // must be flagged. The raw path "linkdir/config" contains no `.git`
    // component, so only the canonical (symlink-resolved) re-check catches it.
    #[cfg(unix)]
    #[test]
    fn symlink_alias_to_restricted_is_flagged() {
        use std::os::unix::fs::symlink;
        let r = root();
        // `linkdir` is a relative symlink to the in-workspace `.git` dir.
        std::fs::create_dir_all(r.join(".git")).unwrap();
        symlink(".git", r.join("linkdir")).unwrap();
        // Reading through the alias must prompt (canonical path = <root>/.git/config).
        let args = json!({"path": "linkdir/config"});
        assert!(
            restricted_path_for_tool("read_file", &args, &r).is_some(),
            "symlink alias to .git must be flagged by the canonical re-check"
        );
        // The literal restricted path is also flagged.
        let args = json!({"path": ".git/config"});
        assert!(restricted_path_for_tool("read_file", &args, &r).is_some());
        // A genuinely safe path is not flagged.
        let args = json!({"path": "src/main.rs"});
        assert!(restricted_path_for_tool("read_file", &args, &r).is_none());
    }
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_and_missing() {
        assert!(validate_ask_questions(&json!({})).is_err());
        assert!(validate_ask_questions(&json!({"questions": []})).is_err());
        // missing id
        assert!(
            validate_ask_questions(&json!({"questions": [{"prompt":"p","type":"text"}]})).is_err()
        );
        // missing prompt
        assert!(validate_ask_questions(&json!({"questions": [{"id":"a","type":"text"}]})).is_err());
        // invalid type
        assert!(validate_ask_questions(
            &json!({"questions": [{"id":"a","prompt":"p","type":"radio"}]})
        )
        .is_err());
    }

    #[test]
    fn validate_select_requires_options() {
        assert!(validate_ask_questions(
            &json!({"questions": [{"id":"a","prompt":"p","type":"select"}]})
        )
        .is_err());
        assert!(validate_ask_questions(
            &json!({"questions": [{"id":"a","prompt":"p","type":"select","options":[]}]})
        )
        .is_err());
        // valid select
        let q = validate_ask_questions(
            &json!({"questions": [{"id":"a","prompt":"p","type":"select","options":["x","y"]}]}),
        )
        .unwrap();
        assert_eq!(q.as_array().unwrap().len(), 1);
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let r = validate_ask_questions(&json!({"questions": [
            {"id":"a","prompt":"p","type":"text"},
            {"id":"a","prompt":"q","type":"text"}
        ]}));
        assert!(r.is_err());
    }

    #[test]
    fn format_answers_marks_skipped() {
        let qs = validate_ask_questions(&json!({"questions": [
            {"id":"fw","prompt":"Which framework?","type":"select","options":["React","Vue"]},
            {"id":"notes","prompt":"Any notes?","type":"text","required":false}
        ]}))
        .unwrap();
        // answered fw, skipped notes
        let out = format_ask_answers(&qs, &json!({"fw": "React"}));
        assert!(out.contains("fw (Which framework?): React"));
        assert!(out.contains("notes (Any notes?): (skipped)"));
    }

    #[test]
    fn format_answers_all_answered() {
        let qs = validate_ask_questions(&json!({"questions": [
            {"id":"a","prompt":"Q1","type":"text"}
        ]}))
        .unwrap();
        let out = format_ask_answers(&qs, &json!({"a": "hello"}));
        assert!(out.contains("a (Q1): hello"));
        assert!(!out.contains("(skipped)"));
    }
}

#[cfg(test)]
mod expand_mentions_tests {
    use super::*;

    fn fresh_workspace() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catalyst-code-mentions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn inlines_existing_file() {
        let ws = fresh_workspace();
        std::fs::write(ws.join("main.rs"), "fn main() {}\n").unwrap();
        let (out, attached) = expand_file_mentions("fix @main.rs please", &ws, u64::MAX);
        assert_eq!(attached, vec!["main.rs".to_string()]);
        assert!(out.contains("<file path=\"main.rs\">"));
        assert!(out.contains("fn main() {}"));
        assert!(out.contains("</file>"));
        // The surrounding prose is preserved (no trailing newline in input).
        assert!(out.starts_with("fix "));
        assert!(out.ends_with(" please"));
    }

    #[test]
    fn leaves_missing_path_as_is() {
        let ws = fresh_workspace();
        let (out, attached) = expand_file_mentions("look at @nope.rs", &ws, u64::MAX);
        assert!(attached.is_empty());
        assert_eq!(out, "look at @nope.rs");
    }

    #[test]
    fn email_not_triggered() {
        // `foo@bar` has no whitespace before `@`, so it must NOT be a mention.
        let ws = fresh_workspace();
        std::fs::write(ws.join("bar"), "x").unwrap();
        let (out, attached) = expand_file_mentions("email foo@bar.com here", &ws, u64::MAX);
        assert!(attached.is_empty());
        assert_eq!(out, "email foo@bar.com here");
    }

    #[test]
    fn inline_param_tag_not_triggered_without_space() {
        // `@param` embedded mid-word (no leading space) is left alone even if a
        // file named `param` exists.
        let ws = fresh_workspace();
        std::fs::write(ws.join("param"), "x").unwrap();
        let (out, attached) = expand_file_mentions("see the@param tag", &ws, u64::MAX);
        assert!(attached.is_empty());
        assert_eq!(out, "see the@param tag");
    }

    #[test]
    fn strips_trailing_punctuation() {
        let ws = fresh_workspace();
        std::fs::write(ws.join("file.rs"), "pub fn f() {}\n").unwrap();
        let (out, attached) = expand_file_mentions("see @file.rs.", &ws, u64::MAX);
        assert_eq!(attached, vec!["file.rs".to_string()]);
        assert!(out.contains("<file path=\"file.rs\">"));
        // A file with a trailing dot literally does not exist, so the literal
        // candidate is skipped and the trimmed one wins.
        assert!(!out.contains("<file path=\"file.rs.\">"));
    }

    #[test]
    fn skips_directory() {
        let ws = fresh_workspace();
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        let (out, attached) = expand_file_mentions("look at @sub", &ws, u64::MAX);
        assert!(attached.is_empty());
        // Directory left as-is so the model can fall back to read_file/list_dir.
        assert_eq!(out, "look at @sub");
    }

    #[test]
    fn skips_oversized_file() {
        let ws = fresh_workspace();
        // max_bytes = 3, file is 10 bytes → skipped, left as-is.
        std::fs::write(ws.join("big.txt"), "0123456789").unwrap();
        let (out, attached) = expand_file_mentions("@big.txt", &ws, 3);
        assert!(attached.is_empty());
        assert_eq!(out, "@big.txt");
    }

    #[test]
    fn multiple_mentions_inlined() {
        let ws = fresh_workspace();
        std::fs::write(ws.join("a.rs"), "a\n").unwrap();
        std::fs::write(ws.join("b.go"), "b\n").unwrap();
        let (out, attached) = expand_file_mentions("@a.rs and @b.go", &ws, u64::MAX);
        assert_eq!(attached, vec!["a.rs".to_string(), "b.go".to_string()]);
        assert!(out.contains("<file path=\"a.rs\">"));
        assert!(out.contains("<file path=\"b.go\">"));
    }

    #[cfg(unix)]
    #[test]
    fn absolute_path_inlined() {
        // Absolute paths are honored (core has unrestricted FS access), even
        // though they lie outside the workspace.
        let dir = std::env::temp_dir().join(format!(
            "catalyst-code-abs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abs.txt");
        std::fs::write(&f, "abs content\n").unwrap();
        let ws = fresh_workspace();
        let mention = format!("@{}", f.display());
        let (out, attached) = expand_file_mentions(&mention, &ws, u64::MAX);
        assert_eq!(attached.len(), 1);
        assert!(out.contains("abs content"));
        assert!(out.contains("<file path=\""));
    }
}
