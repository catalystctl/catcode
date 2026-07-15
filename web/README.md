# Catalyst Code — Web

A browser frontend for the [catalyst-code](..) coding agent, built with Next.js.
It is the web equivalent of the Go Bubble Tea TUI: instead of spawning the core
in a terminal, the Next server spawns it once (via the `@catalyst-code/coding-agent`
SDK's low-level `CoreProcess`) and streams its events to the browser over
Server-Sent Events. The browser renders a full agentic UI — streaming markdown,
reasoning, tool calls, approvals, metrics, sessions — and sends commands back.

```
Browser ──SSE──▶ /api/stream ──▶ HarnessBridge ──stdio JSONL──▶ catcode-core ──▶ Umans API
Browser ──POST─▶ /api/command ─▶ HarnessBridge ─▶ (stdin)
```

## Install

The release installer downloads the prebuilt web bundle, installs the core, and
starts a background service. It does not compile the project.

Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

Windows PowerShell:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/packaging/windows/install-web.ps1)))
```

Open `http://localhost:49283` after installation.

## Develop locally

```bash
cd web
bun install                 # or: npm install / pnpm install

bun run dev                 # http://localhost:3000  (NODE_ENV=development matters — see below)
# production:
bun run build && bun run start
```

> The dev script runs `next dev`. If your shell exports `NODE_ENV=production`,
> start the dev server with it overridden: `NODE_ENV=development bun run dev`.
> `next dev` expects development mode; a production `NODE_ENV` corrupts the build.

**Runtime.** This project runs on [Bun](https://bun.sh) (`bun install`, `bun run`).
It also works under Node — just replace `bun` with `node`/`npm run`.

## The core binary

The server spawns `catcode-core` (the Rust binary). It is resolved, in order:

1. `CATCODE_CORE` env var (absolute path — use this in production).
2. A dev build found by walking up from the server cwd: `<…>/core/target/release/core`.
3. `core` / `catcode-core` on `PATH`.

The harness repo ships a built core at `../core/target/release/core`, so it is
found automatically when running from `web/`. To point at a different build, set
`CATCODE_CORE=/path/to/core`.

## API key & workspace

- **API key**: the web layer does not inject keys. Use `/login` in the UI to
  paste a key or complete OAuth. Keys previously saved via `/login`
  (`~/.config/catalyst-code/settings.json` `provider_keys`) are loaded by the
  core for returning users. A key-entry overlay appears when nothing is
  configured.
- **Workspace**: defaults to the repo root (the directory containing the located
  `core/` binary). Override with `CATALYST_CODE_WORKSPACE=/path/to/project`. The
  workspace constrains all file/bash operations the agent performs.

## Architecture

| Path | Role |
|------|------|
| `src/server/core-bridge.ts` | The `HarnessBridge` singleton: owns one `CoreProcess` for the server lifetime, reduces the raw event stream into `AgentState`, fans events to SSE subscribers, forwards POST commands. Auto-loads `settings.json` + respawns on crash. |
| `src/app/api/stream/route.ts` | `GET` — Server-Sent Events. Ensures the core, sends a `_snapshot` of the current state, then streams every live core event. |
| `src/app/api/command/route.ts` | `POST` — forwards a raw core command to the core stdin. Responses arrive over SSE. |
| `src/lib/reducer.ts` | The single agent-state reducer (mirrors the SDK's `AgentSession` message-assembly logic). Shared by the bridge (snapshots) and the client (live events). |
| `src/lib/types.ts` | Typed wire contract (core events + commands) and the UI message model. |
| `src/lib/use-agent.ts` | The client hook: opens the SSE stream, hydrates from the snapshot, reduces live events, exposes typed actions (prompt, steer, abort, approve, setKey, …). |
| `src/components/*` | The UI: chat shell, message list, markdown, tool-call cards, reasoning, approval gate, composer, header (model/thinking/approval/metrics), session sidebar, toasts. |

### Why the low-level `CoreProcess` (not the PI-compatible `AgentSession`)?

The SDK ships two layers. The high-level `AgentSession` is a PI-compatibility
adapter (its value is drop-in swap-in for pi-web) — but it routes approvals
through a boolean `confirm()` that can only return yes/no, losing the core's
per-tool-kind **"always"** escalation. The low-level `CoreProcess` speaks the raw
newline-delimited JSON protocol and gives full yes/no/always approval control,
direct session/model/stats commands, and every event (`delta`, `thinking`,
`tool_call`, `tool_result`, `metrics`, `approval_request`, …). For a custom web
frontend this is the simpler, more capable layer — fewer abstractions, more
control. The message transcript is assembled by the shared reducer.

## Wire protocol

The SSE stream emits `data: <json>\n\n` lines: one `_snapshot` (full
`AgentState`) on connect, then raw core events (`ready`, `delta`, `thinking`,
`tool_call`, `tool_result`, `approval_request`, `metrics`, `done`, `sessions`,
`stats`, `history`, …). The browser POSTs raw core commands to `/api/command`
(`send`, `steer`, `abort`, `approve`, `set_key`, `set_approval`, `list_sessions`,
`load_session`, `new_session`, `stats`, `compact`, `reset`, …). See
`core/src/protocol.rs` for the canonical command/event set.

## Status

Production-built and end-to-end verified (dev + prod) against a live Umans
endpoint: streaming markdown, reasoning, multi-step tool loops (tool_call →
tool_result → follow-up assistant), metrics, session resume, and the approval
gate.
