//! Memory recall telemetry — measure whether catalog/relevance surfaces lead to
//! `memory get`, and detect synonym misses (body matches prompt but name+
//! description keyword match does not).
//!
//! Storage: `~/.config/catalyst-code/memory-metrics/<workspace-hash>.jsonl`,
//! capped at [`MAX_ENTRIES`] (oldest trimmed). Turn-local state lives in
//! process memory so `get` mid-turn can be attributed to the open turn.
//!
//! Design (SELF_LEARNING.md): keyword synonym misses are the trigger for
//! Milestone 4 embedding retrieval — this module produces that signal.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::memory::{significant_tokens, MemoryEntry};

const MAX_ENTRIES: usize = 500;

/// In-flight recall tracking for the active turn, keyed by workspace hash.
fn turn_map() -> &'static Mutex<HashMap<String, TurnRecall>> {
    static TURN: OnceLock<Mutex<HashMap<String, TurnRecall>>> = OnceLock::new();
    TURN.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Debug, Default)]
struct TurnRecall {
    relevant: Vec<String>,
    synonym_misses: Vec<String>,
    got: HashSet<String>,
    prompt_preview: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct RecallEntry {
    kind: String,
    ts: u64,
    relevant: Vec<String>,
    synonym_misses: Vec<String>,
    got: Vec<String>,
    /// Relevant memories that were never fetched with `get` this turn.
    missed_relevant: Vec<String>,
    /// Synonym-miss memories that *were* fetched (good recovery).
    synonym_hits: Vec<String>,
    prompt_preview: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RecallSummary {
    pub turns: u64,
    pub relevant_offers: u64,
    pub relevant_gets: u64,
    pub relevant_misses: u64,
    pub synonym_miss_offers: u64,
    pub synonym_miss_gets: u64,
    /// Fraction of relevant offers that received a `get` (0..1). None if no offers.
    pub relevant_hit_rate: Option<f64>,
    /// Fraction of synonym-miss offers recovered via `get`.
    pub synonym_recovery_rate: Option<f64>,
}

/// Snapshot returned when a turn is finalized (also persisted).
#[derive(Clone, Debug, Default, Serialize)]
pub struct TurnRecallStats {
    pub relevant: Vec<String>,
    pub synonym_misses: Vec<String>,
    pub got: Vec<String>,
    pub missed_relevant: Vec<String>,
    pub synonym_hits: Vec<String>,
}

struct Store {
    root: PathBuf,
}

impl Store {
    fn default_root() -> PathBuf {
        crate::config::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/catalyst-code/memory-metrics")
    }

    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, workspace: &Path) -> PathBuf {
        let hash = crate::memory::project_hash(&workspace.to_string_lossy());
        self.root.join(format!("{hash}.jsonl"))
    }

    fn append(&self, workspace: &Path, entry: &RecallEntry) {
        let _ = std::fs::create_dir_all(&self.root);
        let path = self.path(workspace);
        let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
        let mut lines = read_lines(&path);
        let line = serde_json::to_string(entry).unwrap_or_default();
        if line.is_empty() {
            return;
        }
        lines.push(line);
        if lines.len() > MAX_ENTRIES {
            let drop = lines.len() - MAX_ENTRIES;
            lines.drain(0..drop);
        }
        let mut out = lines.join("\n");
        out.push('\n');
        let _ = crate::fsutil::atomic_write_str(&path, &out);
    }

    fn summary(&self, workspace: &Path) -> RecallSummary {
        let entries = read_entries(&self.path(workspace));
        let mut s = RecallSummary {
            turns: entries.len() as u64,
            ..RecallSummary::default()
        };
        for e in &entries {
            s.relevant_offers += e.relevant.len() as u64;
            s.relevant_misses += e.missed_relevant.len() as u64;
            s.synonym_miss_offers += e.synonym_misses.len() as u64;
            s.synonym_miss_gets += e.synonym_hits.len() as u64;
            let got_set: HashSet<&str> = e.got.iter().map(|x| x.as_str()).collect();
            for id in &e.relevant {
                if got_set.contains(id.as_str()) {
                    s.relevant_gets += 1;
                }
            }
        }
        if s.relevant_offers > 0 {
            s.relevant_hit_rate = Some(s.relevant_gets as f64 / s.relevant_offers as f64);
        }
        if s.synonym_miss_offers > 0 {
            s.synonym_recovery_rate =
                Some(s.synonym_miss_gets as f64 / s.synonym_miss_offers as f64);
        }
        s
    }
}

/// Begin (or replace) turn recall tracking for `workspace` against `prompt`.
/// Classifies memories into keyword-relevant vs synonym-miss (body-only).
pub fn begin_turn(workspace: &Path, prompt: &str, memories: &[MemoryEntry]) {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return;
    }
    let key = crate::memory::project_hash(&workspace.to_string_lossy());
    let mut relevant = Vec::new();
    let mut synonym_misses = Vec::new();
    for m in memories {
        let id = memory_id(m);
        let name_hit = crate::memory::is_name_relevant(m, prompt);
        let body_hit = is_body_relevant(m, prompt);
        if name_hit {
            relevant.push(id);
        } else if body_hit {
            synonym_misses.push(id);
        }
    }
    relevant.sort();
    relevant.dedup();
    synonym_misses.sort();
    synonym_misses.dedup();
    let preview = truncate(prompt, 160);
    let mut map = turn_map().lock().unwrap_or_else(|e| e.into_inner());
    map.insert(
        key,
        TurnRecall {
            relevant,
            synonym_misses,
            got: HashSet::new(),
            prompt_preview: preview,
        },
    );
}

