// Built-in tools the agent can call. OpenAI function-calling schema.
// All file ops are confined to the workspace root; bash runs with cwd=workspace
// and a real timeout+kill. read_file uses hashline format for anchored editing.
use crate::config::Config;
use crate::hashline::{line_hash, tag_line};
use crate::workspace;
use serde_json::{json, Value};

/// ToolKind drives the approval gate in main.rs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    ReadOnly,   // read_file, list_dir, grep, glob — never gated
    Destructive,// bash, write_file, edit — gated under Approval::Destructive
}

/// Classify a tool by name for approval purposes.
pub fn classify(name: &str) -> ToolKind {
    match name {
        "read_file" | "list_dir" | "grep" | "glob" | "bulk_read" | "todo_read" | "diagnostics" | "finish" => ToolKind::ReadOnly,
        _ => ToolKind::Destructive,
    }
}

pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file. Each line is returned as \"HASH│content\". Use the 4-char HASH values as anchors for the edit tool. Path is relative to the workspace root. For large files, pass offset (1-indexed line) and limit (line count) to page through the file instead of loading it all at once. Files >5MB are refused.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "offset": { "type": "integer", "description": "1-indexed line to start at (for paging large files)" },
                        "limit": { "type": "integer", "description": "max lines to return (for paging large files)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "edit",
                "description": "Apply hash-anchored edits to a file. Read it first with read_file to get HASH anchors, then call edit with those hashes. On a stale-anchor error, re-read and retry with fresh hashes. ops: replace (start+end hashes, inclusive; single line = start==end; delete = lines:[]), append (insert after pos hash; omit pos for EOF), prepend (insert before pos hash; omit pos for BOF). All ops in one call apply atomically to one file snapshot.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "op": { "type": "string", "enum": ["replace", "append", "prepend"] },
                                    "start": { "type": "string", "description": "hash anchor; required for replace" },
                                    "end": { "type": "string", "description": "hash anchor; required for replace" },
                                    "pos": { "type": "string", "description": "hash anchor; for append/prepend (omit for EOF/BOF)" },
                                    "lines": { "type": "array", "items": { "type": "string" }, "description": "new content, one string per line" }
                                },
                                "required": ["op", "lines"]
                            }
                        }
                    },
                    "required": ["path", "edits"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write full content to a file (creates parents, overwrites if present). Use for new files or complete rewrites; prefer edit for targeted changes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List entries in a directory (relative path). Returns one entry per line, directories suffixed with /.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search file contents for a pattern (regex) under the workspace. Returns matching lines as path:line:content, capped at 50 matches.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Rust regex" },
                        "path": { "type": "string", "description": "directory to search (relative); defaults to workspace root" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "glob",
                "description": "Find files by glob pattern (e.g. \"**/*.rs\") under the workspace. Returns relative paths, capped at 200.",
                "parameters": {
                    "type": "object",
                    "properties": { "pattern": { "type": "string" } },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Run a bash command in the workspace root. Returns combined stdout+stderr (truncated to 8KB). Commands run with a 30s timeout (configurable). A small denylist blocks catastrophic commands.",
                "parameters": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bulk",
                "description": "Run several tool calls in one round-trip. Each entry has a tool name and its args object. Dispatches any built-in tool (read_file, write_file, edit, list_dir, grep, glob, bash). Returns one result block per call, in order. Use this to batch independent operations and cut round-trips; the whole batch shares one approval gate.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "calls": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string", "enum": ["read_file","write_file","edit","list_dir","grep","glob","bash"] },
                                    "args": { "type": "object" }
                                },
                                "required": ["name","args"]
                            }
                        }
                    },
                    "required": ["calls"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bulk_read",
                "description": "Read many files in one call. Returns each file as a headed block with hashlined content (same HASH│content format as read_file). Paths are relative to the workspace root. Per-file errors are reported inline rather than failing the whole call.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["paths"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bulk_write",
                "description": "Write many files in one call. Each entry is {path, content}; parents are created and existing files are overwritten, exactly like write_file. Returns one line per file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "content": { "type": "string" }
                                },
                                "required": ["path","content"]
                            }
                        }
                    },
                    "required": ["files"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bulk_edit",
                "description": "Apply hash-anchored edits to many files in one call. Each entry is {path, ops} where ops is the same edit op array as the edit tool (replace/append/prepend with HASH anchors). Read each file first with read_file/bulk_read to get fresh hashes. All ops on a file apply atomically to one snapshot; one stale anchor fails only that file's block.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "ops": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "op": { "type": "string", "enum": ["replace","append","prepend"] },
                                                "start": { "type": "string" },
                                                "end": { "type": "string" },
                                                "pos": { "type": "string" },
                                                "lines": { "type": "array", "items": { "type": "string" } }
                                            },
                                            "required": ["op","lines"]
                                        }
                                    }
                                },
                                "required": ["path","ops"]
                            }
                        }
                    },
                    "required": ["edits"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "todo_write",
                "description": "Write the full task list (plan). Each todo has {subject, status, content?}. status is pending|in_progress|completed. Replaces the whole list. Use this to track multi-step work across context compaction.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "subject": { "type": "string" },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                                    "content": { "type": "string", "description": "optional detail" }
                                },
                                "required": ["subject", "status"]
                            }
                        }
                    },
                    "required": ["todos"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "todo_read",
                "description": "Read the current task list. Returns the JSON plan (or [] if empty).",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "finish",
                "description": "Signal that the task is complete. Call this when you have finished the user's request and verified your work; it exits the agentic loop cleanly.",
                "parameters": { "type": "object", "properties": {} }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "patch",
                "description": "Apply a unified diff patch to a file. Use for larger refactors than edit handles well. Context lines must match. Path is relative to the workspace root.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "patch": { "type": "string", "description": "unified diff (@@ hunks, +/-/ space prefixes)" }
                    },
                    "required": ["path", "patch"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "diagnostics",
                "description": "Run the project's type checker / compiler (cargo check, tsc --noEmit, go build, or py_compile) and return diagnostics. Use after edits to catch type/syntax errors before declaring done.",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "subdirectory to check (defaults to workspace root)" } }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "spawn",
                "description": "Run a nested agentic turn with a fresh sub-conversation and its own tool loop. Use to delegate a bounded sub-task (research, review, implementation) and get a text result back. The sub-agent shares the same workspace and tools but cannot spawn further sub-agents.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string", "description": "the sub-task for the nested agent" },
                        "model": { "type": "string", "description": "model id (defaults to the parent's model)" }
                    },
                    "required": ["prompt"]
                }
            }
        }),
    ]
}

