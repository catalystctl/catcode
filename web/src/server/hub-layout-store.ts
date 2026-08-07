// Server-side hub layout store — account-scoped persistence for project tabs,
// last-viewed chat sessions, and git sidebar chrome.
//
// Chat sessions themselves live as on-disk JSONL + live catcode-core processes
// in the HarnessBridge. This store only remembers which projects are open and
// which session each project last viewed, so every signed-in device reopens
// the same chats and reattaches the live SSE feed.
//
// Single-account install: one file under the shared config dir. The owning
// userId is recorded so a future multi-user deploy can partition cleanly.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export const HUB_LAYOUT_VERSION = 2 as const;

export interface HubPersistState {
  version: typeof HUB_LAYOUT_VERSION;
  /** Open project tabs, in order (absolute workspace paths). */
  tabPaths: string[];
  /** Display name per project path (basename at add time). */
  names: Record<string, string>;
  /** Active tab path (must be in tabPaths, else null). */
  active: string | null;
  /** Last-viewed chat session file (.jsonl) per project path. */
  sessions: Record<string, string>;
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
    version: HUB_LAYOUT_VERSION,
    tabPaths: [],
    names: {},
    active: null,
    sessions: {},
    gitOpen: true,
    gitWidth: 320,
  };
}

/** Sanitize untrusted JSON into a HubPersistState (v1 terminal → v2 chat). */
export function sanitizeHubState(raw: unknown): HubPersistState {
  const base = defaultHubState();
  if (!raw || typeof raw !== "object") return base;
  const parsed = raw as Partial<HubPersistState> & {
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
  const target = layoutFile();
  const tmp = `${target}.${process.pid}.${Date.now()}.tmp`;
  // Atomic replace: write temp then rename so a crash mid-write cannot leave a
  // truncated hub-layout.json that loadHubLayout would treat as empty.
  writeFileSync(tmp, JSON.stringify(record, null, 2), "utf8");
  renameSync(tmp, target);
  return { state, updatedAt: record.updatedAt, userId };
}
