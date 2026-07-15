//! Hybrid filesystem checkpoints for undo / rewind.
//!
//! - Git workspace: `git stash create` + ref `refs/catcode/checkpoints/<id>`
//! - Non-git: copy touched (or listed) files under `.catalyst-code/checkpoints/<id>/`
//!
//! An index JSONL sidecar next to the session file tracks metadata.

use crate::protocol::{emit, Event};
use crate::worktree;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub id: String,
    pub label: String,
    pub created_at: u64,
    pub kind: String, // "git" | "files"
    pub head_sha: Option<String>,
    pub stash_sha: Option<String>,
    pub paths: Vec<String>,
    pub dir: Option<String>,
    /// When true, created automatically before a destructive tool wave.
    pub auto: bool,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    format!("cp-{}", now_secs())
}

pub fn index_path(session_file: Option<&Path>, workspace: &Path) -> PathBuf {
    if let Some(p) = session_file {
        let mut s = p.to_path_buf();
        s.set_extension("checkpoints.jsonl");
        return s;
    }
    workspace
        .join(".catalyst-code")
        .join("checkpoints")
        .join("index.jsonl")
}

fn checkpoints_dir(workspace: &Path) -> PathBuf {
    workspace.join(".catalyst-code").join("checkpoints")
}

fn append_index(index: &Path, meta: &CheckpointMeta) {
    if let Some(parent) = index.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(index)
    {
        use std::io::Write;
        if let Ok(line) = serde_json::to_string(meta) {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub fn list(index: &Path) -> Vec<CheckpointMeta> {
    let Ok(content) = std::fs::read_to_string(index) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn git_out(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("git spawn: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Create a checkpoint. `paths` is used for the file-snapshot backend (and as
/// documentation for git). Empty paths → snapshot all tracked+untracked dirty
/// files for file backend; for git, stash create captures the full dirty tree.
pub fn create(
    workspace: &Path,
    session_file: Option<&Path>,
    label: &str,
    paths: &[String],
    auto: bool,
) -> Result<CheckpointMeta, String> {
    let id = new_id();
    let index = index_path(session_file, workspace);
    let meta = if worktree::is_git_repo(workspace) {
        create_git(workspace, &id, label, paths, auto)?
    } else {
        create_files(workspace, &id, label, paths, auto)?
    };
    append_index(&index, &meta);
    emit(
        &Event::new("checkpoint_created")
            .with("id", json!(meta.id))
            .with("label", json!(meta.label))
            .with("kind", json!(meta.kind))
            .with("auto", json!(meta.auto))
            .with("paths", json!(meta.paths)),
    );
    Ok(meta)
}

fn create_git(
    workspace: &Path,
    id: &str,
    label: &str,
    paths: &[String],
    auto: bool,
) -> Result<CheckpointMeta, String> {
    let head = git_out(workspace, &["rev-parse", "HEAD"]).ok();
    // Include untracked so new files are recoverable.
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(workspace)
        .status();
    let stash = git_out(workspace, &["stash", "create", label])?;
    // Reset the index stage from `git add -A` without losing worktree changes.
    let _ = Command::new("git")
        .args(["reset", "-q"])
        .current_dir(workspace)
        .status();
    if stash.is_empty() {
        // Clean tree — still record a checkpoint anchored at HEAD.
        let refname = format!("refs/catcode/checkpoints/{id}");
        if let Some(h) = &head {
            let _ = git_out(workspace, &["update-ref", &refname, h]);
        }
        return Ok(CheckpointMeta {
            id: id.to_string(),
            label: label.to_string(),
            created_at: now_secs(),
            kind: "git".into(),
            head_sha: head,
            stash_sha: None,
            paths: paths.to_vec(),
            dir: None,
            auto,
        });
    }
    let refname = format!("refs/catcode/checkpoints/{id}");
    git_out(workspace, &["update-ref", &refname, &stash])?;
    Ok(CheckpointMeta {
        id: id.to_string(),
        label: label.to_string(),
        created_at: now_secs(),
        kind: "git".into(),
        head_sha: head,
        stash_sha: Some(stash),
        paths: paths.to_vec(),
        dir: None,
        auto,
    })
}

fn collect_paths(workspace: &Path, paths: &[String]) -> Vec<String> {
    if !paths.is_empty() {
        return paths.to_vec();
    }
    // Walk a shallow tree of non-ignored files (cap for safety).
    let mut out = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    let mut seen = 0u32;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in rd.flatten() {
            seen += 1;
            if seen > 5000 || out.len() > 2000 {
                return out;
            }
            let p = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "dist" | ".catalyst-code" | ".venv"
            ) {
                continue;
            }
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                if let Ok(rel) = p.strip_prefix(workspace) {
                    out.push(rel.display().to_string());
                }
            }
        }
    }
    out
}

fn create_files(
    workspace: &Path,
    id: &str,
    label: &str,
    paths: &[String],
    auto: bool,
) -> Result<CheckpointMeta, String> {
    let dir = checkpoints_dir(workspace).join(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir checkpoint: {e}"))?;
    let rels = collect_paths(workspace, paths);
    let mut saved = Vec::new();
    for rel in &rels {
        let src = workspace.join(rel);
        if !src.is_file() {
            continue;
        }
        let dest = dir.join(rel);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&src, &dest).is_ok() {
            saved.push(rel.clone());
        }
    }
    let manifest = json!({ "paths": saved });
    let _ = std::fs::write(dir.join("manifest.json"), manifest.to_string());
    Ok(CheckpointMeta {
        id: id.to_string(),
        label: label.to_string(),
        created_at: now_secs(),
        kind: "files".into(),
        head_sha: None,
        stash_sha: None,
        paths: saved,
        dir: Some(dir.display().to_string()),
        auto,
    })
}

/// Restore a checkpoint by id. Git: `git stash apply <sha>` (keeps ref).
/// Files: copy snapshot contents back over the workspace.
pub fn restore(
    workspace: &Path,
    session_file: Option<&Path>,
    id: &str,
) -> Result<CheckpointMeta, String> {
    let index = index_path(session_file, workspace);
    let metas = list(&index);
    let meta = metas
        .into_iter()
        .rev()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("checkpoint '{id}' not found"))?;
    match meta.kind.as_str() {
        "git" => {
            if let Some(sha) = &meta.stash_sha {
                // Apply without removing the ref so restore is repeatable.
                let out = Command::new("git")
                    .args(["stash", "apply", sha])
                    .current_dir(workspace)
                    .output()
                    .map_err(|e| format!("stash apply spawn: {e}"))?;
                if !out.status.success() {
                    // Fall back to checkout of the tree if apply conflicts.
                    let err = String::from_utf8_lossy(&out.stderr);
                    return Err(format!("git stash apply failed: {}", err.trim()));
                }
            } else if let Some(head) = &meta.head_sha {
                // Clean checkpoint — reset tracked files to HEAD at that time.
                let _ = git_out(workspace, &["checkout", head, "--", "."]);
            }
        }
        "files" => {
            let dir = meta
                .dir
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| checkpoints_dir(workspace).join(&meta.id));
            for rel in &meta.paths {
                let src = dir.join(rel);
                let dest = workspace.join(rel);
                if !src.is_file() {
                    continue;
                }
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&src, &dest);
            }
        }
        other => return Err(format!("unknown checkpoint kind '{other}'")),
    }
    emit(
        &Event::new("checkpoint_restored")
            .with("id", json!(meta.id))
            .with("kind", json!(meta.kind)),
    );
    Ok(meta)
}

