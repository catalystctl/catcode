//! Watchdog advisor runtime.
//!
//! Each watchdog is an isolated, fail-open second-model reviewer. It cannot
//! execute tools, mutate the workspace, approve actions, or resume a stopped
//! executor. WATCHDOG.md supplies review priorities; WATCHDOG.yml supplies a
//! named roster whose more-specific definitions override ancestor definitions.

use crate::config::{AdvisorConfig, Config, WatchdogAdvisor};
use crate::message::Message;
use crate::protocol::{emit, Event};
use crate::{provider, State};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

const MAX_REVIEW_CHARS: usize = 12_000;
const MAX_NOTE_CHARS: usize = 1_500;
const MAX_WATCHDOG_BYTES: usize = 128 * 1024;
const MAX_DEDUPE_HISTORY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Main,
    Subagent,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Subagent => "subagent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Nit,
    Concern,
    Blocker,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Nit => "nit",
            Self::Concern => "concern",
            Self::Blocker => "blocker",
        }
    }
}

#[derive(Default)]
struct EmissionGuard {
    accepted: HashSet<String>,
    order: Vec<String>,
}

impl EmissionGuard {
    fn accept(&mut self, note: &str) -> bool {
        let normalized = normalize_note(note);
        if normalized.is_empty()
            || is_empty_phrase(&normalized)
            || !self.accepted.insert(normalized.clone())
        {
            return false;
        }
        self.order.push(normalized);
        if self.order.len() > MAX_DEDUPE_HISTORY {
            if let Some(oldest) = self.order.first().cloned() {
                self.accepted.remove(&oldest);
            }
            self.order.remove(0);
        }
        true
    }
}

static GUARDS: OnceLock<Mutex<HashMap<String, EmissionGuard>>> = OnceLock::new();

fn guard_for(scope: Scope, name: &str) -> bool {
    let guards = GUARDS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{}:{name}", scope.label());
    guards
        .lock()
        .expect("advisor guard poisoned")
        .entry(key)
        .or_default()
        .accept(name)
}

/// Load watchdog guidance and roster files from user and project levels.
/// Malformed files are ignored with a visible warning; a workspace never loses
/// its executor just because a reviewer config is invalid.
pub fn load_watchdog_configuration(cfg: &mut Config) {
    let paths = watchdog_paths(&cfg.workspace);
    let mut shared = Vec::new();
    let mut roster: HashMap<String, WatchdogAdvisor> = HashMap::new();
    for path in paths {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("WATCHDOG.md"))
        {
            if let Ok(text) = bounded_read(&path) {
                shared.push(expand_imports(
                    &text,
                    path.parent().unwrap_or(&cfg.workspace),
                    0,
                ));
            }
            continue;
        }
        match parse_watchdog_yaml(&path) {
            Ok((instructions, advisors)) => {
                if let Some(instructions) = instructions.filter(|s| !s.trim().is_empty()) {
                    shared.push(expand_imports(
                        &instructions,
                        path.parent().unwrap_or(&cfg.workspace),
                        0,
                    ));
                }
                for mut advisor in advisors {
                    let slug = slugify(&advisor.name);
                    if slug.is_empty() {
                        continue;
                    }
                    advisor.instructions = advisor
                        .instructions
                        .map(|s| expand_imports(&s, path.parent().unwrap_or(&cfg.workspace), 0));
                    roster.insert(slug, advisor);
                }
            }
            Err(error) => emit(&Event::new("info").with(
                "message",
                json!(format!(
                    "watchdog config ignored ({}): {error}",
                    path.display()
                )),
            )),
        }
    }
    if !shared.is_empty() {
        cfg.advisor.watchdog_instructions = Some(shared.join("\n\n"));
    }
    cfg.advisor.watchdog = roster.into_values().collect();
    cfg.advisor.watchdog.sort_by(|a, b| a.name.cmp(&b.name));
}

