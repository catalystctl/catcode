// HarnessBridge — the server-side singleton that owns one umans-core process,
// reduces the raw event stream into AgentState (so reconnecting clients can
// hydrate instantly), and fans core events out to SSE subscribers. Commands
// arrive via POST and are forwarded to the core; the bridge also feeds
// synthetic events into its own reducer for send/steer (user-message hydration)
// and session-rename / project overlays.
//
// Multi-workspace: the bridge can switch its bound workspace at runtime by
// disposing the current core and respawning with a new --workspace. Sessions,
// memories, and plugins are scoped per-workspace (mirroring the core). The
// project list (~/.config/umans-harness/projects.json) tracks recent workspaces
// so the picker can list/switch between them.
//
// Low-level CoreProcess layer (not the PI-compatible AgentSession): this gives
// full yes/no/always approval control and direct session/model/stats commands.

import { CoreProcess } from "@umans-harness/coding-agent";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import { reduce, initialState } from "@/lib/reducer";
import type { AgentEvent, AgentState, CoreCommand, CoreEvent, ProjectEntry, ReadyPayload } from "@/lib/types";
import { loadTitles, setTitle } from "@/lib/session-titles";
import { loadProjects, touchProject, removeProject } from "@/lib/projects";

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
  /** The workspace this bridge is currently bound to. */
  private workspace: string;
  /** Cached session-title overlay for the current workspace. */
  private titleMap: Record<string, string> = {};
  /** True while disposing/respawning the core for a workspace switch. */
  private switching = false;

  constructor() {
    const { root } = resolveCore();
    this.workspace = process.env.UMANS_HARNESS_WORKSPACE ?? root;
  }

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
    const { binary } = resolveCore();
    const workspace = this.workspace;
    this.titleMap = loadTitles(workspace);
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

    // Record this workspace in the recent-projects list + emit it.
    const projects = touchProject(workspace);
    this.dispatch({ type: "projects", projects });

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

  /** Overlay web-layer custom session titles onto a `sessions` event before it
   *  is reduced / fanned out, so both the snapshot and live clients see the
   *  user-chosen names. Returns the (possibly mutated) event. */
  private enrich(ev: CoreEvent): CoreEvent {
    if (ev.type !== "sessions") return ev;
    if (!this.titleMap || Object.keys(this.titleMap).length === 0) return ev;
    const sessions = ev.sessions.map((s) => {
      const custom = this.titleMap[s.name];
      return custom ? { ...s, title: custom } : s;
    });
    return { ...ev, sessions };
  }

  private onCoreEvent(ev: CoreEvent): void {
    const enriched = this.enrich(ev);
    this.state = reduce(this.state, enriched);
    this.fanout(enriched);
  }

  /** Reduce + fanout any event (core or synthetic). Used by the bridge to push
   *  synthetic state changes (project list, session-rename overlay, switching)
   *  to both its snapshot and all connected clients. */
  private dispatch(ev: AgentEvent): void {
    this.state = reduce(this.state, ev);
    this.fanout(ev as CoreEvent);
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

  /** Switch the bound workspace: dispose the current core, reset state, and
   *  respawn with the new --workspace. Clients see a `_set_switching` event,
   *  then `projects` + `workspace_changed` once the new core is ready. */
  async switchWorkspace(path: string): Promise<void> {
    if (this.switching) return;
    if (path === this.workspace && this.state.ready) return;
    this.switching = true;

    // Tear down the current core.
    const oldCore = this.core;
    this.core = null;
    this.starting = null;
    if (oldCore) {
      try {
        await oldCore.dispose();
      } catch {
        /* best-effort */
      }
    }

    // Reset state for the new workspace (keep projects + sinks).
    this.workspace = path;
    this.titleMap = loadTitles(path);
    const projects = touchProject(path);
    this.state = { ...initialState, projects, switching: true, workspace: path };
    this.dispatch({ type: "_set_switching", switching: true });
    this.dispatch({ type: "projects", projects });

    try {
      await this.ensure();
    } catch {
      this.switching = false;
      this.dispatch({ type: "_set_switching", switching: false });
      return;
    }

    // The new core is ready (ready event already reduced via onCoreEvent).
    // Finalize the switch: clear switching, reset per-session state.
    this.switching = false;
    this.dispatch({
      type: "workspace_changed",
      workspace: path,
      projects,
    });
  }

  /** Rename a session (web-layer overlay). Persists to the titles file and
   *  pushes a `_session_title` synthetic event so all clients update. */
  renameSession(name: string, title: string): void {
    this.titleMap = setTitle(this.workspace, name, title);
    this.dispatch({ type: "_session_title", name, title });
  }

  /** List known projects (recent workspaces). */
  listProjects(): ProjectEntry[] {
    return loadProjects();
  }

  /** The workspace this bridge is currently bound to. */
  getWorkspace(): string {
    return this.workspace;
  }

  /** Add a workspace to the recent-projects list. */
  addProject(path: string): ProjectEntry[] {
    const projects = touchProject(path);
    this.dispatch({ type: "projects", projects });
    return projects;
  }

  /** Remove a workspace from the recent-projects list. */
  removeProjectEntry(path: string): ProjectEntry[] {
    const projects = removeProject(path);
    this.dispatch({ type: "projects", projects });
    return projects;
  }

  /** Forward a command to the core. For send/steer, also record the user message
   *  in the bridge state (for snapshot hydration) — the client adds it
   *  optimistically itself, so it is NOT fanned out. Web-layer commands
   *  (switch_workspace, rename_session, list/add/remove_project) are handled
   *  here and NOT forwarded to the core. */
  send(cmd: CoreCommand): void {
    // ── Web-layer intercepts (never forwarded to the core) ──
    switch (cmd.type) {
      case "switch_workspace":
        // Fire-and-forget; the client set switching=true optimistically and
        // will receive workspace_changed when the respawn completes.
        void this.switchWorkspace(cmd.path);
        return;
      case "rename_session":
        this.renameSession(cmd.name, cmd.title);
        return;
      case "list_projects":
        this.dispatch({ type: "projects", projects: loadProjects() });
        return;
      case "add_project":
        this.addProject(cmd.path);
        return;
      case "remove_project":
        this.removeProjectEntry(cmd.path);
        return;
    }

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
