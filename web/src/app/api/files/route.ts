// File listing for the composer's @-mention flyout. Walks the current workspace
// (skipping common ignored directories) and returns entries whose path matches
// the query. Server-side only — the browser has no direct fs access. Confined to
// the bridge's workspace root so a crafted path can't escape.

import { lstatSync, readdirSync, realpathSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { getSession } from "@/lib/auth";
import type { FileEntry } from "@/lib/types";
import {
  confinePathReal,
  resolveAuthorizedWorkspace,
  SKIP_DIRS,
  isSecretFile,
} from "@/server/workspace";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_DEPTH = 8;
const MAX_RESULTS = 50;

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(req);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }

  const url = new URL(req.url);
  const q = (url.searchParams.get("q") ?? "").toLowerCase().trim();
  const base = url.searchParams.get("path") ?? "";

  // Confine the base path to the workspace (realpath when it exists).
  let root: string;
  let realWorkspace: string;
  try {
    root = confinePathReal(workspace, base);
    realWorkspace = realpathSync(workspace);
  } catch {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }

  const results: FileEntry[] = [];
  const seen = new Set<string>();

  function underWorkspace(abs: string): boolean {
    try {
      const real = realpathSync(abs);
      const r = relative(realWorkspace, real);
      return r !== ".." && !r.startsWith(`..${sep}`);
    } catch {
      return false;
    }
  }

  function walk(dir: string, depth: number) {
    if (results.length >= MAX_RESULTS || depth > MAX_DEPTH) return;
    if (!underWorkspace(dir)) return;
    let entries: string[];
    try {
      entries = readdirSync(dir);
    } catch {
      return;
    }
    for (const name of entries) {
      if (results.length >= MAX_RESULTS) return;
      if (name.startsWith(".") && SKIP_DIRS.has(name)) continue;
      const full = join(dir, name);
      let st;
      try {
        st = lstatSync(full);
      } catch {
        continue;
      }
      // Do not follow symlink directories out of the workspace.
      if (st.isSymbolicLink()) continue;
      if (st.isDirectory()) {
        if (SKIP_DIRS.has(name)) continue;
        walk(full, depth + 1);
      } else if (st.isFile()) {
        if (isSecretFile(name)) continue;
        const filePath = relative(workspace, full).replace(/\\/g, "/");
        if (seen.has(filePath)) continue;
        seen.add(filePath);
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
