# IDE Panels Integration Contract

**Status:** Authoritative — single source of truth for all implementers.
**Scope:** Turn `web/` into a VSCode-class IDE shell wrapping the existing chat as a "copilot" dock, adding four panels: file explorer/editor, terminal, git, preview.
**Grounded in:** `web/src/lib/types.ts`, `web/src/lib/reducer.ts`, `web/src/app/api/files/route.ts`, `web/src/server/core-bridge.ts`, `web/next.config.mjs`, `web/package.json`, `release-web.sh`, plus the two recon briefs (`web-ide-shell-recon`, `vscode-panels-recon-step2`).

---

## 0. Governing Principles (read first)

1. **The 4 panels are USER-driven, not agent turns.** A human clicks/types in the file tree, editor, terminal, git panel, and preview. These do NOT go through the core's agent loop (which would consume context/tokens and return unstructured text to the model). They are **direct Node server routes** over the workspace. The core agent stays for chat only. (Evidence: `core/src/protocol.rs` `Command` enum has no file/git/terminal direct commands; `core/src/tools.rs` `git_*` run via `current_dir(&cfg.workspace)` inside a turn.)
2. **Do NOT pollute `AgentState`/`reducer.ts` with IDE layout state.** `AgentState` is already ~40 fields (`types.ts:658-725`) and is shared by the bridge snapshot + client reducer + SSE round-trip. IDE layout state is **client-only** and lives in a separate `useIde()` hook. This keeps the `web-event-fieldname-mismatch` + snapshot round-trip contracts intact.
3. **Single `useAgent()` instance.** `IdeShell` owns the one `useAgent()` connection and injects it into `<Chat>`. Never create a second core connection.
4. **Mirror `api/files/route.ts` for every new route:** `getSession` auth guard + workspace path confinement (`normalize`/`join`/`relative` + `..` check) + `runtime = "nodejs"` + `dynamic = "force-dynamic"`.
5. **Cross-platform pure-JS release.** No native modules. The release tarball stays one pure-JS bundle for Linux/macOS/Windows. (This rules out `node-pty` and `monaco-editor` self-hosting complexity — see §6.)
6. **Chat stays fully functional.** Streaming, approval, composer, tool calls, memory, subagents, sessions — all unchanged. Chat's only edit is a 3-line agent-injection + a `docked` height class.

---

## 1. IDE Shell Component Tree & File Paths

### 1.1 New files (all greenfield — none exist today)

```
web/src/components/ide/
  shell.tsx              # IdeShell — outermost client shell
  activity-bar.tsx       # far-left 48px vertical icon strip
  primary-sidebar.tsx    # resizable panel-content column (driven by activePanel)
  main-work-area.tsx     # editor tabs + main content + bottom panel + status bar
  status-bar.tsx         # bottom 24px bar
  copilot-dock.tsx       # right resizable dock hosting <Chat docked/>
  panel-registry.ts      # IdePanelId → { sidebar, main, icon, label } mapping
  file-tree.tsx          # explorer tree (lazy one-level expand)
  editor.tsx             # CodeMirror 6 editor (dynamically imported)
  terminal.tsx           # xterm.js terminal (dynamically imported)
  git-panel.tsx          # git changes list + actions
  preview.tsx            # iframe preview
  resize-handle.tsx      # shared draggable splitter (horizontal/vertical)
web/src/lib/
  use-ide.ts             # client-only IDE layout/panel state hook
  ide-context.ts         # React context exposing { workspace, ide } to panels
web/src/server/
  server.ts              # custom Next server (HTTP + WebSocket on one port)
  workspace.ts           # shared confinement helpers (resolveWorkspace, confinePath, isSecret)
web/src/app/api/
  tree/route.ts          # GET one-level directory listing
  file/route.ts          # GET read / PUT write
  git/route.ts           # GET status / POST action
  preview/route.ts       # GET serve workspace file for preview
  terminal/route.ts      # (HTTP 426 placeholder; real WS handled by server.ts)
```

### 1.2 Component tree

```
<page.tsx>  (server component — unchanged auth gate)
  <ErrorBoundary>
    <IdeShell/>                       // "use client"; owns useAgent() + useIde()
      <div className="flex h-[100dvh] w-full overflow-hidden bg-ink-950 text-ink-100">
        <ActivityBar/>                // w-12 (48px), far-left
        <PrimarySidebar/>             // w-64 default, resizable 200–480, collapsible
        <MainWorkArea/>               // flex-1 min-w-0 flex-col
          <MainTabs/>                 // open file tabs (Editor/Preview/Terminal-host)
          <MainContent/>              // active editor view (Editor | Preview | Terminal-host)
          <BottomPanel/>              // resizable height 0–50%, collapsible (Terminal/Output)
          <StatusBar/>                // h-6, bottom
        <CopilotDock>                 // w-[440px] default, resizable 360–720, collapsible
          <Chat docked agent={agent}/> // existing Chat, minimal change (§1.4)
        </CopilotDock>
      </div>
```

### 1.3 `app/page.tsx` change

`page.tsx` currently renders `<Chat/>` inside `<ErrorBoundary>`. It changes to render `<IdeShell/>`:

```tsx
// web/src/app/page.tsx  (server component — auth gate unchanged)
import { IdeShell } from "@/components/ide/shell";
// ...
return (
  <ErrorBoundary label="app">
    <IdeShell />
  </ErrorBoundary>
);
```

`IdeShell` is `"use client"` and is the new owner of `useAgent()`. It renders the layout above and injects `agent` into `<Chat>`.

### 1.4 `Chat` change (minimal, surgical)

`web/src/components/chat.tsx`:
- **Signature:** `export function Chat({ agent: injected, docked }: { agent?: AgentApi; docked?: boolean } = {})`.
- **Line 84-85** becomes:
  ```ts
  const ownAgent = useAgent();
  const agent = injected ?? ownAgent;
  const { state } = agent;
  ```
- **Root div (line ~514):** `className="flex h-[100dvh] w-full overflow-hidden ..."` → when `docked`, use `h-full` instead of `h-[100dvh]`:
  ```ts
  className={`flex ${docked ? "h-full" : "h-[100dvh]"} w-full overflow-hidden bg-ink-950 bg-grid text-ink-100`}
  ```
