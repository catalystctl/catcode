// LiveSession — one live conversation backed by its OWN catcode-core process.
//
// The web frontend used to keep a SINGLE core for the whole server: switching
// sessions or workspaces disposed it (killing any in-flight turn) and only one
// session could be "live" at a time. LiveSession fixes that — there is one
// CoreProcess per session file, kept alive independently of client connections
// so that:
//   • closing the tab and returning to a session shows it still running,
//     including in-flight tool calls / streaming deltas;
//   • switching between sessions and projects never tears down other live
//     sessions;
//   • many sessions can run concurrently, across sessions or projects.
//
// Each LiveSession reduces the raw core event stream into its own AgentState
// (so a reconnecting client hydrates instantly from a per-session snapshot) and
// fans events to its SSE subscribers. Workspace-level concerns (the session
// list, project list, custom titles) are shared across siblings and surfaced
// back to the owning HarnessBridge via callbacks.

import { CoreProcess } from "@catalyst-code/coding-agent";
import { reduce, initialState } from "@/lib/reducer";
import type { AgentEvent, AgentState, CoreCommand, CoreEvent, ReadyPayload } from "@/lib/types";
import { loadTitles } from "@/lib/session-titles";

export interface SessionEnv {
  binary: string;
  apiKey?: string;
  model?: string;
  baseUrl?: string;
  provider?: string;
}

/** Cross-session concerns the owning bridge handles (broadcasts, project list). */
export interface SessionCallbacks {
  /** A `sessions` list arrived for this workspace — re-broadcast to siblings. */
  onSessions: (workspace: string, ev: CoreEvent, originFile: string) => void;
  /** This session's core just became ready — touch the project + fan projects. */
  onReady: (workspace: string) => void;
  /** This session's core died unexpectedly (bridge may log / allow respawn). */
  onDead: (sessionFile: string) => void;
}

type Sink = (ev: CoreEvent) => void;

/** Idle sessions (no subscribers, not streaming, no activity) are reaped after
 *  this long to bound the number of live cores. Mid-turn sessions are NEVER
 *  reaped, so closing the tab mid-run and returning always shows it live. The
 *  default is generous (2h); reaped sessions reload seamlessly from disk when
 *  next viewed (conversation intact, core restarted). Tune via
 *  UMANS_WEB_IDLE_GC_MS; set 0 to disable reaping entirely. */
const IDLE_GC_MS = Number(process.env.UMANS_WEB_IDLE_GC_MS ?? 2 * 60 * 60 * 1000);

export class LiveSession {
  readonly workspace: string;
  readonly sessionFile: string;

  private core: CoreProcess | null = null;
  private state: AgentState;
  private sinks = new Set<Sink>();
  private starting: Promise<void> | null = null;
  private disposed = false;
  private crashCheck: ReturnType<typeof setInterval> | null = null;
  private titleMap: Record<string, string>;
  private readonly cb: SessionCallbacks;
  private readonly env: SessionEnv;

  /** Updated on any activity (command or event) — drives idle GC. */
  lastActivity: number;

  constructor(workspace: string, sessionFile: string, env: SessionEnv, cb: SessionCallbacks) {
    this.workspace = workspace;
    this.sessionFile = sessionFile;
    this.env = env;
    this.cb = cb;
    this.titleMap = loadTitles(workspace);
    this.lastActivity = Date.now();
    this.state = { ...initialState, workspace, currentSessionFile: sessionFile };
  }

  get isRunning(): boolean {
    return !!this.core?.isRunning;
  }

  get ready(): boolean {
    return !!this.state.ready;
  }

  get streaming(): boolean {
    return this.state.streaming;
  }

  get sinkCount(): number {
    return this.sinks.size;
  }

  /** Spawn the core (if needed) and resolve once `ready` has been reduced. */
  async ensure(): Promise<void> {
    if (this.disposed) throw new Error("session disposed");
    this.lastActivity = Date.now();
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
    const env: Record<string, string> = {};
    if (this.env.apiKey) env.UMANS_API_KEY = this.env.apiKey;
    const core = new CoreProcess({
      cwd: this.workspace,
      binaryPath: this.env.binary,
      approval: "destructive",
      model: this.env.model,
      baseUrl: this.env.baseUrl,
      provider: this.env.provider,
      sessionFile: this.sessionFile,
      env,
    });
    this.core = core;

    // Register the listener BEFORE awaiting `start()` so no stdout line is
    // missed. `ready` is consumed by CoreProcess.start()'s resolver and never
    // reaches `on` listeners, so we reduce + fan it out explicitly below.
    core.on((ev) => this.onCoreEvent(ev as unknown as CoreEvent));

    let ready: ReadyPayload;
    try {
      ready = (await core.start()) as unknown as ReadyPayload;
    } catch (err: any) {
      this.core = null;
      this.starting = null;
      const msg = err?.message ?? String(err);
      this.fanout({ type: "error", message: `Failed to start session: ${msg}` });
      throw err;
    }
    this.onCoreEvent(ready as unknown as CoreEvent);

    // Populate the UI immediately (per-session models/plugins/memories/vision +
    // the workspace session list + stats carrying the session_file).
    core.send({ type: "list_sessions" } as unknown as CoreCommand);
    core.send({ type: "stats" } as unknown as CoreCommand);
    core.send({ type: "list_memory" } as unknown as CoreCommand);
    core.send({ type: "list_plugins" } as unknown as CoreCommand);
    core.send({ type: "get_vision_config" } as unknown as CoreCommand);
    core.send({ type: "list_skills" } as unknown as CoreCommand);

    if (!this.crashCheck) {
      this.crashCheck = setInterval(() => this.checkAlive(), 5000);
    }
  }

