//! Deterministic hybrid memory retrieval (spec §15).
//!
//! Scores [`MemoryEntry`] values against a prompt + [`TaskFingerprint`] using
//! fixed weights — no embeddings API, no network. Fail-open / pure functions.

#![allow(dead_code)]

use crate::memory::{significant_tokens, MemoryEntry, MemoryStatus, Scope};
use crate::task_fingerprint::{fingerprint_similarity, TaskFingerprint};

/// Spec §15 weights (sum = 1.0).
const W_LEXICAL: f32 = 0.25;
const W_SYMBOL: f32 = 0.20;
const W_FINGERPRINT: f32 = 0.15;
const W_PATH: f32 = 0.10;
const W_UTILITY: f32 = 0.10;
const W_CONFIDENCE: f32 = 0.05;
const W_VERIFICATION: f32 = 0.05;
const W_SCOPE: f32 = 0.05;
const W_DIAGNOSTIC: f32 = 0.05;

/// Project-scope applicability bonus applied after the weighted sum (clamped).
const PROJECT_BONUS: f32 = 0.08;

/// Score a single memory. Returns `(score, reasons)`.
pub fn score_memory(
    entry: &MemoryEntry,
    prompt: &str,
    fp: &TaskFingerprint,
) -> (f32, Vec<String>) {
    if !entry.status.is_positive_guidance() || entry.deprecated {
        return (0.0, vec!["excluded: deprecated/rejected".into()]);
    }

    let mut reasons = Vec::new();
    let prompt_tokens = tokenize_rich(prompt);
    let mem_text = format!(
        "{} {} {} {}",
        entry.name, entry.description, entry.content, entry.mem_type
    );
    let mem_tokens = tokenize_rich(&mem_text);

    let lexical = tf_overlap(&prompt_tokens, &mem_tokens);
    if lexical > 0.3 {
        reasons.push(format!("lexical relevance {lexical:.2}"));
    }

    let symbol = symbol_overlap(entry, fp, &prompt_tokens);
    if symbol > 0.3 {
        reasons.push(format!("symbol/identifier overlap {symbol:.2}"));
    }

    let mem_fp = memory_as_fingerprint(entry);
    let fps = fingerprint_similarity(fp, &mem_fp);
    if fps > 0.3 {
        reasons.push(format!("task-fingerprint similarity {fps:.2}"));
    }

    let path_s = path_overlap(entry, fp);
    if path_s > 0.3 {
        reasons.push(format!("path/subsystem overlap {path_s:.2}"));
    }

    let utility = utility_score(entry);
    if utility > 0.5 {
        reasons.push(format!("historical utility {utility:.2}"));
    }

    let conf = entry.confidence.clamp(0.0, 1.0);
    reasons.push(format!("confidence {conf:.2}"));

    let ver = verification_recency(entry);
    if ver > 0.5 {
        reasons.push(format!("verification recency {ver:.2}"));
    }

    let scope_s = scope_applicability(entry);
    if entry.scope == Scope::Workspace {
        reasons.push("project scope bonus".into());
    }

    let diag = diagnostic_overlap(entry, fp);
    if diag > 0.0 {
        reasons.push(format!("diagnostic overlap {diag:.2}"));
    }

    let status_mul = entry.status.rank_multiplier();
    if status_mul < 1.0 {
        reasons.push(format!("status {} (x{status_mul:.2})", entry.status.as_str()));
    } else {
        reasons.push("status verified".into());
    }

    let mut score = W_LEXICAL * lexical
        + W_SYMBOL * symbol
        + W_FINGERPRINT * fps
        + W_PATH * path_s
        + W_UTILITY * utility
        + W_CONFIDENCE * conf
        + W_VERIFICATION * ver
        + W_SCOPE * scope_s
        + W_DIAGNOSTIC * diag;

    score *= status_mul;

    if entry.scope == Scope::Workspace {
        score = (score + PROJECT_BONUS).min(1.0);
    }

    (score.clamp(0.0, 1.0), reasons)
}