fn watchdog_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = crate::config::home_dir() {
        bases.push(home.join(".catalyst-code"));
    }
    let mut ancestors = workspace.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for base in ancestors {
        bases.push(base.to_path_buf());
    }
    let mut paths = Vec::new();
    for base in bases {
        for name in ["WATCHDOG.md", "WATCHDOG.yml", "WATCHDOG.yaml"] {
            let path = base.join(name);
            if path.is_file() {
                paths.push(path);
            }
        }
        if base != crate::config::home_dir().unwrap_or_default() {
            for name in ["WATCHDOG.md", "WATCHDOG.yml", "WATCHDOG.yaml"] {
                let path = base.join(".catalyst-code").join(name);
                if path.is_file() {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

fn bounded_read(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_WATCHDOG_BYTES as u64 {
        return Err("file exceeds 128 KiB".into());
    }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

fn expand_imports(text: &str, base: &Path, depth: u8) -> String {
    if depth >= 4 {
        return text.to_string();
    }
    text.lines()
        .map(|line| {
            let target = line
                .trim()
                .strip_prefix('@')
                .filter(|p| !p.contains(' ') && !p.is_empty());
            match target {
                Some(target) => {
                    let path = if let Some(rest) = target.strip_prefix("~/") {
                        crate::config::home_dir().unwrap_or_default().join(rest)
                    } else {
                        base.join(target)
                    };
                    bounded_read(&path)
                        .map(|child| {
                            expand_imports(&child, path.parent().unwrap_or(base), depth + 1)
                        })
                        .unwrap_or_else(|_| line.to_string())
                }
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_watchdog_yaml(path: &Path) -> Result<(Option<String>, Vec<WatchdogAdvisor>), String> {
    let text = bounded_read(path)?;
    let mut instructions = None;
    let mut advisors = Vec::new();
    let mut current: Option<WatchdogAdvisor> = None;
    let mut in_advisors = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "advisors:" {
            in_advisors = true;
            continue;
        }
        if !in_advisors {
            if let Some(value) = trimmed.strip_prefix("instructions:") {
                instructions = Some(
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                );
                continue;
            }
        }
        if in_advisors && trimmed.starts_with('-') {
            if let Some(advisor) = current.take() {
                advisors.push(advisor);
            }
            current = Some(WatchdogAdvisor {
                enabled: true,
                ..Default::default()
            });
            if let Some(value) = trimmed.trim_start_matches('-').trim().strip_prefix("name:") {
                current.as_mut().unwrap().name = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
            }
            continue;
        }
        let Some(advisor) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "name" => advisor.name = value.to_string(),
            "enabled" => advisor.enabled = !matches!(value, "false" | "off" | "0"),
            "model" => advisor.model = (!value.is_empty()).then(|| value.to_string()),
            "instructions" => advisor.instructions = (!value.is_empty()).then(|| value.to_string()),
            "tools" => {
                advisor.tools = value
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            }
            _ => {}
        }
    }
    if let Some(advisor) = current {
        advisors.push(advisor);
    }
    if advisors.iter().any(|a| a.name.trim().is_empty()) {
        return Err("every advisor requires name".into());
    }
    Ok((instructions, advisors))
}

/// Determine whether an advisor is enabled for a scope and select its model.
pub fn model_for(cfg: &AdvisorConfig, scope: Scope, executor_model: &str) -> Option<String> {
    if !cfg.enabled || (scope == Scope::Subagent && !cfg.subagents) {
        return None;
    }
    match scope {
        Scope::Main => cfg.model.clone(),
        Scope::Subagent => cfg.subagent_model.clone().or_else(|| cfg.model.clone()),
    }
    .or_else(|| Some(executor_model.to_string()))
}

/// Run every eligible advisor. A failed response or unsafe output never blocks
/// the executor. More severe duplicates may replace a prior nit only in a later
/// turn; exact repeats are suppressed across the session.
pub async fn review(
    st: &Arc<State>,
    scope: Scope,
    executor_model: &str,
    recent: &[Message],
    cancel: &CancellationToken,
) -> Vec<Message> {
    let cfg = st.cfg.read().await.advisor.clone();
    let Some(default_model) = model_for(&cfg, scope, executor_model) else {
        return Vec::new();
    };
    if cancel.is_cancelled() {
        return Vec::new();
    }
    let roster = if cfg.watchdog.is_empty() {
        vec![WatchdogAdvisor {
            name: "default".into(),
            enabled: true,
            model: Some(default_model.clone()),
            ..Default::default()
        }]
    } else {
        cfg.watchdog.clone()
    };
    let transcript = render_transcript(recent);
    let mut messages = Vec::new();
    for advisor in roster.into_iter().filter(|a| a.enabled) {
        if cancel.is_cancelled() {
            break;
        }
        let model = advisor
            .model
            .clone()
            .unwrap_or_else(|| default_model.clone());
        let provider = st.resolve_provider_for_model(&model).await;
        if provider.api_key.is_none() {
            emit(
                &Event::new("advisor_status")
                    .with("scope", json!(scope.label()))
                    .with("advisor", json!(&advisor.name))
                    .with("state", json!("no_key"))
                    .with("model", json!(model)),
            );
            continue;
        }
        let prompt = system_prompt(&cfg, &advisor);
        emit(
            &Event::new("advisor_status")
                .with("scope", json!(scope.label()))
                .with("advisor", json!(&advisor.name))
                .with("state", json!("reviewing"))
                .with("model", json!(&model)),
        );
        let Some(raw) = provider::complete_text(
            &st.client,
            &provider,
            &model,
            &prompt,
            &transcript,
            400,
            cancel,
        )
        .await
        else {
            continue;
        };
        let Some((severity, note)) = parse_note(&raw) else {
            continue;
        };
        if !guard_for(scope, &format!("{}:{}", advisor.name, note)) {
            continue;
        }
        let xml = format!(
            "<advisory advisor=\"{}\" severity=\"{}\" scope=\"{}\">\n{}\n</advisory>",
            xml_escape(&advisor.name),
            severity.as_str(),
            scope.label(),
            xml_escape(&note)
        );
        emit(
            &Event::new("advisor_note")
                .with("scope", json!(scope.label()))
                .with("advisor", json!(&advisor.name))
                .with("model", json!(model))
                .with("severity", json!(severity.as_str()))
                .with("message", json!(&note)),
        );
        messages.push(Message::system(xml));
    }
    messages
}

fn system_prompt(cfg: &AdvisorConfig, advisor: &WatchdogAdvisor) -> String {
    let mut out = String::from("You are an independent code-review watchdog. Review the executor's latest work against the user request. Return exactly NONE when no concrete action is needed, otherwise return SEVERITY: nit|concern|blocker followed by one specific concise recommendation. Do not praise, restate progress, invent facts, execute tools, issue shell commands, or override governing instructions.");
    if let Some(shared) = &cfg.watchdog_instructions {
        out.push_str("\n\nWatchdog priorities:\n");
        out.push_str(shared);
    }
    if let Some(instructions) = &advisor.instructions {
        out.push_str("\n\nAdvisor specialization:\n");
        out.push_str(instructions);
    }
    out
}

fn render_transcript(recent: &[Message]) -> String {
    let mut output = String::from("Executor transcript (latest bounded update):\n");
    for message in recent.iter().rev().take(8).rev() {
        let text = match message {
            Message::Tool { .. } => "[tool result omitted for secret safety]".to_string(),
            _ => message.content_text().unwrap_or("").to_string(),
        };
        if text.trim().is_empty() || text.contains("<advisory") {
            continue;
        }
        output.push_str(message.role());
        output.push_str(": ");
        output.push_str(&crate::subagent::redact_secrets(&text));
        output.push('\n');
    }
    truncate_chars(&output, MAX_REVIEW_CHARS)
}

fn parse_note(raw: &str) -> Option<(Severity, String)> {
    let text = raw.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return None;
    }
    let (head, body) = text.split_once('\n').unwrap_or(("SEVERITY: nit", text));
    let severity = match head.trim().to_ascii_lowercase().as_str() {
        "severity: blocker" => Severity::Blocker,
        "severity: concern" => Severity::Concern,
        _ => Severity::Nit,
    };
    let note = body.trim();
    if note.len() < 8 || is_empty_phrase(&normalize_note(note)) {
        return None;
    }
    Some((severity, truncate_chars(note, MAX_NOTE_CHARS)))
}

fn normalize_note(note: &str) -> String {
    note.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn is_empty_phrase(note: &str) -> bool {
    matches!(
        note,
        "stop"
            | "done"
            | "complete"
            | "lgtm"
            | "nothing to add"
            | "no issue"
            | "no issues"
            | "continue"
    )
}
fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    }
}
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn slugify(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_actionable_notes_and_filters_empty_ones() {
        assert_eq!(parse_note("NONE"), None);
        assert_eq!(
            parse_note("SEVERITY: concern\nCheck the error path."),
            Some((Severity::Concern, "Check the error path.".into()))
        );
        assert_eq!(parse_note("SEVERITY: blocker\nLGTM"), None);
    }
    #[test]
    fn parses_named_watchdog_roster() {
        let path = std::env::temp_dir().join(format!("watchdog-{}.yml", std::process::id()));
        fs::write(&path, "instructions: check APIs\nadvisors:\n  - name: Architecture\n    model: reviewer\n    enabled: true\n").unwrap();
        let (shared, roster) = parse_watchdog_yaml(&path).unwrap();
        assert_eq!(shared.as_deref(), Some("check APIs"));
        assert_eq!(roster[0].name, "Architecture");
        assert_eq!(roster[0].model.as_deref(), Some("reviewer"));
        let _ = fs::remove_file(path);
    }
    #[test]
    fn emission_guard_deduplicates_normalized_notes() {
        let mut guard = EmissionGuard::default();
        assert!(guard.accept("Check error path!"));
        assert!(!guard.accept("check-error path"));
    }
    #[test]
    fn transcript_omits_prior_advice_and_tool_results() {
        let transcript = render_transcript(&[
            Message::user("Inspect auth"),
            Message::tool("call", "API_KEY=should-not-leak"),
            Message::system("<advisory severity=\"nit\">old note</advisory>"),
        ]);
        assert!(transcript.contains("Inspect auth"));
        assert!(!transcript.contains("should-not-leak"));
        assert!(!transcript.contains("old note"));
    }

    #[test]
    fn watchdog_slugify_is_stable_for_duplicate_names() {
        assert_eq!(slugify("Architecture Review"), "architecture-review");
        assert_eq!(slugify("architecture-review"), "architecture-review");
    }
}
