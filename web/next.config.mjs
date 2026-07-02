/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // The local SDK uses Node built-ins (child_process/fs/os); keep it external
  // so Next never tries to bundle it for the edge/client.
  serverExternalPackages: ["@umans-harness/coding-agent"],
  // Allow streaming responses to stay open for the lifetime of a turn.
};
export default nextConfig;
