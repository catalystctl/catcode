<a id="readme-top"></a>

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![MIT License][license-shield]][license-url]
[![Website][website-shield]][website-url]

<br />
<div align="center">
  <a href="https://github.com/catalystctl/catcode">
    <img src="docs/logo.svg" alt="Catalyst Code logo" width="120" height="120">
  </a>

  <h3 align="center">Catalyst Code</h3>

  <p align="center">
    A self-hosted, OpenAI-compatible coding-agent harness — one binary, any provider, with a human-in-the-loop approval gate.
    <br />
    <a href="https://github.com/catalystctl/catcode/releases"><strong>Releases »</strong></a>
    <br />
    <br />
    <a href="https://github.com/catalystctl/catcode">View Demo</a>
    &middot;
    <a href="https://github.com/catalystctl/catcode/issues/new?labels=bug&template=bug-report.md">Report Bug</a>
    &middot;
    <a href="https://github.com/catalystctl/catcode/issues/new?labels=enhancement&template=feature-request.md">Request Feature</a>
  </p>
</div>

<details>
  <summary>Table of Contents</summary>
  <ol>
    <li>
      <a href="#about-the-project">About The Project</a>
      <ul>
        <li><a href="#built-with">Built With</a></li>
      </ul>
    </li>
    <li>
      <a href="#getting-started">Getting Started</a>
      <ul>
        <li><a href="#prerequisites">Prerequisites</a></li>
        <li><a href="#installation">Installation</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#providers-and-login">Providers and Login</a></li>
    <li><a href="#architecture">Architecture</a></li>
    <li><a href="#subagents-and-intercom">Subagents and Intercom</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

## About The Project

Catalyst Code is a coding agent you run on your own machine against **any
OpenAI- or Anthropic-compatible endpoint** — a cloud API, a local model, or a
self-hosted gateway. It is not a hosted service and does not phone home: your
code, prompts, and API key stay on your box.

It exists because most agent tooling is locked to one provider, runs as a hosted
SaaS, or ships as an opaque binary. Catalyst Code is the opposite — a small set
of readable components sharing one newline-delimited JSON protocol, with a real
safety model (workspace confinement + an approval gate) and first-class
subagents. You can drive it from the terminal, the browser, or your own code.

Key highlights:

* **Multi-provider, no lock-in** — one `/login` picker for Umans, OpenCode Go,
  and OpenRouter. Be logged into several at once; any model you pick routes that
  turn to its endpoint. Other vendors ship as plugins (API key and/or OAuth).
* **Human-in-the-loop safety** — destructive tools (`bash`, `write_file`,
  `edit`, …) require consent under the default `destructive` mode. Restricted
  paths (`.env`, `.git`, `.ssh`) are gated for reads *and* writes. Optional Linux
  hard sandbox: `--sandbox firejail --no-network`.
* **Workspace confinement** — every file op resolves against a workspace root;
  absolute paths, `..`, and symlink escapes are rejected. `bash` runs with
  `cwd = workspace`.
* **Built-in subagents** — delegate to focused child agents (`scout`,
  `planner`, `worker`, `reviewer`, …) over single / parallel / chain execution,
  with a peer intercom bus for coordination.
* **Robust by default** — HTTP retry/backoff, idle-stream timeout, summarizing
  context compaction with orphaned-tool-call sanitization, fsync'd append-only
  sessions, and core-crash auto-recovery.
* **Pluggable** — bundled agents, skills, and hook-based plugins (pre/post +
  lifecycle + `pre_turn` model handoff) live in [`.catalyst-code/`](.catalyst-code).
  Plugins can even declare custom tools without MCP or a recompile.
* **Full toolset** — `edit`/`patch`/`grep`/`glob`, async `bash`, `diagnostics`
  (cargo / tsc / go / py), `fetch`, `todo`, `memory`, git tools, `spawn`, and
  `subagent` delegation.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Built With

