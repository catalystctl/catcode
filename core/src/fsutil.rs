//! Cross-process-safe filesystem helpers: unique-temp atomic writes and an
//! advisory cross-process file lock.
//!
//! ## Why this exists
//!
//! The harness can run as multiple concurrent processes (two TUI sessions, a
//! TUI + a web server, parallel CI). Several files under
//! `~/.config/catalyst-code/` are SHARED across processes — the models cache,
//! the memory store, the pattern log, OAuth tokens, config.json, settings.json.
//! Two hazards arise when two processes touch the same shared file:
//!
//! 1. **Temp-file collision (corruption).** The atomic-write pattern writes a
//!    sibling temp file then renames it over the target. If the temp name is
//!    FIXED (e.g. `foo.json.tmp`), two concurrent writers open the SAME temp
//!    file, interleave their writes, and one renames a corrupted file over the
//!    target. Fix: every temp file gets a unique name (pid + random suffix).
//!
//! 2. **Lost update (read-modify-write).** Several writers read the existing
//!    file, merge their change, and write it back. Two concurrent writers both
//!    read the same base; the second to rename clobbers the first's change.
//!    For accumulating stores (memory) this is silent durable data loss. Fix:
//!    a cross-process advisory lock around the read-modify-write critical
//!    section.
//!
//! `presence.rs` already solved #1 for its own files (per-pid names). This
//! module generalizes the fix to every shared-file writer.

use std::io;
use std::path::{Path, PathBuf};

/// Build a unique temp-file path beside `target` (same directory). The name is
/// `.<original>.<pid>.<rand>.tmp` — hidden, unique per process AND per call, so
/// two concurrent writers never share a temp file. A crash mid-write leaves an
/// orphaned hidden temp (benign: small, rare, never read by anyone).
pub fn unique_tmp(target: &Path) -> PathBuf {
    use rand::Rng;
    let pid = std::process::id();
    let rand: u64 = rand::thread_rng().gen();
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("tmp");
    target.with_file_name(format!(".{name}.{pid}.{rand:016x}.tmp"))
}

/// Atomically write `content` to `target`: a unique temp file is written,
/// fsync'd, then renamed over the target. A crash mid-write leaves the orphaned
/// temp (never a truncated target). The unique temp means concurrent writers
/// never collide on the temp file.
pub fn atomic_write(target: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = unique_tmp(target);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.flush()?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Convenience wrapper for `&str` content.
pub fn atomic_write_str(target: &Path, content: &str) -> io::Result<()> {
    atomic_write(target, content.as_bytes())
}

/// Like [`atomic_write`] but sets 0600 perms on the file (secrets: OAuth tokens,
/// config with API keys). The temp is chmod'd BEFORE the rename so the target
/// is never briefly world-readable — matching the original oauth/config pattern.
#[cfg(unix)]
pub fn atomic_write_secure(target: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let tmp = unique_tmp(target);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.flush()?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Non-Unix fallback: no permission bits to set.
#[cfg(not(unix))]
pub fn atomic_write_secure(target: &Path, content: &[u8]) -> io::Result<()> {
    atomic_write(target, content)
}

/// An advisory cross-process exclusive lock held for the lifetime of the guard.
///
/// On Unix this is an `flock(2)` on a sidecar lock file: it blocks until
/// acquired and AUTO-RELEASES when the process exits (even on `kill -9` or a
/// crash), so there are never stale locks. On non-Unix platforms it is a no-op
/// (the unique-temp atomic write still prevents corruption; only the
/// read-modify-write lost-update remains possible, which is acceptable for the
/// rare non-Unix multi-process case).
///
/// Use it to serialize a read-modify-write critical section on a shared file:
///
/// ```ignore
/// let _lock = fsutil::FileLock::acquire(&path.with_extension("lock"))?;
/// let existing = std::fs::read_to_string(&path).unwrap_or_default();
/// // ... merge ...
/// fsutil::atomic_write(&path, &merged)?;
/// // lock released on drop
/// ```
pub struct FileLock {
    // The file handle is held for the lock's lifetime; dropping it closes the
    // fd and releases the flock. Never read/written — it exists only to own
    // the lock.
    #[cfg(unix)]
    _file: std::fs::File,
    // Mark unused on non-Unix so the field isn't flagged dead code.
    #[cfg(not(unix))]
    _file: (),
}

impl FileLock {
    /// Block until an exclusive lock on `lock_path` is acquired. Creates the
    /// lock file (and its parent dir) if absent. Returns a guard that releases
    /// the lock on drop.
    pub fn acquire(lock_path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            if let Some(parent) = lock_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // create(true) so the lock file exists; we never read/write its
            // contents — flock only needs an open file description.
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .read(true)
                // Explicitly non-truncating: this is a sidecar LOCK file whose
                // content is never read or written — flock operates on the open
                // file description, not the bytes. Truncating would be
                // meaningless (and clobber any concurrent holder's file, though
                // they don't use the bytes either).
                .truncate(false)
                .open(lock_path)?;
            // Blocking exclusive flock (LOCK_EX). Auto-released on close/exit.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(FileLock { _file: file })
        }
        #[cfg(not(unix))]
        {
            let _ = lock_path;
            Ok(FileLock { _file: () })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_tmp_is_unique_and_beside_target() {
        let p = Path::new("/tmp/catalyst_code_fsutil_test.json");
        let a = unique_tmp(p);
        let b = unique_tmp(p);
        assert_ne!(a, b, "two calls must yield different temp paths");
        assert_eq!(a.parent(), p.parent(), "temp must be in the same dir");
        assert!(a
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .starts_with(".catalyst_code_fsutil_test.json."));
    }

    #[test]
    fn atomic_write_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("catalyst_code_fsutil_rw_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("data.json");
        atomic_write(&p, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        // overwrite
        atomic_write(&p, b"world").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"world");
        // no leftover temps
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("."))
                    .unwrap_or(false)
            })
            .collect();
        assert!(temps.is_empty(), "no orphaned temp files: {temps:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn atomic_write_secure_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("catalyst_code_fsutil_sec_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("secret.json");
        atomic_write_secure(&p, b"{\"key\":\"x\"}").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "secure write must be 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn file_lock_serializes_concurrent_writers() {
        // Two threads doing a read-modify-write under the SAME lock must not
        // lose updates: the lock serializes them so both increments land.
        let dir =
            std::env::temp_dir().join(format!("catalyst_code_fsutil_lock_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let data = dir.join("counter.json");
        let lock = dir.join("counter.lock");
        atomic_write(&data, b"0").unwrap();

        let n = 8usize;
        let mut handles = Vec::new();
        for _ in 0..n {
            let data = data.clone();
            let lock = lock.clone();
            handles.push(std::thread::spawn(move || {
                let _g = FileLock::acquire(&lock).unwrap();
                let cur: u64 = std::fs::read_to_string(&data)
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap_or(0);
                atomic_write(&data, format!("{}", cur + 1).as_bytes()).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Without the lock this would race and lose increments; with it, all 8
        // land and the counter is exactly n.
        let final_val: u64 = std::fs::read_to_string(&data)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            final_val, n as u64,
            "lock must serialize RMW: got {final_val}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
