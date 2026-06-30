// CoreProcess — spawns the umans-harness core binary and speaks its JSONL
// stdio protocol (commands → stdin, events ← stdout). Mirrors the Go TUI's
// `startCore` plumbing (`tui/main.go`): binary resolution, pipe wiring, the
// `init` handshake, and back-pressured line-delimited JSON I/O.
//
// This is the ONLY module that touches `child_process`. Everything above it
// (AgentSession, factories) consumes a typed event stream and sends typed
// commands, so the agent loop itself is never reimplemented here.

import { spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { openSync, closeSync } from "node:fs";
import { join } from "node:path";
import { resolveCoreBinary, configDir, ensureDir } from "./config.js";

export interface CoreProcessOptions {
  cwd: string;
  sessionFile?: string;
  approval?: "never" | "destructive" | "always";
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKey?: string;
  debugLog?: string;
  sandbox?: "none" | "firejail";
  noNetwork?: boolean;
  maxSessionTokens?: number;
  idleTimeout?: number;
  trustProjectPlugins?: boolean;
  /** Extra env for the core process. */
  env?: Record<string, string>;
  /** Override the binary path (skips resolution). */
  binaryPath?: string;
  /** stdio forwarding for tests/debug. */
  silent?: boolean;
}

export interface ReadyPayload {
  models: any[];
  authed: boolean;
  workspace: string;
  approval: string;
  base_url: string;
  provider: string;
  providerKind: string;
  providers: string[];
  bash_timeout_secs: number;
  resumed_messages: number;
}

export type CoreEvent = Record<string, any> & { type: string };

export interface PendingRequest {
  matchType: string | string[];
  resolve: (ev: CoreEvent) => void;
  reject: (err: Error) => void;
  timer: NodeJS.Timeout | null;
}

/** One-line command shape sent to the core (`{"type": "...", ...}`). */
export type CoreCommand = Record<string, any> & { type: string };

export class CoreProcess {
  readonly cwd: string;
  readonly options: CoreProcessOptions;
  private proc: ChildProcess | null = null;
  private stdin: NodeJS.WritableStream | null = null;
  private listeners = new Set<(ev: CoreEvent) => void>();
  private pending: PendingRequest[] = [];
  private readyPromise: Promise<ReadyPayload> | null = null;
  private disposed = false;
  private debugFd: number | null = null;

  constructor(options: CoreProcessOptions) {
    this.cwd = options.cwd;
    this.options = options;
  }

  /** Spawn the core, send `init`, and resolve once `ready` arrives. */
  start(): Promise<ReadyPayload> {
    if (this.readyPromise) return this.readyPromise;
    this.readyPromise = new Promise<ReadyPayload>((resolve, reject) => {
      const binary = this.options.binaryPath ?? resolveCoreBinary({ cwd: this.cwd });
      const args = this.buildArgs();
      const env = { ...process.env, ...this.options.env };

      // Debug log: append-mode JSONL (matches TUI's configDir()/debug.jsonl).
      const debugLog = this.options.debugLog ?? join(configDir(), "debug.jsonl");
      try {
        ensureDir(configDir());
        this.debugFd = openSync(debugLog, "a");
      } catch {
        this.debugFd = null;
      }

      try {
        this.proc = spawn(binary, args, {
          cwd: this.cwd,
          env,
          stdio: ["pipe", "pipe", this.debugFd ?? "pipe"],
        });
      } catch (err: any) {
        reject(new Error(`Failed to spawn umans-core (${binary}): ${err?.message ?? err}`));
        return;
      }

      if (!this.proc.stdin || !this.proc.stdout) {
        reject(new Error("umans-core stdio is not piped"));
        return;
      }
      this.stdin = this.proc.stdin;

      // Route stderr (if piped) to the debug log; surface fatal exit.
      if (this.debugFd === null && this.proc.stderr) {
        this.proc.stderr.on("data", () => {
          /* discarded — keeps the protocol channel clean */
        });
      }

      // Line-delimited JSON reader with backpressure (buffered channel).
      const rl = createInterface({ input: this.proc.stdout, crlfDelay: Infinity });
      rl.on("line", (line) => this.handleLine(line, resolve, reject));

      this.proc.on("error", (err) => {
        if (!this.disposed) reject(new Error(`umans-core error: ${err.message}`));
      });
      this.proc.on("exit", (code, signal) => {
        this.cleanup();
        // Reject any still-pending requests.
        const err = new Error(`umans-core exited (code=${code} signal=${signal})`);
        for (const p of this.pending) {
          if (p.timer) clearTimeout(p.timer);
          p.reject(err);
        }
        this.pending = [];
      });

      // Send the handshake.
      this.send({ type: "init" });
    });
    return this.readyPromise;
  }

  private buildArgs(): string[] {
    const o = this.options;
    const args: string[] = ["--workspace", o.cwd];
    args.push("--approval", o.approval ?? "destructive");
    if (o.sessionFile) args.push("--session", o.sessionFile);
    const debugLog = o.debugLog ?? join(configDir(), "debug.jsonl");
    args.push("--debug-log", debugLog);
    if (typeof o.idleTimeout === "number") args.push("--idle-timeout", String(o.idleTimeout));
    if (o.provider) args.push("--provider", o.provider);
    if (o.baseUrl) args.push("--base-url", o.baseUrl);
    if (o.noNetwork) args.push("--no-network");
    if (o.sandbox && o.sandbox !== "none") args.push("--sandbox", o.sandbox);
    if (typeof o.maxSessionTokens === "number" && o.maxSessionTokens > 0) {
      args.push("--max-session-tokens", String(o.maxSessionTokens));
    }
    if (o.trustProjectPlugins) args.push("--trust-project-plugins");
    return args;
  }

  private handleLine(line: string, resolveReady: (r: ReadyPayload) => void, rejectReady: (e: Error) => void): void {
    const trimmed = line.trim();
    if (!trimmed) return;
    let ev: CoreEvent;
    try {
      ev = JSON.parse(trimmed);
    } catch {
      return; // ignore non-JSON lines
    }
    if (typeof ev.type !== "string") return;

    if (ev.type === "ready") {
      resolveReady(ev as unknown as ReadyPayload);
      return;
    }

    // Dispatch to one-shot request waiters first (oldest match wins).
    const idx = this.pending.findIndex((p) => matchesType(ev.type, p.matchType));
    if (idx >= 0) {
      const [req] = this.pending.splice(idx, 1);
      if (req.timer) clearTimeout(req.timer);
      req.resolve(ev);
    }

    // Then broadcast to stream listeners.
    for (const fn of this.listeners) {
      try {
        fn(ev);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  }

  /** Write a command (newline-delimited JSON) to the core's stdin. */
  send(command: CoreCommand): void {
    if (!this.stdin || this.disposed) {
      throw new Error("umans-core is not running (no stdin)");
    }
    this.stdin.write(JSON.stringify(command) + "\n");
  }

  /** Send a command and resolve with the first matching event. */
  request<T extends CoreEvent = CoreEvent>(
    command: CoreCommand,
    matchType: string | string[],
    timeoutMs = 30000,
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const req: PendingRequest = {
        matchType,
        resolve: resolve as (ev: CoreEvent) => void,
        reject,
        timer: null,
      };
      req.timer = setTimeout(() => {
        const i = this.pending.indexOf(req);
        if (i >= 0) this.pending.splice(i, 1);
        reject(new Error(`Timed out waiting for ${JSON.stringify(matchType)}`));
      }, timeoutMs);
      this.pending.push(req);
      try {
        this.send(command);
      } catch (err: any) {
        if (req.timer) clearTimeout(req.timer);
        this.pending.splice(this.pending.indexOf(req), 1);
        reject(err);
      }
    });
  }

  /** Subscribe to the event stream. Returns an unsubscribe function. */
  on(fn: (ev: CoreEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** A promise that resolves when the core has emitted `ready`. */
  get ready(): Promise<ReadyPayload> {
    if (!this.readyPromise) {
      this.readyPromise = this.start();
    }
    return this.readyPromise;
  }

  get isRunning(): boolean {
    return !!this.proc && !this.disposed && this.proc.exitCode === null;
  }

  /** Kill the core process and release resources. */
  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    // Try a clean shutdown by closing stdin; the core exits on EOF.
    try {
      if (this.stdin) this.stdin.end();
    } catch {
      /* ignore */
    }
    if (this.proc) {
      const proc = this.proc;
      await new Promise<void>((resolve) => {
        const t = setTimeout(() => {
          try {
            proc.kill("SIGKILL");
          } catch {
            /* ignore */
          }
          resolve();
        }, 3000);
        proc.once("exit", () => {
          clearTimeout(t);
          resolve();
        });
      });
    }
    this.cleanup();
  }

  private cleanup(): void {
    if (this.debugFd !== null) {
      try {
        closeSync(this.debugFd);
      } catch {
        /* ignore */
      }
      this.debugFd = null;
    }
  }
}

function matchesType(type: string, matchType: string | string[]): boolean {
  return Array.isArray(matchType) ? matchType.includes(type) : type === matchType;
}
