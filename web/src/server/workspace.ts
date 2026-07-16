// Shared workspace-confinement helpers for the IDE API routes. Mirrors the
// inline guard in api/files/route.ts so every new fs/git/preview/terminal route
// confines paths identically. Pure Node — no React, no core bridge events.

import { existsSync, realpathSync } from "node:fs";
import { normalize, join, relative, resolve, sep } from "node:path";
import { getBridge } from "@/server/core-bridge";
import { loadProjects } from "@/lib/projects";

/** Resolve the workspace for a request: explicit ?workspace=, else default. */
export function resolveWorkspace(req: Request): string {
  const url = new URL(req.url);
  const w = url.searchParams.get("workspace");
  return w ?? getBridge().getDefaultWorkspace();
}

/**
 * Resolve a client workspace root to an allowlisted project path.
 * Allowed: the bridge default workspace + every path in loadProjects().
 */
export function authorizeWorkspaceOrThrow(candidate: string): string {
  const requested = resolve(candidate);
  const allowed = [getBridge().getDefaultWorkspace(), ...loadProjects().map((project) => project.path)]
    .map((workspace) => resolve(workspace));
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
    getBridge().getDefaultWorkspace();
  return authorizeWorkspaceOrThrow(candidate);
}

/**
 * Resolve `rel` under `workspace` and CONFINE it. Returns the absolute path, or
 * throws "path outside workspace" if `rel` escapes (mirrors api/files/route.ts).
 * Accepts workspace-relative paths with forward slashes.
 */
export function confinePath(workspace: string, rel: string): string {
  const abs = normalize(join(workspace, rel));
  const r = relative(workspace, abs);
  if (r === ".." || r.startsWith(`..${sep}`)) {
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

/** Reused from api/files/route.ts — secret-ish filenames/extensions. */
export const SKIP_FILES = /\.(env|pem|key|p12|pfx|crt|cer)$/i;
export const SKIP_FILE_NAMES = new Set([
  ".env",
  ".env.local",
  ".env.development",
  ".env.production",
  ".env.test",
  "credentials.json",
  "credentials",
  "id_rsa",
  "id_ed25519",
  "id_ecdsa",
  "id_dsa",
  "id_rsa.pub",
  "known_hosts",
  "authorized_keys",
]);

/** True if `name` looks like a secret file that must never be served/leaked. */
export function isSecretFile(name: string): boolean {
  if (SKIP_FILE_NAMES.has(name)) return true;
  if (name.startsWith(".env")) return true;
  return SKIP_FILES.test(name);
}

/** Directories never descended into / listed by tree + files (perf + safety). */
export const SKIP_DIRS = new Set([
  "node_modules", ".git", ".next", ".svn", ".hg", "dist", "build", "target",
  ".cache", ".turbo", "coverage", ".nuxt", ".output", "__pycache__", ".venv",
  "venv", ".idea", ".vscode", ".ssh", ".aws", ".gnupg", ".kube", ".docker",
]);
