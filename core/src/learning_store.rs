//! Versioned learning storage layout under `~/.config/catalyst-code/learning/`.
//!
//! Preserves the existing memory store (`memory/`) unchanged. Learning data
//! (episodes, preferences, activations, codebase index) lives beside it:
//!
//! ```text
//! ~/.config/catalyst-code/
//! ├── memory/…
//! ├── learning/
//! │   ├── global/
//! │   │   ├── preferences.jsonl
//! │   │   ├── rejected-approaches.jsonl
//! │   │   ├── skill-metrics.jsonl
//! │   │   └── activations.jsonl
//! │   └── projects/<project-id>/
//! │       ├── project.json
//! │       ├── episodes.jsonl
//! │       └── …
//! └── project-registry.json
//! ```
//!
//! Append-only JSONL files skip malformed lines. Compact JSON uses atomic
//! writes + cross-process locks. Growth is capped (see module constants).

#![allow(dead_code)]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Learning store schema version written into `project.json`.
pub const LEARNING_SCHEMA_VERSION: u32 = 1;

/// Soft cap for detailed episodes retained per project (spec §23).
pub const MAX_EPISODES: usize = 1000;
/// Soft cap for activation records before compaction.
pub const MAX_ACTIVATIONS: usize = 500;
/// Soft cap for feedback events before compaction.
pub const MAX_FEEDBACK: usize = 500;

/// Optional override for the learning root (tests).
static ROOT_OVERRIDE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub struct LearningRootGuard {
    prev: Option<PathBuf>,
}

impl Drop for LearningRootGuard {
    fn drop(&mut self) {
        let mut g = ROOT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
        *g = self.prev.take();
    }
}

/// Serializes tests that redirect the learning root (process-global override).
#[cfg(test)]
pub fn learning_test_serial() -> &'static std::sync::Mutex<()> {
    use std::sync::OnceLock;
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Install `root` as the learning store root until the guard drops (tests).
#[cfg(test)]
pub fn override_learning_root(root: PathBuf) -> LearningRootGuard {
    let mut g = ROOT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    let prev = g.replace(root);
    LearningRootGuard { prev }
}

fn learning_root() -> PathBuf {
    if let Ok(g) = ROOT_OVERRIDE.lock() {
        if let Some(ref p) = *g {
            return p.clone();
        }
    }
    crate::config::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/catalyst-code/learning")
}

/// Paths for the global learning bucket.
#[derive(Clone, Debug)]
pub struct GlobalLearningPaths {
    pub root: PathBuf,
    pub preferences: PathBuf,
    pub rejected_approaches: PathBuf,
    pub skill_metrics: PathBuf,
    pub activations: PathBuf,
}

/// Paths for a single project's learning bucket.
#[derive(Clone, Debug)]
pub struct ProjectLearningPaths {
    pub root: PathBuf,
    pub project_json: PathBuf,
    pub episodes: PathBuf,
    pub activations: PathBuf,
    pub task_patterns: PathBuf,
    pub feedback: PathBuf,
    pub coverage: PathBuf,
    pub index_dir: PathBuf,
}

/// Compact current-state metadata for a project learning dir.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectLearningMeta {
    pub schema_version: u32,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default)]
    pub legacy_workspace_hashes: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl GlobalLearningPaths {
    pub fn resolve() -> Self {
        let root = learning_root().join("global");
        Self {
            preferences: root.join("preferences.jsonl"),
            rejected_approaches: root.join("rejected-approaches.jsonl"),
            skill_metrics: root.join("skill-metrics.jsonl"),
            activations: root.join("activations.jsonl"),
            root,
        }
    }

    /// Ensure the global learning directory exists.
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)
    }
}

impl ProjectLearningPaths {
    pub fn resolve(project_id: &str) -> Self {
        let root = learning_root().join("projects").join(project_id);
        let index_dir = root.join("index");
        Self {
            project_json: root.join("project.json"),
            episodes: root.join("episodes.jsonl"),
            activations: root.join("activations.jsonl"),
            task_patterns: root.join("task-patterns.jsonl"),
            feedback: root.join("feedback.jsonl"),
            coverage: root.join("coverage.json"),
            index_dir,
            root,
        }
    }

