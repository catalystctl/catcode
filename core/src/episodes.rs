//! Coding episode capture (spec §7.2 / §17).
//!
//! An episode is a compact structured record of one coding task — not a raw
//! transcript. Stored as JSONL under the project's learning directory
//! (`learning/projects/<id>/episodes.jsonl`). Fail-open: I/O errors never
//! abort a coding turn.
//!
//! Does **not** store full tool outputs or unbounded logs — only signatures,
//! paths, counts, and short summaries.

#![allow(dead_code)]
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, MAX_EPISODES};
use crate::project_identity::ProjectIdentity;
use crate::task_fingerprint::{self, FingerprintInputs, TaskFingerprint};

/// Episode outcome (spec §7.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOutcome {
    SuccessVerified,
    SuccessUnverified,
    Partial,
    FailedTests,
    Reverted,
    Aborted,
    Unknown,
}

impl Default for EpisodeOutcome {
    fn default() -> Self {
        Self::Unknown
    }
}

impl EpisodeOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SuccessVerified => "success_verified",
            Self::SuccessUnverified => "success_unverified",
            Self::Partial => "partial",
            Self::FailedTests => "failed_tests",
            Self::Reverted => "reverted",
            Self::Aborted => "aborted",
            Self::Unknown => "unknown",
        }
    }

    /// Strong evidence for skill promotion requires verified success.
    pub fn is_strong_success(&self) -> bool {
        matches!(self, Self::SuccessVerified)
    }
}

/// Compact diagnostic signature (not a full log).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSignature {
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Compact test result (command + status + optional short failure class).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub command: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_signature: Option<String>,
}

/// User correction / undo feedback attached to an episode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub kind: String,
    pub summary: String,
    pub at: u64,
}

/// Compact coding episode record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodingEpisode {
    pub id: String,
    pub schema_version: u32,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub started_at: u64,
    pub completed_at: u64,

    pub user_intent: String,
    pub task_fingerprint: TaskFingerprint,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_commit: Option<String>,

    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub symbols_read: Vec<String>,
    #[serde(default)]
    pub symbols_changed: Vec<String>,

    #[serde(default)]
    pub tools_used: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticSignature>,
    #[serde(default)]
    pub tests_run: Vec<TestResult>,

    #[serde(default)]
    pub approach_summary: String,
    #[serde(default)]
    pub rejected_approaches: Vec<String>,
    #[serde(default)]
    pub memories_used: Vec<String>,
    #[serde(default)]
    pub skills_used: Vec<String>,

    #[serde(default)]
    pub user_corrections: Vec<FeedbackEvent>,
    #[serde(default)]
    pub undo_count: u32,
    #[serde(default)]
    pub checkpoint_restores: u32,

    pub outcome: EpisodeOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_diff_hash: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

pub const EPISODE_SCHEMA_VERSION: u32 = 1;

/// Builder / in-progress episode accumulator for a turn.
#[derive(Clone, Debug, Default)]
pub struct EpisodeBuilder {
    pub session_id: Option<String>,
    pub user_intent: String,
    pub started_at: u64,
    pub base_commit: Option<String>,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub symbols_read: Vec<String>,
    pub symbols_changed: Vec<String>,
    pub tools_used: Vec<String>,
    pub diagnostics: Vec<DiagnosticSignature>,
    pub tests_run: Vec<TestResult>,
    pub approach_summary: String,
    pub rejected_approaches: Vec<String>,
    pub memories_used: Vec<String>,
    pub skills_used: Vec<String>,
    pub user_corrections: Vec<FeedbackEvent>,
    pub undo_count: u32,
    pub checkpoint_restores: u32,
    pub model: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
}

impl EpisodeBuilder {
    pub fn new(user_intent: impl Into<String>) -> Self {
        Self {
            user_intent: truncate(&user_intent.into(), 240).to_string(),
            started_at: now_secs(),
            ..Default::default()
        }
    }

    pub fn record_tool(&mut self, name: &str) {
        if self.tools_used.last().map(|s| s.as_str()) != Some(name) {
            // Dedup consecutive duplicates; keep first occurrences of each.
            if !self.tools_used.iter().any(|t| t == name) {
                self.tools_used.push(name.to_string());
            }
        }
    }

