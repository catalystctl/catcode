// Filesystem-based git state reader. Reads .git directory directly
// (HEAD, refs, config, packed-refs) to extract branch, SHA, remote,
// and default branch. No external git dependency — pure std.
use std::path::{Path, PathBuf};

pub struct GitContext {
    pub branch: String,
    pub head_sha: String,
    pub remote_url: Option<String>,
    pub default_branch: String,
}

/// Read git context from a workspace directory. Returns None if the
/// workspace is not a git repository (no .git directory).
pub fn read_git_context(workspace: &Path) -> Option<GitContext> {
    let dot_git = workspace.join(".git");
    // `.git` is usually a directory, but for a linked worktree or submodule it
    // is a FILE — a `gitdir: <path>` pointer. Treat either as a valid repo.
    if !dot_git.exists() {
        return None;
    }
    // Follow the gitdir pointer (if any) to the real git dir, then resolve the
    // common dir: linked worktrees keep HEAD in their own git dir but share
    // refs/config/packed-refs with the main repo via a `commondir` file.
    let git_dir = resolve_gitdir(&dot_git, workspace);
    let common_dir = resolve_common_dir(&git_dir);

    let branch = read_branch(&git_dir);
    let head_sha = read_head_sha(&common_dir, &branch);
    let remote_url = read_remote_url(&common_dir);
    let default_branch = read_default_branch(&common_dir);

    Some(GitContext {
        branch,
        head_sha,
        remote_url,
        default_branch,
    })
}

/// Build a prompt injection string describing the git context.
pub fn git_context_injection(ctx: &GitContext) -> String {
    match &ctx.remote_url {
        Some(url) => format!(
            "You are working on branch `{}` (HEAD: {}) in repo <{}>. The default branch is `{}`.",
            ctx.branch, ctx.head_sha, url, ctx.default_branch
        ),
        None => format!(
            "You are working on branch `{}` (HEAD: {}). The default branch is `{}`.",
            ctx.branch, ctx.head_sha, ctx.default_branch
        ),
    }
}

// --- internal helpers ---

/// If `.git` is a `gitdir: <path>` pointer file, follow it to the real git dir
/// (a relative path is resolved against the worktree root — the directory that
/// contains the `.git` file). Otherwise return the path unchanged (a normal
/// `.git` directory). Mirrors `git rev-parse --git-dir` for worktrees/submodules
/// without shelling out to `git`.
fn resolve_gitdir(dot_git: &Path, workspace: &Path) -> PathBuf {
    if dot_git.is_file() {
        if let Ok(content) = std::fs::read_to_string(dot_git) {
            let line = content.lines().next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("gitdir: ") {
                let p = PathBuf::from(rest);
                if p.is_absolute() {
                    return p;
                }
                return workspace.join(p);
            }
        }
    }
    dot_git.to_path_buf()
}

/// Resolve the common git dir (shared refs/config/packed-refs) for a linked
/// worktree. A worktree's git dir contains a `commondir` file pointing at the
/// main `.git`; for a normal repo or submodule there is none, so the git dir
/// itself is the common dir. Mirrors `git rev-parse --git-common-dir`.
fn resolve_common_dir(git_dir: &Path) -> PathBuf {
    let commondir_file = git_dir.join("commondir");
    if let Ok(content) = std::fs::read_to_string(&commondir_file) {
        let line = content.lines().next().unwrap_or("").trim();
        if !line.is_empty() {
            let p = PathBuf::from(line);
            if p.is_absolute() {
                return p;
            }
            return git_dir.join(p);
        }
    }
    git_dir.to_path_buf()
}

fn read_branch(git_dir: &Path) -> String {
    let head_path = git_dir.join("HEAD");
    let content = match std::fs::read_to_string(&head_path) {
        Ok(c) => c,
        Err(_) => return "unknown-branch".to_string(),
    };
    let line = content.lines().next().unwrap_or("").trim();
    if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
        rest.to_string()
    } else {
        "unknown-branch".to_string()
    }
}

fn read_head_sha(git_dir: &Path, branch: &str) -> String {
    let loose = git_dir.join("refs").join("heads").join(branch);
    if let Ok(sha) = std::fs::read_to_string(&loose) {
        let short: String = sha.trim().chars().take(7).collect();
        if short.len() == 7 {
            return short;
        }
    }
    if let Some(sha) = read_packed_ref(git_dir, &format!("refs/heads/{branch}")) {
        return sha.chars().take(7).collect();
    }
    "0000000".to_string()
}

