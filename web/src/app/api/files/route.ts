// File listing for the composer's @-mention flyout. Walks the current workspace
// (skipping common ignored directories) and returns entries whose path matches
// the query. Server-side only — the browser has no direct fs access. Confined to
// the bridge's workspace root so a crafted path can't escape.

import { readdirSync, statSync } from "node:fs";
import { join, relative, normalize, sep } from "node:path";
import { getBridge } from "@/server/core-bridge";
import { getSession } from "@/lib/auth";
import type { FileEntry } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** Directories that are almost never useful to @-mention and are slow to walk. */
const SKIP = new Set([
  "node_modules",
  ".git",
  ".next",
  ".svn",
  ".hg",
  "dist",
  "build",
  "target",
  ".cache",
  ".turbo",
  "coverage",
  ".nuxt",
  ".output",
  "__pycache__",
  ".venv",
  "venv",
  ".idea",
  ".vscode",
]);

const MAX_DEPTH = 8;
const MAX_RESULTS = 50;

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });
  const bridge = getBridge();
  const url = new URL(req.url);
  const workspace = url.searchParams.get("workspace") ?? bridge.getDefaultWorkspace();
  const q = (url.searchParams.get("q") ?? "").toLowerCase().trim();
  const base = url.searchParams.get("path") ?? "";

  // Confine the base path to the workspace.
  const root = normalize(join(workspace, base));
  const rel = relative(workspace, root);
  if (rel.startsWith("..") || rel.includes(`..${sep}`)) {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }

  const results: FileEntry[] = [];
  const seen = new Set<string>();

  function walk(dir: string, depth: number) {
    if (results.length >= MAX_RESULTS || depth > MAX_DEPTH) return;
    let entries: string[];
    try {
      entries = readdirSync(dir);
    } catch {
      return;
    }
    for (const name of entries) {
      if (results.length >= MAX_RESULTS) return;
      if (name.startsWith(".") && SKIP.has(name)) continue;
      const full = join(dir, name);
      let st;
      try {
        st = statSync(full);
      } catch {
        continue;
      }
      if (st.isDirectory()) {
        if (SKIP.has(name)) continue;
        walk(full, depth + 1);
      } else {
        const filePath = relative(workspace, full).replace(/\\/g, "/");
        if (seen.has(filePath)) continue;
        seen.add(filePath);
        // Match against the full relative path or just the filename.
        if (!q || filePath.toLowerCase().includes(q) || name.toLowerCase().includes(q)) {
          results.push({ path: filePath, name, dir: false });
        }
      }
    }
  }

  walk(root, 0);

  // Sort: paths that start with the query first, then by depth (shallower first),
  // then alphabetically.
  results.sort((a, b) => {
    const as = a.path.toLowerCase().startsWith(q) ? 0 : 1;
    const bs = b.path.toLowerCase().startsWith(q) ? 0 : 1;
    if (as !== bs) return as - bs;
    const ad = a.path.split("/").length;
    const bd = b.path.split("/").length;
    if (ad !== bd) return ad - bd;
    return a.path.localeCompare(b.path);
  });

  return Response.json({ files: results.slice(0, MAX_RESULTS) });
}
