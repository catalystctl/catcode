# Architecture Overview

Catalyst Code is a **self-hosted, OpenAI-compatible coding-agent harness** built
as four cooperating components around a single newline-delimited JSON protocol
over stdio. This document describes the component boundaries, communication
model, core subsystems, security model, configuration flow, and plugin
architecture.

---

## Four-Component Architecture

| Component | Language | Role | Entry Point |
|-----------|----------|------|-------------|
| **`core/`** | Rust (async, tokio) | The engine: conversation, model streaming, the agentic tool loop with an approval gate, sessions, memory, plugins, subagents, self-learning | `core/src/main.rs` (~10.4k lines) |
| **`tui/`** | Go ([Bubble Tea](https://github.com/charmbracelet/bubbletea)) | Terminal UI (`catcode` binary). Spawns `core` as a subprocess, writes commands to stdin, reads events from stdout, renders approvals and metrics | `tui/main.go` + `tui/handlers.go` |
| **`web/`** | Next.js 15 + React 19 | Browser equivalent of the TUI — an SSE bridge to one core process. Prebuilt as a standalone tarball (no `next build` on the host) | `web/` (see `web/README.md`) |
| **`sdk/`** | TypeScript | Thin pi-compatible wrapper (`@catalyst-code/coding-agent`) so the web frontend and other JS consumers can drive the core | `sdk/` |

**Source:** CONTRIBUTING.md (/CONTRIBUTING.md), README.md (/README.md#architecture)

### Process Model

The TUI and web frontend each **spawn one `core` process per session**. The
frontend is a `catcode` Go binary (TUI) or a Next.js Node/Bun service (web).
Each core process is a long-lived tokio async runtime that reads commands from
stdin and writes events to stdout. When using the embedded build
(`-tags embed_core`), the core binary is compiled into the TUI binary itself;
otherwise it is found on `PATH` or at `CATCODE_CORE`.

```
┌──────────┐   stdin (JSONL Commands)   ┌──────────┐
│ Frontend │ ────────────────────────── │   core   │
│ (TUI/web)│                           │ (Rust)   │
│          │ ── stdout (JSONL Events) ──│          │
└──────────┘                           └──────────┘
```

---

## Wire Protocol

Components communicate over **newline-delimited JSON on stdio**. One JSON
object per line, UTF-8 encoded.

- **stdin:** Frontend writes `Command` (/core/src/protocol.rs) objects tagged by `type`.
- **stdout:** Core writes `Event` (/core/src/protocol.rs) objects with a `type` field and flat data.

**Commands** include `init`, `send`, `steer`, `approve`, `set_key`, `login`,
`logout`, `list_provider_presets`, `oauth_code`, `remember`, `forget`,
`load_tools`, `cancel_goal`, and more.

**Events** include `ready`, `authed`, `thinking`, `delta`, `tool_call_start`,
`tool_call`, `approval_request`, `tool_result`, `compacted`, `http_retry`,
`metrics`, `approval_changed`, `done`, `aborted`, `reset`, `error`, and
subagent/memory/session lifecycle events.

Full reference: [`docs/architecture/protocol.md`](protocol.md).

**Source:** `core/src/protocol.rs` (462 lines), `README.md` wire protocol section.

---

## Core Subsystems

The core (`core/src/`) is a flat module layout — each `.rs` file is a distinct
subsystem. Key subsystems:

### Turn Loop (`main.rs`, ~10k lines)

The async stdin command loop reads `Command` objects, dispatches each to the
appropriate handler, and emits `Event` objects. The main loop is:

1. Read command from stdin
2. Match on command type (`send`, `steer`, `approve`, `set_key`, `login`, …)
3. For `send`: call `stream_turn` — the model streaming loop with tool dispatch
4. For `steer`: inject a human message and continue the running turn
5. For `approve` / `deny`: resolve the pending approval request and continue
6. Handle compaction, memory operations, goal phases, and subagent management

The `run_turn` / `stream_turn` functions implement the agentic tool loop:
stream model tokens, detect tool calls, run the approval gate, execute tools
(sequentially for writes, concurrently for reads), inject results, and repeat
until `finish` is called or the token budget is exhausted.

**Source:** `core/src/main.rs`

### Tool Dispatch (`tools.rs`, 5922 lines)

Every tool is defined as an OpenAI-compatible function-calling schema and
executed by name. Tools are classified as:

- **Core tools** (always available): `read_file`, `edit`, `grep`, `glob`,
  `list_dir`, `bash`, `write_file`, `memory`, `subagent`, `finish`, `patch`,
  `todo_read`, `todo_write`, `load_tools`
- **Deferred tools** (load via `load_tools`): `git_*`, `fetch`, `web_search`,
  `bulk`, `diagnostics`, `spawn`, `workspace_activity`, `test_env`, `browser`
- **Async dispatch** (sentinel errors, dispatched by main loop): `fetch`,
  `web_search`, `browser/*`, `spawn`, `subagent`, `contact_supervisor`,
  `intercom`, `ask`

`is_parallel_wave_tool()` distinguishes readonly tools (safe for concurrent
execution) from mutating tools (sequential after approval gate).

**Source:** `core/src/tools.rs`

### Provider Abstraction (`provider.rs`, 7850 lines)

Abstracts OpenAI-compatible and Anthropic-compatible endpoints. Handles:

- Chat completion streaming over HTTP
- Retry with exponential backoff and jitter
- Model discovery via `/models` and `/models/info`
- `reasoning_effort` / `thinking` parameter mapping
- `reasoning_content` field extraction (OpenAI-style reasoning)
- Non-standard discovery fallback

Providers are configured declaratively: `kind` (`openai` or `anthropic`), `base_url`,
`api_key_env` (env var name, preferred) or `api_key` (literal). Multi-provider
routing: each turn routes to the selected model's provider at runtime.

**Source:** `core/src/provider.rs`

### Session Persistence (`session.rs`, 611 lines)

Append-only JSONL conversation log. Design:

- Schema-version header line (`{"_session_version": 1}`) for forward migration
- Each finalized message is appended and fsync'd at turn end
- Crash mid-task loses at most the in-flight turn
- `load()` replays existing sessions on startup, rejecting future versions
- Session files live under `~/.config/catalyst-code/sessions/`

**Source:** `core/src/session.rs`

### Memory Store (`memory.rs`, 2765 lines)

Persistent markdown-file memory store under
`~/.config/catalyst-code/memory/<project-hash>/`. Features:

- Named memories with YAML frontmatter (type, importance, scope)
- Per-workspace hashed canonical path isolation
- Write serialization (global mutex prevents parallel subagent races)
- Scan cache invalidated on write; `memory_injection()` splices memories into
  system prompt
- Self-learning modules: `embed.rs`, `learning_*.rs`, `memory_recall.rs`,
  `memory_hygiene.rs`, `memory_staleness.rs`

**Source:** `core/src/memory.rs`, `core/src/embed.rs`, `core/src/learning_*.rs`

### Subagent Orchestration (`subagent.rs`, 3662 lines)

Nested agentic loops that share the workspace, tools, and API key but run with
a focused system prompt and optional tool allowlist. Three execution modes:

- **Single:** one agent + task
- **Parallel:** tasks array with configurable concurrency, optional git worktree
  isolation
- **Chain:** sequential steps with `{previous}` output substitution

Eight built-in agents: `scout`, `researcher`, `planner`, `worker`, `reviewer`,
`context-builder`, `oracle`, `delegate`. Custom agents from
`.catalyst-code/agents/*.md` with YAML frontmatter.

Cross-agent coordination via `intercom.rs` (in-process mailboxes).

Full guide: [`docs/guides/subagents.md`](../guides/subagents.md).

**Source:** `core/src/subagent.rs`, `core/src/intercom.rs`

### Goal Mode (`goal.rs`, 2473 lines)

First-class plan-then-deploy orchestration via `/goal`. Phase machine:

```
idle → planning → plan_ready (optional) → deploying → running → synthesizing → done|failed
```

The planning turn must call `goal_write_plan` with a structured plan. Deploy
runs subagents under user-specified concurrency and model/provider caps. After
workers finish, a parent synthesizing turn reports results.

**Source:** `core/src/goal.rs`

### Plugin Hooks (`plugins.rs`, 5155 lines)

Subprocess-based plugin system. Each plugin is a directory with `plugin.json`
and hook scripts. Hooks are spawned as subprocesses with JSON context on stdin
and JSON response on stdout. Hook points include:

| Category | Hook Points | Fail Behaviour |
|----------|-------------|----------------|
| Pre-operation (blocking) | `pre_bash`, `pre_write`, `pre_read`, `pre_tool`, `pre_input` | Deny on failure |
| Post-operation (best-effort) | `post_bash`, `post_write`, `post_read`, `post_tool` | Skip on failure |
| Lifecycle (advisory) | `session_start`, `session_stop`, `pre_compact`, `pre_turn`, `turn_start`, `turn_end`, `session_shutdown`, `pre_agent_start`, `pre_context` | Log on failure |

Plugins can declare custom tools, OAuth providers, memory backends, and slash
commands. Plugins from a repo load only with `--trust-project-plugins`.

**Source:** `core/src/plugins.rs`

### Self-Learning

Harness-native learning system that extracts durable facts from conversations:

- **Memory store** (`memory.rs`) — persistent markdown files with frontmatter
- **Embedding-sketch recall** (`embed.rs`) — hash-based memory retrieval
- **Learning activations/proposals/retrieval** �� automatic fact extraction
- **Memory hygiene/staleness** — consolidation and decay
- **Codebase indexing** (`codebase_index.rs`) — repository structure awareness
- **Failure atlas** (`failure_atlas.rs`) �� persistent error patterns
- **Pattern logging** (`pattern_log.rs`) — telemetry for reflection

Designed in: `docs/SELF_LEARNING.md`

**Source:** `core/src/{embed,learning_*,memory_recall,memory_hygiene,memory_staleness,codebase_index,failure_atlas,pattern_log,skill_metrics}.rs`

### Other Subsystems

| Module | Purpose |
|--------|---------|
| `workspace.rs` | Path confinement (absolute/`..`/symlink rejection) |
| `worktree.rs` | Git worktree isolation for parallel subagents |
| `checkpoint.rs` | Hybrid FS checkpoints (git stash or file snapshot) |
| `audit.rs` | Optional security audit sidecar |
| `git_ctx.rs` | Git context injection into system prompt |
| `vision.rs` | Vision model config + image attachment |
| `fetch_tool.rs` | HTTP fetch tool (egress-controlled) |
| `oauth.rs` | Plugin OAuth plumbing (loopback, enrich) |
| `staging.rs` | Global default-file staging (`~/.catalyst-code/`) |
| `fsutil.rs` | Shared filesystem utilities |
| `browser/` | Native WRY browser integration |

**Source:** Each module in `core/src/`

---

## Data Flow

A typical user turn flows through the system as follows:

```
User input (TUI/web)
  │
  ▼
stdin: {"type":"send","prompt":"...","model":"...","reasoning_effort":"high"}
  │
  ▼
core/main.rs: stream_turn()
  │
  ├─► Build system prompt (base + git context + memory + plugins + skills)
  ├─► Call provider (provider.rs): stream chat completion
  ├��► Emit delta events (thinking + text tokens)
  ├─► On tool_call:
  │     Emit tool_call_start, tool_call_name, tool_call_args, tool_call
  │     ���─► Approval gate (config.rs Approval): prompt user if destructive
  │     └─► Execute tool (tools.rs dispatch)
  │           ├─► Sequential (writes, bash, subagent, ask)
  │           └─► Parallel wave (reads, grep, glob, memory)
  │         Inject result as tool_result event
  │         Repeat until finish or budget exhausted
  │
  └─► Emit done event
        Session append + fsync (session.rs)
        Memory auto-append (memory.rs)
```

**Subagent data flow** adds a nesting layer:

```
Parent agent loop
  │
  └─► subagent tool → run_agent() in subagent.rs
        ├─► Create forked/fresh context
        ├─► Launch nested agent loop (same model + tools, filtered by agent config)
        ├─► Subagent may contact_supervisor (blocks parent)
        ├─► Subagent may intercom with peers
        └─► Return result to parent
```

---

## Security Boundaries

### Workspace Confinement (`workspace.rs`)

Every file tool resolves paths against a workspace root and rejects:

- Absolute paths (e.g. `/etc/passwd`)
- Parent traversal (`../`)
- Symlink escapes (symlinks pointing outside the workspace)

Under `Approval::Never`, all confinement checks are bypassed (trust-the-model
mode). Otherwise, the confinement is unconditional.

### Restricted Paths

The following paths are gated (require approval for reads AND writes):

- `.git/**` — VCS internals
- `**/.bashrc`, `**/.bash_profile`, `**/.profile`, `**/.zshrc` — shell config
- `**/.ssh/**`, `**/.gnupg/**` — SSH/GPG secrets
- `**/id_rsa`, `**/id_ed25519` — private keys
- `**/.env`, `**/.env.local`, `**/.env.production` — env files

Restricted paths are **not enforced** under `Approval::Never`.

### Approval Gate

Three modes, settable per session:

| Mode | Behavior |
|------|----------|
| `never` | Auto-approve everything (trust the model fully, no prompts) |
| `destructive` | Ask for `bash`, `write_file`, `edit` (default) |
| `always` | Ask for every tool call |

The approval request surfaces as a TUI modal or web dialog. The user can reply
`yes`, `no`, or `always` (remembers decision for that tool in the session).

### Optional Sandbox

- **Linux:** `--sandbox firejail` — runs `bash` inside Firejail
- **macOS:** `--sandbox seatbelt` — seatbelt sandbox for bash
- **Windows:** denylist-only (no sandbox)
- `--no-network` — blocks network access for bash/fetch
- Sandboxing is **Linux-only** for hard security; the denylist is a tripwire

### Plugin Trust

Plugins from `.catalyst-code/plugins/` (repo-scoped) load only with an explicit
`--trust-project-plugins` CLI flag. The flag is never read from a config file
the repo could ship. Plugin scripts run as subprocesses with a timeout
(pre_*: 5s, post_*: 30s default). Broken hooks never crash the core.

**Sources:** `core/src/workspace.rs`, `core/src/config.rs` (Approval enum,
`strip_untrusted_keys`), `core/src/plugins.rs`, README security notes.

---

## Configuration Flow

Configuration uses an **8-layer precedence** (later overrides earlier):

| # | Source | Scope | Trusted |
|---|--------|-------|---------|
| 1 | Built-in defaults (`Config::default()`) | Code | Yes |
| 2 | `managed-settings.json` (`~/.config/catalyst-code/config.json`) | Managed | Yes |
| 3 | `managed-settings.d/*.json` | Managed directory | Yes |
| 4 | `~/.config/catalyst-code/settings.json` | User | Yes |
| 5 | `<workspace>/settings.json` | Project | **No** (stripped) |
| 6 | `<workspace>/settings.local.json` | Project local | **No** (stripped) |
| 7 | Environment variables (`CATALYST_CODE_*`, `UMANS_*`) | Process | Yes |
| 8 | CLI flags (`--approval`, `--sandbox`, `--base-url`, …) | Invocation | Yes |

**Security stripping:** Project-scoped files (5, 6) have security-sensitive keys
removed: `approval`, `sandbox`, `no_network`, `fetch_allowlist`, `bash_deny`,
`providers`, `provider_keys`, `search_keys`, `activeProvider`. These can only
be set via env vars, CLI, or user-owned config files. Per-project settings for
model, theme, timeout, and subagent preferences are allowed.

**Array/object merge:** Arrays concatenate and deduplicate; objects deep merge;
`null` values delete keys.

The config is hand-rolled (no clap, no toml, no serde for loading) — defined
entirely in `core/src/config.rs` (2107 lines).

**Source:** `core/src/config.rs`

---

## Plugin System Architecture

Plugins extend the harness without modifying core code. They are loaded from
`.catalyst-code/plugins/<name>/` directories.

### Plugin Structure

```
plugins/
  <name>/
    plugin.json      # manifest (name, version, hooks, tools, oauth, commands, memory)
    hooks/
      pre_bash.sh    # hook script (any executable)
      pre_write.sh
      ...
    tools/
      my-tool.py     # custom tool definition + execution script
```

### Hook Execution Model

1. Core loads `plugin.json` from each enabled plugin directory
2. For each hook point, core spawns the hook script as a subprocess
3. JSON context is written to stdin of the hook process
4. Hook writes JSON response to stdout
5. Core interprets response: `allow: false` blocks (pre_*), `modify` keys can
   alter tool arguments, system prompt, or context
6. Timeouts and parse failures are graceful — pre_* hooks deny the operation,
   post_* hooks are silently skipped

### Hook Points

See the [plugin hooks table](#plugin-hooks) above. Full contract in the
`plugin-authoring` skill.

### Custom Tools

Plugins can declare custom tools that appear in the model's function-calling
schema. The plugin script receives the tool call arguments and returns results.
No MCP required — hooks are the substrate.

### OAuth Providers

Plugins can declare an `oauth` block in `plugin.json`, enabling subscription
logins (ChatGPT Plus/Pro, Claude Pro/Max, SuperGrok, etc.). The harness owns
`/login` / `/oauth-code` loopback + paste UX; the plugin script owns the
authorize/token/refresh logic.

**Source:** `core/src/plugins.rs` (5155 lines), `core/src/oauth.rs`,
`.catalyst-code/plugins/` directory, `docs/examples/plugins/` directory.

---

## Component Summary Table

| Component | Language | Framework | Lines (approx) | Config | Entry Point |
|-----------|----------|-----------|-----------------|--------|-------------|
| **core** | Rust | tokio async | ~63k (54 `.rs`) | CLI + env + JSON | `main.rs` |
| **tui** | Go | Bubble Tea v2 | ~10k | `settings.json` via core | `main.go` |
| **web** | TypeScript/TSX | Next.js 15, React 19 | ~5k | `.env.local` | `app/page.tsx` |
| **sdk** | TypeScript | — | ~2k | — | `src/index.ts` |

All four components are in the same monorepo. The core is the single source of
truth for agent behavior — the frontends are wire-compatible views into the
same engine, and the SDK is a thin wrapper for programmatic access.

---

## See Also

- [Wire Protocol Reference](protocol.md) — full command/event schemas
- [Configuration Reference](../configuration/index.md) — all config keys
- [Subagent Guide](../guides/subagents.md) — subagent orchestration
- [Plugin Examples](../examples/plugins/README.md) — plugin authoring examples
- [Self-Learning Design](../SELF_LEARNING.md) — memory and learning
- [Getting Started](../quickstart.md) — first run
