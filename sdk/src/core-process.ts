// CoreProcess — spawns the catalyst-code core binary and speaks its JSONL
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
import type { CoreEvent } from "./core-events.js";
import {
  normalizeSandboxMode,
  type SandboxConfig,
  type SandboxMode,
  type SandboxOption,
  type SandboxStatus,
  type SandboxReady,
  type SandboxPreflightReport,
  type SandboxPreflightCheck,
  type SandboxPreflightCheckStatus,
  type SandboxSetupAction,
} from "./sandbox.js";
import { VERSION } from "./version.js";

export type {
  SandboxMode,
  SandboxOption,
  SandboxLegacyAlias,
  SandboxNetworkMode,
  SandboxConfig,
  SandboxPreflightCheck,
  SandboxPreflightCheckStatus,
  SandboxPreflightReport,
  SandboxSetupAction,
  SandboxStatus,
  SandboxReady,
  SandboxError,
} from "./sandbox.js";

export type { CoreEvent } from "./core-events.js";
export {
  CORE_EVENT_TYPES,
  isKnownCoreEventType,
  type CoreEventType,
  type NarrowCoreEvent,
  type ProtocolHelloEvent,
  type FileChangeEvent,
  type CheckpointCreatedEvent,
  type CheckpointRestoredEvent,
  type CheckpointsEvent,
  type WorktreeReadyEvent,
  type WorktreeCleanedEvent,
  type WorktreePromotedEvent,
  type AuditEvent,
  type CostUpdateEvent,
  type GoalStepVerdictEvent,
  type GoalStateEvent,
  type GoalPlanEvent,
  type GoalPhaseEvent,
  type SubagentStartEvent,
  type SubagentDoneEvent,
  type SubagentProgressEvent,
  type ApprovalRequestEvent,
  type AskRequestEvent,
  type SudoRequestEvent,
  type MetricsEvent,
  type SandboxStatusEvent,
  type SandboxPrepareProgressEvent,
  type SandboxReadyEvent,
  type SandboxErrorEvent,
} from "./core-events.js";

export interface CoreProcessOptions {
  cwd: string;
  sessionFile?: string;
  approval?: "never" | "destructive" | "always";
  provider?: string;
  model?: string;
  baseUrl?: string;
  apiKey?: string;
  debugLog?: string;
  /**
   * Sandbox mode for agent-controlled workloads. Operational values are
   * `"none"` (run on the host) and `"microsandbox"` (run inside a
   * Microsandbox microVM). Legacy spellings (firejail/fj/seatbelt/macos/
   * sandbox-exec) are accepted for source compatibility and normalized to
   * `"microsandbox"` with a deprecation notice — they are never emitted to
   * the core. Undefined omits `--sandbox` (core default).
   */
  sandbox?: SandboxOption;
  /**
   * Backward-compat: deny all guest network. Implemented by the core through
   * Microsandbox network policy (`sandbox-network=none`), not `unshare`.
   */
  noNetwork?: boolean;
  /**
   * Typed sandbox resource/network/image configuration. Fields with a core
   * CLI flag are emitted (`--sandbox-image`, `--sandbox-cpus`,
   * `--sandbox-memory-mb`, `--sandbox-disk-mb`, `--sandbox-network`,
   * `--sandbox-env-allowlist`). Fields without a CLI flag (networkAllowlist,
   * allowPrivateNetworks, idleTimeoutSecs) are documented in {@link SandboxConfig}
   * and must be set via the core config file / `CATALYST_CODE_*` env.
   */
  sandboxConfig?: SandboxConfig;
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
  // ── Sandbox status (Microsandbox migration) ──
  /** Effective sandbox mode: "none" | "microsandbox". */
  sandbox?: SandboxMode | string;
  /** Effective shell kind emitted by the core: "bash" | "powershell". */
  shell?: string;
  /** Active sandbox OCI image reference (when sandboxed). */
  sandboxImage?: string;
  /** Guest vCPU limit. */
  sandboxCpus?: number;
  /** Guest memory limit in MiB. */
  sandboxMemoryMb?: number;
  /** Guest network egress policy: "none" | "restricted" | "allowlist". */
  sandboxNetworkMode?: string;
  /** Whether the sandbox is ready to execute commands right now. */
  sandboxReady?: boolean;
}

