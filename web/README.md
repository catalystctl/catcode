# Catalyst Code — Web (Hub)

A browser frontend for running [CatCode](..) terminal sessions side by side.
It is a project-centric workspace: open project tabs, inspect git state, and
auto-launch persistent `catcode` PTYs in a split grid. The former IDE shell
and chat-only view have been removed — hub is the only authenticated UI.

```
Browser ──WS───▶ /api/terminal ──▶ zigpty ──▶ catcode (per pane)
Browser ──HTTP─▶ /api/git|/api/browse|/api/hub/projects ──▶ workspace FS
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
npm install                 # Bun may also be used to install dependencies

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

See [TESTING.md](TESTING.md) for the unit suite and hub browser regression.
Architecture details live in [`docs/hub-frontend.md`](../docs/hub-frontend.md).

## CatCode TUI binary

Each hub pane spawns the `catcode` TUI on the server (not `catcode-core`).
Resolution order:

1. `CATCODE_WEB_TUI_BIN` env var (absolute or server-cwd-relative). A set but
   missing override fails fast rather than falling back.
2. A `PATH` walk for `catcode` (POSIX) / `catcode.exe|cmd|bat` (Windows).

## Workspace

- **Projects**: registered via the ProjectSwitcher / `POST /api/hub/projects`
  into `~/.config/catalyst-code/projects.json`. That list is the capability
  grant for terminal + git + file routes.
- **Default workspace**: `CATALYST_CODE_WORKSPACE` or a discovered repo root
  (`web/src/server/default-workspace.ts`).

## Auth

Single-account Better Auth (email/password, optional passkey + 2FA). First
visit hits `/setup`; later visits hit `/login`. The session cookie gates every
page and API route.
