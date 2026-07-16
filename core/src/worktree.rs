//! Git worktree helpers for parallel subagent isolation.
//!
//! When `worktree: true` is set on a parallel subagent (or goal deploy with
//! concurrency > 1 for mutating agents), each task gets a linked worktree under
//! `.catalyst-code/worktrees/<run_id>/` so writers cannot clobber each other.
//! Non-git workspaces cannot isolate this way — callers should error clearly.
//!
//! All add/remove calls are serialized: concurrent `git worktree add` races on
//! `.git` locks and can fail spuriously (observed as uniform goal-step failures).
//!
//! Goal waves: `promote_worktree` copies wt→main; the next wave's `add_worktree`
//! checks out HEAD only, so call `seed_worktree_from_main` (main→wt) so dependents
//! see uncommitted promoted files from prior waves.

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
/// Also removes from main any tracked paths deleted in the worktree vs HEAD
/// (`git diff --diff-filter=D`). Does **not** full-mirror-delete (would wipe
/// sibling promotions on concurrent waves).
pub fn promote_worktree(main_ws: &Path, wt_path: &Path) -> Result<Vec<String>, String> {
    let mut promoted = Vec::new();
    copy_tree_diff(wt_path, main_ws, wt_path, &mut promoted)?;
    sync_git_deletions(wt_path, main_ws, &mut promoted)?;
    Ok(promoted)
}

/// After `add_worktree` (checkout HEAD only), mirror main's working tree into
/// the new worktree so dependent goal steps see prior-wave output — including
/// deletions (full mirror under skip rules). Memory: promote is wt→main; seed
/// is main→wt — without seed, next-wave workers miss uncommitted promotes.
pub fn seed_worktree_from_main(main_ws: &Path, wt_path: &Path) -> Result<Vec<String>, String> {
    let mut seeded = Vec::new();
    copy_tree_diff(main_ws, wt_path, main_ws, &mut seeded)?;
    // Full mirror deletes: drop wt paths that no longer exist on main.
    remove_dest_missing_from_src(main_ws, wt_path, wt_path, &mut seeded)?;
    Ok(seeded)
}

fn should_skip_name(name: &str) -> bool {
    name == ".git" || name == ".catalyst-code"
}

/// Recursively copy files under `dir` (within `src_root`) into `dest_root` when
/// content differs or the dest file is missing. Skips `.git` and `.catalyst-code`.
fn copy_tree_diff(
    src_root: &Path,
    dest_root: &Path,
    dir: &Path,
    out_paths: &mut Vec<String>,
) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for ent in rd.flatten() {
        let p = ent.path();
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if should_skip_name(&name_s) {
            continue;
        }
        let rel = p
            .strip_prefix(src_root)
            .map_err(|_| "strip prefix".to_string())?;
        let dest = dest_root.join(rel);
        let ft = ent.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            copy_tree_diff(src_root, dest_root, &p, out_paths)?;
        } else if ft.is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let src_bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let same = std::fs::read(&dest)
                .ok()
                .map(|b| b == src_bytes)
                .unwrap_or(false);
            if !same {
                std::fs::write(&dest, &src_bytes).map_err(|e| e.to_string())?;
                out_paths.push(rel.display().to_string());
            }
        }
    }
    Ok(())
}

/// Remove paths under `dest_dir` that have no counterpart under `src_root`.
/// Used by seed to full-mirror main→worktree (including deletions).
fn remove_dest_missing_from_src(
    src_root: &Path,
    dest_root: &Path,
    dest_dir: &Path,
    out_paths: &mut Vec<String>,
) -> Result<(), String> {
    let rd = match std::fs::read_dir(dest_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", dest_dir.display())),
    };
    for ent in rd.flatten() {
        let p = ent.path();
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if should_skip_name(&name_s) {
            continue;
        }
        let rel = p
            .strip_prefix(dest_root)
            .map_err(|_| "strip prefix".to_string())?;
        let src = src_root.join(rel);
        let ft = ent.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            remove_dest_missing_from_src(src_root, dest_root, &p, out_paths)?;
            if src.is_dir() {
                continue;
            }
            if std::fs::read_dir(&p)
                .map(|mut i| i.next().is_none())
                .unwrap_or(false)
            {
                let _ = std::fs::remove_dir(&p);
                out_paths.push(format!("deleted:{}", rel.display()));
            } else if !src.exists() {
                std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
                out_paths.push(format!("deleted:{}", rel.display()));
            }
        } else if ft.is_file() && !src.exists() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            out_paths.push(format!("deleted:{}", rel.display()));
        }
    }
    Ok(())
}