/// Outcome of a tool call. For bash we need a future with timeout+kill, so
/// destructive/bash execution is split: execute() handles sync tools;
/// execute_bash() is async and takes a runtime handle.
pub struct Outcome {
    pub ok: bool,
    pub output: String,
}

/// Execute a (non-bash) tool call synchronously. `cfg` provides confinement+limits.
/// bash is handled separately by execute_bash (async, timeout+kill).
pub fn execute(name: &str, args: &Value, cfg: &Config) -> Outcome {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "read_file" => read_file(s("path"), args, cfg),
        "todo_read" => todo_read(cfg),
        "todo_write" => todo_write(args, cfg),
        "finish" => Outcome::ok("__finish__"), // sentinel; main.rs treats as loop exit
        "patch" => apply_patch(args, cfg),
        "diagnostics" => Outcome::err("diagnostics must be dispatched through execute_diagnostics (async)"),
        "spawn" => Outcome::err("spawn must be dispatched through execute_spawn (async)"),
        "edit" => {
            let path = s("path");
            match args.get("edits").and_then(|v| v.as_array()) {
                Some(e) if !e.is_empty() => execute_edit(path, e, cfg),
                _ => Outcome::err("edit requires a non-empty 'edits' array"),
            }
        }
        "write_file" => write_file(s("path"), s("content"), cfg),
        "list_dir" => list_dir(s("path"), cfg),
        "grep" => grep(s("pattern"), s("path"), cfg),
        "glob" => glob(s("pattern"), cfg),
        "bulk_read" => bulk_read(args, cfg),
        "bulk_write" => bulk_write(args, cfg),
        "bulk_edit" => bulk_edit(args, cfg),
        "bash" => Outcome::err("bash must be dispatched through execute_bash (async)"),
        "bulk" => Outcome::err("bulk must be dispatched through execute_bulk (async)"),
        other => Outcome::err(&format!("unknown tool: {other}")),
    }
}

impl Outcome {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self { ok: true, output: msg.into() }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self { ok: false, output: msg.into() }
    }
}

// ---- file tools ----

fn read_file(input: &str, args: &Value, cfg: &Config) -> Outcome {
    let path = match workspace::resolve(&cfg.workspace, input) {
        Ok(p) => p,
        Err(e) => return Outcome::err(e),
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return Outcome::err(format!("read_file {input:?} failed: {e}")),
    };
    if meta.len() > cfg.max_read_bytes {
        return Outcome::err(format!(
            "read_file {input:?} is {} bytes (max {}); use grep to slice it or pass offset/limit",
            meta.len(), cfg.max_read_bytes
        ));
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return Outcome::err(format!("read_file {input:?} failed: {e}")),
    };
    let (lines, _trailing) = split_lines(&content);
    // Optional pagination: offset (1-indexed) + limit slice a window so
    // files >max_read_lines still load page-by-page instead of being refused.
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
    if offset > 0 || limit.is_some() {
        let start = offset.saturating_sub(1).min(lines.len());
        let end = match limit {
            Some(n) => (start + n).min(lines.len()),
            None => lines.len(),
        };
        let window = &lines[start..end];
        let mut out = String::new();
        out.push_str(&format!("# {input} lines {}-{} of {}\n", start + 1, end, lines.len()));
        for l in window {
            out.push_str(&tag_line(l));
            out.push('\n');
        }
        return Outcome::ok(out);
    }
    if lines.len() > cfg.max_read_lines {
        return Outcome::err(format!(
            "read_file {input:?} has {} lines (max {}); pass offset/limit to page it",
            lines.len(), cfg.max_read_lines
        ));
    }
    Outcome::ok(read_hashlined(&content))
}


fn write_file(input: &str, content: &str, cfg: &Config) -> Outcome {
    let path = match workspace::resolve(&cfg.workspace, input) {
        Ok(p) => p,
        Err(e) => return Outcome::err(e),
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Outcome::err(format!("write_file mkdir failed: {e}"));
            }
        }
    }
    match std::fs::write(&path, content) {
        Ok(_) => Outcome::ok(format!("wrote {} bytes to {input}", content.len())),
        Err(e) => Outcome::err(format!("write_file {input:?} failed: {e}")),
    }
}

fn list_dir(input: &str, cfg: &Config) -> Outcome {
    let path = match workspace::resolve(&cfg.workspace, input) {
        Ok(p) => p,
        Err(e) => return Outcome::err(e),
    };
    match std::fs::read_dir(&path) {
        Ok(rd) => {
            let mut entries: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        format!("{name}/")
                    } else {
                        name
                    }
                })
                .collect();
            entries.sort();
            Outcome::ok(entries.join("\n"))
        }
        Err(e) => Outcome::err(format!("list_dir {input:?} failed: {e}")),
    }
}

fn grep(pattern: &str, input: &str, cfg: &Config) -> Outcome {
    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return Outcome::err(format!("grep bad pattern: {e}")),
    };
    let root = if input.is_empty() {
        cfg.workspace.clone()
    } else {
        match workspace::resolve(&cfg.workspace, input) {
            Ok(p) => p,
            Err(e) => return Outcome::err(e),
        }
    };
    let mut hits: Vec<String> = Vec::new();
    let mut dirs: Vec<std::path::PathBuf> = vec![root.clone()];
    let mut seen = 0u32;
    while let Some(dir) = dirs.pop() {
        if seen > 5000 {
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            seen += 1;
            let p = e.path();
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                // ponytail: skip VCS/build dirs — noise.
                let name = e.file_name().to_string_lossy().to_string();
                if !matches!(name.as_str(), ".git" | "node_modules" | "target" | "dist" | "build" | ".venv") {
                    dirs.push(p);
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            if p.extension().and_then(|x| x.to_str()).map(|x| x.len()).unwrap_or(0) > 40 {
                continue; // skip binary-ish extensions
            }
            // ponytail: size guard + content sniff so we don't slurp a 2GB log.
            let Ok(meta) = e.metadata() else { continue };
            if meta.len() > 5_000_000 { continue; } // 5MB cap per file
            let Ok(content) = std::fs::read_to_string(&p) else { continue };
            // binary sniff: NUL bytes mean binary — skip.
            if content.contains('\0') { continue; }
            let rel = p.strip_prefix(&cfg.workspace).unwrap_or(&p).display().to_string();
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(format!("{rel}:{}:{}", i + 1, line));
                    if hits.len() >= 50 {
                        return Outcome::ok(hits.join("\n") + "\n...[50 match cap reached]");
                    }
                }
            }
        }
    }
    Outcome::ok(hits.join("\n"))
}

