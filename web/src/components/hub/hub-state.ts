// Persisted client state for the /hub terminal workspace.
//
// One localStorage key holds the tab list + per-project layouts so a refresh
// restores every project tab with its split arrangement. Server-side PTYs
// outlive the browser tab (see web/src/server/server.ts), and pane ids ARE the
// terminal session ids — so restoring the layout reattaches every terminal.
// Closing a tab terminates its PTYs but KEEPS the layout, so reopening the
// project restores the arrangement with freshly launched catcode instances.

import { leafNode, validateLayout, type LayoutNode } from "@/lib/hub-layout";

export const HUB_STORAGE_KEY = "catcode:hub:v1";

export interface HubPersistState {
  version: 1;
  /** Open project tabs, in order (absolute workspace paths). */
  tabPaths: string[];
  /** Display name per project path (basename at add time). */
  names: Record<string, string>;
  /** Split-tree layout per project path (survives tab close). */
  layouts: Record<string, LayoutNode>;
  /** Active tab path (must be in tabPaths, else null). */
  active: string | null;
  /** Last-focused pane id per project path. */
  focused: Record<string, string>;
  /** Git sidebar visibility + width. */
  gitOpen: boolean;
  gitWidth: number;
}

let paneSeq = 0;

/** Fresh pane id — also the persistent server-side terminal session id. */
export function newPaneId(): string {
  paneSeq += 1;
  return `hub_${Date.now().toString(36)}_${paneSeq}`;
}

export function defaultHubState(): HubPersistState {
  return {
    version: 1,
    tabPaths: [],
    names: {},
    layouts: {},
    active: null,
    focused: {},
    gitOpen: true,
    gitWidth: 320,
  };
}

/** Basename tolerant of both POSIX and Windows separators. */
export function pathBasename(abs: string): string {
  return abs.split(/[\\/]/).filter(Boolean).pop() ?? abs;
}

/** Load + sanitize persisted state. Invalid layouts degrade to a single fresh
 *  pane (catcode auto-launches into it) rather than dropping the tab. */
export function loadHubState(): HubPersistState {
  const base = defaultHubState();
  try {
    const raw = window.localStorage.getItem(HUB_STORAGE_KEY);
    if (!raw) return base;
    const parsed = JSON.parse(raw) as Partial<HubPersistState>;
    if (!parsed || typeof parsed !== "object") return base;

    const tabPaths = Array.isArray(parsed.tabPaths)
      ? parsed.tabPaths.filter((p): p is string => typeof p === "string" && p.length > 0)
      : [];

    const layouts: Record<string, LayoutNode> = {};
    const allPaths = new Set<string>([
      ...tabPaths,
      ...Object.keys(parsed.layouts ?? {}).filter((k) => typeof k === "string"),
    ]);
    for (const path of allPaths) {
      const restored = validateLayout(parsed.layouts?.[path]);
      layouts[path] = restored ?? leafNode(newPaneId());
    }

    const names: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed.names ?? {})) {
      if (typeof k === "string" && typeof v === "string" && v) names[k] = v;
    }

    const focused: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed.focused ?? {})) {
      if (typeof k === "string" && typeof v === "string" && v) focused[k] = v;
    }

    const active =
      typeof parsed.active === "string" && tabPaths.includes(parsed.active)
        ? parsed.active
        : (tabPaths[0] ?? null);

    return {
      version: 1,
      tabPaths,
      names,
      layouts,
      active,
      focused,
      gitOpen: typeof parsed.gitOpen === "boolean" ? parsed.gitOpen : true,
      gitWidth:
        typeof parsed.gitWidth === "number" && Number.isFinite(parsed.gitWidth)
          ? Math.min(560, Math.max(240, parsed.gitWidth))
          : 320,
    };
  } catch {
    return base;
  }
}

export function saveHubState(state: HubPersistState): void {
  try {
    window.localStorage.setItem(HUB_STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* storage full / unavailable — layout persistence is best-effort */
  }
}
