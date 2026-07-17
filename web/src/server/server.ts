// web/src/server/server.ts
//
// Custom Next.js server: serves the Next app (HTTP) AND a WebSocket terminal
// endpoint at /api/terminal on the SAME port. Next app-router route handlers
// cannot upgrade to WebSocket, so the WS is attached to the raw http.Server.
//
// Run:
//   dev : `npm run dev` (or `node --import tsx src/server/server.ts`)
//   prod: `NODE_ENV=production node --import tsx src/server/server.ts`
//
// IMPORTANT: relative imports only (not "@/…"). Bun runs this TS directly and a
// future tsc compile would not rewrite path aliases — relative paths keep the
// emitted file Node-resolvable.
import { createServer, type IncomingMessage, type Server as HttpServer } from "node:http";
import { type Duplex } from "node:stream";
import { parse } from "node:url";
import { normalize, join, relative, resolve, sep } from "node:path";
import next from "next";
import * as pty from "@lydell/node-pty";
import type { IPty } from "@lydell/node-pty";
import { WebSocketServer, WebSocket, type RawData } from "ws";
import { getSession } from "../lib/auth";
import { loadProjects } from "../lib/projects";
import { getBridge } from "./core-bridge";

const dev = process.env.NODE_ENV !== "production";
const port = Number(process.env.PORT) || 3000;
const hostname = process.env.HOSTNAME || "0.0.0.0";

// ── workspace path confinement ─────────────────────────────────────────────
// Verbatim from api/files/route.ts:80-85 / contract §4.6. Rejects ".."
// traversal so a crafted cwd can never escape the workspace.
function confinePath(workspace: string, rel: string): string {
  const abs = normalize(join(workspace, rel));
  const r = relative(workspace, abs);
  if (r.startsWith("..") || r.includes(`..${sep}`)) {
    throw new Error("path outside workspace");
  }
  return abs;
}

// Convert Node IncomingHttpHeaders → web Headers so better-auth's
// getSession (which reads the cookie header) can validate the WS upgrade.
function toHeaders(h: IncomingMessage["headers"]): Headers {
  const headers = new Headers();
  for (const [k, v] of Object.entries(h)) {
    if (typeof v === "string") headers.set(k, v);
    else if (Array.isArray(v)) for (const x of v) headers.append(k, x);
  }
  return headers;
}

// ws delivers message payloads as Buffer | Buffer[] | ArrayBuffer.
function rawToString(raw: RawData): string {
  if (Buffer.isBuffer(raw)) return raw.toString("utf8");
  if (Array.isArray(raw)) return Buffer.concat(raw).toString("utf8");
  return Buffer.from(raw).toString("utf8");
}

// ── WS message envelopes (contract §4.5) ───────────────────────────────────
interface OpenMsg {
  type: "open";
  sessionId: string;
  workspace?: string;
  cwd?: string;
  cols?: number;
  rows?: number;
}
interface DataMsg { type: "data"; data: string; }
interface ResizeMsg { type: "resize"; cols: number; rows: number; }
interface PingMsg { type: "ping"; }
interface TerminateMsg { type: "terminate"; sessionId: string; }
type ClientMsg = OpenMsg | DataMsg | ResizeMsg | PingMsg | TerminateMsg;

function send(ws: WebSocket, msg: Record<string, unknown>): void {
  if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function fail(ws: WebSocket, message: string): void {
  send(ws, { type: "data", data: `\r\nerror: ${message}\r\n` });
  send(ws, { type: "exit", code: 1 });
  try { ws.close(); } catch { /* already closed */ }
}

const MAX_SCROLLBACK_BYTES = 2 * 1024 * 1024;
// Exited shells are kept briefly so a refresh can still show the exit banner.
// Live PTYs are NEVER reaped on detach — they survive hard refresh until the
// user hits × (terminate) or the web process restarts.
const EXITED_TTL_MS = 5 * 60 * 1000;

interface TerminalProcess {
  key: string;
  id: string;
  ownerId: string;
  workspace: string;
  pty: IPty | null;
  clients: Set<WebSocket>;
  scrollback: string;
  exitCode: number | null;
  cleanupTimer: ReturnType<typeof setTimeout> | null;
}

const terminals = new Map<string, TerminalProcess>();

function terminalKey(ownerId: string, workspace: string, sessionId: string): string {
  return `${ownerId}:${workspace}:${sessionId}`;
}

function validSessionId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}

function clampDimension(value: number | undefined, fallback: number, max: number): number {
  return Number.isFinite(value) ? Math.max(2, Math.min(max, Math.floor(value!))) : fallback;
}

