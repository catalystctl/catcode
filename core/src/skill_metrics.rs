//! Skill utility metrics (spec §19).
//!
//! Tracks uses / success / correction / revert counts and promotes or demotes
//! skill stages. Stored as JSONL under `learning/global/skill-metrics.jsonl`.
//! Fail-open on I/O errors.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, GlobalLearningPaths, MAX_FEEDBACK};

/// Skill lifecycle stage (spec §19.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillStage {
    #[default]
    Candidate,
    Trusted,
    NeedsRevision,
    Deprecated,
}

impl SkillStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Trusted => "trusted",
            Self::NeedsRevision => "needs_revision",
            Self::Deprecated => "deprecated",
        }
    }
}

/// Outcome recorded for a skill activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeKind {
    Success,
    Corrected,
    Reverted,
}

/// Per-skill utility metrics record (spec §19.3 utility block).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SkillMetricsRecord {
    pub name: String,
    /// `"project"` or `"global"`.
    pub scope: String,
    pub stage: SkillStage,
    pub uses: u32,
    pub successful: u32,
    pub corrected: u32,
    pub reverted: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<u64>,
    #[serde(default)]
    pub task_intents: Vec<String>,
    #[serde(default)]
    pub evidence_episodes: Vec<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load all skill metrics (skips malformed lines).
pub fn load_skill_metrics() -> Vec<SkillMetricsRecord> {
    let paths = GlobalLearningPaths::resolve();
    learning_store::read_jsonl(&paths.skill_metrics)
}

