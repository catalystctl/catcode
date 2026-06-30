// Workspace path confinement. Every file tool resolves paths against a root
// and rejects escapes (absolute paths, `..` traversal, symlinks pointing out).
// bash runs with cwd locked to the root.
// Also includes a dangerous-path blocklist for write/edit operations.
use std::path::{Path, PathBuf};

/// Dangerous paths that should never be written to or edited by the agent.
/// These are shell/ssh config files and VCS internals that could cause
/// permanent damage or security issues if modified by an AI.
const DANGEROUS_PATHS: &[&str] = &[
    ".git/**",
    "**/.bashrc",
    "**/.bash_profile",
    "**/.profile",
    "**/.zshrc",
    "**/.ssh/**",
    "**/.gnupg/**",
    "**/id_rsa",
    "**/id_ed25519",
    "**/.env",
    "**/.env.local",
    "**/.env.production",
];

/// Check if a resolved path matches any dangerous pattern.
/// Returns Some(error_message) if blocked, None if allowed.
pub fn check_dangerous_path(input: &str) -> Option<String> {
    for pattern in DANGEROUS_PATHS {
        if glob_match_path(pattern, input) {
            return Some(format!(
                "path {input:?} matches dangerous pattern '{pattern}'; write/edit blocked"
            ));
        }
    }
    None
}

/// Collapse `./` segments, duplicate slashes, and trailing slashes from a
/// relative path so `.//.git/config`, `a/./b`, and `a//b` all match their
/// canonical forms. Dot-dirs like `.git` are kept (the `.` is part of the
/// name, not a current-dir marker).
fn normalize_rel_path(s: &str) -> String {
    s.split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Simple glob match for path patterns. Supports ** (any depth). Matching is
/// case-insensitive (so `.GIT/config`/`.SSH/...`/`.ENV` are blocked on
/// case-insensitive filesystems like macOS/Windows) and the path is normalized
/// so `.//.git/config` can't slip past a pattern by adding a stray `./`.
fn glob_match_path(pattern: &str, path: &str) -> bool {
    let pattern = normalize_rel_path(pattern).to_ascii_lowercase();
    let path = normalize_rel_path(path).to_ascii_lowercase();
    // ** matches any path depth
    if pattern.contains("**") {
        // Split on **, match prefix and suffix
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            let suffix = suffix.trim_start_matches('/');
            if suffix.is_empty() {
                // ** with a leading anchor (e.g. `.git/**`) — match the anchor
                // as a path COMPONENT (anywhere in the tree), NOT as a substring.
                // `.git/**` must match `.git/config` and `sub/.git/refs` but NOT
                // `.github/workflows/ci.yml`, which merely contains the substring
                // `.git` (a false-positive that blocked editing `.github/**`).
                let anchor = prefix.trim_start_matches('/').trim_end_matches('/');
                if anchor.is_empty() {
                    return true; // bare `**` matches everything
                }
                return path.split('/').any(|seg| seg == anchor);
            }
            // Check if path starts with prefix and ends with suffix
            // For "**/.bashrc", match any path ending with /.bashrc
            if prefix.is_empty() {
                return path == suffix
                    || path.ends_with(&format!("/{suffix}"))
                    || path == format!("/{suffix}");
            }
            if path.starts_with(prefix) {
                return path.ends_with(suffix) || path.ends_with(&format!("/{suffix}"));
            }
            return false;
        }
        if parts.len() == 3 {
            // **/anchor/** — match if the path has `anchor` as a path segment
            // (e.g. **/.ssh/** matches ".ssh/config" and "home/.ssh/x"); the two
            // ** absorb anything before/after the anchor. Fixes a latent bug
            // where two-** patterns never matched.
            let prefix = parts[0];
            let middle = parts[1];
            let suffix = parts[2];
            if !path.starts_with(prefix) || !path.ends_with(suffix) {
                return false;
            }
            let anchor = middle.trim_matches('/');
            if anchor.is_empty() {
                return true;
            }
            return path.split('/').any(|seg| seg == anchor);
        }
    }
    // Exact or suffix match for patterns without **
    path == pattern || path.ends_with(&format!("/{pattern}"))
}

