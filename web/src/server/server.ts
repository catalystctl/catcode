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
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { StringDecoder } from "node:string_decoder";
import { normalize, join, relative, sep } from "node:path";
import next from "next";
import { WebSocketServer, WebSocket, type RawData } from "ws";
import { getSession } from "../lib/auth";
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

// xterm.js expects CRLF line endings (it does not auto-CR on LF). Normalise
// every chunk we send back so shell output renders on its own line.
function toOut(s: string): string {
  return s.replace(/\r?\n/g, "\r\n");
}

// No-PTY local echo: the shell's stdin is a pipe (not a TTY) so it never echoes
// typed input. Mirror input back to the client so the user can see what they
// type. Enter → newline, Backspace/DEL → erase one cell.
function toEcho(s: string): string {
  return s
    .replace(/\r/g, "\r\n")
    .replace(/\n/g, "\r\n")
    .replace(/\x7f/g, "\b \b")
    .replace(/\x08/g, "\b \b");
}

// ── WS message envelopes (contract §4.5) ───────────────────────────────────
interface OpenMsg {
  type: "open";
  workspace?: string;
  cwd?: string;
  cols?: number;
  rows?: number;
}
interface DataMsg { type: "data"; data: string; }
interface ResizeMsg { type: "resize"; cols: number; rows: number; }
interface PingMsg { type: "ping"; }
type ClientMsg = OpenMsg | DataMsg | ResizeMsg | PingMsg;

function send(ws: WebSocket, msg: Record<string, unknown>): void {
  if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function fail(ws: WebSocket, message: string): void {
  send(ws, { type: "data", data: toOut(message) });
  send(ws, { type: "exit", code: 1 });
  try { ws.close(); } catch { /* already closed */ }
}

// Spawn the line-mode shell (child_process pipes, NOT a PTY — contract §0.5).
// Returns the child on success; returns null after closing the socket on error.
function openShell(
  ws: WebSocket,
  msg: OpenMsg,
  outDec: StringDecoder,
  errDec: StringDecoder,
): ChildProcessWithoutNullStreams | null {
  let workspace: string;
  try {
    // The terminal always runs in the configured workspace. A client-provided
    // `workspace` root is intentionally NOT honoured — it would let an
    // authenticated client point the shell at an arbitrary directory. Only the
    // `cwd` (workspace-relative, confined below) is respected.
    workspace = getBridge().getDefaultWorkspace();
  } catch (err) {
    fail(ws, `error: no workspace configured: ${String(err)}\n`);
    return null;
  }

  let cwd = workspace;
  if (msg.cwd) {
    try {
      cwd = confinePath(workspace, msg.cwd);
    } catch {
      fail(ws, "error: cwd outside workspace\n");
      return null;
    }
  }

  const cols = msg.cols && msg.cols > 0 ? msg.cols : 80;
  const rows = msg.rows && msg.rows > 0 ? msg.rows : 24;

  const shell =
    process.env.SHELL ||
    (process.platform === "win32" ? process.env.COMSPEC || "cmd.exe" : "/bin/sh");
  const env = {
    ...process.env,
    TERM: "xterm-256color",
    COLUMNS: String(cols),
    LINES: String(rows),
  };

  let child: ChildProcessWithoutNullStreams;
  try {
    child = spawn(shell, [], { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  } catch (err) {
    fail(ws, `error: failed to spawn shell: ${String(err)}\n`);
    return null;
  }

  child.stdout.on("data", (chunk: Buffer) => {
    const s = outDec.write(chunk);
    if (s) send(ws, { type: "data", data: toOut(s) });
  });
  child.stderr.on("data", (chunk: Buffer) => {
    const s = errDec.write(chunk);
    if (s) send(ws, { type: "data", data: toOut(s) });
  });
  child.stdin.on("error", () => {}); // swallow EPIPE on client disconnect
  child.stdout.on("error", () => {});
  child.stderr.on("error", () => {});

  child.on("exit", (code, signal) => {
    const tail = outDec.end() + errDec.end();
    if (tail) send(ws, { type: "data", data: toOut(tail) });
    send(ws, { type: "exit", code: code ?? (signal ? 128 : 0) });
    try { ws.close(); } catch { /* already closed */ }
  });

  send(ws, {
    type: "data",
    data: toOut(
      "Catalyst Code terminal — line mode (no PTY: no vim/nano/TUI apps).\n" +
        `cwd: ${cwd}\n`,
    ),
  });
  return child;
}

// Per-connection handler. The first message MUST be {type:"open"}; afterwards
// data/resize/ping are wired to the shell.
function handleTerminal(ws: WebSocket): void {
  let child: ChildProcessWithoutNullStreams | null = null;
  const outDec = new StringDecoder("utf8");
  const errDec = new StringDecoder("utf8");

  ws.on("message", (raw: RawData) => {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(rawToString(raw)) as ClientMsg;
    } catch {
      return; // ignore malformed envelopes
    }

    if (!child) {
      if (msg.type !== "open") {
        fail(ws, 'error: expected {type:"open"} as first message\n');
        return;
      }
      child = openShell(ws, msg, outDec, errDec);
      return;
    }

    switch (msg.type) {
      case "data":
        // local echo (no PTY) so typed input is visible
        send(ws, { type: "data", data: toEcho(msg.data) });
        // shells expect LF; xterm sends CR on Enter
        if (!child.stdin.destroyed && child.stdin.writable) {
          child.stdin.write(msg.data.replace(/\r/g, "\n"));
        }
        break;
      case "resize":
        // No PTY → cannot signal the running shell. Best-effort only (§4.5).
        break;
      case "ping":
        send(ws, { type: "pong" });
        break;
      default:
        break;
    }
  });

  const cleanup = (): void => {
    if (!child) return;
    try { child.kill("SIGTERM"); } catch { /* already dead */ }
    // escalate if it refuses to die
    const t = setTimeout(() => {
      try { child?.kill("SIGKILL"); } catch { /* already dead */ }
    }, 1500);
    t.unref?.();
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
  getSession(toHeaders(req.headers))
    .then((session) => {
      if (!session) {
        socket.write("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
        socket.destroy();
        return;
      }
      wss.handleUpgrade(req, socket, head, (ws) => handleTerminal(ws));
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
