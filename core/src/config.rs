// Config: CLI flags + env vars + config files. No clap, no toml — hand-rolled.
// Precedence: CLI > env > settings.local.json > settings.json
//   > ~/.config/settings.json > managed-settings.json > managed-settings.d/*.json
// Arrays concatenate+deduplicate; objects deep merge; null means delete.
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Approval {
    Never,      // auto-approve everything (trust the model fully)
    Destructive,// ask only for bash + write_file + edit (default)
    Always,     // ask for every tool call
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
    if tool_name.is_empty() { return None; }
    Some(PermissionRule { tool_name, rule_content, behavior })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sandbox {
    None,       // no sandboxing (default; denylist tripwire only)
    Firejail,   // wrap bash in `firejail` with a writable-workspace profile
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
    pub bash_deny: Vec<String>,
    pub max_read_bytes: u64,
    pub max_read_lines: usize,
    pub context_compact_at: f32, // fraction of context_window that triggers compaction
    pub context_digest_at: f32,  // fraction of context_window that triggers stale-tool-result digesting (sub-threshold reclaim; 0 disables)
    pub debug_log: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub default_model: Option<String>,
    // --- production knobs (items 3,4,7) ---
    pub sandbox: Sandbox,            // --sandbox firejail wraps bash
    pub no_network: bool,           // --no-network: unshare -n on bash
    pub idle_timeout_secs: u64,      // per-chunk SSE idle timeout
    pub max_session_tokens: u64,     // hard session token budget (0 = unlimited)
    pub summarize_on_compact: bool, // use a model call to summarize dropped turns
    pub allow_vision: bool,         // accept image_url content in send
    // --- permission rules (item 1) ---
    pub allow_rules: Vec<PermissionRule>,
    pub deny_rules: Vec<PermissionRule>,
    pub ask_rules: Vec<PermissionRule>,
    // --- plugin system (centerpiece) ---
    pub plugin_dir: PathBuf,        // directory scanned for plugins
    pub plugins_disabled: Vec<String>, // plugin names that are explicitly disabled
    // --- regex denylist upgrade (quick win) ---
    pub bash_deny_regex: Vec<String>, // regex patterns that block bash commands
    pub bash_deny_regex_compiled: Vec<regex::Regex>, // pre-compiled at startup
    // --- subagent system (pi-subagents port) ---
    pub subagents: SubagentConfig,
}

/// Intercom bridge mode: controls whether subagents get a coordination channel
/// back to the orchestrator and to each other.
#[derive(Clone, Debug, PartialEq)]
pub enum IntercomBridgeMode {
    Off,       // no intercom tools injected into subagents
    ForkOnly,  // only for forked-context runs
    Always,    // always inject (default)
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

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "https://api.code.umans.ai/v1".into(),
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            approval: Approval::Destructive,
            bash_timeout_secs: 30,
            bash_deny: vec![
                // ponytail: minimal denylist of obviously catastrophic commands.
                // Not a security boundary (use a sandbox for that); a tripwire.
                "rm -rf /".into(),
                "rm -rf ~".into(),
                "mkfs".into(),
                "dd if=/dev/zero of=/dev/sd".into(),
                ":(){:|:&};:".into(),
            ],
            max_read_bytes: 5_242_880,   // 5 MiB (was 1 MiB; real files exceed 1MB)
            max_read_lines: 10_000,        // was 2000; pagination covers the rest
            context_compact_at: 0.70,
            context_digest_at: 0.40,
            debug_log: None,
            session_file: None,
            default_model: None,
            sandbox: Sandbox::None,
            no_network: false,
            idle_timeout_secs: 120, // some reasoning models think >60s before first token
            max_session_tokens: 0,
            summarize_on_compact: true,
            allow_vision: true,
            allow_rules: Vec::new(),
            deny_rules: Vec::new(),
            ask_rules: Vec::new(),
            plugin_dir: PathBuf::from(".umans-harness/plugins"),
            plugins_disabled: Vec::new(),
            bash_deny_regex: Vec::new(),
            bash_deny_regex_compiled: Vec::new(),
            subagents: SubagentConfig::default(),
        }
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const HELP: &str = "\
umans-harness-core — OpenAI-compatible coding agent core (native Umans)

USAGE:
  core [OPTIONS]

OPTIONS:
      --workspace <DIR>         Workspace root (constrains all file/bash ops) [env: UMANS_HARNESS_WORKSPACE]
      --base-url <URL>          OpenAI-compatible base URL [env: UMANS_BASE_URL]
      --approval <MODE>         never | destructive | always  [env: UMANS_HARNESS_APPROVAL]
      --bash-timeout <SECS>     Per-command bash timeout in seconds [env: UMANS_HARNESS_BASH_TIMEOUT]
      --sandbox <MODE>          none | firejail  (wraps bash in a sandbox) [env: UMANS_HARNESS_SANDBOX]
      --no-network             Block bash network egress (unshare -n) [env: UMANS_HARNESS_NO_NETWORK=1]
      --idle-timeout <SECS>    SSE idle timeout in seconds [env: UMANS_HARNESS_IDLE_TIMEOUT]
      --max-session-tokens <N> Hard session token budget (0=unlimited) [env: UMANS_HARNESS_MAX_SESSION_TOKENS]
      --debug-log <FILE>        Structured JSONL debug log [env: UMANS_HARNESS_DEBUG_LOG]
      --session <FILE>          Append-only JSONL session file (resume on restart) [env: UMANS_HARNESS_SESSION]
      --model <ID>              Default model id
      --config <FILE>           JSON config file (defaults: ./umans-harness.json, ~/.config/umans-harness/config.json)
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
            "--workspace" => if let Some(v) = take_val(&mut i) { c.workspace = PathBuf::from(v); },
            "--base-url" => if let Some(v) = take_val(&mut i) { c.base_url = v; },
            "--approval" => if let Some(v) = take_val(&mut i) { c.approval = Approval::parse(&v); },
            "--bash-timeout" => if let Some(v) = take_val(&mut i) { c.bash_timeout_secs = v.parse().unwrap_or(c.bash_timeout_secs); },
            "--debug-log" => if let Some(v) = take_val(&mut i) { c.debug_log = Some(PathBuf::from(v)); },
            "--session" => if let Some(v) = take_val(&mut i) { c.session_file = Some(PathBuf::from(v)); },
            "--model" => if let Some(v) = take_val(&mut i) { c.default_model = Some(v); },
            "--config" => if let Some(v) = take_val(&mut i) { config_file = Some(PathBuf::from(v)); },
            _ => { /* ignore unknown */ }
        }
        i += 1;
    }
    if help {
        print!("{HELP}");
        std::process::exit(0);
    }
    if version {
        println!("umans-harness-core {VERSION}");
        std::process::exit(0);
    }

    // Layer 1: config file (lowest precedence among the three, applied first so
    // env/CLI can override). Pick explicit --config, else ./umans-harness.json,
    // else ~/.config/umans-harness/config.json.
    // Multi-layer: also load managed-settings and settings.local.json.
    let candidates: Vec<PathBuf> = match config_file {
        Some(p) => vec![p],
        None => {
            let managed = dirs_config_path();
            let managed_dir = managed.with_file_name("umans-harness.d");
            let home = home_dir().unwrap_or_default();
            let settings_path = home.join(".config/umans-harness/settings.json");
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
    if let Ok(v) = std::env::var("UMANS_BASE_URL") { c.base_url = v; }
    if let Ok(v) = std::env::var("UMANS_HARNESS_WORKSPACE") { c.workspace = PathBuf::from(v); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_APPROVAL") { c.approval = Approval::parse(&v); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_BASH_TIMEOUT") { c.bash_timeout_secs = v.parse().unwrap_or(c.bash_timeout_secs); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_DEBUG_LOG") { c.debug_log = Some(PathBuf::from(v)); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_SESSION") { c.session_file = Some(PathBuf::from(v)); }
    // Sandbox / network / token-budget knobs advertised in --help (P1-19: these
    // were documented as env vars but never read, so the Dockerfile's
    // `ENV UMANS_HARNESS_SANDBOX=firejail` etc. were dead). Wire them up here.
    if let Ok(v) = std::env::var("UMANS_HARNESS_SANDBOX") { c.sandbox = Sandbox::parse(&v); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_NO_NETWORK") {
        // Present without a value, or "1"/"true", means block network; "0"/"false" off.
        let on = v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true");
        c.no_network = on;
    }
    if let Ok(v) = std::env::var("UMANS_HARNESS_IDLE_TIMEOUT") {
        if let Ok(n) = v.parse::<u64>() { c.idle_timeout_secs = n; }
    }
    if let Ok(v) = std::env::var("UMANS_HARNESS_MAX_SESSION_TOKENS") {
        if let Ok(n) = v.parse::<u64>() { c.max_session_tokens = n; }
    }

    // Pre-compile bash denylist regexes once at startup.
    c.bash_deny_regex_compiled = c.bash_deny_regex.iter()
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

/// Cross-platform config base: `~/.config/umans-harness` on Unix, and
/// `%USERPROFILE%\.config\umans-harness` on Windows (kept under the same
/// relative path so settings are shared across shells / WSL).
fn dirs_config_path() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/umans-harness/config.json")
}

fn apply_json(c: &mut Config, v: &Value) {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    if let Some(x) = s("base_url") { c.base_url = x; }
    if let Some(x) = s("workspace") { c.workspace = PathBuf::from(x); }
    if let Some(x) = s("approval") { c.approval = Approval::parse(&x); }
    if let Some(b) = v.get("bash_timeout_secs").and_then(|x| x.as_u64()) { c.bash_timeout_secs = b; }
    if let Some(x) = s("sandbox") { c.sandbox = Sandbox::parse(&x); }
    if let Some(b) = v.get("no_network").and_then(|x| x.as_bool()) { c.no_network = b; }
    if let Some(b) = v.get("idle_timeout_secs").and_then(|x| x.as_u64()) { c.idle_timeout_secs = b; }
    if let Some(b) = v.get("max_session_tokens").and_then(|x| x.as_u64()) { c.max_session_tokens = b; }
    if let Some(b) = v.get("allow_vision").and_then(|x| x.as_bool()) { c.allow_vision = b; }
    if let Some(b) = v.get("summarize_on_compact").and_then(|x| x.as_bool()) { c.summarize_on_compact = b; }
    if let Some(f) = v.get("context_digest_at").and_then(|x| x.as_f64()) { c.context_digest_at = f as f32; }
    if let Some(x) = s("debug_log") { c.debug_log = Some(PathBuf::from(x)); }
    if let Some(x) = s("session") { c.session_file = Some(PathBuf::from(x)); }
    if let Some(x) = s("model") { c.default_model = Some(x); }
    if let Some(arr) = v.get("bash_deny").and_then(|x| x.as_array()) {
        c.bash_deny = arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    }
    // Regex denylist patterns
    if let Some(arr) = v.get("bash_deny_regex").and_then(|x| x.as_array()) {
        c.bash_deny_regex = arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
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
            c.plugins_disabled = disabled.iter().filter_map(|x| x.as_str().map(String::from)).collect();
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
            if let Some(mt) = n.get("maxTasks").and_then(|x| x.as_u64()) { c.subagents.parallel_max_tasks = mt as u32; }
            if let Some(cc) = n.get("concurrency").and_then(|x| x.as_u64()) { c.subagents.parallel_concurrency = cc as u32; }
        }
        if let Some(b) = sa.get("asyncByDefault").and_then(|x| x.as_bool()) { c.subagents.async_by_default = b; }
        if let Some(b) = sa.get("disableBuiltins").and_then(|x| x.as_bool()) { c.subagents.disable_builtins = b; }
        if let Some(ovs) = sa.get("agentOverrides").and_then(|x| x.as_object()) {
            for (name, ov) in ovs {
                let mut o = AgentOverride::default();
                if let Some(m) = ov.get("model").and_then(|x| x.as_str()) { o.model = Some(m.to_string()); }
                if let Some(arr) = ov.get("fallbackModels").and_then(|x| x.as_array()) {
                    o.fallback_models = arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                }
                if let Some(t) = ov.get("thinking").and_then(|x| x.as_str()) { o.thinking = Some(t.to_string()); }
                if let Some(d) = ov.get("disabled").and_then(|x| x.as_bool()) { o.disabled = d; }
                c.subagents.agent_overrides.insert(name.clone(), o);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            ("UMANS_HARNESS_SANDBOX", "firejail"),
            ("UMANS_HARNESS_NO_NETWORK", "1"),
            ("UMANS_HARNESS_IDLE_TIMEOUT", "42"),
            ("UMANS_HARNESS_MAX_SESSION_TOKENS", "123456"),
        ];
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in &vars { std::env::set_var(k, v); }
        let c = load();
        for (k, _) in &vars { std::env::remove_var(k); }
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
}
