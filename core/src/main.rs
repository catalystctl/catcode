// umans-harness-core: stdio JSON-RPC server. The TUI spawns this binary,
// writes commands to stdin, and reads newline-delimited events from stdout.
mod config;
mod git_ctx;
mod hashline;
mod logging;
mod memory;
mod plugins;
mod protocol;
mod provider;
mod session;
mod tools;
mod workspace;

use config::{Config, Approval, PermissionRule};
use logging::{estimate_messages_tokens, Logger, TurnTimer};
use protocol::{emit, Command, Event, ModelInfo};
use plugins::{PluginManager, PLUGIN_DOCS};
use memory::memory_injection;
use git_ctx::{read_git_context, git_context_injection};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct QueuedPrompt {
    prompt: String,
    model: String,
    effort: String,
}

/// A pending approval request the TUI must answer before the tool runs.

const SYSTEM_PROMPT_BASE: &str = r#"You are a coding agent operating inside a Rust/Go harness with native Umans model access.
You can read, edit, write, and list files, search with grep/glob, and run bash commands — all confined to the current workspace directory.

File editing uses HASH ANCHORS, not line numbers:
- read_file returns each line as "HASH│content". The 4-char HASH on the left is the anchor for that line.
- To change a file, call edit with those hashes: op=replace needs start+end hashes (inclusive; single line = start==end; delete = lines:[]); op=append inserts after a pos hash (omit pos for end-of-file); op=prepend inserts before a pos hash (omit pos for start-of-file). You can pass multiple ops in one edit call; they apply atomically.
- If edit returns a "stale anchor" error, the file changed since your read — re-read it and retry with fresh hashes.
- Use write_file only for new files or complete rewrites; prefer edit for targeted changes. Use grep to search and glob to find files by pattern.

All paths are relative to the workspace root; absolute paths and ".." are rejected.
Work step by step: read/search before changing, make the smallest correct change, then verify with a command.
Be concise. Prefer standard tools. When done, summarize what you did in two lines."#;

/// Build the full system prompt by appending git context, memory context,
/// and the plugin self-bootstrapping docs.
fn build_system_prompt(workspace: &std::path::Path) -> String {
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
    prompt
}

/// A pending approval request the TUI must answer before the tool runs.
struct PendingApproval {
    request_id: String,
    tool: String,
    args: Value,
    notify: Arc<Notify>,
    granted: Mutex<Option<bool>>, // Some(true)=approved, Some(false)=denied, None=awaiting
    escalated: Mutex<bool>,       // "always" was chosen → upgrade session mode
}

struct State {
    cfg: RwLock<Config>,
    api_key: RwLock<Option<String>>,
    conversation: Mutex<Vec<Value>>,
    models: RwLock<Vec<ModelInfo>>,
    current: Mutex<Option<CancellationToken>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    pending: Mutex<Option<Arc<PendingApproval>>>,
    logger: Logger,
    /// Token counts accumulated across the session (for the status bar).
    tokens_in: Mutex<u64>,
    tokens_out: Mutex<u64>,
    /// Tool kinds ("destructive"/"readonly") the user said "always" to,
    /// so subsequent calls of that kind skip the gate without escalating all.
    escalated_kinds: Mutex<std::collections::HashSet<&'static str>>,
    /// Prompt queued while a turn was running (one-deep buffer).
    queued: Mutex<Option<QueuedPrompt>>,
    /// Plugin manager — scans, loads, and executes hooks.
    plugin_manager: PluginManager,
    /// Last time a turn completed (for idle compaction).
    last_turn_time: Mutex<std::time::Instant>,
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
    let cfg = config::load();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("client");

    // Discover models up front (live /models/info, snapshot fallback).
    let models = provider::discover_models(&client, &cfg.base_url).await;
    let logger = Logger::new(cfg.debug_log.as_deref());
    logger.log("init", json!({ "workspace": cfg.workspace.display().to_string(), "base_url": cfg.base_url, "approval": cfg.approval.as_str() }));

    // Resume session if configured and present.
    let resumed: Vec<Value> = cfg
        .session_file
        .as_ref()
        .map(|p| session::load(p.as_path()))
        .unwrap_or_default();

    // Ensure the session file exists (header only) so the active session is
    // always listed by `list_sessions`, even before the first message lands.
    if let Some(p) = cfg.session_file.as_ref() {
        session::ensure(p.as_path());
    }