fn glob(pattern: &str, cfg: &Config) -> Outcome {
    let mut out: Vec<String> = Vec::new();
    walk_glob(&cfg.workspace, &cfg.workspace, pattern, &mut out, 0);
    if out.is_empty() {
        Outcome::ok("")
    } else if out.len() > 200 {
        out.truncate(200);
        Outcome::ok(out.join("\n") + "\n...[200 result cap reached]")
    } else {
        Outcome::ok(out.join("\n"))
    }
}

// ponytail: hand-rolled glob. Supports *, **, ?, and literal segments.
// Not a full POSIX glob; covers the common **/*.ext and dir/*.rs patterns.
fn walk_glob(root: &std::path::Path, dir: &std::path::Path, pattern: &str, out: &mut Vec<String>, depth: usize) {
    if out.len() >= 200 || depth > 15 {
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        if out.len() >= 200 {
            break;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if matches!(name.as_str(), ".git" | "node_modules" | "target" | "dist" | "build" | ".venv") {
            continue;
        }
        let p = e.path();
        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if glob_match(pattern, &rel) {
            out.push(rel.clone());
        }
        if is_dir {
            walk_glob(root, &p, pattern, out, depth + 1);
        }
    }
}

fn glob_match(pattern: &str, name: &str) -> bool {
    // ponytail: convert glob to a simple matcher. ** matches any path depth.
    // Handle the common cases; fall back to substring match.
    if pattern.contains("**") {
        let suffix = pattern.replace("**/", "").replace("**", "");
        if suffix.is_empty() {
            return true;
        }
        // Match the suffix (which may contain * and ?) against the file's basename, or as a path suffix for multi-segment suffixes.
        if suffix.contains('/') {
            return name == suffix || name.ends_with(&format!("/{suffix}"));
        }
        let basename = name.rsplit('/').next().unwrap_or(name);
        return star_match(&suffix, basename);
    }
    // single-segment glob with * and ?
    if !pattern.contains('/') {
        return star_match(pattern, name);
    }
    // multi-segment: match segment by segment
    let ps: Vec<&str> = pattern.split('/').collect();
    let ns: Vec<&str> = name.split('/').collect();
    if ps.len() != ns.len() {
        return false;
    }
    ps.iter().zip(ns.iter()).all(|(p, n)| star_match(p, n))
}

fn star_match(pat: &str, s: &str) -> bool {
    // ponytail: classic * and ? glob without a crate.
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    glob_dp(&p, &t)
}

fn glob_dp(p: &[char], t: &[char]) -> bool {
    // DP matching for * (any run) and ? (one char).
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            match p[i - 1] {
                '*' => dp[i][j] = dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i][j] = dp[i - 1][j - 1],
                c => dp[i][j] = dp[i - 1][j - 1] && c == t[j - 1],
            }
        }
    }
    dp[p.len()][t.len()]
}

// ---- bash (async, timeout + kill) ----

/// Run bash with cwd=workspace, a real timeout, and a denylist tripwire.
/// Optional hard sandbox: --sandbox firejail wraps the command in a
/// firejail profile that whitelists only the workspace; --no-network adds
/// `unshare -n` so the command can't phone home. Both are belt-and-suspenders
/// on top of the denylist tripwire.
pub async fn execute_bash(command: &str, cfg: &Config) -> Outcome {
    // ponytail: denylist is a tripwire, not a sandbox. It blocks the most
    // catastrophic obvious commands; a determined model bypasses it.
    let lower = command.to_ascii_lowercase();
    for bad in &cfg.bash_deny {
        if lower.contains(&bad.to_ascii_lowercase()) {
            return Outcome::err(format!("bash command blocked by denylist (matched '{bad}'); use a sandbox for hard isolation"));
        }
    }
    // Regex denylist: block commands matching regex patterns.
    for pattern in &cfg.bash_deny_regex {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if re.is_match(command) {
                    return Outcome::err(format!("bash command blocked by regex denylist (matched '{pattern}'); use a sandbox for hard isolation"));
                }
            }
            Err(_) => {} // Bad regex: ignore (config error, not runtime)
        }
    }

    // Build the argv. If a sandbox is configured, we exec the sandbox wrapper
    // instead of bash directly; the wrapper runs bash -c <command> inside.
    // ponytail: firejail profile is generated per-run into a temp file so the
    // workspace whitelist is always the current cfg.workspace. One file per
    // call is wasteful under load but correct; cache if it ever matters.
    let (_profile, mut cmd) = build_bash_command(command, cfg);

    cmd.current_dir(&cfg.workspace);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());
    cmd.kill_on_drop(true);

    let child: tokio::process::Child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let hint = match cfg.sandbox {
                crate::config::Sandbox::Firejail if e.kind() == std::io::ErrorKind::NotFound =>
                    " (is firejail installed and on PATH?)",
                _ => "",
            };
            return Outcome::err(format!("bash failed to spawn: {e}{hint}"));
        }
    };

    let timeout = std::time::Duration::from_secs(cfg.bash_timeout_secs);
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;
    match result {
        Ok(Ok(o)) => {
            let mut combined = String::new();
            if !o.stdout.is_empty() {
                combined.push_str(&String::from_utf8_lossy(&o.stdout));
            }
            if !o.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n--- stderr ---\n");
                }
                combined.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            let ok = o.status.success();
            if combined.is_empty() {
                combined.push_str("(no output)");
            }
            // ponytail: 32KB cap (was 8KB) — large builds/logs need room to
            // reach the error. Truncate the *head* when over cap so the tail
            // (where errors usually are) survives.
            const CAP: usize = 32_768;
            if combined.len() > CAP {
                let start = combined.len() - CAP;
                let mut cut = String::with_capacity(CAP + 64);
                cut.push_str("...[head truncated, showing last 32KB]...\n");
                cut.push_str(&combined[start..]);
                combined = cut;
            }
            Outcome { ok, output: combined }
        }
        Ok(Err(e)) => Outcome::err(format!("bash wait failed: {e}")),
        Err(_) => Outcome::err(format!("bash timed out after {}s (killed)", cfg.bash_timeout_secs)),
    }
}