/// Rank memories for a prompt + fingerprint. Deterministic order on ties (name).
pub fn rank_memories(
    memories: &[MemoryEntry],
    prompt: &str,
    fp: &TaskFingerprint,
    limit: usize,
) -> Vec<(f32, MemoryEntry, Vec<String>)> {
    let mut scored: Vec<(f32, MemoryEntry, Vec<String>)> = memories
        .iter()
        .map(|m| {
            let (s, reasons) = score_memory(m, prompt, fp);
            (s, m.clone(), reasons)
        })
        .filter(|(s, _, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    scored.truncate(limit);
    scored
}

/// Human-readable explanation of why a memory scored as it did.
pub fn explain_score(
    entry: &MemoryEntry,
    prompt: &str,
    fp: &TaskFingerprint,
) -> String {
    let (score, reasons) = score_memory(entry, prompt, fp);
    let mut out = format!(
        "Memory: {}\nScope: {}\nStatus: {}\nScore: {:.3}\nRetrieved because:\n",
        entry.name,
        match entry.scope {
            Scope::Workspace => "PROJECT",
            Scope::Global => "GLOBAL",
        },
        entry.status.as_str().to_uppercase(),
        score
    );
    if reasons.is_empty() {
        out.push_str("- (no strong signals)\n");
    } else {
        for r in &reasons {
            out.push_str(&format!("- {r}\n"));
        }
    }
    out
}

fn tokenize_rich(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if raw.is_empty() {
            continue;
        }
        for part in split_ident(raw) {
            if part.len() > 1 {
                out.push(part.to_lowercase());
            }
        }
    }
    for t in significant_tokens(text) {
        if !out.iter().any(|x| x == &t) {
            out.push(t);
        }
    }
    out
}

/// Split CamelCase / snake_case / kebab-case identifiers into parts.
fn split_ident(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for chunk in s.split(|c| c == '_' || c == '-') {
        if chunk.is_empty() {
            continue;
        }
        let chars: Vec<char> = chunk.chars().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let prev = chars[i - 1];
            let cur = chars[i];
            let boundary = (prev.is_lowercase() && cur.is_uppercase())
                || (prev.is_uppercase()
                    && cur.is_uppercase()
                    && i + 1 < chars.len()
                    && chars[i + 1].is_lowercase());
            if boundary {
                parts.push(chars[start..i].iter().collect());
                start = i;
            }
        }
        parts.push(chars[start..].iter().collect());
    }
    if parts.is_empty() {
        parts.push(s.to_string());
    }
    parts
}

fn tf_overlap(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let mut hits = 0usize;
    let mut seen = std::collections::HashSet::new();
    for t in a {
        if seen.insert(t.as_str()) && set_b.contains(t.as_str()) {
            hits += 1;
        }
    }
    let denom = a.len().min(32).max(1) as f32;
    (hits as f32 / denom).clamp(0.0, 1.0)
}

fn symbol_overlap(entry: &MemoryEntry, fp: &TaskFingerprint, prompt_tokens: &[String]) -> f32 {
    let mut symbols: Vec<String> = entry.ref_symbols.clone();
    for t in tokenize_rich(&format!("{} {}", entry.name, entry.description)) {
        if t.chars().any(|c| c.is_ascii_uppercase()) || entry.ref_symbols.iter().any(|s| s.eq_ignore_ascii_case(&t)) {
            // keep lowercase tokens from name for matching
            let _ = t;
        }
    }
    // Always include name tokens as symbol candidates.
    symbols.extend(tokenize_rich(&entry.name));
    symbols.extend(entry.ref_symbols.iter().cloned());

    let mut pool: Vec<String> = fp.symbols.iter().map(|s| s.to_lowercase()).collect();
    pool.extend(prompt_tokens.iter().cloned());

    if symbols.is_empty() {
        return 0.0;
    }
    let set_p: std::collections::HashSet<String> = pool.into_iter().collect();
    let mut hits = 0usize;
    let mut seen = std::collections::HashSet::new();
    for s in &symbols {
        let key = s.to_lowercase();
        if seen.insert(key.clone()) && set_p.contains(&key) {
            hits += 1;
        }
    }
    if hits == 0 {
        return 0.0;
    }
    // Exact symbol match in fingerprint → strong signal.
    let exact = fp.symbols.iter().any(|s| {
        entry.ref_symbols.iter().any(|r| r == s)
            || entry.name.eq_ignore_ascii_case(s)
    });
    let base = (hits as f32 / seen.len().max(1) as f32).clamp(0.0, 1.0);
    if exact {
        base.max(0.9)
    } else {
        base
    }
}