export interface PendingRequest {
  matchType: string | string[];
  /** Optional event type(s) that should REJECT this request (e.g. an error event). */
  errorMatchType?: string | string[];
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
        reject(new Error(`Failed to spawn catcode-core (${binary}): ${err?.message ?? err}`));
        return;
      }

      if (!this.proc.stdin || !this.proc.stdout) {
        reject(new Error("catcode-core stdio is not piped"));
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
        if (!this.disposed) reject(new Error(`catcode-core error: ${err.message}`));
      });
      this.proc.on("exit", (code, signal) => {
        this.cleanup();
        // Reject any still-pending requests.
        const err = new Error(`catcode-core exited (code=${code} signal=${signal})`);
        for (const p of this.pending) {
          if (p.timer) clearTimeout(p.timer);
          p.reject(err);
        }
        this.pending = [];
      });

      // Send the handshake.
      this.send({
        type: "init",
        protocol_version: 2,
        client: {
          name: "catcode-sdk",
          version: VERSION,
          capabilities: ["run_ids", "session_ids", "event_sequence"],
        },
      });
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
    // Sandbox mode: normalize legacy aliases (firejail/seatbelt/…) to
    // "microsandbox" with a deprecation notice. The SDK NEVER emits a legacy
    // value to the core — only "none" or "microsandbox" reach the wire.
    const sandboxMode = normalizeSandboxMode(o.sandbox);
    if (sandboxMode && sandboxMode !== "none") args.push("--sandbox", sandboxMode);
    // Typed sandbox config → recognized core CLI flags. Fields without a CLI
    // flag (networkAllowlist, allowPrivateNetworks, idleTimeoutSecs) are
    // intentionally NOT emitted here; see SandboxConfig docs.
    const sc = o.sandboxConfig;
    if (sc) {
      if (sc.image) args.push("--sandbox-image", sc.image);
      if (typeof sc.cpus === "number") args.push("--sandbox-cpus", String(sc.cpus));
      if (typeof sc.memoryMb === "number") args.push("--sandbox-memory-mb", String(sc.memoryMb));
      if (typeof sc.diskMb === "number") args.push("--sandbox-disk-mb", String(sc.diskMb));
      if (sc.networkMode) args.push("--sandbox-network", sc.networkMode);
      if (sc.envAllowlist && sc.envAllowlist.length > 0) {
        args.push("--sandbox-env-allowlist", sc.envAllowlist.join(","));
      }
    }
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
    // An error-match type takes precedence and rejects the request.
    const errIdx = this.pending.findIndex(
      (p) => p.errorMatchType !== undefined && matchesType(ev.type, p.errorMatchType),
    );
    if (errIdx >= 0) {
      const [req] = this.pending.splice(errIdx, 1);
      if (req.timer) clearTimeout(req.timer);
      req.reject(new SandboxCommandError(ev));
      // Still fall through to broadcast so stream listeners see the error event.
    }
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
      throw new Error("catcode-core is not running (no stdin)");
    }
    this.stdin.write(JSON.stringify(command) + "\n");
  }

  /** Send a command and resolve with the first matching event. */
  request<T extends CoreEvent = CoreEvent>(
    command: CoreCommand,
    matchType: string | string[],
    timeoutMs = 30000,
    errorMatchType?: string | string[],
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const req: PendingRequest = {
        matchType,
        errorMatchType,
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

  // ── Sandbox lifecycle commands (Microsandbox migration) ──
  // These send the new protocol commands and await the matching terminal
  // events. They NEVER assume that requesting microsandbox mode guarantees
  // activation — callers must inspect the returned report / ready flag.

  /**
   * Request the current sandbox status (mode + preflight report). Sends
   * `get_sandbox_status` and awaits `sandbox_status`.
   */
  async getSandboxStatus(timeoutMs = 30000): Promise<SandboxStatus> {
    const ev = await this.request<CoreEvent>(
      { type: "get_sandbox_status" },
      "sandbox_status",
      timeoutMs,
    );
    return normalizeSandboxStatusEvent(ev);
  }

  /**
   * Prepare the sandbox runtime/image assets (first-use download). Sends
   * `prepare_sandbox` and awaits `sandbox_ready` (success) or `sandbox_error`
   * (failure). Progress events (`sandbox_prepare_progress`) stream to listeners
   * during preparation; pass an `onProgress` callback to observe them.
   */
  async prepareSandbox(
    timeoutMs = 300000,
    onProgress?: (phase: string) => void,
  ): Promise<SandboxReady> {
    let unsub: (() => void) | undefined;
    if (onProgress) {
      unsub = this.on((ev) => {
        if (ev.type === "sandbox_prepare_progress" && typeof ev.phase === "string") {
          try {
            onProgress(ev.phase as string);
          } catch {
            /* progress callback errors are non-fatal */
          }
        }
      });
    }
    try {
      const ev = await this.request<CoreEvent>(
        { type: "prepare_sandbox" },
        "sandbox_ready",
        timeoutMs,
        "sandbox_error",
      );
      return normalizeSandboxReadyEvent(ev);
    } finally {
      unsub?.();
    }
  }

  /**
   * Reset an unhealthy sandbox. Sends `reset_sandbox` and awaits `sandbox_status`
   * (the core emits `sandbox_status` with `reset: true` after resetting).
   */
  async resetSandbox(timeoutMs = 60000): Promise<SandboxStatus> {
    const ev = await this.request<CoreEvent>(
      { type: "reset_sandbox" },
      "sandbox_status",
      timeoutMs,
    );
    return normalizeSandboxStatusEvent(ev);
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

/**
 * Error raised when a sandbox command's terminal `sandbox_error` event arrives
 * (e.g. `prepare_sandbox` failed). Carries the core's human-readable message,
 * which never contains secret values. A setup-required failure surfaces here;
 * callers should treat it as "do not execute on the host" and may re-query via
 * {@link CoreProcess.getSandboxStatus} for the full structured report.
 */
export class SandboxCommandError extends Error {
  /** Raw error event (type === "sandbox_error"). */
  readonly event: CoreEvent;
  constructor(event: CoreEvent) {
    const msg =
      typeof event.error === "string" ? event.error : "sandbox command failed";
    super(msg);
    this.name = "SandboxCommandError";
    this.event = event;
  }
}

/** Coerce a wire `mode` string to the strict {@link SandboxMode} union. */
function coerceSandboxMode(mode: unknown): SandboxMode {
  return mode === "microsandbox" ? "microsandbox" : "none";
}

/** Best-effort coercion of a report-shaped value into the typed interface. */
function coerceReport(raw: unknown): SandboxPreflightReport | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const r = raw as Record<string, unknown>;
  const checks = Array.isArray(r.checks) ? (r.checks as unknown[]).map(coerceCheck) : [];
  const actions = Array.isArray(r.actions) ? (r.actions as unknown[]).map(coerceAction) : [];
  return {
    requested: Boolean(r.requested),
    supported: Boolean(r.supported),
    ready: Boolean(r.ready),
    platform: typeof r.platform === "string" ? r.platform : "",
    architecture: typeof r.architecture === "string" ? r.architecture : "",
    checks,
    actions,
  };
}

function coerceCheck(raw: unknown): SandboxPreflightCheck {
  const c = (raw ?? {}) as Record<string, unknown>;
  const status: SandboxPreflightCheckStatus =
    c.status === "pass" || c.status === "fail" || c.status === "warn" || c.status === "info"
      ? (c.status as SandboxPreflightCheckStatus)
      : "info";
  return {
    code: typeof c.code === "string" ? c.code : "",
    title: typeof c.title === "string" ? c.title : "",
    status,
    detail: typeof c.detail === "string" ? c.detail : "",
  };
}

function coerceAction(raw: unknown): SandboxSetupAction {
  const a = (raw ?? {}) as Record<string, unknown>;
  return {
    title: typeof a.title === "string" ? a.title : "",
    explanation: typeof a.explanation === "string" ? a.explanation : "",
    command: a.command == null ? null : String(a.command),
    requires_admin: Boolean(a.requires_admin),
    requires_reboot: Boolean(a.requires_reboot),
  };
}

/** Normalize a `sandbox_status` event into the typed {@link SandboxStatus}. */
function normalizeSandboxStatusEvent(ev: CoreEvent): SandboxStatus {
  return {
    mode: coerceSandboxMode(ev.mode),
    report: coerceReport(ev.report),
    reset: typeof ev.reset === "boolean" ? (ev.reset as boolean) : undefined,
  };
}

/** Normalize a `sandbox_ready` event into the typed {@link SandboxReady}. */
function normalizeSandboxReadyEvent(ev: CoreEvent): SandboxReady {
  return {
    ready: Boolean(ev.ready),
    report: coerceReport(ev.report),
  };
}