/** Resolve a client workspace root to an allowlisted project path. */
function authorizedWorkspace(candidate: string | undefined): string {
  const fallback = getBridge().getDefaultWorkspace();
  if (!candidate || typeof candidate !== "string") return fallback;
  const requested = resolve(candidate);
  const allowed = [fallback, ...loadProjects().map((project) => project.path)].map((workspace) =>
    resolve(workspace),
  );
  if (!allowed.includes(requested)) {
    throw new Error("unauthorized workspace");
  }
  return requested;
}

function findOwnedTerminal(ownerId: string, sessionId: string): TerminalProcess | undefined {
  for (const session of terminals.values()) {
    if (session.ownerId === ownerId && session.id === sessionId) return session;
  }
  return undefined;
}

function broadcast(session: TerminalProcess, msg: Record<string, unknown>): void {
  for (const client of session.clients) send(client, msg);
}

function destroyTerminal(session: TerminalProcess): void {
  if (session.cleanupTimer) clearTimeout(session.cleanupTimer);
  terminals.delete(session.key);
  try { session.pty?.kill(); } catch { /* already exited */ }
  session.pty = null;
  for (const client of session.clients) {
    send(client, { type: "terminated" });
    try { client.close(); } catch { /* already closed */ }
  }
  session.clients.clear();
}

function scheduleExitedCleanup(session: TerminalProcess): void {
  if (terminals.get(session.key) !== session || session.pty || session.clients.size || session.cleanupTimer) {
    return;
  }
  session.cleanupTimer = setTimeout(() => destroyTerminal(session), EXITED_TTL_MS);
  session.cleanupTimer.unref?.();
}

function createTerminal(ownerId: string, msg: OpenMsg): TerminalProcess {
  let workspace: string;
  try {
    // Prefer the client project workspace when it is an allowlisted path so
    // terminals are per-project. Fall back to the process default otherwise.
    workspace = authorizedWorkspace(msg.workspace);
  } catch (err) {
    throw new Error(`no workspace configured: ${String(err)}`);
  }

  let cwd = workspace;
  if (msg.cwd) {
    try {
      cwd = confinePath(workspace, msg.cwd);
    } catch {
      throw new Error("cwd outside workspace");
    }
  }

  const cols = clampDimension(msg.cols, 80, 500);
  const rows = clampDimension(msg.rows, 24, 300);

  const shell =
    process.env.SHELL ||
    (process.platform === "win32" ? process.env.COMSPEC || "cmd.exe" : "/bin/sh");
  const env = Object.fromEntries(Object.entries({
    ...process.env,
    TERM: "xterm-256color",
    COLORTERM: "truecolor",
    TERM_PROGRAM: "Catalyst Code",
    COLUMNS: String(cols),
    LINES: String(rows),
  }).filter((entry): entry is [string, string] => typeof entry[1] === "string"));

  const key = terminalKey(ownerId, workspace, msg.sessionId);
  const terminal: TerminalProcess = {
    key,
    id: msg.sessionId,
    ownerId,
    workspace,
    pty: null,
    clients: new Set(),
    scrollback: "",
    exitCode: null,
    cleanupTimer: null,
  };
  terminal.pty = pty.spawn(shell, [], { name: "xterm-256color", cwd, env, cols, rows });
  terminal.pty.onData((data) => {
    terminal.scrollback += data;
    if (Buffer.byteLength(terminal.scrollback) > MAX_SCROLLBACK_BYTES) {
      terminal.scrollback = terminal.scrollback.slice(-MAX_SCROLLBACK_BYTES);
    }
    broadcast(terminal, { type: "data", data });
  });
  terminal.pty.onExit(({ exitCode, signal }) => {
    terminal.exitCode = exitCode ?? (signal ? 128 + signal : 0);
    terminal.pty = null;
    broadcast(terminal, { type: "exit", code: terminal.exitCode });
    scheduleExitedCleanup(terminal);
  });
  terminals.set(key, terminal);
  return terminal;
}

