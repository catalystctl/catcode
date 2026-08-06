// Server-side hub layout store — account-scoped persistence for project tabs,
// split trees, focused panes, and git sidebar chrome.
//
// Pane ids ARE terminal session ids, so every signed-in device that loads this
// layout reattaches the same still-running server PTYs (see server.ts).
//
// Single-account install: one file under the shared config dir. The owning
// userId is recorded so a future multi-user deploy can partition cleanly.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { leafNode, validateLayout, type LayoutNode } from "@/lib/hub-layout";

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

interface HubLayoutFile {
  version: 1;
  /** better-auth user id that last wrote this layout. */
  userId: string;
  /** ms epoch — multi-device last-write-wins. */
  updatedAt: number;
  state: HubPersistState;
}

function configDir(): string {
  // Prefer HOME so tests (and rare overrides) can redirect the store without
  // monkey-patching os.homedir(). Fall back to the real home directory.
  const home = process.env.HOME || process.env.USERPROFILE || homedir() || ".";
  return join(home, ".config", "catalyst-code");
}

function layoutFile(): string {
  if (process.env.CATCODE_HUB_LAYOUT_PATH) {
    return process.env.CATCODE_HUB_LAYOUT_PATH;
  }
  return join(configDir(), "hub-layout.json");
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

/** Sanitize untrusted JSON into a HubPersistState (server + client). */
export function sanitizeHubState(
  raw: unknown,
  makePaneId: () => string = () => `hub_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`,
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
    // Keep a single fresh leaf when a layout is corrupt so the tab still opens.
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

export interface LoadedHubLayout {
  state: HubPersistState;
  updatedAt: number;
  userId: string;
}

/** Load the account hub layout, or null when nothing has been saved yet. */
export function loadHubLayout(): LoadedHubLayout | null {
  const p = layoutFile();
  try {
    if (!existsSync(p)) return null;
    const raw = JSON.parse(readFileSync(p, "utf8")) as Partial<HubLayoutFile>;
    if (!raw || typeof raw !== "object" || !raw.state) return null;
    const state = sanitizeHubState(raw.state);
    return {
      state,
      updatedAt: typeof raw.updatedAt === "number" ? raw.updatedAt : 0,
      userId: typeof raw.userId === "string" ? raw.userId : "",
    };
  } catch {
    return null;
  }
}

/** Persist the layout for `userId`. Returns the written record. */
export function saveHubLayout(userId: string, raw: unknown): LoadedHubLayout {
  const state = sanitizeHubState(raw);
  const record: HubLayoutFile = {
    version: 1,
    userId,
    updatedAt: Date.now(),
    state,
  };
  const dir = configDir();
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  writeFileSync(layoutFile(), JSON.stringify(record, null, 2), "utf8");
  return { state, updatedAt: record.updatedAt, userId };
}
