// Hub layout state helpers — pure sanitize + client fetch/push against the
// account-scoped server store (`/api/hub/layout`).
//
// Server-side PTYs outlive the browser (see web/src/server/server.ts), and
// pane ids ARE the terminal session ids — so restoring the layout on ANY
// signed-in device reattaches every running catcode. localStorage is only a
// same-device cache / one-shot migration source, never the source of truth.

import { leafNode, validateLayout, type LayoutNode } from "@/lib/hub-layout";

/** Legacy same-device cache key (pre multi-device sync). Still read once to migrate. */
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

/** Sanitize untrusted layout JSON into a HubPersistState. */
export function sanitizeHubState(
  raw: unknown,
  makePaneId: () => string = newPaneId,
): HubPersistState {
  const base = defaultHubState();
  if (!raw || typeof raw !== "object") return base;
  const parsed = raw as Partial<HubPersistState>;

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
    layouts[path] = restored ?? leafNode(makePaneId());
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
}

/** Read the legacy localStorage cache (may be empty / corrupt). */
export function readLocalHubCache(): HubPersistState | null {
  try {
    const raw = window.localStorage.getItem(HUB_STORAGE_KEY);
    if (!raw) return null;
    return sanitizeHubState(JSON.parse(raw));
  } catch {
    return null;
  }
}

/** Best-effort same-device cache so a hard refresh paints faster. */
export function writeLocalHubCache(state: HubPersistState): void {
  try {
    window.localStorage.setItem(HUB_STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* storage full / unavailable — server is source of truth */
  }
}

export interface HubLayoutFetchResult {
  layout: HubPersistState;
  updatedAt: number;
  /** True when we pushed a localStorage migration to the server. */
  migrated: boolean;
}

/**
 * Load the account layout from the server. If the server has never stored a
 * layout but this browser still has the legacy localStorage key, migrate it
 * once so the first multi-device session picks up existing tabs/panes.
 */
export async function fetchHubLayout(): Promise<HubLayoutFetchResult> {
  const res = await fetch("/api/hub/layout", { cache: "no-store" });
  if (!res.ok) {
    // Fall back to local cache when offline / server error so the shell still opens.
    const cached = readLocalHubCache();
    return {
      layout: cached ?? defaultHubState(),
      updatedAt: 0,
      migrated: false,
    };
  }
  const data = (await res.json()) as {
    layout?: unknown;
    updatedAt?: number;
  };
  const serverLayout = sanitizeHubState(data.layout);
  const updatedAt = typeof data.updatedAt === "number" ? data.updatedAt : 0;

  // Empty server + non-empty local cache → one-shot migration.
  const isEmpty =
    serverLayout.tabPaths.length === 0 &&
    Object.keys(serverLayout.layouts).length === 0 &&
    updatedAt === 0;
  if (isEmpty) {
    const local = readLocalHubCache();
    if (local && (local.tabPaths.length > 0 || Object.keys(local.layouts).length > 0)) {
      const pushed = await pushHubLayout(local);
      return { layout: pushed.layout, updatedAt: pushed.updatedAt, migrated: true };
    }
  }

  writeLocalHubCache(serverLayout);
  return { layout: serverLayout, updatedAt, migrated: false };
}

export interface HubLayoutPushResult {
  layout: HubPersistState;
  updatedAt: number;
}

/** Persist the layout to the account store (and refresh the local cache). */
export async function pushHubLayout(state: HubPersistState): Promise<HubLayoutPushResult> {
  writeLocalHubCache(state);
  const res = await fetch("/api/hub/layout", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(state),
  });
  if (!res.ok) {
    return { layout: state, updatedAt: 0 };
  }
  const data = (await res.json().catch(() => ({}))) as {
    layout?: unknown;
    updatedAt?: number;
  };
  const layout = data.layout ? sanitizeHubState(data.layout) : state;
  writeLocalHubCache(layout);
  return {
    layout,
    updatedAt: typeof data.updatedAt === "number" ? data.updatedAt : Date.now(),
  };
}