fn read_packed_ref(git_dir: &Path, ref_name: &str) -> Option<String> {
    let packed = git_dir.join("packed-refs");
    let content = std::fs::read_to_string(&packed).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('^') {
            continue;
        }
        if let Some((sha, name)) = trimmed.split_once(' ') {
            if name == ref_name {
                return Some(sha.to_string());
            }
        }
    }
    None
}

fn read_remote_url(git_dir: &Path) -> Option<String> {
    let config_path = git_dir.join("config");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let mut in_origin = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[remote \"origin\"]" {
            in_origin = true;
            continue;
        }
        if in_origin {
            if trimmed.starts_with('[') {
                break;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "url" {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

fn read_default_branch(git_dir: &Path) -> String {
    let origin_head = git_dir
        .join("refs")
        .join("remotes")
        .join("origin")
        .join("HEAD");
    if let Ok(content) = std::fs::read_to_string(&origin_head) {
        let first = content.lines().next().unwrap_or("").trim();
        if let Some(rest) = first.strip_prefix("ref: refs/remotes/origin/") {
            return rest.to_string();
        }
    }
    for name in &["main", "master"] {
        if git_dir.join("refs/remotes/origin").join(name).exists() {
            return name.to_string();
        }
    }
    "main".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Create a temp directory with a fake .git layout for tests.
    fn tmp_git(branch: &str, sha: &str, remote_url: Option<&str>, default_branch: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("umans_git_ctx_test_{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();

        // HEAD
        fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();

        // Loose ref for branch (branch may contain '/' — ensure parent dirs exist)
        let heads = git.join("refs").join("heads");
        let loose_ref = heads.join(branch);
        if let Some(parent) = loose_ref.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&loose_ref, format!("{sha}\n")).unwrap();

        // Config
        if let Some(url) = remote_url {
            fs::write(
                git.join("config"),
                format!(
                    "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
                ),
            )
            .unwrap();
        }

        // Remote HEAD
        let origin_refs = git.join("refs").join("remotes").join("origin");
        fs::create_dir_all(&origin_refs).unwrap();
        fs::write(
            origin_refs.join("HEAD"),
            format!("ref: refs/remotes/origin/{default_branch}\n"),
        )
        .unwrap();
        // Touch the remote branch ref so the fallback check passes too.
        fs::write(origin_refs.join(default_branch), format!("{sha}\n")).unwrap();

        dir
    }

    fn short(sha: &str) -> String {
        sha.chars().take(7).collect()
    }

    #[test]
    fn full_context() {
        let d = tmp_git(
            "feature/xyz",
            "abcd12345678901234567890",
            Some("git@github.com:user/repo.git"),
            "main",
        );
        let ctx = read_git_context(&d).expect("should find .git");
        assert_eq!(ctx.branch, "feature/xyz");
        assert_eq!(ctx.head_sha, short("abcd12345678901234567890"));
        assert_eq!(
            ctx.remote_url.as_deref(),
            Some("git@github.com:user/repo.git")
        );
        assert_eq!(ctx.default_branch, "main");
    }

    #[test]
    fn non_git_dir_returns_none() {
        let d = tmp_git("main", "abcd12345678901234567890", None, "main");
        // Remove .git so it's not a repo.
        fs::remove_dir_all(d.join(".git")).unwrap();
        assert!(read_git_context(&d).is_none());
    }

    #[test]
    fn detached_head_is_unknown_branch() {
        let d = tmp_git("main", "abcd12345678901234567890", None, "main");
        // Write a raw SHA as HEAD (detached state).
        fs::write(d.join(".git/HEAD"), "deadbeef12345678901234567890\n").unwrap();
        let ctx = read_git_context(&d).expect("should find .git");
        assert_eq!(ctx.branch, "unknown-branch");
    }

    #[test]
    fn injection_with_remote() {
        let ctx = GitContext {
            branch: "feature/x".into(),
            head_sha: "abc1234".into(),
            remote_url: Some("https://github.com/u/r.git".into()),
            default_branch: "main".into(),
        };
        let s = git_context_injection(&ctx);
        assert!(s.contains("branch `feature/x`"));
        assert!(s.contains("HEAD: abc1234"));
        assert!(s.contains("https://github.com/u/r.git"));
        assert!(s.contains("default branch is `main`"));
    }

    #[test]
    fn injection_without_remote() {
        let ctx = GitContext {
            branch: "dev".into(),
            head_sha: "def5678".into(),
            remote_url: None,
            default_branch: "master".into(),
        };
        let s = git_context_injection(&ctx);
        assert!(s.contains("branch `dev`"));
        assert!(s.contains("HEAD: def5678"));
        assert!(s.contains("default branch is `master`"));
        assert!(!s.contains("repo"));
    }

    #[test]
    fn packed_refs_fallback() {
        let d = tmp_git("main", "abcd12345678901234567890", None, "main");
        // Remove the loose ref so we fall back to packed-refs.
        fs::remove_file(d.join(".git/refs/heads/main")).unwrap();
        fs::write(
            d.join(".git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\ndeadbeef12345678901234567890 refs/heads/main\nffeeddcc12345678901234567890 refs/heads/other\n",
        )
        .unwrap();
        let ctx = read_git_context(&d).expect("should find .git");
        assert_eq!(ctx.head_sha, short("deadbeef12345678901234567890"));
    }

    #[test]
    fn default_branch_fallback_to_master() {
        let d = tmp_git("main", "abcd12345678901234567890", None, "main");
        // Remove origin/HEAD symref and origin/main ref.
        let origin = d.join(".git/refs/remotes/origin");
        fs::remove_file(origin.join("HEAD")).unwrap();
        fs::remove_file(origin.join("main")).unwrap();
        // Create origin/master ref instead.
        fs::write(origin.join("master"), "abcd12345678901234567890\n").unwrap();
        let ctx = read_git_context(&d).expect("should find .git");
        assert_eq!(ctx.default_branch, "master");
    }

    #[test]
    fn default_branch_fallback_to_main_hardcoded() {
        let d = tmp_git("main", "abcd12345678901234567890", None, "main");
        let origin = d.join(".git/refs/remotes/origin");
        fs::remove_file(origin.join("HEAD")).unwrap();
        fs::remove_file(origin.join("main")).unwrap();
        // No main, no master → falls back to "main".
        let ctx = read_git_context(&d).expect("should find .git");
        assert_eq!(ctx.default_branch, "main");
    }

    fn unique_dir(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static M: AtomicU64 = AtomicU64::new(0);
        let n = M.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("{prefix}_{n}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn gitdir_file_pointer_is_followed() {
        // A submodule's `.git` is a FILE: a `gitdir: <path>` pointer to a
        // complete git dir. The reader must follow it instead of returning None
        // (the old `is_dir()` gate dropped all worktree/submodule context).
        let main = tmp_git(
            "feature/sub",
            "0123456789abcdef",
            Some("git@github.com:u/sub.git"),
            "main",
        );
        let main_git = main.join(".git");
        // Workspace whose `.git` is a pointer file (not a directory).
        let ws = unique_dir("umans_git_ctx_sub");
        fs::write(ws.join(".git"), format!("gitdir: {}\n", main_git.display())).unwrap();
        let ctx = read_git_context(&ws).expect("gitdir pointer should resolve");
        assert_eq!(ctx.branch, "feature/sub");
        assert_eq!(ctx.head_sha, short("0123456789abcdef"));
        assert_eq!(ctx.remote_url.as_deref(), Some("git@github.com:u/sub.git"));
        assert_eq!(ctx.default_branch, "main");
    }

    #[test]
    fn linked_worktree_uses_commondir() {
        // A linked worktree's `.git` points to a per-worktree git dir that holds
        // only HEAD; refs/config/packed-refs live in the COMMON dir (main .git),
        // referenced by a `commondir` file. HEAD must come from the worktree dir
        // and everything else from the common dir.
        let main = tmp_git(
            "main",
            "abcdef0123456789",
            Some("git@github.com:u/wt.git"),
            "main",
        );
        let main_git = main.join(".git");
        // The worktree's branch is a normal ref stored in the common (main) repo.
        fs::write(main_git.join("refs/heads/wt-branch"), "abcdef0123456789\n").unwrap();

        // Per-worktree git dir: HEAD + a commondir pointer to the main repo.
        let wt_git = unique_dir("umans_git_ctx_wtgit");
        fs::write(wt_git.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        fs::write(
            wt_git.join("commondir"),
            format!("{}\n", main_git.display()),
        )
        .unwrap();

        // Workspace whose `.git` is a pointer to the per-worktree git dir.
        let ws = unique_dir("umans_git_ctx_wtws");
        fs::write(ws.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();

        let ctx = read_git_context(&ws).expect("worktree should resolve");
        // Branch from the worktree's own HEAD.
        assert_eq!(ctx.branch, "wt-branch");
        // SHA/remote/default pulled from the common (main) dir.
        assert_eq!(ctx.head_sha, short("abcdef0123456789"));
        assert_eq!(ctx.remote_url.as_deref(), Some("git@github.com:u/wt.git"));
        assert_eq!(ctx.default_branch, "main");
    }
}
