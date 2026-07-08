<p align="center">
  <img src="docs/logo.svg" width="200" alt="Catalyst Code logo" />
</p>

<h1 align="center">Catalyst Code</h1>

<p align="center">
  A self-hosted, <strong>OpenAI-compatible</strong> coding-agent harness.<br>
  One binary, any provider — Umans · OpenAI · Gemini · Anthropic — with a human-in-the-loop approval gate.
</p>

<p align="center">
  <a href="https://github.com/catalystctl/catcode/releases"><img alt="version" src="https://img.shields.io/badge/version-0.2.0-ff9e28?style=flat-square"></a>
  <img alt="platforms" src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1a1716?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-ce422b?style=flat-square">
  <img alt="Go" src="https://img.shields.io/badge/Go-1.24%2B-00add8?style=flat-square">
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-ff9e28?style=flat-square"></a>
</p>

---

## About

Catalyst Code is a coding agent you run on your own machine against **any
OpenAI- or Anthropic-compatible endpoint** — a cloud API, a local model, or a
self-hosted gateway. It is not a hosted service and does not phone home: your
code, prompts, and API key stay on your box.

It exists because most agent tooling is locked to one provider, runs as a hosted
SaaS, or ships as an opaque binary. Catalyst Code is the opposite — a small set
of readable components sharing one newline-delimited JSON protocol, with a real
safety model (workspace confinement + an approval gate) and first-class
subagents. You can drive it from the terminal, the browser, or your own code.

## Features

**Multi-provider, no lock-in** — one `/login` picker for Umans, OpenAI, Gemini,
and Anthropic. Be logged into several at once; any model you pick routes that
turn to its endpoint. API key *and* subscription OAuth (no key) supported.

**Human-in-the-loop safety** — destructive tools (`bash`, `write_file`,
`edit`, …) require consent under the default `destructive` mode. Restricted
paths (`.env`, `.git`, `.ssh`) are gated for reads *and* writes. Optional Linux
hard sandbox: `--sandbox firejail --no-network`.

**Workspace confinement** — every file op resolves against a workspace root;
absolute paths, `..`, and symlink escapes are rejected. `bash` runs with
`cwd = workspace`.

**Built-in subagents** — delegate to focused child agents (`scout`,
`planner`, `worker`, `reviewer`, …) over single / parallel / chain execution,
with a peer intercom bus for coordination.

**Robust by default** — HTTP retry/backoff, idle-stream timeout, summarizing
context compaction with orphaned-tool-call sanitization, fsync'd append-only
sessions, and core-crash auto-recovery.

**Pluggable** — bundled agents, skills, and hook-based plugins (pre/post +
lifecycle + `pre_turn` model handoff) live in [`.catalyst-code/`](.catalyst-code).
Plugins can even declare custom tools without MCP or a recompile.

**Full toolset** — `edit`/`patch`/`grep`/`glob`, async `bash`, `diagnostics`
(cargo / tsc / go / py), `fetch`, `todo`, `memory`, git tools, `spawn`, and
`subagent` delegation.

## Installation

### Quick start (Linux & macOS)

```bash
bash install.sh            # downloads prebuilt catcode + catcode-core to PATH
catcode                    # launches in your current directory
```

No compiler needed — the installer pulls prebuilt binaries from GitHub
Releases. Run `catcode` from any folder to work on that project.

```bash
bash install.sh --with-web      # also install the web frontend as a service
bash install.sh --version 0.2.0 # pin a release
bash install.sh --dry-run       # preview the plan, execute nothing
```

### Requirements

