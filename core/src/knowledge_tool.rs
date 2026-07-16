//! Read-only knowledge dispatch (spec §21.2).
//!
//! Surfaces codebase index, context packs, preferences, episodes, and
//! rejected approaches without mutating learning state. Fail-open / compact
//! string outputs.

#![allow(dead_code)]

use std::path::Path;

use serde_json::Value;

use crate::change_coupling;
use crate::coverage_ledger;
use crate::codebase_index;
use crate::context_pack;
use crate::episodes;
use crate::learning_retrieval;
use crate::memory;
use crate::preferences;
use crate::project_identity;
use crate::rejected_approaches;
use crate::task_fingerprint::{self, FingerprintInputs};

/// Dispatch a knowledge action. Returns a compact human-readable string.
pub fn dispatch(action: &str, args: &Value, workspace: &Path) -> Result<String, String> {
    match action {
        "context" => Ok(action_context(workspace, args)),
        "search" => Ok(action_search(workspace, args)),
        "symbol" => Ok(action_symbol(workspace, args)),
        "related" => Ok(action_related(workspace, args)),
        "episodes" => Ok(action_episodes(workspace, args)),
        "preferences" => Ok(action_preferences()),
        "rejected" => Ok(action_rejected(workspace, args)),
        "coverage" => Ok(action_coverage(workspace)),
        "explain" => action_explain(workspace, args),
        "tests" => Ok(action_tests(workspace, args)),
        other => Err(format!(
            "unknown knowledge action '{other}'; expected context|search|symbol|related|episodes|preferences|rejected|coverage|explain|tests"
        )),
    }
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_usize(args: &Value, key: &str, default: usize) -> usize {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
}

fn project_id(workspace: &Path) -> String {
    project_identity::resolve_project_identity(workspace).id
}

fn fingerprint_for(prompt: &str) -> task_fingerprint::TaskFingerprint {
    task_fingerprint::build_fingerprint(&FingerprintInputs {
        user_intent: prompt,
        files_read: &[],
        files_changed: &[],
        symbols: &[],
        tools_used: &[],
        diagnostics: &[],
        tests_run: &[],
    })
}

fn action_context(workspace: &Path, args: &Value) -> String {
    let prompt = arg_str(args, "prompt").unwrap_or_else(|| arg_str(args, "query").unwrap_or_default());
    context_pack::build_context_pack(workspace, &prompt)
}

fn action_search(workspace: &Path, args: &Value) -> String {
    let query = arg_str(args, "query")
        .or_else(|| arg_str(args, "prompt"))
        .unwrap_or_default();
    let limit = arg_usize(args, "limit", 8);
    let memories = memory::scan_all_memories(workspace);
    let fp = fingerprint_for(&query);
    let ranked = learning_retrieval::rank_memories(&memories, &query, &fp, limit);
    if ranked.is_empty() {
        return "(no memory hits)\n".into();
    }
    let mut out = String::from("Knowledge search hits:\n");
    for (score, entry, reasons) in ranked {
        out.push_str(&format!(
            "- {} [{:?}] score={:.3} — {}\n",
            entry.name, entry.scope, score, entry.description
        ));
        for r in reasons.iter().take(3) {
            out.push_str(&format!("    reason: {r}\n"));
        }
    }
    out
}

fn action_symbol(workspace: &Path, args: &Value) -> String {
    let name = match arg_str(args, "name").or_else(|| arg_str(args, "symbol")) {
        Some(n) if !n.is_empty() => n,
        _ => return "symbol: missing 'name'\n".into(),
    };
    let pid = project_id(workspace);
    let syms = codebase_index::find_symbols(&pid, &name);
    if syms.is_empty() {
        return format!("symbol '{name}': no indexed matches\n");
    }
    let mut out = format!("Symbols named '{name}' ({}):\n", syms.len());
    for s in syms.iter().take(20) {
        out.push_str(&format!(
            "- {} {} {}:{}-{}\n",
            s.kind, s.path, s.name, s.line_start, s.line_end
        ));
    }
    out
}

fn action_related(workspace: &Path, args: &Value) -> String {
    let path = match arg_str(args, "path") {
        Some(p) if !p.is_empty() => p,
        _ => return "related: missing 'path'\n".into(),
    };
    let pid = project_id(workspace);
    let limit = arg_usize(args, "limit", 6);

    let mut out = format!("Related to {path}:\n");

    let stem = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path.as_str());
    let rels = codebase_index::find_relations_to(&pid, stem);
    if rels.is_empty() {
        out.push_str("- relations: (none)\n");
    } else {
        out.push_str("- relations:\n");
        for r in rels.iter().take(limit) {
            out.push_str(&format!(
                "  {} {} → {} (conf={:.2})\n",
                r.kind, r.from_path, r.to_name, r.confidence
            ));
        }
    }

    let companions = change_coupling::companions_for(&pid, &path, limit);
    if companions.is_empty() {
        out.push_str("- companions: (none)\n");
    } else {
        out.push_str("- companions:\n");
        for c in &companions {
            out.push_str(&format!(
                "  {} (conf={:.2}, n={})\n",
                c.companion, c.confidence, c.supporting_commits
            ));
        }
    }

    let memories = memory::scan_all_memories(workspace);
    let mentioning: Vec<_> = memories
        .iter()
        .filter(|m| {
            m.ref_files
                .iter()
                .any(|f| f == &path || path.ends_with(f) || f.ends_with(&path))
                || m.content.contains(&path)
                || m.description.contains(&path)
        })
        .take(limit)
        .collect();
    if mentioning.is_empty() {
        out.push_str("- memories: (none)\n");
    } else {
        out.push_str("- memories:\n");
        for m in mentioning {
            out.push_str(&format!("  {} — {}\n", m.name, m.description));
        }
    }
    out
}

