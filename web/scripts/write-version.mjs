#!/usr/bin/env node
// Write version.json for the running web server / release bundle.
// Invoked after `next build` and by release-web.sh / install.sh helpers.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = join(root, "..");

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

const commitFull =
  process.env.CATCODE_GIT_COMMIT_FULL || git(["rev-parse", "HEAD"]) || "";
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
const text = `${JSON.stringify(payload, null, 2)}\n`;

const targets = [
  join(root, "version.json"),
  join(root, ".next", "version.json"),
];
// Optional explicit output dir (release-web stage, install web-dir).
if (process.env.CATCODE_VERSION_OUT) {
  targets.push(process.env.CATCODE_VERSION_OUT);
}

for (const target of targets) {
  try {
    const dir = dirname(target);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    writeFileSync(target, text);
    console.log(`wrote ${target} (${commit}${dirty ? "*" : ""}, source=${source})`);
  } catch (err) {
    console.warn(`skip ${target}: ${err instanceof Error ? err.message : err}`);
  }
}
