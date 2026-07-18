import path from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";

const webRoot = path.dirname(fileURLToPath(import.meta.url));
// Monorepo root — Turbopack only follows symlinks that stay inside its root.
// `@catalyst-code/coding-agent` is `file:../sdk`, so the root must include sdk/.
const repoRoot = path.join(webRoot, "..");

function git(args) {
  try {
    return execFileSync("git", args, {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
}

/** Write version.json into .next/ (gitignored) so /api/version works for source builds. */
function writeBuildVersion() {
  const commitFull =
    process.env.CATCODE_GIT_COMMIT_FULL ||
    git(["rev-parse", "HEAD"]) ||
    process.env.CATCODE_GIT_COMMIT ||
    "";
  const commit =
    process.env.CATCODE_GIT_COMMIT ||
    (commitFull ? commitFull.slice(0, 7) : "") ||
    "unknown";
  const dirtyEnv = process.env.CATCODE_GIT_DIRTY;
  const dirty =
    dirtyEnv === "1" || dirtyEnv === "true"
      ? true
      : dirtyEnv === "0" || dirtyEnv === "false"
        ? false
        : Boolean(git(["status", "--porcelain"]));
  const source = process.env.CATCODE_VERSION_SOURCE || "dev";
  const payload = {
    commit,
    commitFull: commitFull || commit,
    dirty,
    builtAt: new Date().toISOString(),
    source,
  };
  const nextDir = path.join(webRoot, ".next");
  try {
    if (!existsSync(nextDir)) mkdirSync(nextDir, { recursive: true });
    writeFileSync(path.join(nextDir, "version.json"), `${JSON.stringify(payload, null, 2)}\n`);
  } catch {
    // Build hosts without a writable .next yet — release-web.sh embeds version.json later.
  }
}

writeBuildVersion();

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // The local SDK uses Node built-ins (child_process/fs/os); keep it external
  // so Next never tries to bundle it for the edge/client.
  serverExternalPackages: [
    "@catalyst-code/coding-agent",
    "zigpty", // real PTY (all OS prebuilds in one package); custom server only
    "kysely",
    "ws", // server-only (custom server /api/terminal WS); never bundled for the client
  ],
  // We serve plain <img> / API URLs — never use next/image optimization, so
  // sharp's platform .node + libvips must not be required at runtime.
  images: { unoptimized: true },
  // Produce a self-contained server bundle (.next/standalone) for the release
  // pipeline — `release-web.sh` ships it as a ready-to-run tarball so the
  // installer never runs `next build` on the host. Strictly additive: `next
  // dev` / `next start` are unaffected; this only adds the standalone output.
  output: "standalone",
  outputFileTracingRoot: repoRoot,
  turbopack: {
    root: repoRoot,
  },
  // Allow streaming responses to stay open for the lifetime of a turn.
  env: {
    NEXT_PUBLIC_CATCODE_COMMIT:
      process.env.CATCODE_GIT_COMMIT ||
      git(["rev-parse", "--short", "HEAD"]) ||
      "unknown",
  },
};
export default nextConfig;
