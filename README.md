# Catalyst Code

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
- **Idle stream timeout** — if no bytes arrive for 120s mid-stream (default `--idle-timeout`; raised from 60s so reasoning models that think before the first token don't abort), the turn aborts instead of hanging for 300s.
- **Context window management** — token estimate (~4 chars/token) triggers compaction at 70% of the model's window: oldest tool results dropped, system + recent turns kept, with a compaction marker. **Orphaned-tool-call sanitization** inserts synthetic tool results so a compacted history never sends an assistant `tool_calls` without matching results (mirrors the `pi-provider-umans` extension).
- **File-size guards** — `read_file` refuses files >5 MiB or >10 000 lines (with `offset`/`limit` pagination for the rest); `grep`/`glob` cap results (50/200). No OOM from a giant log.
- **SSE parser** — handles `data:` framing, `[DONE]`, keepalive comments, and the final `usage` chunk (`stream_options.include_usage`).

**Tooling**
- **Search-and-replace editing** — `read_file` returns a file's plain content; `edit` takes `{search, replace}` pairs (exact, unique match; empty replace deletes; atomic, multi-op). To insert, anchor on a unique line and include it in the replacement. No hashes or line numbers to drift.
- **grep + glob** — purpose-built search tools (regex content search, `**/*.ext` glob) so the model doesn't fumble with raw bash for exploration.
- **bash** — async, timeout, kill, denylist, cwd-locked, 32 KB output cap (head truncated, tail kept).

**Observability & persistence**
- **Structured debug log** — JSONL records (`init`, `tool`, `turn_done`, `http_retry`, `turn_error`) to `--debug-log <file>` for post-mortem.
- **Metrics** — TTFT, elapsed, tokens in/out, TPS emitted per turn (`metrics` event) and shown in the TUI status bar.
- **Session persistence** — sessions are stored **per workspace** under `~/.config/catalyst-code/sessions/<hex(cwd)>/` as append-only JSONL files; one project can hold an unlimited number of them. On restart the most-recently-modified session is replayed (crash-safe: a mid-turn crash loses at most the in-flight turn). `/new` starts a fresh session file (the previous one is kept on disk); `/sessions` opens a searchable picker to switch between this project's sessions. A legacy single-file layout is migrated into the per-project dir automatically.

**Config & packaging**
- **CLI flags + env vars + JSON config file** — `--workspace`, `--base-url`, `--approval`, `--bash-timeout`, `--debug-log`, `--session`, `--model`, `--config`. Env: `UMANS_BASE_URL`, `CATALYST_CODE_*`. Config files: `./catalyst-code.json` or `~/.config/catalyst-code/config.json`.
- **`--help` / `--version`** — CLI is self-documenting.
- **OpenAI-compatible** — change `--base-url` and model IDs to point at any OpenAI-shaped endpoint. Umans is the default; the GLM `reasoning_effort=high` clamp and `reasoning_content` replay are Umans/Zhipu-specific.

## Layout

```

## Subagents & intercom

A port of [`pi-subagents`](https://github.com/nicobailon/pi-subagents) is built into the core. The orchestrator (parent agent) can delegate to focused child agents via the `subagent` tool; children can prompt the orchestrator for decisions and talk to each other over an in-process intercom bus.

**Built-in agents** (`.catalyst-code/agents/*.md`, overridable): `scout`, `researcher`, `planner`, `worker`, `reviewer`, `context-builder`, `oracle`, `delegate`. Each is a markdown file with YAML frontmatter (`tools`, `model`, `thinking`, `systemPromptMode`, `defaultContext`, …). Discover with `subagent({ action: "list" })` or `/subagents`.

**Execution modes**: single `{ agent, task }`, parallel `{ tasks, concurrency }`, chain `{ chain: [...] }` (with `{previous}`/`{outputs.name}` templating and inline parallel groups), plus management actions `list`/`get`/`create`/`update`/`delete`/`status`/`interrupt`/`resume`/`doctor`/`models`. Recursion is capped by `maxSubagentDepth` (default 2; env `CATALYST_CODE_SUBAGENT_MAX_DEPTH`).

**Intercom (the centerpiece):**
- `contact_supervisor({ reason: "need_decision", message })` — a subagent asks the orchestrator a blocking question. It surfaces in the TUI as a prompt (`❓ subagent … asks: …`); type a reply + Enter (or Esc to unblock with best-judgment). This is how subagents prompt the orchestrator for issues.
- `intercom({ action: "send"|"ask"|"receive"|"reply"|"targets", to, message })` — peer-to-peer plumbing so subagents can talk to each other (e.g. a worker `ask`s a parallel reviewer, then `reply`).
- Allowed by setup: the `intercomBridge` mode (`off`/`fork-only`/`always`, default `always`) and an agent's `tools` list (`contact_supervisor`/`intercom` must be present). Each subagent gets a registered target; discover peers with `action: "targets"`.

**Slash commands** (TUI): `/run <agent> "<task>"`, `/parallel <a> "t" | <b> "t"`, `/chain <a> "t" -> <b> "t"`, `/subagents`, `/subagents-doctor`, `/subagents-status`, `/subagents-models`.

**Config** (settings JSON under `subagents`):
```json
{ "subagents": { "maxSubagentDepth": 2, "intercomBridge": { "mode": "always" }, "parallel": { "maxTasks": 8, "concurrency": 4 }, "asyncByDefault": false, "disableBuiltins": false, "agentOverrides": { "reviewer": { "model": "umans-glm-5.2", "thinking": "high" } } } }
```

Forked context (`context: "fork"`) starts a child from a filtered snapshot of the parent conversation; model fallback tries `model` then `fallbackModels` on provider failures; the orchestrator skill (`.catalyst-code/skills/pi-subagents/SKILL.md`) is injected into the parent only — children never receive it.
core/                 Rust core (stdio JSON-RPC server)
  src/main.rs         stdin dispatch, approval gate, turn loop, compaction, metrics
  src/provider.rs     OpenAI streaming client: retry/backoff, idle timeout, orphaned-call sanitize
  src/subagent.rs     subagent execution (single/parallel/chain), forked context, depth cap
  src/intercom.rs     peer intercom bus (contact_supervisor / intercom ask/receive/reply)
  src/plugins.rs      plugin manager + hook execution (pre_*/post_*/lifecycle/pre_turn)
  src/protocol.rs     wire types (Command / Event) + line emit
  src/config.rs       CLI + env + JSON config, approval modes
  src/workspace.rs    path confinement (absolute/.. /symlink rejection)
  src/tools.rs        read_file / edit / write_file / list_dir / grep / glob / bash
  src/logging.rs      JSONL debug log + token estimation + turn timer
  src/memory.rs       persistent memory store (injected into the system prompt)
  src/git_ctx.rs      git status/branch context for the system prompt
  src/vision.rs       vision model config + image attachment helpers
  src/session.rs      append-only JSONL session persistence