* [![Rust][Rust-badge]][Rust-url] — async engine (`core/`)
* [![Go][Go-badge]][Go-url] — terminal UI (`tui/`, Bubble Tea)
* [![Next.js][Next.js-badge]][Next.js-url] — web frontend (`web/`, React 19)
* [![TypeScript][TypeScript-badge]][TypeScript-url] — SDK wrapper (`sdk/`)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Getting Started

Get up and running in under a minute — no clone, no compiler.

### Prerequisites

* **Linux / macOS / Windows** (the hard sandbox is Linux-only; on macOS/Windows
  leave it `none`).
* `curl` + coreutils (always). No compiler unless you build from source.
* For the web frontend: a [Node](https://nodejs.org) or [Bun](https://bun.sh)
  runtime to *run* the service (not to build it).
* To build from source: Rust (stable) + **Go 1.25+**.

### Installation

The recommended setup is the web app. It is a one-line install with **no clone
and no compiler**; the installer downloads a prebuilt bundle and configures a
background service.

**Linux & macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

Then open `http://localhost:49283`. Pass service options after `bash -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash -s -- --port 8080 --host 127.0.0.1
```

The terminal-only install remains available:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh | bash
```

Other options:

```bash
... | bash -s -- --version 0.2.0   # pin a release
... | bash -s -- --dry-run          # preview the plan, execute nothing
... | bash -s -- --uninstall        # remove everything
```

<details>
<summary><strong>install.sh options</strong></summary>

| Option | Default | Description |
|:---|:---|:---|
| `--with-web` | off | Also install the web frontend (automatically set by `install-web.sh`) |
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

**Windows web app:**

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/packaging/windows/install-web.ps1)))
```

This installs the core and prebuilt web bundle, then runs it through NSSM when
available or a login Scheduled Task. To install only the terminal app instead:

```powershell
irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1 | iex
```

No admin, no compiler. Open a NEW PowerShell window (so PATH reloads) and run
`catcode`. Web installer options include `-Version`, `-BaseUrl`, `-Port`,
`-BindHost`, `-WebDir`, and `-Uninstall`.

<details>
<summary><strong>Windows alternatives (MSI / standalone .exe)</strong></summary>

* **Per-user MSI** (`catcode-<ver>-windows.msi`) — no admin, clean
  Add/Remove Programs entry, in-place upgrades:
  ```powershell
  msiexec /i catcode-<ver>-windows.msi            # interactive (no UAC)
  msiexec /i catcode-<ver>-windows.msi /quiet     # silent
  ```
* **Standalone `.exe`** (`catcode-<ver>-windows-x86_64.exe`) — core embedded,
  no install; run from any directory.

</details>

> **Private repo?** The one-liners above fetch the installers from
> `raw.githubusercontent.com`, which works once the repo is **public**. If it
> is private, clone the repo and run `bash install.sh` / `pwsh -File install.ps1`
> locally, or point the installer at a public mirror with `--base-url <url>`
> (Linux/macOS) / `-BaseUrl <url>` (Windows).

<details>
<summary><strong>Prebuilt binaries (standalone, no installer)</strong></summary>

| Platform | Artifact | Run |
|:---|:---|:---|
| **Linux** | `catcode-<ver>-<arch>.AppImage` | `./catcode-<ver>-x86_64.AppImage` |
| **macOS** | `catcode-<ver>-macos-{arm64,x86_64}.dmg` | mount → "Install catcode.command" |
| **Windows** | `catcode-<ver>-windows.msi` | double-click / `msiexec` |

Each platform also ships a **standalone executable** with the Rust core embedded
(`-tags embed_core`) — one file, no install, run from any directory.

</details>

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Usage

### Terminal — first run

```bash
catcode            # launches in the current directory (your workspace)
```

In the TUI:

* `/login` — pick a provider. An API key in an env var logs in instantly;
  subscription accounts use OAuth (no key).
* `/model [N|substr]` — list models, or switch (`/model 3`, `/model glm-5.2`).
* `/approval` — open the approval-mode picker (`never` · `destructive` · `always`); or `/approval always` to set directly.
* `/settings` — settings hub; each option opens its own dedicated modal (`/theme`, `/sandbox`, `/mouse-wheel`, …).
* `/goal` — goal mode: multi-field modal for a high-level objective, concurrency,
  and model/provider allowlists. The core plans (`goal_write_plan`) then deploys
  subagents under those caps. `/cancel-goal` aborts. Optional “review plan before
  deploy” stops at plan-ready for approve/revise. Concurrency 8+ automatically
  uses the ultra-parallel planning profile: broad independent fan-out first,
  with chains retained only for real dependencies.
* type a prompt to chat. `/help` lists every command.
* `!command` runs bash and adds the output to model context; `!!command` runs without adding it (PI-compatible).

### Web frontend

The Next.js web app is the browser equivalent of the TUI — it spawns one
`catcode-core` and streams events to the browser over SSE. The installer
downloads a **prebuilt** standalone bundle (no `next build` on the host); it only
needs a Node or Bun runtime to run.

| Platform | Install command | Service manager |
|:---|:---|:---|
| **Linux** | `curl -fsSL .../install.sh \| bash -s -- --with-web` | systemd (boot-start) |
| **macOS** | `curl -fsSL .../install.sh \| bash -s -- --with-web` | launchd (login-start) |
| **Windows** | `& ([scriptblock]::Create((irm .../install.ps1))) -WithWeb` | NSSM service, or scheduled task |

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

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Providers and Login

`/login` opens a picker of the bundled presets. You can be logged into several
at once; `/model` lists every provider's models (tagged `[umans]`,
`[opencode-go]`, `[openrouter]`, …), and any model you pick routes that turn to
its endpoint. Everything else is a plugin.