/// Record a successful `memory get` against the open turn (best-effort).
pub fn record_get(workspace: &Path, id_or_name: &str) {
    let key = crate::memory::project_hash(&workspace.to_string_lossy());
    let slug = crate::memory::slugify_public(id_or_name);
    if slug.is_empty() {
        return;
    }
    let mut map = turn_map().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(turn) = map.get_mut(&key) {
        turn.got.insert(slug);
    }
}

/// Finalize the open turn: persist a recall record and return stats for metrics.
/// Returns `None` when no turn was tracked (empty prompt / provider path).
pub fn finalize_turn(workspace: &Path) -> Option<TurnRecallStats> {
    let key = crate::memory::project_hash(&workspace.to_string_lossy());
    let turn = {
        let mut map = turn_map().lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&key)?
    };
    // Skip empty turns (no offers and no gets) to keep the log useful.
    if turn.relevant.is_empty() && turn.synonym_misses.is_empty() && turn.got.is_empty() {
        return None;
    }
    let got: Vec<String> = {
        let mut v: Vec<String> = turn.got.iter().cloned().collect();
        v.sort();
        v
    };
    let got_set: HashSet<&str> = got.iter().map(|s| s.as_str()).collect();
    let missed_relevant: Vec<String> = turn
        .relevant
        .iter()
        .filter(|id| !got_set.contains(id.as_str()))
        .cloned()
        .collect();
    let synonym_hits: Vec<String> = turn
        .synonym_misses
        .iter()
        .filter(|id| got_set.contains(id.as_str()))
        .cloned()
        .collect();
    let stats = TurnRecallStats {
        relevant: turn.relevant.clone(),
        synonym_misses: turn.synonym_misses.clone(),
        got: got.clone(),
        missed_relevant: missed_relevant.clone(),
        synonym_hits: synonym_hits.clone(),
    };
    let entry = RecallEntry {
        kind: "recall".into(),
        ts: now_secs(),
        relevant: turn.relevant,
        synonym_misses: turn.synonym_misses,
        got,
        missed_relevant,
        synonym_hits,
        prompt_preview: turn.prompt_preview,
    };
    Store::new(Store::default_root()).append(workspace, &entry);
    Some(stats)
}

/// Aggregate recall quality for a workspace (for `memory action=stats`).
pub fn summary(workspace: &Path) -> RecallSummary {
    Store::new(Store::default_root()).summary(workspace)
}

/// Rolling synonym miss/hit counts for the embedding preference gate.
pub fn rolling_synonym_counts() -> (u64, u64) {
    // Best-effort: use the current process cwd as workspace; callers in
    // build_relevant_tail don't always have a Path, and empty is fine (gate off).
    let ws = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let s = summary(&ws);
    (s.synonym_miss_offers, s.synonym_miss_gets)
}