/// Rewrite the skill-metrics JSONL from an in-memory set (fail-open).
fn save_all(records: &[SkillMetricsRecord]) {
    let paths = GlobalLearningPaths::resolve();
    let _ = paths.ensure();
    let _lock = crate::fsutil::FileLock::acquire(&paths.skill_metrics.with_extension("lock"));
    let mut body = String::new();
    for r in records.iter().take(MAX_FEEDBACK) {
        if let Ok(line) = serde_json::to_string(r) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let _ = crate::fsutil::atomic_write_str(&paths.skill_metrics, &body);
}

/// Find a metrics record by name, if any.
pub fn get_metrics(name: &str) -> Option<SkillMetricsRecord> {
    load_skill_metrics().into_iter().find(|r| r.name == name)
}

/// Ensure a record exists (creates Candidate stub if missing).
pub fn ensure_record(name: &str, scope: &str) -> SkillMetricsRecord {
    if let Some(r) = get_metrics(name) {
        return r;
    }
    let rec = SkillMetricsRecord {
        name: name.to_string(),
        scope: scope.to_string(),
        stage: SkillStage::Candidate,
        uses: 0,
        successful: 0,
        corrected: 0,
        reverted: 0,
        last_verified_at: None,
        task_intents: Vec::new(),
        evidence_episodes: Vec::new(),
    };
    let mut all = load_skill_metrics();
    all.push(rec.clone());
    save_all(&all);
    rec
}

/// Record an outcome and update stage per promotion rules (spec §19.4).
///
/// - Candidate after ≥2 successful (starting stage; stays until Trusted)
/// - Trusted after ≥3 successful and `reverted == 0` or `reverted < successful/3`
/// - NeedsRevision if Trusted and (`corrected >= 2` or `reverted >= 1`)
pub fn record_outcome(name: &str, kind: OutcomeKind) -> SkillMetricsRecord {
    let mut all = load_skill_metrics();
    let idx = match all.iter().position(|r| r.name == name) {
        Some(i) => i,
        None => {
            all.push(SkillMetricsRecord {
                name: name.to_string(),
                scope: "global".into(),
                stage: SkillStage::Candidate,
                uses: 0,
                successful: 0,
                corrected: 0,
                reverted: 0,
                last_verified_at: None,
                task_intents: Vec::new(),
                evidence_episodes: Vec::new(),
            });
            all.len() - 1
        }
    };

    {
        let rec = &mut all[idx];
        rec.uses = rec.uses.saturating_add(1);
        match kind {
            OutcomeKind::Success => {
                rec.successful = rec.successful.saturating_add(1);
                rec.last_verified_at = Some(now_secs());
            }
            OutcomeKind::Corrected => {
                rec.corrected = rec.corrected.saturating_add(1);
            }
            OutcomeKind::Reverted => {
                rec.reverted = rec.reverted.saturating_add(1);
            }
        }
        apply_stage_rules(rec);
    }

    let out = all[idx].clone();
    save_all(&all);
    out
}

fn apply_stage_rules(rec: &mut SkillMetricsRecord) {
    if rec.stage == SkillStage::Deprecated {
        return;
    }

    // Demotion from Trusted takes priority.
    if rec.stage == SkillStage::Trusted && (rec.corrected >= 2 || rec.reverted >= 1) {
        rec.stage = SkillStage::NeedsRevision;
        return;
    }

    // Promotion / confirmation.
    let revert_ok = rec.reverted == 0 || rec.reverted < rec.successful / 3;
    if rec.successful >= 3 && revert_ok {
        if rec.stage != SkillStage::NeedsRevision || rec.corrected == 0 {
            // Fresh Trusted only when not stuck in NeedsRevision with corrections.
            if rec.stage == SkillStage::Candidate || rec.stage == SkillStage::Trusted {
                rec.stage = SkillStage::Trusted;
            }
        }
    } else if rec.successful >= 2 {
        // Explicitly remain / confirm Candidate after two successes.
        if rec.stage != SkillStage::Trusted && rec.stage != SkillStage::NeedsRevision {
            rec.stage = SkillStage::Candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static N: AtomicU64 = AtomicU64::new(0);
    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn tmp_root() -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "skill-metrics-{}-{}-{}",
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn promotes_candidate_then_trusted() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);

        let r1 = record_outcome("extend-memory-tool", OutcomeKind::Success);
        assert_eq!(r1.stage, SkillStage::Candidate);
        assert_eq!(r1.successful, 1);

        let r2 = record_outcome("extend-memory-tool", OutcomeKind::Success);
        assert_eq!(r2.stage, SkillStage::Candidate);
        assert_eq!(r2.successful, 2);

        let r3 = record_outcome("extend-memory-tool", OutcomeKind::Success);
        assert_eq!(r3.stage, SkillStage::Trusted);
        assert_eq!(r3.successful, 3);
        assert_eq!(r3.uses, 3);
    }

    #[test]
    fn trusted_demotes_on_revert() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);

        for _ in 0..3 {
            record_outcome("fragile-skill", OutcomeKind::Success);
        }
        let trusted = get_metrics("fragile-skill").unwrap();
        assert_eq!(trusted.stage, SkillStage::Trusted);

        let demoted = record_outcome("fragile-skill", OutcomeKind::Reverted);
        assert_eq!(demoted.stage, SkillStage::NeedsRevision);
        assert_eq!(demoted.reverted, 1);
    }

    #[test]
    fn trusted_demotes_on_two_corrections() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);

        for _ in 0..3 {
            record_outcome("corrected-skill", OutcomeKind::Success);
        }
        record_outcome("corrected-skill", OutcomeKind::Corrected);
        let still = get_metrics("corrected-skill").unwrap();
        assert_eq!(still.stage, SkillStage::Trusted);

        let demoted = record_outcome("corrected-skill", OutcomeKind::Corrected);
        assert_eq!(demoted.stage, SkillStage::NeedsRevision);
        assert_eq!(demoted.corrected, 2);
    }

    #[test]
    fn revert_ratio_blocks_trust() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);

        record_outcome("flaky", OutcomeKind::Success);
        record_outcome("flaky", OutcomeKind::Reverted);
        record_outcome("flaky", OutcomeKind::Success);
        // successful=2, reverted=1 → not yet Trusted; still Candidate
        let r = record_outcome("flaky", OutcomeKind::Success);
        // successful=3, reverted=1; 1 < 3/3=1 is false, so revert_ok is false
        assert_eq!(r.successful, 3);
        assert_eq!(r.reverted, 1);
        assert_eq!(r.stage, SkillStage::Candidate);
    }
}