/// Build the tokio Command for a bash invocation, applying the configured
/// sandbox. Returns (optional temp profile path, Command). The profile path
/// is kept alive for the lifetime of the returned Command via the temp file.
fn build_bash_command(command: &str, cfg: &Config) -> (Option<std::path::PathBuf>, tokio::process::Command) {
    use crate::config::Sandbox;
    match cfg.sandbox {
        Sandbox::None => {
            // --no-network still applies even without firejail: unshare -n
            // creates a new network namespace with no interfaces.
            if cfg.no_network {
                let mut c = tokio::process::Command::new("unshare");
                c.arg("-n").arg("bash").arg("-c").arg(command);
                return (None, c);
            }
            let mut c = tokio::process::Command::new("bash");
            c.arg("-c").arg(command);
            (None, c)
        }
        Sandbox::Firejail => {
            // Generate a per-run profile that whitelists the workspace (read+write)
            // and the system paths bash needs, and optionally drops networking.
            let profile = firejail_profile(&cfg.workspace, cfg.no_network);
            let path = std::env::temp_dir().join(format!("umans-harness-fj-{}.profile", std::process::id()));
            let _ = std::fs::write(&path, &profile);
            let mut c = tokio::process::Command::new("firejail");
            c.arg("--quiet").arg("--profile").arg(&path).arg("bash").arg("-c").arg(command);
            (Some(path), c)
        }
    }
}

/// A firejail profile that whitelists the workspace (read+write), the shell
/// and its libs, /tmp, and nothing else. With no_network, drops net entirely.
fn firejail_profile(workspace: &std::path::Path, no_network: bool) -> String {
    let ws = workspace.display();
    let mut s = String::new();
    s.push_str("# auto-generated by umans-harness-core\n");
    s.push_str("# ponytail: whitelist the workspace + shell paths; deny everything else.\n");
    // Shell + coreutils locations
    for p in ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives", "/dev/null"] {
        s.push_str(&format!("read-only {p}\n"));
    }
    // Workspace is read-write
    s.push_str(&format!("whitelist {ws}\n"));
    s.push_str("read-write {ws}\n");
    // /tmp for scratch (firejail private-tmp)
    s.push_str("read-write /tmp\n");
    s.push_str("whitelist /tmp\n");
    if no_network {
        s.push_str("net none\n");
    } else {
        // still drop raw sockets etc.
        s.push_str("protocol unix,inet,inet6\n");
    }
    s.push_str("caps.drop all\n");
    s.push_str("seccomp\n");
    s.push_str("noroot\n");
    s.push_str("private-tmp\n");
    s
}


// ---- hashline edit (unchanged logic, now confined) ----

fn read_hashlined(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let trailing_nl = content.ends_with('\n');
    let body = if trailing_nl { &content[..content.len() - 1] } else { content };
    let mut out = String::with_capacity(content.len() + body.lines().count() * 6);
    for line in body.split('\n') {
        out.push_str(&tag_line(line));
        out.push('\n');
    }
    out
}

fn split_lines(content: &str) -> (Vec<String>, bool) {
    let trailing_nl = content.ends_with('\n');
    if content.is_empty() {
        return (Vec::new(), false);
    }
    let mut v: Vec<String> = content.split('\n').map(String::from).collect();
    if trailing_nl {
        v.pop();
    }
    (v, trailing_nl)
}

fn find_hash(lines: &[String], hash: &str, from: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, l)| line_hash(l) == hash)
        .map(|(i, _)| i)
}

fn stale_msg(hash: &str, path: &str) -> String {
    format!("stale anchor '{hash}' not found in {path:?}; re-read the file and use fresh hashes")
}

fn execute_edit(input: &str, edits: &[Value], cfg: &Config) -> Outcome {
    let path = match workspace::resolve(&cfg.workspace, input) {
        Ok(p) => p,
        Err(e) => return Outcome::err(e),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return Outcome::err(format!("edit: read {input:?} failed: {e}")),
    };
    let (mut lines, trailing_nl) = split_lines(&content);
    let mut log: Vec<String> = Vec::new();
    let mut applied = 0usize;

    for (i, ev) in edits.iter().enumerate() {
        let op = ev.get("op").and_then(|v| v.as_str()).unwrap_or("");
        let new_lines: Vec<String> = ev
            .get("lines")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let anchor = |key: &str| ev.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());

        match op {
            "replace" => {
                let (Some(start), Some(end)) = (anchor("start"), anchor("end")) else {
                    return Outcome::err(format!("edit #{i}: replace needs both start and end hashes"));
                };
                let si = match find_hash(&lines, &start, 0) {
                    Some(x) => x,
                    None => return Outcome::err(stale_msg(&start, input)),
                };
                let ei = match find_hash(&lines, &end, si) {
                    Some(x) => x,
                    None => return Outcome::err(stale_msg(&end, input)),
                };
                let removed = ei - si + 1;
                lines.splice(si..=ei, new_lines.clone());
                log.push(format!("replaced {removed} line(s) [{start}..{end}] with {}", new_lines.len()));
                applied += 1;
            }
            "append" => {
                let at = match anchor("pos") {
                    Some(p) => match find_hash(&lines, &p, 0) {
                        Some(x) => x + 1,
                        None => return Outcome::err(stale_msg(&p, input)),
                    },
                    None => lines.len(),
                };
                for (k, l) in new_lines.iter().enumerate() {
                    lines.insert(at + k, l.clone());
                }
                log.push(format!("appended {} line(s) at {at}", new_lines.len()));
                applied += 1;
            }
            "prepend" => {
                let at = match anchor("pos") {
                    Some(p) => match find_hash(&lines, &p, 0) {
                        Some(x) => x,
                        None => return Outcome::err(stale_msg(&p, input)),
                    },
                    None => 0,
                };
                for (k, l) in new_lines.iter().enumerate() {
                    lines.insert(at + k, l.clone());
                }
                log.push(format!("prepended {} line(s) at {at}", new_lines.len()));
                applied += 1;
            }
            other => return Outcome::err(format!("edit #{i}: unknown op '{other}'")),
        }
    }

    let mut out = lines.join("\n");
    if trailing_nl && !lines.is_empty() {
        out.push('\n');
    }
    if let Err(e) = std::fs::write(&path, out) {
        return Outcome::err(format!("edit: write {input:?} failed: {e}"));
    }
    Outcome::ok(format!("applied {applied} edit(s): {}", log.join("; ")))
}

