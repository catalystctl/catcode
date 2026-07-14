import path from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = path.dirname(fileURLToPath(import.meta.url));
// Monorepo root — Turbopack only follows symlinks that stay inside its root.
// `@catalyst-code/coding-agent` is `file:../sdk`, so the root must include sdk/.
const repoRoot = path.join(webRoot, "..");

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // The local SDK uses Node built-ins (child_process/fs/os); keep it external
  // so Next never tries to bundle it for the edge/client.
  serverExternalPackages: [
    "@catalyst-code/coding-agent",
    "better-sqlite3",
    "kysely",
    "ws", // server-only (custom server /api/terminal WS); never bundled for the client
    "node-pty", // native PTY binding; loaded only by the custom server
  ],
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
};
export default nextConfig;
