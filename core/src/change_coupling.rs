//! File- and symbol-change coupling from git history (spec §10).
//!
//! Learns which files (and declared symbols) historically change together.
//! Bulk formatting commits (>20 files) are ignored. Recent commits weigh more
//! (linear decay). Fail-open: missing git or timeouts never abort a coding turn.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, ProjectLearningPaths};

const MAX_COMMIT_FILES: usize = 20;
const GIT_LOG_N: usize = 80;
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COUPLING_LINES: usize = 5000;
const MAX_SYMBOLS_PER_FILE: usize = 5;

/// One directed coupling edge: `trigger` often changes with `companion`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CouplingEdge {
    pub trigger: String,
    pub companion: String,
    pub confidence: f32,
    pub supporting_commits: u32,
}

/// Symbol-level coupling: symbols declared in files that co-change.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SymbolCouplingEdge {
    pub trigger_symbol: String,
    pub companion_symbol: String,
    pub trigger_path: String,
    pub companion_path: String,
    pub confidence: f32,
    pub supporting_commits: u32,
}

#[derive(Default)]
struct PairAccum {
    weight: f32,
    commits: u32,
}

/// Aggregate couplings from commit file lists (newest-first).
///
/// Commits with more than 20 files are ignored. Commit weight decays linearly:
/// newest = 1.0, oldest = 1/n.
pub fn aggregate_couplings(commits: &[Vec<String>], min_support: u32) -> Vec<CouplingEdge> {
    let n = commits.len();
    if n == 0 {
        return Vec::new();
    }

    let mut pairs: HashMap<(String, String), PairAccum> = HashMap::new();
    let mut trigger_weight: HashMap<String, f32> = HashMap::new();

    for (i, files) in commits.iter().enumerate() {
        if files.len() > MAX_COMMIT_FILES || files.len() < 2 {
            continue;
        }
        // Newest (i=0) → weight 1.0; oldest → 1/n.
        let weight = (n - i) as f32 / n as f32;
        let mut uniq: Vec<String> = files.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() < 2 || uniq.len() > MAX_COMMIT_FILES {
            continue;
        }
        for a in &uniq {
            *trigger_weight.entry(a.clone()).or_insert(0.0) += weight;
        }
        for i_a in 0..uniq.len() {
            for i_b in 0..uniq.len() {
                if i_a == i_b {
                    continue;
                }
                let key = (uniq[i_a].clone(), uniq[i_b].clone());
                let acc = pairs.entry(key).or_default();
                acc.weight += weight;
                acc.commits += 1;
            }
        }
    }

    let mut edges: Vec<CouplingEdge> = pairs
        .into_iter()
        .filter(|(_, acc)| acc.commits >= min_support)
        .map(|((trigger, companion), acc)| {
            let denom = trigger_weight
                .get(&trigger)
                .copied()
                .unwrap_or(acc.weight)
                .max(1e-6);
            let confidence = (acc.weight / denom).clamp(0.0, 1.0);
            CouplingEdge {
                trigger,
                companion,
                confidence,
                supporting_commits: acc.commits,
            }
        })
        .collect();

    edges.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.supporting_commits.cmp(&a.supporting_commits))
            .then_with(|| a.trigger.cmp(&b.trigger))
            .then_with(|| a.companion.cmp(&b.companion))
    });
    edges
}

/// Run `git log`, aggregate couplings, rewrite `index/coupling.jsonl`.
/// Returns number of edges written. Fail-open → 0.
/// Also refreshes symbol coupling (fail-open) so callers need not wire it.
pub fn refresh_coupling(workspace: &Path, project_id: &str) -> usize {
    let Some(raw) = run_git_log(workspace) else {
        return 0;
    };
    let commits = parse_git_log(&raw);
    let edges = aggregate_couplings(&commits, 2);
    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let out = paths.index_dir.join("coupling.jsonl");
    rewrite_coupling_jsonl(&out, &edges);
    let _ = refresh_symbol_coupling_from_commits(project_id, &commits);
    edges.len()
}

