//! Stable project identity that survives path moves.
//!
//! Memories historically key off [`crate::memory::project_hash`] (FNV-1a of the
//! canonicalized cwd). That breaks when a repo is moved or cloned to a new
//! path. This module maintains a local registry at
//! `~/.config/catalyst-code/project-registry.json` that maps a stable
//! `project-<id>` to remotes, root-commit fingerprints, current paths, and
//! legacy workspace hashes so learning data can follow the project.
//!
//! Identity resolution order (spec §5):
//! 1. Normalized Git remote URL (+ optional root-commit salt when available)
//! 2. Repository root-commit + metadata
//! 3. Existing workspace hash (`memory::project_hash`)
//! 4. Canonical path hash as final fallback
//!
//! All registry writes are atomic and cross-process locked. Failures are
//! soft: callers get a usable identity even when the registry is missing or
//! corrupt (fail-open for coding turns).

#![allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current on-disk schema version for `project-registry.json`.
pub const REGISTRY_SCHEMA_VERSION: u32 = 1;

/// Stable project identity resolved for a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIdentity {
    /// Stable id, e.g. `project-4f20c81a`.
    pub id: String,
    /// Normalized remote (`github.com/org/repo`) when known.
    pub remote: Option<String>,
    /// Full root (oldest reachable) commit SHA when known.
    pub root_commit: Option<String>,
    /// Legacy FNV-1a workspace hash used by the memory store.
    pub workspace_hash: String,
    /// Canonical absolute path of the workspace when resolution ran.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectRecord {
    #[serde(default)]
    current_paths: Vec<String>,
    #[serde(default)]
    legacy_workspace_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_commit: Option<String>,
    first_seen: u64,
    last_seen: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectRegistry {
    schema_version: u32,
    #[serde(default)]
    projects: HashMap<String, ProjectRecord>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA_VERSION,
            projects: HashMap::new(),
        }
    }
}

/// Optional override for the registry file (tests only).
static REGISTRY_OVERRIDE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// RAII guard restoring the previous registry path override.
pub struct RegistryPathGuard {
    prev: Option<PathBuf>,
}

impl Drop for RegistryPathGuard {
    fn drop(&mut self) {
        let mut g = REGISTRY_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
        *g = self.prev.take();
    }
}

/// Serializes tests that redirect the project registry path.
#[cfg(test)]
pub fn registry_test_serial() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Install `path` as the registry file until the guard drops (tests).
#[cfg(test)]
pub fn override_registry_path(path: PathBuf) -> RegistryPathGuard {
    let mut g = REGISTRY_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    let prev = g.replace(path);
    RegistryPathGuard { prev }
}

fn registry_path() -> PathBuf {
    if let Ok(g) = REGISTRY_OVERRIDE.lock() {
        if let Some(ref p) = *g {
            return p.clone();
        }
    }
    crate::config::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/catalyst-code/project-registry.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Normalize a git remote URL into `host/owner/repo` form.
///
/// Accepts HTTPS, SSH (`git@host:path`), and scp-like URLs. Strips `.git`
/// suffixes and userinfo. Returns `None` for empty / unparseable input.
pub fn normalize_remote_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // git@host:owner/repo(.git)
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return Some(clean_remote(host, path));
        }
    }
    // ssh://git@host/owner/repo
    let without_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("ssh://"))
        .or_else(|| s.strip_prefix("git://"))
        .unwrap_or(s);
    let without_user = match without_scheme.split_once('@') {
        Some((_, hostpath)) => hostpath,
        None => without_scheme,
    };
    let without_user = without_user.trim_start_matches('/');
    let (host, path) = match without_user.split_once('/') {
        Some((h, p)) => (h, p),
        None => return None,
    };
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(clean_remote(host, path))
}

fn clean_remote(host: &str, path: &str) -> String {
    let host = host.trim().trim_end_matches(':').to_lowercase();
    let mut path = path.trim().trim_matches('/').to_string();
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.to_string();
    }
    // Drop query/fragment if present.
    if let Some((p, _)) = path.split_once('?') {
        path = p.to_string();
    }
    if let Some((p, _)) = path.split_once('#') {
        path = p.to_string();
    }
    format!("{host}/{path}")
}