// Per-connection handler. Live PTYs outlive sockets so panel switches and hard
// refreshes detach/reattach with scrollback replay until × or server restart.
function handleTerminal(ws: WebSocket, ownerId: string): void {
  let attached: TerminalProcess | null = null;

  ws.on("message", (raw: RawData) => {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(rawToString(raw)) as ClientMsg;
    } catch {
      return; // ignore malformed envelopes
    }

    if (!attached) {
      if (msg.type === "terminate") {
        if (!validSessionId(msg.sessionId)) { fail(ws, "invalid terminal session ID"); return; }
        const existing = findOwnedTerminal(ownerId, msg.sessionId);
        if (existing) destroyTerminal(existing);
        else { send(ws, { type: "terminated" }); ws.close(); }
        return;
      }
      if (msg.type !== "open" || !validSessionId(msg.sessionId)) {
        fail(ws, 'expected a valid {type:"open", sessionId} message');
        return;
      }
      let workspace: string;
      try {
        workspace = authorizedWorkspace(msg.workspace);
      } catch (err) {
        fail(ws, `failed to open terminal: ${String(err)}`);
        return;
      }
      const key = terminalKey(ownerId, workspace, msg.sessionId);
      try {
        attached = terminals.get(key) ?? createTerminal(ownerId, msg);
      } catch (err) {
        fail(ws, `failed to open terminal: ${String(err)}`);
        return;
      }
      if (attached.cleanupTimer) clearTimeout(attached.cleanupTimer);
      attached.cleanupTimer = null;
      attached.clients.add(ws);
      send(ws, { type: "ready", sessionId: attached.id });
      if (attached.scrollback) send(ws, { type: "data", data: attached.scrollback, replay: true });
      if (attached.exitCode !== null) send(ws, { type: "exit", code: attached.exitCode });
      return;
    }

    switch (msg.type) {
      case "data":
        if (typeof msg.data === "string" && msg.data.length <= 1024 * 1024) attached.pty?.write(msg.data);
        break;
      case "resize":
        attached.pty?.resize(
          clampDimension(msg.cols, attached.pty.cols, 500),
          clampDimension(msg.rows, attached.pty.rows, 300),
        );
        break;
      case "terminate":
        if (msg.sessionId === attached.id) destroyTerminal(attached);
        break;
      case "ping":
        send(ws, { type: "pong" });
        break;
      default:
        break;
    }
  });

  const cleanup = (): void => {
    if (!attached) return;
    attached.clients.delete(ws);
    // Live shells stay running with no TTL. Only exited shells are reaped.
    if (!attached.pty) scheduleExitedCleanup(attached);
    attached = null;
  };
  ws.on("close", cleanup);
  ws.on("error", cleanup);
}

// ── boot Next + attach WS ──────────────────────────────────────────────────
// Keep the fast Turbopack development pipeline while wrapping Next in our raw
// HTTP server. The installed Next 15 server API exposes `turbopack` directly;
// production continues to serve the regular prebuilt output.
const app = next({ dev, hostname, port, turbopack: dev });
const handle = app.getRequestHandler();

await app.prepare();

const server: HttpServer = createServer((req, res) => {
  const parsedUrl = parse(req.url!, true);
  handle(req, res, parsedUrl);
});

const wss = new WebSocketServer({ noServer: true });

// Authenticate the WS upgrade (same-origin; better-auth cookies are host-only
// so same-host any-port works) and, on success, hand off to wss.handleUpgrade.
function handleTerminalUpgrade(req: IncomingMessage, socket: Duplex, head: Buffer): void {
  const origin = req.headers.origin;
  if (origin) {
    try {
      if (new URL(origin).host !== req.headers.host) {
        socket.write("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
        socket.destroy();
        return;
      }
    } catch {
      socket.destroy();
      return;
    }
  }
  getSession(toHeaders(req.headers))
    .then((session) => {
      if (!session) {
        socket.write("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
        socket.destroy();
        return;
      }
      wss.handleUpgrade(req, socket, head, (ws) => handleTerminal(ws, session.user.id));
    })
    .catch((err) => {
      console.error("[terminal] auth error:", err);
      socket.destroy();
    });
}

// Intercept the 'upgrade' event at the emit level so /api/terminal is handled
// EXCLUSIVELY by our WS transport. Next.js, in custom-server prod mode,
// attaches its own 'upgrade' listener that aborts unknown upgrade paths; if it
// runs it destroys the /api/terminal socket before the async getSession +
// wss.handleUpgrade can complete the WS handshake (surfacing as "Connection
// closed before receiving a handshake response"). Removing sibling listeners is
// unreliable here — Next attaches lazily AND Node clones the listener array per
// emit, so a sibling removed mid-emit still fires. Gating the event itself is
// deterministic: /api/terminal → our transport (origEmit NOT called, so no
// other listener runs); any other upgrade path → delegate to the original emit
// (Next's HMR in dev, a no-op in prod). The override touches ONLY the
// 'upgrade' event; every other event passes through unchanged.
const origEmit = server.emit.bind(server);
server.emit = function (event: string, ...args: unknown[]): boolean {
  if (event === "upgrade" && args.length >= 3) {
    const req = args[0] as IncomingMessage;
    const { pathname } = parse(req.url ?? "/", true);
    if (pathname === "/api/terminal") {
      handleTerminalUpgrade(req, args[1] as Duplex, args[2] as Buffer);
      return true;
    }
  }
  return origEmit(event, ...(args as unknown[]));
};

server.listen(port, hostname, () => {
  console.log(`> catalyst-code web ready on http://${hostname}:${port} (dev=${dev})`);
});
