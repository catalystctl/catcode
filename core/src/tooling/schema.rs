#[cfg(test)]
use crate::tooling::metadata;
use crate::tools::BASH_TOOL_DESC;
use serde_json::{json, Value};

pub fn is_core_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "edit"
            | "write_file"
            | "delete"
            | "rename"
            | "mkdir"
            | "list_dir"
            | "grep"
            | "glob"
            | "bash"
            | "todo_write"
            | "todo_read"
            | "finish"
            | "memory"
            | "knowledge"
            | "ask"
            | "load_tools"
            | "subagent"
            | "patch"
    )
}

/// Deferred tools — enabled via `load_tools` (or goal planning for goal_write_plan).
pub fn is_deferred_tool(name: &str) -> bool {
    deferred_tool_names().contains(&name)
}

/// Names of built-in tools that are not in the core set.
pub fn deferred_tool_names() -> &'static [&'static str] {
    &[
        "bulk",
        "bulk_read",
        "bulk_write",
        "bulk_edit",
        "goal_write_plan",
        "diagnostics",
        "fetch",
        "web_search",
        "git_status",
        "git_diff",
        "git_log",
        "workspace_activity",
        "git_add",
        "git_commit",
        "spawn",
        "test_env",
        "browser_create",
        "browser_close",
        "browser_list_sessions",
        "browser_navigate",
        "browser_back",
        "browser_reload",
        "browser_snapshot",
        "browser_find",
        "browser_click",
        "browser_fill",
        "browser_type",
        "browser_press",
        "browser_scroll",
        "browser_wait",
        "browser_evaluate",
        "browser_screenshot",
        "browser_show",
        "browser_hide",
    ]
}