/// JSON-friendly summary for tool output / metrics.
pub fn summary_json(workspace: &Path) -> serde_json::Value {
    let s = summary(workspace);
    json!({
        "turns": s.turns,
        "relevant_offers": s.relevant_offers,
        "relevant_gets": s.relevant_gets,
        "relevant_misses": s.relevant_misses,
        "relevant_hit_rate": s.relevant_hit_rate,
        "synonym_miss_offers": s.synonym_miss_offers,
        "synonym_miss_gets": s.synonym_miss_gets,
        "synonym_recovery_rate": s.synonym_recovery_rate,
    })
}

fn memory_id(m: &MemoryEntry) -> String {
    m.path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::memory::slugify_public(&m.name))
}

fn is_body_relevant(entry: &MemoryEntry, prompt: &str) -> bool {
    if entry.content.trim().is_empty() {
        return false;
    }
    let prompt_tokens: HashSet<String> = significant_tokens(prompt).into_iter().collect();
    if prompt_tokens.is_empty() {
        return false;
    }
    let body_tokens = significant_tokens(&entry.content);
    for t in body_tokens {
        if prompt_tokens.contains(&t) {
            return true;
        }
    }
    false
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    let truncated: String = s.chars().take(n).collect();
    format!("{truncated}…")
}

fn read_lines(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn read_entries(path: &Path) -> Vec<RecallEntry> {
    read_lines(path)
        .into_iter()
        .filter_map(|l| serde_json::from_str::<RecallEntry>(&l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn with_temp_store<R>(name: &str, f: impl FnOnce(&Path) -> R) -> R {
        let _serial = crate::memory::memory_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = AtomicU64::new(0).fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("catalyst_recall_root_{name}_{n}"));
        let _ = std::fs::create_dir_all(&d);
        let _guard = crate::memory::override_memory_root(d.clone());
        let ws = d.join("ws");
        let _ = std::fs::create_dir_all(&ws);
        f(&ws)
    }

    fn entry(name: &str, desc: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            name: name.into(),
            description: desc.into(),
            mem_type: "note".into(),
            content: content.into(),
            path: PathBuf::from(format!("/fake/{}.md", crate::memory::slugify_public(name))),
            scope: crate::memory::Scope::Workspace,
            pinned: false,
            importance: crate::memory::Importance::Normal,
            deprecated: false,
            superseded_by: None,
        }
    }

    #[test]
    fn begin_classifies_relevant_and_synonym_miss() {
        with_temp_store("classify", |ws| {
            let memories = vec![
                entry(
                    "typescript-style",
                    "strict mode preferred",
                    "use tabs in TS files",
                ),
                entry(
                    "build-pipeline",
                    "ci conventions",
                    "jest is the test runner; prefer jest over mocha",
                ),
            ];
            begin_turn(
                ws,
                "please add jest unit tests for the component",
                &memories,
            );
            let stats = finalize_turn(ws).expect("tracked");
            assert!(
                stats.synonym_misses.iter().any(|id| id == "build-pipeline"),
                "jest body-only should be synonym miss: {:?}",
                stats.synonym_misses
            );
        });
    }

    #[test]
    fn get_marks_relevant_hit() {
        with_temp_store("hit", |ws| {
            let memories = vec![entry(
                "formatting",
                "indent and commas",
                "always use 2-space indent",
            )];
            begin_turn(ws, "adjust indent width please", &memories);
            record_get(ws, "formatting");
            let stats = finalize_turn(ws).unwrap();
            assert!(stats.got.contains(&"formatting".to_string()));
            assert!(stats.missed_relevant.is_empty());
        });
    }

    #[test]
    fn missed_relevant_when_no_get() {
        with_temp_store("miss", |ws| {
            let memories = vec![entry(
                "formatting",
                "indent rules",
                "always use 2-space indent",
            )];
            begin_turn(ws, "fix the indent please", &memories);
            let stats = finalize_turn(ws).unwrap();
            assert!(!stats.relevant.is_empty());
            assert_eq!(stats.missed_relevant, stats.relevant);
        });
    }
}
