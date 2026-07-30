// Pure validation helpers for project creation / cloning. Extracted from the
// /api/project route so they can be unit-tested without a server. Both the
// route and tests import from here.

export const PROJECT_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9 _.\-]{0,254}$/;
export const CLONE_URL_RE = /^(https?:\/\/|git:\/\/|ssh:\/\/|git@)/i;

/** Validate a project folder name. Returns the trimmed name or throws. */
export function validateProjectName(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) throw new Error("project name is required");
  if (trimmed.includes("/") || trimmed.includes("\\") || trimmed.includes("\0")) {
    throw new Error("project name must not contain path separators");
  }
  if (trimmed === "." || trimmed === "..") throw new Error("invalid project name");
  if (!PROJECT_NAME_RE.test(trimmed)) {
    throw new Error(
      "project name may contain letters, numbers, spaces, dashes, underscores, and dots only",
    );
  }
  return trimmed;
}

/** Validate a clone URL. Returns the trimmed URL or throws. */
export function validateCloneUrl(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) throw new Error("repository URL is required");
  if (
    !CLONE_URL_RE.test(trimmed) &&
    !/^[A-Za-z0-9_.-]+@[A-Za-z0-9_.-]+:/.test(trimmed)
  ) {
    throw new Error(
      "repository URL must start with https://, git://, ssh://, or use the git@host:path form",
    );
  }
  return trimmed;
}

/** Derive a folder name from a clone URL when the user didn't provide one. */
export function nameFromUrl(url: string): string {
  // Strip a trailing slash first, then the .git suffix, so "repo.git/" works.
  const cleaned = url.trim().replace(/\/+$/, "").replace(/\.git$/, "");
  const tail = cleaned.split(/[\/:]/).pop() ?? cleaned;
  return tail || "cloned-repo";
}

/** True if `abs` resolves under the user's home directory tree. */
export function isUnderHome(abs: string, home: string): boolean {
  const rel = abs.replace(/\\/g, "/");
  const homeN = home.replace(/\\/g, "/").replace(/\/$/, "");
  return rel === homeN || rel.startsWith(homeN + "/");
}