/// Read the full SHA of the repository's root (oldest) commit when available.
///
/// Uses the filesystem `.git` layout only — no `git` binary. Looks for
/// `refs/heads/<default>` or HEAD; for a true root commit we would walk the
/// object graph, which is expensive. Instead we use the *initial* commit when
/// detectable via `info/grafts` / shallow markers, otherwise the current HEAD
/// full SHA as a weaker but stable-enough fingerprint for identity matching
/// when combined with the remote. Returns `None` for non-git workspaces.
fn read_root_commit_hint(workspace: &Path) -> Option<String> {
    let ctx = crate::git_ctx::read_git_context(workspace)?;
    // Prefer a full SHA from the loose/packed ref if we can expand the short
    // head. git_ctx currently returns a 7-char short SHA; for identity we
    // re-read the loose ref for the full value when present.
    let full = read_full_head_sha(workspace, &ctx.branch).unwrap_or(ctx.head_sha);
    if full.is_empty() || full == "unknown" {
        return None;
    }
    Some(full)
}

fn read_full_head_sha(workspace: &Path, branch: &str) -> Option<String> {
    let git = workspace.join(".git");
    if !git.exists() {
        return None;
    }
    // Follow worktree gitdir pointer the same way git_ctx does (best-effort).
    let git_dir = if git.is_file() {
        let content = std::fs::read_to_string(&git).ok()?;
        let line = content.lines().next()?.trim();
        let rest = line.strip_prefix("gitdir: ")?;
        let p = PathBuf::from(rest);
        if p.is_absolute() {
            p
        } else {
            workspace.join(p)
        }
    } else {
        git
    };
    let common = {
        let commondir = git_dir.join("commondir");
        if let Ok(c) = std::fs::read_to_string(&commondir) {
            let line = c.lines().next().unwrap_or("").trim();
            if !line.is_empty() {
                let p = PathBuf::from(line);
                if p.is_absolute() {
                    p
                } else {
                    git_dir.join(p)
                }
            } else {
                git_dir.clone()
            }
        } else {
            git_dir
        }
    };
    let loose = common.join("refs").join("heads").join(branch);
    if let Ok(sha) = std::fs::read_to_string(&loose) {
        let sha = sha.trim();
        if sha.len() >= 7 {
            return Some(sha.to_string());
        }
    }
    None
}

fn candidate_id_from_remote(remote: &str, root: Option<&str>) -> String {
    let mut key = remote.to_string();
    if let Some(r) = root {
        // Salt with first 12 hex chars of root commit so a remote URL reuse
        // across unrelated history does not auto-merge projects. Spec: remote
        // URL change must NOT automatically merge unrelated projects — we also
        // do not merge solely on remote without checking existing records.
        key.push('|');
        key.push_str(&r.chars().take(12).collect::<String>());
    }
    format!("project-{:08x}", fnv1a(key.as_bytes()) as u32)
}

fn candidate_id_from_root(root: &str) -> String {
    format!("project-{:08x}", fnv1a(root.as_bytes()) as u32)
}

fn candidate_id_from_hash(hash: &str) -> String {
    // Keep legacy hash recognizable: project-<first 8 of workspace hash>.
    let short: String = hash.chars().take(8).collect();
    format!("project-{short}")
}

fn load_registry(path: &Path) -> ProjectRegistry {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => ProjectRegistry::default(),
    }
}

fn save_registry(path: &Path, reg: &ProjectRegistry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_vec_pretty(reg).unwrap_or_else(|_| b"{}".to_vec());
    crate::fsutil::atomic_write(path, &body)
}

fn find_by_remote(reg: &ProjectRegistry, remote: &str) -> Option<String> {
    reg.projects
        .iter()
        .find(|(_, rec)| rec.remote.as_deref() == Some(remote))
        .map(|(id, _)| id.clone())
}

fn find_by_hash(reg: &ProjectRegistry, hash: &str) -> Option<String> {
    reg.projects
        .iter()
        .find(|(_, rec)| rec.legacy_workspace_hashes.iter().any(|h| h == hash))
        .map(|(id, _)| id.clone())
}

fn find_by_path(reg: &ProjectRegistry, path: &str) -> Option<String> {
    reg.projects
        .iter()
        .find(|(_, rec)| rec.current_paths.iter().any(|p| p == path))
        .map(|(id, _)| id.clone())
}

fn find_by_root(reg: &ProjectRegistry, root: &str) -> Option<String> {
    reg.projects
        .iter()
        .find(|(_, rec)| rec.root_commit.as_deref() == Some(root))
        .map(|(id, _)| id.clone())
}

