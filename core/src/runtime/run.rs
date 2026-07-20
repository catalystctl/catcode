use serde::Serialize;
use std::fmt;
use tokio_util::sync::CancellationToken;

/// Stable identity for one logical chat session. A session owns zero or more
/// runs and is replaced on `/new` and `load_session`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identity for one model/tool turn within a session.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Capability handed to work belonging to one run. Clones share cancellation.
/// Identity checks still go through [`super::RuntimeCoordinator`]; possession
/// of this value alone never makes a stale run active again.
#[derive(Clone, Debug)]
pub struct RunContext {
    session_id: SessionId,
    run_id: RunId,
    cancellation: CancellationToken,
}

/// Session-only ownership for background work (goal deploy, child agents,
/// plugin tasks) that is not itself the foreground model turn.
#[derive(Clone, Debug)]
pub struct SessionContext {
    session_id: SessionId,
    cancellation: CancellationToken,
}

impl SessionContext {
    pub(crate) fn new(session_id: SessionId, cancellation: CancellationToken) -> Self {
        Self {
            session_id,
            cancellation,
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl RunContext {
    pub(crate) fn new(
        session_id: SessionId,
        run_id: RunId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            session_id,
            run_id,
            cancellation,
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}
