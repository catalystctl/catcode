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
pub const SESSION_VERSION: u32 = 1;

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
}

/// Append one message to the session file (creating it with a header if needed).
/// fsync'd so a crash never truncates a finalized message mid-write.
pub fn append(path: &Path, msg: &Message) {
    ensure_header(path);
    let Ok(mut f) = OpenOptions::new().append(true).open(path) else {
        return;
    };
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let _ = f.write_all(line.as_bytes());
    let _ = f.flush();
    let _ = f.sync_all(); // crash durability for finalized turns
}

/// Load all messages from a session file. Skips the version header and any
/// unparseable lines. Returns `Ok(Vec)` for a missing file (nothing to resume)
/// or a current-version file. Returns `Err(human_message)` when the file's
/// header version is NEWER than `SESSION_VERSION` — refusing to silently
/// misread/drop a session on upgrade (the caller surfaces the error to the
/// user instead of quietly starting blank).
pub fn load(path: &Path) -> Result<Vec<Message>, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(Vec::new()); // missing file: nothing to resume (not an error)
    };
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    // First non-empty line must be the version header. If it's absent or a
    // future version, bail with a clear error rather than guess.
    let first = lines.next().unwrap_or("");
    if let Ok(v) = serde_json::from_str::<Value>(first) {
        if let Some(ver) = v.get("_session_version").and_then(|x| x.as_u64()) {
            if ver as u32 > SESSION_VERSION {
                return Err(format!(
                    "session file {} is version {ver}, newer than supported ({SESSION_VERSION}); not loaded to avoid corrupting it. Delete the file (or migrate it) to continue.",
                    path.display()
                ));
            }
            // header consumed; load the rest
        } else {
            // no header on an old file — treat the first line as a real message
            let mut out = Vec::new();
            if let Ok(m) = serde_json::from_str::<Message>(first) {
                out.push(m);
            }
            for l in lines {
                if let Ok(m) = serde_json::from_str::<Message>(l) {
                    out.push(m);
                }
            }
            return Ok(out);
        }
    }
    Ok(lines
        .filter_map(|l| serde_json::from_str::<Message>(l).ok())
        .collect())
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

/// Persist the current set of escalated approval kinds atomically (temp +
/// fsync + rename) so a crash never truncates it.
pub fn save_escalations(session_path: &Path, kinds: &std::collections::HashSet<String>) {
    let p = escalations_path(session_path);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = p.with_extension("tmp");
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
    let tmp = p.with_extension("tmp");
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
    let tmp = path.with_extension("tmp");
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
        let dir = std::env::temp_dir().join("umans_harness_session_test");
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
    fn header_version_is_present_and_validated() {
        let dir = std::env::temp_dir().join("umans_harness_session_hdr_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        append(&p, &Message::user("hi"));
        // header line present and well-formed
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.starts_with("{\"_session_version\": 1}"));
        // load() drops the header, returns only real messages
        assert_eq!(load(&p).unwrap().len(), 1);
    }

    #[test]
    fn future_version_refused() {
        let dir = std::env::temp_dir().join("umans_harness_session_future");
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
    fn describe_extracts_first_user_title_and_count() {
        let dir = std::env::temp_dir().join("umans_harness_session_describe_test");
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
        let dir = std::env::temp_dir().join("umans_harness_session_describe_mm");
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
        let dir = std::env::temp_dir().join("umans_harness_session_describe_long");
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
        let dir = std::env::temp_dir().join("umans_harness_session_stats_test");
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
            },
        );
        // “Reopen” → the sidecar restores the same totals.
        let s = load_stats(&p);
        assert_eq!(s.tokens_in, 12345);
        assert_eq!(s.tokens_out, 678);
        assert_eq!(s.cached_tokens, 9000);
        assert_eq!(s.turns, 7);
        // Garbage in the sidecar degrades to zeros (never panics).
        std::fs::write(stats_path(&p), "not json").unwrap();
        let s = load_stats(&p);
        assert_eq!(
            (s.tokens_in, s.tokens_out, s.cached_tokens, s.turns),
            (0, 0, 0, 0)
        );
    }
}
