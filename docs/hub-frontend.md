# Hub — the project-centric terminal frontend

The web UI is a deliberately minimal frontend for the harness, served by the
custom Next server (`web/src/server/server.ts`). It answers one workflow: jump
between projects, see their git state, and run any number of CatCode TUI
sessions side by side.

- **URL**: `http://localhost:49283/` (installed service) or `:3000/` (dev).
  `/hub` is kept as a permanent alias of the same shell.
- The former IDE shell at `/` and the chat-only view have been removed; hub is
  the only authenticated app surface.

## What it does

| Feature | How |
| --- | --- |
| Project switching | A tab bar at the top; one tab per open project. `Ctrl/Cmd+1..9` jumps between tabs. Closing a tab terminates its terminals but remembers the layout. |
| Add projects by browsing | The `+` button (or the empty state) opens the ProjectSwitcher: Recent / Browse / Create / Clone. Browse walks the real filesystem via `GET /api/browse`; opening a folder registers it via `POST /api/hub/projects`. |
| Git panel | Collapsible, resizable sidebar running the full GitPanel (changes, history, branches, stashes, remotes — every `POST /api/git` action) bound to the ACTIVE project. Diff/patch/file viewing opens in a lightweight modal. |
| Terminals | Every pane is a persistent server PTY that auto-runs `catcode` in the project root. Panes survive refreshes and tab switches (server-side PTY persistence + scrollback replay). |
| Split layouts | Per-pane split right / split down buttons, draggable dividers, and presets in the header: 1, 1×2, 2×1, 2×2, 3×3, 4×4 (16-pane cap). Presets keep existing terminals alive — surplus panes are terminated, existing panes keep their sessions. |
| Persistence | Tabs, layouts, ratios, focused panes, and sidebar width persist to the **account store** (`GET/PUT /api/hub/layout` → `~/.config/catalyst-code/hub-layout.json`). Pane ids ARE the terminal session ids, so any signed-in device reattaches every running catcode. localStorage is only a same-device cache. |
| Account menu | The person icon opens **Sign out**. Signing out (or closing the page) never touches the server-side PTYs — sign back in and every terminal reattaches to its live catcode. |

## Mobile responsiveness

The shell is built off `useIsMobile()` (below Tailwind's `lg` breakpoint):

- **Git panel** becomes a right-hand overlay drawer with a backdrop (a fixed-width
  column would eat a phone's screen); the drawer state is transient on mobile
  so a small screen never boots with the terminals covered.
- **Layout presets** become a native `<select>` (touch-friendly picker) instead
  of the inline button group.
- **Pane toolbar + tab close buttons** are always visible on touch (no hover
  required), with larger hit areas; split dividers are 12px-wide touch targets
  with `touch-none` so the drag resizes instead of scrolling the page.
- Tab names truncate earlier; the ProjectSwitcher gets its `mobile` layout.
- Verified headless at 390×844: preset select present, git is a drawer, no
  horizontal overflow, pane controls visible (hub-regression.mjs).

## Persistence guarantee (leave / sign out / close the page)

Terminals are owned by the SERVER's per-user session map — not by the browser
connection — and live PTYs have no TTL (only exited shells are reaped):

- **Close the tab / navigate away**: WebSockets detach; PTYs keep running with
  scrollback buffering. Returning reattaches via `attachOnly` opens.
- **Sign out**: the account menu's sign-out only clears the auth session; it
  deliberately terminates NOTHING. Signing back in (any device) restores
  tabs/layouts from the account store and every pane reattaches (e2e: process
  count unchanged across sign-out → sign-in).
- **Close the browser / refresh / open another device**: same reattach path;
  layout restoration reuses pane ids = session ids from the account store.
- **Half-open sockets** (laptop lid, phone backgrounding, NAT timeout): the
  Terminal pings every 30s and a watchdog force-closes the socket after 75s
  without ANY server traffic, then reconnects (`maxReconnects=Infinity` since a
  gone PTY answers `missing` to the attach-only open and surfaces via
  `onUnavailable`).
- The ONLY paths that terminate PTYs: closing a tab, closing a pane, restart,
  preset surplus, or a web-server restart.

## Architecture

```
web/src/app/page.tsx                auth-gated primary entry
web/src/app/hub/page.tsx            /hub alias (same HubShell)
web/src/components/hub/
  hub-shell.tsx                     tabs + presets + account layout sync + keyboard
  split-view.tsx                    recursive split-tree renderer + drag dividers
  pane.tsx                          one pane = one Ghostty Terminal (launch:"catcode")
  git-sidebar.tsx                   GitPanel + minimal IdeContext shim + viewer modal
  hub-state.ts                      sanitize + fetch/push account layout
web/src/lib/hub-layout.ts           pure split-tree model (presets, split/close/ratio)
web/src/server/catcode-launch.ts    resolves the catcode TUI binary on the server
web/src/server/default-workspace.ts default allowlist root (no core bridge)
web/src/server/hub-layout-store.ts  account-scoped layout file (hub-layout.json)
web/src/app/api/hub/layout/route.ts GET/PUT account layout
web/src/app/api/hub/projects/route.ts  add/remove/list registry entries
```

Shared transport pieces: the zigpty WebSocket terminal transport
(`/api/terminal` in `server.ts`), better-auth session gating, `/api/browse`,
`/api/git`, `/api/file`, the `Terminal` component, `ProjectSwitcher`, and
`GitPanel`.

### catcode auto-launch

The terminal WS protocol has an optional `launch` field on the `open`
envelope (`web/src/lib/terminal-protocol.ts`). `"catcode"` makes the server
spawn the TUI binary directly (no login-shell rc noise, cwd = project root);
omitting it or `"shell"` keeps a plain login shell. Binary resolution order:

1. `CATCODE_WEB_TUI_BIN` env var (absolute or server-cwd-relative). A set but
   missing override fails fast rather than falling back.
2. A `PATH` walk for `catcode` (POSIX) / `catcode.exe|cmd|bat` (Windows).

If the binary can't be found, the pane shows the resolution error inside the
terminal.

### Workspace allowlist

`POST /api/hub/projects {action:"add", path}` validates the path is an
existing directory and registers it in the shared projects store
(`~/.config/catalyst-code/projects.json`). That registration is the capability
grant: the terminal WS handler and every workspace route (`workspace.ts`)
allowlist workspaces from that same store (plus the default workspace from
`default-workspace.ts`).

## Tests

- `web/src/lib/hub-layout.test.ts` — preset shapes, split/close/ratio ops,
  pane cap, restart id swap, localStorage round-trip validation.
- `web/src/server/catcode-launch.test.ts` — PATH walk, exec-bit handling,
  win32 names, override semantics.
- `web/src/lib/terminal-protocol.test.ts` — envelope shape for `launch`.
- `web/scripts/hub-regression.mjs` — puppeteer smoke (login → browse-add →
  preset → terminal panes → git sidebar → close pane → **leave/return,
  sign-out/sign-in persistence, mobile viewport**), run via
  `npm run test:e2e:hub` against a running server (`AUDIT_BASE`).
