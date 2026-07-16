//! Optional append-only security audit sidecar.
//!
//! Records tool decisions (approve/deny/allow_pattern), args hashes, and
//! optional diff hashes. Gated by `Config.audit_log`.

use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn audit_path(session_file: Option<&Path>, workspace: &Path) -> PathBuf {
    if let Some(p) = session_file {
        let mut s = p.to_path_buf();
        s.set_extension("audit.jsonl");
        return s;
    }
    workspace.join(".catalyst-code").join("audit.jsonl")
}

pub fn args_hash(args: &str) -> String {
    let mut h = Sha256::new();
    h.update(args.as_bytes());
    format!("{:x}", h.finalize())
}

pub fn record(
    enabled: bool,
    session_file: Option<&Path>,
    workspace: &Path,
    tool: &str,
    args: &str,
    decision: &str,
    actor: &str,
    agent_id: Option<&str>,
    diff: Option<&str>,
) {
    if !enabled {
        return;
    }
    let path = audit_path(session_file, workspace);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let diff_hash = diff.map(|d| {
        let mut h = Sha256::new();
        h.update(d.as_bytes());
        format!("{:x}", h.finalize())
    });
    let line = json!({
        "ts": crate::logging::now_iso(),
        "tool": tool,
        "args_hash": args_hash(args),
        "decision": decision,
        "actor": actor,
        "agent_id": agent_id,
        "diff_hash": diff_hash,
    });
    let _ = writeln!(f, "{line}");
    crate::protocol::emit(
        &crate::protocol::Event::new("audit")
            .with("tool", json!(tool))
            .with("decision", json!(decision))
            .with("actor", json!(actor)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn args_hash_is_stable_and_hex() {
        let h1 = args_hash(r#"{"path":"foo.txt"}"#);
        let h2 = args_hash(r#"{"path":"foo.txt"}"#);
        assert_eq!(h1, h2, "same input must produce the same hash");
        assert_eq!(h1.len(), 64, "SHA-256 hex is 64 chars");
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn args_hash_is_deterministic() {
        let h = args_hash("hello world");
        assert_eq!(h, args_hash("hello world"));
    }

    #[test]
    fn args_hash_different_inputs_differ() {
        let h1 = args_hash("a");
        let h2 = args_hash("b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn audit_path_uses_session_file_extension() {
        let ws = Path::new("/tmp/ws");
        let session = Path::new("/tmp/session.json");
        let p = audit_path(Some(session), ws);
        assert_eq!(p.extension().and_then(|e| e.to_str()), Some("audit.jsonl"));
        assert!(p.to_string_lossy().contains("session"));
    }

    #[test]
    fn audit_path_falls_back_to_default() {
        let ws = Path::new("/tmp/ws");
        let p = audit_path(None, ws);
        assert!(p.ends_with(".catalyst-code/audit.jsonl"));
    }

    #[test]
    fn record_respects_disabled_flag() {
        // When enabled is false, record returns immediately without error.
        let tmp = std::env::temp_dir().join("catcode-audit-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // Should not panic and should not create the audit file.
        record(false, None, &tmp, "write", "{}", "deny", "user", None, None);
        let default_path = tmp.join(".catalyst-code").join("audit.jsonl");
        assert!(
            !default_path.exists(),
            "disabled record should not create file"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
