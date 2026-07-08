// Config: CLI flags + env vars + config files. No clap, no toml — hand-rolled.
// Precedence: CLI > env > settings.local.json > settings.json
//   > ~/.config/settings.json > managed-settings.json > managed-settings.d/*.json
// Arrays concatenate+deduplicate; objects deep merge; null means delete.
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Approval {
    Never,       // auto-approve everything (trust the model fully)
    Destructive, // ask only for bash + write_file + edit (default)
    Always,      // ask for every tool call
}

impl Approval {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "never" | "off" | "none" | "auto" => Approval::Never,
            "always" | "all" | "y" => Approval::Always,
            _ => Approval::Destructive,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Approval::Never => "never",
            Approval::Destructive => "destructive",
            Approval::Always => "always",
        }
    }
}

/// Permission rule: per-tool, per-content matching with allow/deny/ask behavior.
#[derive(Clone, Debug)]
pub struct PermissionRule {
    pub tool_name: String,
    pub rule_content: String,
    pub behavior: PermissionBehavior,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

impl PermissionBehavior {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "allow" | "yes" | "true" => PermissionBehavior::Allow,
            "deny" | "no" | "false" => PermissionBehavior::Deny,
            _ => PermissionBehavior::Ask,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionBehavior::Allow => "allow",
            PermissionBehavior::Deny => "deny",
            PermissionBehavior::Ask => "ask",
        }
    }
}

/// Parse a rule string like "Bash(npm test)" or "Edit(//src/**)" into PermissionRule.
/// The format is: ToolName(ruleContent).
pub fn parse_permission_rule(s: &str, behavior: PermissionBehavior) -> Option<PermissionRule> {
    let s = s.trim();
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let tool_name = s[..open].to_string();
    let rule_content = s[open + 1..close].to_string();
    if tool_name.is_empty() {
        return None;
    }
    Some(PermissionRule {
        tool_name,
        rule_content,
        behavior,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sandbox {
    None,     // no sandboxing (default; denylist tripwire only)
    Firejail, // wrap bash in `firejail` with a writable-workspace profile
}

impl Sandbox {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "firejail" | "fj" => Sandbox::Firejail,
            _ => Sandbox::None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Sandbox::None => "none",
            Sandbox::Firejail => "firejail",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: String,
    pub workspace: PathBuf,
    pub approval: Approval,
    pub bash_timeout_secs: u64,
    pub diag_timeout_secs: u64, // wall-clock timeout for the diagnostics tool (cargo check / tsc / go build)
    /// Hard cap for a per-call bash timeout override (the `timeout` arg on the
    /// bash tool). A model can request more time for a slow build/test, but it
    /// can't escalate past this ceiling (default 600s) without changing config.
    pub max_bash_timeout_secs: u64,
    /// Allowlist of host glob patterns the `fetch` tool may contact (e.g.
    /// `["*.rust-lang.org", "docs.rs", "crates.io"]`). Empty = allow any http(s)
    /// host (the tool is still useful out of the box, and works under
    /// `--no-network` where bash curl is dead — PROVIDED you populate this
    /// allowlist, since `--no-network` + empty allowlist denies fetch to avoid a
    /// surprise bypass of the egress block). Populate it to restrict egress.
    pub fetch_allowlist: Vec<String>,
    /// Wall-clock timeout for the `fetch` tool (default 20s).
    pub fetch_timeout_secs: u64,
    /// Max response body the `fetch` tool returns (default 256 KiB).
    pub fetch_max_bytes: usize,
    pub bash_deny: Vec<String>,
    pub max_read_bytes: u64,
    pub max_read_lines: usize,
    pub context_compact_at: f32, // fraction of context_window that triggers compaction
    pub context_digest_at: f32, // fraction of context_window that triggers stale-tool-result digesting (sub-threshold reclaim; 0 disables)
    pub auto_compact: bool, // automatically compact when context approaches the limit (threshold + idle); manual /compact always works regardless
    pub debug_log: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub default_model: Option<String>,
    // --- production knobs (items 3,4,7) ---
    pub sandbox: Sandbox,           // --sandbox firejail wraps bash
    pub no_network: bool,           // --no-network: unshare -n on bash
    pub idle_timeout_secs: u64,     // per-chunk SSE idle timeout
    pub max_session_tokens: u64,    // hard session token budget (0 = unlimited)
    pub summarize_on_compact: bool, // use a model call to summarize dropped turns
    pub compact_instructions: Option<String>, // optional guidance woven into the summarize prompt (e.g. "Focus on code samples and API usage"); /compact <instructions> overrides per-call
    pub rolling_state: bool,        // inject a transient tail work-state summary (KV-cache-aware)
    /// Auto-reflect: on a non-trivial turn (≥ `auto_reflect_min_tool_calls` tool
    /// calls), inject a reflection continuation before `finish` exits so durable
    /// facts get persisted (memory) and recurring patterns get written as skills
    /// — without relying on the model remembering to reflect. The deterministic
    /// seam SELF_LEARNING.md §11 deferred. Skips reflect/index turns and trivial
    /// turns. Default on.
    pub auto_reflect: bool,
    /// Minimum non-trivial tool-call count for auto-reflect to fire. 1 = any
    /// real work. 0 is treated as 1 (a no-work turn should never reflect).
    pub auto_reflect_min_tool_calls: u32,
    pub allow_vision: bool, // accept image_url content in send
    // --- permission rules (item 1) ---
    pub allow_rules: Vec<PermissionRule>,
    pub deny_rules: Vec<PermissionRule>,
    pub ask_rules: Vec<PermissionRule>,
    // --- plugin system (centerpiece) ---
    pub plugin_dir: PathBuf,           // directory scanned for plugins
    pub plugins_disabled: Vec<String>, // plugin names that are explicitly disabled
    pub trust_project_plugins: bool, // allow loading project-scoped plugins (.catalyst-code/plugins). Default false for safety; set via env/CLI only — never a project config file, which an untrusted repo could use to self-enable its own hooks.
    // --- regex denylist upgrade (quick win) ---
    pub bash_deny_regex: Vec<String>, // regex patterns that block bash commands
    pub bash_deny_regex_compiled: Vec<regex::Regex>, // pre-compiled at startup
    // --- subagent system (pi-subagents port) ---
    pub subagents: SubagentConfig,
    // --- custom providers (openai/anthropic endpoints) ---
    /// Named, configurable model providers. Empty = legacy single-endpoint mode
    /// (uses `base_url` + the runtime key set via `set_key`).
    pub providers: Vec<ProviderConfig>,
    /// Name of the active provider. None = use the first configured provider, or
    /// the legacy default when none are configured.
    pub active_provider: Option<String>,
    /// Per-provider API keys persisted by the TUI (settings.json `provider_keys`
    /// + the legacy `api_key` under "default"). Seeded into `State::api_keys` at
    ///   startup so they override config/env keys (runtime keys win in provider
    ///   resolution) and survive restarts — this is what makes `/key` sticky.
    pub persisted_keys: std::collections::HashMap<String, String>,
}

/// Intercom bridge mode: controls whether subagents get a coordination channel
/// back to the orchestrator and to each other.
#[derive(Clone, Debug, PartialEq)]
pub enum IntercomBridgeMode {
    Off,      // no intercom tools injected into subagents
    ForkOnly, // only for forked-context runs
    Always,   // always inject (default)
}

impl IntercomBridgeMode {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "off" | "none" => IntercomBridgeMode::Off,
            "fork-only" | "fork_only" | "fork" => IntercomBridgeMode::ForkOnly,
            _ => IntercomBridgeMode::Always,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            IntercomBridgeMode::Off => "off",
            IntercomBridgeMode::ForkOnly => "fork-only",
            IntercomBridgeMode::Always => "always",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SubagentConfig {
    /// Max nesting depth for subagent delegation (main → sub → sub-sub).
    /// 0 blocks all subagents; default 2.
    pub max_depth: u32,
    /// Whether subagents receive intercom coordination tools + instructions.
    pub intercom_bridge_mode: IntercomBridgeMode,
    /// Max tasks in a top-level parallel run.
    pub parallel_max_tasks: u32,
    /// Default concurrency for parallel runs.
    pub parallel_concurrency: u32,
    /// Top-level calls use background execution when async is not explicitly set.
    pub async_by_default: bool,
    /// Hide builtin agents from discovery.
    pub disable_builtins: bool,
    /// Per-builtin agent overrides keyed by agent name.
    pub agent_overrides: std::collections::HashMap<String, AgentOverride>,
}

#[derive(Clone, Debug, Default)]
pub struct AgentOverride {
    pub model: Option<String>,
    pub fallback_models: Vec<String>,
    pub thinking: Option<String>,
    pub disabled: bool,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            intercom_bridge_mode: IntercomBridgeMode::Always,
            parallel_max_tasks: 8,
            parallel_concurrency: 4,
            async_by_default: false,
            disable_builtins: false,
            agent_overrides: std::collections::HashMap::new(),
        }
    }
}

/// A model provider: a named OpenAI- or Anthropic-compatible endpoint with
/// its own base URL, auth, and wire protocol. Defined in config (JSON/env);
/// switched at runtime via the `set_provider` command.
///
/// The harness keeps the *internal* conversation in OpenAI chat-completions
/// shape (role:"tool", assistant `tool_calls`, ...) because every other layer
/// (compaction, sanitization, subagents, session persistence) understands
/// that shape. The provider abstraction only translates at the HTTP boundary:
/// `kind` decides whether requests/responses are OpenAI-shaped or translated
/// to/from the Anthropic Messages API. This means adding a provider never
/// touches the rest of the harness.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum ProviderKind {
    #[default]
    OpenAI,
    Anthropic,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAI,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::OpenAI => "openai",
            ProviderKind::Anthropic => "anthropic",
        }
    }
    pub fn is_openai(&self) -> bool {
        matches!(self, ProviderKind::OpenAI)
    }
    pub fn is_anthropic(&self) -> bool {
        matches!(self, ProviderKind::Anthropic)
    }
}
impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A configured provider as it appears in the config file/env (no resolved
/// runtime key). `api_key` is a literal (user-owned config only, never a
/// project-local file); `api_key_env` names an env var to read instead.
#[derive(Clone, Debug, Default)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    /// Literal API key stored in the (user-owned, 0600) config file. Optional.
    pub api_key: Option<String>,
    /// Name of an env var holding the API key (resolved at request time).
    pub api_key_env: Option<String>,
    /// Extra HTTP headers appended to every request (e.g. `HTTP-Referer`).
    pub headers: Vec<(String, String)>,
}