tui/                  Go Bubble Tea TUI
web/                  Next.js web frontend (SSE bridge to the core) — see web/README.md
.github/workflows/    CI (core clippy/test, tui vet/test/build, docker image)
```

## Build

```bash
cd core && cargo build --release      # -> core/target/release/core
cd tui && go build -o tui             # -> tui/tui
```

Requires Rust (stable) and Go 1.21+ (tested with Go 1.23).

To build **all** release artifacts at once — Windows MSI + standalone `.exe`,
macOS standalone + `.dmg`, Linux standalone + AppImage — run
`./release-all.sh [version]`. It runs each platform script independently and
reports per-platform pass/fail, so a host with only a partial toolchain (e.g.
no `zig` for macOS) still builds whatever it can. See the per-platform sections
below for each toolchain's requirements.

## Run

```bash
./tui/tui
```

In the TUI:
- `/login`              log in / switch provider — OpenAI (Codex), Google Gemini, Anthropic
- `/logout`             log out of a provider
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

## Providers & login

First-party providers are built-in and one click to set up. `/login` opens a
picker of the bundled presets:

| Preset | Kind | Endpoint | Key env var |
|---|---|---|---|
| **Umans (GLM-5.2)** | OpenAI | `api.code.umans.ai/v1` | `UMANS_API_KEY` |
| **OpenAI (Codex)** | OpenAI | `api.openai.com/v1` | `OPENAI_API_KEY` |
| **Google Gemini** | OpenAI (compat shim) | `generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` (or `GOOGLE_API_KEY`) |
| **Anthropic Claude** | Anthropic | `api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |

Picking a preset:
- **key in the env var** → logs in instantly (`/login` sends `login` with no key);
- **no key** → prompts you to paste one, then logs in.