- **Nothing else changes.** Chat keeps its own `<Sidebar>` (sessions/projects — becomes the copilot dock's session switcher, collapsible via its existing hamburger), Header, WorkStatePanel, messages, Composer, all modals, all `useAgent` actions.
- **Invariant:** when `docked === true`, `agent` MUST be provided by the parent (IdeShell always passes it). When `docked` is falsy/absent (any standalone/test use), Chat creates its own `useAgent()` — backward compatible.

### 1.5 `panel-registry.ts`

Maps `IdePanelId` → panel descriptor. The shell imports panel components **by path** (dynamic) so only the active panel's heavy deps load.

```ts
// web/src/components/ide/panel-registry.ts
import dynamic from "next/dynamic";
import { FolderIcon, GitBranchIcon, TerminalIcon, GlobeIcon, SparkIcon } from "@/components/icons";
import type { IdePanelId } from "@/lib/types";
import type { ComponentType } from "react";

// Heavy panels are dynamically imported (ssr:false) so they never bloat the
// main bundle and never run on the server.
const FileTree = dynamic(() => import("./file-tree").then(m => m.FileTree), { ssr: false });
const Editor  = dynamic(() => import("./editor").then(m => m.Editor), { ssr: false });
const Terminal = dynamic(() => import("./terminal").then(m => m.Terminal), { ssr: false });
const GitPanel = dynamic(() => import("./git-panel").then(m => m.GitPanel), { ssr: false });
const Preview  = dynamic(() => import("./preview").then(m => m.Preview), { ssr: false });

export interface PanelDescriptor {
  id: IdePanelId;
  label: string;
  icon: ComponentType<{ width?: number; height?: number }>;
  sidebar: ComponentType<PanelProps>;   // rendered in PrimarySidebar
  main: ComponentType<PanelProps>;      // rendered in MainContent
}

export interface PanelProps {
  workspace: string;
  ide: ReturnType<typeof import("@/lib/use-ide").useIde>;
}

export const PANELS: Record<IdePanelId, PanelDescriptor> = {
  explorer: { id: "explorer", label: "Explorer", icon: FolderIcon,    sidebar: FileTree,  main: Editor },
  git:      { id: "git",      label: "Source Control", icon: GitBranchIcon, sidebar: GitPanel, main: GitPanel },
  terminal: { id: "terminal", label: "Terminal", icon: TerminalIcon, sidebar: Terminal, main: Terminal },
  preview:  { id: "preview",  label: "Preview",  icon: GlobeIcon,   sidebar: Preview,  main: Preview },
};
```

> The "copilot" dock is NOT in the registry — it is always present (collapsible) and hosts `<Chat>`. The activity bar's copilot icon toggles `copilotVisible` in `IdeLayoutState`.

---

## 2. Shared Types (add to `web/src/lib/types.ts`)

Append these to `types.ts`. Exact field names + types — implementers must match verbatim (the reducer/bridge never touch them; they are client-only + API DTOs).

```ts
// ─── IDE panel types (client-only + API DTOs) ──────────────────────────────

/** A panel the IDE shell can show. "copilot" is handled separately (the dock). */
export type IdePanelId = "explorer" | "git" | "terminal" | "preview";

/** One entry in the file-explorer tree (one level of a directory). */
export interface FileNode {
  /** Workspace-relative path with forward slashes (e.g. "src/lib/foo.ts"). */
  path: string;
  /** Just the basename. */
  name: string;
  /** True if this is a directory. */
  dir: boolean;
  /** File size in bytes (0 for dirs). */
  size?: number;
  /** mtime in ms (for change detection / refresh). */
  mtime?: number;
  /** True if the entry is a symlink (rendered with an arrow). */
  symlink?: boolean;
}

/** One row of `git status --porcelain=v2`. */
export interface GitStatusEntry {
  /** Workspace-relative path. For renames: "old -> new". */
  path: string;
  /** Original path for renames, else null. */
  oldPath?: string | null;
  /** XY status codes from porcelain v2 (e.g. "M ", " M", "A ", "??", "R "). */
  xy: string;
  /** Human label: "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted". */
  status: "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";
  /** Staged (index) vs unstaged (worktree). */
  staged: boolean;
}

/** Aggregate git state for the git panel + status bar. */
export interface GitStatus {
  /** Current branch name, or "HEAD (detached)". */
  branch: string;
  /** Commits ahead of upstream (0 if no upstream). */
  ahead: number;
  /** Commits behind upstream. */
  behind: number;
  /** All changed entries (staged + unstaged + untracked). */
  entries: GitStatusEntry[];
  /** HEAD commit short oid, or null if no commits. */
  head: { oid: string; message: string; author: string; ts: number } | null;
  /** True if the workspace is not a git repo (panel shows "initialize" CTA). */
  bare: boolean;
}

/** A live terminal session (one PTY-less spawn per tab). */
export interface TerminalSession {
  /** Client-generated id (e.g. "term_<ts>_<n>"). */
  id: string;
  /** Display title (defaults to shell name; user-renamable). */
  title: string;
  /** Workspace-relative or absolute cwd the shell started in. */
  cwd: string;
  /** True while the shell process is alive. */
  alive: boolean;
  /** Last exit code (null while alive / not yet exited). */
  exitCode: number | null;
}

/** Preview panel state. */
export interface PreviewState {
  /** What is being previewed. */
  kind: "file" | "url" | "none";
  /** Workspace-relative file path (kind="file") or absolute URL (kind="url"). */
  target: string;
  /** Optional query/anchor to append. */
  query?: string;
}

/** A tab in the main work area (open file / preview / terminal-host). */
export interface IdeTab {
  /** Unique id (path for files, "preview:<target>", "term:<id>"). */
  id: string;
  kind: "file" | "preview" | "terminal";
  /** Workspace-relative path (file) or target (preview) or terminal id. */
  target: string;
  /** Display label (basename for files). */
  label: string;
  /** Dirty flag (unsaved editor changes). */
  dirty: boolean;
  /** Detected language id for the editor (e.g. "typescript", "markdown"). */
  language?: string;
}

/** Client-only IDE layout state. NEVER sent over SSE / never in AgentState. */
export interface IdeLayoutState {
  /** Which panel's sidebar is shown in PrimarySidebar. */
  activePanel: IdePanelId;
  /** Open tabs in the main work area (ordered). */
  openTabs: IdeTab[];
  /** id of the active tab (null = none). */
  activeTabId: string | null;
  /** PrimarySidebar width in px. */
  sidebarWidth: number;
  /** True when PrimarySidebar is collapsed (hidden). */
  sidebarCollapsed: boolean;
  /** Bottom panel height in px (0 = collapsed). */
  bottomPanelHeight: number;
  /** True when the bottom panel is visible. */
  bottomPanelVisible: boolean;
  /** True when the copilot (Chat) dock is visible. */
  copilotVisible: boolean;
  /** Copilot dock width in px. */
  copilotWidth: number;
  /** Live terminal sessions. */
  terminals: TerminalSession[];
  /** Active terminal session id (null = none). */
  activeTerminalId: string | null;
  /** Last-known git status (null until first refresh). */
  gitStatus: GitStatus | null;
  /** Current preview target. */
  preview: PreviewState;
  /** File-tree expanded directory paths (set, persisted across reloads). */
  expandedDirs: string[];
}
```

---

## 3. State Slice — `web/src/lib/use-ide.ts`

A **separate client-only hook**. It does NOT import or touch `reducer.ts`/`AgentState`. State persists to `localStorage` (mirror `use-agent.ts`'s `lsGet`/`lsSet` pattern) for layout prefs; transient runtime state (terminals, gitStatus) is in-memory only.

### 3.1 Shape

```ts
// web/src/lib/use-ide.ts
"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import type { IdeLayoutState, IdePanelId, IdeTab, TerminalSession, GitStatus, PreviewState, FileNode } from "./types";

const STORAGE_KEY = "catcode:ide-layout";
const DEFAULTS: IdeLayoutState = {
  activePanel: "explorer",
  openTabs: [],
  activeTabId: null,
  sidebarWidth: 256,
  sidebarCollapsed: false,
  bottomPanelHeight: 220,
  bottomPanelVisible: false,
  copilotVisible: true,
  copilotWidth: 440,
  terminals: [],
  activeTerminalId: null,
  gitStatus: null,
  preview: { kind: "none", target: "" },
  expandedDirs: [],
};

export interface IdeApi {
  state: IdeLayoutState;
  // ── panels / layout ──
  setActivePanel: (p: IdePanelId) => void;
  /** VSCode activity-bar behavior: clicking the active icon collapses the sidebar;
   *  clicking another switches + expands. */
  togglePanel: (p: IdePanelId) => void;
  toggleSidebar: () => void;
  setSidebarWidth: (px: number) => void;
  setBottomPanelHeight: (px: number) => void;
  toggleBottomPanel: () => void;
  toggleCopilot: () => void;
  setCopilotWidth: (px: number) => void;
  // ── tabs / files ──
  openFile: (path: string, language?: string) => void;     // opens or focuses a file tab
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  markDirty: (id: string, dirty: boolean) => void;
  // ── terminal ──
  newTerminal: (cwd?: string) => string;                    // returns new session id
  /** Run a one-off command: focus (or create) a terminal and send `command + "\n"`
   *  to its shell stdin via the WS. Used by "run" buttons (git panel, future
   *  command palette). Does NOT spawn a new shell if one is active. */
  runCommand: (command: string, cwd?: string) => void;
  closeTerminal: (id: string) => void;
  setActiveTerminal: (id: string) => void;
  setTerminalExit: (id: string, code: number) => void;
  // ── git ──
  setGitStatus: (s: GitStatus | null) => void;
  refreshGit: () => void;                                   // triggers GET /api/git (caller wires fetch)
  // ── preview ──
  setPreview: (p: PreviewState) => void;
  // ── tree ──
  toggleDir: (path: string) => void;
  isExpanded: (path: string) => boolean;
}

export function useIde(): IdeApi { /* useState + localStorage persist + reducers */ }
```

### 3.2 Persistence rules
- **Persisted to localStorage:** `activePanel`, `sidebarWidth`, `sidebarCollapsed`, `bottomPanelHeight`, `bottomPanelVisible`, `copilotVisible`, `copilotWidth`, `expandedDirs`. Loaded once on mount; saved on change (debounced).
- **NOT persisted (in-memory only):** `openTabs`, `activeTabId`, `terminals`, `activeTerminalId`, `gitStatus`, `preview`. (Tabs/terminals don't survive reload — matches VSCode's "reopen" being opt-in; gitStatus/preview are re-fetched.)
- `openFile(path)`: if a tab with `kind:"file"` and `target===path` exists, set `activeTabId` to it; else push a new `IdeTab` (id=`path`, label=basename, language from extension map) and activate. Does NOT fetch content — the `Editor` component fetches via `/api/file` on mount/activation.
- `refreshGit()`: sets a "loading" sentinel and the **caller** (git-panel or status-bar) performs `fetch("/api/git?workspace=…")` then calls `setGitStatus`. The hook itself does no I/O (keeps it pure/testable). Same pattern for tree/preview fetches — components own fetch, hook owns state.

### 3.3 Context
`web/src/lib/ide-context.ts` exposes `IdeContext = { workspace: string; ide: IdeApi }`. `IdeShell` provides it; all panels consume via `useContext(IdeContext)` so props stay minimal. `workspace` comes from `agent.state.workspace` (the core's `ready` event) and updates on workspace switch.

---

## 4. Per-Panel API Route Signatures

All routes: `export const dynamic = "force-dynamic"; export const runtime = "nodejs";`. All require `getSession(req.headers)` (401 if missing). All confine paths to the workspace via the shared helper (§4.6). All accept `workspace` query param (absolute) defaulting to `bridge.getDefaultWorkspace()` — exactly like `api/files/route.ts:76`.

### 4.6 Shared confinement helper — `web/src/server/workspace.ts`

```ts
import { normalize, join, relative, sep } from "node:path";
import { getBridge } from "@/server/core-bridge";

/** Resolve the workspace for a request: explicit ?workspace=, else default. */
export function resolveWorkspace(req: Request): string {
  const url = new URL(req.url);
  const w = url.searchParams.get("workspace");
  return w ?? getBridge().getDefaultWorkspace();
}

/**
 * Resolve `rel` under `workspace` and CONFINE it. Returns the absolute path, or
 * throws "path outside workspace" if `rel` escapes (mirrors api/files/route.ts:80-85).
 * Accepts workspace-relative paths with forward slashes.
 */
export function confinePath(workspace: string, rel: string): string {
  const abs = normalize(join(workspace, rel));
  const r = relative(workspace, abs);
  if (r.startsWith("..") || r.includes(`..${sep}`)) {
    throw new Error("path outside workspace");
  }
  return abs;
}

/** Reused from api/files/route.ts — secret-ish filenames/extensions. */
export const SKIP_FILES = /\.(env|pem|key|p12|pfx|crt|cer)$/i;
export const SKIP_FILE_NAMES = new Set([
  ".env", ".env.local", ".env.development", ".env.production", ".env.test",
  "credentials.json", "credentials", "id_rsa", "id_ed25519", "id_ecdsa",
  "id_dsa", "id_rsa.pub", "known_hosts", "authorized_keys",
]);
export function isSecretFile(name: string): boolean {
  if (SKIP_FILE_NAMES.has(name)) return true;
  if (name.startsWith(".env")) return true;
  return SKIP_FILES.test(name);
}

/** Directories never descended into by tree/files (perf + safety). */
export const SKIP_DIRS = new Set([
  "node_modules", ".git", ".next", ".svn", ".hg", "dist", "build", "target",
  ".cache", ".turbo", "coverage", ".nuxt", ".output", "__pycache__", ".venv",
  "venv", ".idea", ".vscode", ".ssh", ".aws", ".gnupg", ".kube", ".docker",
]);
```

> `api/files/route.ts` is refactored to import these (no behavior change — pure extraction). This guarantees the new routes use the IDENTICAL confinement + secret logic.

### 4.1 `GET /api/tree` — one-level directory listing

```
GET /api/tree?path=<rel>&workspace=<abs>
→ 200 { nodes: FileNode[] }
→ 400 { error: "path outside workspace" }
→ 401 { error: "unauthorized" }
```
- Lists **immediate children** of `path` (lazy expand, matches VSCode). `path` defaults to `""` (workspace root).
- Skip entries in `SKIP_DIRS` (don't descend, don't list `node_modules` etc.).
- Do NOT filter secret files here — the tree is the user's own workspace (VSCode shows `.env`). Secret-filtering applies to search/mention (`/api/files`) and preview (`/api/preview`) only (§8.3).
- Each node: `{ path (rel, forward slashes), name, dir, size, mtime, symlink }`. Use `readdirSync({withFileTypes:true})` + `statSync`.

### 4.2 `GET /api/file` — read  ·  `PUT /api/file` — write

```
GET /api/file?path=<rel>&workspace=<abs>
→ 200 { path, content, size, language?, mtime? }
→ 400 { error: "path outside workspace" }
→ 404 { error: "not found" }
→ 401 { error: "unauthorized" }

PUT /api/file   body: { path: string; content: string; workspace?: string }
→ 200 { ok: true, path, size }
→ 400 { error: "path outside workspace" | "invalid body" }
→ 401 { error: "unauthorized" }
```
- `GET`: read file UTF-8. Refuse if `statSync` says directory (400). No size cap beyond Node defaults; cap at 5 MiB to avoid loading binaries (400 `{error:"file too large"}`).
- `PUT`: `confinePath`; create parent dirs with `mkdirSync(dirname, {recursive:true})` if missing; `writeFileSync(abs, content, "utf8")`. Returns new size.
- Both confined via `confinePath`. No secret filtering (user edits their own files — VSCode parity).

### 4.3 `GET /api/git` — status  ·  `POST /api/git` — action

```
GET /api/git?workspace=<abs>
→ 200 GitStatus
→ 401 { error: "unauthorized" }
→ 502 { error: "git failed: <msg>" }   (non-repo → { bare: true, ... })

POST /api/git   body: { action, path?, message?, branch?, workspace? }
  action: "stage" | "unstage" | "stageAll" | "unstageAll" |
          "commit" | "pull" | "push" | "fetch" |
          "checkout" | "discard" | "init"
→ 200 { ok: true, status: GitStatus }   (fresh status after the action)
→ 400 { error: "unknown action" | "path outside workspace" }
→ 401 { error: "unauthorized" }
→ 502 { error: "git failed: <msg>" }
```
- All git commands run with `cwd: workspace` via `execFile("git", [...args], {cwd: workspace})`.
- `GET` parses `git status --porcelain=v2 --branch` (+ `git rev-parse HEAD`, `git log -1 --format=...` for head). Map XY → `status`/`staged`. `ahead`/`behind` from the `# branch.ab` line.
- `POST` `path?` is confined via `confinePath` when present (stage/unstage/checkout/discard operate on a file). `commit` requires `message`. `init` runs `git init` when `bare`.
- After every successful `POST` action, re-run the status query and return fresh `GitStatus` so the UI updates atomically.
- **Never** pass user input to a shell — always `execFile` with arg arrays (no `sh -c`).

### 4.4 `GET /api/preview` — serve workspace file for preview

```
GET /api/preview?path=<rel>&workspace=<abs>
→ 200 <file bytes with Content-Type by extension>
→ 400 { error: "path outside workspace" }
→ 403 { error: "refused: secret file" }   (isSecretFile(name) → block)
→ 404 { error: "not found" }
→ 401 { error: "unauthorized" }
```
- Serves workspace static files (`.html`, `.htm`, `.svg`, `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.pdf`, `.txt`, `.json`, `.css`, `.js`) with correct `Content-Type` + `Content-Disposition: inline`.
- **Secret files are blocked** (`isSecretFile` → 403). Preview is for content the user wants rendered, never credentials.
- Loopback URLs (`localhost` / `127.0.0.1` / `::1`) are rewritten by the Preview panel to `/api/dev-proxy/<port>/…`, which reverse-proxies to `127.0.0.1:<port>` (auth-gated, SSRF-safe). Non-loopback URLs still load directly in the iframe. HTML from the proxy and workspace `.html` inject an element-inspect bootstrap for Preview → chat.

### 4.4b `/api/dev-proxy/<port>/[...path]` — loopback reverse proxy

```
ANY /api/dev-proxy/<port>/[...path]
→ upstream http://127.0.0.1:<port>/[...path]
→ 401 { error: "unauthorized" }
→ 400 { error: "invalid port" }
→ 502 { error: "dev-proxy: cannot reach …" }
```

- Auth via `getSession`. Upstream host is always `127.0.0.1` (never user-controlled) to prevent SSRF.
- Rewrites loopback `Location` redirects onto `/api/dev-proxy/<port>/…`.
- Strips `X-Frame-Options` / CSP so the Preview iframe can embed the page.
- HTML responses: inject `<base href>` + element-inspect script (`web/src/server/preview-inject.ts`).
- WebSocket/HMR upgrades are not proxied in v1.

### 4.5 `WS /api/terminal` — terminal over WebSocket

Next.js app-router route handlers **cannot** upgrade to WebSocket. The terminal WS is handled by the **custom server** (`server.ts`, §7), not a route handler. `web/src/app/api/terminal/route.ts` exists only as a placeholder returning `426 Upgrade Required` for any non-WS HTTP hit (so the path is documented).

**Upgrade flow (in `server.ts`):**
1. On `upgrade` event for path `/api/terminal`: read `req.headers`, call `getSession(headers)`. If null → close with `401`. (Cookie is sent on WS upgrade for same-origin; better-auth cookies are host-only so same-host any-port works.)
2. Parse the first WS message as a JSON **open** envelope: `{ type: "open", workspace?: string, cwd?: string, cols?: number, rows?: number }`. Resolve `workspace` (explicit or default); `cwd` defaults to `workspace` and is **confined** via `confinePath` (must stay inside workspace — reject otherwise).
3. Spawn the shell with `child_process.spawn(shell, args, { cwd, env, stdio: ["pipe","pipe","pipe"] })`:
   - `shell`: `process.env.SHELL` or `process.env.COMSPEC` (`cmd.exe`) on Windows; fallback `/bin/sh`.
   - `env`: `{ ...process.env, TERM: "xterm-256color", COLUMNS, LINES }` (strip nothing — it's the user's own workspace shell).
   - **No `node-pty`** (native, breaks cross-platform pure-JS release). Limitation: no full TUI apps (vim/nano) and no raw-mode resize signals. Line-oriented commands (ls, git, build, node, python) work fully. Documented in §9.
4. Wire: `ws.on("message")` → if `{type:"data",data}` write to `child.stdin`; if `{type:"resize",cols,rows}` set env COLUMNS/LINES (best-effort, no PTY); if `{type:"ping"}` reply `{type:"pong"}`. `child.stdout`/`stderr` → `ws.send({type:"data",data})`; `child.on("exit")` → `ws.send({type:"exit",code})` then close.
5. On `ws.close` → `child.kill("SIGTERM")` (no orphaned shells).

**Message envelopes (both directions):** `{ type: "data" | "open" | "resize" | "ping" | "pong" | "exit", ... }`. `data` is a UTF-8 string (xterm.js writes strings).

---

## 5. Per-Panel Component Interfaces

All panels are `"use client"`, consume `useContext(IdeContext)` for `{ workspace, ide }`, and own their own `fetch`/WS lifecycle. Props below are the **explicit** props (in addition to context). Heavy ones are dynamically imported (§1.5).

### 5.1 `file-tree.tsx`
```ts
export function FileTree({ root }: { root?: string }): JSX.Element;
```
- Renders a lazy tree starting at `root ?? ""` (workspace root). Each dir row: click → `ide.toggleDir(path)` + fetch `/api/tree?path=` children (cache in component state). Each file row: click → `ide.openFile(path)`.
- Double-click a dir → set as preview root? No — single behavior: expand/collapse.
- Active file (matches `ide.state.activeTabId`) is highlighted.
- Header: "EXPLORER" + refresh button (re-fetch visible expanded dirs) + new-file/new-folder buttons (PUT a placeholder / use a small inline input → `PUT /api/file`).

### 5.2 `editor.tsx`
```ts
export function Editor({ tab }: { tab: IdeTab }): JSX.Element;
```
- CodeMirror 6 via `@uiw/react-codemirror`. Loads language extension by `tab.language` (dynamic import of only the needed `@codemirror/lang-*`).
- On mount/`tab.target` change: `GET /api/file?path=` → set content. On edit: `ide.markDirty(tab.id, true)`. Ctrl/Cmd+S → `PUT /api/file` → `ide.markDirty(tab.id, false)`.
- Read-only for non-text (images render via `<img src="/api/preview?path=">`).
- Theme matches app (`ink` palette) via a CodeMirror theme extension.

### 5.3 `terminal.tsx`
```ts
export function Terminal({ sessionId }: { sessionId: string }): JSX.Element;
```
- xterm.js (`@xterm/xterm` + `@xterm/addon-fit` + `@xterm/addon-web-links`). On mount: open WS to `${wsScheme}://${host}/api/terminal`, send `{type:"open", workspace, cwd, cols, rows}`. Pipe data ↔ xterm. On `exit` → `ide.setTerminalExit(sessionId, code)`.
- `wsScheme` = `location.protocol === "https:" ? "wss" : "ws"`.
- Fit addon on container resize; send `{type:"resize"}` on resize.
- Renders inside the BottomPanel (and optionally as a main tab). Multiple sessions = multiple `Terminal` instances (each its own WS).

### 5.4 `git-panel.tsx`
```ts
export function GitPanel({ compact }: { compact?: boolean }): JSX.Element;
```
- On mount + on focus: `GET /api/git?workspace=` → `ide.setGitStatus`. Polls every 10s while visible (clear on unmount).
- Renders `GitStatus.entries` grouped: STAGED / CHANGES / UNTRACKED. Row actions: stage (`POST /api/git {action:"stage",path}`), unstage, discard (confirm), open file (`ide.openFile`).
- Commit box (textarea + Commit button → `POST /api/git {action:"commit",message}`). Sync/pull/push buttons in header.
- `compact` (status-bar mode): shows just `branch ↑ahead ↓behind • n changes`.

### 5.5 `preview.tsx`
```ts
export function Preview({ target }: { target?: string }): JSX.Element;
```
- Renders an `<iframe>`. `src` = `/api/preview?path=<rel>&workspace=<abs>` for `kind:"file"`; loopback `kind:"url"` → `/api/dev-proxy/<port>/…`; other URLs use `target` directly.
- Address bar (input + reload + open-in-new-tab + **select element**). Back/forward via local history.
- Inspect mode: posts selected DOM descriptors into the chat composer via `IdeContext.attachToChat` (works for proxied localhost + workspace HTML).
- If `ide.state.preview.kind === "none"`: empty state with "open a file to preview" + URL input.

---

## 6. New Dependencies (package.json) + Dynamic Import

### 6.1 Add to `dependencies`
```jsonc
{
  "@uiw/react-codemirror": "^4.23.x",   // CodeMirror 6 React wrapper (ESM, tree-shakeable)
  "@codemirror/state": "^6.5.x",
  "@codemirror/view": "^6.36.x",
  "@codemirror/lang-javascript": "^6.2.x",
  "@codemirror/lang-typescript": "^6.1.x",
  "@codemirror/lang-python": "^6.1.x",
  "@codemirror/lang-rust": "^6.0.x",
  "@codemirror/lang-markdown": "^6.3.x",
  "@codemirror/lang-json": "^6.0.x",
  "@codemirror/lang-css": "^6.3.x",
  "@codemirror/lang-html": "^6.4.x",
  "@codemirror/lang-yaml": "^6.1.x",
  "@codemirror/lang-sql": "^6.8.x",
  "@xterm/xterm": "^5.5.x",
  "@xterm/addon-fit": "^0.10.x",
  "@xterm/addon-web-links": "^0.11.x",
  "ws": "^8.21.x"                        // server WebSocket (currently transitive via puppeteer; promote to direct)
}
```

### 6.2 Add to `devDependencies`
```jsonc
{ "@types/ws": "^8.5.x" }
```

### 6.3 Dynamic import / SSR rules
- **Editor (`editor.tsx`)** and **Terminal (`terminal.tsx`)** are loaded via `next/dynamic(..., { ssr: false })` in `panel-registry.ts` (§1.5). They never run on the server and never enter the main bundle chunk — only the active panel's chunk loads.
- **CodeMirror language packs** are dynamically imported inside `editor.tsx` based on `tab.language` (`await import("@codemirror/lang-typescript")` etc.) so only the needed language ships.
- **xterm CSS** (`@xterm/xterm/css/xterm.css`) is imported in `terminal.tsx` (client-only, fine under `ssr:false`).
- **`ws`** is server-only (imported in `server.ts` + `workspace.ts` is pure). It is listed in `next.config.mjs` `serverExternalPackages` (§7.3) so Next never bundles it for the client.
- **NO `node-pty`** (native → breaks cross-platform pure-JS release). **NO `monaco-editor`** (5 MB + worker/CDN config + release-pipeline complexity). CodeMirror 6 is the mandated editor.

---

## 7. Custom Server & Build/Run Changes

### 7.1 `web/src/server/server.ts` (new)

A custom Next.js server that serves HTTP (Next app) **and** a WebSocket (`/api/terminal`) on the **same port**. Next 15 app-router supports custom servers; route handlers cannot upgrade to WS, so the WS is attached to the raw `http.Server`.

```ts
// web/src/server/server.ts  (run with: bun run src/server.ts  OR  node server.js)
// IMPORTANT: use RELATIVE imports here (not "@/..." aliases). tsc does NOT rewrite
// path aliases in emitted output, so "@/lib/auth" would break at runtime under Node.
// Relative paths emit clean, Node-resolvable ESM.
import { createServer } from "node:http";
import { parse } from "node:url";
import next from "next";
import { WebSocketServer } from "ws";
import { spawn } from "node:child_process";
import { getSession } from "../lib/auth";
import { getBridge } from "./core-bridge";
import { confinePath } from "./workspace";

const dev = process.env.NODE_ENV !== "production";
const port = Number(process.env.PORT) || 3000;
const hostname = process.env.HOSTNAME || "0.0.0.0";

const app = next({ dev, hostname, port });
const handle = app.getRequestHandler();

await app.prepare();
const server = createServer((req, res) => handle(req, res, parse(req.url!, true)));

// ── WebSocket: /api/terminal ──
const wss = new WebSocketServer({ noServer: true });
server.on("upgrade", (req, socket, head) => {
  const { pathname } = parse(req.url!, true);
  if (pathname !== "/api/terminal") { socket.destroy(); return; }
  // Auth: validate the session cookie carried on the WS upgrade.
  getSession(req.headers).then((session) => {
    if (!session) { socket.write("HTTP/1.1 401 Unauthorized\r\n\r\n"); socket.destroy(); return; }
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws, req));
  });
});

wss.on("connection", (ws, req) => {
  // First message = {type:"open", workspace?, cwd?, cols?, rows?}
  // → resolve workspace (default), confine cwd, spawn shell (§4.5).
  // Wire stdin/stdout/stderr/exit ↔ ws. Kill child on ws.close.
});

server.listen(port, hostname, () => console.log(`> ready on http://${hostname}:${port}`));
```

> `getBridge()` is called lazily inside handlers (it's a singleton; the bridge's constructor reads `process.env.CATALYST_CODE_WORKSPACE` / walks up for the core binary — same as today).

### 7.2 `package.json` scripts

```jsonc
{
  "scripts": {
    "dev": "node scripts/require-node.mjs && node --import tsx src/server/server.ts",
    "build": "node scripts/require-node.mjs && node node_modules/next/dist/bin/next build",
    "start": "node scripts/require-node.mjs && NODE_ENV=production node --import tsx src/server/server.ts",
    "lint": "next lint",
    "typecheck": "tsc --noEmit",
    "test": "bun test"
  }
}
```

- `tsconfig.server.json`: extends the base `tsconfig.json`, `noEmit:false`, `outDir:"."`, `module:"nodenext"`, `moduleResolution:"nodenext"` (NOT `"bundler"` — Node must resolve the emitted ESM), includes only `src/server/server.ts` (+ its imports), emits `server.js` at the web root. Imports of `next`/`ws`/`better-sqlite3`/`@catalyst-code/coding-agent` stay external (resolved from the shipped `node_modules`). `server.ts` uses **relative imports** (see §7.1) so the emitted file is Node-resolvable with no alias-rewrite step.
- `dev`, `build`, and `start` require Node.js 22.13+ because authentication imports `node:sqlite`. Bun remains usable for installation and `bun test`, but is not a supported server runtime.

### 7.3 `next.config.mjs` changes

```js
const nextConfig = {
  reactStrictMode: true,
  serverExternalPackages: [
    "@catalyst-code/coding-agent", "better-sqlite3", "kysely",
    "ws",                          // NEW — server-only (custom server), never bundled for client
  ],
  output: "standalone",           // KEEP — bundles minimal node_modules for shipping
};
```

> `output: "standalone"` is kept. Next's standalone `server.js` is **overwritten** by our compiled `server.ts` output in `release-web.sh` (§7.4). The standalone `node_modules` is still valuable (it bundles Next + better-sqlite3 + kysely + the SDK).

### 7.4 `release-web.sh` changes

After step `[4/5] web: next build`, add a server compile + ensure `ws` is in the standalone node_modules:

```bash
# [4.5/6] web: compile custom server (server.ts → server.js)
( cd web && $RT run build )   # already runs `next build && tsc -p tsconfig.server.json` → emits web/server.js

# [5/6] assembling …
STAGE="dist/.web-stage-${VERSION}"
cp -a "web/.next/standalone/." "$STAGE/"
# Overwrite Next's standalone server.js with our custom server.
cp -a "web/server.js" "$STAGE/server.js"
mkdir -p "$STAGE/.next/static"; cp -a "web/.next/static/." "$STAGE/.next/static/"
[[ -d web/public ]] && { mkdir -p "$STAGE/public"; cp -a "web/public/." "$STAGE/public/"; }

# ws is imported only by server.ts (outside Next's app graph) so the standalone
# trace omits it. Copy it in (pure JS, cross-platform).
mkdir -p "$STAGE/node_modules/ws"
cp -a "web/node_modules/ws/." "$STAGE/node_modules/ws/"
# @types/ws is dev-only; not needed at runtime.
```

- `start.js` (the runner) stays `import("./server.js")` — now it imports our custom server. Env `PORT`/`HOSTNAME` unchanged.
- **Sanity check** in the script: `[[ -f "$STAGE/server.js" ]]` (already present) + add `[[ -d "$STAGE/node_modules/ws" ]]`.

### 7.5 Next 15 app-router compatibility — confirmed
- Custom server with `next({dev})` + `app.prepare()` + `getRequestHandler()` is the documented Next 15 custom-server pattern. App-router routes (`/api/*`, pages) are served by `handle(req,res)`.
- `output: "standalone"` + custom server coexist: standalone bundles the app + minimal node_modules; our `server.js` is the entrypoint that calls `next({dev:false})` + `app.prepare()`. (In production the custom server imports the built `.next` — `next({dev:false})` serves the prebuilt app.)
- WebSocket on the same `http.Server` via `noServer:true` + `handleUpgrade` is the standard `ws` pattern; does not conflict with Next.

### 7.6 Fallback (only if §7.4 proves too risky in CI)
If compiling/shipping the custom server destabilizes the release, fall back to a **separate-port WS server**: a tiny `terminal-ws.ts` run as a second process on `PORT+1`, same auth (cookie is host-only → sent cross-port), same spawn logic. The client computes `ws://host:(PORT+1)/api/terminal`. This avoids touching Next's standalone output entirely. **Prefer the single-port custom server (§7.1-7.4); use this fallback only with supervisor approval.**

---

## 8. Security Rules

1. **Auth on every route.** `getSession(req.headers)` → 401 if null. Applies to `/api/tree`, `/api/file` (GET+PUT), `/api/git` (GET+POST), `/api/preview`, `/api/dev-proxy`, and the `/api/terminal` WS upgrade. No anonymous access.
2. **Workspace confinement on every path.** Use `confinePath(workspace, rel)` (§4.6) — `normalize`+`join`+`relative`+`..` check, identical to `api/files/route.ts:80-85`. Reject `..` traversal with 400. This is the hard validation criterion.
3. **Terminal spawns only within the workspace cwd.** The `open` envelope's `cwd` is confined via `confinePath`; default `cwd = workspace`. The shell's `cwd` can never escape the workspace. `child_process.spawn(shell, args, { cwd })` — never `shell:true`, never `sh -c` with interpolated input.
4. **Writes require auth + confinement.** `PUT /api/file` and `POST /api/git` (commit/checkout/discard) require `getSession` + `confinePath`. Git actions use `execFile` with arg arrays (no shell injection).
5. **Secrets never exposed via search/mention or preview.** Reuse `isSecretFile`/`SKIP_FILES`/`SKIP_FILE_NAMES` (§4.6):
   - `/api/files` (existing @-mention flyout): unchanged — already hides secrets.
   - `/api/preview`: **blocks** secret files (403).
   - `/api/tree` + `/api/file`: the user's own workspace — **no** secret filtering (VSCode parity: you can open/edit your `.env`). Documented explicitly so implementers don't "helpfully" add filtering that breaks editing `.env`.
6. **No secrets in responses.** Git status, file content, terminal output are the user's own workspace data served only to the authenticated session. Never log API keys / `UMANS_API_KEY` / auth secrets. The bridge already does NOT forward `api_key` via env (`core-bridge.ts:loadSettings`).
7. **Terminal output is not persisted.** Terminal sessions are in-memory; closing the tab kills the shell. No shell history is written to disk by the web layer.
8. **`SKIP_DIRS` for tree/files perf + safety.** `node_modules`, `.git`, `.next`, etc. are never listed/walked (prevents accidental exposure + keeps the tree fast).

---

## 9. Acceptance Criteria

### 9.1 Overall (hard gates)
- [ ] `cd web && bun run typecheck` (`tsc --noEmit`) passes with **zero** errors.
- [ ] `cd web && npm run build` succeeds and produces `.next/`.
- [ ] `cd web && npm run dev` boots the custom server; `http://localhost:3000` loads the IDE shell.
- [ ] Existing chat is **fully functional**: streaming, approval gate, composer, tool calls, memory panel, subagents panel, session switching — all work unchanged.
- [ ] All 4 panels render and are switchable via the activity bar; sidebar/main/bottom panels are resizable; copilot dock is collapsible.
- [ ] Every new route requires `getSession` (401 without cookie) and confines paths (400 on `..` escape).
- [ ] Terminal WS authenticates (401 without cookie) and spawns shells with `cwd` inside the workspace only.
- [ ] Updated puppeteer smoke test (`ui-test.cjs` or new `ui-test-ide.cjs`) passes: shell + activity bar + chat load with zero `pageerror`s.

### 9.2 Per-panel
- **File explorer/editor:** tree lists workspace root (one level, lazy expand); clicking a file opens an editor tab; editing + Ctrl/Cmd+S persists via `PUT /api/file` (dirty flag clears); `..` in path → 400.
- **Terminal:** new terminal opens a WS shell in the workspace cwd; typed commands (`ls`, `git status`, `echo hi`) echo output; closing the tab kills the shell; unauthenticated WS → 401.
- **Git:** `GET /api/git` shows branch + changes; stage/unstage/commit/pull/push work; status bar shows `branch ↑ahead ↓behind`; non-repo → "initialize" CTA.
- **Preview:** opening an `.html` file renders it in the iframe via `/api/preview`; secret file (`.env`) → 403; loopback URL preview loads via `/api/dev-proxy/<port>`; external URL preview loads the URL in the iframe; inspect mode posts selected elements into the chat composer.

### 9.3 Known limitations (documented, not blockers)
- Terminal has no full PTY (no `vim`/`nano`/TUI apps, no raw-mode resize) — `child_process.spawn` is used to keep the release cross-platform pure-JS. `node-pty` is a future enhancement that would require per-platform tarballs.
- Preview `/api/dev-proxy` does not forward WebSocket/HMR upgrades (page load works; live reload may not). Element inspect requires proxied localhost or workspace HTML (arbitrary external origins cannot be instrumented).
- Tabs/terminals do not persist across reload (in-memory only) — matches VSCode's opt-in "restore".

---

## 10. Risks & Open Questions

### Risks
- **R1 — Custom server + standalone interaction (§7.4).** Overwriting Next's standalone `server.js` and ensuring `ws` is in the shipped `node_modules` is the riskiest release-pipeline change. Mitigation: `release-web.sh` sanity-checks `server.js` + `node_modules/ws`; fallback to separate-port WS (§7.6) if CI breaks.
- **R2 — Chat refactor scope creep.** The `docked`/`agent` injection is 3 lines, but implementers may be tempted to restructure Chat's internals. Mitigation: the contract mandates the exact 3-line change (§1.4); any further Chat change is out of scope and must be flagged.
- **R3 — `ws` as a transitive dep today.** It's currently only present via `puppeteer`. Promoting to a direct `dependency` + copying into the standalone stage is required or the WS server crashes at runtime in production. Mitigation: §6.1 + §7.4.
- **R4 — CodeMirror/xterm bundle size.** Mitigated by `next/dynamic({ssr:false})` + per-language dynamic imports (§6.3); only the active panel's chunk loads.
- **R5 — Better-auth cookie on WS upgrade.** Host-only cookies are sent cross-port but NOT cross-origin. If `CATCODE_WEB_ORIGIN` is a tunnel/domain, the WS must be same-origin (it is — same server/port). No action needed, but document.

### Open questions (escalate to supervisor if blocking)
- **Q1:** Should the copilot dock default to visible (440px) or collapsed on first load? (Contract assumes visible; trivially configurable via `IdeLayoutState.copilotVisible`.)
- **Q2:** Is the separate-port WS fallback (§7.6) acceptable if the single-port custom server proves too risky, or is single-port a hard requirement?
- **Q3:** Should `node-pty` (true PTY) be a v1 stretch goal requiring per-platform release tarballs, or firmly deferred? (Contract defers it.)

---

## 11. Implementation Order (suggested, non-binding)

1. Types (§2) + `workspace.ts` confinement helper (§4.6) + refactor `api/files/route.ts` to use it.
2. `use-ide.ts` + `ide-context.ts` (§3) — pure, testable, no I/O.
3. `panel-registry.ts` + `shell.tsx` + `activity-bar.tsx` + `primary-sidebar.tsx` + `main-work-area.tsx` + `status-bar.tsx` + `copilot-dock.tsx` + `resize-handle.tsx` (§1) — render Chat docked, panels as empty stubs.
4. Chat `docked`/`agent` injection (§1.4) — verify chat still works.
5. `/api/tree` + `/api/file` (GET/PUT) + `file-tree.tsx` + `editor.tsx` (§4.1-4.2, §5.1-5.2).
6. `/api/git` (GET/POST) + `git-panel.tsx` (§4.3, §5.4).
7. `/api/preview` + `preview.tsx` (§4.4, §5.5).
8. `server.ts` + `/api/terminal` WS + `terminal.tsx` (§4.5, §7.1, §5.3).
9. `package.json` scripts + `next.config.mjs` + `release-web.sh` (§6, §7.2-7.4).
10. Puppeteer smoke test (§9.1) + `npm run typecheck` + `npm run build` green.

---

*End of contract. This document is the single source of truth; deviations require supervisor approval.*