    /// Ensure project learning dirs exist (including `index/`).
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.index_dir)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Initialize (or refresh) a project's learning directory and `project.json`.
/// Idempotent and crash-safe. Returns the resolved paths.
pub fn ensure_project_learning(
    project_id: &str,
    remote: Option<&str>,
    legacy_hash: Option<&str>,
) -> ProjectLearningPaths {
    let paths = ProjectLearningPaths::resolve(project_id);
    let _ = paths.ensure();
    let _ = GlobalLearningPaths::resolve().ensure();

    let lock = crate::fsutil::FileLock::acquire(&paths.project_json.with_extension("lock"));
    let now = now_secs();
    let mut meta = read_project_meta(&paths).unwrap_or(ProjectLearningMeta {
        schema_version: LEARNING_SCHEMA_VERSION,
        project_id: project_id.to_string(),
        remote: remote.map(str::to_string),
        legacy_workspace_hashes: Vec::new(),
        created_at: now,
        updated_at: now,
    });
    meta.schema_version = LEARNING_SCHEMA_VERSION;
    meta.updated_at = now;
    if meta.remote.is_none() {
        if let Some(r) = remote {
            meta.remote = Some(r.to_string());
        }
    }
    if let Some(h) = legacy_hash {
        if !meta.legacy_workspace_hashes.iter().any(|x| x == h) {
            meta.legacy_workspace_hashes.push(h.to_string());
        }
    }
    let _ = write_project_meta(&paths, &meta);
    drop(lock);
    paths
}

fn read_project_meta(paths: &ProjectLearningPaths) -> Option<ProjectLearningMeta> {
    let s = std::fs::read_to_string(&paths.project_json).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_project_meta(
    paths: &ProjectLearningPaths,
    meta: &ProjectLearningMeta,
) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(meta).unwrap_or_else(|_| b"{}".to_vec());
    crate::fsutil::atomic_write(&paths.project_json, &body)
}

/// Append a JSON line to `path`, capping at `max_lines` (oldest trimmed).
/// Skips empty payloads. Cross-process locked. Ignores I/O errors (fail-open).
pub fn append_jsonl(path: &Path, value: &impl Serialize, max_lines: usize) {
    let Ok(line) = serde_json::to_string(value) else {
        return;
    };
    if line.is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
    let mut lines = read_jsonl_raw(path);
    lines.push(line);
    if lines.len() > max_lines {
        let drop = lines.len() - max_lines;
        lines.drain(0..drop);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    let _ = crate::fsutil::atomic_write_str(path, &out);
}

/// Read JSONL records, skipping malformed lines.
pub fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Vec<T> {
    read_jsonl_raw(path)
        .into_iter()
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect()
}

fn read_jsonl_raw(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Atomically write compact JSON state (coverage, manifests, …).
pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_vec_pretty(value).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;
    let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
    crate::fsutil::atomic_write(path, &body)
}

/// Read compact JSON, returning `None` on missing/corrupt files (fail-open).
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp_root() -> PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "learning-store-{}-{}-{}",
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ensure_creates_layout() {
        let root = tmp_root();
        let _lserial = learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root.clone());
        let paths = ensure_project_learning("project-deadbeef", Some("github.com/a/b"), Some("abc123"));
        assert!(paths.root.exists());
        assert!(paths.index_dir.exists());
        assert!(paths.project_json.exists());
        let meta: ProjectLearningMeta = read_json(&paths.project_json).unwrap();
        assert_eq!(meta.project_id, "project-deadbeef");
        assert_eq!(meta.remote.as_deref(), Some("github.com/a/b"));
        assert_eq!(meta.legacy_workspace_hashes, vec!["abc123".to_string()]);
        assert!(GlobalLearningPaths::resolve().root.exists());
    }

    #[test]
    fn append_jsonl_skips_bad_lines_and_caps() {
        let root = tmp_root();
        let _lserial = learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let paths = ensure_project_learning("project-cap", None, None);
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Rec {
            n: u32,
        }
        // Seed a corrupt line.
        std::fs::write(&paths.episodes, "{not-json}\n").unwrap();
        for i in 0..5u32 {
            append_jsonl(&paths.episodes, &Rec { n: i }, 3);
        }
        let got: Vec<Rec> = read_jsonl(&paths.episodes);
        // Cap 3 keeps the newest; corrupt line was dropped on rewrite.
        assert_eq!(got, vec![Rec { n: 2 }, Rec { n: 3 }, Rec { n: 4 }]);
    }

    #[test]
    fn concurrent_appends_do_not_lose_events() {
        use std::thread;
        let root = tmp_root();
        let _lserial = learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _g = override_learning_root(root);
        let paths = ensure_project_learning("project-conc", None, None);
        #[derive(Serialize, Deserialize)]
        struct Rec {
            id: u32,
        }
        let path = paths.episodes.clone();
        let mut handles = Vec::new();
        for t in 0..4u32 {
            let p = path.clone();
            handles.push(thread::spawn(move || {
                for i in 0..25u32 {
                    append_jsonl(&p, &Rec { id: t * 100 + i }, 1000);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let got: Vec<Rec> = read_jsonl(&path);
        assert_eq!(got.len(), 100, "all 100 appends must survive");
    }
}
