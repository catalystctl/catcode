use super::{RunContext, RuntimeCoordinator, SessionContext};
use crate::protocol::Event;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::sync::{Arc, Mutex, RwLock, Weak};

tokio::task_local! {
    static RUN_CONTEXT: RunContext;
    static SESSION_CONTEXT: SessionContext;
}

pub async fn scope_session<F>(context: SessionContext, future: F) -> F::Output
where
    F: Future,
{
    SESSION_CONTEXT.scope(context, future).await
}

/// Associate all direct emissions in `future` with one run. Spawned child tasks
/// must establish their own scope explicitly; Tokio task locals intentionally
/// do not leak into detached work.
pub async fn scope_run<F>(context: RunContext, future: F) -> F::Output
where
    F: Future,
{
    RUN_CONTEXT.scope(context, future).await
}

/// Current task-local run identity for child ownership/tracing. Tokio tasks do
/// not inherit this automatically, so orchestrators must pass the returned ID
/// explicitly when spawning descendants.
pub fn current_run_id() -> Option<String> {
    RUN_CONTEXT
        .try_with(|context| context.run_id().to_string())
        .ok()
}

pub fn current_session_id() -> Option<String> {
    RUN_CONTEXT
        .try_with(|context| context.session_id().to_string())
        .ok()
        .or_else(|| {
            SESSION_CONTEXT
                .try_with(|context| context.session_id().to_string())
                .ok()
        })
}

/// Central JSONL event authority. It attaches protocol/lifecycle metadata,
/// sequences run events, redacts sensitive fields, and rejects stale work.
pub struct EventSink {
    runtime: RwLock<Weak<RuntimeCoordinator>>,
    sequences: Mutex<HashMap<String, u64>>,
}

impl Default for EventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink {
    pub fn new() -> Self {
        Self {
            runtime: RwLock::new(Weak::new()),
            sequences: Mutex::new(HashMap::new()),
        }
    }

