use super::resources::ResourceRegistry;
use super::{
    CancellationReason, CancelledRun, ResourceKind, ResourceLease, ResourceSnapshot, RunContext,
    RunId, SessionContext, SessionId,
};
use rand::RngCore;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn new_id(prefix: &str, entropy: u64) -> String {
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{entropy:016x}-{sequence:016x}")
}

#[derive(Debug)]
struct ActiveRun {
    context: RunContext,
}

#[derive(Debug)]
struct Inner {
    session_id: SessionId,
    session_cancellation: CancellationToken,
    active_run: Option<ActiveRun>,
    discarded_stale_results: u64,
    last_cancellation: Option<CancelledRun>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSnapshot {
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub discarded_stale_results: u64,
    pub last_cancellation: Option<CancelledRun>,
    pub resources: Vec<ResourceSnapshot>,
}

/// Sole authority for the currently active session and run.
///
/// The mutex is intentionally synchronous: operations are tiny identity/token
/// transitions and must also be usable from the synchronous protocol event
/// path without blocking an async executor.
#[derive(Debug)]
pub struct RuntimeCoordinator {
    entropy: u64,
    inner: Mutex<Inner>,
    resources: Arc<ResourceRegistry>,
}

impl Default for RuntimeCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeCoordinator {
    pub fn new() -> Self {
        let entropy = rand::thread_rng().next_u64();
        Self::with_entropy(entropy)
    }