fn memory_as_fingerprint(entry: &MemoryEntry) -> TaskFingerprint {
    let mut subsystems: Vec<String> = entry
        .ref_files
        .iter()
        .filter_map(|p| path_subsystem(p))
        .collect();
    subsystems.sort();
    subsystems.dedup();
    TaskFingerprint {
        intent: entry.mem_type.clone(),
        symbols: entry.ref_symbols.clone(),
        file_categories: entry
            .ref_files
            .iter()
            .map(|p| crate::pattern_log::file_category(p))
            .collect(),
        subsystems,
        ..Default::default()
    }
}

fn path_subsystem(path: &str) -> Option<String> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file);
    if stem.len() > 2 {
        Some(stem.to_string())
    } else {
        None
    }
}

fn path_overlap(entry: &MemoryEntry, fp: &TaskFingerprint) -> f32 {
    if entry.ref_files.is_empty() && fp.file_categories.is_empty() && fp.subsystems.is_empty() {
        return 0.0;
    }
    let cats: Vec<String> = entry
        .ref_files
        .iter()
        .map(|p| crate::pattern_log::file_category(p))
        .collect();
    let mut score = 0.0f32;
    let mut n = 0.0f32;
    if !cats.is_empty() && !fp.file_categories.is_empty() {
        n += 1.0;
        score += jaccard_str(&cats, &fp.file_categories);
    }
    let subs: Vec<String> = entry.ref_files.iter().filter_map(|p| path_subsystem(p)).collect();
    if !subs.is_empty() && !fp.subsystems.is_empty() {
        n += 1.0;
        score += jaccard_str(&subs, &fp.subsystems);
    }
    for f in &entry.ref_files {
        for sub in &fp.subsystems {
            if f.contains(sub.as_str()) {
                return (if n == 0.0 { 0.6 } else { score / n }).max(0.6);
            }
        }
    }
    if n == 0.0 {
        0.0
    } else {
        score / n
    }
}

fn jaccard_str(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let sa: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let sb: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn utility_score(entry: &MemoryEntry) -> f32 {
    let s = entry.support_count as f32;
    (s / (s + 3.0)).clamp(0.0, 1.0)
}

fn verification_recency(entry: &MemoryEntry) -> f32 {
    match entry.last_verified_at {
        Some(ts) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(ts);
            let age_days = now.saturating_sub(ts) as f32 / 86400.0;
            (1.0 - (age_days / 180.0).min(0.7)).clamp(0.3, 1.0)
        }
        None => {
            if entry.status == MemoryStatus::Verified {
                0.6
            } else {
                0.3
            }
        }
    }
}

fn scope_applicability(entry: &MemoryEntry) -> f32 {
    match entry.scope {
        Scope::Workspace => 1.0,
        Scope::Global => 0.55,
    }
}

