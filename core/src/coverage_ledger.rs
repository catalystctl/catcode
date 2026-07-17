//! Knowledge coverage ledger (spec §9).
//!
//! Tracks how well Catalyst understands each repository area. Written as
//! compact JSON under the project learning dir. Fail-open.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::codebase_index;
use crate::episodes;
use crate::learning_store::{self, ProjectLearningPaths};
use crate::memory;

/// Per-area coverage (spec §9).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeCoverage {
    pub area: String,
    pub indexed_files: usize,
    pub indexed_symbols: usize,
    pub related_memories: usize,
    pub related_episodes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_examined_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_changed_at: Option<u64>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct CoverageFile {
    schema_version: u32,
    project_id: String,
    areas: Vec<KnowledgeCoverage>,
    updated_at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn area_of(path: &str) -> String {
    let p = path.replace('\\', "/");
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => "(root)".into(),
        [a] => (*a).to_string(),
        [a, b, ..] => format!("{a}/{b}"),
    }
}

/// Rebuild coverage from index + memories + episodes and persist.
pub fn rebuild_coverage(workspace: &Path, project_id: &str) -> Vec<KnowledgeCoverage> {
    let files = codebase_index::list_files(project_id);
    let mut by_area: BTreeMap<String, KnowledgeCoverage> = BTreeMap::new();

    for f in &files {
        let area = area_of(&f.path);
        let e = by_area.entry(area.clone()).or_insert(KnowledgeCoverage {
            area: area.clone(),
            indexed_files: 0,
            indexed_symbols: 0,
            related_memories: 0,
            related_episodes: 0,
            last_examined_at: None,
            last_changed_at: Some(f.modified_at),
            confidence: 0.0,
        });
        e.indexed_files += 1;
        if let Some(prev) = e.last_changed_at {
            e.last_changed_at = Some(prev.max(f.modified_at));
        }
    }

    // Symbol counts per area (cheap: list all symbols matching path prefixes).
    // Use relations/files only — find_symbols with empty is wrong; count via files.
    for (area, cov) in by_area.iter_mut() {
        // Approximate symbols: 2x files for code areas (updated if index grows).
        let syms: usize = files
            .iter()
            .filter(|f| area_of(&f.path) == *area)
            .filter(|f| {
                matches!(
                    f.language.as_str(),
                    "rust" | "go" | "typescript" | "javascript" | "python"
                )
            })
            .count();
        // Prefer real symbol rows when available.
        let real: usize = codebase_index::list_files(project_id)
            .iter()
            .filter(|f| area_of(&f.path) == *area)
            .count(); // placeholder; refined below
        let _ = real;
        cov.indexed_symbols = syms; // refined after symbol scan
    }

    // Count symbols properly from find across files — use relations file size heuristic.
    for f in &files {
        let area = area_of(&f.path);
        if let Some(e) = by_area.get_mut(&area) {
            // Pull symbols whose path matches.
            let stem = Path::new(&f.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !stem.is_empty() {
                let n = codebase_index::find_symbols(project_id, stem).len();
                e.indexed_symbols += n;
            }
        }
    }

    let memories = memory::scan_all_memories(workspace);
    for m in &memories {
        for rf in &m.ref_files {
            let area = area_of(rf);
            if let Some(e) = by_area.get_mut(&area) {
                e.related_memories += 1;
            }
        }
        // Also match area tokens in memory name/description.
        let hay = format!("{} {}", m.name, m.description).to_lowercase();
        for (area, e) in by_area.iter_mut() {
            let a = area.to_lowercase();
            if hay.contains(&a) || a.split('/').any(|p| p.len() > 2 && hay.contains(p)) {
                e.related_memories += 1;
            }
        }
    }

    let eps = episodes::load_episodes(project_id);
    for ep in &eps {
        for f in ep.files_changed.iter().chain(ep.files_read.iter()) {
            let area = area_of(f);
            if let Some(e) = by_area.get_mut(&area) {
                e.related_episodes += 1;
            }
        }
    }

    let now = now_secs();
    let mut areas: Vec<KnowledgeCoverage> = by_area.into_values().collect();
    for e in &mut areas {
        // Confidence: indexed + documented.
        let mut c: f32 = 0.2;
        if e.indexed_files > 0 {
            c += 0.3;
        }
        if e.indexed_symbols > 0 {
            c += 0.15;
        }
        if e.related_memories > 0 {
            c += 0.2;
        }
        if e.related_episodes > 0 {
            c += 0.15;
        }
        e.confidence = c.clamp(0.0, 1.0);
        e.last_examined_at = Some(now);
    }
    areas.sort_by(|a, b| a.area.cmp(&b.area));

    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let file = CoverageFile {
        schema_version: 1,
        project_id: project_id.to_string(),
        areas: areas.clone(),
        updated_at: now,
    };
    let _ = learning_store::write_json_atomic(&paths.coverage, &file);
    areas
}

/// Load persisted coverage (empty if missing).
pub fn load_coverage(project_id: &str) -> Vec<KnowledgeCoverage> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_json::<CoverageFile>(&paths.coverage)
        .map(|f| f.areas)
        .unwrap_or_default()
}

/// Areas with low confidence / no memories (for exploration hints).
pub fn poorly_covered(project_id: &str, limit: usize) -> Vec<KnowledgeCoverage> {
    let mut areas = load_coverage(project_id);
    areas.sort_by(|a, b| {
        a.confidence
            .partial_cmp(&b.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.indexed_files.cmp(&a.indexed_files))
    });
    areas.truncate(limit);
    areas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::{learning_test_serial, override_learning_root};
    use crate::memory::override_memory_root;
    use crate::project_identity::{self, override_registry_path, registry_test_serial};

    #[test]
    fn rebuild_writes_coverage() {
        let _ls = learning_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _rs = registry_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("cov-{}-{}", std::process::id(), now_secs()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let _mr = override_memory_root(home.join("memory"));
        let _lr = override_learning_root(home.join("learning"));
        let _rr = override_registry_path(home.join("reg.json"));
        let ws = home.join("ws");
        std::fs::create_dir_all(ws.join("core/src")).unwrap();
        std::fs::write(ws.join("core/src/lib.rs"), "pub fn x() {}\n").unwrap();
        let id = project_identity::resolve_project_identity(&ws);
        let _ = codebase_index::ensure_index(&ws);
        let areas = rebuild_coverage(&ws, &id.id);
        assert!(!areas.is_empty());
        let loaded = load_coverage(&id.id);
        assert_eq!(loaded.len(), areas.len());
    }
}
