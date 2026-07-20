use crate::config::Config;
use crate::tools::{smart_truncate, Outcome};
use serde_json::Value;

fn git_exec(cfg: &Config, subcmd: &[&str]) -> Outcome {
    use std::io::Read;
    fn read_all<R: Read>(r: &mut R) -> String {
        let mut v = Vec::new();
        let _ = r.read_to_end(&mut v);
        String::from_utf8_lossy(&v).into_owned()
    }
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(&cfg.workspace)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .args(subcmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Outcome::err("git not found on PATH");
        }
        Err(e) => return Outcome::err(format!("git exec failed: {e}")),
    };
    let out_h = child.stdout.take();
    let err_h = child.stderr.take();
    let t_out = std::thread::spawn(move || out_h.map(|mut r| read_all(&mut r)).unwrap_or_default());
    let t_err = std::thread::spawn(move || err_h.map(|mut r| read_all(&mut r)).unwrap_or_default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(_) => break None,
        }
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();
    // Same 32KB smart_truncate CAP as bash — git log/diff dumps can blow context.
    const CAP: usize = 32_768;
    let cap = |body: String| -> String {
        if body.len() > CAP {
            smart_truncate(&body, CAP)
        } else {
            body
        }
    };
    match status {
        Some(s) if s.success() => {
            let body = if !stdout.trim().is_empty() {
                stdout
            } else if !stderr.trim().is_empty() {
                stderr
            } else {
                String::from("(no output)")
            };
            Outcome::ok(cap(body))
        }
        Some(s) => {
            let body = if !stderr.trim().is_empty() {
                stderr
            } else if !stdout.trim().is_empty() {
                stdout
            } else {
                format!("git {:?} failed (exit {:?})", subcmd, s.code())
            };
            Outcome::err(cap(body))
        }
        None => Outcome::err(format!("git {:?} timed out after 30s", subcmd)),
    }
}

/// Validate a workspace-relative git pathspec: reject absolute paths and `..`
/// escapes. Returns Ok("") for an empty path (meaning "no path filter").
fn git_rel_path(p: &str) -> Result<String, String> {
    if p.is_empty() {
        return Ok(String::new());
    }
    if p.starts_with('/') || p.starts_with('\\') || (p.len() >= 2 && p.as_bytes()[1] == b':') {
        return Err(format!(
            "git path must be workspace-relative, got absolute: {p:?}"
        ));
    }
    for comp in p.split(['/', '\\']) {
        if comp == ".." {
            return Err(format!(
                "git path must not escape the workspace (..): {p:?}"
            ));
        }
    }
    Ok(p.to_string())
}

pub(crate) fn git_status(args: &Value, cfg: &Config) -> Outcome {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    match git_rel_path(path) {
        Err(e) => Outcome::err(e),
        Ok(p) if p.is_empty() => git_exec(cfg, &["status", "--short", "--branch"]),
        Ok(p) => git_exec(cfg, &["status", "--short", "--branch", "--", &p]),
    }
}

pub(crate) fn git_diff(args: &Value, cfg: &Config) -> Outcome {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let staged = args
        .get("staged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    match git_rel_path(path) {
        Err(e) => Outcome::err(e),
        Ok(p) => {
            let mut cmd: Vec<String> = vec!["diff".into(), "--no-color".into()];
            if staged {
                cmd.push("--staged".into());
            }
            if !p.is_empty() {
                cmd.push("--".into());
                cmd.push(p);
            }
            let refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
            git_exec(cfg, &refs)
        }
    }
}

pub(crate) fn git_log(args: &Value, cfg: &Config) -> Outcome {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(1000);
    match git_rel_path(path) {
        Err(e) => Outcome::err(e),
        Ok(p) => {
            let n = format!("-n{limit}");
            let mut cmd: Vec<String> = vec!["log".into(), "--oneline".into(), n];
            if !p.is_empty() {
                cmd.push("--".into());
                cmd.push(p);
            }
            let refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
            git_exec(cfg, &refs)
        }
    }
}

/// List OTHER active catalyst-code sessions in this workspace (separate
/// processes), each with its goal, in-progress work, and recently touched
/// files. Awareness only — read-only broadcast of "who is here". Use when
/// something seems off to decide whether a neighbor caused it before assuming
/// you introduced the error. Stale/crashed sessions are auto-pruned by mtime.
pub(crate) fn workspace_activity(_args: &Value, cfg: &Config) -> Outcome {
    let my_pid = std::process::id();
    let peers = crate::presence::read_peers(&cfg.workspace, my_pid);
    if peers.is_empty() {
        return Outcome::ok(
            "No other active catalyst-code sessions in this workspace. Any error \
             you are seeing is from your own work or the environment.",
        );
    }
    let now = crate::presence::unix_now();
    let mut out = format!(
        "{} other active session(s) in this workspace:\n",
        peers.len()
    );
    for p in &peers {
        out.push_str(&format!(
            "\n- pid {} (started {}, last active {})",
            p.pid,
            age(now, p.started_at),
            age(now, p.last_heartbeat)
        ));
        if let Some(sid) = &p.session_id {
            out.push_str(&format!(", session {sid}"));
        }
        if let Some(m) = &p.model {
            out.push_str(&format!(", model {m}"));
        }
        if !p.goal.is_empty() {
            out.push_str(&format!("\n  goal: {}", truncate(p.goal.as_str(), 140)));
        }
        if !p.in_progress.is_empty() {
            out.push_str(&format!("\n  in progress: {}", p.in_progress.join("; ")));
        }
        if !p.next.is_empty() {
            out.push_str(&format!("\n  next: {}", p.next.join("; ")));
        }
        if !p.recent_files.is_empty() {
            out.push_str(&format!(
                "\n  recently touched: {}",
                p.recent_files.join(", ")
            ));
        }
        if !p.last_activity.is_empty() {
            out.push_str(&format!(
                "\n  last: {}",
                truncate(p.last_activity.as_str(), 140)
            ));
        }
    }
    Outcome::ok(out)
}

/// Render a unix-seconds delta as a compact human age ("3m", "2h", "just now").
fn age(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    if s < 5 {
        "just now".to_string()
    } else if s < 60 {
        format!("{}s ago", s)
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86400)
    }
}

/// Truncate `s` to at most `n` chars, appending an ellipsis if cut. A small
/// local copy of main.rs's `truncate_str` (kept private there) so this module
/// stays self-contained.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

pub(crate) fn git_add(args: &Value, cfg: &Config) -> Outcome {
    let Some(paths) = args.get("paths").and_then(|v| v.as_array()) else {
        return Outcome::err("git_add requires a 'paths' array");
    };
    if paths.is_empty() {
        return Outcome::err("git_add requires a non-empty 'paths' array");
    }
    let mut cmd: Vec<String> = vec!["add".into(), "--".into()];
    for p in paths {
        let Some(ps) = p.as_str() else {
            return Outcome::err("git_add: every path must be a string");
        };
        match git_rel_path(ps) {
            Err(e) => return Outcome::err(e),
            Ok(rp) => cmd.push(rp),
        }
    }
    let refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    git_exec(cfg, &refs)
}

pub(crate) fn git_commit(args: &Value, cfg: &Config) -> Outcome {
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    if message.trim().is_empty() {
        return Outcome::err("git_commit requires a non-empty 'message'");
    }
    let mut cmd: Vec<String> = vec!["commit".into(), "-m".into(), message.to_string()];
    if all {
        cmd.push("--all".into());
    }
    let refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    git_exec(cfg, &refs)
}

// ---- memory tool (agent-callable wrapper over crate::memory) ----
