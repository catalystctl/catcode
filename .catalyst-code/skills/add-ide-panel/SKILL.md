---
name: add-ide-panel
description: Add a new user-driven panel to the web IDE shell (file explorer/editor, terminal, git, preview pattern) — component + API route + registry + state slice + security checklist
---

# Add an IDE Panel

Use this skill when adding a new switchable panel to the `web/` VSCode-class
IDE shell (e.g. a search panel, a problems panel, a database browser). Panels
are **user-driven** (direct Node server routes), NOT agent turns — they must
not go through the core's agent loop. See `web/docs/ide-panels.md` for the
full architecture and `docs/IDE_PANELS_CONTRACT.md` for the authoritative spec.

## When to use

- Adding a new switchable panel to the IDE activity bar.
- Surfacing a workspace capability (search, outline, tasks, …) as a panel.

## Steps

1. **Add the panel id to the type union.** In `web/src/lib/types.ts`, extend
   `IdePanelId` (e.g. add `| "search"`). Add any panel-specific fields to
   `IdeLayoutState` if the panel needs persistent/runtime state.

2. **Create the panel component(s)** in `web/src/components/ide/` (e.g.
   `search-panel.tsx`). Consume `{ workspace, ide }` from `useIdeContext()`
   (`web/src/lib/ide-context.ts`). Export a named component for the registry
   to import.

3. **Add a server API route** (if the panel needs server data) under
   `web/src/app/api/<name>/route.ts`. Mirror `api/files/route.ts` exactly:
   - `export const runtime = "nodejs"; export const dynamic = "force-dynamic";`
   - `getSession` auth guard (return 401 without a session).
   - Workspace confinement: `normalize` / `join` / `relative` + reject `..`
     (use the shared helpers in `web/src/server/workspace.ts`).
   - Filter secret-ish filenames (`.env*`, `*.pem`, `*.key`, `credentials*`,
     `id_rsa*`, …) — never expose them.

4. **Register the panel** in `web/src/components/ide/panel-registry.ts`:
   - Add a `PanelDescriptor` to `PANELS` (`{ id, label, icon }`).
   - Add the id to `PANEL_ORDER` (controls activity-bar render order).
   - If the panel has heavy deps, import it via `next/dynamic` with
     `ssr:false` so it never runs on the server or bloats the main bundle.
     Light panels can be static imports.

5. **Add state actions** to `useIde()` (`web/src/lib/use-ide.ts`) if the panel
   needs layout/runtime state. Keep it in this hook — **never** touch
   `AgentState` / `reducer.ts` (that's the chat SSE snapshot contract).

6. **If the panel needs a WebSocket** (interactive/streaming), it cannot use a
   Next route handler — add it to the custom server
   (`web/src/server/server.ts`), authenticating the WS upgrade with
   `getSession` and confining any paths to the workspace. Prefer pure-JS
   (`child_process` pipes) over native modules (`node-pty`) to keep the
   release cross-platform.

7. **Verify:** `cd web && bun run typecheck` (must be green). Add a smoke
   check to `web/ui-test-ide.cjs` (puppeteer) asserting the activity-bar
   button renders and the panel switches when clicked.

## Security checklist

- [ ] Route requires `getSession` (401 without it).
- [ ] Every path confined to the workspace (`..` rejected).
- [ ] Secret files filtered (never exposed).
- [ ] No native modules (pure-JS release).
- [ ] Heavy deps dynamically imported (`ssr:false`).
- [ ] `AgentState` / `reducer.ts` untouched (chat contract intact).

## Minimal example

Registering a "search" panel:

```ts
// web/src/lib/types.ts
export type IdePanelId = "explorer" | "git" | "terminal" | "preview" | "search";
```

```ts
// web/src/components/ide/panel-registry.ts
import { SearchIcon } from "@/components/icons";
const SearchPanel = dynamic(
  () => import("./search-panel").then((m) => m.SearchPanel),
  { ssr: false },
);

export const PANELS: Record<IdePanelId, PanelDescriptor> = {
  // …existing entries…
  search: { id: "search", label: "Search", icon: SearchIcon },
};
export const PANEL_ORDER: IdePanelId[] = [
  "explorer", "search", "git", "terminal", "preview",
];
```

The activity bar auto-renders from `PANEL_ORDER`; the shell hosts the panel's
sidebar/main view. No shell changes are needed for a standard panel.
