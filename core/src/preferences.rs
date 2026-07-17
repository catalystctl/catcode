//! Developer preference records and project/global scope enforcement (spec §4 / §7.4 / §12).
//!
//! Preferences are stored as JSONL under the learning store (`learning/global/`
//! and later project overrides). This module provides the data model plus
//! deterministic rules for scope inference and precedence — it does **not**
//! replace markdown memories of type `preference`; those remain readable.

#![allow(dead_code)]
use serde::{Deserialize, Serialize};

use crate::learning_store::{self, GlobalLearningPaths, MAX_FEEDBACK};
use crate::memory::Scope;

/// Learning lifecycle status shared with preferences / rejected approaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LearningStatus {
    Candidate,
    #[default]
    Verified,
    NeedsVerification,
    Stale,
    Deprecated,
    Rejected,
}

impl LearningStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Verified => "verified",
            Self::NeedsVerification => "needs_verification",
            Self::Stale => "stale",
            Self::Deprecated => "deprecated",
            Self::Rejected => "rejected",
        }
    }
}

/// A structured preference record (spec §7.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreferenceRecord {
    pub id: String,
    pub scope: String,
    pub category: String,
    pub statement: String,
    pub status: LearningStatus,
    pub confidence: f32,
    pub explicit: bool,
    #[serde(default)]
    pub supporting_events: Vec<String>,
    #[serde(default)]
    pub contradicting_events: Vec<String>,
    #[serde(default)]
    pub project_exceptions: Vec<String>,
    pub first_seen: u64,
    pub last_seen: u64,
}

/// Infer preference scope from user wording (spec §12.1).
///
/// Ambiguous preferences default to **project** (workspace) so they do not
/// leak into global memory.
pub fn infer_preference_scope(statement: &str) -> Scope {
    let s = statement.to_lowercase();
    if s.contains("in this repo")
        || s.contains("in this project")
        || s.contains("for this project")
        || s.contains("for this repo")
        || s.contains("here we")
        || s.contains("this codebase")
    {
        return Scope::Workspace;
    }
    if s.contains("i always")
        || s.contains("from now on")
        || s.contains("across all")
        || s.contains("in every project")
        || s.contains("globally")
    {
        return Scope::Global;
    }
    // Ambiguous → project (do not leak globally).
    Scope::Workspace
}

/// Suggested confidence for an evidence class (spec §12.3).
pub fn confidence_for_evidence(explicit: bool, inferred_single: bool) -> f32 {
    if explicit {
        0.95
    } else if inferred_single {
        0.55
    } else {
        0.65
    }
}

/// Clamp confidence to `0.0..=1.0`.
pub fn clamp_confidence(c: f32) -> f32 {
    c.clamp(0.0, 1.0)
}

/// Whether a preference may be auto-promoted from project → global.
/// Requires multi-project evidence and no project-specific paths (spec §4.4).
pub fn may_promote_to_global(
    statement: &str,
    distinct_projects: usize,
    supporting_events: usize,
    explicit_user: bool,
    has_unresolved_contradiction: bool,
    recently_caused_undo: bool,
) -> bool {
    if explicit_user {
        return !has_project_specific_details(statement);
    }
    if has_unresolved_contradiction || recently_caused_undo {
        return false;
    }
    if has_project_specific_details(statement) {
        return false;
    }
    distinct_projects >= 2 && supporting_events >= 3
}

fn has_project_specific_details(statement: &str) -> bool {
    let s = statement.to_lowercase();
    s.contains('/')
        || s.contains('\\')
        || s.contains(".rs")
        || s.contains(".go")
        || s.contains("core/src")
        || s.contains("this repo's")
}

/// Persist a preference into the global learning JSONL (fail-open).
pub fn append_global_preference(pref: &PreferenceRecord) {
    let paths = GlobalLearningPaths::resolve();
    let _ = paths.ensure();
    learning_store::append_jsonl(&paths.preferences, pref, MAX_FEEDBACK);
}

/// Load global preferences (skips malformed lines).
pub fn load_global_preferences() -> Vec<PreferenceRecord> {
    let paths = GlobalLearningPaths::resolve();
    learning_store::read_jsonl(&paths.preferences)
}

/// Precedence rank for conflict resolution (lower = wins). Spec §4.3.
pub fn precedence_rank(kind: &str) -> u8 {
    match kind {
        "current_user_instruction" => 1,
        "current_task_requirements" => 2,
        "explicit_project_decision" => 3,
        "explicit_global_preference" => 4,
        "verified_project_inference" => 5,
        "repeated_global_inference" => 6,
        "historical_episode" => 7,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;

    #[test]
    fn ambiguous_defaults_to_project() {
        assert_eq!(infer_preference_scope("prefer tabs"), Scope::Workspace);
        assert_eq!(infer_preference_scope("I always use tabs"), Scope::Global);
        assert_eq!(
            infer_preference_scope("In this repo use spaces"),
            Scope::Workspace
        );
    }

    #[test]
    fn one_project_does_not_auto_promote() {
        assert!(!may_promote_to_global(
            "prefer explicit error handling",
            1,
            5,
            false,
            false,
            false
        ));
        assert!(may_promote_to_global(
            "prefer explicit error handling",
            2,
            3,
            false,
            false,
            false
        ));
        assert!(!may_promote_to_global(
            "never edit core/src/memory.rs directly",
            5,
            10,
            false,
            false,
            false
        ));
    }

    #[test]
    fn explicit_confidence_is_high() {
        assert!((confidence_for_evidence(true, false) - 0.95).abs() < f32::EPSILON);
        assert!((confidence_for_evidence(false, true) - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn append_and_load_preference() {
        let root = std::env::temp_dir().join(format!(
            "pref-test-{}-{}",
            std::process::id(),
            precedence_rank("x")
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let pref = PreferenceRecord {
            id: "pref-tabs".into(),
            scope: "global".into(),
            category: "code-style".into(),
            statement: "I always use tabs".into(),
            status: LearningStatus::Verified,
            confidence: 0.95,
            explicit: true,
            supporting_events: vec!["user-msg-1".into()],
            contradicting_events: vec![],
            project_exceptions: vec![],
            first_seen: 1,
            last_seen: 1,
        };
        append_global_preference(&pref);
        let loaded = load_global_preferences();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "pref-tabs");
    }
}