fn action_episodes(workspace: &Path, args: &Value) -> String {
    let pid = project_id(workspace);
    let limit = arg_usize(args, "limit", 5);
    let query = arg_str(args, "query").or_else(|| arg_str(args, "prompt"));

    if let Some(q) = query {
        let fp = fingerprint_for(&q);
        let similar = episodes::similar_episodes(&pid, &fp, 0.15, limit);
        if similar.is_empty() {
            return format!("episodes: no similar matches for project {pid}\n");
        }
        let mut out = format!("Similar episodes ({pid}):\n");
        for (sim, ep) in similar {
            out.push_str(&format!(
                "- {} sim={:.2} outcome={} intent={}\n",
                ep.id,
                sim,
                ep.outcome.as_str(),
                truncate(&ep.user_intent, 80)
            ));
        }
        return out;
    }

    let mut eps = episodes::load_episodes(&pid);
    if eps.is_empty() {
        return format!("episodes: none for {pid}\n");
    }
    eps.reverse();
    let mut out = format!("Recent episodes ({pid}):\n");
    for ep in eps.iter().take(limit) {
        out.push_str(&format!(
            "- {} outcome={} intent={}\n",
            ep.id,
            ep.outcome.as_str(),
            truncate(&ep.user_intent, 80)
        ));
    }
    out
}

fn action_preferences() -> String {
    let prefs = preferences::load_global_preferences();
    if prefs.is_empty() {
        return "preferences: (none)\n".into();
    }
    let mut out = String::from("Global preferences:\n");
    for p in prefs.iter().take(20) {
        out.push_str(&format!(
            "- [{}] {} conf={:.2} — {}\n",
            p.category,
            p.status.as_str(),
            p.confidence,
            truncate(&p.statement, 100)
        ));
    }
    out
}

fn action_rejected(workspace: &Path, args: &Value) -> String {
    let pid = project_id(workspace);
    let query = arg_str(args, "query").unwrap_or_default();
    let limit = arg_usize(args, "limit", 5);

    let mut out = String::from("[REJECTED APPROACHES]\n");
    if !query.is_empty() {
        let fp = fingerprint_for(&query);
        let matched = rejected_approaches::match_rejected(&fp, Some(&pid), 0.15, limit);
        let global = rejected_approaches::match_rejected(&fp, None, 0.15, limit);
        if matched.is_empty() && global.is_empty() {
            let loaded = rejected_approaches::load_rejected(Some(&pid));
            let gloaded = rejected_approaches::load_rejected(None);
            if loaded.is_empty() && gloaded.is_empty() {
                return "rejected: (none)\n".into();
            }
            for r in loaded.iter().chain(gloaded.iter()).take(limit) {
                out.push_str(&format!(
                    "- [{}] {} — {}\n",
                    r.scope,
                    truncate(&r.approach, 60),
                    truncate(&r.rejection_reason, 80)
                ));
            }
            return out;
        }
        for (sim, r) in matched.into_iter().chain(global) {
            out.push_str(&format!(
                "- [{}] sim={:.2} {} — {}\n",
                r.scope,
                sim,
                truncate(&r.approach, 60),
                truncate(&r.rejection_reason, 80)
            ));
        }
        return out;
    }

    let loaded = rejected_approaches::load_rejected(Some(&pid));
    let gloaded = rejected_approaches::load_rejected(None);
    if loaded.is_empty() && gloaded.is_empty() {
        return "rejected: (none)\n".into();
    }
    for r in loaded.iter().chain(gloaded.iter()).take(limit) {
        out.push_str(&format!(
            "- [{}] {} — {}\n",
            r.scope,
            truncate(&r.approach, 60),
            truncate(&r.rejection_reason, 80)
        ));
    }
    out
}

