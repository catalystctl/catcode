# Hub — the project-centric terminal frontend (`/hub`)

A second, deliberately minimal web frontend for the harness, served by the
SAME custom Next server as the IDE (`web/src/server/server.ts`). It answers
one workflow: jump between projects, see their git state, and run any number
of CatCode TUI sessions side by side.

- **URL**: `http://localhost:49283/hub` (installed service) or `:3000/hub` (dev).
- **Coexists** with the IDE at `/` — nothing on the IDE path changed behavior.

## What it does

| Feature | How |
| --- | --- |
| Project switching | A tab bar at the top; one tab per open project. `Ctrl/Cmd+1..9` jumps between tabs. Closing a tab terminates its terminals but remembers the layout. |
| Add projects by browsing | The `+` button (or the empty state) opens the IDE's ProjectSwitcher: Recent / Browse / Create / Clone. Browse walks the real filesystem via `GET /api/browse`; opening a folder registers it via `POST /api/hub/projects`. |
| Git panel | Collapsible, resizable sidebar running the IDE's full GitPanel (changes, history, branches, stashes, remotes — every `POST /api/git` action) bound to the ACTIVE project. Diff/patch/file viewing opens in a lightweight modal instead of Monaco. |
| Terminals | Every pane is a persistent server PTY that auto-runs `catcode` in the project root. Panes survive refreshes and tab switches (server-side PTY persistence + scrollback replay). |
| Split layouts | Per-pane split right / split down buttons, draggable dividers, and presets in the header: 1, 1×2, 2×1, 2×2, 3×3, 4×4 (16-pane cap). Presets keep existing terminals alive — surplus panes are terminated, existing panes keep their sessions. |
| Persistence | Tabs, layouts, ratios, focused panes, and sidebar width persist to `localStorage` (`catcode:hub:v1`). Pane ids ARE the terminal session ids, so a refresh reattaches every running catcode. |

## Architecture

```
web/src/app/hub/page.tsx            auth-gated entry (same pattern as /)
web/src/components/hub/
  hub-shell.tsx                     tabs + presets + tab persistence + keyboard
  split-view.tsx                    recursive split-tree renderer + drag dividers
  pane.tsx                          one pane = one Ghostty Terminal (launch:"catcode")
  git-sidebar.tsx                   GitPanel + minimal IdeContext shim + viewer modal
  hub-state.ts                      localStorage load/save + sanitization
web/src/lib/hub-layout.ts           pure split-tree model (presets, split/close/ratio)
web/src/server/catcode-launch.ts    resolves the catcode TUI binary on the server
web/src/app/api/hub/projects/route.ts  add/remove/list registry entries
```

Reused verbatim (zero duplication): the zigpty WebSocket terminal transport
(`/api/terminal` in `server.ts`), better-auth session gating, `/api/browse`,
`/api/git`, `/api/file`, the `Terminal` component, `ProjectSwitcher`, and
`GitPanel`.

### catcode auto-launch

The terminal WS protocol gained an optional `launch` field on the `open`
envelope (`web/src/lib/terminal-protocol.ts`). `"catcode"` makes the server
spawn the TUI binary directly (no login-shell rc noise, cwd = project root);
omitting it or `"shell"` keeps historical behavior for the IDE and old
clients. Binary resolution order:

1. `CATCODE_WEB_TUI_BIN` env var (absolute or server-cwd-relative). A set but
   missing override fails fast rather than falling back.
2. A `PATH` walk for `catcode` (POSIX) / `catcode.exe|cmd|bat` (Windows).

If the binary can't be found, the pane shows the resolution error inside the
terminal.

### Workspace allowlist

`POST /api/hub/projects {action:"add", path}` validates the path is an
existing directory and registers it in the shared projects store
(`~/.config/catalyst-code/projects.json`). That registration is the capability
grant: the terminal WS handler and every IDE route (`workspace.ts`) allowlist
workspaces from that same store, so the hub never needed its own confinement
rules.

## Tests

- `web/src/lib/hub-layout.test.ts` — preset shapes, split/close/ratio ops,
  pane cap, restart id swap, localStorage round-trip validation.
- `web/src/server/catcode-launch.test.ts` — PATH walk, exec-bit handling,
  win32 names, override semantics.
- `web/src/lib/terminal-protocol.test.ts` — envelope shape for `launch`.
- `web/scripts/hub-regression.mjs` — puppeteer smoke (login → browse-add →
  preset → terminal panes → git sidebar → close pane), run via
  `npm run test:e2e:hub` against a running server (`AUDIT_BASE`).
