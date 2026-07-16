// GET /api/browse — list directories for the project picker.
//
//   GET /api/browse?path=<abs>
//   → 200 { path, parent, home, entries: [{ name, path }] }
//   → 400 { error: "…" }
//   → 401 { error: "unauthorized" }
//
// Auth-gated. Directories only (no file contents). Defaults to the user's home.
// Hides a small set of sensitive system dirs from listings.

import { readdirSync, realpathSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, normalize, resolve } from "node:path";
import { getSession } from "@/lib/auth";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_ENTRIES = 500;

/** Never surface these as browsable children (secrets / system junk). */
const HIDDEN = new Set([
  ".ssh",
  ".gnupg",
  ".aws",
  ".kube",
  ".docker",
  "node_modules",
  "__pycache__",
  ".cache",
  ".Trash",
  "lost+found",
]);

export type BrowseEntry = {
  name: string;
  path: string;
};

export type BrowseResponse = {
  path: string;
  parent: string | null;
  home: string;
  entries: BrowseEntry[];
};

function resolveHome(): string {
  try {
    return realpathSync(homedir() || "/");
  } catch {
    return normalize(homedir() || "/");
  }
}

function resolveBrowsePath(raw: string | null, home: string): string {
  const input = (raw ?? "").trim() || home;
  const expanded = input.startsWith("~")
    ? join(home, input.slice(1).replace(/^\//, ""))
    : input;
  const abs = resolve(expanded);
  try {
    return realpathSync(abs);
  } catch {
    return normalize(abs);
  }
}

export async function GET(req: Request) {
  if (!(await getSession(req.headers)))
    return Response.json({ error: "unauthorized" }, { status: 401 });

  const home = resolveHome();
  const url = new URL(req.url);
  const abs = resolveBrowsePath(url.searchParams.get("path"), home);

  let st;
  try {
    st = statSync(abs);
  } catch {
    return Response.json({ error: "path not found" }, { status: 400 });
  }
  if (!st.isDirectory()) {
    return Response.json({ error: "not a directory" }, { status: 400 });
  }

  let entries: import("node:fs").Dirent[];
  try {
    entries = readdirSync(abs, { withFileTypes: true });
  } catch {
    return Response.json({ error: "unreadable directory" }, { status: 400 });
  }

  const dirs: BrowseEntry[] = [];
  for (const de of entries) {
    if (dirs.length >= MAX_ENTRIES) break;
    const name = de.name;
    if (!name || name === "." || name === "..") continue;
    if (HIDDEN.has(name)) continue;
    // Skip other dotfiles in listings for clarity; still reachable via path bar.
    if (name.startsWith(".")) continue;

    const child = join(abs, name);
    let childSt;
    try {
      // Follow symlinks so linked project folders appear as directories.
      childSt = statSync(child);
    } catch {
      continue;
    }
    if (!childSt.isDirectory()) continue;
    dirs.push({ name, path: child });
  }

  dirs.sort((a, b) => a.name.localeCompare(b.name));

  const parentDir = dirname(abs);
  const parent = parentDir !== abs ? parentDir : null;

  const body: BrowseResponse = {
    path: abs,
    parent,
    home,
    entries: dirs,
  };
  return Response.json(body);
}
