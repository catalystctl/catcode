// Session persistence: append-only JSONL of conversation messages, prefixed
// by a schema-version header line so future shape changes can migrate old
// files instead of silently misreading them. On init, if the session file
// exists it's loaded and replayed; each finalized message is appended (and
// fsync'd) so a crash mid-task loses at most the in-flight turn.
use crate::message::Message;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Bump when the on-disk message shape changes. load() validates the header.
pub const SESSION_VERSION: u32 = 2;

fn header_line() -> String {
    format!("{{\"_session_version\": {}}}", SESSION_VERSION)
}

fn ensure_header(path: &Path) {
    // Create the file with a header if it doesn't exist yet.
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", header_line());
        let _ = f.flush();
        let _ = f.sync_all();
    }
}

/// Create the session file with just its version header if it doesn't already
/// exist. Used so a new/active session shows up in `list_sessions`
/// immediately, even before the first message is appended.
pub fn ensure(path: &Path) {
    ensure_header(path);
    migrate_header(path);
}

fn migrate_header(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Some((first, rest)) = content.split_once('\n') else {
        return;
    };
    let Ok(header) = serde_json::from_str::<Value>(first) else {
        return;
    };
    let Some(version) = header.get("_session_version").and_then(Value::as_u64) else {
        return;
    };
    if version as u32 >= SESSION_VERSION {
        return;
    }
    let migrated = format!("{}\n{}", header_line(), rest);
    let _ = crate::fsutil::atomic_write_str(path, &migrated);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Started,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RunRecord {
    pub session_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub state: RunState,
    pub timestamp_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub messages: Vec<Message>,
    pub warnings: Vec<String>,
    pub unfinished_runs: Vec<RunRecord>,
}

pub fn append_run_state(
    path: &Path,
    session_id: &str,
    run_id: &str,
    state: RunState,
    detail: Option<&str>,
) {
    append_activity_state(path, session_id, run_id, "run", None, None, state, detail);
}

/// Append a lifecycle record for foreground or child activity. New optional
/// identity fields are backward-compatible with v2 journals and let recovery
/// distinguish an interrupted tool, subagent, or goal without ever restarting it.
#[allow(clippy::too_many_arguments)]
pub fn append_activity_state(
    path: &Path,
    session_id: &str,
    run_id: &str,
    kind: &str,
    parent_run_id: Option<&str>,
    tool_call_id: Option<&str>,
    state: RunState,
    detail: Option<&str>,
) {
    ensure_header(path);
    ensure_record_boundary(path);
    let record = RunRecord {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        kind: Some(kind.to_string()),
        parent_run_id: parent_run_id.map(str::to_string),
        tool_call_id: tool_call_id.map(str::to_string),
        state,
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        detail: detail.map(str::to_string),
    };
    let Ok(mut file) = OpenOptions::new().append(true).open(path) else {
        return;
    };
    let line = serde_json::json!({"_run": record});
    let _ = writeln!(file, "{line}");
    let _ = file.flush();
}

/// Append one message to the session file (creating it with a header if needed).
/// Flushed to the kernel but not `fsync`'d — durability is deferred to
/// [`sync`] at turn end so multi-message rounds aren't serialized behind a
/// disk sync per tool result. Crash window: last incomplete turn.
pub fn append(path: &Path, msg: &Message) {
    ensure_header(path);
    ensure_record_boundary(path);
    let Ok(mut f) = OpenOptions::new().append(true).open(path) else {
        return;
    };
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let _ = f.write_all(line.as_bytes());
    let _ = f.flush();
}

fn ensure_record_boundary(path: &Path) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = OpenOptions::new().read(true).append(true).open(path) else {
        return;
    };
    let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
        return;
    };
    if length == 0 || file.seek(SeekFrom::End(-1)).is_err() {
        return;
    }
    let mut last = [0_u8; 1];
    if file.read_exact(&mut last).is_ok() && last[0] != b'\n' {
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

/// fsync the session file so finalized turns survive a crash. Call at turn
/// end (and on abort paths that have already appended results).
pub fn sync(path: &Path) {
    if let Ok(f) = OpenOptions::new().append(true).open(path) {
        let _ = f.sync_all();
    }
    if let Some(parent) = path.parent() {
        fsync_dir(parent);
    }
}

/// Load all messages from a session file. Skips the version header and any
/// unparseable lines. Returns `Ok(Vec)` for a missing file (nothing to resume)
/// or a current-version file. Returns `Err(human_message)` when the file's
/// header version is NEWER than `SESSION_VERSION` — refusing to silently
/// misread/drop a session on upgrade (the caller surfaces the error to the
/// user instead of quietly starting blank).
pub fn load(path: &Path) -> Result<Vec<Message>, String> {
    load_report(path).map(|report| report.messages)
}

/// Load readable conversation records and return explicit recovery details.
/// Malformed records do not erase valid history; an incomplete final line is
/// treated as a crash-truncated append, while malformed interior lines are
/// counted separately. Started runs without a terminal record are returned so
/// startup can persist an `interrupted` terminal state without rerunning work.
pub fn load_report(path: &Path) -> Result<LoadReport, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(LoadReport::default());
    };
    let nonempty: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let mut lines = nonempty.iter();
    // First non-empty line must be the version header. If it's absent or a
    // future version, bail with a clear error rather than guess.
    let first = lines.next().map(|(_, line)| *line).unwrap_or("");
    let mut report = LoadReport::default();
    let mut run_states = std::collections::HashMap::<String, RunRecord>::new();
    let mut first_is_message = false;
    if let Ok(v) = serde_json::from_str::<Value>(first) {
        if let Some(ver) = v.get("_session_version").and_then(|x| x.as_u64()) {
            if ver as u32 > SESSION_VERSION {
                return Err(format!(
                    "session file {} is version {ver}, newer than supported ({SESSION_VERSION}); not loaded to avoid corrupting it. Delete the file (or migrate it) to continue.",
                    path.display()
                ));
            }
            if (ver as u32) < SESSION_VERSION {
                report.warnings.push(format!(
                    "session schema v{ver} loaded through compatibility mode; it will migrate to v{SESSION_VERSION} before the next append"
                ));
            }
        } else {
            first_is_message = true;
            report.warnings.push(
                "legacy session without a version header loaded in compatibility mode".into(),
            );
        }
    } else if !first.is_empty() {
        first_is_message = true;
    }

    let records: Vec<(usize, &str)> = if first_is_message {
        nonempty.clone()
    } else {
        nonempty.into_iter().skip(1).collect()
    };
    let last_line = records.last().map(|(line, _)| *line);
    let final_line_terminated = content.ends_with('\n');
    let mut malformed = Vec::new();
    for (line_number, line) in records {
        let line_number = line_number + 1;
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) => {
                if Some(line_number - 1) == last_line && !final_line_terminated {
                    report.warnings.push(format!(
                        "recovered session after ignoring a truncated final record at line {line_number}"
                    ));
                } else {
                    malformed.push(line_number);
                }
                continue;
            }
        };
        if let Some(run) = value.get("_run") {
            match serde_json::from_value::<RunRecord>(run.clone()) {
                Ok(record) => {
                    run_states.insert(record.run_id.clone(), record);
                }
                Err(_) => malformed.push(line_number),
            }
            continue;
        }
        match serde_json::from_value::<Message>(value) {
            Ok(message) => report.messages.push(message),
            Err(_) => malformed.push(line_number),
        }
    }
    if !malformed.is_empty() {
        report.warnings.push(format!(
            "ignored {} malformed session record(s) at line(s) {}",
            malformed.len(),
            malformed
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    report.unfinished_runs = run_states
        .into_values()
        .filter(|record| record.state == RunState::Started)
        .collect();
    Ok(report)
}

/// Sidecar path for per-session "always" approval escalations (tool kinds the
/// user said "always" to). Stored beside the session file so it travels with
/// the project and survives restart — previously these were in-memory only,
/// so a restart silently un-gated kinds the user had approved.
fn escalations_path(session_path: &Path) -> PathBuf {
    let mut p = session_path.as_os_str().to_os_string();
    p.push(".escalations");
    PathBuf::from(p)
}

/// Load persisted escalated approval kinds (empty set if absent/unreadable).
pub fn load_escalations(session_path: &Path) -> std::collections::HashSet<String> {
    let p = escalations_path(session_path);
    let Ok(content) = std::fs::read_to_string(&p) else {
        return std::collections::HashSet::new();
    };
    serde_json::from_str::<Vec<String>>(&content)
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

/// Best-effort directory fsync after an atomic rename. POSIX does not guarantee
/// a rename survives a power-loss crash unless the parent directory is also
/// fsync'd, so after each temp→target rename we fsync the parent dir. Ignored on
/// platforms where a directory cannot be opened as a file (Windows) — `File::open`
/// on a directory simply fails there and the `if let Ok` skips it.
fn fsync_dir(path: &Path) {
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.sync_all();
    }
}

/// Persist the current set of escalated approval kinds atomically (temp +
/// fsync + rename) so a crash never truncates it.
pub fn save_escalations(session_path: &Path, kinds: &std::collections::HashSet<String>) {
    let p = escalations_path(session_path);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = crate::fsutil::unique_tmp(&p);
    let Ok(mut f) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
    else {
        return;
    };
    let list: Vec<&String> = kinds.iter().collect();
    let _ = writeln!(f, "{}", serde_json::to_string(&list).unwrap_or_default());
    let _ = f.flush();
    let _ = f.sync_all();
    drop(f); // release before rename (Windows)
    let _ = std::fs::rename(&tmp, &p);
    if let Some(parent) = p.parent() {
        fsync_dir(parent);
    }
}

/// Cumulative session stats persisted beside the session file (sidecar
/// `<session>.stats`) so `/stats` survives a restart — previously these were
/// in-memory only, so reopening the harness showed zeros for tokens/turns.
#[derive(Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SessionStats {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cached_tokens: u64,
    pub turns: u64,
    #[serde(default)]
    pub compactions: u64,
}

fn stats_path(session_path: &Path) -> PathBuf {
    let mut p = session_path.as_os_str().to_os_string();
    p.push(".stats");
    PathBuf::from(p)
}

/// Load persisted cumulative stats (all-zero if absent/unreadable).
pub fn load_stats(session_path: &Path) -> SessionStats {
    let p = stats_path(session_path);
    let Ok(content) = std::fs::read_to_string(&p) else {
        return SessionStats::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist cumulative stats atomically (temp + fsync + rename) so a crash never
/// truncates them — same durability story as `save_escalations`.
pub fn save_stats(session_path: &Path, stats: &SessionStats) {
    let p = stats_path(session_path);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = crate::fsutil::unique_tmp(&p);
    let Ok(mut f) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
    else {
        return;
    };
    let _ = writeln!(f, "{}", serde_json::to_string(stats).unwrap_or_default());
    let _ = f.flush();
    let _ = f.sync_all();
    drop(f); // release before rename (Windows)
    let _ = std::fs::rename(&tmp, &p);
    if let Some(parent) = p.parent() {
        fsync_dir(parent);
    }
}

/// Truncate/replace the whole session file with `messages` (used on reset /
/// compaction), re-writing the version header first. Atomic: writes a sibling
/// temp file, fsyncs it, then renames it over the target, so a crash mid-
/// rewrite never truncates the existing conversation — the old file stays intact
/// until the rename lands (P1-3: the old truncate-then-write lost everything on a
/// crash between truncate and final sync).
pub fn rewrite(path: &Path, messages: &[Message]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = crate::fsutil::unique_tmp(path);
    let Ok(mut f) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
    else {
        return;
    };
    let _ = writeln!(f, "{}", header_line());
    for m in messages {
        let mut line = serde_json::to_string(m).unwrap_or_default();
        line.push('\n');
        let _ = f.write_all(line.as_bytes());
    }
    let _ = f.flush();
    let _ = f.sync_all();
    drop(f); // release the handle before rename (Windows requires it)
             // Atomic on POSIX (same dir/same volume); best-effort on Windows.
    let _ = std::fs::rename(&tmp, path);
    if let Some(parent) = path.parent() {
        fsync_dir(parent);
    }
}

/// A lightweight description of a session file used by the session picker.
/// `title` is derived from the first user message so a session is identifiable
/// by its topic instead of by an opaque hex-hash filename. Because it is read
/// fresh from the append-only file each time `list_sessions` runs, it updates
/// automatically as the conversation grows (empty → first prompt → fuller).
pub struct SessionInfo {
    pub title: Option<String>,
    pub messages: usize,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

fn meta_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.meta.json", path.display()))
}

fn process_lock_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lock", path.display()))
}

fn meta_lock_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.meta.lock", path.display()))
}

pub fn read_meta(path: &Path) -> SessionMeta {
    std::fs::read(meta_path(path))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn update_meta(
    path: &Path,
    update: impl FnOnce(&mut SessionMeta),
) -> Result<SessionMeta, String> {
    let _lock =
        crate::fsutil::FileLock::acquire(&meta_lock_path(path)).map_err(|e| e.to_string())?;
    let mut meta = read_meta(path);
    update(&mut meta);
    let data = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    crate::fsutil::atomic_write(&meta_path(path), &data).map_err(|e| e.to_string())?;
    Ok(meta)
}

pub fn delete_with_sidecars(path: &Path) -> Result<(), String> {
    if process_lock_path(path).exists() {
        return Err("session is active in another process".into());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(meta_path(path));
    let _ = std::fs::remove_file(stats_path(path));
    let _ = std::fs::remove_file(meta_lock_path(path));
    Ok(())
}

/// Scan a session file once (streaming, bounded) and return its title + message
/// count. The title is the first user message's text, truncated to 80 chars.
/// Returns `title: None, messages: 0` for a missing or header-only file.
pub fn describe(path: &Path) -> SessionInfo {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return SessionInfo {
                title: None,
                messages: 0,
            }
        }
    };
    let reader = BufReader::new(file);
    let mut title: Option<String> = None;
    let mut messages = 0usize;
    let mut header_consumed = false;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        if !header_consumed {
            header_consumed = true;
            // First non-empty line: if it's the version header, skip it (mirrors load()).
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("_session_version").is_some() {
                    continue;
                }
            }
            // Old file with no header — this line is a real message; fall through.
        }
        messages += 1;
        // Sanity guard against a pathological file; stop counting beyond this.
        if messages > 100_000 {
            break;
        }
        if title.is_none() {
            if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                if let Some(t) = first_user_text(&msg) {
                    let t: String = t.trim().chars().take(80).collect();
                    title = Some(t);
                }
            }
        }
    }
    SessionInfo { title, messages }
}

