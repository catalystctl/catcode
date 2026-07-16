// Cross-session workspace presence: each core process publishes a small
// "I'm here and doing X" record so other sessions in the SAME workspace can
// detect concurrent activity — instead of blaming themselves for phantom errors
// caused by a neighbor's in-flight edits.
//
// Per-pid JSON files under ~/.config/catalyst-code/presence/<hash(cwd)>/<pid>.json.
// Per-pid (not one shared file) → zero write contention; each process owns one
// file. Writes are atomic (temp + fsync + rename) — the same crash-safety
// pattern as session/memory persistence. Stale records (a crashed/killed
// process that stopped rewriting its file) are reaped by mtime on read, so a
// `kill -9` is tolerated: the next reader deletes the dead file.
//
// This is AWARENESS ONLY — read-only broadcast of "who is here and what are
// they touching." It deliberately does NOT coordinate (no locking, no
// work-claiming): partial coordination is more dangerous than none, and 80% of
// the value is an agent *knowing* a neighbor is active so it stops "fixing"
// phantom errors and corrupting in-flight work. See the `workspace_activity`
// tool and the `maybe_concurrency_note` anomaly nudge in main.rs.
use crate::config::home_dir;
use crate::memory::project_hash;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A record older than this (by file mtime) is considered stale/dead and is
/// reaped on read. The heartbeat rewrites the file every ~8s, so a live process
/// is never within 30s of this threshold — `kill -9` / a crashed core leaves a
/// file the next reader deletes.
const STALE_SECS: u64 = 30;

/// One session's published presence. Serialized pretty-printed to disk so a
/// human can eyeball `<pid>.json`; small files.
#[derive(Clone, Serialize, Deserialize)]
pub struct PresenceRecord {
    pub pid: u32,
    pub session_id: Option<String>,
    /// Unix seconds — when this session started. For "started Xm ago".
    pub started_at: u64,
    /// Unix seconds — rewritten every heartbeat. For human readability; the
    /// reap decision uses file mtime (the reliable signal), not this field.
    pub last_heartbeat: u64,
    pub goal: String,
    pub in_progress: Vec<String>,
    pub next: Vec<String>,
    pub recent_files: Vec<String>,
    pub last_activity: String,
    pub model: Option<String>,
}

impl PresenceRecord {
    /// Build from the session's rolling work-state + identifying context.
    pub fn from_work_state(
        ws: &crate::WorkState,
        pid: u32,
        session_id: Option<String>,
        model: Option<String>,
        started_at: u64,
    ) -> Self {
        Self {
            pid,
            session_id,
            started_at,
            last_heartbeat: unix_now(),
            goal: ws.goal.clone(),
            in_progress: ws.in_progress.clone(),
            next: ws.next.clone(),
            recent_files: ws.recent_files.clone(),
            last_activity: ws.last_activity.clone(),
            model,
        }
    }
}

/// The per-workspace presence directory: ~/.config/catalyst-code/presence/<hash>/.
/// Returns None if the home dir can't be determined (presence disabled).
pub fn presence_dir(workspace: &Path) -> Option<PathBuf> {
    let home = home_dir()?;
    Some(
        home.join(".config/catalyst-code/presence")
            .join(project_hash(&workspace.to_string_lossy())),
    )
}

/// The per-process presence file: <dir>/<pid>.json.
pub fn presence_file(workspace: &Path, pid: u32) -> Option<PathBuf> {
    Some(presence_dir(workspace)?.join(format!("{pid}.json")))
}

/// Atomically write (or overwrite) our presence record. Best-effort — presence
/// is advisory, so a write failure is logged to stderr, never fatal.
pub fn write_presence(workspace: &Path, pid: u32, rec: &PresenceRecord) {
    let Some(file) = presence_file(workspace, pid) else {
        return;
    };
    if let Err(e) = atomic_write_json(&file, rec) {
        eprintln!("[presence] failed to write {}: {e}", file.display());
    }
}

/// Delete our presence file on clean shutdown. Best-effort; stale-reaping on
/// read is the real correctness net (covers `kill -9` / crash).
pub fn clear_presence(workspace: &Path, pid: u32) {
    if let Some(file) = presence_file(workspace, pid) {
        let _ = std::fs::remove_file(file);
    }
}