<details>
<summary><strong>Provider presets</strong></summary>

| Preset | Wire | Endpoint | Key env var |
|:---|:---|:---|:---|
| **Umans (GLM-5.2)** | OpenAI | `api.code.umans.ai/v1` | `UMANS_API_KEY` |
| **OpenCode Go** | OpenAI + Anthropic | `opencode.ai/zen/go/v1` | `OPENCODE_GO_API_KEY` |
| **OpenRouter** | OpenAI | `openrouter.ai/api/v1` | `OPENROUTER_API_KEY` |

Keys are persisted per-provider (the env-var *name* is stored when a key came
from the environment, so the secret never lands in a config file).

</details>

### Subscription login (OAuth) — plugins only

Built-in presets are **API-key only**. ChatGPT Plus/Pro, Claude Pro/Max,
SuperGrok, and similar subscription logins live in **plugins** that declare an
`oauth` block in `plugin.json`. The harness still owns `/login` / `/oauth-code`
loopback + paste UX; the plugin script owns authorize/token/refresh.

Example (ChatGPT / Codex):

```text
/plugin-install ~/catcode-chatgpt-provider global
/login chatgpt
```

See [docs/examples/plugins/grok-oauth](docs/examples/plugins/grok-oauth) for the
script contract, and the plugin-authoring skill for the full OAuth schema.
An explicit API key always takes precedence over a plugin OAuth token.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

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
src/worktree.rs   git worktree isolation + promote for parallel agents
src/checkpoint.rs hybrid FS checkpoints (git stash ref or file snapshot)
src/audit.rs      optional security audit sidecar
src/embed.rs      hashing-sketch memory recall (Milestone 4)
src/intercom.rs   peer intercom bus (contact_supervisor / intercom ask/receive/reply)
src/plugins.rs    plugin manager + hooks (pre_*/post_*/lifecycle/pre_turn)
src/protocol.rs   wire types (Command / Event) + line emit
src/config.rs     CLI + env + JSON config, approval modes, providers, routing
src/workspace.rs   path confinement (absolute/.. /symlink rejection)
src/tools.rs      tool schemas + classification + execution
src/session.rs    append-only JSONL session persistence
src/memory.rs     persistent memory store (injected into the system prompt)
src/git_ctx.rs    git status/branch context for the system prompt
src/vision.rs     vision model config + image attachment
src/fetch_tool.rs HTTP fetch tool (read-only, egress-controlled NetworkPolicy)
src/oauth.rs      Plugin OAuth plumbing (loopback, enrich, PendingOauth)
src/logging.rs    JSONL debug log + token estimation
src/staging.rs    global default-file staging (~/.catalyst-code/)
```

</details>

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Subagents and Intercom

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

* `contact_supervisor({ reason: "need_decision", message })` — a subagent asks
  the orchestrator a blocking question (surfaces as a TUI prompt).
* `intercom({ action: "send"|"ask"|"receive"|"reply"|"targets", to, message })`
  — peer-to-peer messaging between parallel subagents.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Roadmap

- [x] Multi-provider login (Umans, OpenCode Go, OpenRouter)
- [x] Subagents + intercom bus
- [x] Plugin system (hooks + custom tools, no MCP)
- [x] Plugin OAuth providers (ChatGPT, SuperGrok, …)
- [x] Plugin slash commands, notify/status, and `/plugin-reload`
- [x] macOS seatbelt sandbox mode (Windows remains denylist-only)
- [x] Git worktree isolation for parallel subagents + shadow promote
- [x] Hybrid filesystem checkpoints + Undo restores disk
- [x] `file_change` / `protocol_hello` / richer approvals / audit sidecar
- [x] Task-aware model routing by agent role
- [x] Goal speculative scout + verifier loop
- [x] Embedding-sketch memory recall + speculative readonly prefetch
- [ ] Broader plugin / skill ecosystem (catalog + more reference plugins)
- [ ] Additional gateway / OAuth providers as plugins as demand appears
- [ ] MCP compatibility as a **plugin adapter** (core stays MCP-free)

See the [open issues](https://github.com/catalystctl/catcode/issues) for a full
list of proposed features (and known issues).

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contributing

Contributions are welcome. This is a young project — issues and PRs that improve
safety, provider coverage, or docs are especially useful.

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

### Development setup

Run from the repo (no install needed):

```bash
cd core && cargo build --release      # -> core/target/release/core
cd tui && go build -o catcode         # -> tui/catcode
./tui/catcode                          # finds ../core/target/release/core
```

Requires Rust (stable) and **Go 1.24+**. The web frontend needs Bun or Node.js
(see [Usage → Web frontend](#usage)).

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
{"type":"login","preset":"umans","api_key":"sk-..."}
{"type":"send","prompt":"...","model":"umans-glm-5.2","reasoning_effort":"high"}
{"type":"steer","prompt":"..."}
{"type":"approve","request_id":"<id>","decision":"yes|no|always"}
```