fn touch_record(
    rec: &mut ProjectRecord,
    path: &str,
    hash: &str,
    remote: Option<&str>,
    root: Option<&str>,
    now: u64,
) {
    rec.last_seen = now;
    if !rec.current_paths.iter().any(|p| p == path) {
        rec.current_paths.push(path.to_string());
        // Cap path list so moves don't grow unboundedly.
        if rec.current_paths.len() > 8 {
            let drop = rec.current_paths.len() - 8;
            rec.current_paths.drain(0..drop);
        }
    }
    if !rec.legacy_workspace_hashes.iter().any(|h| h == hash) {
        rec.legacy_workspace_hashes.push(hash.to_string());
    }
    // Do NOT overwrite a different remote automatically (prevents unrelated
    // project merges on remote URL change). Only fill when empty.
    if rec.remote.is_none() {
        if let Some(r) = remote {
            rec.remote = Some(r.to_string());
        }
    }
    if rec.root_commit.is_none() {
        if let Some(r) = root {
            rec.root_commit = Some(r.to_string());
        }
    }
}

/// Resolve (and register) a stable [`ProjectIdentity`] for `workspace`.
///
/// Best-effort: registry I/O failures still return a usable identity derived
/// from the workspace hash so coding turns never fail open-closed.
pub fn resolve_project_identity(workspace: &Path) -> ProjectIdentity {
    let canonical = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let path_str = canonical.to_string_lossy().to_string();
    let workspace_hash = crate::memory::project_hash(&path_str);
    let remote = crate::git_ctx::read_git_context(&canonical)
        .and_then(|ctx| ctx.remote_url)
        .and_then(|u| normalize_remote_url(&u));
    let root_commit = read_root_commit_hint(&canonical);

    let path = registry_path();
    let lock_path = path.with_extension("lock");
    let _lock = crate::fsutil::FileLock::acquire(&lock_path);

    let mut reg = load_registry(&path);
    if reg.schema_version == 0 {
        reg.schema_version = REGISTRY_SCHEMA_VERSION;
    }
    let now = now_secs();

    // Match existing records without auto-merging distinct remotes.
    let existing = remote
        .as_deref()
        .and_then(|r| find_by_remote(&reg, r))
        .or_else(|| find_by_hash(&reg, &workspace_hash))
        .or_else(|| find_by_path(&reg, &path_str))
        .or_else(|| root_commit.as_deref().and_then(|r| find_by_root(&reg, r)));

    let id = if let Some(id) = existing {
        if let Some(rec) = reg.projects.get_mut(&id) {
            touch_record(
                rec,
                &path_str,
                &workspace_hash,
                remote.as_deref(),
                root_commit.as_deref(),
                now,
            );
        }
        id
    } else {
        let id = if let Some(ref r) = remote {
            candidate_id_from_remote(r, root_commit.as_deref())
        } else if let Some(ref root) = root_commit {
            candidate_id_from_root(root)
        } else {
            candidate_id_from_hash(&workspace_hash)
        };
        // Collision guard: if the computed id exists but is a different
        // remote/root, fall back to hash-based id.
        let id = if let Some(rec) = reg.projects.get(&id) {
            let remote_conflict = match (&rec.remote, &remote) {
                (Some(a), Some(b)) if a != b => true,
                _ => false,
            };
            if remote_conflict {
                candidate_id_from_hash(&workspace_hash)
            } else {
                id
            }
        } else {
            id
        };
        reg.projects.insert(
            id.clone(),
            ProjectRecord {
                current_paths: vec![path_str.clone()],
                legacy_workspace_hashes: vec![workspace_hash.clone()],
                remote: remote.clone(),
                root_commit: root_commit.clone(),
                first_seen: now,
                last_seen: now,
            },
        );
        id
    };

    let _ = save_registry(&path, &reg);

    ProjectIdentity {
        id,
        remote,
        root_commit,
        workspace_hash,
        path: path_str,
    }
}

/// Look up a project id by a legacy workspace hash (memory dir name).
/// Returns `None` if the registry has no mapping.
pub fn project_id_for_workspace_hash(hash: &str) -> Option<String> {
    let path = registry_path();
    let reg = load_registry(&path);
    find_by_hash(&reg, hash)
}