/// Read all LIVE peer records for this workspace (excluding our own pid).
/// Stale records (mtime older than STALE_SECS) are reaped (deleted) and
/// skipped. Unparseable files are skipped (mtime will reap them later).
pub fn read_peers(workspace: &Path, my_pid: u32) -> Vec<PresenceRecord> {
    let Some(dir) = presence_dir(workspace) else {
        return Vec::new();
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(), // dir doesn't exist yet → no peers
    };
    let now = unix_now();
    let mut peers = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // Reap by mtime: a process that hasn't rewritten its file in STALE_SECS
        // is dead. mtime is the reliable signal (the heartbeat updates it every
        // ~8s); last_heartbeat is for human readability only.
        let mtime_secs = match entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            Some(d) => d.as_secs(),
            None => continue,
        };
        if now.saturating_sub(mtime_secs) > STALE_SECS {
            let _ = std::fs::remove_file(&path); // reap stale
            continue;
        }
        let rec = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<PresenceRecord>(&s).ok())
        {
            Some(r) => r,
            None => continue, // unparseable — leave it; mtime will reap
        };
        if rec.pid == my_pid {
            continue; // skip self
        }
        peers.push(rec);
    }
    peers
}

/// Current unix timestamp in seconds (0 on clock error).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Atomic write: temp file in the same dir + fsync + rename. Same crash-safety
/// pattern as session/memory persistence. A crash mid-write leaves the temp
/// (hidden, `.<name>.tmp`) orphaned, never a truncated record.
fn atomic_write_json(path: &Path, rec: &PresenceRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fname = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("presence.json");
    let tmp = path.with_file_name(format!(".{fname}.tmp"));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        let body = serde_json::to_vec_pretty(rec).map_err(std::io::Error::other)?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkState;

    fn ws_dir() -> std::path::PathBuf {
        // A unique temp workspace so parallel tests don't collide.
        let dir = std::env::temp_dir().join(format!(
            "catalyst-presence-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_then_read_peer() {
        let dir = ws_dir();
        let ws = WorkState {
            goal: "refactor auth".into(),
            in_progress: vec!["core/src/main.rs".into()],
            ..Default::default()
        };
        let rec =
            PresenceRecord::from_work_state(&ws, 4242, Some("s.json".into()), None, unix_now());
        write_presence(&dir, 4242, &rec);

        // Self is excluded.
        assert!(read_peers(&dir, 4242).is_empty());

        // Another pid sees it.
        let peers = read_peers(&dir, 9999);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].pid, 4242);
        assert_eq!(peers[0].goal, "refactor auth");
        assert_eq!(peers[0].recent_files, Vec::<String>::new());
        assert_eq!(peers[0].in_progress, vec!["core/src/main.rs".to_string()]);

        // Cleanup clears our own file but leaves peers.
        clear_presence(&dir, 4242);
        assert!(read_peers(&dir, 9999).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_record_is_reaped() {
        let dir = ws_dir();
        // Write a record, then backdate its mtime beyond STALE_SECS.
        let rec =
            PresenceRecord::from_work_state(&WorkState::default(), 5555, None, None, unix_now());
        write_presence(&dir, 5555, &rec);
        let file = presence_file(&dir, 5555).unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(STALE_SECS + 60);
        let _ = filetime::set_file_mtime(&file, filetime::FileTime::from_system_time(past));

        // Stale → reaped (file deleted), not returned.
        assert!(read_peers(&dir, 9999).is_empty());
        assert!(!file.exists(), "stale presence file should be reaped");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unix_now_is_reasonable() {
        let t = unix_now();
        assert!(t > 1_700_000_000, "unix_now must be post-2023: {t}");
    }

    #[test]
    fn presence_dir_returns_some_for_temp_dir() {
        let dir = std::env::temp_dir();
        let pdir = presence_dir(&dir);
        assert!(
            pdir.is_some(),
            "presence_dir should succeed for a valid path"
        );
        assert!(pdir.unwrap().to_string_lossy().contains("presence"));
    }

    #[test]
    fn presence_file_uses_pid_in_name() {
        let dir = std::env::temp_dir();
        let f = presence_file(&dir, 42);
        assert!(f.is_some());
        let name = f
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(name, "42.json");
    }

    #[test]
    fn clear_presence_for_nonexistent_file_does_not_panic() {
        // PID that has never written → clear should be a no-op, not panic.
        let dir = std::env::temp_dir().join(format!("catcode-pres-nonex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        clear_presence(&dir, 99999);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_peers_empty_dir() {
        let dir = std::env::temp_dir().join(format!("catcode-pres-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let peers = read_peers(&dir, 1);
        assert!(peers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
