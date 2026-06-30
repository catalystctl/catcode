// Config / paths — mirrors the umans-harness conventions used by the Go TUI
// (`tui/settings.go` `configDir()` / `sessionsDir()`) and the Rust core's
// `--workspace` / `--session` / `UMANS_*` env flags (`core/src/config.rs`).

import { homedir, platform } from "node:os";
import { join } from "node:path";
import { existsSync, mkdirSync } from "node:fs";

/** The per-user config directory (`~/.config/umans-harness`). */
export function configDir(): string {
  // On Linux/macOS: ~/.config/umans-harness (matches the TUI).
  // On Windows: %USERPROFILE%\.config\umans-harness (kept consistent with the TUI
  // rather than %APPDATA% so sessions/settings resolve identically across the
  // TUI and this SDK).
  return join(homedir() || ".", ".config", "umans-harness");
}

/** The agent directory — pi-coding-agent's `getAgentDir()` equivalent.
 *
 * For umans-harness this is the same config dir (`~/.config/umans-harness`):
 * sessions, settings.json, debug.jsonl and (future) skills/extensions live
 * there. The project-scoped plugin directory is `<cwd>/.umans-harness/plugins`
 * (handled by the core), not here. */
export function getAgentDir(): string {
  return configDir();
}

/** Ensure a directory exists (mkdir -p). Returns the path. */
export function ensureDir(dir: string): string {
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  return dir;
}

/** Per-workspace session directory: `~/.config/umans-harness/sessions/<hex(cwd)>`. */
export function sessionsDirFor(cwd: string): string {
  return join(configDir(), "sessions", fnv64aHex(cwd));
}

/** Timestamped session filename matching the TUI's `newSessionFilename()`. */
export function newSessionFilename(d = new Date()): string {
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const time = `${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`;
  return `${date}_${time}_${String(d.getMilliseconds()).padStart(3, "0")}.jsonl`;
}

/** Stable non-cryptographic hash of a path → hex (matches TUI `fnv64a`). */
export function fnv64aHex(s: string): string {
  let h = 0xcbf29ce484222325n;
  for (let i = 0; i < s.length; i++) {
    h ^= BigInt(s.charCodeAt(i));
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16);
}

/** The default core approval mode. */
export const DEFAULT_APPROVAL = "destructive" as const;

/** Resolve the umans-core binary path.
 *
 * Search order (mirrors `tui/main.go:coreBinaryPath`):
 *  1. `UMANS_CORE` env var (used as-is).
 *  2. A few dev build locations relative to this package / cwd.
 *  3. `umans-core` / `core` on PATH (installed layout).
 */
export function resolveCoreBinary(overrides: { cwd?: string } = {}): string {
  const env = process.env.UMANS_CORE;
  if (env && env.trim()) return env;

  const sfx = platform() === "win32" ? ".exe" : "";
  const candidates: string[] = [
    // dev builds relative to the SDK package (sdk/../core/target/release)
    join(__dirname, "..", "..", "core", "target", "release", `umans-core${sfx}`),
    join(__dirname, "..", "..", "core", "target", "release", `core${sfx}`),
    // relative to cwd (common when running from the harness repo root)
    join(overrides.cwd ?? process.cwd(), "core", "target", "release", `umans-core${sfx}`),
    join(overrides.cwd ?? process.cwd(), "core", "target", "release", `core${sfx}`),
    // installed beside a TUI exe in cwd
    join(overrides.cwd ?? process.cwd(), `umans-core${sfx}`),
    join(overrides.cwd ?? process.cwd(), `core${sfx}`),
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  // Fall back to PATH lookup names.
  return `umans-core${sfx}`;
}
