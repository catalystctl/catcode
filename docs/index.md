# Catalyst Code Documentation

Catalyst Code is a **self-hosted, OpenAI-compatible coding-agent harness** — one
binary, any provider, with a human-in-the-loop approval gate. Run it in the
terminal, the browser, or from your own code.

- **Multi-provider** — log into Umans, OpenCode Go, OpenRouter, or a plugin
  OAuth (ChatGPT, SuperGrok, …) and route any turn to any model.
- **Safe by default** — workspace confinement, destructive-tool approval gate,
  restricted-path protection, optional cross-platform microVM sandbox via
  Microsandbox (Linux KVM, Apple Silicon macOS, Windows WHP).
- **Pluggable** — hooks (pre/post/lifecycle/pre_turn), custom tools, custom
  slash commands, custom memory backends, plugin-declared OAuth — no recompile.
- **Subagents + intercom** — delegate to focused child agents (`scout`,
  `planner`, `worker`, …) over single/parallel/chain execution with a peer
  intercom bus.
- **Self-learning** — embedded memory store, skill library, telemetry-backed
  reflection loop.

---

## Getting Started

| Guide | What you'll do |
|-------|----------------|
| [Installation](installation.md) | Install via one-liner (prebuilt, no compiler) |
| [Quickstart](quickstart.md) | Run `catcode`, log in, send your first prompt |
| [Web frontend](../README.md#web-frontend) | Install and open the browser UI |
| [Windows setup](../README.md#windows-tui--optional-web) | PowerShell install, MSI, standalone .exe |
| [Updating](../README.md#updating) | `catcode --update` or Settings → About → Update |

> **Prerequisites:** Linux / macOS / Windows. `curl` + coreutils. No compiler
> unless building from source. Web frontend needs Node or Bun to *run* the
> service (not to build it). To build from source: Rust (stable) + Go 1.25+.

---

## Components

The harness is four cooperating components around one [stdio JSONL protocol](../README.md#wire-protocol).

### `core/` — Rust engine

The single source of truth: conversation management, model streaming (OpenAI +
Anthropic), the agentic tool loop with approval gate, session persistence,
memory, plugins, subagents, and configuration.

[Source: `core/src/`](../core/src)

**Key modules:**

| Module | Role |
|--------|------|
| `main.rs` | Entry, turn loop, approval gate, compaction |
| `config.rs` | CLI flags + env + JSON config layering |
| `protocol.rs` | Wire types (Command / Event) |
| `provider.rs` | OpenAI / Anthropic streaming, retry/backoff |
| `tools.rs` | Tool schemas, classification, execution |
| `plugins.rs` | Plugin manager + hook system |
| `subagent.rs` | Subagent execution (single/parallel/chain) |
| `memory.rs` | Persistent memory store |
| `session.rs` | Append-only JSONL session persistence |
| `workspace.rs` | Path confinement (absolute/../symlink rejection) |
| `intercom.rs` | Peer intercom bus |
| `checkpoint.rs` | Hybrid filesystem checkpoints |
| `goal.rs` | Goal mode orchestration |
| `oauth.rs` | Plugin OAuth plumbing |
| `fetch_tool.rs` | HTTP fetch (egress-controlled) |
| `vision.rs` | Vision model config + image attachment |
| `git_ctx.rs` | Git status/branch context |
| `logging.rs` | JSONL debug log + token estimation |
| `staging.rs` | Global default-file staging |
| `embed.rs` | Hashing-sketch memory recall |
| `audit.rs` | Optional security audit sidecar |
| `worktree.rs` | Git worktree isolation for parallel subagents |
| `change_coupling.rs` | Change coupling analysis |

[Architecture docs](architecture/) · [CLI reference](commands/) · [Tool reference](tools/) · [Configuration reference](configuration/)

### `tui/` — Go terminal UI

The `catcode` binary. Built with [Bubble Tea](https://github.com/charmbracelet/bubbletea)
v2. Spawns the Rust core, streams its events, renders streaming markdown,
approval prompts, metrics, and a multi-panel interface.

[Source: `tui/`](../tui)

- `/login`, `/model`, `/approval`, `/goal`, `/settings`, `/help` — all TUI commands
- Streaming markdown with incremental render
- Approval gate prompts (yes/no/always)
- Session list, checkpoint restore
- Performance surface map: [`tui-perf-surface.md`](tui-perf-surface.md)

### `sdk/` — TypeScript wrapper

`@catalyst-code/coding-agent` — a thin pi-coding-agent-compatible adapter.
Spawns the Rust core, translates the JSONL protocol, and exposes the same API
surface that pi-web expects.

[Source: `sdk/`](../sdk) · [`sdk/README.md`](../sdk/README.md)

```ts
// Quick start
import { AuthStorage, ModelRegistry, createAgentSessionRuntime } from "@catalyst-code/coding-agent";
const runtime = await createAgentSessionRuntime(factory, { cwd, agentDir });
await runtime.session.prompt("explain this repo");
```

### `web/` — Next.js frontend

A browser equivalent of the TUI. The Next.js server spawns one `catcode-core`
process via the SDK and streams events to the browser over SSE (Server-Sent
Events). Fully prebuilt — no `next build` on the host.

[Source: `web/`](../web) · [`web/README.md`](../web/README.md)

```
Browser ──SSE──▶ /api/stream ──▶ HarnessBridge ──stdio JSONL──▶ catcode-core
Browser ──POST─▶ /api/command ──▶ HarnessBridge ──────── (stdin)
```

- Streaming markdown with reasoning, tool calls, approvals, metrics
- Session management and checkpoint restore
- IDE panels (file explorer, editor, terminal, git, preview)
- Contract: [`IDE_PANELS_CONTRACT.md`](IDE_PANELS_CONTRACT.md)

---

## Using the Harness

### CLI Reference

The `catcode` binary (TUI) and `catcode-core` (headless engine) share
configuration through the core's config layer: CLI flags > environment variables
> JSON settings files.

- [`catcode` CLI flags](commands/#catcode) — workspace, sandbox, approval mode, model, provider
- [`catcode-core` CLI flags](commands/#catcode-core) — headless core flags
- [Settings files](configuration/) — layering, paths, managed settings
- [Environment variables](configuration/index.md) — env vars and CLI flags
  `UMANS_API_KEY`, `OPENROUTER_API_KEY`, `CATALYST_CODE_WORKSPACE`, and more

### Slash Commands

Enter these in the TUI chat prompt or the web UI:

| Command | Action |
|---------|--------|
| `/login` | Open the provider picker (or `/login <preset> api_key`) |
| `/logout` | Log out of a provider |
| `/model [N\|substr]` | List models; switch to one by index or substring |
| `/approval [mode]` | Set approval mode: `never` · `destructive` · `always` |
| `/goal` | Open goal-mode modal |
| `/cancel-goal` | Abort the current goal |
| `/settings` | Open the settings hub |
| `/theme` | Choose a theme |
| `/sandbox` | Set sandbox mode (`none` · `microsandbox`) |
| `/help` | List all available commands |
| `/skill:<name>` | Apply a skill |
| `/plugin-install <source>` | Install a plugin |
| `/plugin-reload` | Reload all plugins after edits |
| `/plugin-list` | List installed plugins |
| `/plugin-disable` | Disable a plugin |
| `/usage` | Show token/model usage (Cursor plugin) |

**Bash integration:** `!command` runs bash and adds output to model context;
`!!command` runs without adding it (PI-compatible).

### Tools

The core exposes a rich toolset. Every tool is gated by the [approval
system](../README.md#providers-and-login) and [workspace
confinement](../README.md#architecture).

| Tool | Purpose | Category |
|------|---------|----------|
| `edit` | Targeted search/replace edits on files | Write |
| `write_file` | Create or overwrite files | Write |
| `patch` | Apply unified diffs | Write |
| `grep` | Regex search within files | Read |
| `glob` | File discovery by glob pattern | Read |
| `bash` | Run shell commands (async) | Write (needs approval) |
| `read_file` | Read file contents | Read |
| `diagnostics` | Run cargo/tsc/go/py diagnostics | Read |
| `fetch` | HTTP GET (egress-controlled) | Read |
| `todo` | Task tracking (read/write) | Internal |
| `memory` | Read/write persistent memory | Internal |
| `git_commit` / `git_add` | Git operations | Write |
| `git_diff` / `git_status` | Git inspection | Read |
| `spawn` | Start a child subprocess (sandboxed) | Write |
| `subagent` | Delegate to a focused child agent | Orchestration |
| `contact_supervisor` | Ask the orchestrator a blocking question | Orchestration |
| `intercom` | Peer-to-peer messaging between subagents | Orchestration |
| `finish` | Signal completion (end turn) | Orchestration |

[Full tool reference](tools/)

**Deferred tools** (loaded on demand): `fetch_web`, `search_web`, `bulk`,
`bulk_edit`, `bulk_write` — available when the plugin or skill provides them.

### Configuration

Configuration follows a strict precedence: **CLI flag > environment variable >
`settings.local.json` > `settings.json` > `~/.config/settings.json` >
`managed-settings.json` > `managed-settings.d/*.json`**. Arrays concatenate and
deduplicate; objects deep-merge; `null` removes a key.

Key configuration categories:

- [Providers & API keys](configuration/#providers)
- [Workspace & path confinement](configuration/#workspace)
- [Sandbox modes](configuration/#sandbox)
- [Approval mode](configuration/#approval)
- [Session & token budget](configuration/#session)
- [Model routing](configuration/#model-routing)
- [Plugins](configuration/#plugins)
- [Permissions (allow/deny rules)](configuration/#permissions)
- [Logging & debug](configuration/#logging)

[Full configuration reference](configuration/)

### Providers & Login

The harness ships with three built-in provider presets. Log into several at
once; the model picker shows every provider's models tagged by prefix.

| Preset | Endpoint | Auth |
|--------|----------|------|
| **Umans** | `api.code.umans.ai/v1` | API key (`UMANS_API_KEY`) |
| **OpenCode Go** | `opencode.ai/zen/go/v1` | API key (`OPENCODE_GO_API_KEY`) |
| **OpenRouter** | `openrouter.ai/api/v1` | API key (`OPENROUTER_API_KEY`) |

**Subscription login (OAuth):** ChatGPT Plus/Pro, Claude Pro/Max, SuperGrok,
and similar — installed as **plugins** that declare an `oauth` block in
`plugin.json`. The harness owns `/login` / `/oauth-code` UX; the plugin owns
authorize/token/refresh.

```text
/plugin-install karutoil/catcode-chatgpt-provider global
/login chatgpt
```

[Provider reference](../README.md#providers-and-login) · [OAuth plugin
contract](plugins/)

### Plugins

Plugins extend the harness without a recompile. They live in
`.catalyst-code/plugins/` and can declare:

- **Hooks:** `pre_input`, `pre_agent_start`, `pre_turn` (model handoff),
  `pre_tool`, `post_tool`, `pre_write`, `bash` (override), `post_turn`, lifecycle
- **Custom tools:** registered at load time — no MCP, no separate process
- **Custom slash commands:** `/hello`, `/status`, etc.
- **Memory providers:** swap the default markdown-file store for SQLite, etc.
- **OAuth providers:** OAuth authorize/token/refresh scripts for subscription login
- **Overrides:** intercept and replace built-in behavior (e.g., `bash`)

[Plugin authoring guide](plugins/) · [Plugin contract](PLUGINS.md) ·
[Example plugins](examples/plugins/README.md) · [Plugin schema
reference](../core/src/plugins.rs)

Example plugins in `docs/examples/plugins/`:

| Plugin | Shows |
|--------|-------|
| `path-guard` | `pre_write` deny for `.env` / keys |
| `hello-command` | `/hello` slash command |
| `sqlite-memory` | `memory_provider` backed by SQLite |
| `sandbox-deny-env` | `bash` tool override blocking secret commands |
| `grok-oauth` | Plugin-declared OAuth provider |

### Subagents

Built-in subagents (`.catalyst-code/agents/*.md`, overridable):

`scout` · `researcher` · `planner` · `worker` · `reviewer` · `context-builder` ·
`oracle` · `delegate`

**Execution modes:**

```ts
{ agent: "worker", task: "refactor auth" }                            // single
{ tasks: [{ agent: "scout" }, { agent: "planner" }], concurrency: 2 } // parallel
{ chain: [{ agent: "scout" }, { agent: "planner" }, { agent: "worker" }] } // chain
```

**Management:** `list` / `get` / `create` / `status` / `interrupt` / `resume` /
`peek` / `steer`

**Intercom:**

- `contact_supervisor` — ask the orchestrator a blocking question (appears as a
  TUI prompt)
- `intercom` — peer-to-peer messaging (`send` · `ask` · `receive` · `reply` ·
  `targets`)

[Subagent reference](../README.md#subagents-and-intercom) · [Intercom
contract](../core/src/intercom.rs)

### Goal Mode

`/goal` opens a multi-field modal for a high-level objective:

- **Objective** — free-text description
- **Concurrency** — how many subagents to run in parallel
- **Model/Provider allowlists** — restrict which models can be used
- **Review plan before deploy** — stop at plan-ready for approve/revise

The core plans (`goal_write_plan`), then deploys subagents under the specified
caps. Concurrency 8+ automatically uses an ultra-parallel planning profile:
broad independent fan-out first, chains retained only for real dependencies.

`/cancel-goal` aborts the current objective.

[Goal mode reference](../core/src/goal.rs)

### Self-Learning

The self-learning layer extends the harness with:

- **Memory store** (`core/src/memory.rs`) — persistent markdown files injected
  into the system prompt
- **Skill library** (`skills-lock.json`, `.catalyst-code/skills/`) — reusable
  capability bundles
- **Telemetry-backed reflection** — automated learning passes that capture
  gotchas, conventions, and architecture notes
- **Embedding-sketch recall** (`core/src/embed.rs`) — hashing-sketch memory
  retrieval (Milestone 4)

[Design document](SELF_LEARNING.md) · [Memory source](../core/src/memory.rs) ·
[Embed source](../core/src/embed.rs)

### Sessions & Checkpoints

- **Append-only JSONL sessions** — every command/event logged to
  `~/.local/share/catalyst-code/sessions/` (or platform equivalent)
- **Auto-compaction** — summarizing context compaction with orphaned-tool-call
  sanitization
- **Hybrid checkpoints** — `checkpoint.rs`: either a git stash ref or a
  filesystem snapshot. Supports **undo** to restore disk state.
- **Core-crash auto-recovery** — TUI and web frontend respawn the core on
  unexpected exit

[Session source](../core/src/session.rs) · [Checkpoint
source](../core/src/checkpoint.rs)

---

## Architecture

```
core/        Rust async engine (stdio JSONL)   tui/   Go + Bubble Tea terminal UI
sdk/         TypeScript pi-compatible wrapper   web/   Next.js web frontend (SSE bridge)
packaging/   per-platform install scripts       .catalyst-code/   bundled agents, plugins, skills
```

**Data flow:**

1. User types a prompt in TUI or web
2. Frontend sends JSONL `Command` to core's stdin
3. Core runs the agentic loop: model streaming → tool calls → approval gate →
   tool execution → loop or `done`
4. Core writes JSONL `Event` to stdout
5. Frontend renders events (streaming deltas, tool calls, approvals, metrics)

**Security boundaries:**

- **Workspace confinement** — all file ops resolve against a workspace root;
  absolute paths, `..`, and symlink escapes are rejected
- **Approval gate** — three modes: `never` (auto-approve), `destructive` (ask
  for bash/write/edit — default), `always` (ask for every tool)
- **Restricted paths** — `.env`, `.git`, `.ssh` gated for both reads and writes
- **Sandbox** — optional Microsandbox microVM (`--sandbox microsandbox`, or
  `--no-network` to block guest egress); defaults to `none` (denylist tripwire
  only). Runs on Linux (KVM), Apple Silicon macOS, and Windows (WHP). See
  [Sandbox Guide](guides/sandbox.md).
- **Plugin permissions** — hooks can deny, allow, or override built-in tools

[Architecture deep dive](architecture/) · [Security model](../README.md#security-notes)

---

## Operations

| Topic | Guide |
|-------|-------|
| Install | [One-liner](../README.md#installation), [Web install](../README.md#web-frontend), [Windows](../README.md#windows-tui--optional-web), [Prebuilt binaries](../README.md#prebuilt-binaries-standalone-no-installer) |
| Update | [`catcode --update`](../README.md#updating) or Settings → About → Update |
| Uninstall | `curl ... \| bash -s -- --uninstall` or Settings → About |
| Service management | systemd (Linux), launchd (macOS), NSSM/Scheduled Task (Windows) |
| Reverse proxy | Bind web to `127.0.0.1`, put Caddy/nginx/IIS with TLS in front |
| Private repo | Clone locally, run `install.sh` or `install.ps1` from the checkout |
| Build from source | [`build.sh`](../build.sh), then `cd core && cargo build --release`, `cd tui && go build` |
| Release | `release-all.sh <version>`, or per-platform: `release-linux.sh`, `release-macos.sh`, `release-windows.sh`, `release-web.sh` |

---

## Contributing

- [Contributing guide](../CONTRIBUTING.md) — Rust/Go/security conventions
- [Code of Conduct](../CODE_OF_CONDUCT.md)
- [Development setup](../README.md#development-setup) — build from source
- [Running tests](../README.md#running-tests) ��� `cargo test` (core) + `go test ./...` (TUI)
- [Wire protocol](../README.md#wire-protocol) — command/event JSONL reference
- [CI workflow](../.github/workflows/ci.yml) — clippy + test + cross-compile matrix
- [Release workflow](../.github/workflows/release.yml) — publishes GitHub Release on `v*` tag

---

## Reference

| Document | Content |
|----------|---------|
| [README](../README.md) | Project overview, install, usage, providers, architecture |
| [CHANGELOG](../CHANGELOG.md) | Release history |
| [LICENSE](../LICENSE) | MIT License |
| [Wire protocol](../README.md#wire-protocol) | JSONL Command/Event types |
| [CLI flags](commands/) | `catcode` and `catcode-core` flags |
| [Configuration](configuration/) | All config keys, env vars, file layering |
| [Tools](tools/) | Full tool reference with schemas |
| [Plugins](plugins/) | Plugin authoring guide |
| [Architecture](architecture/) | Components, data flow, security boundaries |
| [Environment variables](configuration/index.md) | All env vars and CLI flags |
| [Glossary](reference/glossary.md) | Project terminology |
| [Exit codes](reference/exit-codes.md) | Core and CLI exit codes |
| [Compatibility](reference/compatibility.md) | Supported platforms and providers |
| [Sandbox Guide](guides/sandbox.md) | Microsandbox microVM setup, network/env policy, troubleshooting |

**Internal / advanced documents:**

| Document | Scope |
|----------|-------|
| [`SELF_LEARNING.md`](SELF_LEARNING.md) | Self-learning layer design & implementation |
| [`IDE_PANELS_CONTRACT.md`](IDE_PANELS_CONTRACT.md) | Web IDE panels integration contract |
| [`tui-perf-surface.md`](tui-perf-surface.md) | TUI performance surface map |
| [`PLUGINS.md`](PLUGINS.md) | Plugin authoring contract entry point |
| [`examples/plugins/README.md`](examples/plugins/README.md) | Example plugin catalog |
| [`core/src/`](../core/src) | Full core source (flat modules) |
| `tui/` | TUI Go source |
| `sdk/` | TypeScript SDK source |
| `web/` | Next.js web frontend source |

> **Stability:** The wire protocol is stable. Core modules documented in
> `core/src/` are internal — use the protocol and plugin system for extension.
> Plugin hooks and tool schemas are stable; plugin OAuth and memory providers
> are experimental.
