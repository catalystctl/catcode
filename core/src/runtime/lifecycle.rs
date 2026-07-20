use super::{RunId, SessionId};

/// One vocabulary for all paths that invalidate work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationReason {
    Abort,
    Reset,
    Clear,
    NewSession,
    LoadSession,
    Steering,
    GoalCancelled,
    FatalError,
    Shutdown,
}

impl CancellationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Abort => "abort",
            Self::Reset => "reset",
            Self::Clear => "clear",
            Self::NewSession => "new_session",
            Self::LoadSession => "load_session",
            Self::Steering => "steering",
            Self::GoalCancelled => "goal_cancelled",
            Self::FatalError => "fatal_error",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelledRun {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub reason: CancellationReason,
}
