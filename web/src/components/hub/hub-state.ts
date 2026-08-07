// Hub layout state helpers — pure sanitize + client fetch/push against the
// account-scoped server store (`/api/hub/layout`).
//
// Chat sessions live server-side (one catcode-core per session file via the
// HarnessBridge). The account layout only remembers which projects are open,
// which session was last viewed per project, and git-sidebar chrome — so any
// signed-in device reopens the same chats and reattaches the live feed.
// localStorage is only a same-device cache / one-shot migration source.

/** Legacy same-device cache key (pre multi-device sync + pre chat-hub). */
export const HUB_STORAGE_KEY = "catcode:hub:v1";

/** Current account-layout schema version (chat hub, no terminal panes). */
export const HUB_LAYOUT_VERSION = 2 as const;

export interface HubPersistState {
  version: typeof HUB_LAYOUT_VERSION;
  /** Open project tabs, in order (absolute workspace paths). */
  tabPaths: string[];
  /** Display name per project path (basename at add time). */
  names: Record<string, string>;
  /** Active tab path (must be in tabPaths, else null). */
  active: string | null;
  /**
   * Last-viewed chat session file per project path. Absolute .jsonl path.
   * When a device opens a project, it reattaches this session's live core
   * (or starts the most-recent session if missing).
   */
  sessions: Record<string, string>;
  /** Git sidebar visibility + width. */
  gitOpen: boolean;
  gitWidth: number;
}

export function defaultHubState(): HubPersistState {
  return {
    version: HUB_LAYOUT_VERSION,
    tabPaths: [],
    names: {},
    active: null,
    sessions: {},
    gitOpen: true,
    gitWidth: 320,
  };
}

/** Basename tolerant of both POSIX and Windows separators. */
export function pathBasename(abs: string): string {
  return abs.split(/[\\/]/).filter(Boolean).pop() ?? abs;
}

/** Sanitize untrusted layout JSON into a HubPersistState (v1 terminal → v2 chat). */
export function sanitizeHubState(raw: unknown): HubPersistState {
  const base = defaultHubState();
  if (!raw || typeof raw !== "object") return base;
  const parsed = raw as Partial<HubPersistState> & {
    /** v1 terminal hub fields — ignored but tolerated for migration. */
    layouts?: unknown;
    focused?: unknown;
  };

  const tabPaths = Array.isArray(parsed.tabPaths)
    ? parsed.tabPaths.filter((p): p is string => typeof p === "string" && p.length > 0)
    : [];

  const names: Record<string, string> = {};
  for (const [k, v] of Object.entries(parsed.names ?? {})) {
    if (typeof k === "string" && typeof v === "string" && v) names[k] = v;
  }

  const sessions: Record<string, string> = {};
  for (const [k, v] of Object.entries(parsed.sessions ?? {})) {
    if (typeof k === "string" && typeof v === "string" && v.endsWith(".jsonl")) {
      sessions[k] = v;
    }
  }

  const active =
    typeof parsed.active === "string" && tabPaths.includes(parsed.active)
      ? parsed.active
      : (tabPaths[0] ?? null);

  return {
    version: HUB_LAYOUT_VERSION,
    tabPaths,
    names,
    active,
    sessions,
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
 * once so the first multi-device session picks up existing tabs.
 */
export async function fetchHubLayout(): Promise<HubLayoutFetchResult> {
  const res = await fetch("/api/hub/layout", { cache: "no-store" });
  if (!res.ok) {
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

  const isEmpty =
    serverLayout.tabPaths.length === 0 &&
    Object.keys(serverLayout.sessions).length === 0 &&
    updatedAt === 0;
  if (isEmpty) {
    const local = readLocalHubCache();
    if (local && (local.tabPaths.length > 0 || Object.keys(local.sessions).length > 0)) {
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
    updatedAt: typeof data.updatedAt === "number" ? data.updatedAt : 0,
  };
}