**Events:** `ready` · `authed` · `thinking` · `delta` · `tool_call_start` ·
`tool_call_name` · `tool_call_args` · `tool_call` · `approval_request` ·
`tool_result` · `compacted` · `http_retry` · `metrics` · `approval_changed` ·
`done` · `aborted` · `reset` · `error` (plus subagent / memory / session events).

<details>
<summary><strong>Releases</strong></summary>

`release-all.sh [version]` builds **all** distributable artifacts at once and
reports per-platform pass/fail (a host with a partial toolchain still ships what
it can). All artifacts are published to a GitHub Release by
`.github/workflows/release.yml` on a `v*` tag push; the installers download them
so users never compile. Each standalone embeds the Rust core via `go:embed`
(`-tags embed_core`); the separate `catcode-core-*` binaries are for the web
service's `CATCODE_CORE`.

| Script | Outputs |
|:---|:---|
| `release-linux.sh` | standalone `catcode-<ver>-linux-<arch>` + `.AppImage` + `catcode-core-<ver>-linux-<arch>` |
| `release-macos.sh` | standalone `catcode-<ver>-macos-{arm64,x86_64}` + `.dmg` + `catcode-core-<ver>-macos-{arch}` (cross-compiles via `cargo zigbuild`) |
| `release-windows.sh` | `catcode-<ver>-windows.msi` + standalone `.exe` + `.zip` + `catcode-core-<ver>-windows-x86_64.exe` (cross-compiles `x86_64-pc-windows-gnu`) |
| `release-web.sh` | `catcode-web-<ver>.tar.gz` — prebuilt Next.js standalone bundle (one cross-platform tarball) |