**Multiple simultaneous logins.** You can be logged into several providers at
once — e.g. OpenAI + Gemini + Anthropic. `/models` then lists every logged-in
provider's models, each tagged `[openai]`, `[gemini]`, `[anthropic]`. Select any
model and that turn is routed to its provider's endpoint, so you can mix
subscription models in one session. `/logout` drops a provider (its models
leave `/models`). The original Umans provider still works as the default when no
other is logged in.

Keys are persisted per-provider (the env-var *name* is stored when a key came
from the environment, so the secret never lands in a config file).

### Subscription login (OAuth) — no API key needed

ChatGPT Plus/Pro (Codex), Google One AI (Gemini), and Claude Pro/Max
subscriptions are accessed via **OAuth login**, not an API key. You don't need
the official CLI: `/login` performs the OAuth flow itself.

- **Gemini**: `/login` → pick **Google Gemini** runs Google's
  **authorization-code + PKCE + loopback-redirect** flow — it opens your browser
  to accounts.google.com; approve, and the harness captures the token via a
  local callback server. (Uses gcloud's public Desktop-app client credentials;
  the device-code grant doesn't work with Desktop clients — it 401s. If you
  already ran `gcloud auth application-default login`, that token is reused
  automatically.)
- **Anthropic Claude**: `/login` → pick **Anthropic Claude** runs the
  **authorize + PKCE + loopback-redirect** flow — it opens your browser to
  claude.ai; approve, and the harness captures the token. (If you already
  logged in via the `claude` CLI, that token is reused automatically.)
- **OpenAI Codex**: ⚠️ not yet wired — the ChatGPT token works only against the
  `chatgpt.com` Responses API (a different request shape than chat-completions).
  Codex stays on `OPENAI_API_KEY` until a Responses-API integration is added.

Tokens from `/login` are stored at `~/.config/catalyst-code/oauth/<id>.json`
(0600) and refreshed automatically. The OAuth client credentials used are the
vendors' public installed-app client IDs (gcloud's for Google, Claude Code's
for Anthropic).

Everything falls back gracefully: if no OAuth token is present the harness
uses the API-key path, and an explicit API key (env var or pasted via `/login`)
always takes precedence over OAuth — so `/login` → pick the provider → paste a
key overrides a bad OAuth credential.

## Windows install (`catcode`)

`release-windows.sh` cross-compiles for Windows x86_64 and produces two
self-contained artifacts:

- **`catcode-<ver>-windows.msi`** — a per-user MSI installer that installs
  `catcode` + `catcode-core` to `%LOCALAPPDATA%\Programs\catcode` and adds that
  directory to the user PATH (so `catcode` works from any CWD, no admin needed).
- **`catcode-<ver>-windows-x86_64.exe`** — a single standalone executable with the
  Rust core embedded (`-tags embed_core`); no install, no separate
  `catcode-core`. Run it from any CWD — it extracts its bundled core to
  `%LOCALAPPDATA%\catalyst-code` on first run.

```bash
./release-windows.sh        # -> dist/catcode-<ver>-windows.msi + .sha256
                           #    dist/catcode-<ver>-windows-x86_64.exe + .sha256
                           #    dist/catcode-<ver>-windows.zip (no-build fallback)
```

`release-windows.sh` cross-compiles with cargo (`x86_64-pc-windows-gnu`) and Go
(`GOOS=windows`), then builds the MSI with msitools `wixl` from
`packaging/windows/catcode.wxs` (the same `.wxs` also compiles with the WiX
Toolset `candle`+`light` on a Windows build host).

On Windows, install by double-clicking the `.msi`, or silently:

```powershell
msiexec /i catcode-<ver>-windows.msi            # interactive (no UAC prompt)
msiexec /i catcode-<ver>-windows.msi /quiet     # silent
```

The MSI is per-user (no admin), writes a clean Add/Remove Programs entry,
and supports in-place upgrades (fixed `UpgradeCode`). Open a new PowerShell
window after install and run `catcode` from any directory. First run: `/login`
then `/model`.

Prefer no install? Run the standalone `.exe` from anywhere — double-click
`catcode-<ver>-windows-x86_64.exe` (or `.\catcode-<ver>-windows-x86_64.exe` in
PowerShell) and it launches in the current directory with the core embedded.

No `wixl`/WiX available? `packaging/windows/install.ps1` is a no-build fallback:
unzip the two `.exe` files beside it and run `.\install.ps1` to copy them into
`%LOCALAPPDATA%\Programs\catcode` and update the user PATH.