    let state = Arc::new(State {
        cfg: RwLock::new(cfg),
        api_key: RwLock::new(None),
        conversation: Mutex::new(resumed),
        models: RwLock::new(models),
        current: Mutex::new(None),
        handle: Mutex::new(None),
        pending: Mutex::new(None),
        logger,
        tokens_in: Mutex::new(0),
        tokens_out: Mutex::new(0),
        escalated_kinds: Mutex::new(HashSet::new()),
        queued: Mutex::new(None),
        plugin_manager: PluginManager::new(PathBuf::from(".umans-harness/plugins")),
        last_turn_time: Mutex::new(std::time::Instant::now()),
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
                let authed = state.api_key.read().await.is_some();
                let cfg = state.cfg.read().await;
                let conv_len = state.conversation.lock().await.len();
                emit(&Event::new("ready")
                        .with("models", json!(models))
                        .with("authed", json!(authed))
                        .with("workspace", json!(cfg.workspace.display().to_string()))
                        .with("approval", json!(cfg.approval.as_str()))
                        .with("base_url", json!(cfg.base_url))
                        .with("bash_timeout_secs", json!(cfg.bash_timeout_secs))
                        .with("max_turns", json!(cfg.max_turns))
                        .with("resumed_messages", json!(conv_len)));
                // Replay any resumed conversation so the TUI shows prior history
                // on launch instead of starting from an empty transcript.
                if conv_len > 0 {
                    let conv = state.conversation.lock().await;
                    let visible: Vec<&Value> = conv.iter()
                        .filter(|m| m.get("role").and_then(|v| v.as_str()) != Some("system"))
                        .collect();
                    emit(&Event::new("history").with("messages", json!(visible)));
                }
            }
            Command::SetKey { api_key } => {
                *state.api_key.write().await = Some(api_key);
                emit(&Event::new("authed").with("ok", json!(true)));
            }
            Command::SetApproval { mode } => {
                let new = Approval::parse(&mode);
                state.cfg.write().await.approval = new.clone();
                state.logger.log("set_approval", json!({ "mode": new.as_str() }));
                emit(&Event::new("approval_changed").with("mode", json!(new.as_str())));
            }
            Command::SetConfig { key, value } => {
                // ponytail: minimal runtime knob setter for the two values the
                // TUI settings modal edits. Coerce string-or-number to u64.
                let as_u64 = |v: &Value| {
                    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
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
                    "max_turns" => {
                        if let Some(n) = as_u64(&value) {
                            cfg.max_turns = n as usize;
                            out_val = json!(n);
                        }
                    }
                    _ => {
                        drop(cfg);
                        emit(&Event::new("error").with("message", json!(format!("unknown config key: {key}"))));
                        return;
                    }
                }
                state.logger.log("set_config", json!({ "key": out_key, "value": out_val }));
                drop(cfg);
                emit(&Event::new("config_changed").with("key", json!(out_key)).with("value", json!(out_val)));
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
                    if role == "user" { conv.pop(); break; }
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
                    compact_conversation(&mut messages, 200_000);
                    *state.conversation.lock().await = messages.clone();
                    if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                        session::rewrite(p, &messages);
                    }
                    emit(&Event::new("compacted").with("before_tokens", json!(estimate_messages_tokens(&messages))).with("after_tokens", json!(estimate_messages_tokens(&messages))));
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
                        let mtime = e.metadata().ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as u64)
                            .unwrap_or(0);
                        let current = current_name
                            .as_ref()
                            .map(|n| *n == e.file_name())
                            .unwrap_or(false);
                        let title = info.title.unwrap_or_else(|| "(no messages yet)".to_string());
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
                    b["mtime"].as_u64().unwrap_or(0).cmp(&a["mtime"].as_u64().unwrap_or(0))
                });
                let files: Vec<String> = entries.iter()
                    .filter_map(|e| e["name"].as_str().map(|s| s.to_string()))
                    .collect();
                emit(&Event::new("sessions")
                    .with("sessions", json!(entries))
                    .with("files", json!(files)));
            }
            Command::LoadSession { path } => {
                let mut p = std::path::PathBuf::from(&path);
                // Resolve relative paths against the sessions dir so the picker
                // (which may send a bare filename) works.
                if !p.is_absolute() {
                    if let Some(sess_dir) = state.cfg.read().await.session_file.as_ref().and_then(|sf| sf.parent()) {
                        p = sess_dir.join(&p);
                    }
                }
                let loaded = session::load(&p);
                *state.conversation.lock().await = loaded.clone();
                // Point the session_file at the loaded path so future appends go there.
                state.cfg.write().await.session_file = Some(p);
                emit(&Event::new("reset"));
                // Replay the loaded transcript so the TUI shows prior turns
                // instead of an empty view after switching/resuming a session.
                let visible: Vec<&Value> = loaded.iter()
                    .filter(|m| m.get("role").and_then(|v| v.as_str()) != Some("system"))
                    .collect();
                emit(&Event::new("history").with("messages", json!(visible)));
                emit(&Event::new("info").with("message", json!(format!("loaded {} messages from {}", loaded.len(), path))));
            }
            Command::NewSession { path } => {
                // Start a fresh session file in the same project dir. The old
                // file is left on disk so sessions accumulate per project.
                let new_path = match path {
                    Some(name) => {
                        let mut p = std::path::PathBuf::from(name);
                        if !p.is_absolute() {
                            if let Some(sess_dir) = state.cfg.read().await.session_file.as_ref().and_then(|sf| sf.parent()) {
                                p = sess_dir.join(&p);
                            }
                        }
                        p
                    }
                    None => {
                        let dir = state.cfg.read().await.session_file.as_ref()
                            .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        dir.join(new_session_filename())
                    }
                };
                session::ensure(&new_path);
                *state.conversation.lock().await = Vec::new();
                state.cfg.write().await.session_file = Some(new_path.clone());
                state.logger.log("new_session", json!({ "path": new_path.display().to_string() }));
                emit(&Event::new("reset"));
                emit(&Event::new("info").with("message", json!(format!("started new session: {}", new_path.display()))));
            }
            Command::Stats => {
                let ti = *state.tokens_in.lock().await;
                let to = *state.tokens_out.lock().await;
                let turns = state.logger.turn_count();
                emit(&Event::new("stats")
                    .with("tokens_in", json!(ti))
                    .with("tokens_out", json!(to))
                    .with("tokens_total", json!(ti + to))
                    .with("turns", json!(turns))
                    .with("messages", json!(state.conversation.lock().await.len())));
            }
            Command::InstallPlugin { path } => {
                let dir = std::path::PathBuf::from(&path);
                match state.plugin_manager.install(&dir) {
                    Ok(plugin) => {
                        let hooks_list: Vec<String> = plugin.hooks.keys().cloned().collect();
                        emit(&Event::new("plugin_installed")
                            .with("name", json!(plugin.name))
                            .with("version", json!(plugin.version))
                            .with("description", json!(plugin.description))
                            .with("hooks", json!(hooks_list))
                            .with("path", json!(plugin.source_path.display().to_string())));
                    }
                    Err(e) => {
                        emit(&Event::new("plugin_error")
                            .with("name", json!(path))
                            .with("message", json!(e)));
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
                let entries: Vec<Value> = plugins.values().map(|p| {
                    let hooks: Vec<String> = p.hooks.keys().cloned().collect();
                    json!({
                        "name": p.name,
                        "version": p.version,
                        "enabled": p.enabled,
                        "description": p.description,
                        "hooks": hooks,
                    })
                }).collect();
                emit(&Event::new("plugins_list").with("plugins", json!(entries)));
            }
            Command::RefreshMemory => {
                let ws = state.cfg.read().await.workspace.clone();
                let mem = memory_injection(&ws, "");
                // Rebuild the system prompt with fresh memory and persist it.
                let mut conv = state.conversation.lock().await;
                let new_system = build_system_prompt(&ws);
                if let Some(first) = conv.first_mut() {
                    if first.get("role").and_then(|v| v.as_str()) == Some("system") {
                        *first = json!({ "role": "system", "content": new_system });
                        if let Some(p) = state.cfg.read().await.session_file.as_ref() {
                            session::rewrite(p, &conv);
                        }
                    }
                }
                drop(conv);
                emit(&Event::new("info").with("message", json!(format!("memory refreshed: {}", if mem.is_empty() { "no memories found" } else { "memories injected" }))));
            }
            Command::Approve { request_id, decision } => {
                let pending = state.pending.lock().await.clone();
                if let Some(p) = pending {
                    if p.request_id == request_id {
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
            }
            Command::Abort => {
                // Cancel the running turn AND drop any queued follow-up/steer so a
                // single abort fully stops the loop (not just the current turn).
                *state.queued.lock().await = None;
                if let Some(tok) = state.current.lock().await.take() {
                    tok.cancel();
                }
            }
            Command::Send { prompt, model, reasoning_effort, images } => {
                let st = state.clone();
                let client = client.clone();
                let models = st.models.read().await.clone();
                let valid = models.iter().any(|m| m.id == model);
                if !valid {
                    emit(&Event::new("error").with("message", json!(format!("unknown model: {model}"))));
                    continue;
                }
                let effort = reasoning_effort.unwrap_or_else(|| "medium".into());
                // If a turn is already running, buffer this prompt (one-deep) instead
                // of dropping it. It drains when the running turn emits `done`.
                let already = st.current.lock().await.is_some();
                if already {
                    let mut q = st.queued.lock().await;
                    if q.is_some() {
                        emit(&Event::new("error").with("message", json!("a prompt is already queued; send abort first or wait")));
                    } else {
                        *q = Some(QueuedPrompt { prompt, model, effort });
                        emit(&Event::new("info").with("message", json!("prompt queued; will run after the current turn")));
                    }
                    continue;
                }
                let tok = CancellationToken::new();
                *st.current.lock().await = Some(tok.clone());
                let handle = tokio::spawn(run_turn_and_drain(
                    st.clone(), client.clone(), model, prompt, effort, images, tok,
                ));
                *st.handle.lock().await = Some(handle);
            }
            Command::Steer { prompt, model, reasoning_effort } => {
                let st = state.clone();
                let client_c = client.clone();
                let models = st.models.read().await.clone();
                if !models.iter().any(|m| m.id == model) {
                    emit(&Event::new("error").with("message", json!(format!("unknown model: {model}"))));
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
                    *st.queued.lock().await = Some(QueuedPrompt { prompt, model, effort });
                    if let Some(tok) = st.current.lock().await.take() {
                        tok.cancel();
                    }
                } else {
                    let tok = CancellationToken::new();
                    *st.current.lock().await = Some(tok.clone());
                    let handle = tokio::spawn(run_turn_and_drain(
                        st.clone(), client_c, model, prompt, effort, None, tok,
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
fn tool_matches_rule(tool_name: &str, args: &Value, rule: &PermissionRule) -> bool {
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
        "write_file" | "edit" | "patch" | "read_file" | "bulk_read" | "bulk_write" | "bulk_edit" => {
            args.get("path").and_then(|v| v.as_str()).unwrap_or("")
        }
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
        run_turn(&st, &client, model, prompt, effort, images, tok).await;
        st.current.lock().await.take();
        // Drain a queued prompt if one was buffered while we ran (follow-up/steer).
        if let Some(q) = st.queued.lock().await.take() {
            let tok2 = CancellationToken::new();
            *st.current.lock().await = Some(tok2.clone());
            tokio::spawn(run_turn_and_drain(
                st.clone(), client.clone(), q.model, q.prompt, q.effort, None, tok2,
            ));
        }
    })
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
    // Ensure system prompt is present; persist every finalized message to the session file.
    {
        let mut conv = st.conversation.lock().await;
        if conv.is_empty() {
            let workspace = st.cfg.read().await.workspace.clone();
            conv.push(json!({ "role": "system", "content": build_system_prompt(&workspace) }));
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
        conv.push(user_msg);
        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
            session::append(p, conv.last().unwrap());
        }
    }

    let tool_defs = tools::definitions();
    let mut turns = 0usize;
    let mut timer = TurnTimer::new();
    let cfg_snap = st.cfg.read().await.clone();
    let max_turns = cfg_snap.max_turns;

    // Idle compaction: if 60+ minutes since the last turn completed, clear old
    // cached content from messages so the next turn starts with a clean slate.
    {
        let last = *st.last_turn_time.lock().await;
        let elapsed = last.elapsed();
        if elapsed.as_secs() > 3600 {
            let mut messages = st.conversation.lock().await.clone();
            if messages.len() > 4 {
                compact_conversation(&mut messages, 200_000);
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                emit(&Event::new("compacted")
                    .with("before_tokens", json!(0))
                    .with("after_tokens", json!(estimate_messages_tokens(&messages))));
            }
        }
    }

    loop {
        if cancel.is_cancelled() {
            emit(&Event::new("aborted"));
            return;
        }
        turns += 1;
        if turns > max_turns {
            emit(&Event::new("error").with("message", json!(format!("exceeded {max_turns} tool turns; raise --max-turns to continue longer"))));
            emit(&Event::new("done"));
            return;
        }

        // Session token budget (hard ceiling across the whole session, not per turn).
        // 0 = unlimited. Trips before the request so we don't blow past a cost cap.
        let budget = st.cfg.read().await.max_session_tokens;
        if budget > 0 {
            let spent = *st.tokens_in.lock().await + *st.tokens_out.lock().await;
            if spent >= budget {
                emit(&Event::new("error").with("message", json!(format!("session token budget exhausted ({spent} >= {budget}); start a new session"))));
                emit(&Event::new("done"));
                return;
            }
        }

        let api_key = {
            let k = st.api_key.read().await.clone();
            match k {
                Some(k) => k,
                None => {
                    emit(&Event::new("error").with("message", json!("no API key set; use set_key first")));
                    emit(&Event::new("done"));
                    return;
                }
            }
        };

        // Context window management: compact if we're over the threshold.
        let mut messages = st.conversation.lock().await.clone();
        let (model_ctx, thinking_levels) = st
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == model)
            .map(|m| (m.context_window as u64, m.thinking_levels.clone()))
            .unwrap_or((200_000, Vec::new()));
        let est = estimate_messages_tokens(&messages);
        let threshold = (model_ctx as f32 * st.cfg.read().await.context_compact_at) as u64;
        if est > threshold && messages.len() > 4 {
            // Hard cap: if even over 95% of the window, force compact regardless of threshold.
            let summarize = st.cfg.read().await.summarize_on_compact;
            if summarize {
                // Split into keep-head + summarize-tail.
                let tail_start = messages.len().saturating_sub(8);
                let to_summarize: Vec<Value> = messages[1..tail_start].to_vec();
                let kept: Vec<Value> = messages[tail_start..].to_vec();
                let summary = if !to_summarize.is_empty() {
                    provider::summarize(client, &st.cfg.read().await.clone(), &api_key, &model, &to_summarize, &cancel).await
                } else { None };
                let mut compacted = vec![messages[0].clone()];
                if let Some(s) = summary {
                    compacted.push(json!({ "role": "system", "content": format!("[Summary of earlier turns]\n{s}") }));
                } else {
                    // summarization failed — fall back to the drop-oldest marker.
                    compacted.push(json!({ "role": "system", "content": "[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped; summarization was unavailable.]" }));
                }
                compacted.extend(kept);
                messages = compacted;
            } else {
                compact_conversation(&mut messages, model_ctx);
            }
            // persist the compacted conversation
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            emit(&Event::new("compacted")
                .with("before_tokens", json!(est))
                .with("after_tokens", json!(estimate_messages_tokens(&messages))));
        }

        // Sanitize orphaned tool calls right before the request (mirrors Umans extension).
        provider::sanitize_orphaned_tool_calls(&mut messages);

        let cfg = st.cfg.read().await.clone();
        let (assistant, _finish, tokens_in, tokens_out) = match provider::stream_turn(
            client, &cfg, &api_key, &model, &messages, &tool_defs, &effort, &thinking_levels, &cancel, &mut timer,
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

        // Accumulate token counts.
        *st.tokens_in.lock().await += tokens_in;
        *st.tokens_out.lock().await += tokens_out;

        // Append + persist the finalized assistant message.
        {
            let mut conv = st.conversation.lock().await;
            conv.push(assistant.clone());
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, conv.last().unwrap());
            }
        }

        let tool_calls = assistant.get("tool_calls").and_then(|v| v.as_array()).cloned();
        match tool_calls {
            Some(calls) if !calls.is_empty() => {
                for tc in &calls {
                    let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let func = tc.get("function");
                    let name = func.and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let args_str = func.and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}").to_string();
                    let args: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
                    emit(&Event::new("tool_call")
                        .with("id", json!(id))
                        .with("name", json!(name))
                        .with("args", json!(args_str)));

                    // Approval gate for destructive tools.
                    let cfg = st.cfg.read().await.clone();
                    let kind = tools::classify(&name);
                    let kind_str: &'static str = match kind { tools::ToolKind::ReadOnly => "readonly", tools::ToolKind::Destructive => "destructive" };
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
                        emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(false)).with("output", json!(msg)));
                        let mut conv = st.conversation.lock().await;
                        conv.push(json!({ "role": "tool", "tool_call_id": id, "content": msg }));
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
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
                                emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(false)).with("output", json!(msg)));
                                let mut conv = st.conversation.lock().await;
                                conv.push(json!({ "role": "tool", "tool_call_id": id, "content": msg }));
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
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
                        emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(false)).with("output", json!(msg)));
                        let mut conv = st.conversation.lock().await;
                        conv.push(json!({ "role": "tool", "tool_call_id": id, "content": msg }));
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        continue;
                    }

                    // Dispatch pre-execution hooks for this tool.
                    let hook_name = match name.as_str() {
                        "bash" => "pre_bash",
                        "write_file" | "edit" => "pre_write",
                        "read_file" | "grep" | "glob" => "pre_read",
                        _ => "",
                    };
                    let mut modified_args: Option<Value> = None;
                    if !hook_name.is_empty() {
                        let configs = st.plugin_manager.get_hook_configs(hook_name);
                        for (plugin_name, config) in &configs {
                            let session_id = cfg.session_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            let ctx = plugins::build_context(
                                hook_name, &name, &cfg.workspace.display().to_string(),
                                Some(&args), &session_id, config.pass_args,
                            );
                            let result = plugins::execute_hook(hook_name, plugin_name, config, &ctx).await;
                            if !result.allow {
                                let msg = format!("tool call '{}' denied by plugin '{}' hook '{}': {}", name, plugin_name, hook_name, result.reason);
                                emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(false)).with("output", json!(msg)));
                                let mut conv = st.conversation.lock().await;
                                conv.push(json!({ "role": "tool", "tool_call_id": id, "content": msg }));
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
                                // Break out: save the denied state and skip execution.
                                // We need a flag to skip the tool execution below.
                            }
                            if let Some(ref modify) = result.modify {
                                modified_args = Some(modify.clone());
                            }
                        }
                    }

                    // Check if any pre-hook denied (ugly but we can't break the outer loop from here).
                    // Track denied state.
                    let mut denied_by_hook = false;
                    if !hook_name.is_empty() {
                        let configs = st.plugin_manager.get_hook_configs(hook_name);
                        for (plugin_name, config) in &configs {
                            let session_id = cfg.session_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            let ctx = plugins::build_context(
                                hook_name, &name, &cfg.workspace.display().to_string(),
                                Some(&args), &session_id, config.pass_args,
                            );
                            let result = plugins::execute_hook(hook_name, plugin_name, config, &ctx).await;
                            if !result.allow {
                                denied_by_hook = true;
                                let msg = format!("tool call '{}' denied by plugin '{}' hook '{}': {}", name, plugin_name, hook_name, result.reason);
                                emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(false)).with("output", json!(msg)));
                                let mut conv = st.conversation.lock().await;
                                conv.push(json!({ "role": "tool", "tool_call_id": id, "content": msg }));
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
                                break;
                            }
                            if let Some(ref modify) = result.modify {
                                modified_args = Some(modify.clone());
                            }
                        }
                    }
                    if denied_by_hook {
                        continue;
                    }

                    // Use modified args if a hook provided them.
                    let exec_args = modified_args.as_ref().unwrap_or(&args);

                    // Execute. bash/bulk/diagnostics/spawn are async; others sync.
                    let outcome = if name == "bash" {
                        let cmd = exec_args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                        tools::execute_bash(cmd, &cfg).await
                    } else if name == "bulk" {
                        tools::execute_bulk(exec_args, &cfg).await
                    } else if name == "diagnostics" {
                        tools::execute_diagnostics(exec_args, &cfg).await
                    } else if name == "spawn" {
                        let sub_prompt = exec_args.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let sub_model = exec_args.get("model").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| model.clone());
                        let sub_max = st.cfg.read().await.spawn_max_turns;
                        run_spawn(st, client, &api_key, &sub_model, &sub_prompt, sub_max, &cancel).await
                    } else {
                        tools::execute(&name, exec_args, &cfg)
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
                            let session_id = cfg.session_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            let ctx = plugins::build_context(
                                post_hook, &name, &cfg.workspace.display().to_string(),
                                Some(exec_args), &session_id, config.pass_args,
                            );
                            let _ = plugins::execute_hook(post_hook, plugin_name, config, &ctx).await;
                        }
                    }

                    // finish sentinel: the model signaled completion.
                    if name == "finish" && outcome.ok && outcome.output == "__finish__" {
                        *st.last_turn_time.lock().await = std::time::Instant::now();
                        let metrics = timer.finalize(*st.tokens_in.lock().await, *st.tokens_out.lock().await, model.clone());
                        emit(&Event::new("metrics")
                            .with("ttft_ms", json!(metrics.ttft_ms))
                            .with("elapsed_ms", json!(metrics.elapsed_ms))
                            .with("tokens_in", json!(metrics.tokens_in))
                            .with("tokens_out", json!(metrics.tokens_out))
                            .with("tps", json!(metrics.tps))
                            .with("model", json!(metrics.model)));
                        st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "ttft_ms": metrics.ttft_ms, "tps": metrics.tps, "finish_tool": true }));
                        st.logger.record_turn();
                        emit(&Event::new("done"));
                        return;
                    }
                    st.logger.log("tool", json!({ "name": name, "args": args_str, "ok": outcome.ok, "output_len": outcome.output.len() }));
                    emit(&Event::new("tool_result")
                        .with("id", json!(id))
                        .with("ok", json!(outcome.ok))
                        .with("output", json!(outcome.output)));
                    let mut conv = st.conversation.lock().await;
                    conv.push(json!({ "role": "tool", "tool_call_id": id, "content": outcome.output }));
                    if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                        session::append(p, conv.last().unwrap());
                    }
                }
                // Loop back for the model to continue.
            }
            _ => {
                // Turn complete: emit metrics + done.
                *st.last_turn_time.lock().await = std::time::Instant::now();
                let metrics = timer.finalize(*st.tokens_in.lock().await, *st.tokens_out.lock().await, model.clone());
                emit(&Event::new("metrics")
                    .with("ttft_ms", json!(metrics.ttft_ms))
                    .with("elapsed_ms", json!(metrics.elapsed_ms))
                    .with("tokens_in", json!(metrics.tokens_in))
                    .with("tokens_out", json!(metrics.tokens_out))
                    .with("tps", json!(metrics.tps))
                    .with("model", json!(metrics.model)));
                st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "ttft_ms": metrics.ttft_ms, "tps": metrics.tps }));
                st.logger.record_turn();
                emit(&Event::new("done"));
                return;
            }
        }
    }
}

