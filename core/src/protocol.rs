// Wire protocol: newline-delimited JSON over stdio.
// TUI -> Core commands (stdin), Core -> TUI events (stdout).
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub reasoning: bool,
    pub context_window: u32,
    pub max_tokens: u32,
    /// Reasoning/thinking levels the model advertises (e.g. ["low","medium","high"]).
    /// Populated from /models/info when the endpoint provides them; empty means the
    /// model declares no specific levels and any effort string is passed through.
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    /// Whether the model accepts image (vision) inputs. Populated from
    /// /models/info `capabilities.supports_vision` (true/false/"via-handoff";
    /// only boolean true counts as native client-side vision); false otherwise.
    /// Drives the vision-handoff (pre_turn plugin) routing.
    #[serde(default)]
    pub vision: bool,
    /// The provider name that owns this model (e.g. "openai", "gemini",
    /// "anthropic"). Populated by the aggregation layer so a turn can be routed
    /// to the correct endpoint per-model when multiple providers are logged in.
    /// Empty for legacy single-provider models (routes to the active provider).
    #[serde(default)]
    pub provider: String,
}

/// Commands read from stdin.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Command {
    #[serde(rename = "init")]
    Init,
    #[serde(rename = "set_key")]
    SetKey {
        api_key: String,
        /// Optional provider name this key applies to. When omitted, the key
        /// applies to the currently active provider (backward-compatible with
        /// the pre-provider single-endpoint flow).
        #[serde(default)]
        provider: Option<String>,
    },
    /// Switch the active model provider at runtime. Re-resolves base URL / key /
    /// wire protocol, re-discovers models, and emits `provider_changed` + a
    /// fresh `models` event. Unknown names are ignored (stays on current).
    #[serde(rename = "set_provider")]
    SetProvider { name: String },
    /// List the built-in first-party provider presets (OpenAI/Codex, Gemini,
    /// Anthropic). Emits a `provider_presets` event so the TUI/web can render a
    /// one-click "login" picker. Each entry carries whether a key is already
    /// available (env var set) and whether the provider is logged in.
    #[serde(rename = "list_provider_presets")]
    ListProviderPresets,
    /// Log in to a first-party provider (`openai` | `gemini` | `anthropic`):
    /// create the provider config, set its API key, persist, and re-aggregate
    /// models so the provider's models appear in `/models` alongside any others
    /// already logged in. An optional `api_key` overrides the preset's standard
    /// env var; when omitted the key is read from the env var. Multiple
    /// providers can be logged in at once; each turn routes to the selected
    /// model's provider.
    #[serde(rename = "login")]
    Login {
        preset: String,
        #[serde(default)]
        api_key: Option<String>,
    },
    /// Log out of a provider: drop its runtime key, remove it from the
    /// configured providers, persist the change, and re-aggregate models so its
    /// models disappear from `/models`. No-op (error event) when not logged in.
    #[serde(rename = "logout")]
    Logout { provider: String },
    /// Perform the interactive OAuth subscription login for a preset (`gemini` |
    /// `anthropic`) — no official CLI needed. The core runs the OAuth flow
    /// (Google device-code / Claude authorize+PKCE+loopback), emitting
    /// `oauth_prompt` events with the URL/code to visit, stores the resulting
    /// token, then re-aggregates models. Codex (Responses API) is not supported
    /// here yet. Use `login` for an API-key login instead.
    #[serde(rename = "login_oauth")]
    LoginOauth { preset: String },
    /// Complete a pending no-browser (manual-code) OAuth login by submitting the
    /// authorization code the user pasted after visiting the URL from a prior
    /// `oauth_prompt` (the SSH/headless Google flow). The core holds the PKCE
    /// verifier from that prompt; this exchanges the code, stores the token,
    /// and re-aggregates models. No-op (error event) when no login is pending.
    #[serde(rename = "oauth_code")]
    OauthCode { code: String },
    #[serde(rename = "send")]
    Send {
        prompt: String,
        model: String,
        #[serde(default)]
        reasoning_effort: Option<String>,
        /// Optional images: each is a data URL (data:image/png;base64,...) or an
        /// absolute file path to attach. Built into a multimodal user message.
        #[serde(default)]
        images: Option<Vec<String>>,
    },
    /// Steer an in-flight turn: interrupt the running turn and redirect it with
    /// `prompt`. If no turn is running, behaves like `send`. Carries model +
    /// reasoning_effort so the steered turn uses the same leader as the run it
    /// interrupted (the TUI always sends a model from its discovered list).
    #[serde(rename = "steer")]
    Steer {
        prompt: String,
        model: String,
        #[serde(default)]
        reasoning_effort: Option<String>,
    },
    #[serde(rename = "abort")]
    Abort,
    /// Drop a queued follow-up/steer prompt WITHOUT aborting the running
    /// turn. Lets the TUI's Esc cancel just the queued message and leave the
    /// in-flight chat running (vs Abort which cancels both).
    #[serde(rename = "clear_queue")]
    ClearQueue,
    #[serde(rename = "reset")]
    Reset,
    /// Clear the in-memory conversation but keep the session file (vs Reset which wipes both).
    #[serde(rename = "clear")]
    Clear,
    /// Drop the last turn (user prompt + its assistant reply + tool calls/results).
    #[serde(rename = "undo")]
    Undo,
    /// Force a context compaction now (regardless of the 70% threshold).
    #[serde(rename = "compact")]
    Compact,
    /// List available session files (returns a `sessions` event).
    #[serde(rename = "list_sessions")]
    ListSessions,
    /// Load a specific session file (replaces the current conversation).
    #[serde(rename = "load_session")]
    LoadSession { path: String },
    /// Start a fresh session file in the same project directory. The current
    /// file is left intact so sessions accumulate per project. An optional
    /// `path` (a filename) overrides the auto-generated name.
    #[serde(rename = "new_session")]
    NewSession {
        #[serde(default)]
        path: Option<String>,
    },
    /// Request a stats summary (returns a `stats` event).
    #[serde(rename = "stats")]
    Stats,
    /// Approve a pending tool call. decision: "yes" | "no" | "always".
    /// "always" upgrades the session approval mode so subsequent same-tool calls skip the gate.
    #[serde(rename = "approve")]
    Approve {
        request_id: String,
        decision: String,
    },
    /// Change the approval mode at runtime: "never" | "destructive" | "always".
    #[serde(rename = "set_approval")]
    SetApproval { mode: String },
    /// Change a runtime config knob at runtime. Recognized keys:
    ///   bash_timeout_secs (u64).
    /// Values are coerced from the JSON type (string or number).
    #[serde(rename = "set_config")]
    SetConfig { key: String, value: Value },
    /// Plugin lifecycle commands.
    #[serde(rename = "install_plugin")]
    InstallPlugin { path: String },
    #[serde(rename = "remove_plugin")]
    RemovePlugin { name: String },
    #[serde(rename = "enable_plugin")]
    EnablePlugin { name: String },
    #[serde(rename = "disable_plugin")]
    DisablePlugin { name: String },
    #[serde(rename = "list_plugins")]
    ListPlugins,
    /// Ask core to re-inject memories into the system prompt (called after saving a memory).
    #[serde(rename = "refresh_memory")]
    RefreshMemory,
    /// Save a memory note (persisted across sessions). Core generates a name,
    /// saves it, and refreshes the system-prompt injection. Emits a
    /// `memory_saved` event with the new id. `scope` is "workspace" (default)
    /// or "global" (cross-codebase: user identity, tech-stack preferences, etc).
    #[serde(rename = "save_memory")]
    SaveMemory {
        text: String,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        scope: Option<String>,
    },
    /// List saved memories (both global and workspace scopes). Emits a
    /// `memory_list` event.
    #[serde(rename = "list_memory")]
    ListMemory,
    /// Forget (delete) a memory by its id (the slug or the memory name).
    /// Searches both scopes when `scope` is omitted. Emits a `memory_saved`
    /// event describing the outcome.
    #[serde(rename = "forget_memory")]
    ForgetMemory {
        id: String,
        #[serde(default)]
        scope: Option<String>,
    },
    /// Reply to a subagent's contact_supervisor need_decision ask.
    /// The TUI surfaces an `intercom_message` event and the user (acting as
    /// the orchestrator) replies with this command; the awaiting subagent
    /// wakes and continues.
    #[serde(rename = "intercom_reply")]
    IntercomReply { request_id: String, reply: String },
    /// Reply to a pending `ask_request` (the `ask` tool). `answers` is either a
    /// JSON object mapping each question id → its answer string, or JSON null
    /// to indicate the user skipped the questions. The awaiting tool call
    /// resumes and the formatted answers are returned to the model.
    #[serde(rename = "ask_reply")]
    AskReply { request_id: String, answers: Value },
    /// Get the current vision-handoff configuration (curated vision-capable
    /// models + preferred target). Emits a `vision_config` event.
    #[serde(rename = "get_vision_config")]
    GetVisionConfig,
    /// Set the vision-handoff configuration and persist it to
    /// .catalyst-code/vision.json. `vision_model` is the preferred handoff
    /// target; an empty string / null means "pick dynamically". Emits a
    /// `vision_config` event with the new state.
    #[serde(rename = "set_vision_config")]
    SetVisionConfig {
        #[serde(default)]
        vision_models: Vec<String>,
        #[serde(default)]
        vision_model: Option<String>,
    },
    /// List discoverable skills (project then user scope). Emits a `skills`
    /// event with each skill's name, description, and location — used by the
    /// TUI/web to populate the `/skill:<name>` autocomplete.
    #[serde(rename = "list_skills")]
    ListSkills,
    /// Invoke a skill by name: the core reads the matching SKILL.md (resolving
    /// project > user scope, bypassing the read_file path restriction so global
    /// skills under ~/.catalyst-code/skills work too), builds a prompt that
    /// instructs the model to apply it, and runs a normal assistant turn.
    /// `task` is an optional follow-up appended to the skill instructions.
    #[serde(rename = "apply_skill")]
    ApplySkill {
        name: String,
        #[serde(default)]
        task: Option<String>,
        model: String,
        #[serde(default)]
        reasoning_effort: Option<String>,
    },
}

/// Events written to stdout. Constructed with serde_json::json! and emitted via `emit`.
#[derive(Serialize, Debug)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

impl Event {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            data: serde_json::Map::new(),
        }
    }
    pub fn with(mut self, k: &str, v: serde_json::Value) -> Self {
        self.data.insert(k.to_string(), v);
        self
    }
}

/// Emit one event as a single line of JSON to stdout. Thread-safe via stdout lock.
pub fn emit(ev: &Event) {
    let mut line = serde_json::to_string(ev).unwrap_or_else(|_| "{}".into());
    line.push('\n');
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut h = stdout.lock();
    let _ = h.write_all(line.as_bytes());
    let _ = h.flush();
}
