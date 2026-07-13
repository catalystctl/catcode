//! Memory hygiene: write policy (trivia / importance / conflict) and
//! consolidate/dedupe of related notes.
//!
//! Kept separate from the store so the markdown notebook stays simple while
//! saves go through a deterministic gate and the store can be periodically
//! compacted without an embedding model.

use crate::memory::{
    forget_memory_scoped, get_memory_scoped, save_memory_scoped, scan_memories_scoped,
    significant_tokens, slugify_public, Importance, MemoryEntry, Scope, SAVE_COUNT_WARN_THRESHOLD,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Jaccard threshold for considering two memories near-duplicates.
pub const CONSOLIDATE_SIMILARITY: f64 = 0.55;
/// Auto-consolidate when total memory count reaches this (same soft-warn band).
pub const AUTO_CONSOLIDATE_MIN_COUNT: usize = SAVE_COUNT_WARN_THRESHOLD;
/// Minimum seconds between auto-consolidates for a workspace.
const AUTO_CONSOLIDATE_COOLDOWN_SECS: u64 = 3600;

/// Minimum durable content length (chars) for a non-forced save of type `note`.
const MIN_NOTE_CHARS: usize = 12;

#[derive(Clone, Debug)]
pub enum WriteVerdict {
    Allow { warnings: Vec<String> },
    Reject { reason: String },
    Conflict { reason: String },
}

#[derive(Clone, Debug, Default)]
pub struct ConsolidateReport {
    pub merged: Vec<(String, String)>, // (survivor, absorbed)
    pub skipped: usize,
    pub message: String,
}

/// Evaluate whether a save/append should proceed.
pub fn evaluate_write(
    name: &str,
    content: &str,
    mem_type: &str,
    importance: Importance,
    existing: Option<&MemoryEntry>,
    force: bool,
) -> WriteVerdict {
    let content = content.trim();
    let name = name.trim();
    let mut warnings = Vec::new();

    if name.is_empty() {
        return WriteVerdict::Reject {
            reason: "memory name must not be empty".into(),
        };
    }
    if content.is_empty() {
        return WriteVerdict::Reject {
            reason: "memory content must not be empty".into(),
        };
    }

    if !force {
        if let Some(reason) = trivia_reason(content, mem_type, importance) {
            return WriteVerdict::Reject { reason };
        }
    } else if trivia_reason(content, mem_type, importance).is_some() {
        warnings.push("forced write of low-value/trivia content".into());
    }

    if let Some(existing) = existing {
        if content_already_present(&existing.content, content) {
            return WriteVerdict::Reject {
                reason: format!(
                    "content already present in memory '{}' — skipped duplicate append",
                    existing.name
                ),
            };
        }
        if !force {
            if let Some(reason) = conflict_reason(existing, content) {
                return WriteVerdict::Conflict { reason };
            }
        } else if conflict_reason(existing, content).is_some() {
            warnings.push(
                "forced write despite possible conflict with existing memory — review later".into(),
            );
        }
    }

    if importance == Importance::Low && !force {
        warnings.push(
            "importance=low — prefer durable convention/decision/gotcha types for long-lived facts"
                .into(),
        );
    }

    WriteVerdict::Allow { warnings }
}

fn trivia_reason(content: &str, mem_type: &str, importance: Importance) -> Option<String> {
    let t = mem_type.trim().to_lowercase();
    // High-signal types are allowed even when short (e.g. "use tabs").
    let durable_type = matches!(
        t.as_str(),
        "convention" | "decision" | "user" | "identity" | "preference" | "architecture" | "gotcha"
    );
    if importance == Importance::High || durable_type {
        if content.chars().count() < 3 {
            return Some("content too short to be a durable memory".into());
        }
        return ephemeral_phrase_reason(content);
    }

    if importance == Importance::Low {
        return Some(
            "refusing importance=low write — use force=true only if you truly need it, \
             or raise importance / use a durable type (convention/decision/gotcha)"
                .into(),
        );
    }

    if content.chars().count() < MIN_NOTE_CHARS {
        return Some(format!(
            "content too short (<{MIN_NOTE_CHARS} chars) for a durable note — \
             use a durable type, raise importance, or force=true"
        ));
    }

    ephemeral_phrase_reason(content)
}

fn ephemeral_phrase_reason(content: &str) -> Option<String> {
    let lower = content.to_lowercase();
    let trimmed = lower.trim();
    // Whole-content trivia.
    if matches!(
        trimmed,
        "ok" | "okay"
            | "done"
            | "wip"
            | "temp"
            | "tmp"
            | "asdf"
            | "test"
            | "testing"
            | "hello"
            | "hi"
            | "thanks"
            | "ty"
    ) {
        return Some("refusing trivia content — not durable across sessions".into());
    }
    // Ephemeral task-state patterns.
    const EPHEMERAL: &[&str] = &[
        "currently working",
        "working on this",
        "in this session",
        "right now i",
        "just for now",
        "temporary hack",
        "will fix later",
        "todo:",
        "remind me to",
        "user asked me to look",
    ];
    for p in EPHEMERAL {
        if lower.contains(p) {
            return Some(format!(
                "refusing ephemeral/task-state content (matched '{p}') — \
                 persist durable facts only; use force=true to override"
            ));
        }
    }
    None
}

fn content_already_present(existing: &str, new_facts: &str) -> bool {
    let norm_existing = normalize_for_dedupe(existing);
    let norm_new = normalize_for_dedupe(new_facts);
    if norm_new.is_empty() {
        return true;
    }
    if norm_existing.contains(&norm_new) {
        return true;
    }
    // Line-level: every non-empty new line already present.
    let existing_lines: HashSet<String> = existing
        .lines()
        .map(normalize_for_dedupe)
        .filter(|l| !l.is_empty() && l != "--- appended ---")
        .collect();
    let new_lines: Vec<String> = new_facts
        .lines()
        .map(normalize_for_dedupe)
        .filter(|l| !l.is_empty())
        .collect();
    !new_lines.is_empty() && new_lines.iter().all(|l| existing_lines.contains(l))
}

fn normalize_for_dedupe(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Detect simple polarity conflicts between existing memory and new content.
fn conflict_reason(existing: &MemoryEntry, new_content: &str) -> Option<String> {
    let old_claims = polarity_claims(&format!(
        "{} {} {}",
        existing.name, existing.description, existing.content
    ));
    let new_claims = polarity_claims(new_content);
    if old_claims.is_empty() || new_claims.is_empty() {
        return None;
    }
    for (pol_new, topic_new) in &new_claims {
        for (pol_old, topic_old) in &old_claims {
            if pol_new == pol_old {
                continue;
            }
            if topics_overlap(topic_new, topic_old) {
                return Some(format!(
                    "possible conflict with memory '{}': existing suggests {} '{}', \
                     new content suggests {} '{}' — resolve explicitly or pass force=true",
                    existing.name,
                    polarity_label(*pol_old),
                    topic_old,
                    polarity_label(*pol_new),
                    topic_new
                ));
            }
        }
    }
    None
}

fn polarity_label(positive: bool) -> &'static str {
    if positive {
        "prefer/always"
    } else {
        "avoid/never"
    }
}

/// Extract (is_positive, topic) claims from prose using light heuristics.
fn polarity_claims(text: &str) -> Vec<(bool, String)> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    // Patterns: "<polarity-word> <topic…>"
    let rules: &[(&str, bool)] = &[
        ("always ", true),
        ("never ", false),
        ("prefer ", true),
        ("preferred ", true),
        ("avoid ", false),
        ("do not use ", false),
        ("don't use ", false),
        ("must not ", false),
        ("must use ", true),
        ("use ", true),
        ("don't ", false),
        ("do not ", false),
    ];
    for (pat, positive) in rules {
        let mut rest = lower.as_str();
        while let Some(idx) = rest.find(pat) {
            let after = &rest[idx + pat.len()..];
            let topic = after
                .split(['.', ';', '\n', ','])
                .next()
                .unwrap_or("")
                .trim();
            let topic = topic
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            if topic.chars().count() >= 3 {
                out.push((*positive, topic));
            }
            rest = &rest[idx + pat.len()..];
        }
    }
    out
}