The TUI finds the core by searching, in order: `$CATCODE_CORE`, `catcode-core(.exe)`
next to the TUI, then the dev paths `core/target/release/core(.exe)`. Set
`CATCODE_CORE=<path>` to point at a custom core build.

Runtime caveats on Windows:
- The agent's `bash` tool needs bash on PATH (Git Bash or WSL); chat and the
  file tools (read/edit/write/grep/glob/list_dir) work without it.
- Sandboxing (`--sandbox firejail` / `--no-network`) is Linux-only; leave
  `/sandbox` set to `none`.

## macOS install (`catcode`)

`release-macos.sh` cross-compiles per arch (arm64 + x86_64) and produces two
self-contained artifacts:

- **`catcode-<ver>-macos-{arm64,x86_64}`** — a single standalone
  executable with the Rust core embedded (`-tags embed_core`); runs from any
  CWD, no install, no separate `catcode-core`. It extracts its bundled core to
  `~/Library/Caches/catalyst-code` on first run.
- **`catcode-<ver>-macos-{arm64,x86_64}.dmg`** — a disk-image installer wrapping
  that standalone executable. Mount it and double-click `Install catcode.command`
  to copy `catcode` onto your PATH (`/usr/local/bin`), then run `catcode` from any
  terminal.

Grab the matching arch from `dist/` (built by `./release-macos.sh`):

- `catcode-<ver>-macos-arm64` / `catcode-<ver>-macos-arm64.dmg`  — Apple Silicon (M-series)
- `catcode-<ver>-macos-x86_64` / `catcode-<ver>-macos-x86_64.dmg` — Intel

```bash
chmod +x catcode-0.2.0-macos-arm64
./catcode-0.2.0-macos-arm64      # launches in the current directory
# or: open catcode-0.2.0-macos-arm64.dmg, then double-click "Install catcode.command"
```

Then `/login`, `/model`, and type a prompt. The workspace is the directory
you launched from — rerun from another folder to work on a different project.

Build it yourself on Linux (zig is the macOS linker; no Xcode/SDK needed):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo install cargo-zigbuild          # and put zig 0.13+ on PATH
./release-macos.sh                    # -> dist/catcode-<ver>-macos-{arm64,x86_64} + .sha256
                                       #    dist/catcode-<ver>-macos-{arm64,x86_64}.dmg + .sha256
```

`release-macos.sh` cross-compiles the core with `cargo zigbuild` (pure-Rust
`rustls-tls`, so no macOS SDK) and the TUI with `GOOS=darwin`, embedding the
core via `go:embed` (`-tags embed_core`) so each standalone output is one file.
The `.dmg` is built with `hdiutil` (real UDIF) on macOS, or `xorriso` (an HFS+
hybrid image that mounts on macOS) when cross-built on Linux. The TUI resolves
the core as: `$CATCODE_CORE` → embedded extraction → the usual dev/installed
search, so dev builds and the Windows MSI layout are unchanged.

Runtime caveats on macOS:
- Sandboxing (`--sandbox firejail` / `--no-network`) is Linux-only; leave
  `/sandbox` set to `none`.
- The agent's `bash` tool needs `bash` on PATH (present by default on macOS).

## Linux install (`catcode`)

`release-linux.sh` builds for the host arch (x86_64 or aarch64) and produces two
self-contained artifacts:

- **`catcode-<ver>-linux-<arch>`** — a single standalone executable with
  the Rust core embedded (`-tags embed_core`); runs from any CWD, no install,
  no separate `catcode-core`. It extracts its bundled core to
  `~/.cache/catalyst-code` on first run.
- **`catcode-<ver>-<arch>.AppImage`** — a self-contained AppImage (squashfs
  payload) wrapping that standalone executable. Run it from any terminal with
  `./catcode-<ver>-<arch>.AppImage`; it launches the TUI in the current directory.
  `<arch>` is `x86_64` or `aarch64`.

```bash
./release-linux.sh          # -> dist/catcode-<ver>-linux-<arch> + .sha256
                            #    dist/catcode-<ver>-<arch>.AppImage + .sha256
