//! Git worktree helpers for parallel subagent isolation.
//!
//! When `worktree: true` is set on a parallel subagent (or goal deploy with
//! concurrency > 1 for mutating agents), each task gets a linked worktree under
//! `.catalyst-code/worktrees/<run_id>/` so writers cannot clobber each other.
//! Non-git workspaces cannot isolate this way — callers should error clearly.
//!
//! All add/remove calls are serialized: concurrent `git worktree add` races on
//! `.git` locks and can fail spuriously (observed as uniform goal-step failures).

use crate::protocol::{emit, Event};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Process-wide lock so concurrent goal/parallel steps don't race `git worktree`.
static WORKTREE_LOCK: Mutex<()> = Mutex::new(());

/// True when `workspace` is inside a git working tree.
pub fn is_git_repo(workspace: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn worktrees_root(workspace: &Path) -> PathBuf {
    workspace.join(".catalyst-code").join("worktrees")
}

fn git_out(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("git {} failed to spawn: {e}", args.first().unwrap_or(&"")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            err.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Create a linked worktree for `run_id`. Returns the absolute worktree path.
pub fn add_worktree(workspace: &Path, run_id: &str) -> Result<PathBuf, String> {
    let _guard = WORKTREE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    add_worktree_locked(workspace, run_id)
}

fn add_worktree_locked(workspace: &Path, run_id: &str) -> Result<PathBuf, String> {
    if !is_git_repo(workspace) {
        return Err(
            "worktree isolation requires a git repository; use checkpoints instead of worktree:true"
                .into(),
        );
    }
    let root = worktrees_root(workspace);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("create worktrees dir: {e}"))?;
    // Sanitize run_id for a branch/path segment.
    let safe: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let path = root.join(&safe);
    if path.exists() {
        let _ = remove_worktree_locked(workspace, &path);
    }
    let branch = format!("catcode/wt/{safe}");
    // Prefer creating from HEAD; detach if branch already exists.
    let head = git_out(workspace, &["rev-parse", "HEAD"])?;
    let add = Command::new("git")
        .args([
            "worktree",
            "add",
            "-B",
            &branch,
            path.to_str().unwrap_or("."),
            &head,
        ])
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("git worktree add spawn: {e}"))?;
    if !add.status.success() {
        let err = String::from_utf8_lossy(&add.stderr);
        return Err(format!("git worktree add failed: {}", err.trim()));
    }
    emit(
        &Event::new("worktree_ready")
            .with("run_id", json!(run_id))
            .with("path", json!(path.display().to_string()))
            .with("branch", json!(branch)),
    );
    Ok(path)
}

/// Remove a linked worktree (best-effort).
pub fn remove_worktree(workspace: &Path, wt_path: &Path) -> Result<(), String> {
    let _guard = WORKTREE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    remove_worktree_locked(workspace, wt_path)
}

fn remove_worktree_locked(workspace: &Path, wt_path: &Path) -> Result<(), String> {
    let path_str = wt_path.display().to_string();
    let out = Command::new("git")
        .args(["worktree", "remove", "--force", &path_str])
        .current_dir(workspace)
        .output()
        .map_err(|e| format!("git worktree remove spawn: {e}"))?;
    if !out.status.success() {
        // Fall back to plain rm if git refuses (already pruned, etc.).
        let _ = std::fs::remove_dir_all(wt_path);
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(workspace)
            .status();
    }
    emit(
        &Event::new("worktree_cleaned")
            .with("path", json!(path_str)),
    );
    Ok(())
}

/// Copy changed tracked+untracked files from a worktree back into the main
/// workspace (shadow promote). Skips `.git` and `.catalyst-code`.
pub fn promote_worktree(main_ws: &Path, wt_path: &Path) -> Result<Vec<String>, String> {
    let mut promoted = Vec::new();
    promote_dir(main_ws, wt_path, wt_path, &mut promoted)?;
    Ok(promoted)
}

fn promote_dir(
    main_ws: &Path,
    wt_root: &Path,
    dir: &Path,
    promoted: &mut Vec<String>,
) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for ent in rd.flatten() {
        let p = ent.path();
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if name_s == ".git" || name_s == ".catalyst-code" {
            continue;
        }
        let rel = p
            .strip_prefix(wt_root)
            .map_err(|_| "strip prefix".to_string())?;
        let dest = main_ws.join(rel);
        let ft = ent.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            promote_dir(main_ws, wt_root, &p, promoted)?;
        } else if ft.is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // Only copy if content differs (or dest missing).
            let src_bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let same = std::fs::read(&dest)
                .ok()
                .map(|b| b == src_bytes)
                .unwrap_or(false);
            if !same {
                std::fs::write(&dest, &src_bytes).map_err(|e| e.to_string())?;
                promoted.push(rel.display().to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_git_repo_detects_this_workspace() {
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(is_git_repo(&here));
    }

    #[test]
    fn add_worktree_errors_outside_git() {
        let dir = std::env::temp_dir().join("catcode-nongit-wt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = add_worktree(&dir, "t1").unwrap_err();
        assert!(err.contains("git repository"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_git_repo_false_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("catcode-wt-nongit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_git_repo(&dir), "empty temp dir must not be a git repo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn promote_dir_copies_changed_file() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let main = std::env::temp_dir().join(format!("catcode-promote-main-{stamp}"));
        let wt = std::env::temp_dir().join(format!("catcode-promote-wt-{stamp}"));
        let _ = std::fs::remove_dir_all(&main);
        let _ = std::fs::remove_dir_all(&wt);
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        // Write a file only in the worktree.
        std::fs::write(wt.join("new.txt"), b"wt content").unwrap();
        let promoted = promote_worktree(&main, &wt).unwrap();
        assert!(promoted.iter().any(|p| p.contains("new.txt")));
        assert_eq!(
            std::fs::read_to_string(main.join("new.txt")).unwrap(),
            "wt content"
        );
        let _ = std::fs::remove_dir_all(&main);
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[test]
    fn remove_worktree_handles_nonexistent_path() {
        // Should not panic for a path that doesn't exist.
        let dir = std::env::temp_dir().join(format!("catcode-rmwt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Drive-by best-effort: even if git fails, the function shouldn't panic.
        let result = remove_worktree(&dir, &dir.join("nonexistent"));
        // We just assert it doesn't panic; result may be Ok or Err.
        let _ = result;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