- **Linux / macOS / Windows** (sandbox is Linux-only; on macOS/Windows leave it `none`).
- `curl` + coreutils (always). No compiler unless you build from source.
- For `--with-web`: a [Node](https://nodejs.org) or [Bun](https://bun.sh) runtime to *run* the service (not to build it).
- To build from source: Rust (stable) + **Go 1.24+**.

<details>
<summary><strong>install.sh options</strong></summary>

| Option | Default | Description |
|:---|:---|:---|
| `--with-web` | off | Also download + install the web frontend service |
| `--version <v>` | latest | Pin a release (e.g. `0.2.0` or `v0.2.0`) |
| `--base-url <url>` | GitHub Releases | Download from a mirror instead of GitHub |
| `--build-from-source` | off | Compile locally (cargo + go + next build) instead of downloading |
| `--web-dir <path>` | `/opt/catalyst-code/web` (Linux), `~/Library/Application Support/catalyst-code/web` (macOS) | Web bundle install dir |
| `--prefix <dir>` | `/usr/local/bin` | Binary install directory |
| `--port <n>` | `49283` | Web service port |
| `--host <h>` | `0.0.0.0` | Web bind host |
| `--repo <url>` | — | Clone `<url>` first, then install from it |
| `--update` | — | Re-download latest + reinstall (+ restart the service) |
| `--uninstall` | — | Stop + remove binaries, service, and state |
| `--dry-run` | off | Print the plan, execute nothing |

</details>

### Windows

No POSIX `install.sh` on Windows — use the PowerShell / MSI paths.

**TUI** — install `catcode` + `catcode-core` via any of:

- **Per-user MSI** (`catcode-<ver>-windows.msi`) — no admin, clean
  Add/Remove Programs entry, in-place upgrades:
  ```powershell
  msiexec /i catcode-<ver>-windows.msi            # interactive (no UAC)
  msiexec /i catcode-<ver>-windows.msi /quiet     # silent
  ```
- **Standalone `.exe`** (`catcode-<ver>-windows-x86_64.exe`) — core embedded,
  no install; run from any directory.
- **No-build fallback** `packaging/windows/install.ps1` — copies two raw
  `.exe` files to PATH.

**Web service** — `packaging/windows/install-web.ps1` downloads a prebuilt web
bundle (and `catcode-core.exe` if missing) and installs it as a **Windows
Service via [NSSM](https://nssm.cc)** (boot-start, auto-restart) or, without
NSSM, a **logon Scheduled Task** with a restart-loop wrapper. Needs Node or
Bun to run.

```powershell
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1
# options: -Port 49283 -BindHost 0.0.0.0
pwsh -ExecutionPolicy Bypass -File packaging\windows\install-web.ps1 -Uninstall
```

### Prebuilt binaries (standalone, no installer)

| Platform | Artifact | Run |
|:---|:---|:---|
| **Linux** | `catcode-<ver>-<arch>.AppImage` | `./catcode-<ver>-x86_64.AppImage` |
| **macOS** | `catcode-<ver>-macos-{arm64,x86_64}.dmg` | mount → "Install catcode.command" |
| **Windows** | `catcode-<ver>-windows.msi` | double-click / `msiexec` |

Each platform also ships a **standalone executable** with the Rust core embedded
(`-tags embed_core`) — one file, no install, run from any directory.

> **Private repo?** Anonymous download works once the repo is **public**. If it
> is private, point the installer at a public mirror with `--base-url <url>`
> (Linux/macOS) / `-BaseUrl <url>` (Windows), or [build from source](#contributing).

## Usage

### Terminal — first run

```bash
catcode            # launches in the current directory (your workspace)
```

In the TUI:

- `/login` — pick a provider. An API key in an env var logs in instantly;
  subscription accounts use OAuth (no key).
- `/model [N|substr]` — list models, or switch (`/model 3`, `/model glm-5.2`).
- `/approval never|destructive|always` — change the safety gate.
- type a prompt to chat. `/help` lists every command.

### Web frontend

The Next.js web app is the browser equivalent of the TUI — it spawns one
`catcode-core` and streams events to the browser over SSE. The installer
downloads a **prebuilt** standalone bundle (no `next build` on the host); it only
needs a Node or Bun runtime to run.

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

## Providers and login

`/login` opens a picker of the bundled presets. You can be logged into several
at once; `/models` lists every provider's models (tagged `[umans]`, `[openai]`,
`[gemini]`, `[anthropic]`), and any model you pick routes that turn to its
endpoint.

| Preset | Wire | Endpoint | Key env var |
|:---|:---|:---|:---|
| **Umans (GLM-5.2)** | OpenAI | `api.code.umans.ai/v1` | `UMANS_API_KEY` |
| **OpenAI (Codex)** | OpenAI | `api.openai.com/v1` | `OPENAI_API_KEY` |
| **Google Gemini** | OpenAI (compat shim) | `generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` (or `GOOGLE_API_KEY`) |
| **Anthropic Claude** | Anthropic | `api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |

Keys are persisted per-provider (the env-var *name* is stored when a key came
from the environment, so the secret never lands in a config file).

### Subscription login (OAuth) — no API key

ChatGPT Plus/Pro (Codex), Google One AI (Gemini), and Claude Pro/Max are reached
via **OAuth**, performed by `/login` itself (no official CLI needed):

- **Gemini** — authorization-code + PKCE + loopback redirect. Reuses
  `gcloud auth application-default login` if present.
- **Anthropic Claude** — authorize + PKCE + loopback redirect. Reuses the
  `claude` CLI token if present.
- **OpenAI Codex** — ⚠️ not yet wired (the ChatGPT token needs the Responses
  API, a different request shape). Codex stays on `OPENAI_API_KEY` for now.

Tokens are stored at `~/.config/catalyst-code/oauth/<id>.json` (`0600`) and
refreshed automatically. An explicit API key always takes precedence over OAuth.

## Architecture

Four cooperating components around one stdio JSONL protocol:

| Component | Language | Role |
|:---|:---|:---|
| **`core/`** | Rust (async, tokio) | The engine — conversation, model streaming, the agentic tool loop with an approval gate, sessions, memory, plugins, and subagents. |
| **`tui/`** | Go · [Bubble Tea](https://github.com/charmbracelet/bubbletea) | The terminal interface (`catcode`). Spawns the core, streams events, renders approvals and metrics. |
| **`sdk/`** | TypeScript | A thin pi-compatible wrapper (`@catalyst-code/coding-agent`) so the web frontend can drive the core. |
| **`web/`** | Next.js 15 · React 19 | The browser equivalent of the TUI — an SSE bridge to one core process. See [`web/README.md`](web/README.md). |

```
core/        Rust async engine (stdio JSONL)   tui/   Go + Bubble Tea terminal UI
sdk/         TypeScript pi-compatible wrapper   web/   Next.js web frontend (SSE bridge)
packaging/   per-platform install scripts       .catalyst-code/   bundled agents, plugins, skills
```

<details>
<summary><strong>core/ source layout</strong></summary>

```
src/main.rs       entry, State, turn loop, approval gate, compaction, ask
src/provider.rs   OpenAI/Anthropic streaming, retry/backoff, model discovery, sanitize
src/subagent.rs   subagent execution (single/parallel/chain), forked context, depth cap
src/intercom.rs   peer intercom bus (contact_supervisor / intercom ask/receive/reply)
src/plugins.rs    plugin manager + hooks (pre_*/post_*/lifecycle/pre_turn)
src/protocol.rs   wire types (Command / Event) + line emit
src/config.rs     CLI + env + JSON config, approval modes, providers
src/workspace.rs   path confinement (absolute/.. /symlink rejection)
src/tools.rs      tool schemas + classification + execution
src/session.rs    append-only JSONL session persistence
src/memory.rs     persistent memory store (injected into the system prompt)
src/git_ctx.rs    git status/branch context for the system prompt
src/vision.rs     vision model config + image attachment
src/fetch_tool.rs HTTP fetch tool (read-only, egress-controlled)
src/oauth.rs      OAuth flows (Gemini, Anthropic, Codex)
src/logging.rs    JSONL debug log + token estimation
src/staging.rs    global default-file staging (~/.catalyst-code/)
```

</details>

## Subagents and intercom

A port of [`pi-subagents`](https://github.com/nicobailon/pi-subagents) is built
into the core. The orchestrator delegates to focused child agents via the
`subagent` tool; children can prompt the orchestrator for decisions and talk to
peers over an in-process intercom bus.

**Built-in agents** (`.catalyst-code/agents/*.md`, overridable): `scout` ·
`researcher` · `planner` · `worker` · `reviewer` · `context-builder` · `oracle` ·
`delegate`.

**Execution modes:**

```ts
{ agent: "worker", task: "refactor auth" }                 // single
{ tasks: [{ agent: "scout", task: "a" }, { ... }], concurrency: 2 }  // parallel
{ chain: [{ agent: "scout" }, { agent: "planner" }, { agent: "worker" }] }  // chain
// management: list / get / create / status / interrupt / resume / peek / steer
```

**Intercom:**

- `contact_supervisor({ reason: "need_decision", message })` — a subagent asks
  the orchestrator a blocking question (surfaces as a TUI prompt).
- `intercom({ action: "send"|"ask"|"receive"|"reply"|"targets", to, message })`
  — peer-to-peer messaging between parallel subagents.

## Contributing

Contributions are welcome. This is a young project — issues and PRs that improve
safety, provider coverage, or docs are especially useful.

### Development setup

Run from the repo (no install needed):

```bash
cd core && cargo build --release      # -> core/target/release/core
cd tui && go build -o catcode         # -> tui/catcode
./tui/catcode                          # finds ../core/target/release/core
```

Requires Rust (stable) and **Go 1.24+**. The web frontend needs Bun or Node.js
(see [Usage → Web frontend](#web-frontend)).

### Running tests

```bash
cd core && cargo test --locked     # 300+ unit tests (edit, confinement, bash, sanitize, session, …)
cd tui && go test ./...            # TUI tests (handlers, blocks, mention, modal, intercom)
```

CI (`.github/workflows/ci.yml`) runs core clippy/test, tui vet/test/build, a Go
cross-compile matrix (linux / darwin / windows), and a Docker image build.

### Wire protocol

Core reads commands from stdin and writes events to stdout — one JSON object per
line. This is the integration point for alternative frontends:

```json
{"type":"init"}
{"type":"login","preset":"openai","api_key":"sk-..."}
{"type":"send","prompt":"...","model":"umans-glm-5.2","reasoning_effort":"high"}
{"type":"steer","prompt":"..."}
{"type":"approve","request_id":"<id>","decision":"yes|no|always"}
```

**Events:** `ready` · `authed` · `thinking` · `delta` · `tool_call_start` ·
`tool_call_name` · `tool_call_args` · `tool_call` · `approval_request` ·
`tool_result` · `compacted` · `http_retry` · `metrics` · `approval_changed` ·
`done` · `aborted` · `reset` · `error` (plus subagent / memory / session events).

## Releases

`release-all.sh [version]` builds **all** distributable artifacts at once and
reports per-platform pass/fail (a host with a partial toolchain still ships what
it can). All artifacts are published to a GitHub Release by
`.github/workflows/release.yml` on a `v*` tag push; the installers download them
so users never compile. Each standalone embeds the Rust core via `go:embed`
(`-tags embed_core`); the separate `catcode-core-*` binaries are for the web
service's `CATCODE_CORE`.

<details>
<summary><strong>Release scripts &amp; outputs</strong></summary>

| Script | Outputs |
|:---|:---|
| `release-linux.sh` | standalone `catcode-<ver>-linux-<arch>` + `.AppImage` + `catcode-core-<ver>-linux-<arch>` |
| `release-macos.sh` | standalone `catcode-<ver>-macos-{arm64,x86_64}` + `.dmg` + `catcode-core-<ver>-macos-{arch}` (cross-compiles via `cargo zigbuild`) |
| `release-windows.sh` | `catcode-<ver>-windows.msi` + standalone `.exe` + `.zip` + `catcode-core-<ver>-windows-x86_64.exe` (cross-compiles `x86_64-pc-windows-gnu`) |
| `release-web.sh` | `catcode-web-<ver>.tar.gz` — prebuilt Next.js standalone bundle (one cross-platform tarball) |

Artifacts land in `dist/` with `.sha256` checksums.

</details>

## Security notes

- **OpenAI-compatible** — point `--base-url` at any OpenAI-shaped endpoint.
  Provider-specific logic (GLM `reasoning_effort=high` clamp,
  `reasoning_content` replay, `/models/info` discovery) is isolated to
  `provider.rs`.
- **No fixed turn cap** — the ceiling is the session token budget
  (`--max-session-tokens`, `0` = unlimited). The model can call `finish` to exit
  cleanly or `spawn` a nested agent.
- **Hard security boundary** — pass `--sandbox firejail --no-network` (or set it
  in the TUI settings). The denylist is a tripwire on top; workspace confinement
  covers file paths, but `bash` is only sandboxed when `--sandbox` is set.
  Sandboxing is **Linux-only**.
- **Windows bash** — the agent's `bash` tool needs bash on PATH (Git Bash or
  WSL); chat and the file tools work without it.

## Roadmap

- [ ] Wire OpenAI Codex subscription OAuth (Responses API).
- [ ] More provider presets (local gateways, additional OAuth flows).
- [ ] macOS/Windows sandboxing options.
- [ ] Broader plugin / skill ecosystem.

## Acknowledgments

- [pi-subagents](https://github.com/nicobailon/pi-subagents) — the subagent +
  intercom design this project's orchestration is ported from.
- [Bubble Tea](https://github.com/charmbracelet/bubbletea) (TUI),
  [Next.js](https://nextjs.org) (web), and the Rust async ecosystem.

## License

[MIT](LICENSE) © karutoil
