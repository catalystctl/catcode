use serde::Serialize;

/// Stable status vocabulary for tool-result protocol events.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Success,
    Denied,
    Cancelled,
    TimedOut,
    Failed,
    Stale,
    PartiallyCompleted,
}

impl ToolResultStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Failed => "failed",
            Self::Stale => "stale",
            Self::PartiallyCompleted => "partially_completed",
        }
    }

    /// Backward-compatible normalization for legacy execution paths that still
    /// return `{ok, output}`. New executors may set an explicit status before
    /// emission; the event sink only calls this when status is absent.
    pub fn from_legacy(ok: bool, output: &str) -> Self {
        if ok {
            return Self::Success;
        }
        let output = output.to_ascii_lowercase();
        if output.contains("denied")
            || output.contains("declined")
            || output.contains("not approved")
        {
            Self::Denied
        } else if output.contains("cancelled")
            || output.contains("canceled")
            || output.contains("aborted")
        {
            Self::Cancelled
        } else if output.contains("timed out") || output.contains("timeout") {
            Self::TimedOut
        } else if output.contains("partially completed") || output.contains("partial success") {
            Self::PartiallyCompleted
        } else {
            Self::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolResultStatus;

    #[test]
    fn legacy_results_normalize_into_stable_statuses() {
        assert_eq!(
            ToolResultStatus::from_legacy(true, "ok"),
            ToolResultStatus::Success
        );
        assert_eq!(
            ToolResultStatus::from_legacy(false, "user denied"),
            ToolResultStatus::Denied
        );
        assert_eq!(
            ToolResultStatus::from_legacy(false, "bash aborted"),
            ToolResultStatus::Cancelled
        );
        assert_eq!(
            ToolResultStatus::from_legacy(false, "timed out"),
            ToolResultStatus::TimedOut
        );
        assert_eq!(
            ToolResultStatus::from_legacy(false, "bad input"),
            ToolResultStatus::Failed
        );
    }
}