/// Resolve `input` against `root`, refusing absolute paths and `..` escapes.
/// Symlinks are canonicalized and must stay within the canonical root.
pub fn resolve(root: &Path, input: &str) -> Result<PathBuf, String> {
    let p = Path::new(input);
    // Reject absolute paths outright — the agent works inside the workspace.
    if p.is_absolute() {
        return Err(format!(
            "path {input:?} is absolute; only workspace-relative paths allowed"
        ));
    }
    // Reject any component that escapes via `..`.
    for comp in p.components() {
        use std::path::Component::*;
        match comp {
            Prefix(_) | RootDir => {
                return Err(format!("path {input:?} escapes the workspace"));
            }
            ParentDir => {
                return Err(format!(
                    "path {input:?} contains '..'; workspace escape denied"
                ));
            }
            CurDir | Normal(_) => {}
        }
    }
    let joined = root.join(p);
    // Canonicalize to catch symlink escapes. The root itself may not exist yet
    // for write_file (parents created later), so canonicalize the parent chain
    // leniently: canonicalize what exists, then re-check the tail.
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canon = match std::fs::canonicalize(&joined) {
        Ok(c) => c,
        Err(_) => {
            // Target doesn't exist yet (write/create). Canonicalize the EXISTING
            // prefix incrementally so an intermediate symlinked directory that
            // points OUTSIDE the workspace is resolved (and then rejected by the
            // confinement check below). A naive `cur.join(comp)` would leave the
            // symlink un-resolved and the subsequent write would escape (P0-2).
            let mut cur = canon_root.clone();
            for comp in p.components() {
                if let std::path::Component::Normal(s) = comp {
                    let candidate = cur.join(s);
                    cur = std::fs::canonicalize(&candidate).unwrap_or(candidate);
                }
            }
            cur
        }
    };
    if !canon.starts_with(&canon_root) {
        return Err(format!("path {input:?} resolves outside the workspace"));
    }
    Ok(canon)
}

/// True if `path` (already resolved) is confined within `root`.
#[allow(dead_code)]
pub fn is_confined(root: &Path, path: &Path) -> bool {
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canon.starts_with(&canon_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root() -> PathBuf {
        // ponytail: unique dir per call via atomic counter — the old fixed name
        // raced under parallel `cargo test` (one thread removes it while another
        // canonicalizes). Mirrors tools.rs::tmp_ws.
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("umans_harness_ws_test_{}", n));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.txt"), "hi").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        d
    }

    #[test]
    fn relative_inside_ok() {
        let r = tmp_root();
        let p = resolve(&r, "a.txt").unwrap();
        assert!(p.ends_with("a.txt"));
        let p = resolve(&r, "sub/b.txt").unwrap();
        assert!(p.starts_with(std::fs::canonicalize(&r).unwrap()));
    }

    #[test]
    fn absolute_rejected() {
        let r = tmp_root();
        assert!(resolve(&r, "/etc/passwd").is_err());
    }

    #[test]
    fn parent_dir_rejected() {
        let r = tmp_root();
        assert!(resolve(&r, "../escape").is_err());
        assert!(resolve(&r, "sub/../../escape").is_err());
    }

    #[test]
    fn dangerous_paths_case_insensitive() {
        // Case-insensitive match so `.GIT/config`/`.SSH/...`/`.ENV` are blocked
        // on case-insensitive filesystems (macOS, Windows), not just lowercase.
        assert!(check_dangerous_path(".GIT/config").is_some());
        assert!(check_dangerous_path(".SSH/authorized_keys").is_some());
        assert!(check_dangerous_path(".ENV").is_some());
        assert!(check_dangerous_path(".git/config").is_some());
    }

    #[test]
    fn dangerous_paths_normalize_dot_slash() {
        // `.//.git/config` must still match `.git/**` after normalization.
        assert!(check_dangerous_path(".//.git/config").is_some());
        assert!(check_dangerous_path("a/./.env").is_some());
        // A legit path that merely contains "git" (no leading dot) is NOT .git/**.
        assert!(check_dangerous_path("mygitthing/file").is_none());
        // `.github/**` must NOT match `.git/**` (substring false-positive):
        // `.github` is a path component, not the `.git` directory.
        assert!(check_dangerous_path(".github/workflows/ci.yml").is_none());
        assert!(check_dangerous_path(".github/ISSUE_TEMPLATE/foo.md").is_none());
        // A nested `.git` directory IS blocked (component match anywhere).
        assert!(check_dangerous_path("vendor/lib/.git/config").is_some());
        assert!(check_dangerous_path("sub/.git/HEAD").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dir_escape_rejected() {
        use std::os::unix::fs::symlink;
        let r = tmp_root();
        // `linkdir` is a symlink to a directory OUTSIDE the workspace.
        let outside =
            std::env::temp_dir().join(format!("umans_harness_escape_{}", std::process::id()));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, r.join("linkdir")).unwrap();
        // Resolving a NEW file through the symlinked dir must be rejected (P0-2).
        assert!(resolve(&r, "linkdir/newfile").is_err());
        // A normal new file inside the workspace still resolves.
        assert!(resolve(&r, "sub/newfile").is_ok());
        let _ = fs::remove_dir_all(&outside);
    }
}
