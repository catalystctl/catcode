// HarnessBridge — the server-side singleton that owns one umans-core process
// for the server's lifetime, reduces the raw event stream into AgentState (so
// reconnecting clients can hydrate instantly), and fans raw core events out to
// SSE subscribers. Commands arrive via POST and are forwarded to the core; the
// bridge also feeds a synthetic `_user` event into its own reducer for send/steer
// so snapshots include the user's message.
//
// Low-level CoreProcess layer (not the PI-compatible AgentSession): this gives
// full yes/no/always approval control and direct session/model/stats commands.

import { CoreProcess } from "@umans-harness/coding-agent";
import { existsSync } from "node:fs";
import { readFileSync } from "node:fs";
import { readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { reduce, initialState } from "@/lib/reducer";
import type {
  AgentState,
  CoreCommand,
  CoreEvent,
  ReadyPayload,
} from "@/lib/types";

interface HarnessSettings {
  apiKey?: string;
  model?: string;
  baseUrl?: string;
  provider?: string;
}

/** Read the TUI's settings.json (~/.config/umans-harness/settings.json) so the web
 *  app auto-authenticates with the same key the TUI uses — no re-entry needed.
 *  The core itself does not read settings.json (the TUI forwards the key via env). */
function loadSettings(): HarnessSettings {
  const path = join(homedir() || ".", ".config", "umans-harness", "settings.json");
  try {
    if (!existsSync(path)) return {};
    const raw = readFileSync(path, "utf8");
    const s = JSON.parse(raw) as Record<string, unknown>;
    const out: HarnessSettings = {};
    if (typeof s.api_key === "string" && s.api_key) out.apiKey = s.api_key;
    if (typeof s.model === "string" && s.model) out.model = s.model;
    if (typeof s.base_url === "string" && s.base_url) out.baseUrl = s.base_url;
    if (typeof s.provider === "string" && s.provider) out.provider = s.provider;
    return out;
  } catch {
    return {};
  }
}

interface CoreRoot {
  binary: string;
  root: string;
}

/** 64-bit FNV-1a hash (matches the Go TUI's fnv64a). Returns a hex string. */
function fnv64aHex(s: string): string {
  // FNV-1a operates on bytes. The TUI hashes the UTF-8 of the cwd path; use the
  // same byte representation so session dirs align across TUI and web.
  const bytes = Buffer.from(s, "utf8");
  let h = BigInt("0xcbf29ce484222325");
  const prime = BigInt("0x100000001b3");
  const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
  for (let i = 0; i < bytes.length; i++) {
    h ^= BigInt(bytes[i]);
    h = (h * prime) & mask;
  }
  return h.toString(16);
}

/** The per-workspace session directory (mirrors the TUI's sessionsDir()). */
function sessionsDir(workspace: string): string {
  const home = homedir() || ".";
  const cfg = join(home, ".config", "umans-harness", "sessions");
  return join(cfg, fnv64aHex(workspace));
}

/** Pick the most-recently-modified .jsonl in the workspace's session dir, or a
 *  fresh timestamped name if none exists (mirrors the TUI's sessionPath()). */
function resolveSessionFile(workspace: string): string {
  const dir = sessionsDir(workspace);
  try {
    const entries = readdirSync(dir);
    let best: string | null = null;
    let bestMtime = -1;
    for (const name of entries) {
      if (!name.endsWith(".jsonl")) continue;
      const full = join(dir, name);
      const st = statSync(full);
      if (st.isDirectory()) continue;
      const mt = st.mtimeMs;
      if (mt > bestMtime) {
        bestMtime = mt;
        best = name;
      }
    }
    if (best) return join(dir, best);
  } catch {
    /* dir missing — fall through to a fresh name */
  }
  const t = new Date();
  const pad = (n: number, l = 2) => String(n).padStart(l, "0");
  const stamp = `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}_${pad(t.getHours())}-${pad(t.getMinutes())}-${pad(t.getSeconds())}`;
  const ns = String(t.getMilliseconds()).padStart(3, "0") + "000000";
  return join(dir, `${stamp}_${ns}.jsonl`);
}

/** Walk up from cwd to find the built core binary; return its repo root too. */
function resolveCore(): CoreRoot {
  const env = process.env.UMANS_CORE;
  if (env && env.trim()) return { binary: env.trim(), root: process.cwd() };
  const exe = process.platform === "win32" ? ".exe" : "";
  const names = [`core${exe}`, `umans-core${exe}`];
  let dir = process.cwd();
  for (let i = 0; i < 8; i++) {
    for (const name of names) {
      const cand = join(dir, "core", "target", "release", name);
      if (existsSync(cand)) return { binary: cand, root: dir };
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return { binary: names[0], root: process.cwd() };
}

type Sink = (ev: CoreEvent) => void;

class HarnessBridge {
  private core: CoreProcess | null = null;
  private state: AgentState = initialState;
  private sinks = new Set<Sink>();
  private starting: Promise<void> | null = null;
  private disposed = false;
  private crashCheck: ReturnType<typeof setInterval> | null = null;

  /** Ensure the core is running (respawn if it died). */
  async ensure(): Promise<void> {
    if (this.disposed) throw new Error("bridge disposed");
    if (this.core && !this.core.isRunning) {
      this.core = null;
      this.starting = null;
    }
    if (this.state.ready && this.core?.isRunning) return;
    if (this.starting) return this.starting;
    this.starting = this.start();
    return this.starting;
  }

  private async start(): Promise<void> {
    const { binary, root } = resolveCore();
    const workspace = process.env.UMANS_HARNESS_WORKSPACE ?? root;
    const settings = loadSettings();
    const env: Record<string, string> = {};
    if (settings.apiKey) env.UMANS_API_KEY = settings.apiKey;
    const core = new CoreProcess({
      cwd: workspace,
      binaryPath: binary,
      approval: "destructive",
      model: settings.model,
      baseUrl: settings.baseUrl,
      provider: settings.provider,
      sessionFile: resolveSessionFile(workspace),
      env,
    });
    this.core = core;

    // Register the event listener BEFORE awaiting `start()` so no stdout line is
    // missed (readline fires on later ticks; this runs synchronously first). The
    // SDK's CoreEvent is a loose shape; cast to our strict union at the boundary.
    core.on((ev) => this.onCoreEvent(ev as unknown as CoreEvent));

    let ready: ReadyPayload;
    try {
      ready = (await core.start()) as unknown as ReadyPayload;
    } catch (err: any) {
      this.core = null;
      this.starting = null;
      const msg = err?.message ?? String(err);
      this.fanout({ type: "error", message: `Failed to start umans-core: ${msg}` });
      throw err;
    }

    // `ready` is consumed by CoreProcess.start()'s resolver (not broadcast to
    // `on` listeners), so reduce + fan it out explicitly here.
    this.onCoreEvent(ready as unknown as CoreEvent);

    // Ask for the session list + stats so the UI is populated immediately.
    this.core.send({ type: "list_sessions" } as unknown as CoreEvent);
    this.core.send({ type: "stats" } as unknown as CoreEvent);
    this.core.send({ type: "list_memory" } as unknown as CoreEvent);
    this.core.send({ type: "list_plugins" } as unknown as CoreEvent);
    this.core.send({ type: "get_vision_config" } as unknown as CoreEvent);

    if (!this.crashCheck) {
      this.crashCheck = setInterval(() => this.checkAlive(), 2000);
    }
  }

  private checkAlive(): void {
    if (!this.core || this.disposed) return;
    if (this.core.isRunning) return;
    // The core exited unexpectedly. Surface it and allow a respawn on next use.
    this.core = null;
    this.starting = null;
    this.state = { ...this.state, ready: null, streaming: false, retrying: false, messages: [], currentAssistantId: null, pendingApproval: null };
    this.fanout({
      type: "error",
      message: "umans-core exited unexpectedly. Sending any message will restart it.",
    });
  }

  private onCoreEvent(ev: CoreEvent): void {
    this.state = reduce(this.state, ev);
    this.fanout(ev);
  }

  private fanout(ev: CoreEvent): void {
    for (const sink of this.sinks) {
      try {
        sink(ev);
      } catch {
        /* a dead client stream is non-fatal */
      }
    }
  }

  /** Atomically capture a snapshot and register a sink for future events. */
  subscribe(fn: Sink): { snapshot: AgentState; unsubscribe: () => void } {
    const snapshot: AgentState = this.state;
    this.sinks.add(fn);
    return { snapshot, unsubscribe: () => this.sinks.delete(fn) };
  }

  /** Forward a command to the core. For send/steer, also record the user message
   *  in the bridge state (for snapshot hydration) — the client adds it
   *  optimistically itself, so it is NOT fanned out. */
  send(cmd: CoreCommand): void {
    if (cmd.type === "send" || cmd.type === "steer") {
      this.state = reduce(this.state, {
        type: "_user",
        text: cmd.prompt,
        model: cmd.model,
        steer: cmd.type === "steer",
      });
    }
    if (!this.core) throw new Error("core not started");
    this.core.send(cmd as unknown as Parameters<CoreProcess["send"]>[0]);
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    if (this.crashCheck) clearInterval(this.crashCheck);
    this.sinks.clear();
    if (this.core) await this.core.dispose();
  }
}

// Singleton preserved across Next.js dev HMR.
const g = globalThis as unknown as { __UMANS_BRIDGE?: HarnessBridge };
export function getBridge(): HarnessBridge {
  if (!g.__UMANS_BRIDGE) g.__UMANS_BRIDGE = new HarnessBridge();
  return g.__UMANS_BRIDGE;
}
