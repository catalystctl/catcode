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
    use rand::Rng;
    let random: u64 = rand::thread_rng().gen();
    format!("cp-{}-{random:016x}", now_secs())
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

fn append_index(index: &Path, meta: &CheckpointMeta) -> Result<(), String> {
    if let Some(parent) = index.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create checkpoint index parent {}: {e}", parent.display()))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(index)
        .map_err(|e| format!("open checkpoint index {}: {e}", index.display()))?;
    use std::io::Write;
    let line =
        serde_json::to_string(meta).map_err(|e| format!("serialize checkpoint metadata: {e}"))?;
    writeln!(f, "{line}")
        .map_err(|e| format!("write checkpoint index {}: {e}", index.display()))?;
    f.flush()
        .map_err(|e| format!("flush checkpoint index {}: {e}", index.display()))?;
    Ok(())
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
    if let Err(e) = append_index(&index, &meta) {
        match meta.kind.as_str() {
            "git" => {
                let refname = format!("refs/catcode/checkpoints/{}", meta.id);
                let _ = git_out(workspace, &["update-ref", "-d", &refname]);
            }
            "files" => {
                let _ = std::fs::remove_dir_all(checkpoints_dir(workspace).join(&meta.id));
            }
            _ => {}
        }
        return Err(e);
    }
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
    // Build the checkpoint through a temporary index. `git add -A` is needed
    // so `stash create` includes untracked files, but it must never mutate the
    // user's real staging area — even if staging or stash creation fails.
    let index_name = git_out(workspace, &["rev-parse", "--git-path", "index"])?;
    let real_index = {
        let path = PathBuf::from(index_name);
        let joined = if path.is_absolute() {
            path
        } else {
            workspace.join(path)
        };
        if joined.exists() {
            std::fs::canonicalize(&joined)
                .map_err(|e| format!("canonicalize git index {}: {e}", joined.display()))?
        } else {
            let parent = joined
                .parent()
                .ok_or_else(|| "git index has no parent".to_string())?;
            let canon_parent = std::fs::canonicalize(parent)
                .map_err(|e| format!("canonicalize git index parent {}: {e}", parent.display()))?;
            canon_parent.join(
                joined
                    .file_name()
                    .ok_or_else(|| "git index has no filename".to_string())?,
            )
        }
    };
    let temp_index = crate::fsutil::unique_tmp(&real_index);
    match std::fs::copy(&real_index, &temp_index) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("copy git index for checkpoint: {e}")),
    }
    let run_with_temp_index = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git")
            .args(args)
            .env("GIT_INDEX_FILE", &temp_index)
            .current_dir(workspace)
            .output()
            .map_err(|e| format!("git spawn: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let result = (|| {
        run_with_temp_index(&["add", "-A"])?;
        run_with_temp_index(&["stash", "create", label])
    })();
    let cleanup = std::fs::remove_file(&temp_index);
    if let Err(e) = cleanup {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("remove temporary checkpoint index: {e}"));
        }
    }
    let stash = result?;
    let refname = format!("refs/catcode/checkpoints/{id}");
    if stash.is_empty() {
        // Clean tree — still record a checkpoint anchored at HEAD.
        if let Some(h) = &head {
            git_out(workspace, &["update-ref", &refname, h])?;
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

fn collect_paths(workspace: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if !paths.is_empty() {
        let canon_workspace =
            std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
        let mut resolved = Vec::with_capacity(paths.len());
        for input in paths {
            let path = crate::workspace::resolve(workspace, input)?;
            let rel = path
                .strip_prefix(&canon_workspace)
                .map_err(|_| format!("checkpoint path {input:?} resolves outside the workspace"))?;
            resolved.push(rel.to_string_lossy().to_string());
        }
        return Ok(resolved);
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
            if seen > 5000 || out.len() >= 2000 {
                return Ok(out);
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
    Ok(out)
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
    let rels = collect_paths(workspace, paths)?;
    let mut saved = Vec::new();
    for rel in &rels {
        let src = workspace.join(rel);
        if !src.is_file() {
            continue;
        }
        let dest = dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create checkpoint parent {}: {e}", parent.display()))?;
        }
        std::fs::copy(&src, &dest).map_err(|e| {
            format!(
                "copy checkpoint source {} to {}: {e}",
                src.display(),
                dest.display()
            )
        })?;
        saved.push(rel.clone());
    }
    let manifest = json!({ "paths": saved });
    crate::fsutil::atomic_write_str(&dir.join("manifest.json"), &manifest.to_string())
        .map_err(|e| format!("write checkpoint manifest: {e}"))?;
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
fn install_restored_files(
    ready: Vec<(PathBuf, PathBuf)>,
    fail_before: Option<usize>,
) -> Result<(), String> {
    let mut installed: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for (index, (tmp, dest)) in ready.iter().enumerate() {
        let result = (|| {
            if fail_before == Some(index) {
                return Err("injected restore install failure".to_string());
            }
            let backup = if dest.exists() {
                if !dest.is_file() {
                    return Err(format!(
                        "restore destination {} is not a file",
                        dest.display()
                    ));
                }
                let backup = crate::fsutil::unique_tmp(dest);
                std::fs::rename(dest, &backup)
                    .map_err(|e| format!("backup restore destination {}: {e}", dest.display()))?;
                Some(backup)
            } else {
                None
            };
            if let Err(e) = std::fs::rename(tmp, dest) {
                let mut rollback = Vec::new();
                if let Some(backup) = &backup {
                    if let Err(restore_error) = std::fs::rename(backup, dest) {
                        rollback.push(format!(
                            "restore current backup {}: {restore_error}",
                            backup.display()
                        ));
                    }
                }
                let primary = format!("install restored file {}: {e}", dest.display());
                return Err(if rollback.is_empty() {
                    primary
                } else {
                    format!("{primary}; rollback failed: {}", rollback.join("; "))
                });
            }
            installed.push((dest.clone(), backup));
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback = Vec::new();
            for (installed_dest, backup) in installed.into_iter().rev() {
                if let Err(e) = std::fs::remove_file(&installed_dest) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        rollback.push(format!("remove {}: {e}", installed_dest.display()));
                    }
                }
                if let Some(backup) = backup {
                    if let Err(e) = std::fs::rename(&backup, &installed_dest) {
                        rollback.push(format!(
                            "restore backup {} to {}: {e}",
                            backup.display(),
                            installed_dest.display()
                        ));
                    }
                }
            }
            for (pending, _) in &ready[index..] {
                if let Err(e) = std::fs::remove_file(pending) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        rollback.push(format!("remove staged {}: {e}", pending.display()));
                    }
                }
            }
            return Err(if rollback.is_empty() {
                error
            } else {
                format!("{error}; rollback failed: {}", rollback.join("; "))
            });
        }
    }
    for (_, backup) in installed {
        if let Some(backup) = backup {
            std::fs::remove_file(&backup)
                .map_err(|e| format!("remove restore backup {}: {e}", backup.display()))?;
        }
    }
    Ok(())
}

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
                git_out(workspace, &["checkout", head, "--", "."])?;
            }
        }
        "files" => {
            let id_path = Path::new(&meta.id);
            let mut id_components = id_path.components();
            if !matches!(id_components.next(), Some(std::path::Component::Normal(_)))
                || id_components.next().is_some()
            {
                return Err(format!("invalid checkpoint id {:?}", meta.id));
            }
            let dir = checkpoints_dir(workspace).join(&meta.id);
            let mut staged = Vec::with_capacity(meta.paths.len());
            for rel in &meta.paths {
                let src = crate::workspace::resolve(&dir, rel)?;
                let dest = crate::workspace::resolve(workspace, rel)?;
                if !src.is_file() {
                    return Err(format!(
                        "checkpoint source {} is missing or not a file",
                        src.display()
                    ));
                }
                staged.push((src, dest));
            }
            let mut ready = Vec::with_capacity(staged.len());
            for (src, dest) in staged {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("create restore parent {}: {e}", parent.display()))?;
                }
                let tmp = crate::fsutil::unique_tmp(&dest);
                if let Err(e) = std::fs::copy(&src, &tmp) {
                    for (pending, _) in &ready {
                        let _ = std::fs::remove_file(pending);
                    }
                    return Err(format!(
                        "stage checkpoint source {} for {}: {e}",
                        src.display(),
                        dest.display()
                    ));
                }
                ready.push((tmp, dest));
            }
            install_restored_files(ready, None)?;
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
    fn file_checkpoint_rejects_paths_outside_workspace() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-escape-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let err = create(&dir, None, "escape", &["../outside".into()], true).unwrap_err();
        assert!(err.contains("workspace escape denied"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_create_reports_index_open_failure() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-index-fail-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let blocked_parent = dir.join("not-a-directory");
        std::fs::write(&blocked_parent, "file").unwrap();
        let session = blocked_parent.join("session.jsonl");
        let err = create(
            &dir,
            Some(&session),
            "index-failure",
            &["a.txt".into()],
            true,
        )
        .unwrap_err();
        assert!(err.contains("checkpoint index parent"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_checkpoint_restore_reports_missing_snapshot() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-missing-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let meta = create(&dir, None, "create", &["a.txt".into()], true).unwrap();
        std::fs::remove_file(
            dir.join(".catalyst-code/checkpoints")
                .join(&meta.id)
                .join("a.txt"),
        )
        .unwrap();
        let err = restore(&dir, None, &meta.id).unwrap_err();
        assert!(err.contains("missing or not a file"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_restore_install_failure_rolls_back_all_destinations() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-install-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "current-a").unwrap();
        std::fs::write(&b, "current-b").unwrap();
        let tmp_a = crate::fsutil::unique_tmp(&a);
        let tmp_b = crate::fsutil::unique_tmp(&b);
        std::fs::write(&tmp_a, "snapshot-a").unwrap();
        std::fs::write(&tmp_b, "snapshot-b").unwrap();

        let err = install_restored_files(
            vec![(tmp_a.clone(), a.clone()), (tmp_b.clone(), b.clone())],
            Some(1),
        )
        .unwrap_err();
        assert!(err.contains("injected"), "{err}");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "current-a");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "current-b");
        assert!(!tmp_a.exists());
        assert!(!tmp_b.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_id_is_unique_and_has_cp_prefix() {
        let first = new_id();
        let second = new_id();
        assert!(first.starts_with("cp-"), "id must start with cp-: {first}");
        assert_ne!(
            first, second,
            "checkpoints created in one second must not collide"
        );
    }

    #[test]
    fn file_restore_preflight_prevents_partial_restore() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-atomic-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "snapshot-a").unwrap();
        std::fs::write(dir.join("b.txt"), "snapshot-b").unwrap();
        let meta = create(
            &dir,
            None,
            "atomic",
            &["a.txt".into(), "b.txt".into()],
            true,
        )
        .unwrap();
        std::fs::write(dir.join("a.txt"), "current-a").unwrap();
        std::fs::write(dir.join("b.txt"), "current-b").unwrap();
        std::fs::remove_file(
            dir.join(".catalyst-code/checkpoints")
                .join(&meta.id)
                .join("b.txt"),
        )
        .unwrap();

        assert!(restore(&dir, None, &meta.id).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "current-a"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "current-b"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_checkpoint_preserves_staging_state() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("catcode-cp-git-index-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "checkpoint@test.invalid"]);
        git(&["config", "user.name", "Checkpoint Test"]);
        std::fs::write(dir.join("staged.txt"), "base\n").unwrap();
        std::fs::write(dir.join("unstaged.txt"), "base\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
        git(&["add", "staged.txt"]);
        std::fs::write(dir.join("unstaged.txt"), "unstaged\n").unwrap();
        std::fs::write(dir.join("untracked.txt"), "untracked\n").unwrap();

        create(&dir, None, "preserve-index", &[], true).unwrap();

        assert_eq!(git(&["diff", "--cached", "--name-only"]), "staged.txt");
        assert_eq!(git(&["diff", "--name-only"]), "unstaged.txt");
        assert!(git(&["status", "--porcelain"])
            .lines()
            .any(|line| line == "?? untracked.txt"));
        let _ = std::fs::remove_dir_all(&dir);
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
