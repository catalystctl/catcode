// Resolve the running Catalyst Code web build's git commit and whether it is
// up to date vs the latest GitHub release. Used by GET /api/version.
//
// Resolution order for the *running* commit:
//   1. Live git (when a checkout is reachable) — accurate dirty state
//   2. Embedded version.json (release / install bundle, or .next/version.json)

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { EmbeddedVersion, VersionInfo, VersionUpdateStatus } from "./version-types";

export type { EmbeddedVersion, VersionInfo, VersionUpdateStatus } from "./version-types";

const GITHUB_REPO = "catalystctl/catcode";
const LATEST_CACHE_MS = 5 * 60 * 1000;

let latestCache: { at: number; tag: string | null; full: string | null } | null = null;

function shortSha(sha: string): string {
  const s = sha.trim().replace(/^v/, "");
  return s.length > 7 ? s.slice(0, 7) : s;
}

function commitsEqual(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false;
  const x = a.trim().replace(/^v/, "").toLowerCase();
  const y = b.trim().replace(/^v/, "").toLowerCase();
  if (x === y) return true;
  // Release tags are often short SHAs; accept prefix match either way.
  return x.startsWith(y) || y.startsWith(x);
}

function tryGit(cwd: string, args: string[]): string | null {
  try {
    return execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 5_000,
    }).trim();
  } catch {
    return null;
  }
}

function findGitRoot(start: string): string | null {
  let dir = start;
  for (let i = 0; i < 8; i++) {
    if (existsSync(join(dir, ".git"))) return dir;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return null;
}

function readEmbeddedFile(path: string): EmbeddedVersion | null {
  try {
    const raw = readFileSync(path, "utf8");
    const data = JSON.parse(raw) as EmbeddedVersion;
    if (!data || typeof data.commit !== "string" || !data.commit.trim()) return null;
    return data;
  } catch {
    return null;
  }
}

/** Candidate paths for a baked version.json next to the running server. */
export function embeddedVersionPaths(cwd = process.cwd()): string[] {
  return [
    join(cwd, "version.json"),
    join(cwd, ".next", "version.json"),
    join(cwd, "web", "version.json"),
    join(cwd, "web", ".next", "version.json"),
  ];
}

export function loadEmbeddedVersion(cwd = process.cwd()): EmbeddedVersion | null {
  for (const p of embeddedVersionPaths(cwd)) {
    const v = readEmbeddedFile(p);
    if (v) return v;
  }
  return null;
}

export function loadLiveGit(cwd = process.cwd()): {
  root: string;
  commit: string;
  commitFull: string;
  dirty: boolean;
} | null {
  const root = findGitRoot(cwd);
  if (!root) return null;
  const commitFull = tryGit(root, ["rev-parse", "HEAD"]);
  if (!commitFull) return null;
  const porcelain = tryGit(root, ["status", "--porcelain"]);
  return {
    root,
    commit: shortSha(commitFull),
    commitFull,
    dirty: Boolean(porcelain && porcelain.length > 0),
  };
}

async function fetchLatestRelease(): Promise<{ tag: string; full: string } | null> {
  const now = Date.now();
  if (latestCache && now - latestCache.at < LATEST_CACHE_MS) {
    if (!latestCache.tag) return null;
    return { tag: latestCache.tag, full: latestCache.full ?? latestCache.tag };
  }
  try {
    const res = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`, {
      headers: {
        Accept: "application/vnd.github+json",
        "User-Agent": "catalyst-code-web",
      },
      signal: AbortSignal.timeout(8_000),
    });
    if (!res.ok) {
      latestCache = { at: now, tag: null, full: null };
      return null;
    }
    const body = (await res.json()) as { tag_name?: string; target_commitish?: string };
    const tag = typeof body.tag_name === "string" ? body.tag_name.replace(/^v/, "") : null;
    if (!tag) {
      latestCache = { at: now, tag: null, full: null };
      return null;
    }
    const full = tag;
    latestCache = { at: now, tag: shortSha(tag), full };
    return { tag: shortSha(tag), full };
  } catch {
    latestCache = { at: now, tag: null, full: null };
    return null;
  }
}

function statusLabel(status: VersionUpdateStatus): string {
  switch (status) {
    case "up_to_date":
      return "Up to date";
    case "out_of_date":
      return "Out of date";
    case "uncommitted":
      return "Uncommitted changes";
    case "ahead":
      return "Ahead of latest release";
    default:
      return "Unknown";
  }
}

/** True when `maybeAncestor` is an ancestor of `tip` (or equal). */
function isAncestor(repoRoot: string, maybeAncestor: string, tip: string): boolean | null {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", maybeAncestor, tip], {
      cwd: repoRoot,
      stdio: "ignore",
      timeout: 5_000,
    });
    return true;
  } catch (err) {
    const code = (err as { status?: number }).status;
    // exit 1 → not an ancestor; other codes → refs missing / error
    if (code === 1) return false;
    return null;
  }
}

export async function resolveVersionInfo(cwd = process.cwd()): Promise<VersionInfo> {
  const embedded = loadEmbeddedVersion(cwd);
  const live = loadLiveGit(cwd);
  const latest = await fetchLatestRelease();

  const commitFull = live?.commitFull || embedded?.commitFull || embedded?.commit || "unknown";
  const commit = shortSha(live?.commit || embedded?.commit || commitFull);
  const dirty = live ? live.dirty : Boolean(embedded?.dirty);
  const builtAt = embedded?.builtAt ?? null;
  const source = embedded?.source || (live ? "source" : "unknown");

  let status: VersionUpdateStatus = "unknown";
  if (dirty) {
    status = "uncommitted";
  } else if (latest && commitsEqual(commit, latest.tag)) {
    status = "up_to_date";
  } else if (latest && commit !== "unknown" && live?.root) {
    const latestIsAncestor = isAncestor(live.root, latest.full, commitFull);
    const headIsAncestor = isAncestor(live.root, commitFull, latest.full);
    if (latestIsAncestor === true) status = "ahead";
    else if (headIsAncestor === true) status = "out_of_date";
    else status = "unknown";
  } else if (latest && commit !== "unknown") {
    // Release install (no live git): any mismatch means a newer release exists.
    status = "out_of_date";
  }

  const commitUrl =
    commitFull && commitFull !== "unknown"
      ? `https://github.com/${GITHUB_REPO}/commit/${commitFull}`
      : null;
  const latestUrl = latest
    ? `https://github.com/${GITHUB_REPO}/releases/tag/${latest.full}`
    : null;

  return {
    commit,
    commitFull,
    dirty,
    builtAt,
    source,
    latest: latest?.tag ?? null,
    latestFull: latest?.full ?? null,
    status,
    statusLabel: statusLabel(status),
    repo: GITHUB_REPO,
    commitUrl,
    latestUrl,
  };
}
