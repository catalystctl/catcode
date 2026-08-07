// Shared workspace-confinement helpers for hub API routes (git/file/browse/
// terminal). Pure Node — no React, no core bridge.

import { existsSync, realpathSync } from "node:fs";
import { isAbsolute, normalize, join, relative, resolve, sep } from "node:path";
import { loadProjects } from "@/lib/projects";
import { getDefaultWorkspace } from "@/server/default-workspace";

/** Resolve the workspace for a request: explicit ?workspace=, else default. */
export function resolveWorkspace(req: Request): string {
  const url = new URL(req.url);
  const w = url.searchParams.get("workspace");
  return w ?? getDefaultWorkspace();
}

/**
 * Resolve a client workspace root to an allowlisted project path.
 * Allowed: the default workspace + every path in loadProjects().
 */
export function authorizeWorkspaceOrThrow(candidate: string): string {
  const requested = resolve(candidate);
  const allowed = [getDefaultWorkspace(), ...loadProjects().map((project) => project.path)].map(
    (workspace) => resolve(workspace),
  );
  if (!allowed.includes(requested)) throw new Error("unauthorized workspace");
  return requested;
}

/** Alias — same allowlist check as authorizeWorkspaceOrThrow. */
export const authorizedWorkspace = authorizeWorkspaceOrThrow;

/**
 * Resolve workspace from an explicit value, else ?workspace=, else default,
 * then authorize against the allowlist.
 */
export function resolveAuthorizedWorkspace(req: Request, explicit?: string | null): string {
  const candidate =
    (typeof explicit === "string" && explicit) ||
    new URL(req.url).searchParams.get("workspace") ||
    getDefaultWorkspace();
  return authorizeWorkspaceOrThrow(candidate);
}

/**
 * Resolve `rel` under `workspace` and CONFINE it. Returns the absolute path, or
 * throws "path outside workspace" if `rel` escapes.
 * Accepts workspace-relative paths with forward slashes. Absolute `rel` values
 * are rejected (on Windows, `path.join(root, "D:\\evil")` / cross-drive
 * `relative` can fail to look like a `..` escape).
 */
export function confinePath(workspace: string, rel: string): string {
  if (typeof rel !== "string" || rel.length === 0) {
    throw new Error("path outside workspace");
  }
  // Defense in depth: never accept absolute client paths. `path.join` on POSIX
  // strips a leading slash (`join("/proj","/etc")` → `/proj/etc`), which would
  // silently reinterpret absolutes as workspace-relative; on Windows, absolute
  // or cross-drive segments can bypass a naive `..` check.
  if (isAbsolute(rel)) {
    throw new Error("path outside workspace");
  }
  const root = resolve(workspace);
  const abs = normalize(join(root, rel));
  const r = relative(root, abs);
  if (r === ".." || r.startsWith(`..${sep}`) || isAbsolute(r)) {
    throw new Error("path outside workspace");
  }
  return abs;
}

/**
 * Like confinePath, but when the target exists, realpath it and ensure the
 * resolved path stays under realpath(workspace). Prevents symlink escapes.
 * Returns the real path when the file exists; otherwise the confined abs path.
 */
export function confinePathReal(workspace: string, rel: string): string {
  const abs = confinePath(workspace, rel);
  if (!existsSync(abs)) return abs;
  const realWorkspace = realpathSync(workspace);
  const realTarget = realpathSync(abs);
  const confined = relative(realWorkspace, realTarget);
  if (confined === ".." || confined.startsWith(`..${sep}`)) {
    throw new Error("path outside workspace");
  }
  return realTarget;
}

/** Secret-ish filenames/extensions skipped by directory listings. */
export const SKIP_FILES = /\.(env|pem|key|p12|pfx|crt|cer)$/i;
export const SKIP_FILE_NAMES = new Set([
  ".env",
  ".env.local",
  ".env.development",
  ".env.production",
  ".env.test",
  "id_rsa",
  "id_ed25519",
  "id_ecdsa",
  "id_dsa",
]);
export const SKIP_DIRS = new Set([
  "node_modules",
  ".git",
  ".hg",
  ".svn",
  ".next",
  "dist",
  "build",
  "target",
  ".turbo",
  ".cache",
  "__pycache__",
  ".venv",
  "venv",
]);
