// HarnessBridge — a POOL of live sessions, one catcode-core process per session.
//
// Previously the bridge owned a SINGLE CoreProcess for the whole server, so
// switching sessions or workspaces disposed it (killing any in-flight turn) and
// only one session could be live at a time. Now each session file gets its own
// LiveSession (its own CoreProcess + AgentState + SSE subscribers), kept alive
// independently of client connections:
//   • close the tab and return → the session is still running, in-flight tool
//     calls / streaming deltas are visible;
//   • switch between sessions and projects → other live sessions keep running;
//   • many sessions run concurrently, across sessions or projects.
//
// The bridge routes commands and SSE subscriptions to the right LiveSession,
// shares workspace-level state across siblings (session list, project list,
// custom titles), and garbage-collects idle sessions (see LiveSession.isIdle).
// Workspaces are NOT respawned on switch — switching just selects a session in
// the target workspace, leaving every other workspace's sessions untouched.
//
// Low-level CoreProcess layer (not the PI-compatible AgentSession): this gives
// full yes/no/always approval control and direct session/model/stats commands.

import { existsSync, readdirSync, statSync } from "node:fs";
import { join, dirname, relative, normalize, sep } from "node:path";
import { resolveCoreBinary, configDir } from "@catalyst-code/coding-agent";
import type { AgentEvent, AgentState, CoreCommand, CoreEvent, LiveSessionStatus, ProjectEntry, ReadyPayload } from "@/lib/types";
import { loadTitles, setTitle } from "@/lib/session-titles";
import { loadProjects, touchProject, removeProject } from "@/lib/projects";
import { LiveSession, type SessionCallbacks, type SessionEnv } from "./live-session";
import { loadSettings } from "./settings-file";

interface CoreRoot {
  binary: string;
  root: string;
}

// SDK-GAP: fnv64aHex / sessionsDirFor / newSessionFilename exist in the SDK
// (sdk/src/config.ts) but are NOT exported from the public barrel (index.ts).
// The SDK's SessionManager uses them internally. Keep local copies until the
// SDK promotes these helpers to the public API surface.