// ---- bulk tools ----
// ponytail: thin batch wrappers over the single-file primitives. Each entry
// gets its own result block so per-file errors don't abort the whole batch.

/// Read many files. Each file becomes a headed block; per-file errors inline.
fn bulk_read(args: &Value, cfg: &Config) -> Outcome {
    let Some(paths) = args.get("paths").and_then(|v| v.as_array()) else {
        return Outcome::err("bulk_read requires a 'paths' array");
    };
    if paths.is_empty() {
        return Outcome::err("bulk_read requires a non-empty 'paths' array");
    }
    let mut blocks: Vec<String> = Vec::with_capacity(paths.len());
    let mut ok = true;
    for (i, p) in paths.iter().enumerate() {
        let Some(path) = p.as_str() else {
            ok = false;
            blocks.push(format!("### [{i}] <invalid path>
error: path must be a string"));
            continue;
        };
        let r = read_file(path, p, cfg);
        if !r.ok { ok = false; }
        blocks.push(format!("### [{i}] {path}\n{}", r.output));
    }
    Outcome { ok, output: blocks.join("\n\n") }
}

/// Write many files. One status line per file; ok only if every write succeeded.
fn bulk_write(args: &Value, cfg: &Config) -> Outcome {
    let Some(files) = args.get("files").and_then(|v| v.as_array()) else {
        return Outcome::err("bulk_write requires a 'files' array");
    };
    if files.is_empty() {
        return Outcome::err("bulk_write requires a non-empty 'files' array");
    }
    let mut lines: Vec<String> = Vec::with_capacity(files.len());
    let mut ok = true;
    for (i, f) in files.iter().enumerate() {
        let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = f.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            ok = false;
            lines.push(format!("[{i}] error: missing 'path'"));
            continue;
        }
        let r = write_file(path, content, cfg);
        if !r.ok { ok = false; }
        lines.push(format!("[{i}] {path}: {}", r.output));
    }
    Outcome { ok, output: lines.join("\n") }
}

/// Apply edits to many files. Each file edits apply atomically to one snapshot;
/// a stale anchor fails only that file's block.
fn bulk_edit(args: &Value, cfg: &Config) -> Outcome {
    let Some(edits) = args.get("edits").and_then(|v| v.as_array()) else {
        return Outcome::err("bulk_edit requires an 'edits' array");
    };
    if edits.is_empty() {
        return Outcome::err("bulk_edit requires a non-empty 'edits' array");
    }
    let mut blocks: Vec<String> = Vec::with_capacity(edits.len());
    let mut ok = true;
    for (i, e) in edits.iter().enumerate() {
        let path = e.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            ok = false;
            blocks.push(format!("### [{i}] <missing path>\nerror: missing 'path'"));
            continue;
        }
        let Some(ops) = e.get("ops").and_then(|v| v.as_array()) else {
            ok = false;
            blocks.push(format!("### [{i}] {path}\nerror: missing 'ops' array"));
            continue;
        };
        if ops.is_empty() {
            ok = false;
            blocks.push(format!("### [{i}] {path}\nerror: empty 'ops' array"));
            continue;
        }
        // Wrap as an edit tool call and reuse execute_edit.
        let wrapped = json!({ "path": path, "edits": ops });
        // ponytail: execute() takes a tool NAME, not a path — bulk_edit used to
        // pass the path string here, falling through to "unknown tool: <path>".
        let r = execute("edit", &wrapped, cfg);
        blocks.push(format!("### [{i}] {path}\n{}", r.output));
    }
    Outcome { ok, output: blocks.join("\n\n") }
}

/// Run many tool calls in one round-trip. Dispatches any built-in tool,
/// including bash (awaited per-call). One result block per call, in order.
/// ok only if every call succeeded.
pub async fn execute_bulk(args: &Value, cfg: &Config) -> Outcome {
    let Some(calls) = args.get("calls").and_then(|v| v.as_array()) else {
        return Outcome::err("bulk requires a 'calls' array");
    };
    if calls.is_empty() {
        return Outcome::err("bulk requires a non-empty 'calls' array");
    }
    let mut blocks: Vec<String> = Vec::with_capacity(calls.len());
    let mut ok = true;
    for (i, c) in calls.iter().enumerate() {
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let inner_args = c.get("args").cloned().unwrap_or(json!({}));
        if name.is_empty() {
            ok = false;
            blocks.push(format!("### [{i}] <missing name>\nerror: missing 'name'"));
            continue;
        }
        // ponytail: nested bulk/bash would recurse; block it to keep the gate simple.
        if name == "bulk" || name == "bulk_read" || name == "bulk_write" || name == "bulk_edit" {
            ok = false;
            blocks.push(format!("### [{i}] {name}\nerror: nested bulk calls are not allowed"));
            continue;
        }
        let r = if name == "bash" {
            let cmd = inner_args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            execute_bash(cmd, cfg).await
        } else {
            execute(&name, &inner_args, cfg)
        };
        if !r.ok { ok = false; }
        blocks.push(format!("### [{i}] {name}\n{}", r.output));
    }
    Outcome { ok, output: blocks.join("\n\n") }
}

// ---- todo / plan tracking (item 5) ----
// ponytail: a JSON file in .umans-harness/todo.json in the workspace. No DB,
// no schema migration — just a list of {subject, status, content?}.

fn todo_path(cfg: &Config) -> std::path::PathBuf {
    cfg.workspace.join(".umans-harness").join("todo.json")
}

fn todo_read(cfg: &Config) -> Outcome {
    let p = todo_path(cfg);
    match std::fs::read_to_string(&p) {
        Ok(s) => Outcome::ok(s),
        Err(_) => Outcome::ok("[]"), // empty plan, not an error
    }
}

