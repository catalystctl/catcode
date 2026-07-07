// Projects store — persists the user's known workspace directories so the
// multi-project picker can list and switch between them. Stored in the shared
// catalyst-code config dir alongside settings.json so the TUI and web agree.
//
// This is a server-side store (reads/writes the filesystem). The browser talks
// to it via the /api/workspace route.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface ProjectEntry {
  /** Absolute workspace path. */
  path: string;
  /** Display name (basename of the path). */
  name: string;
  /** Last-accessed timestamp (ms) for ordering. */
  lastUsed: number;
}

function projectsFile(): string {
  const home = homedir() || ".";
  const cfg = join(home, ".config", "catalyst-code");
  return join(cfg, "projects.json");
}

function ensureDir(): void {
  const home = homedir() || ".";
  const cfg = join(home, ".config", "catalyst-code");
  if (!existsSync(cfg)) mkdirSync(cfg, { recursive: true });
}

export function loadProjects(): ProjectEntry[] {
  const p = projectsFile();
  try {
    if (!existsSync(p)) return [];
    const raw = readFileSync(p, "utf8");
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter(
      (x: unknown): x is ProjectEntry =>
        !!x && typeof x === "object" && typeof (x as ProjectEntry).path === "string",
    );
  } catch {
    return [];
  }
}

export function saveProjects(list: ProjectEntry[]): void {
  ensureDir();
  writeFileSync(projectsFile(), JSON.stringify(list, null, 2), "utf8");
}

/** Add or bump a workspace to the top of the recent list (dedup by path). */
export function touchProject(path: string): ProjectEntry[] {
  const abs = resolveAbs(path);
  const list = loadProjects().filter((p) => p.path !== abs);
  const entry: ProjectEntry = {
    path: abs,
    name: abs.replace(/\\/g, "/").split("/").filter(Boolean).pop() || abs,
    lastUsed: Date.now(),
  };
  const next = [entry, ...list].slice(0, 50);
  saveProjects(next);
  return next;
}

/** Remove a workspace from the recent list. */
export function removeProject(path: string): ProjectEntry[] {
  const abs = resolveAbs(path);
  const next = loadProjects().filter((p) => p.path !== abs);
  saveProjects(next);
  return next;
}

function resolveAbs(p: string): string {
  // Normalise but keep relative-to-cwd resolution for the server process.
  if (!p) return process.cwd();
  return p;
}