/// A provider fully resolved for an API call: kind, base URL, the effective API
/// key (runtime override -> config literal -> config env var -> global env),
/// and extra headers. This is what `stream_turn` / `discover_models` /
/// `summarize` consume — it carries everything provider-specific so those
/// functions stop depending on `cfg.base_url` + a bare `api_key` string.
#[derive(Clone, Debug)]
pub struct ResolvedProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
    /// When true (Anthropic OAuth), the streaming/discovery path uses
    /// `Authorization: Bearer <api_key>` + the `anthropic-beta: oauth-2025-04-20`
    /// header instead of `x-api-key`. Set by `oauth::enrich_oauth` when a Claude
    /// subscription token is used. (Gemini OAuth needs no flag: it reuses the
    /// OpenAI Bearer path unchanged.)
    pub oauth: bool,
}

impl ResolvedProvider {
    /// The legacy/default provider when none are configured: OpenAI-shaped,
    /// `cfg.base_url` for the URL, key resolved from `runtime_keys["default"]`
    /// then the `UMANS_API_KEY` env var (backward-compatible with the pre-provider
    /// single-endpoint setup).
    pub fn legacy_default(
        cfg: &Config,
        runtime_keys: &std::collections::HashMap<String, String>,
    ) -> Self {
        let api_key = runtime_keys
            .get("default")
            .cloned()
            .or_else(|| std::env::var("UMANS_API_KEY").ok())
            .filter(|s| !s.is_empty());
        ResolvedProvider {
            name: "default".to_string(),
            kind: ProviderKind::OpenAI,
            base_url: cfg.base_url.clone(),
            api_key,
            headers: Vec::new(),
            oauth: false,
        }
    }
}

/// A built-in first-party provider template: a known endpoint + the standard
/// API-key env var for that vendor, so a user can add the provider with a
/// single action (`add_provider`) instead of hand-editing JSON. Presets cover
/// the major vendors. The harness always keeps the conversation in OpenAI
/// chat-completions shape internally; a preset's `kind` only decides the wire
/// translation at the HTTP boundary (Gemini exposes an OpenAI-compatible
/// endpoint, so it maps to `OpenAI`).
#[derive(Clone, Debug)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ProviderKind,
    pub base_url: &'static str,
    /// Primary env var holding the API key (e.g. `OPENAI_API_KEY`).
    pub api_key_env: &'static str,
    /// Alternate env vars checked in order if the primary is unset
    /// (e.g. Gemini accepts `GOOGLE_API_KEY` too).
    pub alt_envs: &'static [&'static str],
    pub description: &'static str,
}

/// The first-party provider presets. Order is the order shown in pickers.
/// Umans (the default/original provider) is listed first.
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "umans",
        label: "Umans (GLM-5.2)",
        kind: ProviderKind::OpenAI,
        // The default Umans endpoint. is_umans() matches `umans.ai` as a parent
        // domain, so the GLM-specific wire logic (reasoning_effort,
        // /models/info discovery) still applies to this preset's turns.
        base_url: "https://api.code.umans.ai/v1",
        api_key_env: "UMANS_API_KEY",
        alt_envs: &[],
        description: "Umans — GLM-5.2, the default provider. Uses your UMANS_API_KEY (https://app.umans.ai/billing → API Keys).",
    },
    ProviderPreset {
        id: "openai",
        label: "OpenAI (Codex)",
        kind: ProviderKind::OpenAI,
        base_url: "https://chatgpt.com/backend-api/codex",
        api_key_env: "OPENAI_API_KEY",
        alt_envs: &[],
        description: "OpenAI Codex via ChatGPT subscription OAuth (or OPENAI_API_KEY for API-key mode)."
    },
    ProviderPreset {
        id: "gemini",
        label: "Google Gemini",
        kind: ProviderKind::OpenAI,
        // Gemini's OpenAI-compatible shim: {base}/chat/completions + {base}/models.
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: "GEMINI_API_KEY",
        alt_envs: &["GOOGLE_API_KEY"],
        description: "Google Gemini via its OpenAI-compatible endpoint — Gemini 2.5 Pro/Flash. Uses GEMINI_API_KEY (or GOOGLE_API_KEY), or `/login` for an OAuth subscription login (matches the `gemini` CLI exactly — supports both regular Google accounts and Google Cloud/ADC/service-account credentials).",
    },
    ProviderPreset {
        id: "anthropic",
        label: "Anthropic Claude",
        kind: ProviderKind::Anthropic,
        base_url: "https://api.anthropic.com/v1",
        api_key_env: "ANTHROPIC_API_KEY",
        alt_envs: &[],
        description: "Anthropic Claude via API key (ANTHROPIC_API_KEY) or Claude subscription OAuth with /login (works locally and over SSH/headless).",
    },
    ProviderPreset {
        id: "opencode-go",
        label: "OpenCode Go",
        kind: ProviderKind::OpenAI,
        // OpenCode Go is one subscription/key that serves some models via an
        // OpenAI-compatible `/v1/chat/completions` endpoint and others via an
        // Anthropic `/v1/messages` endpoint. preset_provider_configs()
        // expands this preset into TWO provider configs (opencode-go +
        // opencode-go-anthropic) sharing this base URL + key, so each model
        // routes to its correct wire protocol. See provider::is_opencode_go.
        base_url: "https://opencode.ai/zen/go/v1",
        api_key_env: "OPENCODE_GO_API_KEY",
        alt_envs: &[],
        description: "OpenCode Go — low-cost subscription for popular open coding models (GLM, Kimi, DeepSeek, MiMo, MiniMax, Qwen). One API key; models route to the OpenAI-compatible or Anthropic endpoint automatically. Uses your OPENCODE_GO_API_KEY.",
    },
];