enum ApprovalResult {
    Granted,
    Denied,
    Aborted,
}

/// Ask the TUI to approve a tool call; block until answered or aborted.
/// On "always", only the matched tool KIND is escalated (not the whole session).
async fn request_approval(st: &Arc<State>, id: &str, name: &str, args: &str, kind_str: &'static str, cancel: &CancellationToken) -> ApprovalResult {
    let request_id = id.to_string();
    let notify = Arc::new(Notify::new());
    let pending = Arc::new(PendingApproval {
        request_id: request_id.clone(),
        tool: name.to_string(),
        args: serde_json::from_str(args).unwrap_or(json!({})),
        notify: notify.clone(),
        granted: Mutex::new(None),
        escalated: Mutex::new(false),
    });

    *st.pending.lock().await = Some(pending.clone());
    emit(&Event::new("approval_request")
        .with("request_id", json!(request_id))
        .with("tool", json!(name))
        .with("args", json!(args)));

    // Wait for the approve command or abort.
    let granted = tokio::select! {
        _ = notify.notified() => pending.granted.lock().await.unwrap_or(false),
        _ = cancel.cancelled() => {
            *st.pending.lock().await = None;
            return ApprovalResult::Aborted;
        }
    };

    // "always" escalates: record this tool KIND so subsequent calls of the same
    // kind skip the gate, without un-gating other kinds or the whole session.
    if *pending.escalated.lock().await {
        st.escalated_kinds.lock().await.insert(kind_str);
        emit(&Event::new("approval_changed").with("mode", json!(format!("{}:always", kind_str))));
    }
    *st.pending.lock().await = None;
    if granted {
        ApprovalResult::Granted
    } else {
        ApprovalResult::Denied
    }
}

