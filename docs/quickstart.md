# Quickstart

Get Catalyst Code running and send your first prompt in under two minutes.

---

## Prerequisites

- **Linux, macOS, or Windows** — the hard sandbox is Linux-only; on macOS and
  Windows leave it `none`.
- **curl + coreutils** on Unix (`sha256sum` is used by the installer).
- **No compiler required** — the installer downloads prebuilt binaries.

For the web frontend, you also need **Node.js** or **Bun** to _run_ the service
(not to build it).

---

## 1. Install

### Linux & macOS (terminal only)

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh | bash
```

This installs `catcode` to `/usr/local/bin`.

### Linux & macOS (with web frontend)

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

Then open `http://localhost:49283` in your browser.

### Windows (terminal only)

```powershell
irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1 | iex
```

Open a **new** PowerShell window (so PATH reloads) and proceed below.

### Windows (with web frontend)

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1))) -WithWeb
```

The web UI is at `http://localhost:49283`.

> See the [Installation guide](installation.md) for all install options
> (custom prefix, port, version pin, dry-run, MSI, Docker, building from
> source, uninstalling, and more).

---

## 2. Run the TUI

```bash
catcode
```

The terminal UI opens in the current directory (your workspace). You'll see a
chat panel on the left, a status bar at the bottom, and an input prompt.

To exit: press `Ctrl+C` or type `/exit`.

---

## 3. Log In

Press `/` to open the command menu, then type:

```
/login
```

A provider picker appears. Select one of the built-in presets:

| Preset | What you need |
|--------|---------------|
| **Umans** | API key from `api.code.umans.ai` (set `UMANS_API_KEY`) |
| **OpenCode Go** | API key from `opencode.ai` (set `OPENCODE_GO_API_KEY`) |
| **OpenRouter** | API key from `openrouter.ai` (set `OPENROUTER_API_KEY`) |

If the corresponding environment variable is already set, the TUI logs you in
automatically ��� no typing required.

You can be logged into several providers at once. The model picker (`/model`)
shows every provider's models tagged by prefix (`[umans]`, `[opencode-go]`,
`[openrouter]`, …).

### Subscription accounts (OAuth)

ChatGPT Plus/Pro, Claude Pro/Max, SuperGrok, and similar subscription logins
use **plugins with OAuth**. Install the plugin first:

```text
/plugin-install <source> global
/login <preset>
```

The harness handles the `/login` / `/oauth-code` flow; the plugin's OAuth
scripts handle authorize/token/refresh.

---

## 4. Send Your First Prompt

Type a message and press Enter:

```
what language was this project written in?
```

The agent thinks, streams its response, and shows you the result. Every
interaction is tracked in an append-only JSONL session.

### Switch models

```
/model
```

Lists all models from all logged-in providers. Pick by number or substring:

```
/model 3
/model glm-5.2
```

---

## 5. Run a Bash Command Through the Agent

Ask the agent to run a shell command:

```
show me the disk usage in this directory
```

The agent calls its `bash` tool. Under the default `destructive` approval mode,
you'll see a prompt like:

```
Approve?  bash  du -sh *
  → [yes]/no/always
```

Type `yes` (or `y`) to approve. The output appears in the chat.

### Quick bash (no agent turn)

Prefix a command with `!` to run it directly and add the output to the model's
context:

```
!du -sh *
```

Use `!!` to run the command without adding output to context (PI-compatible):

```
!!du -sh *
```

---

## 6. Try the Web Frontend

If you installed with `--with-web` (or `install-web.sh` / `-WithWeb`), open:

```
http://localhost:49283
```

The web UI is the browser equivalent of the TUI — it spawns one `catcode-core`
process and streams events to the browser over SSE. You can:

- Chat with the same agents
- See streaming markdown with tool calls, approvals, and metrics
- Manage sessions and restore checkpoints
- Use IDE panels (file explorer, editor, terminal, git, preview)

> For public exposure, bind to `127.0.0.1` and put a TLS reverse proxy
> (Caddy, nginx, IIS) in front.

---

## What's Next

| Topic | Where to go |
|-------|-------------|
| **Full install reference** | [Installation guide](installation.md) |
| **All TUI commands** | `/help` in the TUI, or the [commands reference](commands/) |
| **Configuration & environment variables** | [Configuration guide](configuration/) |
| **Plugins & custom tools** | [Plugin authoring guide](plugins/) |
| **Subagents & goal mode** | [Feature guides](guides/) |
| **Architecture & security** | [Architecture deep-dive](architecture/) |
| **Tools reference (schemas)** | [Tools reference](tools/) |
| **Troubleshooting** | [Troubleshooting guide](troubleshooting.md) |
| **Contributing** | [Contributing guide](../CONTRIBUTING.md) |
| **Wire protocol** | [README](../README.md#wire-protocol) — JSONL Command/Event types |

### Common next tasks

- `/approval destructive` — set the approval mode (default, asks for
  bash/write/edit). Other modes: `never` (auto-approve), `always` (ask for
  every tool).
- `/goal` — open goal mode for multi-step objectives with subagent concurrency
- `/settings` — configure theme, sandbox, mouse wheel, and more
- `/sandbox firejail --no-network` — enable Linux hard sandboxing
- `!catcode --update` — check for and apply updates (or use
  Settings → About → Update in the web UI)

---

## Verification Check

After following this guide, you should be able to:

- [ ] Run `catcode` and see the TUI
- [ ] Log in to at least one provider (`/login`)
- [ ] See models listed (`/model`)
- [ ] Send a prompt and get a response
- [ ] Approve a tool call (bash, edit, etc.)
- [ ] (Optional) Open the web frontend at `http://localhost:49283`

---

## Getting Help

- **TUI:** type `/help` for a command list
- **Web UI:** navigate the built-in help pages
- **GitHub issues:** [open an issue](https://github.com/catalystctl/catcode/issues/new)
- **Docs index:** [docs/index.md](index.md)

---

_Source evidence for this document: `install.sh`, `install.ps1`,
`install-web.sh`, `README.md`, `tui/main.go`, `tui/update.go`.
Commands and workflows were verified against live parser definitions in the
install scripts and TUI command handler (`tui/handlers.go`)._
