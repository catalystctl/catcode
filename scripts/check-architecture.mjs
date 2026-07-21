import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("../", import.meta.url).pathname;
const baseline = JSON.parse(readFileSync(join(root, "architecture-baseline.json"), "utf8"));

function rustFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith(".rs") ? [path] : [];
  });
}

const files = rustFiles(join(root, "core/src"));
const failures = [];

function countSites(pattern) {
  const counts = {};
  for (const file of files) {
    const source = readFileSync(file, "utf8");
    const count = [...source.matchAll(pattern)].length;
    if (count) counts[relative(root, file)] = count;
  }
  return counts;
}

function enforceNoGrowth(label, actual, allowed) {
  for (const [file, count] of Object.entries(actual)) {
    const limit = allowed[file] ?? 0;
    if (count > limit) failures.push(`${label}: ${file} has ${count} sites (baseline ${limit})`);
  }
}

enforceNoGrowth("direct process spawn", countSites(/\.spawn\(\)/g), baseline.process_spawns);
enforceNoGrowth(
  "direct task spawn",
  countSites(/tokio::spawn\(|spawn_blocking\(/g),
  baseline.task_spawns,
);

for (const file of files) {
  const path = relative(root, file);
  const source = readFileSync(file, "utf8");
  const directStdout = /\bprint(?:ln)?!\s*\(|std::io::stdout\s*\(/.test(source);
  if (directStdout && !["core/src/config.rs", "core/src/runtime/event_sink.rs"].includes(path)) {
    failures.push(`direct stdout write outside event sink/bootstrap: ${path}`);
  }
}

for (const [path, warningAt] of Object.entries(baseline.megamodules)) {
  const lines = readFileSync(join(root, path), "utf8").split(/\r?\n/).length;
  if (lines > warningAt) console.warn(`architecture warning: ${path} is ${lines} lines`);
}

if (failures.length) throw new Error(`architecture boundary regression\n${failures.join("\n")}`);
console.log(`architecture consistency ok: ${files.length} Rust source files checked`);
