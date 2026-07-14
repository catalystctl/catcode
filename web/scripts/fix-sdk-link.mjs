import { existsSync, symlinkSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sdkRoot = resolve(webRoot, "../sdk");
const sdkModules = join(sdkRoot, "node_modules");
const webModules = join(webRoot, "node_modules");

// A clean `bun install` of the web package links the local SDK but does not
// install its development dependencies. Let the SDK compiler resolve the
// TypeScript and Node type packages that were already installed for the web.
if (!existsSync(sdkModules)) {
  symlinkSync(webModules, sdkModules, process.platform === "win32" ? "junction" : "dir");
}

const tsc = join(webModules, "typescript", "bin", "tsc");
const result = spawnSync(process.execPath, [tsc, "-p", join(sdkRoot, "tsconfig.json")], {
  stdio: "inherit",
});

if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
