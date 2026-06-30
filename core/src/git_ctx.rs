// Filesystem-based git state reader. Reads .git directory directly
// (HEAD, refs, config, packed-refs) to extract branch, SHA, remote,
// and default branch. No external git dependency — pure std.
use std::path::Path;

pub struct GitContext {
    pub branch: String,
    pub head_sha: String,
    pub remote_url: Option<String>,
    pub default_branch: String,
}

/// Read git context from a workspace directory. Returns None if the
/// workspace is not a git repository (no .git directory).
pub fn read_git_context(workspace: &Path) -> Option<GitContext> {
    let git_dir = workspace.join(".git");
    if !git_dir.is_dir() {
        return None;
    }

    let branch = read_branch(&git_dir);
    let head_sha = read_head_sha(&git_dir, &branch);
    let remote_url = read_remote_url(&git_dir);
    let default_branch = read_default_branch(&git_dir);

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
}
