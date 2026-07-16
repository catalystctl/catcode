// GET /api/tree — one-level directory listing for the file explorer (lazy
// expand, matches VSCode). Per docs/IDE_PANELS_CONTRACT.md §4.1.
//
//   GET /api/tree?path=<rel>&workspace=<abs>
//   → 200 { nodes: FileNode[] }
//   → 400 { error: "path outside workspace" }
//   → 401 { error: "unauthorized" }
//
// Lists IMMEDIATE children of `path` (default "" = workspace root). Entries in
// SKIP_DIRS (node_modules, .git, .next, …) are never listed. No secret filtering
// here — this is the user's own workspace (VSCode parity, §8.5); secret filtering
// applies to search/mention (/api/files) and preview (/api/preview) only.
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { getSession } from "@/lib/auth";
import {
  confinePathReal,
  resolveAuthorizedWorkspace,
  SKIP_DIRS,
} from "@/server/workspace";
import type { FileNode } from "@/lib/types";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

/** Cap the number of entries returned for a single directory (perf guard). */
const MAX_NODES = 5000;

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  const url = new URL(req.url);
  let workspace: string;
  try {
    workspace = resolveAuthorizedWorkspace(req);
  } catch {
    return Response.json({ error: "unauthorized workspace" }, { status: 403 });
  }
  const rel = url.searchParams.get("path") ?? "";

  let abs: string;
  try {
    abs = confinePathReal(workspace, rel);
  } catch {
    return Response.json({ error: "path outside workspace" }, { status: 400 });
  }

  let entries: import("node:fs").Dirent[];
  try {
    entries = readdirSync(abs, { withFileTypes: true });
  } catch (e) {
    const code = (e as NodeJS.ErrnoException).code;
    if (code === "ENOENT")
      return Response.json({ error: "not found" }, { status: 404 });
    if (code === "ENOTDIR")
      return Response.json({ error: "not a directory" }, { status: 400 });
    if (code === "EACCES" || code === "EPERM")
      return Response.json({ error: "permission denied" }, { status: 403 });
    return Response.json({ error: "unreadable" }, { status: 500 });
  }

  const nodes: FileNode[] = [];
  for (const de of entries) {
    if (nodes.length >= MAX_NODES) break;
    const name = de.name;
    // Never list ignored directories (perf + safety — §8.8).
    if (de.isDirectory() && SKIP_DIRS.has(name)) continue;
    // statSync (not lstat) follows symlinks so a symlink-to-dir expands.
    let st;
    try {
      st = statSync(join(abs, name));
    } catch {
      continue;
    }
    const isDir = st.isDirectory();
    const childRel = rel ? `${rel}/${name}` : name;
    nodes.push({
      path: childRel,
      name,
      dir: isDir,
      size: isDir ? 0 : st.size,
      mtime: st.mtimeMs,
      symlink: de.isSymbolicLink(),
    });
  }

  // Dirs first, then files; alphabetical within each group.
  nodes.sort((a, b) => {
    if (a.dir !== b.dir) return a.dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  return Response.json({ nodes });
}
