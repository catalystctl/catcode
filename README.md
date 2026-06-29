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
- **Hash-anchored editing** — `read_file` returns `HASH│content` per line; `edit` targets those 4-char hashes (`replace`/`append`/`prepend`, atomic, multi-op). Stale-anchor errors trigger a re-read loop. No line-number drift.
- **grep + glob** — purpose-built search tools (regex content search, `**/*.ext` glob) so the model doesn't fumble with raw bash for exploration.
- **bash** — async, timeout, kill, denylist, cwd-locked, 8KB output cap.

**Observability & persistence**
- **Structured debug log** — JSONL records (`init`, `tool`, `turn_done`, `http_retry`, `turn_error`) to `--debug-log <file>` for post-mortem.
- **Metrics** — TTFT, elapsed, tokens in/out, TPS emitted per turn (`metrics` event) and shown in the TUI status bar.
- **Session persistence** — sessions are stored **per workspace** under `~/.config/umans-harness/sessions/<hex(cwd)>/` as append-only JSONL files; one project can hold an unlimited number of them. On restart the most-recently-modified session is replayed (crash-safe: a mid-turn crash loses at most the in-flight turn). `/new` starts a fresh session file (the previous one is kept on disk); `/sessions` opens a searchable picker to switch between this project's sessions. A legacy single-file layout is migrated into the per-project dir automatically.

**Config & packaging**
- **CLI flags + env vars + JSON config file** — `--workspace`, `--base-url`, `--approval`, `--bash-timeout`, `--max-turns`, `--debug-log`, `--session`, `--model`, `--config`. Env: `UMANS_BASE_URL`, `UMANS_HARNESS_*`. Config files: `./umans-harness.json` or `~/.config/umans-harness/config.json`.
- **`--help` / `--version`** — CLI is self-documenting.
- **OpenAI-compatible** — change `--base-url` and model IDs to point at any OpenAI-shaped endpoint. Umans is the default; the GLM `reasoning_effort=high` clamp and `reasoning_content` replay are Umans/Zhipu-specific.

## Layout

```
core/                 Rust core (stdio JSON-RPC server)
  src/main.rs         stdin dispatch, approval gate, turn loop, compaction, metrics
  src/provider.rs     OpenAI streaming client: retry/backoff, idle timeout, orphaned-call sanitize
  src/protocol.rs     wire types (Command / Event) + line emit
  src/config.rs       CLI + env + JSON config, approval modes
  src/workspace.rs    path confinement (absolute/.. /symlink rejection)
  src/tools.rs        read_file / edit / write_file / list_dir / grep / glob / bash
  src/hashline.rs     4-char line hashes for anchored editing
  src/logging.rs      JSONL debug log + token estimation + turn timer
  src/session.rs      append-only JSONL session persistence
tui/                  Go Bubble Tea TUI
test_tui.exp          basic e2e (auth, stream, tool call)
test_prod.exp         production e2e (approval gate, confinement, metrics)
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
cd core && cargo test --release    # 25 unit tests (edit, confinement, bash timeout, glob, grep, sanitize, backoff, session, hashline)
./test_tui.exp                     # basic e2e against live Umans
./test_prod.exp                    # approval gate + confinement + metrics e2e
```

## Notes

- The core is OpenAI-compatible; Umans-specific logic (GLM clamp, `reasoning_content` replay, `/models/info` discovery) is isolated to `provider.rs` and toggled by `--base-url`.
- Agentic turns default to 200 (`--max-turns`); the real ceiling is the session token budget (`--max-session-tokens`, 0 = unlimited). The model can also call the `finish` tool to exit the loop cleanly, or `spawn` a nested sub-agent (bounded by `spawn_max_turns`).
- For a hard security boundary, pass `--sandbox firejail --no-network` (or set them in the TUI settings modal). The denylist remains a tripwire on top; the workspace confinement covers file paths, but `bash` itself is only sandboxed when `--sandbox` is set.