    pub fn install_runtime(&self, runtime: &Arc<RuntimeCoordinator>) {
        *self
            .runtime
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::downgrade(runtime);
        self.sequences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// Prepare an event for the wire. `None` means it came from a stale run and
    /// must not become user-visible.
    pub fn prepare(&self, event: &Event) -> Option<Event> {
        let runtime = self
            .runtime
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .upgrade();
        let context = RUN_CONTEXT.try_with(Clone::clone).ok();
        let session_context = SESSION_CONTEXT.try_with(Clone::clone).ok();

        if let (Some(runtime), Some(context)) = (&runtime, &context) {
            if !runtime.is_active(context) {
                runtime.note_stale_result();
                return None;
            }
        }
        if context.is_none() {
            if let (Some(runtime), Some(session)) = (&runtime, &session_context) {
                if !runtime.is_session_active(session) {
                    runtime.note_stale_result();
                    return None;
                }
            }
        }

        let mut prepared = Event {
            kind: event.kind,
            data: event.data.clone(),
        };
        if prepared.kind == "tool_result" && !prepared.data.contains_key("status") {
            let ok = prepared
                .data
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let output = prepared
                .data
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default();
            prepared.data.insert(
                "status".into(),
                Value::String(
                    crate::tooling::ToolResultStatus::from_legacy(ok, output)
                        .as_str()
                        .into(),
                ),
            );
        }
        prepared
            .data
            .entry("protocol_version")
            .or_insert_with(|| Value::from(crate::protocol::PROTOCOL_VERSION));

        if let Some(context) = context {
            prepared
                .data
                .entry("session_id")
                .or_insert_with(|| Value::String(context.session_id().to_string()));
            prepared
                .data
                .entry("run_id")
                .or_insert_with(|| Value::String(context.run_id().to_string()));
            let sequence = {
                let mut sequences = self
                    .sequences
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let value = sequences.entry(context.run_id().to_string()).or_insert(0);
                *value = value.saturating_add(1);
                *value
            };
            prepared
                .data
                .insert("sequence".into(), Value::from(sequence));
        } else if let Some(session) = session_context {
            prepared
                .data
                .entry("session_id")
                .or_insert_with(|| Value::String(session.session_id().to_string()));
        } else if let Some(runtime) = runtime {
            prepared
                .data
                .entry("session_id")
                .or_insert_with(|| Value::String(runtime.session_id().to_string()));
        }

        redact_map(&mut prepared.data);
        Some(prepared)
    }

    pub fn emit(&self, event: &Event) {
        let Some(prepared) = self.prepare(event) else {
            return;
        };
        let mut line = serde_json::to_string(&prepared).unwrap_or_else(|_| "{}".into());
        line.push('\n');
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.flush();
    }
}

fn redact_map(map: &mut Map<String, Value>) {
    for (key, value) in map {
        if is_sensitive_key(key) {
            *value = Value::String("[REDACTED]".into());
        } else {
            redact_value(value);
        }
    }
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => redact_map(map),
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key"
            | "authorization"
            | "password"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "client_secret"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::CancellationReason;
    use serde_json::json;

    #[tokio::test]
    async fn attaches_version_identity_and_monotonic_sequence() {
        let runtime = Arc::new(RuntimeCoordinator::new());
        let sink = Arc::new(EventSink::new());
        sink.install_runtime(&runtime);
        let run = runtime.start_run();
        scope_run(run.clone(), async {
            let first = sink.prepare(&Event::new("delta")).unwrap();
            let second = sink.prepare(&Event::new("done")).unwrap();
            assert_eq!(first.data["protocol_version"], json!(2));
            assert_eq!(first.data["session_id"], json!(run.session_id()));
            assert_eq!(first.data["run_id"], json!(run.run_id()));
            assert_eq!(first.data["sequence"], json!(1));
            assert_eq!(second.data["sequence"], json!(2));
        })
        .await;
    }

    #[tokio::test]
    async fn rejects_event_after_session_replacement() {
        let runtime = Arc::new(RuntimeCoordinator::new());
        let sink = Arc::new(EventSink::new());
        sink.install_runtime(&runtime);
        let old = runtime.start_run();
        runtime.replace_session(CancellationReason::NewSession);
        scope_run(old, async {
            assert!(sink.prepare(&Event::new("delta")).is_none());
        })
        .await;
        assert_eq!(runtime.snapshot().discarded_stale_results, 1);
    }

    #[tokio::test]
    async fn rejects_background_event_from_replaced_session() {
        let runtime = Arc::new(RuntimeCoordinator::new());
        let sink = Arc::new(EventSink::new());
        sink.install_runtime(&runtime);
        let old_session = runtime.session_context();
        runtime.replace_session(CancellationReason::LoadSession);
        scope_session(old_session, async {
            assert!(sink.prepare(&Event::new("subagent_done")).is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn recursively_redacts_secret_fields() {
        let runtime = Arc::new(RuntimeCoordinator::new());
        let sink = EventSink::new();
        sink.install_runtime(&runtime);
        let event = Event::new("info").with(
            "nested",
            json!({"api_key":"secret", "tokens_in":12, "items":[{"password":"pw"}]}),
        );
        let prepared = sink.prepare(&event).unwrap();
        assert_eq!(prepared.data["nested"]["api_key"], "[REDACTED]");
        assert_eq!(prepared.data["nested"]["tokens_in"], 12);
        assert_eq!(
            prepared.data["nested"]["items"][0]["password"],
            "[REDACTED]"
        );
    }

    #[test]
    fn tool_results_receive_stable_status() {
        let runtime = Arc::new(RuntimeCoordinator::new());
        let sink = EventSink::new();
        sink.install_runtime(&runtime);
        let denied = Event::new("tool_result")
            .with("ok", json!(false))
            .with("output", json!("tool call was denied by the user"));
        assert_eq!(sink.prepare(&denied).unwrap().data["status"], "denied");
        let success = Event::new("tool_result")
            .with("ok", json!(true))
            .with("output", json!("done"));
        assert_eq!(sink.prepare(&success).unwrap().data["status"], "success");
    }
}
