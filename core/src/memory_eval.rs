//! Deterministic, local memory-retrieval evaluation harness.
//!
//! Fixtures contain synthetic facts only. The evaluator performs no network
//! access and writes no production memory state.

#![allow(dead_code)]

use serde::Deserialize;
use std::path::PathBuf;

use crate::learning_retrieval::{debug_score_memory, rank_memories, RetrievalScoreDebug};
use crate::memory::{Importance, MemoryEntry, MemoryStatus, Scope};
use crate::task_fingerprint::TaskFingerprint;

#[derive(Debug, Deserialize)]
pub struct EvalSuite {
    pub cases: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
pub struct EvalCase {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub fingerprint: TaskFingerprint,
    pub expected_top: String,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub max_total_tokens: Option<usize>,
    pub memories: Vec<EvalMemory>,
}

#[derive(Debug, Deserialize)]
pub struct EvalMemory {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_type")]
    pub mem_type: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub scope: EvalScope,
    #[serde(default)]
    pub status: EvalStatus,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub contradiction_count: u32,
    #[serde(default)]
    pub ref_files: Vec<String>,
    #[serde(default)]
    pub ref_symbols: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalScope {
    #[default]
    Workspace,
    Global,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatus {
    #[default]
    Verified,
    Candidate,
    NeedsVerification,
    Stale,
    Rejected,
}

#[derive(Debug)]
pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub ranked_names: Vec<String>,
    pub scores: Vec<(String, RetrievalScoreDebug)>,
    pub estimated_tokens: usize,
    pub budget_ok: bool,
}

fn default_type() -> String {
    "note".into()
}

fn default_confidence() -> f32 {
    1.0
}

impl EvalMemory {
    fn entry(&self) -> MemoryEntry {
        MemoryEntry {
            name: self.name.clone(),
            description: self.description.clone(),
            mem_type: self.mem_type.clone(),
            content: self.content.clone(),
            path: PathBuf::from(format!("{}.md", self.name)),
            scope: match self.scope {
                EvalScope::Workspace => Scope::Workspace,
                EvalScope::Global => Scope::Global,
            },
            pinned: false,
            importance: Importance::Normal,
            deprecated: matches!(self.status, EvalStatus::Rejected),
            superseded_by: None,
            schema_version: 2,
            source_session: None,
            source_run: None,
            created_at: None,
            status: match self.status {
                EvalStatus::Verified => MemoryStatus::Verified,
                EvalStatus::Candidate => MemoryStatus::Candidate,
                EvalStatus::NeedsVerification => MemoryStatus::NeedsVerification,
                EvalStatus::Stale => MemoryStatus::Stale,
                EvalStatus::Rejected => MemoryStatus::Rejected,
            },
            confidence: self.confidence,
            support_count: 0,
            contradiction_count: self.contradiction_count,
            last_verified_at: None,
            last_verified_commit: None,
            ref_files: self.ref_files.clone(),
            ref_symbols: self.ref_symbols.clone(),
            evidence_episodes: Vec::new(),
        }
    }
}

pub fn evaluate(suite: &EvalSuite) -> Vec<EvalResult> {
    suite
        .cases
        .iter()
        .map(|case| {
            let memories: Vec<MemoryEntry> = case.memories.iter().map(EvalMemory::entry).collect();
            let ranked = rank_memories(
                &memories,
                &case.prompt,
                &case.fingerprint,
                case.max_results.unwrap_or(memories.len()),
            );
            let ranked_names = ranked
                .iter()
                .map(|item| item.1.name.clone())
                .collect::<Vec<_>>();
            let scores = memories
                .iter()
                .map(|memory| {
                    (
                        memory.name.clone(),
                        debug_score_memory(memory, &case.prompt, &case.fingerprint),
                    )
                })
                .collect();
            let estimated_tokens = ranked
                .iter()
                .map(|(_, memory, _)| {
                    (memory.name.len() + memory.description.len() + memory.content.len())
                        .div_ceil(4)
                })
                .sum();
            let budget_ok = case
                .max_total_tokens
                .is_none_or(|budget| estimated_tokens <= budget);
            EvalResult {
                name: case.name.clone(),
                passed: ranked_names.first() == Some(&case.expected_top) && budget_ok,
                ranked_names,
                scores,
                estimated_tokens,
                budget_ok,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_retrieval_suite_meets_expected_rankings() {
        let suite: EvalSuite =
            serde_json::from_str(include_str!("../tests/fixtures/memory/retrieval-v1.json"))
                .expect("valid memory evaluation fixture");
        let results = evaluate(&suite);
        let failures = results
            .iter()
            .filter(|result| !result.passed)
            .map(|result| format!("{}: {:?}", result.name, result.ranked_names))
            .collect::<Vec<_>>();
        assert!(
            failures.is_empty(),
            "memory evaluation failures: {failures:?}"
        );

        let contradiction = results
            .iter()
            .find(|result| result.name == "contradictory memory suppression")
            .unwrap();
        let disputed = contradiction
            .scores
            .iter()
            .find(|score| score.0 == "disputed-build-command")
            .unwrap();
        assert!(disputed.1.contradiction_penalty > 0.0);
        assert!(results.iter().all(|result| result.budget_ok));
    }
}