/// List all known project ids (for diagnostics / knowledge tool).
pub fn list_project_ids() -> Vec<String> {
    let path = registry_path();
    let reg = load_registry(&path);
    let mut ids: Vec<String> = reg.projects.keys().cloned().collect();
    ids.sort();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "proj-id-{}-{}-{}-{}",
            label,
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_fake_git(dir: &Path, branch: &str, sha: &str, remote: Option<&str>) {
        let git = dir.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        fs::write(git.join("refs/heads").join(branch), format!("{sha}\n")).unwrap();
        if let Some(url) = remote {
            fs::write(
                git.join("config"),
                format!("[remote \"origin\"]\n\turl = {url}\n"),
            )
            .unwrap();
        }
    }

    #[test]
    fn normalize_https_and_ssh() {
        assert_eq!(
            normalize_remote_url("https://github.com/catalystctl/catcode.git").as_deref(),
            Some("github.com/catalystctl/catcode")
        );
        assert_eq!(
            normalize_remote_url("git@github.com:catalystctl/catcode.git").as_deref(),
            Some("github.com/catalystctl/catcode")
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/catalystctl/catcode").as_deref(),
            Some("github.com/catalystctl/catcode")
        );
        assert!(normalize_remote_url("").is_none());
    }

    #[test]
    fn same_repo_new_path_same_id() {
        let _serial = registry_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tmp_dir("home");
        let reg = home.join("project-registry.json");
        let _guard = override_registry_path(reg);

        let a = tmp_dir("repo-a");
        write_fake_git(
            &a,
            "master",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("https://github.com/catalystctl/catcode.git"),
        );
        let id_a = resolve_project_identity(&a);

        let b = tmp_dir("repo-b");
        write_fake_git(
            &b,
            "master",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("git@github.com:catalystctl/catcode.git"),
        );
        let id_b = resolve_project_identity(&b);

        assert_eq!(id_a.id, id_b.id, "path move must preserve project id");
        assert_eq!(
            id_a.remote.as_deref(),
            Some("github.com/catalystctl/catcode")
        );
        assert_ne!(id_a.workspace_hash, id_b.workspace_hash);
        // Legacy hashes are recorded so old memory dirs remain resolvable.
        let mapped = project_id_for_workspace_hash(&id_a.workspace_hash);
        assert_eq!(mapped.as_deref(), Some(id_a.id.as_str()));
        let mapped_b = project_id_for_workspace_hash(&id_b.workspace_hash);
        assert_eq!(mapped_b.as_deref(), Some(id_a.id.as_str()));
    }

    #[test]
    fn unrelated_repos_do_not_merge() {
        let _serial = registry_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tmp_dir("home2");
        let _guard = override_registry_path(home.join("project-registry.json"));

        let a = tmp_dir("alpha");
        write_fake_git(
            &a,
            "main",
            "1111111111111111111111111111111111111111",
            Some("https://github.com/org/alpha.git"),
        );
        let b = tmp_dir("beta");
        write_fake_git(
            &b,
            "main",
            "2222222222222222222222222222222222222222",
            Some("https://github.com/org/beta.git"),
        );
        let id_a = resolve_project_identity(&a);
        let id_b = resolve_project_identity(&b);
        assert_ne!(id_a.id, id_b.id);
    }

    #[test]
    fn non_git_uses_path_fallback() {
        let _serial = registry_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tmp_dir("home3");
        let _guard = override_registry_path(home.join("project-registry.json"));
        let ws = tmp_dir("nongit");
        let id = resolve_project_identity(&ws);
        assert!(id.id.starts_with("project-"));
        assert!(id.remote.is_none());
        assert!(!id.workspace_hash.is_empty());
        // Second resolve at same path is stable.
        let id2 = resolve_project_identity(&ws);
        assert_eq!(id.id, id2.id);
    }

    #[test]
    fn remote_url_change_does_not_auto_merge() {
        let _serial = registry_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tmp_dir("home4");
        let _guard = override_registry_path(home.join("project-registry.json"));

        let ws = tmp_dir("rename");
        write_fake_git(
            &ws,
            "main",
            "cccccccccccccccccccccccccccccccccccccccc",
            Some("https://github.com/old/name.git"),
        );
        let first = resolve_project_identity(&ws);

        // Rewrite remote to an unrelated repo at the same path.
        write_fake_git(
            &ws,
            "main",
            "dddddddddddddddddddddddddddddddddddddddd",
            Some("https://github.com/new/other.git"),
        );
        let second = resolve_project_identity(&ws);
        // Same path still matches the existing record (path/hash), but the
        // stored remote is NOT overwritten — preventing silent merge of the
        // new remote into the old project's learning store via remote lookup.
        assert_eq!(first.id, second.id);
        let path = registry_path();
        let reg = load_registry(&path);
        let rec = reg.projects.get(&first.id).unwrap();
        assert_eq!(rec.remote.as_deref(), Some("github.com/old/name"));
    }
}