    pub fn record_file_read(&mut self, path: &str) {
        push_unique(&mut self.files_read, path, 64);
    }

    pub fn record_file_changed(&mut self, path: &str) {
        push_unique(&mut self.files_changed, path, 64);
    }

    pub fn record_undo(&mut self) {
        self.undo_count = self.undo_count.saturating_add(1);
        self.user_corrections.push(FeedbackEvent {
            kind: "undo".into(),
            summary: "user undid agent changes".into(),
            at: now_secs(),
        });
    }

    pub fn record_checkpoint_restore(&mut self) {
        self.checkpoint_restores = self.checkpoint_restores.saturating_add(1);
        self.user_corrections.push(FeedbackEvent {
            kind: "checkpoint_restore".into(),
            summary: "checkpoint restored".into(),
            at: now_secs(),
        });
    }

    pub fn record_test(&mut self, command: &str, ok: bool, failure_sig: Option<&str>) {
        // Never store full logs — truncate signature.
        self.tests_run.push(TestResult {
            command: truncate(command, 120).to_string(),
            ok,
            duration_ms: None,
            failure_signature: failure_sig.map(|s| truncate(s, 160).to_string()),
        });
    }

    /// Finalize and persist the episode. Fail-open.
    pub fn finish(
        self,
        identity: &ProjectIdentity,
        outcome: EpisodeOutcome,
        elapsed_ms: Option<u64>,
        final_diff_hash: Option<String>,
    ) -> CodingEpisode {
        let diag_classes: Vec<String> = self
            .diagnostics
            .iter()
            .map(|d| d.class.clone())
            .collect();
        let test_cmds: Vec<String> = self.tests_run.iter().map(|t| t.command.clone()).collect();
        let fp = task_fingerprint::build_fingerprint(&FingerprintInputs {
            user_intent: &self.user_intent,
            files_read: &self.files_read,
            files_changed: &self.files_changed,
            symbols: &self.symbols_read,
            tools_used: &self.tools_used,
            diagnostics: &diag_classes,
            tests_run: &test_cmds,
        });

        let completed_at = now_secs();
        let id = format!(
            "ep-{:08x}",
            (fnv1a(identity.id.as_bytes())
                ^ self.started_at
                ^ (self.files_changed.len() as u64).wrapping_mul(0x9e3779b97f4a7c15))
                as u32
        );

        let episode = CodingEpisode {
            id,
            schema_version: EPISODE_SCHEMA_VERSION,
            project_id: identity.id.clone(),
            session_id: self.session_id,
            started_at: self.started_at,
            completed_at,
            user_intent: self.user_intent,
            task_fingerprint: fp,
            base_commit: self.base_commit,
            final_commit: None,
            files_read: self.files_read,
            files_changed: self.files_changed,
            symbols_read: self.symbols_read,
            symbols_changed: self.symbols_changed,
            tools_used: self.tools_used,
            diagnostics: self.diagnostics,
            tests_run: self.tests_run,
            approach_summary: truncate(&self.approach_summary, 400).to_string(),
            rejected_approaches: self.rejected_approaches,
            memories_used: self.memories_used,
            skills_used: self.skills_used,
            user_corrections: self.user_corrections,
            undo_count: self.undo_count,
            checkpoint_restores: self.checkpoint_restores,
            outcome,
            final_diff_hash,
            model: self.model,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            elapsed_ms,
        };

        persist_episode(identity, &episode);
        episode
    }
}

/// Persist an episode under the project's learning dir. Fail-open.
pub fn persist_episode(identity: &ProjectIdentity, episode: &CodingEpisode) {
    let paths = learning_store::ensure_project_learning(
        &identity.id,
        identity.remote.as_deref(),
        Some(&identity.workspace_hash),
    );
    learning_store::append_jsonl(&paths.episodes, episode, MAX_EPISODES);
}

/// Load recent episodes for a project (newest last). Skips bad lines.
pub fn load_episodes(project_id: &str) -> Vec<CodingEpisode> {
    let paths = learning_store::ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl(&paths.episodes)
}

