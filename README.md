# Umans AI Harness

A production-grade, OpenAI-compatible coding-agent harness with **native Umans** provider support.
- **Core** (`core/`): Rust async engine. Streams from any OpenAI-compatible `/chat/completions`, discovers models live, runs an agentic tool loop with a human-in-the-loop approval gate, and speaks a newline-delimited JSON protocol over stdio.
- **TUI** (`tui/`): Go + [Bubble Tea](https://github.com/charmbracelet/bubbletea) terminal interface. Spawns the core, streams events, shows metrics, and handles approval prompts.

> **v0.2.0** adds the production hardening layer: `spawn` subagent + `finish`/`todo`/`patch`/`diagnostics` tools for real agentic loops, summarizing context compaction, session token budgets, `--sandbox firejail` + `--no-network`, persistent per-workspace chats with `/undo`/`/compact`/`/sessions`/`/stats`, vision input (`/attach`), core-crash auto-recovery, and a Dockerfile + `release.sh`. See [`CHANGELOG.md`](CHANGELOG.md) for the full list.

## Production features

**Safety floor**
- **Workspace confinement** — every file op (`read_file`/`write_file`/`edit`/`list_dir`/`grep`/`glob`) resolves paths against a workspace root; absolute paths and `..` escapes are rejected, symlink escapes caught via canonicalization. `bash` runs with `cwd=workspace`.
- **Human-in-the-loop approval** — destructive tools (`bash`, `write_file`, `edit`) require consent under the default `destructive` mode. The TUI shows the call and prompts `[y]es / [n]o / [a]lways`; `always` escalates **only the matched tool kind** (not the whole session). Modes: `never` / `destructive` / `always`. Runtime-switchable via `/approval`.
- **Optional hard sandbox** — `--sandbox firejail` wraps bash in a generated firejail profile that whitelists only the workspace + shell paths and drops caps/seccomp; `--no-network` adds `unshare -n` so bash can't phone home. The denylist is still a tripwire on top.
- **Real bash timeout + kill** — commands run with a configurable wall-clock timeout (`--bash-timeout`, default 30s); a hung command is killed, not held forever. Output capped at 32 KB (head truncated, tail kept).

**Robustness**
- **HTTP retry/backoff** — 429 and 5xx retried with exponential backoff (0.5s→1s→2s→4s, capped 8s), honoring `Retry-After`. Transport errors retried too. Up to 4 attempts.
- **Idle stream timeout** — if no bytes arrive for 60s mid-stream, the turn aborts instead of hanging for 300s.
- **Context window management** — token estimate (~4 chars/token) triggers compaction at 70% of the model's window: oldest tool results dropped, system + recent turns kept, with a compaction marker. **Orphaned-tool-call sanitization** inserts synthetic tool results so a compacted history never sends an assistant `tool_calls` without matching results (mirrors the `pi-provider-umans` extension).
- **File-size guards** — `read_file` refuses files >1 MiB or >2000 lines; `grep`/`glob` cap results (50/200). No OOM from a giant log.
- **SSE parser** — handles `data:` framing, `[DONE]`, keepalive comments, and the final `usage` chunk (`stream_options.include_usage`).

**Tooling**
- **Search-and-replace editing** — `read_file` returns a file's plain content; `edit` takes `{search, replace}` pairs (exact, unique match; empty replace deletes; atomic, multi-op). To insert, anchor on a unique line and include it in the replacement. No hashes or line numbers to drift.
- **grep + glob** — purpose-built search tools (regex content search, `**/*.ext` glob) so the model doesn't fumble with raw bash for exploration.
- **bash** — async, timeout, kill, denylist, cwd-locked, 8KB output cap.

**Observability & persistence**
- **Structured debug log** — JSONL records (`init`, `tool`, `turn_done`, `http_retry`, `turn_error`) to `--debug-log <file>` for post-mortem.
- **Metrics** — TTFT, elapsed, tokens in/out, TPS emitted per turn (`metrics` event) and shown in the TUI status bar.
- **Session persistence** — sessions are stored **per workspace** under `~/.config/umans-harness/sessions/<hex(cwd)>/` as append-only JSONL files; one project can hold an unlimited number of them. On restart the most-recently-modified session is replayed (crash-safe: a mid-turn crash loses at most the in-flight turn). `/new` starts a fresh session file (the previous one is kept on disk); `/sessions` opens a searchable picker to switch between this project's sessions. A legacy single-file layout is migrated into the per-project dir automatically.

**Config & packaging**
- **CLI flags + env vars + JSON config file** — `--workspace`, `--base-url`, `--approval`, `--bash-timeout`, `--debug-log`, `--session`, `--model`, `--config`. Env: `UMANS_BASE_URL`, `UMANS_HARNESS_*`. Config files: `./umans-harness.json` or `~/.config/umans-harness/config.json`.
- **`--help` / `--version`** — CLI is self-documenting.
- **OpenAI-compatible** — change `--base-url` and model IDs to point at any OpenAI-shaped endpoint. Umans is the default; the GLM `reasoning_effort=high` clamp and `reasoning_content` replay are Umans/Zhipu-specific.

## Layout

```

## Subagents & intercom

A port of [`pi-subagents`](https://github.com/nicobailon/pi-subagents) is built into the core. The orchestrator (parent agent) can delegate to focused child agents via the `subagent` tool; children can prompt the orchestrator for decisions and talk to each other over an in-process intercom bus.

**Built-in agents** (`.umans-harness/agents/*.md`, overridable): `scout`, `researcher`, `planner`, `worker`, `reviewer`, `context-builder`, `oracle`, `delegate`. Each is a markdown file with YAML frontmatter (`tools`, `model`, `thinking`, `systemPromptMode`, `defaultContext`, …). Discover with `subagent({ action: "list" })` or `/subagents`.

**Execution modes**: single `{ agent, task }`, parallel `{ tasks, concurrency }`, chain `{ chain: [...] }` (with `{previous}`/`{outputs.name}` templating and inline parallel groups), plus management actions `list`/`get`/`create`/`update`/`delete`/`status`/`interrupt`/`resume`/`doctor`/`models`. Recursion is capped by `maxSubagentDepth` (default 2; env `UMANS_SUBAGENT_MAX_DEPTH`).

**Intercom (the centerpiece):**
- `contact_supervisor({ reason: "need_decision", message })` — a subagent asks the orchestrator a blocking question. It surfaces in the TUI as a prompt (`❓ subagent … asks: …`); type a reply + Enter (or Esc to unblock with best-judgment). This is how subagents prompt the orchestrator for issues.
- `intercom({ action: "send"|"ask"|"receive"|"reply"|"targets", to, message })` — peer-to-peer plumbing so subagents can talk to each other (e.g. a worker `ask`s a parallel reviewer, then `reply`).
- Allowed by setup: the `intercomBridge` mode (`off`/`fork-only`/`always`, default `always`) and an agent's `tools` list (`contact_supervisor`/`intercom` must be present). Each subagent gets a registered target; discover peers with `action: "targets"`.

**Slash commands** (TUI): `/run <agent> "<task>"`, `/parallel <a> "t" | <b> "t"`, `/chain <a> "t" -> <b> "t"`, `/subagents`, `/subagents-doctor`, `/subagents-status`, `/subagents-models`.

**Config** (settings JSON under `subagents`):
```json
{ "subagents": { "maxSubagentDepth": 2, "intercomBridge": { "mode": "always" }, "parallel": { "maxTasks": 8, "concurrency": 4 }, "asyncByDefault": false, "disableBuiltins": false, "agentOverrides": { "reviewer": { "model": "umans-glm-5.2", "thinking": "high" } } } }
```

Forked context (`context: "fork"`) starts a child from a filtered snapshot of the parent conversation; model fallback tries `model` then `fallbackModels` on provider failures; the orchestrator skill (`.umans-harness/skills/pi-subagents/SKILL.md`) is injected into the parent only — children never receive it.
core/                 Rust core (stdio JSON-RPC server)
  src/main.rs         stdin dispatch, approval gate, turn loop, compaction, metrics
  src/provider.rs     OpenAI streaming client: retry/backoff, idle timeout, orphaned-call sanitize
  src/protocol.rs     wire types (Command / Event) + line emit
  src/config.rs       CLI + env + JSON config, approval modes
  src/workspace.rs    path confinement (absolute/.. /symlink rejection)
  src/tools.rs        read_file / edit / write_file / list_dir / grep / glob / bash
  src/logging.rs      JSONL debug log + token estimation + turn timer
  src/session.rs      append-only JSONL session persistence
tui/                  Go Bubble Tea TUI
.github/workflows/    CI (core clippy/test, tui vet/test/build, docker image)
```

## Build

```bash
cd core && cargo build --release      # -> core/target/release/core
cd tui && go build -o tui             # -> tui/tui
```

Requires Rust (stable) and Go 1.21+ (tested with Go 1.23).

## Run

```bash
./tui/tui
```

In the TUI:
- `/key sk-...`          set your Umans API key (https://app.umans.ai/billing → API Keys)
- `/model [N|substr]`    list models, or switch (`/model 3`, `/model glm-5.2`)
- `/approval <mode>`     never | destructive | always
- `/reset`               wipe conversation + current session file
- `/sessions`            open a searchable picker of this project's sessions
- `/new`                 start a fresh session file (keeps the old)
- `/abort`               stop the running turn
- `/help`                commands
- `Ctrl+C`               quit
- when `⚠ APPROVE?` shows: `y` approve once · `a` approve and stop asking · `n` deny
- anything else          sent as a prompt to the current model

## Windows install (`ucli`)

Cross-compile both binaries for Windows and package them into a per-user MSI
that installs `ucli` + `umans-core` to `%LOCALAPPDATA%\Programs\ucli` and adds
that directory to the user PATH (so `ucli` works from any CWD, no admin needed):

```bash
./release-windows.sh        # -> dist/ucli-<ver>-windows.msi + .sha256
```

`release-windows.sh` cross-compiles with cargo (`x86_64-pc-windows-gnu`) and Go
(`GOOS=windows`), then builds the MSI with msitools `wixl` from
`packaging/windows/ucli.wxs` (the same `.wxs` also compiles with the WiX
Toolset `candle`+`light` on a Windows build host).

On Windows, install by double-clicking the `.msi`, or silently:

```powershell
msiexec /i ucli-<ver>-windows.msi            # interactive (no UAC prompt)
msiexec /i ucli-<ver>-windows.msi /quiet     # silent
```

The MSI is per-user (no admin), writes a clean Add/Remove Programs entry,
and supports in-place upgrades (fixed `UpgradeCode`). Open a new PowerShell
window after install and run `ucli` from any directory. First run: `/key sk-...`
then `/model`.

No `wixl`/WiX available? `packaging/windows/install.ps1` is a no-build fallback:
unzip the two `.exe` files beside it and run `.\install.ps1` to copy them into
`%LOCALAPPDATA%\Programs\ucli` and update the user PATH.

The TUI finds the core by searching, in order: `$UMANS_CORE`, `umans-core(.exe)`
next to the TUI, then the dev paths `core/target/release/core(.exe)`. Set
`UMANS_CORE=<path>` to point at a custom core build.

Runtime caveats on Windows:
- The agent's `bash` tool needs bash on PATH (Git Bash or WSL); chat and the
  file tools (read/edit/write/grep/glob/list_dir) work without it.
- Sandboxing (`--sandbox firejail` / `--no-network`) is Linux-only; leave
  `/sandbox` set to `none`.

## macOS standalone executable

Download a single self-contained executable that runs from any CWD — no install,
no separate `umans-core`. The Rust core is embedded in the binary and extracted
to `~/Library/Caches/umans-harness` on first run.

Grab the matching arch from `dist/` (built by `./release-macos.sh`):

- `umans-harness-<ver>-macos-arm64`  — Apple Silicon (M-series)
- `umans-harness-<ver>-macos-x86_64` — Intel

```bash
chmod +x umans-harness-0.2.0-macos-arm64
./umans-harness-0.2.0-macos-arm64      # launches in the current directory
```

Then `/key sk-...`, `/model`, and type a prompt. The workspace is the directory
you launched from — rerun from another folder to work on a different project.

Build it yourself on Linux (zig is the macOS linker; no Xcode/SDK needed):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo install cargo-zigbuild          # and put zig 0.13+ on PATH
./release-macos.sh                    # -> dist/umans-harness-<ver>-macos-{arm64,x86_64} + .sha256
```

`release-macos.sh` cross-compiles the core with `cargo zigbuild` (pure-Rust
`rustls-tls`, so no macOS SDK) and the TUI with `GOOS=darwin`, embedding the
core via `go:embed` (`-tags embed_core`) so each output is one file. The TUI
resolves the core as: `$UMANS_CORE` → embedded extraction → the usual
dev/installed search, so dev builds and the Windows MSI layout are unchanged.

Runtime caveats on macOS:
- Sandboxing (`--sandbox firejail` / `--no-network`) is Linux-only; leave
  `/sandbox` set to `none`.
- The agent's `bash` tool needs `bash` on PATH (present by default on macOS).

## Protocol

Core reads commands from stdin, writes events to stdout, one JSON object per line.

Commands (stdin):
```json
{"type":"init"}
{"type":"set_key","api_key":"sk-..."}
{"type":"send","prompt":"...","model":"umans-glm-5.2","reasoning_effort":"high"}
{"type":"abort"}
{"type":"reset"}
{"type":"approve","request_id":"<id>","decision":"yes|no|always"}
{"type":"set_approval","mode":"never|destructive|always"}
```

Events (stdout): `ready` · `authed` · `thinking` · `delta` · `tool_call_start` · `tool_call_name` · `tool_call_args` · `tool_call` · `approval_request` · `tool_result` · `compacted` · `http_retry` · `metrics` · `approval_changed` · `done` · `aborted` · `reset` · `error`.

## Test

```bash
cd core && cargo test --release    # unit tests (edit search/replace, confinement, bash timeout, glob, grep, sanitize, backoff, session)
cd tui && go test ./...            # TUI tests (handlers, blocks, mention, modal)
# Live e2e (auth, approval gate, confinement) against the Umans API is not yet
# scripted — see .github/workflows/ci.yml for the automated checks that run.
```

## Notes

- The core is OpenAI-compatible; Umans-specific logic (GLM clamp, `reasoning_content` replay, `/models/info` discovery) is isolated to `provider.rs` and toggled by `--base-url`.
- There is no fixed turn cap; the real ceiling is the session token budget (`--max-session-tokens`, 0 = unlimited). The model can also call the `finish` tool to exit the loop cleanly, or `spawn` a nested sub-agent.
- For a hard security boundary, pass `--sandbox firejail --no-network` (or set them in the TUI settings modal). The denylist remains a tripwire on top; the workspace confinement covers file paths, but `bash` itself is only sandboxed when `--sandbox` is set.