/// Apply tracked deletions from `src_repo` (vs HEAD) onto `dest_root`.
/// Safe for promote under concurrency — only paths the worktree deleted.
fn sync_git_deletions(
    src_repo: &Path,
    dest_root: &Path,
    out_paths: &mut Vec<String>,
) -> Result<(), String> {
    if !is_git_repo(src_repo) {
        return Ok(());
    }
    let out = match git_out(src_repo, &["diff", "--name-only", "--diff-filter=D", "HEAD"]) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    for line in out.lines() {
        let rel = line.trim();
        if rel.is_empty() {
            continue;
        }
        let dest = dest_root.join(rel);
        if !dest.exists() {
            continue;
        }
        if dest.is_dir() {
            std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(&dest).map_err(|e| e.to_string())?;
        }
        out_paths.push(format!("deleted:{rel}"));
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
    fn seed_worktree_from_main_copies_uncommitted_files() {
        // promote copies wt→main; add_worktree checks out HEAD only — seed must
        // bring uncommitted main files into the new worktree for dependent waves.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let main = std::env::temp_dir().join(format!("catcode-seed-main-{stamp}"));
        let _ = std::fs::remove_dir_all(&main);
        std::fs::create_dir_all(&main).unwrap();

        // Init a real git repo with committed file A.
        for (args, msg) in [
            (vec!["init"], "init"),
            (vec!["config", "user.email", "test@example.com"], "email"),
            (vec!["config", "user.name", "test"], "name"),
        ] {
            let st = Command::new("git")
                .args(&args)
                .current_dir(&main)
                .status()
                .unwrap_or_else(|e| panic!("{msg}: {e}"));
            assert!(st.success(), "git {msg} failed");
        }
        // Detach from any template/default branch naming quirks.
        let _ = Command::new("git")
            .args(["checkout", "-b", "main"])
            .current_dir(&main)
            .status();
        std::fs::write(main.join("A.txt"), b"committed").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "A.txt"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["commit", "-m", "add A"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );

        // Dirty uncommitted file B in main (simulates prior-wave promote).
        std::fs::write(main.join("B.txt"), b"promoted dirty").unwrap();

        let wt = add_worktree(&main, &format!("seed-{stamp}")).unwrap();
        // Fresh worktree has A (from HEAD) but not dirty B until seeded.
        assert!(wt.join("A.txt").exists());
        assert!(
            !wt.join("B.txt").exists(),
            "add_worktree alone must not see uncommitted B"
        );

        let seeded = seed_worktree_from_main(&main, &wt).unwrap();
        assert!(seeded.iter().any(|p| p.contains("B.txt")), "{seeded:?}");
        assert_eq!(
            std::fs::read_to_string(wt.join("B.txt")).unwrap(),
            "promoted dirty"
        );

        let _ = remove_worktree(&main, &wt);
        let _ = std::fs::remove_dir_all(&main);
    }

    #[test]
    fn seed_worktree_from_main_removes_deleted_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let main = std::env::temp_dir().join(format!("catcode-seed-del-main-{stamp}"));
        let _ = std::fs::remove_dir_all(&main);
        std::fs::create_dir_all(&main).unwrap();

        for (args, msg) in [
            (vec!["init"], "init"),
            (vec!["config", "user.email", "test@example.com"], "email"),
            (vec!["config", "user.name", "test"], "name"),
        ] {
            let st = Command::new("git")
                .args(&args)
                .current_dir(&main)
                .status()
                .unwrap_or_else(|e| panic!("{msg}: {e}"));
            assert!(st.success(), "git {msg} failed");
        }
        let _ = Command::new("git")
            .args(["checkout", "-b", "main"])
            .current_dir(&main)
            .status();
        std::fs::write(main.join("A.txt"), b"committed").unwrap();
        assert!(Command::new("git")
            .args(["add", "A.txt"])
            .current_dir(&main)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "add A"])
            .current_dir(&main)
            .status()
            .unwrap()
            .success());

        // Simulate prior-wave deletion on main (still present at HEAD).
        std::fs::remove_file(main.join("A.txt")).unwrap();

        let wt = add_worktree(&main, &format!("seed-del-{stamp}")).unwrap();
        assert!(wt.join("A.txt").exists(), "fresh wt still has HEAD A.txt");

        let seeded = seed_worktree_from_main(&main, &wt).unwrap();
        assert!(!wt.join("A.txt").exists(), "seed must delete A.txt from wt");
        assert!(
            seeded.iter().any(|p| p.contains("deleted:A.txt")),
            "expected deleted:A.txt in {seeded:?}"
        );

        let _ = remove_worktree(&main, &wt);
        let _ = std::fs::remove_dir_all(&main);
    }

    #[test]
    fn promote_worktree_removes_tracked_deletions() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let main = std::env::temp_dir().join(format!("catcode-promote-del-main-{stamp}"));
        let _ = std::fs::remove_dir_all(&main);
        std::fs::create_dir_all(&main).unwrap();

        for (args, msg) in [
            (vec!["init"], "init"),
            (vec!["config", "user.email", "test@example.com"], "email"),
            (vec!["config", "user.name", "test"], "name"),
        ] {
            let st = Command::new("git")
                .args(&args)
                .current_dir(&main)
                .status()
                .unwrap_or_else(|e| panic!("{msg}: {e}"));
            assert!(st.success(), "git {msg} failed");
        }
        let _ = Command::new("git")
            .args(["checkout", "-b", "main"])
            .current_dir(&main)
            .status();
        std::fs::write(main.join("A.txt"), b"committed").unwrap();
        assert!(Command::new("git")
            .args(["add", "A.txt"])
            .current_dir(&main)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "add A"])
            .current_dir(&main)
            .status()
            .unwrap()
            .success());

        let wt = add_worktree(&main, &format!("promote-del-{stamp}")).unwrap();
        assert!(wt.join("A.txt").exists());
        std::fs::remove_file(wt.join("A.txt")).unwrap();

        let promoted = promote_worktree(&main, &wt).unwrap();
        assert!(!main.join("A.txt").exists(), "promote must delete A.txt on main");
        assert!(
            promoted.iter().any(|p| p.contains("deleted:A.txt")),
            "expected deleted:A.txt in {promoted:?}"
        );

        let _ = remove_worktree(&main, &wt);
        let _ = std::fs::remove_dir_all(&main);
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