/// Refresh symbol-level coupling from git history + indexed symbols.
/// Writes `index/symbol-coupling.jsonl`. Fail-open → 0.
pub fn refresh_symbol_coupling(workspace: &Path, project_id: &str) -> usize {
    let Some(raw) = run_git_log(workspace) else {
        return 0;
    };
    let commits = parse_git_log(&raw);
    refresh_symbol_coupling_from_commits(project_id, &commits)
}

fn refresh_symbol_coupling_from_commits(project_id: &str, commits: &[Vec<String>]) -> usize {
    let symbols_by_path = load_symbols_by_path(project_id);
    let edges = aggregate_symbol_couplings(commits, &symbols_by_path, 2);
    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let out = paths.index_dir.join("symbol-coupling.jsonl");
    rewrite_symbol_coupling_jsonl(&out, &edges);
    edges.len()
}

fn load_symbols_by_path(project_id: &str) -> HashMap<String, Vec<String>> {
    let paths = ProjectLearningPaths::resolve(project_id);
    let records: Vec<crate::codebase_index::SymbolRecord> =
        learning_store::read_jsonl(&paths.index_dir.join("symbols.jsonl"));
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for s in records {
        let entry = map.entry(s.path).or_default();
        if !entry.iter().any(|n| n == &s.name) {
            entry.push(s.name);
        }
    }
    map
}

/// Aggregate symbol couplings from commit file lists (newest-first).
///
/// For each co-changing file pair, emit symbol pairs from declarations in those
/// files (capped at 5 symbols/file). Same bulk-ignore and recency weights as
/// file coupling.
pub fn aggregate_symbol_couplings(
    commits: &[Vec<String>],
    symbols_by_path: &HashMap<String, Vec<String>>,
    min_support: u32,
) -> Vec<SymbolCouplingEdge> {
    let n = commits.len();
    if n == 0 {
        return Vec::new();
    }

    #[derive(Default)]
    struct SymAccum {
        weight: f32,
        commits: u32,
    }

    let mut pairs: HashMap<(String, String, String, String), SymAccum> = HashMap::new();
    let mut trigger_weight: HashMap<(String, String), f32> = HashMap::new();

    for (i, files) in commits.iter().enumerate() {
        if files.len() > MAX_COMMIT_FILES || files.len() < 2 {
            continue;
        }
        let weight = (n - i) as f32 / n as f32;
        let mut uniq: Vec<String> = files.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() < 2 || uniq.len() > MAX_COMMIT_FILES {
            continue;
        }

        // Cap symbols per file for this commit.
        let syms_for: Vec<(String, Vec<String>)> = uniq
            .iter()
            .map(|path| {
                let mut syms = symbols_by_path.get(path).cloned().unwrap_or_default();
                syms.truncate(MAX_SYMBOLS_PER_FILE);
                (path.clone(), syms)
            })
            .collect();

        for (path, syms) in &syms_for {
            for sym in syms {
                *trigger_weight
                    .entry((sym.clone(), path.clone()))
                    .or_insert(0.0) += weight;
            }
        }

        for i_a in 0..syms_for.len() {
            for i_b in 0..syms_for.len() {
                if i_a == i_b {
                    continue;
                }
                let (ref path_a, ref syms_a) = syms_for[i_a];
                let (ref path_b, ref syms_b) = syms_for[i_b];
                if syms_a.is_empty() || syms_b.is_empty() {
                    continue;
                }
                for sa in syms_a {
                    for sb in syms_b {
                        let key = (sa.clone(), sb.clone(), path_a.clone(), path_b.clone());
                        let acc = pairs.entry(key).or_default();
                        acc.weight += weight;
                        acc.commits += 1;
                    }
                }
            }
        }
    }

    let mut edges: Vec<SymbolCouplingEdge> = pairs
        .into_iter()
        .filter(|(_, acc)| acc.commits >= min_support)
        .map(
            |((trigger_symbol, companion_symbol, trigger_path, companion_path), acc)| {
                let denom = trigger_weight
                    .get(&(trigger_symbol.clone(), trigger_path.clone()))
                    .copied()
                    .unwrap_or(acc.weight)
                    .max(1e-6);
                let confidence = (acc.weight / denom).clamp(0.0, 1.0);
                SymbolCouplingEdge {
                    trigger_symbol,
                    companion_symbol,
                    trigger_path,
                    companion_path,
                    confidence,
                    supporting_commits: acc.commits,
                }
            },
        )
        .collect();

    edges.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.supporting_commits.cmp(&a.supporting_commits))
            .then_with(|| a.trigger_symbol.cmp(&b.trigger_symbol))
            .then_with(|| a.companion_symbol.cmp(&b.companion_symbol))
    });
    edges
}

