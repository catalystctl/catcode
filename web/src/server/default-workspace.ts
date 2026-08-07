// Default workspace for hub allowlists and the agent bridge when no project
// is selected yet. Shared by terminal WS, git/file routes, and core-bridge.

import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

/** Walk up from cwd looking for a monorepo / git root; fall back to cwd. */
function discoverWorkspaceRoot(): string {
  let dir = process.cwd();
  for (let i = 0; i < 8; i++) {
    if (
      existsSync(join(dir, ".git")) ||
      existsSync(join(dir, "Cargo.toml")) ||
      existsSync(join(dir, "package.json"))
    ) {
      // Prefer the repo root over web/ when the server is started from web/.
      if (existsSync(join(dir, "web", "package.json")) || existsSync(join(dir, ".git"))) {
        return dir;
      }
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return process.cwd();
}

let cached: string | null = null;

/** Absolute default workspace path (env override or discovered root). */
export function getDefaultWorkspace(): string {
  if (cached) return cached;
  const fromEnv = process.env.CATALYST_CODE_WORKSPACE?.trim();
  cached = resolve(fromEnv && fromEnv.length > 0 ? fromEnv : discoverWorkspaceRoot());
  return cached;
}