/// Find episodes with fingerprint similarity ≥ `min_sim`, most similar first.
pub fn similar_episodes(
    project_id: &str,
    query: &TaskFingerprint,
    min_sim: f32,
    limit: usize,
) -> Vec<(f32, CodingEpisode)> {
    let mut scored: Vec<(f32, CodingEpisode)> = load_episodes(project_id)
        .into_iter()
        .map(|ep| {
            let s = task_fingerprint::fingerprint_similarity(query, &ep.task_fingerprint);
            (s, ep)
        })
        .filter(|(s, _)| *s >= min_sim)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// Convenience: start + finish a one-shot episode from turn evidence.
#[allow(clippy::too_many_arguments)]
pub fn record_turn_episode(
    workspace: &Path,
    user_intent: &str,
    tools_used: &[String],
    files_changed: &[String],
    files_read: &[String],
    outcome: EpisodeOutcome,
    undo_count: u32,
    model: Option<&str>,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
    elapsed_ms: Option<u64>,
) -> Option<CodingEpisode> {
    let identity = crate::project_identity::resolve_project_identity(workspace);
    let mut b = EpisodeBuilder::new(user_intent);
    b.model = model.map(str::to_string);
    b.tokens_in = tokens_in;
    b.tokens_out = tokens_out;
    b.undo_count = undo_count;
    if undo_count > 0 {
        b.user_corrections.push(FeedbackEvent {
            kind: "undo".into(),
            summary: format!("{undo_count} undo(s) during turn"),
            at: now_secs(),
        });
    }
    for t in tools_used {
        b.record_tool(t);
    }
    for f in files_read {
        b.record_file_read(f);
    }
    for f in files_changed {
        b.record_file_changed(f);
    }
    if let Some(ctx) = crate::git_ctx::read_git_context(workspace) {
        b.base_commit = Some(ctx.head_sha);
    }
    Some(b.finish(&identity, outcome, elapsed_ms, None))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn push_unique(v: &mut Vec<String>, item: &str, cap: usize) {
    let item = item.trim();
    if item.is_empty() {
        return;
    }
    if !v.iter().any(|x| x == item) {
        if v.len() < cap {
            v.push(item.to_string());
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use crate::project_identity::override_registry_path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "episodes-{}-{}-{}",
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn persist_and_load_episode() {
        let home = tmp();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _lr = override_learning_root(home.join("learning"));
        let _rserial = crate::project_identity::registry_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _rr = override_registry_path(home.join("project-registry.json"));
        let ws = tmp();

        let ep = record_turn_episode(
            &ws,
            "Extend memory tool schema",
            &["edit".into(), "bash".into()],
            &["core/src/memory.rs".into()],
            &["core/src/tools.rs".into()],
            EpisodeOutcome::SuccessVerified,
            0,
            Some("test-model"),
            Some(100),
            Some(50),
            Some(1200),
        )
        .unwrap();

        assert!(ep.id.starts_with("ep-"));
        assert_eq!(ep.outcome, EpisodeOutcome::SuccessVerified);
        assert!(!ep.task_fingerprint.intent.is_empty());
        // Must not store giant blobs — tools list is compact.
        assert!(ep.tools_used.len() <= 8);

        let loaded = load_episodes(&ep.project_id);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, ep.id);
    }

    #[test]
    fn undo_creates_correction_event() {
        let home = tmp();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _lr = override_learning_root(home.join("learning"));
        let _rserial = crate::project_identity::registry_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _rr = override_registry_path(home.join("project-registry.json"));
        let ws = tmp();

        let ep = record_turn_episode(
            &ws,
            "Refactor everything",
            &["edit".into()],
            &["core/src/main.rs".into()],
            &[],
            EpisodeOutcome::Reverted,
            2,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(ep.undo_count, 2);
        assert!(!ep.user_corrections.is_empty());
        assert_eq!(ep.user_corrections[0].kind, "undo");
    }

    #[test]
    fn unverified_success_is_not_strong() {
        assert!(!EpisodeOutcome::SuccessUnverified.is_strong_success());
        assert!(EpisodeOutcome::SuccessVerified.is_strong_success());
        assert!(!EpisodeOutcome::FailedTests.is_strong_success());
    }

    #[test]
    fn full_command_output_not_stored() {
        let mut b = EpisodeBuilder::new("run tests");
        let huge = "E".repeat(10_000);
        b.record_test("cargo test memory", false, Some(&huge));
        assert!(b.tests_run[0].failure_signature.as_ref().unwrap().len() <= 160);
    }
}