/// Compact the conversation when it nears the context window.
/// ponytail: simple strategy — drop the oldest tool results (the bulk of tokens)
/// and keep system + recent turns. A summarization call would be better but adds
/// cost+complexity; this keeps the agent unblocked. Orphaned-tool-call sanitizer
/// in provider.rs backstops any tool_call/result mismatch this creates.
fn compact_conversation(messages: &mut Vec<Value>, _ctx: u64) {
    // Keep: system (first), and the last ~10 messages verbatim.
    // Drop tool messages (role == "tool") in the middle band to reclaim tokens.
    if messages.len() <= 12 {
        return;
    }
    let system = messages[0].clone();
    let tail_start = messages.len().saturating_sub(10);
    let tail: Vec<Value> = messages[tail_start..].to_vec();
    let mut compacted = vec![system];
    // Insert a marker so the model knows history was trimmed.
    compacted.push(json!({ "role": "system", "content": "[Earlier conversation history was compacted to fit the context window. Tool results from prior turns were dropped.]" }));
    compacted.extend(tail);
    *messages = compacted;
}

/// Run a nested agentic sub-turn with a fresh sub-conversation. Shares the
/// workspace, tools, and api key but cannot spawn further sub-agents (depth 1).
/// Returns the final assistant text as an Outcome (for the spawn tool).
async fn run_spawn(
    st: &Arc<State>,
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    prompt: &str,
    max_turns: usize,
    cancel: &CancellationToken,
) -> tools::Outcome {
    use tools::Outcome;
    let workspace = st.cfg.read().await.workspace.clone();
    let mut sub: Vec<Value> = vec![
        json!({ "role": "system", "content": build_system_prompt(&workspace) }),
        json!({ "role": "user", "content": prompt }),
    ];
    let tool_defs = tools::definitions();
    let cfg = st.cfg.read().await.clone();
    let effort = "medium".to_string();
    // Respect the sub-model's advertised thinking levels (clamp effort like the main loop).
    let thinking_levels = st
        .models
        .read()
        .await
        .iter()
        .find(|m| m.id == model)
        .map(|m| m.thinking_levels.clone())
        .unwrap_or_default();
    let mut timer = TurnTimer::new();
    for turn in 0..=max_turns {
        if cancel.is_cancelled() { return Outcome::ok("[spawn aborted]"); }
        provider::sanitize_orphaned_tool_calls(&mut sub);
        let (assistant, _finish, _ti, _to) = match provider::stream_turn(
            client, &cfg, api_key, model, &sub, &tool_defs, &effort, &thinking_levels, cancel, &mut timer,
        ).await {
            Ok(v) => v,
            Err(e) => return Outcome::err(format!("spawn stream error: {e}")),
        };
        sub.push(assistant.clone());
        let Some(calls) = assistant.get("tool_calls").and_then(|v| v.as_array()).cloned() else {
            // No more tool calls — return the final assistant text.
            let text = assistant.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            return Outcome::ok(text);
        };
        if calls.is_empty() {
            let text = assistant.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            return Outcome::ok(text);
        }
        // Execute each tool call in the sub-context. spawn/diagnostics/bash/bulk
        // are honored; nested spawn is refused (depth 1).
        for tc in &calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let func = tc.get("function");
            let name = func.and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args_str = func.and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}").to_string();
            let args: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
            emit(&Event::new("tool_call").with("id", json!(id)).with("name", json!(format!("spawn:{name}"))).with("args", json!(args_str)));
            let outcome = if name == "spawn" {
                Outcome::err("nested spawn is not allowed (max depth 1)")
            } else if name == "bash" {
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                tools::execute_bash(cmd, &cfg).await
            } else if name == "bulk" {
                tools::execute_bulk(&args, &cfg).await
            } else if name == "diagnostics" {
                tools::execute_diagnostics(&args, &cfg).await
            } else {
                tools::execute(&name, &args, &cfg)
            };
            emit(&Event::new("tool_result").with("id", json!(id)).with("ok", json!(outcome.ok)).with("output", json!(outcome.output.clone())));
            sub.push(json!({ "role": "tool", "tool_call_id": id, "content": outcome.output }));
        }
        let _ = turn;
    }
    Outcome::ok(format!("[spawn reached max_turns ({max_turns}); returning last state]"))
}

/// Turn an image reference into a data URL. Accepts:
/// - an existing data URL (data:image/...;base64,...) → passthrough
/// - an absolute or workspace-relative file path → read + base64-encode
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
            } else {
                "application/octet-stream"
            };
            let b64 = base64_encode(&bytes);
            format!("data:{mime};base64,{b64}")
        }
        Err(e) => format!("data:text/plain;base64,{}", base64_encode(format!("image read failed: {e}").as_bytes())),
    }
}

/// Minimal base64 encoder (no extra crate).
fn base64_encode(input: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i+1] as u32) << 8) | (input[i+2] as u32);
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
        let n = ((input[i] as u32) << 16) | ((input[i+1] as u32) << 8);
        out.push(T[((n >> 18) & 0x3f) as usize] as char);
        out.push(T[((n >> 12) & 0x3f) as usize] as char);
        out.push(T[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}
