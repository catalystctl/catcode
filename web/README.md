# Catalyst Code — Web (Hub)

A browser frontend for [CatCode](..) multi-session agent chat. Open project
tabs, keep git beside the conversation, and reattach live sessions from any
signed-in device — no terminal panes, no restart mid-turn.

```
Browser ──SSE───▶ /api/stream  ──▶ HarnessBridge ──▶ catcode-core (per session)
Browser ──HTTP──▶ /api/command ──▶ HarnessBridge ──▶ same cores
Browser ──HTTP──▶ /api/git|/api/browse|/api/hub/* ──▶ workspace FS
```

## Install

The release installer downloads the prebuilt web bundle and starts a background
service. It does not compile the project.

Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

Windows PowerShell:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1))) -WithWeb
```

Open `http://localhost:49283` after installation (`/hub` is an alias of `/`).

## Develop locally

```bash
cd web
npm install                 # builds the local @catalyst-code/coding-agent SDK link

# Ensure catcode-core is on PATH (or set CATCODE_CORE to the binary).
npm run dev                 # http://localhost:3000
# production:
npm run build && npm run start
```

> The dev script runs the custom server. If your shell exports
> `NODE_ENV=production`, start with it overridden:
> `NODE_ENV=development npm run dev`.

**Runtime.** Building and running the web server requires **Node.js 22.13+**
because authentication uses Node's built-in `node:sqlite` module. Bun remains
supported for dependency installation and `bun test`, but `bun run dev`,
`bun run build`, and `bun run start` require a real Node executable on `PATH`.

See [TESTING.md](TESTING.md) for the unit suite. Architecture details live in
[`docs/hub-frontend.md`](../docs/hub-frontend.md).

## Core binary

Each live chat session spawns `catcode-core` (not the Go TUI). Resolution uses
the SDK's `resolveCoreBinary()` (`CATCODE_CORE`, dev `core/target/release`, then
`PATH`).

## Workspace

- **Projects**: registered via the ProjectSwitcher / `POST /api/hub/projects`
  into `~/.config/catalyst-code/projects.json`.
- **Sessions**: `~/.config/catalyst-code/sessions/<hash>/*.jsonl` — shared with
  the TUI. The account layout stores the last-viewed session per project so
  multi-device clients reattach the same live cores.
- **Default workspace**: `CATALYST_CODE_WORKSPACE` or a discovered repo root
  (`web/src/server/default-workspace.ts`).

## Auth

Single-account Better Auth (email/password, optional passkey + 2FA). First
visit hits `/setup`; later visits hit `/login`. The session cookie gates every
page and API route.