/// Look up a first-party preset by id.
pub fn find_preset(id: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.id == id)
}

impl ProviderPreset {
    /// Resolve an API key for this preset from its env vars (primary first,
    /// then alternates). None when none are set.
    pub fn env_key(&self) -> Option<String> {
        std::env::var(self.api_key_env)
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.alt_envs
                    .iter()
                    .find_map(|e| std::env::var(e).ok().filter(|s| !s.is_empty()))
            })
    }

    /// The env var name that actually held a key, or the primary env var when
    /// none are set (so a future `export` Just Works without re-adding).
    pub fn resolved_env(&self) -> &'static str {
        if std::env::var(self.api_key_env)
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        {
            return self.api_key_env;
        }
        for e in self.alt_envs {
            if std::env::var(e).ok().filter(|s| !s.is_empty()).is_some() {
                return e;
            }
        }
        self.api_key_env
    }

    /// Build a `ProviderConfig` from this preset. When `api_key` is given it is
    /// stored as a literal (user entered it); otherwise the key is read from the
    /// preset's env var and only the env-var NAME is persisted (the secret stays
    /// in the environment, never copied into the config file).
    pub fn to_provider_config(&self, api_key: Option<String>) -> ProviderConfig {
        let (api_key, api_key_env) = match api_key {
            Some(k) => (Some(k), None),
            None => (None, Some(self.resolved_env().to_string())),
        };
        ProviderConfig {
            name: self.id.to_string(),
            kind: self.kind.clone(),
            base_url: self.base_url.to_string(),
            api_key,
            api_key_env,
            headers: Vec::new(),
        }
    }
}

/// The provider config(s) to create when logging in to a preset. Most presets
/// map to a single config; OpenCode Go maps to TWO — one OpenAI-kind (for its
/// `/v1/chat/completions` models: GLM, Kimi, DeepSeek, MiMo) and one
/// Anthropic-kind (for its `/v1/messages` models: MiniMax, Qwen) — because
/// OpenCode Go serves models over both wire protocols under one API key, and
/// the harness's per-provider `kind` decides the wire translation at the HTTP
/// boundary. Both configs share the preset's base URL + key. The first config
/// is the "primary" (used as the active provider + preset identity).
pub fn preset_provider_configs(p: &ProviderPreset, api_key: Option<String>) -> Vec<ProviderConfig> {
    if p.id == "opencode-go" {
        let (api_key_lit, api_key_env) = match api_key {
            Some(k) => (Some(k), None),
            None => (None, Some(p.resolved_env().to_string())),
        };
        let make = |name: &str, kind: ProviderKind| ProviderConfig {
            name: name.to_string(),
            kind,
            base_url: p.base_url.to_string(),
            api_key: api_key_lit.clone(),
            api_key_env: api_key_env.clone(),
            headers: Vec::new(),
        };
        vec![
            make("opencode-go", ProviderKind::OpenAI),
            make("opencode-go-anthropic", ProviderKind::Anthropic),
        ]
    } else {
        vec![p.to_provider_config(api_key)]
    }
}

/// Serialize a `ProviderConfig` back to JSON for persistence. Only writes
/// non-default fields so the file stays readable.
pub fn provider_to_json(p: &ProviderConfig) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("name".into(), json!(p.name));
    o.insert("kind".into(), json!(p.kind.as_str()));
    o.insert("base_url".into(), json!(p.base_url));
    if let Some(k) = &p.api_key {
        o.insert("api_key".into(), json!(k));
    }
    if let Some(e) = &p.api_key_env {
        o.insert("api_key_env".into(), json!(e));
    }
    if !p.headers.is_empty() {
        let h: serde_json::Map<String, Value> = p
            .headers
            .iter()
            .cloned()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        o.insert("headers".into(), Value::Object(h));
    }
    Value::Object(o)
}

/// Path of the core-owned config file (`~/.config/catalyst-code/config.json`),
/// where first-party providers added at runtime are persisted. The TUI does
/// NOT write this file (it owns `settings.json`), so there is no clobber.
pub fn user_config_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".config/catalyst-code/config.json"))
}

