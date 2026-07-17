//! Task-pattern aggregates and project feedback JSONL writers (spec §6 / §17).
//!
//! Reserved paths under `learning/projects/<id>/`:
//! - `task-patterns.jsonl` — merged by fingerprint intent+subsystems
//! - `feedback.jsonl` — user_correction / undo / checkpoint_restore / explicit_preference
//!
//! Wired as `learning_store::task_patterns` (no `main.rs` mod) so the crate
//! root stays untouched.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, ProjectLearningPaths, MAX_FEEDBACK};
use crate::task_fingerprint::TaskFingerprint;

const MAX_TASK_PATTERNS: usize = 500;

/// Aggregated success/failure pattern for similar task fingerprints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskPattern {
    pub id: String,
    pub fingerprint: TaskFingerprint,
    pub success_count: u32,
    pub failure_count: u32,
    pub last_seen: u64,
    pub sample_approach: String,
}

/// Project-scoped feedback event (distinct from episode-inline corrections).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub id: String,
    /// `user_correction` | `undo` | `checkpoint_restore` | `explicit_preference`
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    pub at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pattern_key(fp: &TaskFingerprint) -> String {
    let mut subs = fp.subsystems.clone();
    subs.sort();
    subs.dedup();
    format!("{}|{}", fp.intent, subs.join(","))
}

fn short_id(prefix: &str, key: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}-{h:016x}")
}

/// Merge a task outcome into `task-patterns.jsonl` by intent+subsystems key.
/// Rewrite-all under lock; caps at 500 patterns.
pub fn append_task_pattern(project_id: &str, fp: &TaskFingerprint, success: bool, approach: &str) {
    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let path = &paths.task_patterns;
    let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));

    let key = pattern_key(fp);
    let mut patterns: Vec<TaskPattern> = learning_store::read_jsonl(path);
    let now = now_secs();

    if let Some(existing) = patterns
        .iter_mut()
        .find(|p| pattern_key(&p.fingerprint) == key)
    {
        if success {
            existing.success_count = existing.success_count.saturating_add(1);
        } else {
            existing.failure_count = existing.failure_count.saturating_add(1);
        }
        existing.last_seen = now;
        if !approach.is_empty() {
            existing.sample_approach = approach.to_string();
        }
    } else {
        patterns.push(TaskPattern {
            id: short_id("tp", &key),
            fingerprint: fp.clone(),
            success_count: if success { 1 } else { 0 },
            failure_count: if success { 0 } else { 1 },
            last_seen: now,
            sample_approach: approach.to_string(),
        });
    }

    // Prefer recently seen when capping.
    patterns.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then_with(|| a.id.cmp(&b.id)));
    if patterns.len() > MAX_TASK_PATTERNS {
        patterns.truncate(MAX_TASK_PATTERNS);
    }

    let mut body = String::new();
    for p in &patterns {
        if let Ok(line) = serde_json::to_string(p) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let _ = crate::fsutil::atomic_write_str(path, &body);
}

/// Append a feedback event to `feedback.jsonl` (capped via learning_store).
pub fn append_feedback(project_id: &str, kind: &str, text: &str, episode_id: Option<&str>) {
    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let at = now_secs();
    let id_src = format!("{kind}|{text}|{at}|{}", episode_id.unwrap_or(""));
    let ev = FeedbackEvent {
        id: short_id("fb", &id_src),
        kind: kind.to_string(),
        text: text.to_string(),
        episode_id: episode_id.map(str::to_string),
        at,
    };
    learning_store::append_jsonl(&paths.feedback, &ev, MAX_FEEDBACK);
}

/// Load task patterns for a project (newest-ish first by last_seen after rewrite).
pub fn load_task_patterns(project_id: &str) -> Vec<TaskPattern> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl(&paths.task_patterns)
}

/// Load feedback events for a project.
pub fn load_feedback(project_id: &str) -> Vec<FeedbackEvent> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl(&paths.feedback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::{learning_test_serial, override_learning_root};
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp_root() -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "task-patterns-{}-{}-{}",
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_fp(intent: &str, subsystems: &[&str]) -> TaskFingerprint {
        TaskFingerprint {
            intent: intent.into(),
            subsystems: subsystems.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn append_task_pattern_merges_by_intent_subsystems() {
        let root = tmp_root();
        let _lserial = learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let pid = "project-tp-merge";
        let fp = sample_fp("extend-tool-schema", &["memory", "tools"]);

        append_task_pattern(pid, &fp, true, "add schema field");
        append_task_pattern(pid, &fp, true, "update dispatch");
        append_task_pattern(pid, &fp, false, "bad approach");

        let got = load_task_patterns(pid);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].success_count, 2);
        assert_eq!(got[0].failure_count, 1);
        assert_eq!(got[0].sample_approach, "bad approach");

        let other = sample_fp("fix-bug", &["memory"]);
        append_task_pattern(pid, &other, true, "narrow fix");
        let got = load_task_patterns(pid);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn append_feedback_writes_jsonl() {
        let root = tmp_root();
        let _lserial = learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let pid = "project-fb";

        append_feedback(pid, "undo", "user undid changes", Some("ep-1"));
        append_feedback(pid, "user_correction", "prefer targeted edits", None);

        let got = load_feedback(pid);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, "undo");
        assert_eq!(got[0].episode_id.as_deref(), Some("ep-1"));
        assert_eq!(got[1].kind, "user_correction");
        assert!(got[1].episode_id.is_none());
    }
}