    fn with_entropy(entropy: u64) -> Self {
        Self {
            entropy,
            inner: Mutex::new(Inner {
                session_id: SessionId::new(new_id("session", entropy)),
                session_cancellation: CancellationToken::new(),
                active_run: None,
                discarded_stale_results: 0,
                last_cancellation: None,
            }),
            resources: Arc::new(ResourceRegistry::default()),
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.lock().session_id.clone()
    }

    pub fn session_context(&self) -> SessionContext {
        let inner = self.lock();
        SessionContext::new(
            inner.session_id.clone(),
            inner.session_cancellation.child_token(),
        )
    }

    pub fn is_session_active(&self, context: &SessionContext) -> bool {
        let inner = self.lock();
        inner.session_id == *context.session_id() && !context.cancellation().is_cancelled()
    }

    /// Start a run in the current session. Any prior active run is invalidated
    /// first; callers should normally cancel explicitly so the reason is known.
    pub fn start_run(&self) -> RunContext {
        let mut inner = self.lock();
        if let Some(previous) = inner.active_run.take() {
            previous.context.cancellation().cancel();
            self.resources
                .cancel_run(previous.context.session_id(), previous.context.run_id());
            inner.last_cancellation = Some(CancelledRun {
                session_id: previous.context.session_id().clone(),
                run_id: previous.context.run_id().clone(),
                reason: CancellationReason::Steering,
            });
        }
        let context = RunContext::new(
            inner.session_id.clone(),
            RunId::new(new_id("run", self.entropy)),
            inner.session_cancellation.child_token(),
        );
        inner.active_run = Some(ActiveRun {
            context: context.clone(),
        });
        context
    }

    /// Cancel and invalidate the current run. This transition is idempotent.
    pub fn cancel_current(&self, reason: CancellationReason) -> Option<CancelledRun> {
        let mut inner = self.lock();
        let active = inner.active_run.take()?;
        active.context.cancellation().cancel();
        self.resources
            .cancel_run(active.context.session_id(), active.context.run_id());
        let cancelled = CancelledRun {
            session_id: active.context.session_id().clone(),
            run_id: active.context.run_id().clone(),
            reason,
        };
        inner.last_cancellation = Some(cancelled.clone());
        Some(cancelled)
    }

    /// Replace the logical session and invalidate everything derived from its
    /// cancellation token before publishing the new identity.
    pub fn replace_session(&self, reason: CancellationReason) -> SessionId {
        let mut inner = self.lock();
        let old_session = inner.session_id.clone();
        if let Some(active) = inner.active_run.take() {
            active.context.cancellation().cancel();
            inner.last_cancellation = Some(CancelledRun {
                session_id: active.context.session_id().clone(),
                run_id: active.context.run_id().clone(),
                reason,
            });
        }
        inner.session_cancellation.cancel();
        self.resources.cancel_session(&old_session);
        inner.session_cancellation = CancellationToken::new();
        inner.session_id = SessionId::new(new_id("session", self.entropy));
        inner.session_id.clone()
    }

    pub fn is_active(&self, context: &RunContext) -> bool {
        let inner = self.lock();
        inner.session_id == *context.session_id()
            && inner
                .active_run
                .as_ref()
                .is_some_and(|active| active.context.run_id() == context.run_id())
            && !context.cancellation().is_cancelled()
    }

    /// Mark a run complete only when it is still current. A stale finisher can
    /// therefore never clear a newer run's slot.
    pub fn complete_run(&self, context: &RunContext) -> bool {
        let mut inner = self.lock();
        let matches = inner.session_id == *context.session_id()
            && inner
                .active_run
                .as_ref()
                .is_some_and(|active| active.context.run_id() == context.run_id());
        if matches {
            inner.active_run = None;
            self.resources
                .cancel_run(context.session_id(), context.run_id());
        }
        matches
    }

    pub fn note_stale_result(&self) {
        let mut inner = self.lock();
        inner.discarded_stale_results = inner.discarded_stale_results.saturating_add(1);
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        let inner = self.lock();
        RuntimeSnapshot {
            session_id: inner.session_id.clone(),
            run_id: inner
                .active_run
                .as_ref()
                .map(|active| active.context.run_id().clone()),
            discarded_stale_results: inner.discarded_stale_results,
            last_cancellation: inner.last_cancellation.clone(),
            resources: self.resources.snapshots(),
        }
    }

    pub fn register_run_resource(
        &self,
        context: &RunContext,
        kind: ResourceKind,
        label: impl Into<String>,
    ) -> Option<ResourceLease> {
        let inner = self.lock();
        let active = inner.session_id == *context.session_id()
            && inner
                .active_run
                .as_ref()
                .is_some_and(|active| active.context.run_id() == context.run_id())
            && !context.cancellation().is_cancelled();
        if !active {
            return None;
        }
        Some(self.resources.register(
            kind,
            label,
            context.session_id().clone(),
            Some(context.run_id().clone()),
            context.cancellation().child_token(),
        ))
    }

    /// Register work against whichever foreground run is current at this
    /// instant. This is intended for helpers (approval/ask/tool waiters) that
    /// execute inside a run but do not carry the `RunContext` in their API.
    pub fn register_active_run_resource(
        &self,
        kind: ResourceKind,
        label: impl Into<String>,
    ) -> Option<ResourceLease> {
        let inner = self.lock();
        let context = &inner.active_run.as_ref()?.context;
        if context.cancellation().is_cancelled() {
            return None;
        }
        Some(self.resources.register(
            kind,
            label,
            context.session_id().clone(),
            Some(context.run_id().clone()),
            context.cancellation().child_token(),
        ))
    }

    pub fn register_session_resource(
        &self,
        context: &SessionContext,
        kind: ResourceKind,
        label: impl Into<String>,
    ) -> Option<ResourceLease> {
        let inner = self.lock();
        if inner.session_id != *context.session_id() || context.cancellation().is_cancelled() {
            return None;
        }
        Some(self.resources.register(
            kind,
            label,
            context.session_id().clone(),
            None,
            context.cancellation().child_token(),
        ))
    }

    /// Register child work whose cancellation token is already derived from
    /// its logical parent. Unlike `register_session_resource`, this preserves
    /// that exact token so coordinator cancellation, parent cancellation, and
    /// the child loop all observe one ownership signal. `parent_run_id` may be
    /// a foreground run or a goal ID and is exposed in runtime diagnostics.
    pub fn register_owned_session_resource(
        &self,
        context: &SessionContext,
        kind: ResourceKind,
        label: impl Into<String>,
        parent_run_id: Option<&str>,
        cancellation: CancellationToken,
    ) -> Option<ResourceLease> {
        let inner = self.lock();
        if inner.session_id != *context.session_id()
            || context.cancellation().is_cancelled()
            || cancellation.is_cancelled()
        {
            return None;
        }
        Some(self.resources.register(
            kind,
            label,
            context.session_id().clone(),
            parent_run_id.map(|id| RunId::new(id.to_string())),
            cancellation,
        ))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinator() -> RuntimeCoordinator {
        RuntimeCoordinator::with_entropy(0xfeed)
    }

    #[test]
    fn run_ids_are_unique_within_a_session() {
        let runtime = coordinator();
        let first = runtime.start_run();
        runtime.complete_run(&first);
        let second = runtime.start_run();
        assert_eq!(first.session_id(), second.session_id());
        assert_ne!(first.run_id(), second.run_id());
    }

    #[test]
    fn replacing_session_cancels_and_invalidates_old_run() {
        let runtime = coordinator();
        let old = runtime.start_run();
        let old_session = old.session_id().clone();
        let new_session = runtime.replace_session(CancellationReason::NewSession);
        assert_ne!(old_session, new_session);
        assert!(old.cancellation().is_cancelled());
        assert!(!runtime.is_active(&old));
        assert!(!runtime.complete_run(&old));
    }

    #[test]
    fn stale_finisher_cannot_clear_new_run() {
        let runtime = coordinator();
        let old = runtime.start_run();
        runtime.cancel_current(CancellationReason::Steering);
        let current = runtime.start_run();
        assert!(!runtime.complete_run(&old));
        assert!(runtime.is_active(&current));
        assert_eq!(runtime.snapshot().run_id.as_ref(), Some(current.run_id()));
    }

    #[test]
    fn cancellation_is_idempotent_and_records_reason() {
        let runtime = coordinator();
        let run = runtime.start_run();
        let cancelled = runtime
            .cancel_current(CancellationReason::Abort)
            .expect("active run");
        assert_eq!(cancelled.run_id, *run.run_id());
        assert_eq!(cancelled.reason, CancellationReason::Abort);
        assert!(runtime.cancel_current(CancellationReason::Abort).is_none());
    }

    #[test]
    fn session_resources_are_cancelled_then_unregistered_on_drop() {
        let runtime = coordinator();
        let session = runtime.session_context();
        let resource = runtime
            .register_session_resource(&session, ResourceKind::Goal, "goal deploy")
            .unwrap();
        assert_eq!(runtime.snapshot().resources.len(), 1);
        runtime.replace_session(CancellationReason::NewSession);
        assert!(resource.cancellation().is_cancelled());
        assert!(runtime.snapshot().resources[0].cancelled);
        drop(resource);
        assert!(runtime.snapshot().resources.is_empty());
    }

    #[test]
    fn owned_child_resource_preserves_parent_identity_and_shared_cancellation() {
        let coordinator = coordinator();
        let session = coordinator.session_context();
        let child_cancel = session.cancellation().child_token();
        let lease = coordinator
            .register_owned_session_resource(
                &session,
                ResourceKind::Subagent,
                "subagent:child-1:worker",
                Some("parent-run-1"),
                child_cancel.clone(),
            )
            .unwrap();
        let resource = coordinator
            .snapshot()
            .resources
            .into_iter()
            .find(|resource| resource.label == "subagent:child-1:worker")
            .unwrap();
        assert_eq!(
            resource.run_id.as_ref().map(RunId::as_str),
            Some("parent-run-1")
        );
        coordinator.replace_session(CancellationReason::NewSession);
        assert!(child_cancel.is_cancelled());
        drop(lease);
        assert!(coordinator
            .snapshot()
            .resources
            .iter()
            .all(|resource| resource.label != "subagent:child-1:worker"));
    }

    #[test]
    fn stale_context_cannot_register_new_work() {
        let runtime = coordinator();
        let run = runtime.start_run();
        runtime.cancel_current(CancellationReason::Abort);
        assert!(runtime
            .register_run_resource(&run, ResourceKind::Task, "late child")
            .is_none());
    }
}