/// Persist `providers` (+ optional active provider) into the core-owned config
/// file, merging with any existing JSON so other keys are preserved. Atomic
/// (temp + rename) with 0600 perms. Best-effort: returns an io error on failure.
pub fn save_providers_config(
    providers: &[ProviderConfig],
    active: Option<&str>,
) -> std::io::Result<()> {
    let path = user_config_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Read existing config.json (if any) and merge so other keys survive.
    let mut root: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if root.as_object().is_none() {
        root = json!({});
    }
    let arr: Vec<Value> = providers.iter().map(provider_to_json).collect();
    root["providers"] = json!(arr);
    if let Some(a) = active {
        root["activeProvider"] = json!(a);
    }
    let data = serde_json::to_string_pretty(&root).unwrap_or_default();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Auto-log-in to every first-party preset whose API key is already available
/// in the environment (`UMANS_API_KEY`, `OPENAI_API_KEY`, ...). Gemini/Claude
/// may also reuse their CLI OAuth stores; OpenAI/Codex deliberately does not —
/// use this app's OAuth login so account selection is explicit. For each such
/// preset that isn't already explicitly configured, add a provider entry. API-key
/// providers store the env-var NAME only (the secret stays in the environment);
/// OAuth providers store no key at all (the token is resolved + refreshed at
/// turn time by `oauth::enrich_oauth`). Already-configured providers are left
/// untouched. Returns the names added. NOT persisted — presence (env var / cred
/// file) drives it every launch.
pub fn auto_login_env_presets(cfg: &mut Config) -> Vec<String> {
    let mut added = Vec::new();
    for p in PROVIDER_PRESETS {
        if cfg.find_provider(p.id).is_some() {
            continue; // already configured (explicit login or prior session) — leave it.
        }
        // Auth available without a manual step?
        let has_env_key = p.env_key().is_some();
        let has_oauth = preset_has_oauth_creds(p);
        if has_env_key || has_oauth {
            // Most presets → one config; OpenCode Go → two (OpenAI-kind +
            // Anthropic-kind) sharing the base URL + key.
            let configs = preset_provider_configs(p, None);
            for mut pc in configs {
                // For Umans, honor a custom cfg.base_url (e.g. UMANS_BASE_URL)
                // instead of the preset's default URL, so a custom proxy isn't
                // silently overwritten.
                if p.id == "umans" && !cfg.base_url.is_empty() {
                    pc.base_url = cfg.base_url.clone();
                }
                if cfg.find_provider(&pc.name).is_none() {
                    cfg.providers.push(pc);
                }
            }
            added.push(p.id.to_string());
        }
    }
    // If no active provider is set, prefer Umans (the default), else the first.
    if cfg.active_provider.is_none() && !cfg.providers.is_empty() {
        if cfg.find_provider("umans").is_some() {
            cfg.active_provider = Some("umans".to_string());
        } else {
            cfg.active_provider = Some(cfg.providers[0].name.clone());
        }
    }
    added
}

/// True when a preset's reusable OAuth credentials exist (cheap sync file
/// check). OpenAI/Codex is intentionally excluded: no Codex CLI auto-detect.
fn preset_has_oauth_creds(p: &ProviderPreset) -> bool {
    match p.id {
        "gemini" => crate::oauth::has_google_creds(),
        "anthropic" => crate::oauth::has_claude_creds(),
        _ => false,
    }
}

impl Config {
    pub fn find_provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Names of configured providers in declaration order.
    pub fn provider_names(&self) -> Vec<String> {
        self.providers.iter().map(|p| p.name.clone()).collect()
    }

    /// Resolve the active provider into a `ResolvedProvider`, combining the
    /// config definition with per-provider runtime keys (set via `set_key`).
    ///
    /// Resolution order for the active provider:
    ///   1. `active_provider` (if it names a configured provider)
    ///   2. the first configured provider (if any)
    ///   3. the legacy default (OpenAI, `cfg.base_url`) when none are configured
    ///
    /// API key for the resolved provider:
    ///   runtime_keys[name] -> provider.api_key -> provider.api_key_env (env) ->
    ///   (legacy default only) `UMANS_API_KEY` env. Empty values are dropped.
    pub fn resolve_provider(
        &self,
        runtime_keys: &std::collections::HashMap<String, String>,
    ) -> ResolvedProvider {
        self.resolve_provider_with(runtime_keys, None)
    }

    /// Like `resolve_provider` but with an optional runtime override of the
    /// active provider name (set via `set_provider`). The override wins over
    /// the config's `active_provider`; an unknown override falls back to the
    /// first configured provider, then the legacy default.
    pub fn resolve_provider_with(
        &self,
        runtime_keys: &std::collections::HashMap<String, String>,
        active_override: Option<&str>,
    ) -> ResolvedProvider {
        let pick = match active_override.or(self.active_provider.as_deref()) {
            Some(name) => self
                .find_provider(name)
                .cloned()
                .or_else(|| self.providers.first().cloned()),
            None => self.providers.first().cloned(),
        };
        let Some(p) = pick else {
            return ResolvedProvider::legacy_default(self, runtime_keys);
        };
        let api_key = runtime_keys
            .get(&p.name)
            .cloned()
            .or_else(|| p.api_key.clone())
            .or_else(|| p.api_key_env.as_ref().and_then(|v| std::env::var(v).ok()))
            .filter(|s| !s.is_empty());
        ResolvedProvider {
            name: p.name.clone(),
            kind: p.kind,
            base_url: p.base_url.clone(),
            api_key,
            headers: p.headers.clone(),
            oauth: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "https://api.code.umans.ai/v1".into(),
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            approval: Approval::Destructive,
            bash_timeout_secs: 30,
            diag_timeout_secs: 120, // builds/checks can run longer than bash; bounded so a hung checker can't wedge the turn
            max_bash_timeout_secs: 600, // a model may ask up to 10 min for one command, but no more
            fetch_allowlist: Vec::new(), // empty = allow any http(s) host; populate to restrict
            fetch_timeout_secs: 20,
            fetch_max_bytes: 262_144, // 256 KiB — enough for a doc page, bounded so a giant response can't OOM
            bash_deny: vec![
                // ponytail: minimal denylist of obviously catastrophic commands.
                // Not a security boundary (use a sandbox for that); a tripwire.
                "rm -rf /".into(),
                "rm -rf ~".into(),
                "mkfs".into(),
                "dd if=/dev/zero of=/dev/sd".into(),
                ":(){:|:&};:".into(),
            ],
            max_read_bytes: 5_242_880, // 5 MiB (was 1 MiB; real files exceed 1MB)
            max_read_lines: 10_000,    // was 2000; pagination covers the rest
            context_compact_at: 0.90,
            context_digest_at: 0.40,
            debug_log: None,
            session_file: None,
            default_model: None,
            sandbox: Sandbox::None,
            no_network: false,
            idle_timeout_secs: 120, // some reasoning models think >60s before first token
            max_session_tokens: 0,
            summarize_on_compact: true,
            compact_instructions: None,
            auto_compact: true,
            rolling_state: true,
            auto_reflect: true,
            auto_reflect_min_tool_calls: 1,
            allow_vision: true,
            allow_rules: Vec::new(),
            deny_rules: Vec::new(),
            ask_rules: Vec::new(),
            plugin_dir: PathBuf::from(".catalyst-code/plugins"),
            plugins_disabled: Vec::new(),
            trust_project_plugins: false, // secure default: don't auto-run repo-shipped plugins
            bash_deny_regex: Vec::new(),
            bash_deny_regex_compiled: Vec::new(),
            subagents: SubagentConfig::default(),
            providers: Vec::new(),
            active_provider: None,
            persisted_keys: std::collections::HashMap::new(),
        }
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const HELP: &str = "\
catalyst-code-core — OpenAI-compatible coding agent core (native Umans)

USAGE:
  core [OPTIONS]

OPTIONS:
      --workspace <DIR>         Workspace root (constrains all file/bash ops) [env: CATALYST_CODE_WORKSPACE]
      --base-url <URL>          OpenAI-compatible base URL [env: UMANS_BASE_URL]
      --approval <MODE>         never | destructive | always  [env: CATALYST_CODE_APPROVAL]
      --bash-timeout <SECS>     Per-command bash timeout in seconds [env: CATALYST_CODE_BASH_TIMEOUT]
      --max-bash-timeout <SECS>  Ceiling for the bash tool's per-call `timeout` override [env: CATALYST_CODE_MAX_BASH_TIMEOUT]
      --fetch-timeout <SECS>    Wall-clock timeout for the `fetch` tool [env: CATALYST_CODE_FETCH_TIMEOUT]
      --diag-timeout <SECS>     Diagnostics tool (cargo check/tsc/go build) timeout in seconds [env: CATALYST_CODE_DIAG_TIMEOUT]
      --sandbox <MODE>          none | firejail  (wraps bash in a sandbox) [env: CATALYST_CODE_SANDBOX]
      --no-network             Block bash network egress (unshare -n) [env: CATALYST_CODE_NO_NETWORK=1]
      --trust-project-plugins  Load project-scoped plugins (.catalyst-code/plugins). Off by default for safety [env: CATALYST_CODE_TRUST_PROJECT_PLUGINS=1]
      --idle-timeout <SECS>    SSE idle timeout in seconds [env: CATALYST_CODE_IDLE_TIMEOUT]
      --max-session-tokens <N> Hard session token budget (0=unlimited) [env: CATALYST_CODE_MAX_SESSION_TOKENS]
      --debug-log <FILE>        Structured JSONL debug log [env: CATALYST_CODE_DEBUG_LOG]
      --session <FILE>          Append-only JSONL session file (resume on restart) [env: CATALYST_CODE_SESSION]
      --model <ID>              Default model id
      --provider <NAME>        Active model provider (openai/anthropic endpoint; see `providers` in config) [env: UMANS_ACTIVE_PROVIDER]
      --config <FILE>           JSON config file (defaults: ./catalyst-code.json, ~/.config/catalyst-code/config.json)
  -h, --help                    Print this help
  -V, --version                 Print version

The core speaks newline-delimited JSON over stdio (commands in, events out). See README.
";

/// Parse CLI args, env vars, and config file into a Config.
pub fn load() -> Config {
    let mut c = Config::default();
    let mut config_file: Option<PathBuf> = None;
    let mut help = false;
    let mut version = false;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        let take_val = |i: &mut usize| -> Option<String> {
            if *i + 1 < args.len() {
                *i += 1;
                Some(args[*i].clone())
            } else {
                None
            }
        };
        match a.as_str() {
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--workspace" => {
                if let Some(v) = take_val(&mut i) {
                    c.workspace = PathBuf::from(v);
                }
            }
            "--base-url" => {
                if let Some(v) = take_val(&mut i) {
                    c.base_url = v;
                }
            }
            "--approval" => {
                if let Some(v) = take_val(&mut i) {
                    c.approval = Approval::parse(&v);
                }
            }
            "--bash-timeout" => {
                if let Some(v) = take_val(&mut i) {
                    c.bash_timeout_secs = v.parse().unwrap_or(c.bash_timeout_secs);
                }
            }
            "--max-bash-timeout" => {
                if let Some(v) = take_val(&mut i) {
                    c.max_bash_timeout_secs = v.parse().unwrap_or(c.max_bash_timeout_secs);
                }
            }
            "--fetch-timeout" => {
                if let Some(v) = take_val(&mut i) {
                    c.fetch_timeout_secs = v.parse().unwrap_or(c.fetch_timeout_secs);
                }
            }
            "--diag-timeout" => {
                if let Some(v) = take_val(&mut i) {
                    c.diag_timeout_secs = v.parse().unwrap_or(c.diag_timeout_secs);
                }
            }
            "--trust-project-plugins" => {
                c.trust_project_plugins = true;
            }
            "--no-trust-project-plugins" => {
                c.trust_project_plugins = false;
            }
            "--debug-log" => {
                if let Some(v) = take_val(&mut i) {
                    c.debug_log = Some(PathBuf::from(v));
                }
            }
            "--session" => {
                if let Some(v) = take_val(&mut i) {
                    c.session_file = Some(PathBuf::from(v));
                }
            }
            "--model" => {
                if let Some(v) = take_val(&mut i) {
                    c.default_model = Some(v);
                }
            }
            "--provider" => {
                // Select the active provider by name at startup (overrides
                // config/env `activeProvider`). The provider must be defined in
                // config/env; a name not in the list is ignored with a later
                // `ready` event surfacing the effective provider.
                if let Some(v) = take_val(&mut i) {
                    c.active_provider = Some(v);
                }
            }
            "--config" => {
                if let Some(v) = take_val(&mut i) {
                    config_file = Some(PathBuf::from(v));
                }
            }
            _ => { /* ignore unknown */ }
        }
        i += 1;
    }
    if help {
        print!("{HELP}");
        std::process::exit(0);
    }
    if version {
        println!("catalyst-code-core {VERSION}");
        std::process::exit(0);
    }

    // Layer 1: config file (lowest precedence among the three, applied first so
    // env/CLI can override). Pick explicit --config, else ./catalyst-code.json,
    // else ~/.config/catalyst-code/config.json.
    // Multi-layer: also load managed-settings and settings.local.json.
    let candidates: Vec<PathBuf> = match config_file {
        Some(p) => vec![p],
        None => {
            let managed = dirs_config_path();
            let managed_dir = managed.with_file_name("catalyst-code.d");
            let home = home_dir().unwrap_or_default();
            let settings_path = home.join(".config/catalyst-code/settings.json");
            let local = PathBuf::from("settings.local.json");
            let proj = PathBuf::from("settings.json");
            let mut v = vec![managed];
            // managed-settings.d/*.json
            if let Ok(rd) = std::fs::read_dir(&managed_dir) {
                let mut entries: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
                    .collect();
                entries.sort();
                v.extend(entries);
            }
            v.push(settings_path);
            v.push(proj);
            v.push(local);
            v
        }
    };
    for p in candidates {
        if let Ok(content) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                apply_json(&mut c, &v);
            }
        }
    }

    // Layer 2: env vars.
    if let Ok(v) = std::env::var("UMANS_BASE_URL") {
        c.base_url = v;
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_WORKSPACE") {
        c.workspace = PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_APPROVAL") {
        c.approval = Approval::parse(&v);
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_BASH_TIMEOUT") {
        c.bash_timeout_secs = v.parse().unwrap_or(c.bash_timeout_secs);
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_MAX_BASH_TIMEOUT") {
        if let Ok(n) = v.parse::<u64>() {
            c.max_bash_timeout_secs = n;
        }
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_FETCH_TIMEOUT") {
        if let Ok(n) = v.parse::<u64>() {
            c.fetch_timeout_secs = n;
        }
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_FETCH_MAX_BYTES") {
        if let Ok(n) = v.parse::<usize>() {
            c.fetch_max_bytes = n;
        }
    }
    // Comma-separated host glob allowlist for the fetch tool. Empty/unset = any host.
    if let Ok(v) = std::env::var("CATALYST_CODE_FETCH_ALLOWLIST") {
        c.fetch_allowlist = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_DIAG_TIMEOUT") {
        if let Ok(n) = v.parse::<u64>() {
            c.diag_timeout_secs = n;
        }
    }
    // trust_project_plugins is intentionally NOT read from any JSON config file
    // (those are merged from project-local settings.json, which an untrusted
    // repo could ship to self-enable its own plugins). Env/CLI are user-owned.
    if let Ok(v) = std::env::var("CATALYST_CODE_TRUST_PROJECT_PLUGINS") {
        c.trust_project_plugins = v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_DEBUG_LOG") {
        c.debug_log = Some(PathBuf::from(v));
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_SESSION") {
        c.session_file = Some(PathBuf::from(v));
    }
    // Sandbox / network / token-budget knobs advertised in --help (P1-19: these
    // were documented as env vars but never read, so the Dockerfile's
    // `ENV CATALYST_CODE_SANDBOX=firejail` etc. were dead). Wire them up here.
    if let Ok(v) = std::env::var("CATALYST_CODE_SANDBOX") {
        c.sandbox = Sandbox::parse(&v);
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_NO_NETWORK") {
        // Present without a value, or "1"/"true", means block network; "0"/"false" off.
        let on = v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true");
        c.no_network = on;
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_IDLE_TIMEOUT") {
        if let Ok(n) = v.parse::<u64>() {
            c.idle_timeout_secs = n;
        }
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_MAX_SESSION_TOKENS") {
        if let Ok(n) = v.parse::<u64>() {
            c.max_session_tokens = n;
        }
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_AUTO_REFLECT") {
        let on = v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true");
        let off = v == "0" || v.eq_ignore_ascii_case("false");
        if off {
            c.auto_reflect = false;
        } else if on {
            c.auto_reflect = true;
        }
    }
    if let Ok(v) = std::env::var("CATALYST_CODE_AUTO_REFLECT_MIN_TOOL_CALLS") {
        if let Ok(n) = v.parse::<u32>() {
            c.auto_reflect_min_tool_calls = n.max(1);
        }
    }
    // auto_compact: toggle automatic context compaction (threshold-triggered +
    // idle). Default true. Manual /compact always works regardless of this
    // setting. Mirrors Claude Code's autoCompactEnabled / DISABLE_AUTO_COMPACT.
    if let Ok(v) = std::env::var("CATALYST_CODE_AUTO_COMPACT") {
        let on = v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true");
        let off = v == "0" || v.eq_ignore_ascii_case("false");
        if off {
            c.auto_compact = false;
        } else if on {
            c.auto_compact = true;
        }
    }
    // compact_instructions: optional guidance woven into the compaction summarize
    // prompt ("Focus on code samples and API usage"). /compact <instructions>
    // overrides per-call; this sets the default used by auto-compaction.
    if let Ok(v) = std::env::var("CATALYST_CODE_COMPACT_INSTRUCTIONS") {
        if v.trim().is_empty() {
            c.compact_instructions = None;
        } else {
            c.compact_instructions = Some(v);
        }
    }

    // Custom providers. `UMANS_PROVIDERS` is a JSON array of provider objects
    // (same shape as the config-file `providers` field); merged after the file
    // layers so env-defined providers are appended (and deduped by name, env
    // winning). `UMANS_ACTIVE_PROVIDER` selects the active one. `--provider`
    // (CLI) wins over both.
    if let Ok(v) = std::env::var("UMANS_PROVIDERS") {
        if let Ok(arr) = serde_json::from_str::<Value>(&v) {
            if let Some(list) = arr.as_array() {
                for p in list {
                    if let Some(pc) = parse_provider(p) {
                        if !c.providers.iter().any(|x| x.name == pc.name) {
                            c.providers.push(pc);
                        }
                    }
                }
            }
        }
    }
    if let Ok(v) = std::env::var("UMANS_ACTIVE_PROVIDER") {
        c.active_provider = Some(v);
    }

    // Pre-compile bash denylist regexes once at startup.
    c.bash_deny_regex_compiled = c
        .bash_deny_regex
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect();

    c
}

