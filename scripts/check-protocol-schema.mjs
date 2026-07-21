import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../", import.meta.url).pathname;
const protocolSource = readFileSync(join(root, "core/src/protocol/commands.rs"), "utf8");
const commandStart = protocolSource.indexOf("pub enum Command");
if (commandStart < 0) throw new Error("Command enum not found in protocol/commands.rs");
const commandBlock = protocolSource.slice(commandStart);
const rustCommands = new Set(
  [...commandBlock.matchAll(/#\[serde\(rename = "([^"]+)"\)\]/g)].map((match) => match[1]),
);
const schema = JSON.parse(readFileSync(join(root, "protocol.schema.json"), "utf8"));
const schemaCommands = new Set(schema.$defs.command.properties.type.enum);
const schemaEvents = new Set(schema.$defs.event.properties.type.enum);

function rustFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith(".rs") ? [path] : [];
  });
}

const rustEvents = new Set();
for (const path of rustFiles(join(root, "core/src"))) {
  const source = readFileSync(path, "utf8");
  for (const match of source.matchAll(/Event::new\("([^"]+)"\)/g)) rustEvents.add(match[1]);
}

const sdkSource = readFileSync(join(root, "sdk/src/core-events.ts"), "utf8");
const catalog = sdkSource.slice(
  sdkSource.indexOf("CORE_EVENT_TYPES = ["),
  sdkSource.indexOf("] as const"),
);
const sdkEvents = new Set([...catalog.matchAll(/"([^"]+)"/g)].map((match) => match[1]));

function assertSame(label, expected, actual) {
  const missing = [...expected].filter((item) => !actual.has(item)).sort();
  const extra = [...actual].filter((item) => !expected.has(item)).sort();
  if (missing.length || extra.length) {
    throw new Error(
      `${label} mismatch\nmissing: ${missing.join(", ") || "(none)"}\nextra: ${extra.join(", ") || "(none)"}`,
    );
  }
}

assertSame("Rust commands vs JSON Schema", rustCommands, schemaCommands);
assertSame("Rust events vs SDK catalog", rustEvents, sdkEvents);
assertSame("Rust events vs JSON Schema", rustEvents, schemaEvents);

const fixtureDirectory = join(root, "protocol/fixtures");
const fixtures = readdirSync(fixtureDirectory).filter((fixture) => fixture.endsWith(".json") || fixture.endsWith(".jsonl"));
for (const fixture of fixtures) {
  const text = readFileSync(join(fixtureDirectory, fixture), "utf8");
  const records = fixture.endsWith(".jsonl") ? text.split(/\r?\n/).filter(Boolean) : [text];
  for (const record of records) JSON.parse(record);
}

function fixtureTypes(name) {
  return new Set(
    readFileSync(join(fixtureDirectory, name), "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => JSON.parse(line).type),
  );
}

assertSame(
  "command fixture coverage",
  rustCommands,
  fixtureTypes("commands-v2.jsonl"),
);
assertSame("event fixture coverage", rustEvents, fixtureTypes("events-v2.jsonl"));

console.log(
  `protocol consistency ok: ${rustCommands.size} commands, ${rustEvents.size} events, ${fixtures.length} fixture files`,
);