/// Restore the most recent auto checkpoint (for Undo). Returns None if none.
pub fn restore_latest_auto(
    workspace: &Path,
    session_file: Option<&Path>,
) -> Option<CheckpointMeta> {
    let index = index_path(session_file, workspace);
    let metas = list(&index);
    let id = metas.iter().rev().find(|m| m.auto).map(|m| m.id.clone())?;
    restore(workspace, session_file, &id).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_checkpoint_roundtrip() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-{stamp}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let meta = create(&dir, None, "create", &["a.txt".into()], true).unwrap();
        assert_eq!(meta.kind, "files");
        std::fs::write(dir.join("a.txt"), "changed").unwrap();
        restore(&dir, None, &meta.id).unwrap();
        let got = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        assert_eq!(got, "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_id_has_cp_prefix() {
        let id = new_id();
        assert!(id.starts_with("cp-"), "id must start with cp-: {id}");
        let suffix = id.strip_prefix("cp-").unwrap();
        assert!(suffix.parse::<u64>().is_ok(), "suffix must be timestamp: {suffix}");
    }

    #[test]
    fn list_returns_empty_for_nonexistent_file() {
        let tmp = std::path::PathBuf::from(format!(
            "/tmp/catcode-test-nonexistent-{}",
            std::process::id()
        ));
        let metas = list(&tmp);
        assert!(metas.is_empty());
    }

    #[test]
    fn meta_json_roundtrip() {
        let meta = CheckpointMeta {
            id: "cp-42".into(),
            label: "snapshot".into(),
            created_at: 1700000000,
            kind: "git".into(),
            head_sha: Some("abc123".into()),
            stash_sha: Some("def456".into()),
            paths: vec!["a.txt".into()],
            dir: None,
            auto: true,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: CheckpointMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, meta.id);
        assert_eq!(back.kind, "git");
        assert_eq!(back.head_sha, Some("abc123".into()));
    }

    #[test]
    fn restore_unknown_checkpoint_returns_err() {
        let dir = std::env::temp_dir().join(format!("catcode-cp2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No index file exists, so any id is unknown.
        let err = restore(&dir, None, "nonexistent").unwrap_err();
        assert!(err.contains("not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