fn todo_write(args: &Value, cfg: &Config) -> Outcome {
    let Some(todos) = args.get("todos").and_then(|v| v.as_array()) else {
        return Outcome::err("todo_write requires a 'todos' array");
    };
    // Validate shape: each must have subject + status.
    for (i, t) in todos.iter().enumerate() {
        let subject = t.get("subject").and_then(|v| v.as_str()).unwrap_or("");
        let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if subject.is_empty() {
            return Outcome::err(format!("todo #{i}: missing 'subject'"));
        }
        if !matches!(status, "pending" | "in_progress" | "completed") {
            return Outcome::err(format!("todo #{i}: status must be pending|in_progress|completed, got {status:?}"));
        }
    }
    let p = todo_path(cfg);
    if let Some(parent) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Outcome::err(format!("todo_write mkdir failed: {e}"));
        }
    }
    let body = json!({ "todos": todos });
    let pretty = serde_json::to_string_pretty(&body).unwrap_or_default();
    match std::fs::write(&p, pretty) {
        Ok(_) => Outcome::ok(format!("wrote {} todo(s)", todos.len())),
        Err(e) => Outcome::err(format!("todo_write failed: {e}")),
    }
}

// ---- unified diff / patch tool (item G) ----
// ponytail: hand-rolled unified-diff applier. Handles @@ hunk headers and
// context/add/remove lines. No rename/binary support; covers the common case
// of `diff --git a/... b/...` or bare hunks the model emits.

fn apply_patch(args: &Value, cfg: &Config) -> Outcome {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let patch = args.get("patch").and_then(|v| v.as_str()).unwrap_or("");
    if path.is_empty() || patch.is_empty() {
        return Outcome::err("patch requires 'path' and 'patch'");
    }
    let resolved = match workspace::resolve(&cfg.workspace, path) {
        Ok(p) => p,
        Err(e) => return Outcome::err(e),
    };
    let original = std::fs::read_to_string(&resolved).unwrap_or_default();
    match apply_unified_diff(&original, patch) {
        Ok(new) => {
            if let Err(e) = std::fs::write(&resolved, &new) {
                return Outcome::err(format!("patch write failed: {e}"));
            }
            Outcome::ok(format!("applied patch to {path} ({} -> {} bytes)", original.len(), new.len()))
        }
        Err(e) => Outcome::err(format!("patch failed: {e}")),
    }
}

