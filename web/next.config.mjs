/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // The local SDK uses Node built-ins (child_process/fs/os); keep it external
  // so Next never tries to bundle it for the edge/client.
  serverExternalPackages: ["@catalyst-code/coding-agent", "better-sqlite3", "kysely"],
  // Produce a self-contained server bundle (.next/standalone) for the release
  // pipeline — `release-web.sh` ships it as a ready-to-run tarball so the
  // installer never runs `next build` on the host. Strictly additive: `next
  // dev` / `next start` are unaffected; this only adds the standalone output.
  output: "standalone",
  // Allow streaming responses to stay open for the lifetime of a turn.
};
export default nextConfig;