fn action_coverage(workspace: &Path) -> String {
    let pid = project_id(workspace);
    let areas = coverage_ledger::rebuild_coverage(workspace, &pid);
    let poor = coverage_ledger::poorly_covered(&pid, 5);
    let mut out = format!("Coverage for {pid} ({} areas):\n", areas.len());
    for a in areas.iter().take(20) {
        out.push_str(&format!(
            "- {} files={} syms={} mems={} eps={} conf={:.2}\n",
            a.area, a.indexed_files, a.indexed_symbols, a.related_memories, a.related_episodes, a.confidence
        ));
    }
    if !poor.is_empty() {
        out.push_str("Poorly covered:\n");
        for a in &poor {
            out.push_str(&format!("- {} conf={:.2}\n", a.area, a.confidence));
        }
    }
    out
}

fn action_explain(workspace: &Path, args: &Value) -> Result<String, String> {
    let name = arg_str(args, "name")
        .or_else(|| arg_str(args, "memory"))
        .ok_or_else(|| "explain: missing 'name'".to_string())?;
    let prompt = arg_str(args, "prompt")
        .or_else(|| arg_str(args, "query"))
        .unwrap_or_default();
    let memories = memory::scan_all_memories(workspace);
    let entry = memories
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| format!("explain: memory '{name}' not found"))?;
    let fp = fingerprint_for(&prompt);
    Ok(learning_retrieval::explain_score(&entry, &prompt, &fp))
}

fn action_tests(workspace: &Path, args: &Value) -> String {
    let path = arg_str(args, "path").unwrap_or_default();
    let pid = project_id(workspace);
    let eps = episodes::load_episodes(&pid);
    let mut cmds: Vec<(String, u32, u32)> = Vec::new();

    for ep in &eps {
        let path_relevant = path.is_empty()
            || ep
                .files_changed
                .iter()
                .any(|f| f.contains(&path) || path.contains(f.as_str()))
            || ep
                .files_read
                .iter()
                .any(|f| f.contains(&path) || path.contains(f.as_str()));
        if !path_relevant {
            continue;
        }
        for t in &ep.tests_run {
            if let Some((_, ok, fail)) = cmds.iter_mut().find(|(c, _, _)| c == &t.command) {
                if t.ok {
                    *ok += 1;
                } else {
                    *fail += 1;
                }
            } else {
                cmds.push((
                    t.command.clone(),
                    if t.ok { 1 } else { 0 },
                    if t.ok { 0 } else { 1 },
                ));
            }
        }
    }

    if cmds.is_empty() {
        return format!(
            "tests: advisory stub — no episode tests_run matching path '{}'\n",
            if path.is_empty() { "*" } else { &path }
        );
    }
    cmds.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));
    let mut out = format!("Advisory validation for '{path}':\n");
    for (cmd, ok, fail) in cmds.into_iter().take(8) {
        out.push_str(&format!("- {cmd} (ok={ok} fail={fail})\n"));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use crate::preferences::{LearningStatus, PreferenceRecord};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static N: AtomicU64 = AtomicU64::new(0);
    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn tmp_root() -> std::path::PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "knowledge-tool-{}-{}-{}",
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
    fn dispatch_unknown_errors() {
        let err = dispatch("nope", &serde_json::json!({}), Path::new(".")).unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[test]
    fn preferences_and_coverage_with_temp_root() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);

        preferences::append_global_preference(&PreferenceRecord {
            id: "pref-1".into(),
            scope: "global".into(),
            category: "testing".into(),
            statement: "Always add tests before finishing".into(),
            status: LearningStatus::Verified,
            confidence: 0.95,
            explicit: true,
            supporting_events: vec![],
            contradicting_events: vec![],
            project_exceptions: vec![],
            first_seen: 1,
            last_seen: 1,
        });

        let ws = tmp_root();
        std::fs::create_dir_all(&ws).unwrap();
        let prefs_out = dispatch("preferences", &serde_json::json!({}), &ws).unwrap();
        assert!(prefs_out.contains("Always add tests"));

        let cov = dispatch("coverage", &serde_json::json!({}), &ws).unwrap();
        assert!(cov.contains("Coverage"), "{cov}");
        assert!(cov.contains("areas") || cov.contains("conf="), "{cov}");

        let rejected = dispatch("rejected", &serde_json::json!({}), &ws).unwrap();
        assert!(rejected.contains("rejected") || rejected.contains("REJECTED"));

        let tests = dispatch(
            "tests",
            &serde_json::json!({"path": "core/src/memory.rs"}),
            &ws,
        )
        .unwrap();
        assert!(tests.contains("advisory") || tests.contains("Advisory"));
    }

    #[test]
    fn context_action_returns_pack() {
        let _lock = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let ws = tmp_root();
        std::fs::create_dir_all(&ws).unwrap();
        let out = dispatch(
            "context",
            &serde_json::json!({"prompt": "extend memory tool schema"}),
            &ws,
        )
        .unwrap();
        assert!(out.contains("[TASK CONTEXT]") || out.contains("Task"));
    }
}
