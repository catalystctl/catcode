# Web IDE Panels

The `web/` app is a VSCode-class IDE shell. The existing chat lives in a
collapsible **copilot dock** on the right; four user-driven panels — file
explorer/editor, terminal, git, and preview — surround it. This doc describes
the architecture, the wire boundaries, the security model, and how to add a
new panel. The authoritative design spec is `docs/IDE_PANELS_CONTRACT.md`.

## Architecture

`IdeShell` (`web/src/components/ide/shell.tsx`) is the outermost client
component. It owns the **single** `useAgent()` connection and the client-only
`useIde()` layout slice, and provides both to all panels via `IdeContext`.

```
<page.tsx>                        server component — auth gate unchanged
  <ErrorBoundary>
    <IdeShell>                    "use client"; owns useAgent() + useIde()
      [ActivityBar]               far-left icon strip (Explorer/Git/Terminal/Preview/Copilot)
      [PrimarySidebar ⇄ resize]    hosts the active panel's sidebar view
      [MainWorkArea]               flex-1
        ├── TabStrip              open file/preview tabs
        ├── MainContent           active editor / preview
        ├── ⇄ resize ── BottomPanel (terminal)
        └── StatusBar
      [resize ⇄ CopilotDock]      right dock hosting <Chat docked agent={agent}/>
```

The existing `<Chat>` is mounted as the docked copilot panel. It receives the
injected `agent` so it shares the one core connection — streaming, approval,
composer, tool calls, memory, and subagents all keep working with **zero**
changes to the chat reducer or `AgentState`.

### Governing principles

1. **Panels are user-driven, not agent turns.** A human clicks/types in the
   tree, editor, terminal, git panel, and preview. These go through **direct
   Node server routes** over the workspace — never through the core's agent
   loop (which would burn context/tokens and return unstructured text). The
   core agent stays for chat only.
2. **No `AgentState` pollution.** IDE layout state is client-only, in a
   separate `useIde()` hook (`web/src/lib/use-ide.ts`). The chat
   `AgentState` / `reducer.ts` / SSE snapshot contract is untouched.
3. **Single `useAgent()` instance.** `IdeShell` owns it; panels never open a
   second core connection.
4. **Pure-JS release.** No native modules — the release tarball stays one
   cross-platform bundle. The terminal uses a line-mode shell
   (`child_process` pipes, no PTY) and the editor uses CodeMirror 6 (not
   self-hosted Monaco).

## The four panels

| Panel | Components | API route |
|-------|-----------|-----------|
| Explorer | `file-tree.tsx` (sidebar) + `editor.tsx` (CodeMirror 6, main) | `/api/tree`, `/api/file` |
| Source Control | `git-panel.tsx` (sidebar + main) | `/api/git` |
| Terminal | `terminal.tsx` (xterm.js, bottom panel) | `/api/terminal` (WebSocket) |
| Preview | `preview.tsx` (iframe, main) | `/api/preview` |

All panel components live in `web/src/components/ide/` and are registered in
`panel-registry.ts`. Heavy panels (editor, terminal, preview) are loaded via
`next/dynamic` with `ssr:false`; light panels (file-tree, git-panel) are
static imports.

## State management

`useIde()` (`web/src/lib/use-ide.ts`) holds all IDE layout/panel state:
`activePanel`, `openTabs` / `activeTabId`, `sidebarWidth`, `bottomPanelHeight`,
`copilotVisible` / `copilotWidth`, `terminals`, `gitStatus`, `preview`,
`expandedDirs`. Layout prefs persist to `localStorage` (`catcode:ide-layout`);
transient runtime state is in-memory. The hook does no I/O — components own
their fetch/WS lifecycle and call back into the hook.

`IdeContext` (`web/src/lib/ide-context.ts`) exposes `{ workspace, ide }` to
panels via React context. The shared types (`IdePanelId`, `IdeLayoutState`,
`IdeTab`, `TerminalSession`, `PreviewState`, `FileNode`, `GitStatus`) live in
`web/src/lib/types.ts`.

## API routes

All routes mirror `api/files/route.ts`: `getSession` auth guard + workspace
path confinement (`normalize` / `join` / `relative` + `..` rejection) +
`runtime = "nodejs"` + `dynamic = "force-dynamic"`. Shared confinement helpers
live in `web/src/server/workspace.ts`.

| Route | Method | Purpose |
|-------|--------|---------|
| `/api/tree` | GET | One-level directory listing (`FileNode[]`) |
| `/api/file` | GET | Read a file (content + size + language) |
| `/api/file` | PUT | Write a file (path + content) |
| `/api/git` | GET | `git status --porcelain=v2 -b` + recent log |
| `/api/git` | POST | stage / unstage / commit / checkout |
| `/api/preview` | GET | Serve a workspace file for iframe preview (safe Content-Type) |
| `/api/terminal` | WS | Interactive shell (see below) |

## Custom server & WebSocket terminal

Next.js app-router route handlers cannot upgrade to WebSocket, so
`web/src/server/server.ts` is a **custom Next server** that serves the app
(HTTP) **and** a `ws` WebSocketServer at `/api/terminal` on the same port.

- The WS upgrade is authenticated via `getSession` (validates the session
  cookie from the upgrade request headers).
- The first client message must be `{type:"open", cwd?, cols?, rows?}`; then
  `{type:"data"}`, `{type:"resize"}`, `{type:"ping"}`.
- The shell is **line-mode** (`child_process.spawn` with pipes, no PTY) for a
  pure-JS cross-platform release — no `node-pty` native build. TUI apps
  (vim/nano) won't work; ordinary commands do. Local echo is mirrored back.
- The shell always runs in the configured workspace; a client `cwd` is
  confined (rejected if it escapes with `..`). A client-provided `workspace`
  root is **not** honoured (would let a client point the shell anywhere).

`package.json` scripts route `dev` / `start` through `server.ts`; `build`
remains `next build`. `release-web.sh` was updated accordingly.

## Security model

- **Auth:** every route (and the WS upgrade) requires `getSession`.
- **Confinement:** every path is confined to the workspace via the
  `..`-rejection guard; the terminal spawns only in the workspace cwd.
- **Secrets:** file/tree/preview routes filter secret-ish filenames
  (`.env*`, `*.pem`, `*.key`, `credentials*`, `id_rsa*`, …) — never exposed.

## Adding a new panel

See the `add-ide-panel` skill (`.catalyst-code/skills/add-ide-panel/`). In
short: add the id to the `IdePanelId` union in `types.ts`, create the panel
component in `web/src/components/ide/`, register it in `panel-registry.ts`
(`PANELS` + `PANEL_ORDER`), add any server route (mirroring the confinement
pattern), and add state to `useIde()` if needed — never to `AgentState`.
