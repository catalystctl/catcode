<p align="center">
  <img src="docs/logo.svg" width="200" alt="Catalyst Code logo" />
</p>

<h1 align="center">Catalyst Code</h1>

<p align="center">
  A production-grade, <strong>OpenAI-compatible</strong> coding-agent harness.<br>
  Native multi-provider — Umans · OpenAI · Gemini · Anthropic — with a human-in-the-loop approval gate.
</p>

<p align="center">
  <a href="https://github.com/catalystctl/catcode/releases"><img alt="version" src="https://img.shields.io/badge/version-0.2.0-ff9e28?style=flat-square"></a>
  <img alt="platforms" src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1a1716?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-ce422b?style=flat-square">
  <img alt="Go" src="https://img.shields.io/badge/Go-1.24.2%2B-00add8?style=flat-square">
  <img alt="license" src="https://img.shields.io/badge/license-MIT-ff9e28?style=flat-square">
</p>

---

## Overview

Catalyst Code is a self-hosted coding agent that runs against any OpenAI- or
Anthropic-compatible endpoint. Four cooperating components share one
newline-delimited JSON protocol over stdio:

| Component | Language | Role |
|:---|:---|:---|
| **`core/`** | Rust (async, tokio) | The engine — conversation, model streaming, an agentic tool loop with a human-in-the-loop approval gate, sessions, memory, plugins, and subagents. |
| **`tui/`** | Go · [Bubble Tea](https://github.com/charmbracelet/bubbletea) | The terminal interface (`catcode`). Spawns the core, streams events, renders approvals and metrics. |
| **`sdk/`** | TypeScript | A thin pi-compatible wrapper (`@catalyst-code/coding-agent`) so the web frontend can drive the core. |
| **`web/`** | Next.js 15 · React 19 | The browser equivalent of the TUI — an SSE bridge to one core process. |

> **v0.2.0** ships the production hardening layer: subagents + intercom,
> summarizing context compaction, session token budgets, `--sandbox firejail`
> + `--no-network`, persistent per-workspace sessions, vision input, core-crash
> auto-recovery, multi-provider `/login` (API key **and** OAuth), a Dockerfile,
> and cross-platform install scripts. Full history in
> [`CHANGELOG.md`](CHANGELOG.md).

---

## Table of contents

- [Installation](#installation)
  - [Linux & macOS — `install.sh` (recommended)](#linux--macos--installsh-recommended)
  - [Windows — MSI + `install-web.ps1`](#windows--msi--install-webps1)
  - [First run](#first-run)
  - [Web frontend (as a service)](#web-frontend-as-a-service)
  - [Prebuilt binaries (optional)](#prebuilt-binaries-optional)
- [Features](#features)
- [Providers and login](#providers-and-login)
- [Build from source](#build-from-source)
- [Architecture](#architecture)
- [Subagents and intercom](#subagents-and-intercom)
- [Releases](#releases)
- [Protocol](#protocol)
- [Testing](#testing)
- [Security and notes](#security-and-notes)
- [License](#license)

---

## Installation

The **recommended** path is the bundled install script — it builds from source
and installs `catcode` + `catcode-core` to your PATH, and can also install the
web frontend as a background service. Prebuilt binaries (AppImage / `.dmg` /
MSI) are available as a no-toolchain [alternative](#prebuilt-binaries-optional).

### Linux & macOS — `install.sh` (recommended)

`install.sh` builds the Rust core + Go TUI and installs them to your PATH. With
`--with-web` it also builds the Next.js web frontend and installs it as a system
service (**systemd** on Linux, **launchd** on macOS — auto-detected).

**Prerequisites:** Rust (stable), Go **1.24.2+**, and (for the web) [Bun](https://bun.sh)
or Node.js + npm. On macOS the Xcode Command Line Tools provide the C linker.

```bash
git clone https://github.com/catalystctl/catcode.git
cd catcode
bash install.sh                 # build + install catcode and catcode-core
bash install.sh --with-web      # …also build + install the web service
bash install.sh --dry-run        # preview the full plan, execute nothing
```

Then run `catcode` from any directory. The workspace is your current directory —
launch it from another folder to work on a different project.

<details>
<summary><strong>install.sh options</strong></summary>

| Option | Default | Description |
|:---|:---|:---|
| `--with-web` | off | Also build + install the web frontend service |
| `--repo <url>` | — | Clone `<url>` first, then install from it |
| `--prefix <dir>` | `/usr/local/bin` | Binary install directory |
| `--port <n>` | `49283` | Web service port |
| `--host <h>` | `0.0.0.0` | Web bind host |
| `--update` | — | `git pull` + rebuild + reinstall (+ restart the service) |
| `--uninstall` | — | Stop + remove binaries, service, and state |
| `--dry-run` | off | Print the plan, execute nothing |
| `-h`, `--help` | — | Show help |

</details>

| | Linux | macOS |
|:---|:---|:---|
| **TUI / core** | `catcode`, `catcode-core` → `$PREFIX` (sudo) | same |
| **Web service** | systemd unit `catalyst-code-web.service` — starts at boot, auto-restarts | launchd agent `~/Library/LaunchAgents/com.catalyst-code.web.plist` — starts at login, `KeepAlive` |
| **Web logs** | `journalctl -u catalyst-code-web.service -f` | `~/Library/Logs/catalyst-code-web.log` |

```bash
bash install.sh --update        # pull latest + rebuild + restart the service
bash install.sh --uninstall     # stop + remove binaries, service, and state
```

### Windows — MSI + `install-web.ps1`

Windows has no POSIX `install.sh`; use the PowerShell scripts instead.

**TUI** — install `catcode` + `catcode-core` via one of:

- **Per-user MSI** (`catcode-<ver>-windows.msi`) — double-click, or
  `msiexec /i catcode-<ver>-windows.msi`. Installs to
  `%LOCALAPPDATA%\Programs\catcode` and adds it to PATH. No admin, clean
  Add/Remove Programs entry, in-place upgrades.
- **Standalone `.exe`** (`catcode-<ver>-windows-x86_64.exe`) — core embedded, no
  install; run from any CWD.
- **No-build fallback** `packaging/windows/install.ps1` — copies two raw `.exe`
  files to PATH.

```powershell
msiexec /i catcode-<ver>-windows.msi            # interactive (no UAC)
msiexec /i catcode-<ver>-windows.msi /quiet     # silent
```

**Web service** — `packaging/windows/install-web.ps1` builds the web from source
and installs it as a **Windows Service via [NSSM](https://nssm.cc)** (starts at
boot, auto-restarts, runs with no user logged in) or, if NSSM isn't installed, a
**Scheduled Task at logon** with a restart-loop wrapper (zero extra deps).
Requires `catcode-core.exe` already installed (MSI or `install.ps1`).

```powershell
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1
# options: -Port 49283 -BindHost 0.0.0.0
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1 -Uninstall
```

Logs: `%LOCALAPPDATA%\catalyst-code\catalyst-code-web.log`.

### First run

```bash
catcode              # launches in the current directory (your workspace)
```

In the TUI:

- `/login` — pick a provider (Umans / OpenAI / Gemini / Anthropic). An API key in
  an env var logs in instantly; otherwise it prompts. Subscription accounts use
  OAuth (no key) — see [Providers and login](#providers-and-login).
- `/model [N|substr]` — list models, or switch (`/model 3`, `/model glm-5.2`).
- type a prompt to chat.
- `/help` — all commands.

### Web frontend (as a service)

The Next.js web frontend is the browser equivalent of the TUI — it spawns one
`catcode-core` and streams events to the browser over SSE. Because it is a
Next.js app it is **built from source** (run from a checkout of this repo), and a
`catcode-core` must already be installed for it to spawn.

| Platform | Install command | Service manager |
|:---|:---|:---|
| **Linux** | `bash install.sh --with-web` | systemd (boot-start) |
| **macOS** | `bash install.sh --with-web` | launchd (login-start) |
| **Windows** | `packaging/windows/install-web.ps1` | NSSM service, or scheduled task |

**Manual run (any platform, no service wrapper):**

```bash
cd sdk && bun install && bun run build      # build the SDK (sdk/dist/)
cd ../web && bun install && bun run build   # build the Next.js app
PORT=49283 bun run start                    # -> http://localhost:49283
```

Set `CATCODE_CORE=<path>` if the core isn't found automatically (it searches
`CATCODE_CORE`, then a dev build, then `catcode-core` on `PATH`).

> For public exposure, bind to `127.0.0.1` and put a TLS reverse proxy
> (Caddy / nginx / IIS) in front.

### Prebuilt binaries (optional)

No toolchain? Grab a prebuilt artifact (built by the `release-*.sh` scripts; see
[Releases](#releases)):

| Platform | Artifact | Run |
|:---|:---|:---|
| **Linux** | `catcode-<ver>-<arch>.AppImage` | `./catcode-<ver>-x86_64.AppImage` |
| **macOS** | `catcode-<ver>-macos-{arm64,x86_64}.dmg` | mount → "Install catcode.command" |
| **Windows** | `catcode-<ver>-windows.msi` | double-click / `msiexec` |

Each platform also ships a **standalone executable** with the Rust core embedded
(`-tags embed_core`) — one file, no install, run from any CWD.

---

## Features

### Safety

- **Workspace confinement** — every file op resolves against a workspace root;
  absolute paths, `..`, and symlink escapes are rejected. `bash` runs with
  `cwd = workspace`.
- **Human-in-the-loop approval** — destructive tools (`bash`, `write_file`,
  `edit`, …) require consent under the default `destructive` mode:
  `y` approve once · `a` approve and stop asking for this kind · `n` deny.
  Modes: `never` / `destructive` / `always`, switchable via `/approval`.
  Restricted paths (`.env`, `.git`, `.ssh`, …) are approval-gated for reads
  *and* writes.
- **Optional hard sandbox** (Linux) — `--sandbox firejail` wraps bash in a
  firejail profile (workspace + shell paths only, dropped caps/seccomp);
  `--no-network` adds `unshare -n`. The denylist is a tripwire on top.

### Robustness

- **HTTP retry/backoff** — 429, 5xx, and transport errors retried with
  exponential backoff (0.5s→8s), honoring `Retry-After`.
- **Idle stream timeout** — `--idle-timeout` (default 120s); a stuck stream
  aborts instead of hanging.
- **Context compaction** — at 70% of the model window, oldest tool results are
  dropped (system + recent turns kept) with **orphaned-tool-call sanitization**
  so a compacted history never sends `tool_calls` without matching results.
- **File-size guards** — `read_file` refuses >5 MiB / 10k lines (with
  `offset`/`limit` pagination); `grep`/`glob` cap results.
- **Crash-safe sessions** — append-only JSONL, fsync per message, atomic
  rewrites; core-crash auto-recovery on restart.

### Tooling

Search-and-replace `edit` (exact, unique, atomic, multi-op) · `grep` + `glob` ·
async `bash` (timeout, kill, denylist, 32 KB output cap) · `patch` ·
`diagnostics` (cargo check / tsc / go build / py_compile) · `fetch` (read-only
HTTP, egress-controlled) · `todo_write`/`todo_read` · `memory` · git tools ·
`spawn` · `subagent` delegation.

### Observability and persistence

JSONL debug log (`--debug-log`) · per-turn metrics (TTFT, elapsed, tokens
in/out, TPS) · per-workspace sessions under
`~/.config/catalyst-code/sessions/<hex(cwd)>/` with `/sessions` · `/new` ·
`/undo` · `/compact` · `/stats`.

---

## Providers and login

`/login` opens a picker of the bundled presets. You can be logged into several at
once; `/models` lists every provider's models (tagged `[umans]`, `[openai]`,
`[gemini]`, `[anthropic]`), and any model you pick routes that turn to its
endpoint.

| Preset | Kind | Endpoint | Key env var |
|:---|:---|:---|:---|
| **Umans (GLM-5.2)** | OpenAI | `api.code.umans.ai/v1` | `UMANS_API_KEY` |
| **OpenAI (Codex)** | OpenAI | `api.openai.com/v1` | `OPENAI_API_KEY` |
| **Google Gemini** | OpenAI (compat shim) | `generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` (or `GOOGLE_API_KEY`) |
| **Anthropic Claude** | Anthropic | `api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |

Keys are persisted per-provider (the env-var *name* is stored when a key came
from the environment, so the secret never lands in a config file).

### Subscription login (OAuth) — no API key

ChatGPT Plus/Pro (Codex), Google One AI (Gemini), and Claude Pro/Max are accessed
via **OAuth**, performed by `/login` itself (no official CLI needed):

- **Gemini** — authorization-code + PKCE + loopback-redirect (opens
  accounts.google.com). Reuses `gcloud auth application-default login` if present.
- **Anthropic Claude** — authorize + PKCE + loopback-redirect (opens claude.ai).
  Reuses the `claude` CLI token if present.
- **OpenAI Codex** — ⚠️ not yet wired (the ChatGPT token needs the Responses API,
  a different request shape). Codex stays on `OPENAI_API_KEY` for now.

Tokens are stored at `~/.config/catalyst-code/oauth/<id>.json` (`0600`) and
refreshed automatically. An explicit API key always takes precedence over OAuth.

---

## Build from source

For development (no install, run from the repo):

```bash
cd core && cargo build --release      # -> core/target/release/core
cd tui && go build -o catcode         # -> tui/catcode
./tui/catcode                          # the TUI finds ../core/target/release/core
```

Requires Rust (stable) and **Go 1.24.2+**. The web frontend needs Bun or Node.js
(see [Web frontend](#web-frontend-as-a-service)).

---

## Architecture

```
core/                 Rust async engine (stdio JSONL)
  src/main.rs         entry, State, turn loop, approval gate, compaction, ask
  src/provider.rs     OpenAI/Anthropic streaming, retry/backoff, model discovery, sanitize
  src/subagent.rs     subagent execution (single/parallel/chain), forked context, depth cap
  src/intercom.rs     peer intercom bus (contact_supervisor / intercom ask/receive/reply)
  src/plugins.rs      plugin manager + hooks (pre_*/post_*/lifecycle/pre_turn)
  src/protocol.rs     wire types (Command / Event) + line emit
  src/config.rs       CLI + env + JSON config, approval modes, providers
  src/workspace.rs    path confinement (absolute/.. /symlink rejection)
  src/tools.rs        tool schemas + classification + execution
  src/session.rs      append-only JSONL session persistence
  src/memory.rs       persistent memory store (injected into the system prompt)
  src/git_ctx.rs      git status/branch context for the system prompt
  src/vision.rs       vision model config + image attachment
  src/fetch_tool.rs   HTTP fetch tool (read-only, egress-controlled)
  src/oauth.rs        OAuth flows (Gemini, Anthropic, Codex)
  src/logging.rs      JSONL debug log + token estimation
  src/staging.rs      global default-file staging (~/.catalyst-code/)
tui/                  Go + Bubble Tea terminal UI (spawns the core)
sdk/                  TypeScript pi-compatible SDK wrapper
web/                  Next.js web frontend (SSE bridge) — see web/README.md
packaging/            per-platform install scripts + packaging (linux/ macos/ windows/)
.catalyst-code/       bundled agents, plugins, skills (shipped defaults)
.github/workflows/    CI (core clippy/test, tui vet/test/build + cross-compile, docker)
```

---

## Subagents and intercom

A port of [`pi-subagents`](https://github.com/nicobailon/pi-subagents) is built
into the core. The orchestrator delegates to focused child agents via the
`subagent` tool; children can prompt the orchestrator for decisions and talk to
peers over an in-process intercom bus.

**Built-in agents** (`.catalyst-code/agents/*.md`, overridable): `scout` ·
`researcher` · `planner` · `worker` · `reviewer` · `context-builder` · `oracle` ·
`delegate`.

**Execution modes:** single `{ agent, task }` · parallel `{ tasks, concurrency }`
· chain `{ chain: [...] }` (with `{previous}`/`{outputs.name}` templating), plus
management actions (`list`/`get`/`create`/`update`/`delete`/`status`/`interrupt`/
`resume`/`peek`/`steer`/`doctor`).

**Intercom:**

- `contact_supervisor({ reason: "need_decision", message })` — a subagent asks
  the orchestrator a blocking question (surfaces as a TUI prompt).
- `intercom({ action: "send"|"ask"|"receive"|"reply"|"targets", to, message })`
  — peer-to-peer messaging between parallel subagents.

**Slash commands:** `/run` · `/parallel` · `/chain` · `/subagents` ·
`/subagents-status`. Config lives under `subagents` in settings JSON
(`maxSubagentDepth`, `intercomBridge.mode`, `parallel.maxTasks`, …).

---

## Releases

`release-all.sh [version]` builds **all** distributable artifacts at once —
Windows MSI + standalone `.exe`, macOS standalone + `.dmg` (arm64 + x86_64),
Linux standalone + AppImage — running each platform script independently and
reporting per-platform pass/fail (a host with a partial toolchain still ships
what it can).

| Script | Outputs |
|:---|:---|
| `release-linux.sh` | standalone `catcode-<ver>-linux-<arch>` + `.AppImage` |
| `release-macos.sh` | standalone `catcode-<ver>-macos-{arm64,x86_64}` + `.dmg` (cross-compiles via `cargo zigbuild`) |
| `release-windows.sh` | `catcode-<ver>-windows.msi` + standalone `.exe` (cross-compiles `x86_64-pc-windows-gnu`) |

Each standalone embeds the Rust core via `go:embed` (`-tags embed_core`) so it's
one self-contained file. Artifacts land in `dist/` with `.sha256` checksums.

---

## Protocol

Core reads commands from stdin and writes events to stdout — one JSON object per
line.

```json
{"type":"init"}
{"type":"login","preset":"openai","api_key":"sk-..."}
{"type":"send","prompt":"...","model":"umans-glm-5.2","reasoning_effort":"high"}
{"type":"steer","prompt":"..."}
{"type":"abort"}
{"type":"approve","request_id":"<id>","decision":"yes|no|always"}
{"type":"set_approval","mode":"never|destructive|always"}
```

**Events:** `ready` · `authed` · `thinking` · `delta` · `tool_call_start` ·
`tool_call_name` · `tool_call_args` · `tool_call` · `approval_request` ·
`tool_result` · `compacted` · `http_retry` · `metrics` · `approval_changed` ·
`done` · `aborted` · `reset` · `error` (plus subagent / memory / session events).

---

## Testing

```bash
cd core && cargo test --locked     # 314 unit tests (edit, confinement, bash, sanitize, session, …)
cd tui && go test ./...            # TUI tests (handlers, blocks, mention, modal, intercom)
```

CI (`.github/workflows/ci.yml`) runs core clippy/test, tui vet/test/build, a Go
cross-compile matrix (linux / darwin / windows), and a Docker image build.

---

## Security and notes

- **OpenAI-compatible** — point `--base-url` at any OpenAI-shaped endpoint.
  Umans-specific logic (GLM `reasoning_effort=high` clamp, `reasoning_content`
  replay, `/models/info` discovery) is isolated to `provider.rs`.
- **No fixed turn cap** — the ceiling is the session token budget
  (`--max-session-tokens`, `0` = unlimited). The model can call `finish` to exit
  cleanly or `spawn` a nested sub-agent.
- **Hard security boundary** — pass `--sandbox firejail --no-network` (or set in
  the TUI settings modal). The denylist is a tripwire on top; workspace
  confinement covers file paths, but `bash` is only sandboxed when `--sandbox` is
  set. Sandboxing is **Linux-only**; on macOS/Windows leave it `none`.
- **Windows bash** — the agent's `bash` tool needs bash on PATH (Git Bash or
  WSL); chat and the file tools work without it.

---

## License

[MIT](LICENSE) © karutoil
