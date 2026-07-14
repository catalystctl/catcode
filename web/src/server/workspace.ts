// Shared workspace-confinement helpers for the IDE API routes. Mirrors the
// inline guard in api/files/route.ts so every new fs/git/preview/terminal route
// confines paths identically. Pure Node — no React, no core bridge events.

import { normalize, join, relative, sep } from "node:path";
import { getBridge } from "@/server/core-bridge";

/** Resolve the workspace for a request: explicit ?workspace=, else default. */
export function resolveWorkspace(req: Request): string {
  const url = new URL(req.url);
  const w = url.searchParams.get("workspace");
  return w ?? getBridge().getDefaultWorkspace();
}

/**
 * Resolve `rel` under `workspace` and CONFINE it. Returns the absolute path, or
 * throws "path outside workspace" if `rel` escapes (mirrors api/files/route.ts).
 * Accepts workspace-relative paths with forward slashes.
 */
export function confinePath(workspace: string, rel: string): string {
  const abs = normalize(join(workspace, rel));
  const r = relative(workspace, abs);
  if (r.startsWith("..") || r.includes(`..${sep}`)) {
    throw new Error("path outside workspace");
  }
  return abs;
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
