import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

try {
  require("node-pty");
  process.exit(0);
} catch {
  // node-pty publishes macOS/Windows prebuilds, but Linux is compiled for the
  // release host. Bun does not always expose node-gyp to dependency lifecycle
  // scripts, so make the root postinstall deterministic.
}

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const nodeGyp = path.join(projectRoot, "node_modules", "node-gyp", "bin", "node-gyp.js");
const nodePty = path.join(projectRoot, "node_modules", "node-pty");
const result = spawnSync(
  process.execPath,
  [nodeGyp, "rebuild", "--directory", nodePty],
  { cwd: projectRoot, stdio: "inherit" },
);

if (result.status !== 0) {
  console.error("Failed to build node-pty. A C/C++ compiler, make, and Python are required on Linux.");
  process.exit(result.status ?? 1);
}