fn topics_overlap(a: &str, b: &str) -> bool {
    let ta: HashSet<String> = significant_tokens(a).into_iter().collect();
    let tb: HashSet<String> = significant_tokens(b).into_iter().collect();
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    let inter = ta.intersection(&tb).count();
    inter >= 1 && (inter as f64 / ta.len().min(tb.len()) as f64) >= 0.5
}

/// Consolidate near-duplicate memories within a scope (or both when `scope` is None).
pub fn consolidate(workspace: &Path, scope: Option<Scope>) -> Result<ConsolidateReport, String> {
    match scope {
        Some(s) => consolidate_scope(workspace, s),
        None => {
            let mut report = consolidate_scope(workspace, Scope::Workspace)?;
            let global = consolidate_scope(workspace, Scope::Global)?;
            report.merged.extend(global.merged);
            report.skipped += global.skipped;
            report.message = format_report(&report);
            // Stamp cooldown after a full pass.
            stamp_consolidate(workspace);
            Ok(report)
        }
    }
}

fn consolidate_scope(workspace: &Path, scope: Scope) -> Result<ConsolidateReport, String> {
    let mut entries = scan_memories_scoped(workspace, scope);
    let mut report = ConsolidateReport::default();
    if entries.len() < 2 {
        report.message = format!("nothing to consolidate in {} scope", scope.as_str());
        return Ok(report);
    }

    // Also dedupe repeated appended blocks inside each file first.
    for e in &entries {
        if let Some(cleaned) = dedupe_appended_blocks(&e.content) {
            if cleaned != e.content {
                let _ = save_memory_scoped(
                    workspace,
                    scope,
                    &e.name,
                    &cleaned,
                    &e.mem_type,
                    &e.description,
                );
            }
        }
    }
    entries = scan_memories_scoped(workspace, scope);

    // Greedy pairwise merge: highest similarity first.
    let mut absorbed: HashSet<String> = HashSet::new();
    let ids: Vec<String> = entries.iter().map(|e| slugify_public(&e.name)).collect();

    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let sim = jaccard_similarity(&entries[i], &entries[j]);
            if sim >= CONSOLIDATE_SIMILARITY {
                pairs.push((i, j, sim));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    for (i, j, _sim) in pairs {
        let a_id = &ids[i];
        let b_id = &ids[j];
        if absorbed.contains(a_id) || absorbed.contains(b_id) {
            report.skipped += 1;
            continue;
        }
        // Prefer pinned, then higher importance, then longer content, then name.
        let (survivor_idx, loser_idx) = pick_survivor(&entries[i], i, &entries[j], j);
        let survivor = &entries[survivor_idx];
        let loser = &entries[loser_idx];
        let survivor_name = survivor.name.clone();
        let loser_name = loser.name.clone();
        let loser_id = slugify_public(&loser_name);
        let survivor_id = slugify_public(&survivor_name);

        let merge_blob = unique_append_content(&survivor.content, &loser.content);
        let desc = if survivor.description.trim().is_empty() {
            loser.description.clone()
        } else {
            survivor.description.clone()
        };
        let mem_type = if survivor.mem_type.trim().is_empty() {
            loser.mem_type.clone()
        } else {
            survivor.mem_type.clone()
        };

        let winner_importance = survivor.importance;
        crate::memory::save_memory_scoped_with_importance(
            workspace,
            scope,
            &survivor_name,
            &merge_blob,
            &mem_type,
            &desc,
            winner_importance,
        )?;
        forget_memory_scoped(workspace, scope, &loser_id)?;
        absorbed.insert(loser_id);
        report
            .merged
            .push((survivor_id, slugify_public(&loser_name)));
    }

    report.message = format_report(&report);
    Ok(report)
}

fn format_report(report: &ConsolidateReport) -> String {
    if report.merged.is_empty() {
        "consolidate: no near-duplicates found".into()
    } else {
        let mut lines = vec![format!(
            "consolidate: merged {} pair(s)",
            report.merged.len()
        )];
        for (surv, abs) in &report.merged {
            lines.push(format!("  - kept '{surv}', absorbed '{abs}'"));
        }
        lines.join("\n")
    }
}

fn pick_survivor(a: &MemoryEntry, ai: usize, b: &MemoryEntry, bi: usize) -> (usize, usize) {
    let score = |e: &MemoryEntry| -> (i32, i32, usize, String) {
        (
            if e.pinned { 1 } else { 0 },
            match e.importance {
                Importance::High => 2,
                Importance::Normal => 1,
                Importance::Low => 0,
            },
            e.content.len(),
            e.name.clone(),
        )
    };
    let sa = score(a);
    let sb = score(b);
    // Higher pin/importance/len wins; for equal, lexicographically smaller name wins (stable).
    if sa.0 != sb.0 {
        return if sa.0 > sb.0 { (ai, bi) } else { (bi, ai) };
    }
    if sa.1 != sb.1 {
        return if sa.1 > sb.1 { (ai, bi) } else { (bi, ai) };
    }
    if sa.2 != sb.2 {
        return if sa.2 > sb.2 { (ai, bi) } else { (bi, ai) };
    }
    if sa.3 <= sb.3 {
        (ai, bi)
    } else {
        (bi, ai)
    }
}

fn jaccard_similarity(a: &MemoryEntry, b: &MemoryEntry) -> f64 {
    // Same type preferred; different types get a penalty unless both are note-like.
    let ta = a.mem_type.trim().to_lowercase();
    let tb = b.mem_type.trim().to_lowercase();
    let type_ok = ta == tb
        || ta.is_empty()
        || tb.is_empty()
        || (matches!(ta.as_str(), "note" | "session") && matches!(tb.as_str(), "note" | "session"));
    if !type_ok {
        return 0.0;
    }
    let sa: HashSet<String> =
        significant_tokens(&format!("{} {} {}", a.name, a.description, a.content))
            .into_iter()
            .collect();
    let sb: HashSet<String> =
        significant_tokens(&format!("{} {} {}", b.name, b.description, b.content))
            .into_iter()
            .collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn unique_append_content(survivor: &str, loser: &str) -> String {
    if content_already_present(survivor, loser) {
        return survivor.to_string();
    }
    let mut out = survivor.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n--- consolidated ---\n");
    // Drop loser lines already present.
    let existing_lines: HashSet<String> = survivor
        .lines()
        .map(normalize_for_dedupe)
        .filter(|l| !l.is_empty())
        .collect();
    for line in loser.lines() {
        let n = normalize_for_dedupe(line);
        if n.is_empty() || n == "--- appended ---" || n == "--- consolidated ---" {
            continue;
        }
        if existing_lines.contains(&n) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Remove duplicate appended sections / repeated lines inside one memory body.
fn dedupe_appended_blocks(content: &str) -> Option<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out_lines: Vec<&str> = Vec::new();
    let mut changed = false;
    for line in content.lines() {
        let n = normalize_for_dedupe(line);
        if n.is_empty() {
            out_lines.push(line);
            continue;
        }
        if n == "--- appended ---" || n == "--- consolidated ---" {
            out_lines.push(line);
            continue;
        }
        if !seen.insert(n) {
            changed = true;
            continue;
        }
        out_lines.push(line);
    }
    if !changed {
        return None;
    }
    Some(out_lines.join("\n"))
}

/// Auto-run consolidate when the store is large and cooldown has elapsed.
/// Returns a human message when work was done.
pub fn maybe_auto_consolidate(workspace: &Path) -> Option<String> {
    let count = crate::memory::memory_count(workspace);
    if count < AUTO_CONSOLIDATE_MIN_COUNT {
        return None;
    }
    if !cooldown_elapsed(workspace) {
        return None;
    }
    match consolidate(workspace, None) {
        Ok(report) if !report.merged.is_empty() => Some(report.message),
        Ok(_) => {
            stamp_consolidate(workspace);
            None
        }
        Err(_) => None,
    }
}

fn meta_path(workspace: &Path) -> PathBuf {
    let hash = crate::memory::project_hash(&workspace.to_string_lossy());
    // Prefer the same store root the memory module uses (honors test override).
    crate::memory::memory_store_root_public()
        .join(hash)
        .join(".hygiene.json")
}

fn cooldown_elapsed(workspace: &Path) -> bool {
    let path = meta_path(workspace);
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let last = raw
        .lines()
        .find_map(|l| l.strip_prefix("last_consolidate="))
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    now_secs().saturating_sub(last) >= AUTO_CONSOLIDATE_COOLDOWN_SECS
}

fn stamp_consolidate(workspace: &Path) {
    let path = meta_path(workspace);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!("last_consolidate={}\n", now_secs());
    let _ = crate::fsutil::atomic_write_str(&path, &body);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Resolve existing memory for policy checks (scoped).
pub fn existing_for_write(workspace: &Path, scope: Scope, name: &str) -> Option<MemoryEntry> {
    get_memory_scoped(workspace, scope, name).ok()
}

/// Convenience: run evaluate_write then map to Result with force semantics.
pub fn gate_write(
    workspace: &Path,
    scope: Scope,
    name: &str,
    content: &str,
    mem_type: &str,
    importance: Importance,
    force: bool,
) -> Result<Vec<String>, String> {
    let existing = existing_for_write(workspace, scope, name);
    match evaluate_write(
        name,
        content,
        mem_type,
        importance,
        existing.as_ref(),
        force,
    ) {
        WriteVerdict::Allow { warnings } => Ok(warnings),
        WriteVerdict::Reject { reason } | WriteVerdict::Conflict { reason } => Err(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{save_memory_scoped, Importance, Scope};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(test)]
    fn with_temp_store<R>(label: &str, f: impl FnOnce(&Path) -> R) -> R {
        let _serial = crate::memory::memory_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = AtomicU64::new(0).fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("catalyst_hygiene_{label}_{n}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let _guard = crate::memory::override_memory_root(root.clone());
        let ws = root.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        f(&ws)
    }

    #[test]
    fn rejects_trivia() {
        let v = evaluate_write("x", "ok", "note", Importance::Normal, None, false);
        assert!(matches!(v, WriteVerdict::Reject { .. }));
        let v = evaluate_write(
            "x",
            "currently working on the login form",
            "note",
            Importance::Normal,
            None,
            false,
        );
        assert!(matches!(v, WriteVerdict::Reject { .. }));
    }

    #[test]
    fn allows_durable_short_convention() {
        let v = evaluate_write(
            "indent",
            "use tabs",
            "convention",
            Importance::High,
            None,
            false,
        );
        assert!(matches!(v, WriteVerdict::Allow { .. }));
    }

    #[test]
    fn detects_conflict() {
        let existing = MemoryEntry {
            name: "indent".into(),
            description: "spacing".into(),
            mem_type: "convention".into(),
            content: "always use tabs".into(),
            path: PathBuf::from("/fake/indent.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
        };
        let v = evaluate_write(
            "indent",
            "never use tabs — prefer spaces",
            "convention",
            Importance::High,
            Some(&existing),
            false,
        );
        assert!(matches!(v, WriteVerdict::Conflict { .. }), "{v:?}");
        let forced = evaluate_write(
            "indent",
            "never use tabs — prefer spaces",
            "convention",
            Importance::High,
            Some(&existing),
            true,
        );
        assert!(matches!(forced, WriteVerdict::Allow { .. }));
    }

    #[test]
    fn skips_duplicate_append() {
        let existing = MemoryEntry {
            name: "rules".into(),
            description: "".into(),
            mem_type: "note".into(),
            content: "No panics in production code".into(),
            path: PathBuf::from("/fake/rules.md"),
            scope: Scope::Workspace,
            pinned: false,
            importance: Importance::Normal,
        };
        let v = evaluate_write(
            "rules",
            "No panics in production code",
            "note",
            Importance::Normal,
            Some(&existing),
            false,
        );
        assert!(matches!(v, WriteVerdict::Reject { .. }));
    }

    #[test]
    fn consolidate_merges_near_duplicates() {
        with_temp_store("cons", |ws| {
            save_memory_scoped(
                ws,
                Scope::Workspace,
                "tabs-rule",
                "always use tabs for indentation in rust files",
                "convention",
                "always use tabs for indentation",
            )
            .unwrap();
            save_memory_scoped(
                ws,
                Scope::Workspace,
                "indent-tabs",
                "always use tabs for indentation in rust files — never spaces",
                "convention",
                "always use tabs for indentation",
            )
            .unwrap();
            let report = consolidate(ws, Some(Scope::Workspace)).unwrap();
            assert!(
                !report.merged.is_empty(),
                "expected merge, got: {} (entries={:?})",
                report.message,
                crate::memory::scan_memories_scoped(ws, Scope::Workspace)
                    .iter()
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
            );
            let left = crate::memory::scan_memories_scoped(ws, Scope::Workspace);
            assert_eq!(left.len(), 1);
        });
    }
}