/// Apply a unified diff to `original`, returning the new text. Supports
/// multiple @@ hunk headers. Context lines must match.
fn apply_unified_diff(original: &str, patch: &str) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(String::from).collect();
    // Track trailing newline so we don't add/drop one spuriously.
    let had_trailing_nl = original.ends_with('\n');
    let mut i = 0;
    let patch_lines: Vec<&str> = patch.lines().collect();
    while i < patch_lines.len() {
        let l = patch_lines[i];
        // Skip file headers (--- / +++) and diff --git lines.
        if l.starts_with("---") || l.starts_with("+++") || l.starts_with("diff --git") || l.starts_with("Index:") {
            i += 1;
            continue;
        }
        if let Some(rest) = l.strip_prefix("@@") {
            // Parse @@ -start,count +start2,count2 @@ context
            // We only need the old start.
            let old_start = rest.split(' ').find_map(|tok| {
                tok.strip_prefix('-').and_then(|s| s.split(',').next()).and_then(|n| n.parse::<usize>().ok())
            }).ok_or_else(|| format!("bad hunk header: {l}"))?;
            i += 1;
            let mut target = old_start.saturating_sub(1); // 1-indexed -> 0
            // Apply lines until the next hunk or EOF.
            while i < patch_lines.len() && !patch_lines[i].starts_with("@@") {
                let pl = patch_lines[i];
                if let Some(content) = pl.strip_prefix(' ') {
                    // context: must match
                    if target < lines.len() && lines[target] != content {
                        return Err(format!("context mismatch at line {}: expected {:?}, got {:?}", target + 1, lines[target], content));
                    }
                    target += 1;
                } else if let Some(content) = pl.strip_prefix('-') {
                    // removal
                    if target < lines.len() && lines[target] == content {
                        lines.remove(target);
                    } else {
                        return Err(format!("removal mismatch at line {}: {:?} not found", target + 1, content));
                    }
                } else if let Some(content) = pl.strip_prefix('+') {
                    // addition
                    lines.insert(target, content.to_string());
                    target += 1;
                } else if pl.is_empty() {
                    // blank context line (treat as context)
                    target += 1;
                } else {
                    // unknown line (\\ No newline, etc.) — skip
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    let mut out = lines.join("\n");
    if had_trailing_nl { out.push('\n'); }
    Ok(out)
}

// ---- diagnostics (item 5) ----
// ponytail: detect the project type from marker files and run the right
// checker. Returns stdout+stderr. Async because it shells out.

pub async fn execute_diagnostics(args: &Value, cfg: &Config) -> Outcome {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let target = if path.is_empty() { cfg.workspace.clone() } else {
        match workspace::resolve(&cfg.workspace, path) {
            Ok(p) => p,
            Err(e) => return Outcome::err(e),
        }
    };
    // Pick checker by marker files present.
    let (cmd, label) = if target.join("Cargo.toml").exists() {
        (vec!["cargo", "check", "--message-format=short"], "cargo check")
    } else if target.join("package.json").exists() {
        // try tsc, fall back to npm run build if no tsc
        (vec!["sh", "-c", "npx --no-install tsc --noEmit 2>&1 || npm run --silent build 2>&1"], "tsc/npm build")
    } else if target.join("go.mod").exists() {
        (vec!["go", "build", "./..."], "go build")
    } else if target.join("pyproject.toml").exists() || target.join("setup.py").exists() {
        (vec!["sh", "-c", "python -m py_compile $(find . -name '*.py' -not -path './.venv/*' | head -50) 2>&1"], "py_compile")
    } else {
        return Outcome::err("no recognized project marker (Cargo.toml/package.json/go.mod/pyproject.toml)");
    };
    let mut c = tokio::process::Command::new(&cmd[0]);
    c.args(&cmd[1..]);
    c.current_dir(&target);
    c.stdin(std::process::Stdio::null());
    let out = match c.output().await {
        Ok(o) => o,
        Err(e) => return Outcome::err(format!("{label} failed to run: {e}")),
    };
    let mut s = String::new();
    if !out.stdout.is_empty() { s.push_str(&String::from_utf8_lossy(&out.stdout)); }
    if !out.stderr.is_empty() {
        if !s.is_empty() { s.push_str("\n--- stderr ---\n"); }
        s.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    if s.is_empty() { s.push_str("(no diagnostics — clean)"); }
    // ponytail: diagnostics "ok" is true only when the checker exits 0.
    Outcome { ok: out.status.success(), output: format!("{label}\n{s}") }
}

// ---- spawn (subagent) (item 8) ----
// ponytail: the spawn tool's body is in main.rs (it needs the reqwest client,
// api key, models, conversation). tools.rs just exposes the tool definition.
// execute() returns a sentinel so misuse surfaces clearly.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_ws() -> (PathBuf, Config) {
        // ponytail: unique dir per call via atomic counter — tests run in parallel and share temp otherwise.
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("umans_harness_tools_ws_{}", n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.workspace = dir.clone();
        cfg.max_read_bytes = 1_048_576;
        cfg.max_read_lines = 2000;
        (dir, cfg)
    }

    fn hashes(content: &str) -> Vec<String> {
        let (lines, _) = split_lines(content);
        lines.iter().map(|l| line_hash(l)).collect()
    }

    #[test]
    fn read_hashlined_tags_each_line() {
        let out = read_hashlined("a\nb\n");
        assert!(out.starts_with(&format!("{}│a\n", line_hash("a"))));
        assert!(out.contains(&format!("{}│b\n", line_hash("b"))));
    }

    #[test]
    fn edit_replace_single_line() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "one\ntwo\nthree\n").unwrap();
        let h = hashes("one\ntwo\nthree\n");
        let args = json!({ "path": "f.txt", "edits": [{ "op": "replace", "start": h[1], "end": h[1], "lines": ["TWO"] }] });
        let o = execute("edit", &args, &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "one\nTWO\nthree\n");
    }

    #[test]
    fn edit_replace_range_then_append_and_prepend() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "a\nb\nc\nd\n").unwrap();
        let h = hashes("a\nb\nc\nd\n");
        let edits = vec![
            json!({ "op": "replace", "start": h[1], "end": h[2], "lines": ["X", "Y"] }),
            json!({ "op": "append", "pos": h[3], "lines": ["Z"] }),
            json!({ "op": "prepend", "pos": h[0], "lines": ["P"] }),
        ];
        let args = json!({ "path": "f.txt", "edits": edits });
        let o = execute("edit", &args, &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "P\na\nX\nY\nd\nZ\n");
    }

    #[test]
    fn edit_append_eof_and_prepend_bof_without_pos() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "mid\n").unwrap();
        let edits = json!([
            { "op": "prepend", "lines": ["TOP"] },
            { "op": "append", "lines": ["BOT"] }
        ]);
        let args = json!({ "path": "f.txt", "edits": edits });
        let o = execute("edit", &args, &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "TOP\nmid\nBOT\n");
    }

    #[test]
    fn edit_delete_via_empty_lines() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "keep\nkill\nkeep2\n").unwrap();
        let h = hashes("keep\nkill\nkeep2\n");
        let args = json!({ "path": "f.txt", "edits": [{ "op": "replace", "start": h[1], "end": h[1], "lines": [] }] });
        let o = execute("edit", &args, &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "keep\nkeep2\n");
    }

    #[test]
    fn edit_stale_anchor_rejected() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "one\ntwo\n").unwrap();
        let args = json!({ "path": "f.txt", "edits": [{ "op": "replace", "start": "ZZZZ", "end": "ZZZZ", "lines": ["x"] }] });
        let o = execute("edit", &args, &cfg);
        assert!(!o.ok);
        assert!(o.output.contains("stale anchor"), "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn edit_preserves_no_trailing_newline() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "a\nb").unwrap();
        let h = hashes("a\nb");
        let args = json!({ "path": "f.txt", "edits": [{ "op": "append", "pos": h[1], "lines": ["c"] }] });
        let o = execute("edit", &args, &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("f.txt")).unwrap(), "a\nb\nc");
    }

    #[test]
    fn workspace_confines_paths() {
        let (_root, cfg) = tmp_ws();
        // absolute rejected
        let o = execute("read_file", &json!({"path":"/etc/hostname"}), &cfg);
        assert!(!o.ok);
        // .. rejected
        let o = execute("read_file", &json!({"path":"../escape"}), &cfg);
        assert!(!o.ok);
        // inside ok
        fs::write(cfg.workspace.join("inside.txt"), "ok").unwrap();
        let o = execute("read_file", &json!({"path":"inside.txt"}), &cfg);
        assert!(o.ok, "{}", o.output);
    }

    #[test]
    fn read_file_size_guard() {
        let (_root, cfg) = tmp_ws();
        let big = "x".repeat((cfg.max_read_bytes + 100) as usize);
        fs::write(cfg.workspace.join("big.txt"), &big).unwrap();
        let o = execute("read_file", &json!({"path":"big.txt"}), &cfg);
        assert!(!o.ok);
        assert!(o.output.contains("max"), "{}", o.output);
    }

    #[test]
    fn glob_matches_double_star() {
        let (root, cfg) = tmp_ws();
        fs::create_dir_all(root.join("src/a")).unwrap();
        fs::write(root.join("src/a/main.rs"), "x").unwrap();
        fs::write(root.join("src/lib.rs"), "x").unwrap();
        let o = execute("glob", &json!({"pattern":"**/*.rs"}), &cfg);
        assert!(o.ok, "{}", o.output);
        assert!(o.output.contains("main.rs"));
        assert!(o.output.contains("lib.rs"));
    }

    #[test]
    fn grep_finds_matches() {
        let (root, cfg) = tmp_ws();
        fs::write(root.join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
        fs::write(root.join("b.txt"), "beta again\n").unwrap();
        let o = execute("grep", &json!({"pattern":"beta"}), &cfg);
        assert!(o.ok, "{}", o.output);
        assert!(o.output.contains("a.txt:2:beta"));
        assert!(o.output.contains("b.txt:1:beta again"));
    }

    #[tokio::test]
    async fn bash_timeout_kills() {
        let (_root, cfg) = tmp_ws();
        let mut cfg = cfg;
        cfg.bash_timeout_secs = 1;
        let o = execute_bash("sleep 30", &cfg).await;
        assert!(!o.ok);
        assert!(o.output.contains("timed out"), "{}", o.output);
    }

    #[tokio::test]
    async fn bash_denylist_blocks() {
        let (_root, cfg) = tmp_ws();
        let o = execute_bash("rm -rf /", &cfg).await;
        assert!(!o.ok);
        assert!(o.output.contains("denylist"), "{}", o.output);
    }

    #[tokio::test]
    async fn bash_runs_in_workspace() {
        let (root, cfg) = tmp_ws();
        let o = execute_bash("pwd", &cfg).await;
        assert!(o.ok, "{}", o.output);
        // canonicalize both for comparison (tmp may be a symlink)
        assert_eq!(
            std::fs::canonicalize(o.output.trim()).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );
    }

    #[tokio::test]
    async fn bulk_read_write_edit_roundtrip() {
        let (_root, cfg) = tmp_ws();
        // bulk_write three files
        let w = bulk_write(&json!({ "files": [
            { "path": "a.txt", "content": "alpha\nbeta\n" },
            { "path": "sub/b.txt", "content": "one\ntwo\n" },
            { "path": "c.txt", "content": "x\ny\nz\n" }
        ] }), &cfg);
        assert!(w.ok, "{}", w.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("sub/b.txt")).unwrap(), "one\ntwo\n");

        // bulk_read them back; middle file via hashlined content
        let r = bulk_read(&json!({ "paths": ["a.txt","sub/b.txt","nope.txt"] }), &cfg);
        assert!(!r.ok, "per-file error should mark batch not-ok");
        assert!(r.output.contains("alpha"), "{}", r.output);
        assert!(r.output.contains("### [2] nope.txt"), "{}", r.output);

        // bulk_edit: replace line 1 of a.txt, append to c.txt
        let ha = hashes("alpha\nbeta\n");
        let hc = hashes("x\ny\nz\n");
        let e = bulk_edit(&json!({ "edits": [
            { "path": "a.txt", "ops": [{ "op": "replace", "start": ha[0], "end": ha[0], "lines": ["ALPHA"] }] },
            { "path": "c.txt", "ops": [{ "op": "append", "pos": hc[2], "lines": ["END"] }] }
        ] }), &cfg);
        assert!(e.ok, "{}", e.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("a.txt")).unwrap(), "ALPHA\nbeta\n");
        assert_eq!(fs::read_to_string(cfg.workspace.join("c.txt")).unwrap(), "x\ny\nz\nEND\n");
    }

    #[tokio::test]
    async fn bulk_dispatches_bash_and_read() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("f.txt"), "hello\n").unwrap();
        let o = execute_bulk(&json!({ "calls": [
            { "name": "read_file", "args": { "path": "f.txt" } },
            { "name": "bash", "args": { "command": "echo hi" } }
        ] }), &cfg).await;
        assert!(o.ok, "{}", o.output);
        assert!(o.output.contains("hello"), "{}", o.output);
        assert!(o.output.contains("hi"), "{}", o.output);
    }

    #[tokio::test]
    async fn bulk_rejects_nested_bulk() {
        let (_root, cfg) = tmp_ws();
        let o = execute_bulk(&json!({ "calls": [
            { "name": "bulk_read", "args": { "paths": ["f.txt"] } }
        ] }), &cfg).await;
        assert!(!o.ok);
        assert!(o.output.contains("nested bulk"), "{}", o.output);
    }

    #[test]
    fn todo_write_then_read_roundtrip() {
        let (_root, cfg) = tmp_ws();
        let o = execute("todo_write", &json!({ "todos": [
            { "subject": "step 1", "status": "completed" },
            { "subject": "step 2", "status": "in_progress", "content": "detail" }
        ] }), &cfg);
        assert!(o.ok, "{}", o.output);
        let r = execute("todo_read", &json!({}), &cfg);
        assert!(r.ok);
        assert!(r.output.contains("step 1"));
        assert!(r.output.contains("in_progress"));
        // bad status rejected
        let bad = execute("todo_write", &json!({ "todos": [ { "subject": "x", "status": "bogus" } ] }), &cfg);
        assert!(!bad.ok);
    }

    #[test]
    fn patch_applies_unified_diff() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("p.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let diff = "@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n";
        let o = execute("patch", &json!({ "path": "p.txt", "patch": diff }), &cfg);
        assert!(o.ok, "{}", o.output);
        assert_eq!(fs::read_to_string(cfg.workspace.join("p.txt")).unwrap(), "alpha\nBETA\ngamma\n");
    }

    #[test]
    fn patch_rejects_context_mismatch() {
        let (_root, cfg) = tmp_ws();
        fs::write(cfg.workspace.join("p.txt"), "alpha\nbeta\n").unwrap();
        let diff = "@@ -1,2 +1,2 @@\n WRONG\n-beta\n+BETA\n";
        let o = execute("patch", &json!({ "path": "p.txt", "patch": diff }), &cfg);
        assert!(!o.ok);
        assert!(o.output.contains("context mismatch"), "{}", o.output);
    }

    #[test]
    fn read_file_pagination_window() {
        let (_root, cfg) = tmp_ws();
        let body: String = (1..=500).map(|n| format!("line {n}\n")).collect();
        fs::write(cfg.workspace.join("big.txt"), &body).unwrap();
        let o = execute("read_file", &json!({ "path": "big.txt", "offset": 10, "limit": 3 }), &cfg);
        assert!(o.ok, "{}", o.output);
        assert!(o.output.contains("lines 10-12 of 500"), "{}", o.output);
        assert!(o.output.contains("line 10"));
        assert!(o.output.contains("line 12"));
        assert!(!o.output.contains("line 13"));
    }

    #[test]
    fn finish_returns_sentinel() {
        let (_root, cfg) = tmp_ws();
        let o = execute("finish", &json!({}), &cfg);
        assert!(o.ok);
        assert_eq!(o.output, "__finish__");
    }

    #[test]
    fn firejail_profile_whitelists_workspace() {
        let (_root, cfg) = tmp_ws();
        let p = firejail_profile(&cfg.workspace, false);
        assert!(p.contains("whitelist"), "{}", p);
        assert!(p.contains(&cfg.workspace.display().to_string()), "{}", p);
        assert!(p.contains("caps.drop all"));
        assert!(p.contains("seccomp"));
        assert!(p.contains("protocol unix,inet,inet6")); // network allowed
        let pn = firejail_profile(&cfg.workspace, true);
        assert!(pn.contains("net none")); // network dropped
        assert!(!pn.contains("protocol unix"));
    }
}