/// Whether `name` is a built-in tool (one returned by [`definitions`]).
///
/// Used to ensure a plugin-declared tool that collides with a built-in name
/// can never hijack the built-in's dispatch. The registry merge hides the
/// colliding plugin tool from the model's tool list (the built-in wins), and
/// the dispatch + classify sites guard on `is_builtin` so a call to a built-in
/// name always routes to the built-in handler and classification — never a
/// same-named plugin tool. Derived from `definitions()` (cached in a
/// `OnceLock`) so it can never drift from the real built-in set.
pub fn is_builtin(name: &str) -> bool {
    use std::sync::OnceLock;
    static SET: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let set = SET.get_or_init(|| {
        definitions()
            .into_iter()
            .filter_map(|d| {
                d.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect()
    });
    set.contains(name)
}

/// Built-in tool schemas. Cached in a `OnceLock` — the JSON is large and was
/// previously rebuilt on every turn start. Callers that filter deferred tools
/// clone from this base.
pub fn definitions() -> Vec<Value> {
    use std::sync::OnceLock;
    static DEFS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFS.get_or_init(definitions_uncached).clone()
}

fn definitions_uncached() -> Vec<Value> {
    let mut defs = vec![
        json!({
            "type": "function",
            "function": {
                "name": "subagent",
                "description": "Delegate to a child agent (scout/reviewer/worker/oracle/planner/researcher/context-builder/delegate/custom). Modes: single, parallel (tasks), chain, plus management actions (list/status/interrupt/resume/peek/steer/doctor).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list","get","models","create","update","delete","status","interrupt","resume","peek","steer","doctor"], "description": "management/control action" },
                        "agent": { "type": "string", "description": "agent name for single mode or target for management" },
                        "task": { "type": "string", "description": "task string for single mode" },
                        "model": { "type": "string", "description": "override model for this run" },
                        "tasks": { "type": "array", "description": "parallel tasks: each {agent, task, model?, count?}" },
                        "chain": { "type": "array", "description": "sequential steps: {agent, task, as?, parallel?, concurrency?}" },
                        "concurrency": { "type": "integer", "description": "parallel concurrency (default from config)" },
                        "worktree": { "type": "boolean", "description": "isolate each parallel task in a git worktree under .catalyst-code/worktrees/ (requires a git repo; changes are promoted on success)" },
                        "context": { "type": "string", "enum": ["fresh","fork"], "description": "fresh = clean child; fork = branched from parent" },
                        "async": { "type": "boolean", "description": "background execution" },
                        "id": { "type": "string", "description": "run id for status/interrupt/resume/peek/steer" },
                        "message": { "type": "string", "description": "follow-up for resume, or steering text for steer" },
                        "config": { "type": "object", "description": "agent/chain config for create/update" },
                        "agentScope": { "type": "string", "enum": ["user","project","both"] }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "contact_supervisor",
                "description": "Contact the orchestrator (parent session) that delegated this task. reason 'need_decision' blocks until the orchestrator replies; 'progress_update' is non-blocking. Use for blocking decisions, approvals, or scope ambiguity — not routine completion handoffs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "reason": { "type": "string", "enum": ["need_decision","progress_update"] },
                        "message": { "type": "string" }
                    },
                    "required": ["reason","message"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "intercom",
                "description": "Peer-to-peer coordination between subagents (only available when the setup allows it). action 'send' posts to a peer mailbox (non-blocking); 'ask' posts and blocks for a reply; 'receive'/'poll' reads your mailbox; 'reply' answers a pending ask by id; 'targets' lists known peers.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["send","ask","receive","poll","reply","targets"] },
                        "to": { "type": "string", "description": "recipient target (peer subagent or the orchestrator)" },
                        "message": { "type": "string" },
                        "reason": { "type": "string", "description": "e.g. need_decision, progress_update" },
                        "id": { "type": "string", "description": "ask id being replied to" },
                        "reply": { "type": "string" }
                    },
                    "required": ["action"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file (workspace-relative). Large files auto-window; pass offset/limit to page. Prefer grep to locate first. line_numbers:true for citations only — never copy numbered lines into edit search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "offset": { "type": "integer", "description": "1-indexed start line (pagination)" },
                        "limit": { "type": "integer", "description": "max lines to return" },
                        "line_numbers": { "type": "boolean", "description": "prefix each line with N| for navigation; omit when preparing edit search/replace" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "edit",
                "description": "Search/replace edits on a file. Read first; each search must match exactly and be unique (or set replace_all). Empty replace deletes. normalize_whitespace tolerates indent drift. All edits apply atomically.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "search": { "type": "string", "description": "exact text to find (unique unless replace_all)" },
                                    "replace": { "type": "string", "description": "replacement (empty = delete)" },
                                    "replace_all": { "type": "boolean", "description": "replace every occurrence" },
                                    "normalize_whitespace": { "type": "boolean", "description": "match whitespace-collapsed text" }
                                },
                                "required": ["search", "replace"]
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
                "name": "delete",
                "description": "Delete a file or empty directory (workspace-relative). Refuses non-empty directories — remove contents first. Prefer this over bash rm.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "rename",
                "description": "Rename or move a file/directory within the workspace (creates parent dirs of the destination). Prefer this over bash mv.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string", "description": "existing path" },
                        "to": { "type": "string", "description": "new path" }
                    },
                    "required": ["from", "to"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "mkdir",
                "description": "Create a directory (and parents) at a workspace-relative path. Prefer this over bash mkdir.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
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
                "description": "Search file contents (regex). Default output: path:line:content. Use glob/type to scope, case_insensitive for -i, output_mode files_with_matches|count|content, head_limit/offset to page, context for -C windows.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Rust regex" },
                        "path": { "type": "string", "description": "directory or file to search (relative); default workspace root" },
                        "glob": { "type": "string", "description": "only files matching this glob (e.g. **/*.rs)" },
                        "type": { "type": "string", "description": "language/file-type shortcut (rs, go, py, js, ts, …) — filters by extension" },
                        "case_insensitive": { "type": "boolean", "description": "case-insensitive match (default false)" },
                        "output_mode": {
                            "type": "string",
                            "enum": ["content", "files_with_matches", "count"],
                            "description": "content (default) = matching lines; files_with_matches = paths only; count = path:N per file"
                        },
                        "head_limit": { "type": "integer", "description": "max matches (content) or files (other modes); default 50" },
                        "offset": { "type": "integer", "description": "skip first N matches/files (pagination)" },
                        "context": { "type": "integer", "description": "lines before+after each match (content mode, like grep -C). 0 = match line only." }
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
                "description": BASH_TOOL_DESC,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "timeout": { "type": "integer", "description": "per-call wall-clock timeout in seconds (clamped to [1, max_bash_timeout_secs]; default = the configured bash timeout)" }
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bulk",
                "description": "Batch several independent tool calls in one round-trip (shared approval). Do not wrap a single call. Avoid long quote-heavy commands inside bulk JSON — write a script instead.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "calls": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "name": { "type": "string", "enum": ["read_file","write_file","edit","list_dir","grep","glob","bash","fetch","web_search","delete","rename","mkdir"] },
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
                "description": "Read many files in one call. Returns each file as a headed block with its plain content (same format as read_file). Paths are relative to the workspace root. Per-file errors are reported inline rather than failing the whole call.",
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
                "description": "Apply search/replace edits to many files. Each entry: {path, edits} (same shape as edit). Per-file atomic; failed search fails only that file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "edits": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "search": { "type": "string", "description": "exact text to find (must be unique in the file)" },
                                                "replace": { "type": "string", "description": "replacement text (empty = delete)" },
                                                "replace_all": { "type": "boolean", "description": "replace every occurrence instead of requiring a unique match" },
                                                "normalize_whitespace": { "type": "boolean", "description": "match on whitespace-collapsed text so indentation/spacing drift still lands" }
                                            },
                                            "required": ["search","replace"]
                                        }
                                    }
                                },
                                "required": ["path","edits"]
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
                "name": "goal_write_plan",
                "description": "GOAL MODE ONLY. Submit the structured multi-subagent plan exactly once. Each step becomes a subagent prompt under the goal's concurrency/model caps.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "summary": { "type": "string", "description": "short plan summary" },
                        "steps": {
                            "type": "array",
                            "description": "deployment DAG; keep depends_on empty for independent work so the goal scheduler can fill its concurrency window, and use dependencies only for required ordering",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "agent": { "type": "string", "description": "scout|researcher|planner|worker|reviewer|context-builder|oracle|delegate|custom" },
                                    "title": { "type": "string" },
                                    "task": { "type": "string", "description": "full self-contained prompt for the subagent" },
                                    "model": { "type": "string", "description": "optional model override (must be on the goal allowlist)" },
                                    "depends_on": { "type": "array", "items": { "type": "string" } },
                                    "parallel_group": { "type": "string" }
                                },
                                "required": ["agent", "task"]
                            }
                        },
                        "risks": { "type": "array", "items": { "type": "string" } },
                        "validation": { "type": "array", "items": { "type": "string" }, "description": "how to know the goal succeeded" }
                    },
                    "required": ["summary", "steps"]
                }
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
                "name": "fetch",
                "description": "Fetch a URL over HTTP(S) and return the response body as text (HTML is lightly stripped to text, bounded to the configured max bytes). Unlike bash curl, this is a native tool that still works under --no-network (it is not subject to the bash sandbox). A host allowlist may restrict which domains are reachable; empty allowlist = any host. Use for looking up docs, man pages, or API references. Read-only.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "absolute http(s) URL to fetch" },
                        "raw": { "type": "boolean", "description": "if true, return the raw body without HTML stripping (use for JSON/API responses)" }
                    },
                    "required": ["url"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Web search. Prefers the Exa and Tavily APIs (set EXA_API_KEY / TAVILY_API_KEY) with round-robin load balancing + monthly quota tracking; with both keys it alternates and cooldowns on rate limits. Falls back to public SearXNG instances + DDG/Mojeek scrapes when no key is set or all API providers are exhausted. Returns top hits as text. Honors --no-network / fetch_allowlist. Pair with fetch to read a page.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "search query" },
                        "count": { "type": "integer", "description": "max results to return (default 8, clamped 1-20)" },
                        "region": { "type": "string", "description": "DDG region/locale code, e.g. us-en (default) or uk-en" }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Show the working-tree status (staged, unstaged, untracked) as `git status --short --branch`. Optional relative `path` limits the scope. Read-only.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "optional relative path to scope the status" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "Show unstaged changes (`git diff --no-color`) or staged changes with staged:true. Optional relative `path` scopes the diff. Read-only.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "optional relative path to scope the diff" },
                        "staged": { "type": "boolean", "description": "if true, show staged (--cached) changes" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_log",
                "description": "Show recent commit history as `git log --oneline -n <limit>`. Optional relative `path` limits to a file's history. Read-only.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "max commits to show (default 20)" },
                        "path": { "type": "string", "description": "optional relative path to filter history" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "workspace_activity",
                "description": "List OTHER active catalyst-code agent sessions running in THIS workspace (separate processes), with each one's goal, what it's working on, and the files it recently touched. Use this when something seems off (a build failing for reasons you didn't cause, a file that changed unexpectedly, a test suddenly breaking) to check whether another session is the cause before assuming you introduced the error. Read-only — awareness only, no coordination. Returns the live peers (stale/crashed sessions are auto-pruned).",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_add",
                "description": "Stage files for commit (`git add -- <paths>`). Paths must be workspace-relative; absolute paths and `..` escapes are rejected. Destructive (modifies the index).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" }, "description": "relative paths to stage" }
                    },
                    "required": ["paths"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "Create a commit (`git commit -m <message>`). By default commits only already-staged changes; pass all:true to also stage modified tracked files first (does NOT add untracked files). Destructive.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "commit message" },
                        "all": { "type": "boolean", "description": "if true, stage modified tracked files before committing (git commit --all)" }
                    },
                    "required": ["message"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "memory",
                "description": "Persist/list/get/forget durable memories (workspace default; scope:global for cross-project). Standing prompt carries a capped catalog (name+one-line); use get for full text. Prefer append over new saves; require a short description. Rejects trivia unless force=true. Use consolidate to merge near-duplicates; stats for recall quality.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["save", "append", "list", "get", "forget", "consolidate", "stats", "deprecate", "migrate"], "description": "save/append/list/get/forget; consolidate merges near-duplicates; stats shows recall hit/miss + synonym-miss rates; deprecate marks a memory superseded (excluded from catalog/relevant surfaces); migrate rewrites stale project-name refs (umans-harness→catalyst-code, idempotent)" },
                        "scope": { "type": "string", "enum": ["workspace", "global"], "description": "where the memory lives: 'workspace' (per-codebase, default) or 'global' (cross-codebase). For list/get/forget, omit to search both scopes" },
                        "name": { "type": "string", "description": "(save/append) short memory name; becomes the file slug and the id. append looks up the same name to accumulate onto" },
                        "content": { "type": "string", "description": "(save/append) the memory body (save) or the facts to append (append)" },
                        "type": { "type": "string", "description": "(save/append) memory type, e.g. note/convention/decision/user (default note). user/identity/preference are pinned in the catalog; convention/decision are NOT auto-pinned (pin explicitly via pin:true or importance:high)" },
                        "description": { "type": "string", "description": "(save/append) one-line summary shown in the standing catalog (auto-filled from the first content line if omitted)" },
                        "importance": { "type": "string", "enum": ["high", "normal", "low"], "description": "(save/append) durability hint; high preferred in catalog; low rejected unless force=true" },
                        "force": { "type": "boolean", "description": "(save/append) override trivia/conflict write policy when intentional" },
                        "id": { "type": "string", "description": "(get/forget/deprecate) the memory id (slug or name)" },
                        "replaces": { "type": "string", "description": "(save) name/id of a memory this one supersedes — marks it deprecated so it's excluded from the catalog + relevant tail. Use to resolve contradictions: save the corrected memory with replaces=<stale-name>" },
                        "superseded_by": { "type": "string", "description": "(deprecate) name of the memory that supersedes the one being deprecated (optional)" }
                    },
                    "required": ["action"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "knowledge",
                "description": "Read-only codebase intelligence queries (offline). Actions: context (task context pack), search (hybrid memory rank), symbol, related (imports/coupling/memories), tests, episodes, preferences, rejected, coverage, explain (why a memory ranked).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["context", "search", "symbol", "related", "tests", "episodes", "preferences", "rejected", "coverage", "explain"], "description": "knowledge query kind" },
                        "query": { "type": "string", "description": "search/context prompt text" },
                        "prompt": { "type": "string", "description": "alias of query for context" },
                        "path": { "type": "string", "description": "related/tests: workspace-relative path" },
                        "name": { "type": "string", "description": "symbol/explain: symbol or memory name" },
                        "limit": { "type": "integer", "description": "max results (default varies by action)" }
                    },
                    "required": ["action"]
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
        json!({
            "type": "function",
            "function": {
                "name": "ask",
                "description": "Ask the user structured questions and wait for answers. REQUIRED when the request is ambiguous, missing information you cannot find yourself, or a decision has real trade-offs or destructive outcomes. Do NOT ask about things you can determine from the workspace — check first. User may skip optional questions or dismiss the prompt.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "minItems": 1,
                            "description": "Questions in order; each is a flyout field.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "description": "stable id; answers keyed by this" },
                                    "prompt": { "type": "string", "description": "question text" },
                                    "type": { "type": "string", "enum": ["select", "text"], "description": "select = options; text = free input" },
                                    "options": {
                                        "type": "array",
                                        "items": { "type": "string" },
                                        "description": "required for select"
                                    },
                                    "allowCustom": { "type": "boolean", "description": "select: allow typed custom answer" },
                                    "placeholder": { "type": "string", "description": "text: input placeholder" },
                                    "required": { "type": "boolean", "description": "if false, may skip (default true)" }
                                },
                                "required": ["id", "prompt", "type"]
                            }
                        }
                    },
                    "required": ["questions"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "load_tools",
                "description": "Enable deferred tools for this session (schemas not sent until loaded). Pass tools:[...] or tool:\"name\". Groups: all, git, web, bulk, browser. Deferred: bulk*, git_*, fetch, web_search, diagnostics, spawn, workspace_activity, test_env, browser_*. (goal_write_plan is planning-phase only — not loadable.)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "tools": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "tool names or groups (all|git|web|bulk|browser)"
                        },
                        "tool": { "type": "string", "description": "single tool name or group" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "test_env",
                "description": "Spin up and drive ephemeral Linux containers / Windows VMs for platform-specific testing, with VNC screen access. Linux uses a Podman container (catalyst/linux-gui image); Windows uses a QEMU/KVM VM cloned from a base qcow2 (built via packaging/vm-images/windows/build.sh). Actions: create (platform linux|windows; optional image/gui/cpus/memory_mb) → returns env_id + vnc_url; exec (run a command inside the env — SSH+PTY for Windows, podman exec for Linux; ideal for TUI tests); screenshot (capture the screen to a PNG file + metadata); input (key/click/type — drive the GUI); vnc_url (live-screen websocket for the noVNC panel); destroy; list. For TUI tests use exec; for webui/GUI tests use screenshot/input or the vnc_url panel.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create","exec","screenshot","input","vnc_url","destroy","list"] },
                        "env_id": { "type": "string", "description": "env id (from create); required for exec/screenshot/input/vnc_url/destroy" },
                        "platform": { "type": "string", "enum": ["linux","windows"], "description": "create only; default linux" },
                        "image": { "type": "string", "description": "create only; container image (linux) — default catalyst/linux-gui:24.04" },
                        "gui": { "type": "boolean", "description": "create only; start the GUI/VNC stack (default true for linux)" },
                        "cpus": { "type": "integer", "description": "create only; Windows VM vCPUs (default 4)" },
                        "memory_mb": { "type": "integer", "description": "create only; Windows VM RAM in MB (default 4096)" },
                        "command": { "type": "string", "description": "exec: the command to run inside the env" },
                        "pty": { "type": "boolean", "description": "exec: allocate a PTY (needed for TUIs on Windows; default false)" },
                        "timeout": { "type": "integer", "description": "exec: wall-clock timeout in seconds (default 120)" },
                        "input_type": { "type": "string", "enum": ["key","click","type"], "description": "input action type" },
                        "keys": { "type": "array", "items": { "type": "string" }, "description": "input key: key names (Windows QKeyCode / Linux xdotool syntax)" },
                        "x": { "type": "integer", "description": "input click: x coordinate (in the env's screen resolution)" },
                        "y": { "type": "integer", "description": "input click: y coordinate" },
                        "text": { "type": "string", "description": "input type: text to type" }
                    },
                    "required": ["action"]
                }
            }
        }),
    ];
    defs.extend(crate::browser::definitions());
    defs
}

#[cfg(test)]
mod metadata_invariant_tests {
    #[test]
    fn every_builtin_definition_has_policy_metadata() {
        let missing: Vec<String> = super::definitions()
            .iter()
            .filter_map(|definition| {
                definition
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(|name| name.as_str())
            })
            .filter(|name| super::metadata(name).is_none())
            .map(str::to_string)
            .collect();
        assert!(
            missing.is_empty(),
            "tools without policy metadata: {missing:?}"
        );
    }
}
