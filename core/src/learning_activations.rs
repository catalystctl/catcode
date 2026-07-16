//! Memory/skill activation tracking (spec §7.6 / §22).
//!
//! Records when knowledge is injected so utility can be measured beyond
//! `memory get`. Fail-open; capped JSONL under the project learning dir.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, ProjectLearningPaths, MAX_ACTIVATIONS};

/// Retrieval stage labels (spec §7.6 / §14.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrievalStage {
    PrePlan,
    Implementation,
    ErrorRecovery,
    PreValidation,
    Reflection,
}

impl RetrievalStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrePlan => "pre_plan",
            Self::Implementation => "implementation",
            Self::ErrorRecovery => "error_recovery",
            Self::PreValidation => "pre_validation",
            Self::Reflection => "reflection",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "implementation" => Self::Implementation,
            "error_recovery" | "error" => Self::ErrorRecovery,
            "pre_validation" | "validation" => Self::PreValidation,
            "reflection" => Self::Reflection,
            _ => Self::PrePlan,
        }
    }
}

/// Compact activation record (spec §7.6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningActivation {
    pub id: String,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    pub item_kind: String,
    pub item_id: String,
    pub stage: String,
    pub rank: usize,
    pub retrieval_score: f32,
    pub tokens_injected: usize,
    #[serde(default)]
    pub explicitly_opened: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed_by_agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append one activation (fail-open).
pub fn record_activation(act: &LearningActivation) {
    let paths = ProjectLearningPaths::resolve(&act.project_id);
    let _ = paths.ensure();
    learning_store::append_jsonl(&paths.activations, act, MAX_ACTIVATIONS);
}

/// Record a batch of pack-injected items for a stage.
pub fn record_pack_activations(
    project_id: &str,
    stage: RetrievalStage,
    episode_id: Option<&str>,
    items: &[(/*kind*/ &str, /*id*/ &str, /*rank*/ usize, /*score*/ f32, /*tokens*/ usize)],
) {
    let ts = now_secs();
    for (i, (kind, id, rank, score, tokens)) in items.iter().enumerate() {
        let act = LearningActivation {
            id: format!("act-{ts}-{i}"),
            project_id: project_id.to_string(),
            episode_id: episode_id.map(|s| s.to_string()),
            item_kind: (*kind).to_string(),
            item_id: (*id).to_string(),
            stage: stage.as_str().to_string(),
            rank: *rank,
            retrieval_score: *score,
            tokens_injected: *tokens,
            explicitly_opened: false,
            followed_by_agent: None,
            outcome: None,
        };
        record_activation(&act);
    }
}

/// Load activations (skips malformed lines).
pub fn load_activations(project_id: &str) -> Vec<LearningActivation> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl(&paths.activations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::{learning_test_serial, override_learning_root};

    #[test]
    fn record_and_load_activations() {
        let _serial = learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!(
            "act-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _g = override_learning_root(root);
        record_pack_activations(
            "project-test",
            RetrievalStage::PrePlan,
            None,
            &[("memory", "foo", 0, 0.8, 40), ("episode", "ep-1", 1, 0.5, 20)],
        );
        let loaded = load_activations("project-test");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].item_id, "foo");
        assert_eq!(loaded[0].stage, "pre_plan");
    }
}
