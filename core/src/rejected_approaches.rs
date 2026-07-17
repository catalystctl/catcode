//! Rejected-approach memory (spec §7.5 / §13).
//!
//! First-class knowledge of approaches that failed or were rejected. Stored as
//! JSONL under `learning/global/rejected-approaches.jsonl` or
//! `learning/projects/<id>/rejected-approaches.jsonl`. Fail-open.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, GlobalLearningPaths, ProjectLearningPaths, MAX_FEEDBACK};
use crate::preferences::LearningStatus;
use crate::task_fingerprint::{fingerprint_similarity, TaskFingerprint};

/// A rejected approach record (spec §7.5).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedApproach {
    pub id: String,
    /// `"project"` or `"global"`.
    pub scope: String,
    pub task_fingerprint: TaskFingerprint,
    pub approach: String,
    pub rejection_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_alternative: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub confidence: f32,
    pub status: LearningStatus,
}

fn rejected_path(project_id: Option<&str>) -> std::path::PathBuf {
    match project_id {
        None => GlobalLearningPaths::resolve().rejected_approaches,
        Some(id) => ProjectLearningPaths::resolve(id)
            .root
            .join("rejected-approaches.jsonl"),
    }
}

/// Append a rejected-approach record. `project_id = None` → global store.
/// Fail-open on I/O errors.
pub fn append_rejected(project_id: Option<&str>, record: &RejectedApproach) {
    let path = rejected_path(project_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if project_id.is_none() {
        let _ = GlobalLearningPaths::resolve().ensure();
    } else if let Some(id) = project_id {
        let _ = ProjectLearningPaths::resolve(id).ensure();
    }
    learning_store::append_jsonl(&path, record, MAX_FEEDBACK);
}

/// Load rejected approaches for a scope (skips malformed lines).
pub fn load_rejected(project_id: Option<&str>) -> Vec<RejectedApproach> {
    learning_store::read_jsonl(&rejected_path(project_id))
}

/// Match rejected approaches by task-fingerprint similarity.
/// Returns records with `sim >= min_sim`, sorted by similarity descending, capped at `limit`.
pub fn match_rejected(
    fp: &TaskFingerprint,
    project_id: Option<&str>,
    min_sim: f32,
    limit: usize,
) -> Vec<(f32, RejectedApproach)> {
    let mut scored: Vec<(f32, RejectedApproach)> = load_rejected(project_id)
        .into_iter()
        .filter(|r| r.status != LearningStatus::Deprecated && r.status != LearningStatus::Rejected)
        .map(|r| {
            let sim = fingerprint_similarity(fp, &r.task_fingerprint);
            (sim, r)
        })
        .filter(|(sim, _)| *sim >= min_sim)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// Confidence helper: a single environmental failure must not become high confidence.
///
/// Spec §13: "Do not permanently reject an approach based on one environmental failure."
pub fn confidence_for_rejection(
    is_environmental: bool,
    evidence_count: usize,
    explicit_user: bool,
) -> f32 {
    if explicit_user {
        return 0.95;
    }
    if is_environmental && evidence_count <= 1 {
        // Cap well below "high" so one flake/env failure cannot dominate planning.
        return 0.40;
    }
    if evidence_count >= 3 {
        0.80
    } else if evidence_count == 2 {
        0.65
    } else {
        0.55
    }
}

/// Whether confidence is considered "high" for rejection ranking.
pub fn is_high_confidence(c: f32) -> bool {
    c >= 0.75
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static N: AtomicU64 = AtomicU64::new(0);
    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn with_temp_learning<R>(f: impl FnOnce() -> R) -> R {
        let _serial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "rejected-{}-{}-{}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let _g = override_learning_root(d);
        f()
    }

    fn sample_fp(intent: &str, symbols: &[&str]) -> TaskFingerprint {
        TaskFingerprint {
            intent: intent.into(),
            symbols: symbols.iter().map(|s| (*s).to_string()).collect(),
            subsystems: vec!["memory".into()],
            languages: vec!["rust".into()],
            ..Default::default()
        }
    }

    #[test]
    fn store_and_load_global_and_project() {
        with_temp_learning(|| {
            let global = RejectedApproach {
                id: "rej-g1".into(),
                scope: "global".into(),
                task_fingerprint: sample_fp("add-provider", &["ProviderConfig"]),
                approach: "put oauth in config.json".into(),
                rejection_reason: "oauth belongs in plugins".into(),
                preferred_alternative: Some("use plugin oauth".into()),
                evidence: vec!["ep-1".into()],
                confidence: 0.9,
                status: LearningStatus::Verified,
            };
            append_rejected(None, &global);

            let project = RejectedApproach {
                id: "rej-p1".into(),
                scope: "project".into(),
                task_fingerprint: sample_fp("extend-tool-schema", &["MemoryEntry"]),
                approach: "broad refactor of tools.rs".into(),
                rejection_reason: "user asked for minimal change".into(),
                preferred_alternative: Some("narrow edit".into()),
                evidence: vec!["ep-2".into()],
                confidence: 0.85,
                status: LearningStatus::Verified,
            };
            append_rejected(Some("project-abc"), &project);

            let g = load_rejected(None);
            assert_eq!(g.len(), 1);
            assert_eq!(g[0].id, "rej-g1");

            let p = load_rejected(Some("project-abc"));
            assert_eq!(p.len(), 1);
            assert_eq!(p[0].id, "rej-p1");
            assert!(ProjectLearningPaths::resolve("project-abc")
                .root
                .join("rejected-approaches.jsonl")
                .exists());
        });
    }

    #[test]
    fn match_by_fingerprint() {
        with_temp_learning(|| {
            append_rejected(
                Some("project-match"),
                &RejectedApproach {
                    id: "rej-match".into(),
                    scope: "project".into(),
                    task_fingerprint: sample_fp(
                        "extend-tool-schema",
                        &["MemoryEntry", "tool_definitions"],
                    ),
                    approach: "add action without schema".into(),
                    rejection_reason: "schema must stay in sync".into(),
                    preferred_alternative: None,
                    evidence: vec!["ep-9".into()],
                    confidence: 0.88,
                    status: LearningStatus::Verified,
                },
            );
            append_rejected(
                Some("project-match"),
                &RejectedApproach {
                    id: "rej-other".into(),
                    scope: "project".into(),
                    task_fingerprint: sample_fp("docs", &["README"]),
                    approach: "rewrite all docs".into(),
                    rejection_reason: "too broad".into(),
                    preferred_alternative: None,
                    evidence: vec![],
                    confidence: 0.7,
                    status: LearningStatus::Verified,
                },
            );

            let query = sample_fp("extend-tool-schema", &["MemoryEntry", "tool_definitions"]);
            let hits = match_rejected(&query, Some("project-match"), 0.3, 5);
            assert!(!hits.is_empty());
            assert_eq!(hits[0].1.id, "rej-match");
            assert!(hits[0].0 > 0.5);
        });
    }

    #[test]
    fn environmental_single_failure_not_high_confidence() {
        let c = confidence_for_rejection(true, 1, false);
        assert!(
            !is_high_confidence(c),
            "single environmental failure must not be high confidence, got {c}"
        );
        assert!(c <= 0.45);

        let repeated = confidence_for_rejection(false, 3, false);
        assert!(is_high_confidence(repeated));

        let explicit = confidence_for_rejection(true, 1, true);
        assert!(is_high_confidence(explicit));
    }
}