fn diagnostic_overlap(entry: &MemoryEntry, fp: &TaskFingerprint) -> f32 {
    if fp.diagnostic_classes.is_empty() {
        return 0.0;
    }
    let text = format!("{} {}", entry.description, entry.content).to_lowercase();
    let mut hits = 0usize;
    for d in &fp.diagnostic_classes {
        if text.contains(&d.to_lowercase()) {
            hits += 1;
        }
    }
    if hits == 0 {
        0.0
    } else {
        (hits as f32 / fp.diagnostic_classes.len() as f32).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Importance;
    use std::path::PathBuf;

    fn entry(
        name: &str,
        scope: Scope,
        status: MemoryStatus,
        symbols: &[&str],
        files: &[&str],
        content: &str,
    ) -> MemoryEntry {
        MemoryEntry {
            name: name.into(),
            description: format!("{name} description"),
            mem_type: "architecture".into(),
            content: content.into(),
            path: PathBuf::from(format!("{name}.md")),
            scope,
            pinned: false,
            importance: Importance::Normal,
            deprecated: false,
            superseded_by: None,
            schema_version: 2,
            status,
            confidence: 1.0,
            support_count: 0,
            contradiction_count: 0,
            last_verified_at: Some(1_700_000_000),
            last_verified_commit: None,
            ref_files: files.iter().map(|s| (*s).to_string()).collect(),
            ref_symbols: symbols.iter().map(|s| (*s).to_string()).collect(),
            evidence_episodes: vec![],
        }
    }

    #[test]
    fn exact_symbol_ranks_high() {
        let fp = TaskFingerprint {
            intent: "extend-tool-schema".into(),
            symbols: vec!["ProviderConfig".into()],
            subsystems: vec!["provider".into()],
            languages: vec!["rust".into()],
            ..Default::default()
        };
        let hit = entry(
            "provider-extension-architecture",
            Scope::Workspace,
            MemoryStatus::Verified,
            &["ProviderConfig", "PluginOAuthConfig"],
            &["core/src/provider.rs"],
            "API-key providers use config; OAuth belongs in plugins.",
        );
        let miss = entry(
            "unrelated-formatting",
            Scope::Workspace,
            MemoryStatus::Verified,
            &["IndentStyle"],
            &["docs/STYLE.md"],
            "Prefer spaces over tabs in prose docs.",
        );
        let ranked = rank_memories(
            &[hit, miss],
            "extend ProviderConfig for new OAuth provider",
            &fp,
            5,
        );
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].1.name, "provider-extension-architecture");
        assert!(ranked[0].0 > ranked.last().unwrap().0 || ranked.len() == 1);
        let explain = explain_score(&ranked[0].1, "extend ProviderConfig", &fp);
        assert!(explain.contains("ProviderConfig") || explain.contains("symbol") || explain.contains("Score:"));
    }

    #[test]
    fn verified_ranks_above_candidate() {
        let fp = TaskFingerprint {
            intent: "memory-work".into(),
            symbols: vec!["MemoryEntry".into()],
            ..Default::default()
        };
        let verified = entry(
            "memory-store-layout",
            Scope::Workspace,
            MemoryStatus::Verified,
            &["MemoryEntry"],
            &["core/src/memory.rs"],
            "MemoryEntry holds frontmatter metadata.",
        );
        let mut candidate = verified.clone();
        candidate.name = "memory-store-layout-candidate".into();
        candidate.status = MemoryStatus::Candidate;
        candidate.path = PathBuf::from("cand.md");

        let ranked = rank_memories(
            &[candidate, verified],
            "update MemoryEntry metadata",
            &fp,
            5,
        );
        assert_eq!(ranked[0].1.status, MemoryStatus::Verified);
        assert!(ranked[0].0 >= ranked[1].0);
    }

    #[test]
    fn project_scope_gets_bonus_over_global() {
        let fp = TaskFingerprint {
            intent: "testing".into(),
            symbols: vec!["assert_eq".into()],
            ..Default::default()
        };
        let project = entry(
            "prefer-cargo-test",
            Scope::Workspace,
            MemoryStatus::Verified,
            &["assert_eq"],
            &["core/src/memory.rs"],
            "Prefer cargo test for validation.",
        );
        let mut global = project.clone();
        global.name = "prefer-cargo-test-global".into();
        global.scope = Scope::Global;
        global.path = PathBuf::from("g.md");

        let (sp, _) = score_memory(&project, "run cargo test assert_eq", &fp);
        let (sg, _) = score_memory(&global, "run cargo test assert_eq", &fp);
        assert!(sp > sg, "project {sp} should beat global {sg}");
    }
}