```

Run either from any directory:

```bash
chmod +x catcode-0.2.0-linux-x86_64 && ./catcode-0.2.0-linux-x86_64
chmod +x catcode-0.2.0-x86_64.AppImage       && ./catcode-0.2.0-x86_64.AppImage
```

Or install either as a `catcode` command on your PATH (the AppImage is a single
ELF you can rename and place on PATH, just like the standalone):

```bash
sudo install -m 0755 catcode-0.2.0-x86_64.AppImage /usr/local/bin/catcode   # then run: catcode
```

Then `/login`, `/model`, and type a prompt. The workspace is the directory
you launched from — rerun from another folder to work on a different project.

`release-linux.sh` builds the core natively (`cargo --release`), embeds it into
the TUI via `go:embed` (`-tags embed_core`), then wraps that standalone binary
in an AppImage with `appimagetool` (fetched once to `~/.cache/appimagetool/`
if not on PATH; set `APPIMAGETOOL=<path>` to use a local copy). The AppImage
needs no install and no root; on headless/CI boxes without FUSE it is still
built (`APPIMAGE_EXTRACT_AND_RUN=1` runs appimagetool without FUSE).

Runtime caveats on Linux:
- Sandboxing (`--sandbox firejail` / `--no-network`) is Linux-only; set it in
  the TUI settings modal or pass `--sandbox firejail --no-network` to the core.
- The agent's `bash` tool needs `bash` on PATH (present by default).

## Web frontend (as a service)

The Next.js web frontend (`web/`) is the browser equivalent of the TUI — it
spawns one `catcode-core` and streams events to the browser over SSE. It can be
installed to run continuously as a background service on **all three platforms**.
Because it is a Next.js app it is **built from source** (run from a checkout of
this repo), and a `catcode-core` must already be installed for it to spawn.

### Linux (systemd) & macOS (launchd)

`install.sh` builds the TUI + core from source and, with `--with-web`, also
builds and installs the web frontend as a system service. It works on both
Linux and macOS (it detects the platform and uses the right service manager):

```bash
bash install.sh --with-web                 # builds catcode + catcode-core + web
# optional: --port 49283 --host 0.0.0.0 --prefix /usr/local/bin
bash install.sh --dry-run --with-web        # preview the plan without executing
bash install.sh --update                    # git pull + rebuild + restart the service
bash install.sh --uninstall                 # stop + remove everything
```

- **Linux** installs a `systemd` unit (`catalyst-code-web.service`) that starts
  at boot and auto-restarts on crash. Logs: `journalctl -u catalyst-code-web.service -f`.
- **macOS** installs a user `launchd` agent
  (`~/Library/LaunchAgents/com.catalyst-code.web.plist`) that starts at login and
  auto-restarts (`KeepAlive`). Logs: `~/Library/Logs/catalyst-code-web.log`.

On macOS you can install the TUI via the `.dmg` first and still run
`install.sh --with-web` afterwards (it detects the core on PATH), or let
`install.sh` build the TUI from source too — both work.

### Windows (NSSM service or scheduled task)

`packaging/windows/install-web.ps1` builds the web and installs it as a Windows
Service via [NSSM](https://nssm.cc) (preferred — starts at boot, auto-restarts,
runs with no user logged in) or, if NSSM isn't installed, as a Scheduled Task at
logon with a restart-loop wrapper (zero extra dependencies). Requires
`catcode-core.exe` already installed (MSI or `packaging/windows/install.ps1`):

```powershell
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1
# optional: -Port 49283 -BindHost 0.0.0.0
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1 -Uninstall
```

Logs: `%LOCALAPPDATA%\catalyst-code\catalyst-code-web.log`. For public exposure
bind `-BindHost 127.0.0.1` and put a TLS reverse proxy (Caddy/nginx/IIS) in front.

### Manual (any platform, no service wrapper)

```bash
cd sdk && bun install && bun run build      # build the SDK first (sdk/dist/)
cd ../web && bun install && bun run build   # build the Next.js app
PORT=49283 bun run start                    # -> http://localhost:49283
```

Set `CATCODE_CORE=<path to catcode-core>` if the core isn't found automatically
(it searches `CATCODE_CORE`, then a dev build, then `catcode-core` on `PATH`).

## Protocol

Core reads commands from stdin, writes events to stdout, one JSON object per line.

Commands (stdin):
```json
{"type":"init"}
{"type":"login","preset":"openai","api_key":"sk-..."}
{"type":"send","prompt":"...","model":"umans-glm-5.2","reasoning_effort":"high"}
{"type":"logout","provider":"openai"}
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