/// Cross-platform home directory: prefers `$HOME`, falls back to `$USERPROFILE`
/// (Windows). Returns None if neither is set.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    None
}

/// Cross-platform config base: `~/.config/catalyst-code` on Unix, and
/// `%USERPROFILE%\.config\catalyst-code` on Windows (kept under the same
/// relative path so settings are shared across shells / WSL).
fn dirs_config_path() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/catalyst-code/config.json")
}

fn apply_json(c: &mut Config, v: &Value) {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    if let Some(x) = s("base_url") {
        c.base_url = x;
    }
    if let Some(x) = s("workspace") {
        c.workspace = PathBuf::from(x);
    }
    if let Some(x) = s("approval") {
        c.approval = Approval::parse(&x);
    }
    if let Some(b) = v.get("bash_timeout_secs").and_then(|x| x.as_u64()) {
        c.bash_timeout_secs = b;
    }
    if let Some(b) = v.get("diag_timeout_secs").and_then(|x| x.as_u64()) {
        c.diag_timeout_secs = b;
    }
    if let Some(b) = v.get("max_bash_timeout_secs").and_then(|x| x.as_u64()) {
        c.max_bash_timeout_secs = b;
    }
    if let Some(b) = v.get("fetch_timeout_secs").and_then(|x| x.as_u64()) {
        c.fetch_timeout_secs = b;
    }
    if let Some(b) = v.get("fetch_max_bytes").and_then(|x| x.as_u64()) {
        c.fetch_max_bytes = b as usize;
    }
    if let Some(arr) = v.get("fetch_allowlist").and_then(|x| x.as_array()) {
        c.fetch_allowlist = arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    if let Some(x) = s("sandbox") {
        c.sandbox = Sandbox::parse(&x);
    }
    if let Some(b) = v.get("no_network").and_then(|x| x.as_bool()) {
        c.no_network = b;
    }
    if let Some(b) = v.get("idle_timeout_secs").and_then(|x| x.as_u64()) {
        c.idle_timeout_secs = b;
    }
    if let Some(b) = v.get("max_session_tokens").and_then(|x| x.as_u64()) {
        c.max_session_tokens = b;
    }
    if let Some(b) = v.get("allow_vision").and_then(|x| x.as_bool()) {
        c.allow_vision = b;
    }
    if let Some(b) = v.get("summarize_on_compact").and_then(|x| x.as_bool()) {
        c.summarize_on_compact = b;
    }
    if let Some(b) = v.get("auto_compact").and_then(|x| x.as_bool()) {
        c.auto_compact = b;
    }
    if let Some(s) = v.get("compact_instructions").and_then(|x| x.as_str()) {
        c.compact_instructions = if s.trim().is_empty() { None } else { Some(s.to_string()) };
    }
    if let Some(f) = v.get("context_compact_at").and_then(|x| x.as_f64()) {
        c.context_compact_at = f as f32;
    }
    if let Some(b) = v.get("rolling_state").and_then(|x| x.as_bool()) {
        c.rolling_state = b;
    }
    if let Some(b) = v.get("auto_reflect").and_then(|x| x.as_bool()) {
        c.auto_reflect = b;
    }
    if let Some(n) = v
        .get("auto_reflect_min_tool_calls")
        .and_then(|x| x.as_u64())
    {
        c.auto_reflect_min_tool_calls = n.max(1) as u32;
    }
    if let Some(f) = v.get("context_digest_at").and_then(|x| x.as_f64()) {
        c.context_digest_at = f as f32;
    }
    if let Some(x) = s("debug_log") {
        c.debug_log = Some(PathBuf::from(x));
    }
    if let Some(x) = s("session") {
        c.session_file = Some(PathBuf::from(x));
    }
    if let Some(x) = s("model") {
        c.default_model = Some(x);
    }
    if let Some(arr) = v.get("bash_deny").and_then(|x| x.as_array()) {
        c.bash_deny = arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    // Regex denylist patterns
    if let Some(arr) = v.get("bash_deny_regex").and_then(|x| x.as_array()) {
        c.bash_deny_regex = arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    // Permission rules
    if let Some(perms) = v.get("permissions").and_then(|x| x.as_object()) {
        for (behavior_key, rules_arr) in perms {
            let behavior = PermissionBehavior::parse(behavior_key);
            if let Some(arr) = rules_arr.as_array() {
                for entry in arr {
                    if let Some(rule_str) = entry.as_str() {
                        if let Some(rule) = parse_permission_rule(rule_str, behavior.clone()) {
                            match behavior {
                                PermissionBehavior::Allow => c.allow_rules.push(rule),
                                PermissionBehavior::Deny => c.deny_rules.push(rule),
                                PermissionBehavior::Ask => c.ask_rules.push(rule),
                            }
                        }
                    }
                }
            }
        }
    }
    // Plugin settings
    if let Some(plugins) = v.get("plugins") {
        if let Some(dir) = plugins.get("dir").and_then(|x| x.as_str()) {
            c.plugin_dir = PathBuf::from(dir);
        }
        if let Some(disabled) = plugins.get("disabled").and_then(|x| x.as_array()) {
            c.plugins_disabled = disabled
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
    }
    // Subagent system config (pi-subagents port).
    if let Some(sa) = v.get("subagents").and_then(|x| x.as_object()) {
        if let Some(n) = sa.get("maxSubagentDepth").and_then(|x| x.as_u64()) {
            c.subagents.max_depth = n as u32;
        }
        if let Some(m) = sa.get("intercomBridge").and_then(|x| x.as_object()) {
            if let Some(mode) = m.get("mode").and_then(|x| x.as_str()) {
                c.subagents.intercom_bridge_mode = IntercomBridgeMode::parse(mode);
            }
        }
        if let Some(n) = sa.get("parallel").and_then(|x| x.as_object()) {
            if let Some(mt) = n.get("maxTasks").and_then(|x| x.as_u64()) {
                c.subagents.parallel_max_tasks = mt as u32;
            }
            if let Some(cc) = n.get("concurrency").and_then(|x| x.as_u64()) {
                c.subagents.parallel_concurrency = cc as u32;
            }
        }
        if let Some(b) = sa.get("asyncByDefault").and_then(|x| x.as_bool()) {
            c.subagents.async_by_default = b;
        }
        if let Some(b) = sa.get("disableBuiltins").and_then(|x| x.as_bool()) {
            c.subagents.disable_builtins = b;
        }
        if let Some(ovs) = sa.get("agentOverrides").and_then(|x| x.as_object()) {
            for (name, ov) in ovs {
                let mut o = AgentOverride::default();
                if let Some(m) = ov.get("model").and_then(|x| x.as_str()) {
                    o.model = Some(m.to_string());
                }
                if let Some(arr) = ov.get("fallbackModels").and_then(|x| x.as_array()) {
                    o.fallback_models = arr
                        .iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect();
                }
                if let Some(t) = ov.get("thinking").and_then(|x| x.as_str()) {
                    o.thinking = Some(t.to_string());
                }
                if let Some(d) = ov.get("disabled").and_then(|x| x.as_bool()) {
                    o.disabled = d;
                }
                c.subagents.agent_overrides.insert(name.clone(), o);
            }
        }
    }
    // Custom providers (openai/anthropic endpoints).
    if let Some(arr) = v.get("providers").and_then(|x| x.as_array()) {
        for p in arr {
            if let Some(pc) = parse_provider(p) {
                c.providers.push(pc);
            }
        }
    }
    // Per-provider API keys persisted by the TUI (settings.json `provider_keys`)
    // and the legacy single `api_key`. These seed the runtime key map at
    // startup (see main.rs) so a key set via `/key` or the settings modal
    // survives a restart and overrides config/env keys (runtime keys win).
    if let Some(obj) = v.get("provider_keys").and_then(|x| x.as_object()) {
        for (name, key) in obj {
            if let Some(k) = key.as_str().filter(|s| !s.is_empty()) {
                c.persisted_keys.insert(name.clone(), k.to_string());
            }
        }
    }
    if let Some(k) = s("api_key").filter(|x| !x.is_empty()) {
        // Legacy single key applies to the default provider; only seed
        // "default" when no per-provider key already named it.
        c.persisted_keys.entry("default".to_string()).or_insert(k);
    }
    // Active provider: accept the TUI's snake_case `active_provider` as well as
    // the camelCase form used by core-owned config files.
    if let Some(name) = v.get("activeProvider").and_then(|x| x.as_str()) {
        c.active_provider = Some(name.to_string());
    }
    if let Some(name) = v.get("active_provider").and_then(|x| x.as_str()) {
        c.active_provider = Some(name.to_string());
    }
}

/// Parse one provider object from JSON. Requires a non-empty `name` and
/// `base_url`; `kind` defaults to `openai`. `headers` accepts an object
/// `{"K":"V"}` or an array of `["K","V"]` / `{"name","value"}` entries.
pub fn parse_provider(v: &Value) -> Option<ProviderConfig> {
    let name = v.get("name").and_then(|x| x.as_str())?.to_string();
    if name.is_empty() {
        return None;
    }
    let base_url = v
        .get("base_url")
        .or_else(|| v.get("baseUrl"))
        .and_then(|x| x.as_str())?
        .to_string();
    let kind = v
        .get("kind")
        .and_then(|x| x.as_str())
        .map(ProviderKind::parse)
        .unwrap_or(ProviderKind::OpenAI);
    let api_key = v
        .get("api_key")
        .or_else(|| v.get("apiKey"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let api_key_env = v
        .get("api_key_env")
        .or_else(|| v.get("apiKeyEnv"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let headers = parse_headers(v.get("headers"));
    Some(ProviderConfig {
        name,
        kind,
        base_url,
        api_key,
        api_key_env,
        headers,
    })
}

/// Parse a `headers` value into an ordered (name, value) vec. Accepts an
/// object `{"K":"V"}` or an array of `["K","V"]` / `{"name","value"}`.
pub fn parse_headers(v: Option<&Value>) -> Vec<(String, String)> {
    let Some(v) = v else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            if let Some(val) = val.as_str() {
                out.push((k.clone(), val.to_string()));
            }
        }
        return out;
    }
    if let Some(arr) = v.as_array() {
        for entry in arr {
            if let Some(obj) = entry.as_object() {
                let k = obj.get("name").and_then(|x| x.as_str());
                let val = obj.get("value").and_then(|x| x.as_str());
                if let (Some(k), Some(val)) = (k, val) {
                    out.push((k.to_string(), val.to_string()));
                }
            } else if let Some(pair) = entry.as_array() {
                if pair.len() == 2 {
                    if let (Some(k), Some(val)) = (pair[0].as_str(), pair[1].as_str()) {
                        out.push((k.to_string(), val.to_string()));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn approval_parse_roundtrip() {
        assert_eq!(Approval::parse("never"), Approval::Never);
        assert_eq!(Approval::parse("always"), Approval::Always);
        assert_eq!(Approval::parse("destructive"), Approval::Destructive);
        assert_eq!(Approval::parse("garbage"), Approval::Destructive);
    }

    #[test]
    fn sandbox_parse() {
        assert_eq!(Sandbox::parse("firejail"), Sandbox::Firejail);
        assert_eq!(Sandbox::parse("fj"), Sandbox::Firejail);
        assert_eq!(Sandbox::parse("none"), Sandbox::None);
        assert_eq!(Sandbox::parse(""), Sandbox::None);
        assert_eq!(Sandbox::Firejail.as_str(), "firejail");
        assert_eq!(Sandbox::None.as_str(), "none");
    }

    #[test]
    fn defaults_lean_agentic() {
        let c = Config::default();
        assert!(c.idle_timeout_secs >= 120);
        assert!(c.summarize_on_compact);
        assert!(c.allow_vision);
    }

    #[test]
    fn env_overrides_applied() {
        // Save, set, restore the advertised env knobs (P1-19). Only this test
        // calls load(), so there's no parallel-reader race on these vars.
        let vars = [
            ("CATALYST_CODE_SANDBOX", "firejail"),
            ("CATALYST_CODE_NO_NETWORK", "1"),
            ("CATALYST_CODE_IDLE_TIMEOUT", "42"),
            ("CATALYST_CODE_MAX_SESSION_TOKENS", "123456"),
        ];
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in &vars {
            std::env::set_var(k, v);
        }
        let c = load();
        for (k, _) in &vars {
            std::env::remove_var(k);
        }
        for (k, prev) in saved {
            match prev {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
        assert_eq!(c.sandbox, Sandbox::Firejail);
        assert!(c.no_network);
        assert_eq!(c.idle_timeout_secs, 42);
        assert_eq!(c.max_session_tokens, 123456);
    }

    #[test]
    fn provider_kind_parse() {
        assert_eq!(ProviderKind::parse("openai"), ProviderKind::OpenAI);
        assert_eq!(ProviderKind::parse("OpenAI"), ProviderKind::OpenAI);
        assert_eq!(ProviderKind::parse("anthropic"), ProviderKind::Anthropic);
        assert_eq!(ProviderKind::parse("claude"), ProviderKind::Anthropic);
        assert_eq!(ProviderKind::parse("garbage"), ProviderKind::OpenAI);
        assert!(ProviderKind::OpenAI.is_openai());
        assert!(ProviderKind::Anthropic.is_anthropic());
        assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
    }

    #[test]
    fn parse_provider_valid() {
        let v = json!({
            "name": "anthropic",
            "kind": "anthropic",
            "base_url": "https://api.anthropic.com/v1",
            "api_key_env": "ANTHROPIC_API_KEY",
            "headers": {"anthropic-version": "2023-06-01"}
        });
        let p = parse_provider(&v).unwrap();
        assert_eq!(p.name, "anthropic");
        assert_eq!(p.kind, ProviderKind::Anthropic);
        assert_eq!(p.base_url, "https://api.anthropic.com/v1");
        assert_eq!(p.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert!(p.api_key.is_none());
        assert_eq!(
            p.headers,
            vec![("anthropic-version".into(), "2023-06-01".into())]
        );
    }

    #[test]
    fn parse_provider_defaults_openai() {
        let v = json!({"name": "local", "base_url": "http://localhost:11434/v1"});
        let p = parse_provider(&v).unwrap();
        assert_eq!(p.kind, ProviderKind::OpenAI);
        assert!(p.api_key.is_none());
        assert!(p.api_key_env.is_none());
        assert!(p.headers.is_empty());
    }

    #[test]
    fn parse_provider_requires_name_and_url() {
        assert!(parse_provider(&json!({"base_url": "x"})).is_none()); // no name
        assert!(parse_provider(&json!({"name": "x"})).is_none()); // no base_url
        assert!(parse_provider(&json!({"name": "", "base_url": "x"})).is_none());
    }

    #[test]
    fn parse_headers_variants() {
        // object form
        let h = parse_headers(Some(&json!({"X-A": "1", "X-B": "2"})));
        assert_eq!(h.len(), 2);
        assert!(h.contains(&("X-A".into(), "1".into())));
        // array of [k,v]
        let h = parse_headers(Some(&json!([["K", "V"], ["K2", "V2"]])));
        assert_eq!(
            h,
            vec![("K".into(), "V".into()), ("K2".into(), "V2".into())]
        );
        // array of {name,value}
        let h = parse_headers(Some(&json!([{"name":"N","value":"V"}])));
        assert_eq!(h, vec![("N".into(), "V".into())]);
        // none / empty
        assert!(parse_headers(None).is_empty());
    }

    #[test]
    fn resolve_provider_legacy_default_when_none_configured() {
        let c = Config {
            base_url: "https://example.test/v1".into(),
            ..Default::default()
        };
        let keys = std::collections::HashMap::new();
        let r = c.resolve_provider(&keys);
        assert_eq!(r.name, "default");
        assert!(r.kind.is_openai());
        assert_eq!(r.base_url, "https://example.test/v1");
        assert!(r.api_key.is_none());
    }

    #[test]
    fn resolve_provider_uses_runtime_key_then_config_then_env() {
        // Save/restore env vars this test touches.
        let env = "PROV_TEST_KEY_ENV";
        let saved = std::env::var(env).ok();
        std::env::set_var(env, "env-key");

        let mut c = Config::default();
        c.providers.push(ProviderConfig {
            name: "p".into(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: Some("config-key".into()),
            api_key_env: Some(env.into()),
            headers: vec![("h".into(), "v".into())],
        });
        // active_provider None -> first configured provider.
        let mut keys = std::collections::HashMap::new();
        // runtime wins over config + env
        keys.insert("p".to_string(), "runtime-key".to_string());
        let r = c.resolve_provider(&keys);
        assert_eq!(r.api_key.as_deref(), Some("runtime-key"));

        // no runtime key -> config literal wins
        keys.clear();
        let r = c.resolve_provider(&keys);
        assert_eq!(r.api_key.as_deref(), Some("config-key"));

        // no runtime, no config literal -> env var
        c.providers[0].api_key = None;
        let r = c.resolve_provider(&keys);
        assert_eq!(r.api_key.as_deref(), Some("env-key"));
        assert!(r.kind.is_anthropic());
        assert_eq!(r.headers, vec![("h".into(), "v".into())]);

        match saved {
            Some(v) => std::env::set_var(env, v),
            None => std::env::remove_var(env),
        }
    }

    #[test]
    fn resolve_provider_honors_active_name() {
        let mut c = Config::default();
        c.providers.push(ProviderConfig {
            name: "first".into(),
            kind: ProviderKind::OpenAI,
            base_url: "https://first/v1".into(),
            ..Default::default()
        });
        c.providers.push(ProviderConfig {
            name: "second".into(),
            kind: ProviderKind::Anthropic,
            base_url: "https://second/v1".into(),
            ..Default::default()
        });
        let keys = std::collections::HashMap::new();
        // None -> first
        assert_eq!(c.resolve_provider(&keys).name, "first");
        // explicit active -> that one
        c.active_provider = Some("second".into());
        assert_eq!(c.resolve_provider(&keys).name, "second");
        assert!(c.resolve_provider(&keys).kind.is_anthropic());
        // unknown active name -> falls back to first configured
        c.active_provider = Some("nope".into());
        assert_eq!(c.resolve_provider(&keys).name, "first");
    }

    #[test]
    fn apply_json_loads_providers() {
        let mut c = Config::default();
        let v = json!({
            "providers": [
                {"name":"umans","kind":"openai","base_url":"https://api.code.umans.ai/v1"},
                {"name":"anthropic","kind":"anthropic","base_url":"https://api.anthropic.com/v1","api_key_env":"ANTHROPIC_API_KEY"}
            ],
            "activeProvider": "anthropic"
        });
        apply_json(&mut c, &v);
        assert_eq!(c.providers.len(), 2);
        assert_eq!(c.providers[0].name, "umans");
        assert!(c.providers[0].kind.is_openai());
        assert_eq!(c.providers[1].name, "anthropic");
        assert!(c.providers[1].kind.is_anthropic());
        assert_eq!(c.active_provider.as_deref(), Some("anthropic"));
    }

    // The TUI persists API keys to settings.json as `provider_keys` (a
    // name->key map) + the legacy `api_key`, and the active provider as
    // snake_case `active_provider`. The core must read these so a key set via
    // /key survives a restart and overrides config/env (it seeds runtime
    // keys, which win in resolution).
    #[test]
    fn apply_json_loads_tui_persisted_keys() {
        let mut c = Config::default();
        let v = json!({
            "providers": [
                {"name":"glm","kind":"openai","base_url":"https://open.bigmodel.cn/api/paas/v4","api_key_env":"GLM_API_KEY"}
            ],
            "provider_keys": {"glm": "sk-tui-saved"},
            "api_key": "sk-legacy",
            "active_provider": "glm"
        });
        apply_json(&mut c, &v);
        assert_eq!(
            c.persisted_keys.get("glm").map(|s| s.as_str()),
            Some("sk-tui-saved")
        );
        // legacy key seeds "default" but does NOT clobber a named provider key
        assert_eq!(
            c.persisted_keys.get("default").map(|s| s.as_str()),
            Some("sk-legacy")
        );
        assert_eq!(c.active_provider.as_deref(), Some("glm"));
    }

    // A persisted TUI key (seeded into runtime_keys at startup) must override
    // both the provider's config `api_key` and its `api_key_env` env var.
    #[test]
    fn persisted_tui_key_overrides_config_and_env() {
        let mut c = Config::default();
        let v = json!({
            "providers": [
                {"name":"p","kind":"openai","base_url":"https://x/v1","api_key":"sk-config","api_key_env":"P_API_KEY"}
            ],
            "active_provider": "p",
            "provider_keys": {"p": "sk-tui-new"}
        });
        apply_json(&mut c, &v);
        // config key alone (no runtime override, no env)
        let empty = std::collections::HashMap::new();
        assert_eq!(
            c.resolve_provider(&empty).api_key.as_deref(),
            Some("sk-config")
        );
        // runtime override (seeded from persisted_keys) wins over config + env
        let mut keys = std::collections::HashMap::new();
        keys.insert("p".to_string(), "sk-tui-new".to_string());
        assert_eq!(
            c.resolve_provider(&keys).api_key.as_deref(),
            Some("sk-tui-new")
        );
    }

    // OpenCode Go is one subscription/key serving models over TWO wire
    // protocols. preset_provider_configs() must expand the single preset into
    // two provider configs (OpenAI-kind + Anthropic-kind) sharing the base URL
    // + key, while every other preset stays a single config.
    #[test]
    fn preset_provider_configs_opencode_go_expands_to_two() {
        let p = find_preset("opencode-go").expect("opencode-go preset exists");
        // with an explicit key -> stored as a literal on both configs
        let configs = preset_provider_configs(p, Some("sk-go".to_string()));
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].name, "opencode-go");
        assert!(configs[0].kind.is_openai());
        assert_eq!(configs[1].name, "opencode-go-anthropic");
        assert!(configs[1].kind.is_anthropic());
        for c in &configs {
            assert_eq!(c.base_url, "https://opencode.ai/zen/go/v1");
            assert_eq!(c.api_key.as_deref(), Some("sk-go"));
            assert!(c.api_key_env.is_none());
        }
        // without a key -> env-var NAME persisted (secret stays in env), both
        // configs reference the same env var.
        let configs = preset_provider_configs(p, None);
        assert_eq!(configs.len(), 2);
        for c in &configs {
            assert_eq!(c.api_key_env.as_deref(), Some("OPENCODE_GO_API_KEY"));
            assert!(c.api_key.is_none());
        }
    }

    #[test]
    fn preset_provider_configs_single_for_other_presets() {
        let p = find_preset("openai").expect("openai preset exists");
        let configs = preset_provider_configs(p, Some("sk".to_string()));
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "openai");
    }
}