/// Load companions for `path` from stored coupling.jsonl, highest confidence first.
pub fn companions_for(project_id: &str, path: &str, limit: usize) -> Vec<CouplingEdge> {
    let paths = ProjectLearningPaths::resolve(project_id);
    let all: Vec<CouplingEdge> =
        learning_store::read_jsonl(&paths.index_dir.join("coupling.jsonl"));
    let mut matched: Vec<CouplingEdge> = all
        .into_iter()
        .filter(|e| e.trigger == path || e.trigger.ends_with(path) || path.ends_with(&e.trigger))
        .collect();
    matched.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matched.truncate(limit);
    matched
}

/// Load symbol companions for `symbol` from `symbol-coupling.jsonl`.
pub fn symbol_companions_for(
    project_id: &str,
    symbol: &str,
    limit: usize,
) -> Vec<SymbolCouplingEdge> {
    let paths = ProjectLearningPaths::resolve(project_id);
    let all: Vec<SymbolCouplingEdge> =
        learning_store::read_jsonl(&paths.index_dir.join("symbol-coupling.jsonl"));
    let mut matched: Vec<SymbolCouplingEdge> = all
        .into_iter()
        .filter(|e| e.trigger_symbol == symbol)
        .collect();
    matched.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.companion_symbol.cmp(&b.companion_symbol))
    });
    matched.truncate(limit);
    matched
}