  private checkAlive(): void {
    if (this.disposed) return;
    if (this.core?.isRunning) return;
    // The core exited unexpectedly. Surface it; the next command/ensure respawns.
    this.core = null;
    this.starting = null;
    this.state = {
      ...initialState,
      workspace: this.workspace,
      currentSessionFile: this.sessionFile,
    };
    this.fanout({
      type: "error",
      message: "This session's core exited. Sending a message will restart it.",
    });
  }

  /** Overlay web-layer custom session titles onto a `sessions` event. */
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
    this.lastActivity = Date.now();

    // `sessions` is workspace-level: re-broadcast to every live sibling so all
    // views of this workspace stay in sync (the originator already reduced it).
    if (ev.type === "sessions") {
      const enriched = this.enrich(ev);
      this.state = reduce(this.state, enriched);
      this.fanout(enriched);
      this.cb.onSessions(this.workspace, enriched, this.sessionFile);
      return;
    }

    if (ev.type === "ready") {
      this.cb.onReady(this.workspace);
    }

    this.state = reduce(this.state, ev);
    this.fanout(ev);

    // After a turn ends, refresh the session list + stats (mtimes / counts may
    // have changed) so every sibling's sidebar stays current.
    if (ev.type === "done" || ev.type === "aborted") {
      this.core?.send({ type: "list_sessions" } as unknown as CoreCommand);
      this.core?.send({ type: "stats" } as unknown as CoreCommand);
    }
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

  /** Atomically capture this session's snapshot and register a live sink. */
  subscribe(fn: Sink): { snapshot: AgentState; unsubscribe: () => void } {
    const snapshot: AgentState = this.state;
    this.sinks.add(fn);
    return { snapshot, unsubscribe: () => this.sinks.delete(fn) };
  }

  /** Reduce + fanout an event the bridge synthesized (projects, titles, …). */
  inject(ev: AgentEvent): void {
    this.state = reduce(this.state, ev);
    this.fanout(ev as CoreEvent);
  }

  /** Reload the title overlay (after a rename) and refresh the list. */
  reloadTitles(): void {
    this.titleMap = loadTitles(this.workspace);
    this.core?.send({ type: "list_sessions" } as unknown as CoreCommand);
  }

  /** Re-emit the session list (used after a sibling deletes a session file). */
  refreshSessions(): void {
    this.core?.send({ type: "list_sessions" } as unknown as CoreCommand);
  }

  /** Apply a rename overlay immediately (before the refreshed list lands). */
  applyRename(name: string, title: string): void {
    this.inject({ type: "_session_title", name, title });
  }

  /** Forward a command to this session's core (recording user messages for the
   *  snapshot). Callers MUST have `ensure()`d first. */
  send(cmd: CoreCommand): void {
    this.lastActivity = Date.now();
    if (cmd.type === "send" || cmd.type === "steer") {
      this.state = reduce(this.state, {
        type: "_user",
        text: cmd.prompt,
        model: cmd.model,
        steer: cmd.type === "steer",
      });
    }
    if (!this.core) throw new Error("session core not started");
    this.core.send(cmd as unknown as Parameters<CoreProcess["send"]>[0]);
  }

  /** Whether this session is safe to garbage-collect (see IDLE_GC_MS). */
  isIdle(now: number): boolean {
    if (IDLE_GC_MS <= 0) return false; // reaping disabled
    if (this.sinks.size > 0) return false; // has viewers
    if (this.state.streaming) return false; // mid-turn — never reap
    return now - this.lastActivity > IDLE_GC_MS;
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    if (this.crashCheck) clearInterval(this.crashCheck);
    this.crashCheck = null;
    this.sinks.clear();
    if (this.core) {
      try {
        await this.core.dispose();
      } catch {
        /* best-effort */
      }
    }
    this.core = null;
    this.starting = null;
  }
}
