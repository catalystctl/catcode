// Runtime smoke test against the REAL umans-core binary.
// Validates CoreProcess spawn + JSONL I/O + `ready` handshake + dispose.
import { CoreProcess } from "./dist/index.js";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.UMANS_CORE = process.env.UMANS_CORE || join(process.cwd(), "core", "target", "release", "core");
const cwd = mkdtempSync(join(tmpdir(), "umans-sdk-smoke-"));

const proc = new CoreProcess({ cwd, approval: "never", idleTimeout: 60 });
const timeout = new Promise((_, rej) => setTimeout(() => rej(new Error("ready timeout")), 15000));
try {
  const ready = await Promise.race([proc.start(), timeout]);
  console.log("READY");
  console.log("  provider:", ready.provider, "(", ready.providerKind, ")");
  console.log("  authed:", ready.authed);
  console.log("  workspace:", ready.workspace);
  console.log("  models:", ready.models.length, ready.models.map((m) => m.id).slice(0, 5));
  console.log("  providers:", ready.providers);
  if (!proc.isRunning) throw new Error("core not running after ready");
  console.log("IS_RUNNING:", proc.isRunning);
} finally {
  await proc.dispose();
  console.log("DISPOSED");
}