Artifacts land in `dist/` with `.sha256` checksums.

</details>

<details>
<summary><strong>Security notes</strong></summary>

* **OpenAI-compatible** — point `--base-url` at any OpenAI-shaped endpoint.
  Provider-specific logic (GLM `reasoning_effort=high` clamp,
  `reasoning_content` replay, `/models/info` discovery) is isolated to
  `provider.rs`.
* **No fixed turn cap** — the ceiling is the session token budget
  (`--max-session-tokens`, `0` = unlimited). The model can call `finish` to exit
  cleanly or `spawn` a nested agent.
* **Hard security boundary** — pass `--sandbox firejail --no-network` (or set it
  in the TUI settings). The denylist is a tripwire on top; workspace confinement
  covers file paths, but `bash` is only sandboxed when `--sandbox` is set.
  Sandboxing is **Linux-only**.
* **Windows bash** — the agent's `bash` tool needs bash on PATH (Git Bash or
  WSL); chat and the file tools work without it.

</details>

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## License

Distributed under the MIT License. See [`LICENSE`](LICENSE) for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Contact

karutoil — [github.com/karutoil](https://github.com/karutoil)

Project Link: [https://github.com/catalystctl/catcode](https://github.com/catalystctl/catcode)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

## Acknowledgments

* [pi-subagents](https://github.com/nicobailon/pi-subagents) — the subagent +
  intercom design this project's orchestration is ported from.
* [Bubble Tea](https://github.com/charmbracelet/bubbletea) (TUI),
  [Next.js](https://nextjs.org) (web), and the Rust async ecosystem.
* [Best-README-Template](https://github.com/othneildrew/Best-README-Template) —
  README structure inspiration.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->
[contributors-shield]: https://img.shields.io/github/contributors/catalystctl/catcode.svg?style=for-the-badge
[contributors-url]: https://github.com/catalystctl/catcode/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/catalystctl/catcode.svg?style=for-the-badge
[forks-url]: https://github.com/catalystctl/catcode/network/members
[stars-shield]: https://img.shields.io/github/stars/catalystctl/catcode.svg?style=for-the-badge
[stars-url]: https://github.com/catalystctl/catcode/stargazers
[issues-shield]: https://img.shields.io/github/issues/catalystctl/catcode.svg?style=for-the-badge
[issues-url]: https://github.com/catalystctl/catcode/issues
[license-shield]: https://img.shields.io/github/license/catalystctl/catcode.svg?style=for-the-badge
[license-url]: https://github.com/catalystctl/catcode/blob/master/LICENSE
[website-shield]: https://img.shields.io/badge/website-code.catalystctl.com-ff9e28?style=for-the-badge
[website-url]: https://code.catalystctl.com
[Rust-badge]: https://img.shields.io/badge/Rust-stable-ce422b?style=for-the-badge&logo=rust&logoColor=white
[Rust-url]: https://www.rust-lang.org/
[Go-badge]: https://img.shields.io/badge/Go-1.25%2B-00add8?style=for-the-badge&logo=go&logoColor=white
[Go-url]: https://go.dev/
[Next.js-badge]: https://img.shields.io/badge/Next.js-15-000000?style=for-the-badge&logo=nextdotjs&logoColor=white
[Next.js-url]: https://nextjs.org/
[TypeScript-badge]: https://img.shields.io/badge/TypeScript-5-3178c6?style=for-the-badge&logo=typescript&logoColor=white
[TypeScript-url]: https://www.typescriptlang.org/