/// Extract the text of a user message: plain string content, or the joined
/// text parts of a multimodal content array. Returns None for non-user or
/// empty messages (so tool/assistant/system messages never become a title).
fn first_user_text(msg: &Message) -> Option<String> {
    if !msg.is_user() {
        return None;
    }
    if let Some(s) = msg.content_text() {
        if s.trim().is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if let Some(parts) = msg.content_parts() {
        let mut out = String::new();
        for p in parts {
            if let crate::message::ContentPart::Text { text } = p {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
        if out.trim().is_empty() {
            return None;
        }
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentPart, ImageUrl};

    #[test]
    fn append_then_load_roundtrip() {
        let dir = std::env::temp_dir().join("catalyst_code_session_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::system("x"));
        append(&p, &Message::user("hi"));
        let v = load(&p).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].role(), "system");
        // rewrite
        rewrite(&p, &[Message::system("y")]);
        assert_eq!(load(&p).unwrap().len(), 1);
    }

    #[test]
    fn append_flush_then_sync_persists() {
        let dir = std::env::temp_dir().join("catalyst_code_session_sync_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("one"));
        append(&p, &Message::user("two"));
        // Durability is deferred to sync — content must already be readable
        // after flush-only appends, and sync must be a no-op success.
        assert_eq!(load(&p).unwrap().len(), 2);
        sync(&p);
        assert_eq!(load(&p).unwrap().len(), 2);
    }

    #[test]
    fn header_version_is_present_and_validated() {
        let dir = std::env::temp_dir().join("catalyst_code_session_hdr_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("hi"));
        // header line present and well-formed
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.starts_with("{\"_session_version\": 2}"));
        // load() drops the header, returns only real messages
        assert_eq!(load(&p).unwrap().len(), 1);
    }

    #[test]
    fn future_version_refused() {
        let dir = std::env::temp_dir().join("catalyst_code_session_future");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(
            &p,
            "{\"_session_version\": 99}\n{\"role\":\"user\",\"content\":\"x\"}\n",
        )
        .unwrap();
        // refuses to load a newer-version file and returns a clear error
        let r = load(&p);
        assert!(r.is_err(), "expected an error for a future-version session");
        assert!(r.unwrap_err().contains("newer than supported"));
    }

    #[test]
    fn legacy_header_migrates_without_losing_messages() {
        let dir = std::env::temp_dir().join("catalyst_code_session_migration");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(
            &p,
            "{\"_session_version\": 1}\n{\"role\":\"user\",\"content\":\"old\"}\n",
        )
        .unwrap();
        let report = load_report(&p).unwrap();
        assert_eq!(report.messages.len(), 1);
        assert!(!report.warnings.is_empty());
        ensure(&p);
        assert!(std::fs::read_to_string(&p)
            .unwrap()
            .starts_with("{\"_session_version\": 2}"));
        assert_eq!(load(&p).unwrap().len(), 1);
    }

    #[test]
    fn recovery_reports_truncation_malformed_records_and_unfinished_runs() {
        let dir = std::env::temp_dir().join("catalyst_code_session_recovery");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        std::fs::write(
            &p,
            concat!(
                "{\"_session_version\": 2}\n",
                "{\"role\":\"user\",\"content\":\"survives\"}\n",
                "not-json\n",
                "{\"_run\":{\"session_id\":\"s1\",\"run_id\":\"r1\",\"state\":\"started\",\"timestamp_ms\":1}}\n",
                "{\"role\":\"assistant\",\"content\":"
            ),
        )
        .unwrap();
        let report = load_report(&p).unwrap();
        assert_eq!(report.messages.len(), 1);
        assert_eq!(report.unfinished_runs.len(), 1);
        assert_eq!(report.unfinished_runs[0].run_id, "r1");
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("malformed")));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("truncated")));

        append_run_state(&p, "s1", "r1", RunState::Interrupted, Some("restart"));
        assert!(load_report(&p).unwrap().unfinished_runs.is_empty());
    }

    #[test]
    fn completed_run_is_not_reported_as_unfinished() {
        let dir = std::env::temp_dir().join("catalyst_code_session_run_terminal");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        ensure(&p);
        append_run_state(&p, "s1", "r1", RunState::Started, None);
        append_run_state(&p, "s1", "r1", RunState::Completed, None);
        assert!(load_report(&p).unwrap().unfinished_runs.is_empty());
    }

    #[test]
    fn recovery_identifies_interrupted_tool_subagent_and_goal_without_resuming() {
        let dir = std::env::temp_dir().join("catalyst_code_session_activity_recovery");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        ensure(&p);
        append_activity_state(
            &p,
            "s1",
            "tool-run",
            "tool",
            Some("parent-run"),
            Some("call-7"),
            RunState::Started,
            Some("bash"),
        );
        append_activity_state(
            &p,
            "s1",
            "sub-run",
            "subagent",
            Some("parent-run"),
            None,
            RunState::Started,
            Some("worker"),
        );
        append_activity_state(
            &p,
            "s1",
            "goal-1",
            "goal",
            None,
            None,
            RunState::Started,
            Some("deploying"),
        );

        let report = load_report(&p).unwrap();
        assert!(
            report.messages.is_empty(),
            "recovery must not synthesize replay work"
        );
        assert_eq!(report.unfinished_runs.len(), 3);
        let by_kind: std::collections::HashMap<_, _> = report
            .unfinished_runs
            .iter()
            .map(|record| (record.kind.as_deref().unwrap(), record))
            .collect();
        assert_eq!(by_kind["tool"].tool_call_id.as_deref(), Some("call-7"));
        assert_eq!(
            by_kind["subagent"].parent_run_id.as_deref(),
            Some("parent-run")
        );
        assert_eq!(by_kind["goal"].run_id, "goal-1");

        for record in report.unfinished_runs {
            append_activity_state(
                &p,
                &record.session_id,
                &record.run_id,
                record.kind.as_deref().unwrap_or("run"),
                record.parent_run_id.as_deref(),
                record.tool_call_id.as_deref(),
                RunState::Interrupted,
                Some("recovered after restart; not resumed"),
            );
        }
        assert!(load_report(&p).unwrap().unfinished_runs.is_empty());
    }

    #[test]
    fn normal_replay_preserves_messages_and_terminal_activity() {
        let dir = std::env::temp_dir().join("catalyst_code_session_normal_replay");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("hello"));
        append(&p, &Message::assistant("done"));
        append_run_state(&p, "s1", "r1", RunState::Started, None);
        append_run_state(&p, "s1", "r1", RunState::Completed, None);
        let report = load_report(&p).unwrap();
        assert_eq!(report.messages.len(), 2);
        assert!(report.unfinished_runs.is_empty());
    }

    #[test]
    fn describe_extracts_first_user_title_and_count() {
        let dir = std::env::temp_dir().join("catalyst_code_session_describe_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::system("sys"));
        append(&p, &Message::user("Add a login form to the app"));
        append(&p, &Message::assistant("done"));
        append(&p, &Message::user("now add tests"));
        let info = describe(&p);
        assert_eq!(info.messages, 4);
        // Title is the FIRST user message (the stable topic), not the latest.
        assert_eq!(info.title.as_deref(), Some("Add a login form to the app"));
    }

    #[test]
    fn describe_header_only_and_multimodal() {
        let dir = std::env::temp_dir().join("catalyst_code_session_describe_mm");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        // header-only file: no messages, no title
        let info = describe(&p);
        assert_eq!(info.messages, 0);
        assert!(info.title.is_none());
        // a multimodal user message: title is the joined text parts
        append(
            &p,
            &Message::user_multimodal(vec![
                ContentPart::Text {
                    text: "describe this".into(),
                },
                ContentPart::Image {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAAA".into(),
                        detail: None,
                    },
                },
            ]),
        );
        let info = describe(&p);
        assert_eq!(info.messages, 1);
        assert_eq!(info.title.as_deref(), Some("describe this"));
    }

    #[test]
    fn describe_truncates_long_titles() {
        let dir = std::env::temp_dir().join("catalyst_code_session_describe_long");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        let long = "x".repeat(200);
        append(&p, &Message::user(long));
        let info = describe(&p);
        assert_eq!(info.title.as_deref().map(|s| s.len()), Some(80));
    }

    #[test]
    fn stats_roundtrip_survives_restart() {
        let dir = std::env::temp_dir().join("catalyst_code_session_stats_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        // No sidecar yet → all zeros.
        let s = load_stats(&p);
        assert_eq!(
            (s.tokens_in, s.tokens_out, s.cached_tokens, s.turns),
            (0, 0, 0, 0)
        );
        // Persist some cumulative usage.
        save_stats(
            &p,
            &SessionStats {
                tokens_in: 12345,
                tokens_out: 678,
                cached_tokens: 9000,
                turns: 7,
                compactions: 3,
            },
        );
        // “Reopen” → the sidecar restores the same totals.
        let s = load_stats(&p);
        assert_eq!(s.tokens_in, 12345);
        assert_eq!(s.tokens_out, 678);
        assert_eq!(s.cached_tokens, 9000);
        assert_eq!(s.turns, 7);
        assert_eq!(s.compactions, 3);
        // Garbage in the sidecar degrades to zeros (never panics).
        std::fs::write(stats_path(&p), "not json").unwrap();
        let s = load_stats(&p);
        assert_eq!(
            (s.tokens_in, s.tokens_out, s.cached_tokens, s.turns),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn metadata_roundtrip_and_delete_removes_sidecars() {
        let dir = std::env::temp_dir().join("catalyst_code_session_meta_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("hello"));
        save_stats(
            &p,
            &SessionStats {
                turns: 1,
                ..SessionStats::default()
            },
        );

        update_meta(&p, |meta| {
            meta.title = Some("Renamed conversation".into());
            meta.pinned = true;
        })
        .unwrap();
        let meta = read_meta(&p);
        assert_eq!(meta.title.as_deref(), Some("Renamed conversation"));
        assert!(meta.pinned);

        delete_with_sidecars(&p).unwrap();
        assert!(!p.exists());
        assert!(!meta_path(&p).exists());
        assert!(!stats_path(&p).exists());
    }

    #[test]
    fn delete_refuses_a_session_locked_by_another_process() {
        let dir = std::env::temp_dir().join("catalyst_code_session_locked_delete_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("still active"));
        std::fs::write(process_lock_path(&p), "pid=123").unwrap();

        assert!(delete_with_sidecars(&p).is_err());
        assert!(p.exists());
        std::fs::remove_file(process_lock_path(&p)).unwrap();
        delete_with_sidecars(&p).unwrap();
        assert!(!p.exists());
    }
}