/** 64-bit FNV-1a hash (matches SDK + Go TUI + Rust core). */
function fnv64aHex(s: string): string {
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

/** The per-workspace session directory (mirrors the TUI's sessionsDir()).
 *  Uses the SDK's configDir() so the path is identical to the SDK/TUI. */
export function sessionsDir(workspace: string): string {
  return join(configDir(), "sessions", fnv64aHex(workspace));
}

function pad(n: number, l = 2): string {
  return String(n).padStart(l, "0");
}

/** A fresh timestamped .jsonl name (mirrors the core's new_session_filename()). */
export function freshSessionFile(workspace: string): string {
  const dir = sessionsDir(workspace);
  const t = new Date();
  const stamp = `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}_${pad(t.getHours())}-${pad(t.getMinutes())}-${pad(t.getSeconds())}`;
  const ns = String(t.getMilliseconds()).padStart(3, "0") + "000000";
  return join(dir, `${stamp}_${ns}.jsonl`);
}

/** Pick the most-recently-modified .jsonl in the workspace's session dir, or a
 *  fresh timestamped name if none exists (mirrors the TUI's sessionPath()). */
export function resolveSessionFile(workspace: string): string {
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
  return freshSessionFile(workspace);
}

/** Resolve the core binary and repo root for the default workspace.
 *  Uses the SDK's resolveCoreBinary (handles CATCODE_CORE env, dev paths, PATH)
 *  with a walk-up fallback for the common dev layout where the web server cwd
 *  is a subdirectory of the repo root (e.g. web/). */
function resolveCore(): CoreRoot {
  const sdkBinary = resolveCoreBinary();
  // If the SDK resolved to an actual on-disk file, use it.
  if (existsSync(sdkBinary)) {
    return { binary: sdkBinary, root: process.cwd() };
  }
  // Fallback: walk up from cwd looking for core/target/release/{core,catcode-core}.
  // This catches the dev case where cwd is web/ but the binary is at ../core/target/release/.
  const exe = process.platform === "win32" ? ".exe" : "";
  const names = [`core${exe}`, `catcode-core${exe}`];
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
  return { binary: sdkBinary, root: process.cwd() };
}

class HarnessBridge {
  /** Live sessions keyed by absolute session-file path. */
  private sessions = new Map<string, LiveSession>();
  private disposed = false;
  private gcTimer: ReturnType<typeof setInterval> | null = null;
  /** Workspace used when no session is specified (the initial connection). */
  private defaultWorkspace: string;
  private cachedEnv: SessionEnv | null = null;

  constructor() {
    const { root } = resolveCore();
    this.defaultWorkspace = process.env.CATALYST_CODE_WORKSPACE ?? root;
    // Reap idle sessions every minute so the live-core count stays bounded.
    // Mid-turn sessions and sessions with active viewers are never reaped.
    this.gcTimer = setInterval(() => this.gcIdle(), 60_000);
  }

  private env(): SessionEnv {
    // Re-read settings each time so a TUI/web approval change is visible to
    // newly spawned sessions (the binary/model bits rarely change).
    const { binary } = resolveCore();
    const s = loadSettings();
    this.cachedEnv = {
      binary,
      apiKey: s.apiKey,
      model: s.model,
      baseUrl: s.baseUrl,
      provider: s.provider,
      approval: s.approval,
    };
    return this.cachedEnv;
  }

  private callbacks(): SessionCallbacks {
    return {
      onSessions: (ws, ev, originFile) => this.broadcastSessions(ws, ev, originFile),
      onReady: (ws) => this.onSessionReady(ws),
      onDead: () => {
        /* left in the map; next ensure() respawns */
      },
      onStatusChange: () => this.broadcastStatus(),
      onStatusSnapshot: () => this.statusList(),
    };
  }

  /** Snapshot of every live session's status (source of truth for the
   *  cross-session session_status broadcast + fresh-subscriber seeding). */
  private statusList(): LiveSessionStatus[] {
    return [...this.sessions.values()].map((s) => s.status());
  }

  /** Fan a session_status snapshot to EVERY live session (all workspaces), so a
   *  client viewing any session/project learns about every other live session's
   *  streaming / attention state. Global like broadcastProjects, not
   *  workspace-scoped like broadcastSessions. */
  private broadcastStatus(): void {
    if (this.disposed) return;
    const sessions = this.statusList();
    const ev: CoreEvent = { type: "session_status", sessions };
    for (const s of this.sessions.values()) {
      s.inject(ev as unknown as AgentEvent);
    }
  }

  /** Find or create the LiveSession for a session file. Does NOT start it. */
  getOrCreate(workspace: string, sessionFile: string): LiveSession {
    let s = this.sessions.get(sessionFile);
    if (!s) {
      s = new LiveSession(workspace, sessionFile, this.env(), this.callbacks());
      this.sessions.set(sessionFile, s);
    }
    return s;
  }

  /** Ensure the session's core is running. If `sessionFile` is omitted, use the
   *  workspace's most-recent session (creating one if the workspace is empty). */
  async ensure(workspace?: string, sessionFile?: string): Promise<LiveSession> {
    const ws = workspace ?? this.defaultWorkspace;
    const file = sessionFile ?? resolveSessionFile(ws);
    const s = this.getOrCreate(ws, file);
    await s.ensure();
    return s;
  }

  /** The default workspace (initial connection with no session specified). */
  getDefaultWorkspace(): string {
    return this.defaultWorkspace;
  }

  /** Workspace of an already-live session (undefined if not live). */
  getWorkspaceForSession(sessionFile: string | undefined): string | undefined {
    if (!sessionFile) return undefined;
    return this.sessions.get(sessionFile)?.workspace;
  }

  /** Most-recent session file for a workspace (or a fresh name if empty). */
  mostRecentSession(workspace: string): string {
    return resolveSessionFile(workspace);
  }

  // ── Workspace switching (no respawn — other sessions keep running) ──

  /** Switch the client to a workspace: record it, fan the project list, and
   *  ensure its most-recent session is live. Returns the session to show. */
  async switchWorkspace(path: string): Promise<{ session: string; workspace: string }> {
    touchProject(path);
    this.broadcastProjects();
    const file = resolveSessionFile(path);
    const s = this.getOrCreate(path, file);
    await s.ensure();
    return { session: file, workspace: path };
  }

  /** Start a brand-new session file in a workspace and return it. */
  async newSession(workspace: string): Promise<{ session: string; workspace: string }> {
    const file = freshSessionFile(workspace);
    const s = this.getOrCreate(workspace, file);
    await s.ensure(); // starts the core, which creates the file (session::ensure)
    return { session: file, workspace };
  }

  /** Rename a session (web-layer overlay). Broadcast to all live sessions in
   *  the workspace so every view updates. */
  renameSession(workspace: string, name: string, title: string): void {
    setTitle(workspace, name, title);
    for (const s of this.sessions.values()) {
      if (s.workspace !== workspace) continue;
      s.reloadTitles();
      s.applyRename(name, title);
    }
  }

  // ── Cross-session broadcasts ──

  /** Fan a workspace-level `sessions` list to every live sibling except the
   *  originator (which already reduced it). */
  private broadcastSessions(workspace: string, ev: CoreEvent, originFile: string): void {
    for (const s of this.sessions.values()) {
      if (s.workspace !== workspace) continue;
      if (s.sessionFile === originFile) continue;
      s.inject(ev);
    }
  }

  /** Fan the global project list to EVERY live session (all workspaces). */
  broadcastProjects(): ProjectEntry[] {
    const projects = loadProjects();
    const ev: CoreEvent = { type: "projects", projects };
    for (const s of this.sessions.values()) s.inject(ev);
    return projects;
  }

  addProject(path: string): ProjectEntry[] {
    const projects = touchProject(path);
    const ev: CoreEvent = { type: "projects", projects };
    for (const s of this.sessions.values()) s.inject(ev);
    return projects;
  }

  removeProjectEntry(path: string): ProjectEntry[] {
    const projects = removeProject(path);
    const ev: CoreEvent = { type: "projects", projects };
    for (const s of this.sessions.values()) s.inject(ev);
    return projects;
  }

  /** Delete a session file from disk + dispose its live core (if any). Returns
   *  the most-recent remaining session (or a fresh name) so a client that was
   *  viewing the deleted session can switch. Refreshes every live sibling's
   *  session list so all sidebars update. */
  async deleteSession(workspace: string, sessionFile: string): Promise<{ session: string; workspace: string }> {
    // Confine the file to this workspace's session directory (no escaping).
    const dir = sessionsDir(workspace);
    const resolved = normalize(sessionFile);
    const rel = relative(dir, resolved);
    if (rel === "" || rel.startsWith("..") || rel.includes(`..${sep}`)) {
      throw new Error("session file outside its workspace session dir");
    }
    // Dispose the live core so it stops writing to the (now deleted) file.
    const live = this.sessions.get(sessionFile);
    if (live) {
      this.sessions.delete(sessionFile);
      await live.dispose().catch(() => {});
    }
    // Delete the file from disk (best-effort).
    try {
      const { unlinkSync } = await import("node:fs");
      unlinkSync(resolved);
    } catch {
      /* already gone — fine */
    }
    // Refresh the session list on every live sibling so sidebars update.
    for (const s of this.sessions.values()) {
      if (s.workspace === workspace) s.refreshSessions();
    }
    // The deleted session left the live map — re-broadcast status so clients
    // drop its sidebar badge / feed entry.
    this.broadcastStatus();
    return { session: resolveSessionFile(workspace), workspace };
  }

  /** A session's core became ready — bump its workspace in the recent list and
   *  fan the project list to all sessions. */
  private onSessionReady(workspace: string): void {
    touchProject(workspace);
    this.broadcastProjects();
  }

  // ── Idle GC ──

  private gcIdle(): void {
    if (this.disposed) return;
    const now = Date.now();
    let reaped = false;
    for (const [file, s] of this.sessions) {
      if (!s.isIdle(now)) continue;
      this.sessions.delete(file);
      void s.dispose();
      reaped = true;
    }
    // A reaped session leaves the live map — re-broadcast so clients drop it.
    if (reaped) this.broadcastStatus();
  }

  // ── Lifecycle ──

  /** Snapshot of live sessions (for diagnostics). */
  liveSessions(): { workspace: string; sessionFile: string; running: boolean; streaming: boolean; viewers: number }[] {
    return [...this.sessions.values()].map((s) => ({
      workspace: s.workspace,
      sessionFile: s.sessionFile,
      running: s.isRunning,
      streaming: s.streaming,
      viewers: s.sinkCount,
    }));
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    if (this.gcTimer) clearInterval(this.gcTimer);
    this.gcTimer = null;
    const all = [...this.sessions.values()];
    this.sessions.clear();
    await Promise.all(all.map((s) => s.dispose().catch(() => {})));
  }
}

// Avoid unused-import warnings for types re-exported via the bridge surface.
export type { ReadyPayload, AgentState, CoreCommand };

// Singleton preserved across Next.js dev HMR.
const g = globalThis as unknown as { __CATALYST_BRIDGE?: HarnessBridge };
export function getBridge(): HarnessBridge {
  if (!g.__CATALYST_BRIDGE) g.__CATALYST_BRIDGE = new HarnessBridge();
  return g.__CATALYST_BRIDGE;
}
