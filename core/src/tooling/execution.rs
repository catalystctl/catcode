use crate::config::{Approval, Config};
use crate::protocol::{emit, Event};
use crate::runtime::{ResourceKind, ResourceLease, RunContext, RuntimeCoordinator};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// Optional ownership label for work delegated below the foreground turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParentIdentity {
    Subagent(String),
    Goal(String),
}

/// Narrow event capability handed to tool executors. It retains the central
/// protocol sink's ordering/redaction/stale-run behavior without exposing a
/// second stdout path.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolEventSink;

impl ToolEventSink {
    pub fn emit(&self, event: &Event) {
        emit(event);
    }
}

/// Restricted view over configured credentials. Executors must ask for the
/// specific provider/search credential they need; they cannot enumerate or log
/// the complete secret maps through the context API.
pub struct RestrictedSecrets<'a> {
    config: &'a Config,
}

impl RestrictedSecrets<'_> {
    pub fn provider_key(&self, provider: &str) -> Option<&str> {
        self.config.persisted_keys.get(provider).map(String::as_str)
    }

    pub fn search_key(&self, provider: &str) -> Option<&str> {
        self.config.search_keys.get(provider).map(String::as_str)
    }
}

/// Complete identity and lifecycle capability for one tool call.
///
/// Configuration is owned so blocking executors may safely clone it, but the
/// fields remain private: normal callers use the non-secret accessors below and
/// credentials are available only through [`RestrictedSecrets`].
pub struct ToolExecutionContext {
    run: RunContext,
    tool_call_id: String,
    approval: Approval,
    configuration: Config,
    runtime: Arc<RuntimeCoordinator>,
    events: ToolEventSink,
    parent: Option<ParentIdentity>,
    started_at: Instant,
}

impl ToolExecutionContext {
    pub fn new(
        run: RunContext,
        tool_call_id: impl Into<String>,
        configuration: Config,
        runtime: Arc<RuntimeCoordinator>,
        parent: Option<ParentIdentity>,
    ) -> Self {
        Self {
            approval: configuration.approval.clone(),
            run,
            tool_call_id: tool_call_id.into(),
            configuration,
            runtime,
            events: ToolEventSink,
            parent,
            started_at: Instant::now(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.configuration.workspace
    }

    pub fn session_id(&self) -> &str {
        self.run.session_id().as_str()
    }

    pub fn run_id(&self) -> &str {
        self.run.run_id().as_str()
    }

    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }

    pub fn cancellation(&self) -> &CancellationToken {
        self.run.cancellation()
    }

    pub fn approval(&self) -> &Approval {
        &self.approval
    }

    pub fn events(&self) -> ToolEventSink {
        self.events
    }

    pub fn secrets(&self) -> RestrictedSecrets<'_> {
        RestrictedSecrets {
            config: &self.configuration,
        }
    }

    pub fn parent(&self) -> Option<&ParentIdentity> {
        self.parent.as_ref()
    }

    pub fn is_active(&self) -> bool {
        self.runtime.is_active(&self.run)
    }

    pub fn note_stale_result(&self) {
        self.runtime.note_stale_result();
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn persist_state(&self, state: crate::session::RunState, detail: Option<&str>) {
        let Some(path) = self.configuration.session_file.as_ref() else {
            return;
        };
        let activity_id = format!("{}:tool:{}", self.run_id(), self.tool_call_id);
        crate::session::append_activity_state(
            path,
            self.session_id(),
            &activity_id,
            "tool",
            Some(self.run_id()),
            Some(self.tool_call_id()),
            state,
            detail,
        );
    }

    pub fn register_resource(
        &self,
        kind: ResourceKind,
        label: impl Into<String>,
    ) -> Option<ResourceLease> {
        self.runtime.register_run_resource(&self.run, kind, label)
    }

    /// Transitional bridge for existing built-in executors. This is kept
    /// crate-private while built-ins move behind per-tool implementations.
    pub(crate) fn configuration(&self) -> &Config {
        &self.configuration
    }
}
