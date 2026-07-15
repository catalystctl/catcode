// Session-titles store — persists user-defined names for session files so the
// sidebar can show a meaningful title instead of an auto-generated timestamp
// filename. The core auto-derives a title from the first user message, but has
// no rename command; this web-layer overlay fills that gap without touching the
// Rust core. Stored per-workspace (keyed by workspace hash, same as sessions).

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { configDir } from "@catalyst-code/coding-agent";

// SDK-GAP: fnv64aHex exists in the SDK (sdk/src/config.ts) but is NOT exported from
// the public barrel (index.ts). The SDK's SessionManager uses it internally for
// sessionsDirFor() but neither symbol is part of the public API surface.
// KEEP-LOCAL until the SDK promotes fnv64aHex/sessionsDirFor to public exports.
/** 64-bit FNV-1a hash (matches SDK + Go TUI + Rust core). */
function fnv64aHex(s: string): string {
  const bytes = Buffer.from(s, "utf8");
  let h = BigInt("0xcbf29ce484222325");
  const prime = BigInt("0x100000001b3");
  const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
  for (let i = 0; i < bytes.length; i++) {
    h ^= BigInt(bytes[i]);
    h = (h * prime) & mask;
  }
  return h.toString(16);
}

function titlesFile(workspace: string): string {
  const cfg = join(configDir(), "session-titles");
  if (!existsSync(cfg)) mkdirSync(cfg, { recursive: true });
  return join(cfg, `${fnv64aHex(workspace)}.json`);
}

/** Map of session filename → custom title, for a given workspace. */
export type TitleMap = Record<string, string>;

export function loadTitles(workspace: string): TitleMap {
  try {
    const p = titlesFile(workspace);
    if (!existsSync(p)) return {};
    const raw = readFileSync(p, "utf8");
    const obj = JSON.parse(raw);
    return obj && typeof obj === "object" ? (obj as TitleMap) : {};
  } catch {
    return {};
  }
}

/** Set or update a custom title for a session file. Returns the new map. */
export function setTitle(
  workspace: string,
  sessionName: string,
  title: string,
): TitleMap {
  const map = loadTitles(workspace);
  const clean = title.trim();
  if (clean) {
    map[sessionName] = clean;
  } else {
    delete map[sessionName];
  }
  writeFileSync(titlesFile(workspace), JSON.stringify(map, null, 2), "utf8");
  return map;
}

/** Remove a custom title (revert to auto-derived). */
export function clearTitle(workspace: string, sessionName: string): TitleMap {
  return setTitle(workspace, sessionName, "");
}
