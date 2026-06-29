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
    pub max_turns: usize,
    pub context_compact_at: f32, // fraction of context_window that triggers compaction
    pub debug_log: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub default_model: Option<String>,
    // --- production knobs (items 3,4,7) ---
    pub sandbox: Sandbox,            // --sandbox firejail wraps bash
    pub no_network: bool,           // --no-network: unshare -n on bash
    pub idle_timeout_secs: u64,      // per-chunk SSE idle timeout
    pub max_session_tokens: u64,     // hard session token budget (0 = unlimited)
    pub summarize_on_compact: bool, // use a model call to summarize dropped turns
    pub spawn_max_turns: usize,     // turn cap for the `spawn` subagent tool
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
            // ponytail: raised from 25 — real agentic loops need headroom.
            // A token/cost budget (max_session_tokens) is the real ceiling.
            max_turns: 200,
            context_compact_at: 0.70,
            debug_log: None,
            session_file: None,
            default_model: None,
            sandbox: Sandbox::None,
            no_network: false,
            idle_timeout_secs: 120, // some reasoning models think >60s before first token
            max_session_tokens: 0,
            summarize_on_compact: true,
            spawn_max_turns: 10,
            allow_vision: true,
            allow_rules: Vec::new(),
            deny_rules: Vec::new(),
            ask_rules: Vec::new(),
            plugin_dir: PathBuf::from(".umans-harness/plugins"),
            plugins_disabled: Vec::new(),
            bash_deny_regex: Vec::new(),
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
      --max-turns <N>           Max agentic tool turns per prompt [env: UMANS_HARNESS_MAX_TURNS]
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
            "--max-turns" => if let Some(v) = take_val(&mut i) { c.max_turns = v.parse().unwrap_or(c.max_turns); },
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
            let home = std::env::var("HOME").unwrap_or_default();
            let settings_path = PathBuf::from(home).join(".config/umans-harness/settings.json");
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
    if let Ok(v) = std::env::var("UMANS_HARNESS_MAX_TURNS") { c.max_turns = v.parse().unwrap_or(c.max_turns); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_DEBUG_LOG") { c.debug_log = Some(PathBuf::from(v)); }
    if let Ok(v) = std::env::var("UMANS_HARNESS_SESSION") { c.session_file = Some(PathBuf::from(v)); }

    c
}

fn dirs_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/umans-harness/config.json")
}

fn apply_json(c: &mut Config, v: &Value) {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    if let Some(x) = s("base_url") { c.base_url = x; }
    if let Some(x) = s("workspace") { c.workspace = PathBuf::from(x); }
    if let Some(x) = s("approval") { c.approval = Approval::parse(&x); }
    if let Some(b) = v.get("bash_timeout_secs").and_then(|x| x.as_u64()) { c.bash_timeout_secs = b; }
    if let Some(n) = v.get("max_turns").and_then(|x| x.as_u64()) { c.max_turns = n as usize; }
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
        assert!(c.max_turns >= 100, "max_turns default too low: {}", c.max_turns);
        assert!(c.idle_timeout_secs >= 120);
        assert!(c.summarize_on_compact);
        assert!(c.allow_vision);
    }
}