fn rewrite_coupling_jsonl(path: &Path, edges: &[CouplingEdge]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
    let capped = if edges.len() > MAX_COUPLING_LINES {
        &edges[..MAX_COUPLING_LINES]
    } else {
        edges
    };
    let mut body = String::new();
    for e in capped {
        if let Ok(line) = serde_json::to_string(e) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let _ = crate::fsutil::atomic_write_str(path, &body);
}

fn rewrite_symbol_coupling_jsonl(path: &Path, edges: &[SymbolCouplingEdge]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
    let capped = if edges.len() > MAX_COUPLING_LINES {
        &edges[..MAX_COUPLING_LINES]
    } else {
        edges
    };
    let mut body = String::new();
    for e in capped {
        if let Ok(line) = serde_json::to_string(e) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let _ = crate::fsutil::atomic_write_str(path, &body);
}

fn run_git_log(workspace: &Path) -> Option<String> {
    let ws = workspace.to_str()?;
    let mut child = Command::new("git")
        .args([
            "-C",
            ws,
            "log",
            &format!("-n{GIT_LOG_N}"),
            "--name-only",
            "--pretty=format:===",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut pipe = child.stdout.take()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) if start.elapsed() >= GIT_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => return None,
        }
    }

    use std::io::Read;
    let mut stdout = String::new();
    let _ = pipe.read_to_string(&mut stdout);
    Some(stdout)
}

/// Parse `git log --name-only --pretty=format:===` output into per-commit path lists.
fn parse_git_log(raw: &str) -> Vec<Vec<String>> {
    let mut commits = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut in_commit = false;

    for line in raw.lines() {
        if line == "===" {
            if in_commit && !current.is_empty() {
                commits.push(std::mem::take(&mut current));
            } else if in_commit {
                current.clear();
            }
            in_commit = true;
            continue;
        }
        if !in_commit {
            continue;
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Skip likely non-path lines (commit subjects if format drifts).
        if !t.contains('/') && !t.contains('.') {
            continue;
        }
        current.push(t.to_string());
    }
    if in_commit && !current.is_empty() {
        commits.push(current);
    }
    commits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp_root() -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "coupling-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn aggregate_ignores_bulk_and_weights_recent() {
        let commits = vec![
            // newest: a+b together
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "c.rs".into()],
            // bulk commit ignored
            (0..25).map(|i| format!("bulk{i}.rs")).collect(),
        ];
        let edges = aggregate_couplings(&commits, 1);
        let ab = edges
            .iter()
            .find(|e| e.trigger == "a.rs" && e.companion == "b.rs")
            .expect("a→b edge");
        let ac = edges
            .iter()
            .find(|e| e.trigger == "a.rs" && e.companion == "c.rs")
            .expect("a→c edge");
        assert!(ab.supporting_commits >= 2);
        assert_eq!(ac.supporting_commits, 1);
        // a+b appears in newer commits → higher confidence than a+c
        assert!(ab.confidence >= ac.confidence);
        // bulk files must not appear
        assert!(!edges.iter().any(|e| e.trigger.starts_with("bulk")));
    }

    #[test]
    fn aggregate_min_support_filters() {
        let commits = vec![
            vec!["x.rs".into(), "y.rs".into()],
            vec!["x.rs".into(), "z.rs".into()],
        ];
        let edges = aggregate_couplings(&commits, 2);
        assert!(edges.is_empty());
    }

    #[test]
    fn companions_for_reads_store() {
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let pid = "project-coupling-test";
        let paths = learning_store::ensure_project_learning(pid, None, None);
        let edge = CouplingEdge {
            trigger: "core/src/protocol.rs".into(),
            companion: "sdk/src/core-events.ts".into(),
            confidence: 0.88,
            supporting_commits: 9,
        };
        rewrite_coupling_jsonl(&paths.index_dir.join("coupling.jsonl"), &[edge.clone()]);
        let got = companions_for(pid, "core/src/protocol.rs", 5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].companion, "sdk/src/core-events.ts");
        assert!((got[0].confidence - 0.88).abs() < 0.01);
    }

    #[test]
    fn parse_git_log_splits_commits() {
        let raw = "===\na.rs\nb.rs\n\n===\nc.rs\nd.rs\n";
        let commits = parse_git_log(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0], vec!["a.rs", "b.rs"]);
        assert_eq!(commits[1], vec!["c.rs", "d.rs"]);
    }

    #[test]
    fn aggregate_symbol_couplings_from_fake_map() {
        let commits = vec![
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "c.rs".into()],
            // bulk ignored
            (0..25).map(|i| format!("bulk{i}.rs")).collect(),
        ];
        let mut symbols: HashMap<String, Vec<String>> = HashMap::new();
        symbols.insert("a.rs".into(), vec!["Foo".into(), "Bar".into()]);
        symbols.insert("b.rs".into(), vec!["Baz".into()]);
        symbols.insert("c.rs".into(), vec!["Qux".into()]);

        let edges = aggregate_symbol_couplings(&commits, &symbols, 1);
        let foo_baz = edges
            .iter()
            .find(|e| {
                e.trigger_symbol == "Foo"
                    && e.companion_symbol == "Baz"
                    && e.trigger_path == "a.rs"
                    && e.companion_path == "b.rs"
            })
            .expect("Foo→Baz edge");
        assert!(foo_baz.supporting_commits >= 2);
        assert!(foo_baz.confidence > 0.0);

        let with_min2 = aggregate_symbol_couplings(&commits, &symbols, 2);
        assert!(with_min2
            .iter()
            .any(|e| { e.trigger_symbol == "Foo" && e.companion_symbol == "Baz" }));
        // a+c only once → filtered at min_support=2
        assert!(!with_min2
            .iter()
            .any(|e| { e.trigger_symbol == "Foo" && e.companion_symbol == "Qux" }));
        assert!(!edges.iter().any(|e| e.trigger_path.starts_with("bulk")));
    }

    #[test]
    fn symbol_companions_for_reads_store() {
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let pid = "project-sym-coupling-test";
        let paths = learning_store::ensure_project_learning(pid, None, None);
        let edge = SymbolCouplingEdge {
            trigger_symbol: "ProviderConfig".into(),
            companion_symbol: "PluginOAuthConfig".into(),
            trigger_path: "core/src/provider.rs".into(),
            companion_path: "core/src/plugins.rs".into(),
            confidence: 0.9,
            supporting_commits: 4,
        };
        rewrite_symbol_coupling_jsonl(
            &paths.index_dir.join("symbol-coupling.jsonl"),
            &[edge.clone()],
        );
        let got = symbol_companions_for(pid, "ProviderConfig", 5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].companion_symbol, "PluginOAuthConfig");
    }
}
