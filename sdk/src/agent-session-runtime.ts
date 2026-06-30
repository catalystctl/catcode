// AgentSessionRuntime — mirrors `pi-coding-agent`'s
// `core/agent-session-runtime.ts`. Wraps a session + services + a recreate
// factory so session replacement (`newSession`/`switchSession`/`fork`) can rebind.
//
// NOTE: the umans-harness core reuses ONE process across session switches (it
// repoints to a different session file via `load_session`/`new_session`),
// whereas PI creates a new AgentSession per replacement. The runtime still
// calls `rebindSession`/`beforeSessionInvalidate`/`withSession` so pi-web's pool
// rekeying and `session_loaded` notification work unchanged.

import type {
  AgentSessionServices,
  AgentSessionRuntimeDiagnostic,
  CreateAgentSessionRuntimeResult,
} from "./agent-session-services.js";
import type { AgentSession } from "./agent-session.js";
import type { SessionManager } from "./session-manager.js";

export interface CreateAgentSessionRuntimeFactory {
  (options: {
    cwd: string;
    agentDir: string;
    sessionManager: SessionManager;
    sessionStartEvent?: any;
  }): Promise<CreateAgentSessionRuntimeResult>;
}

export interface ReplacedSessionContext {
  sessionPath: string;
}

type RebindFn = (session: AgentSession) => Promise<void> | void;

type SessionReplaceOptions = {
  withSession?: (ctx: ReplacedSessionContext) => Promise<void> | void;
};

export class SessionImportFileNotFoundError extends Error {
  readonly filePath: string;
  constructor(filePath: string) {
    super(`Session file not found: ${filePath}`);
    this.filePath = filePath;
  }
}

export class MissingSessionCwdError extends Error {
  constructor() {
    super("Cannot import a session without a cwd");
  }
}

export class AgentSessionRuntime {
  private _session: AgentSession;
  private _services: AgentSessionServices;
  private _createRuntime: CreateAgentSessionRuntimeFactory;
  private _diagnostics: readonly AgentSessionRuntimeDiagnostic[];
  private _modelFallbackMessage?: string;
  private rebindSession?: RebindFn;
  private beforeSessionInvalidate?: () => void;

  constructor(
    session: AgentSession,
    services: AgentSessionServices,
    createRuntime: CreateAgentSessionRuntimeFactory,
    diagnostics: AgentSessionRuntimeDiagnostic[] = [],
    modelFallbackMessage?: string,
  ) {
    this._session = session;
    this._services = services;
    this._createRuntime = createRuntime;
    this._diagnostics = diagnostics;
    this._modelFallbackMessage = modelFallbackMessage;
  }

  get services(): AgentSessionServices {
    return this._services;
  }
  get session(): AgentSession {
    return this._session;
  }
  get cwd(): string {
    return this._services.cwd;
  }
  get diagnostics(): readonly AgentSessionRuntimeDiagnostic[] {
    return this._diagnostics;
  }
  get modelFallbackMessage(): string | undefined {
    return this._modelFallbackMessage;
  }

  setRebindSession(rebindSession?: RebindFn): void {
    this.rebindSession = rebindSession;
  }
  setBeforeSessionInvalidate(beforeSessionInvalidate?: () => void): void {
    this.beforeSessionInvalidate = beforeSessionInvalidate;
  }

  async switchSession(sessionPath: string, options?: SessionReplaceOptions): Promise<{ cancelled: boolean }> {
    this.beforeSessionInvalidate?.();
    await this._session._switchSession(sessionPath);
    await this.rebindSession?.(this._session);
    await options?.withSession?.({ sessionPath: this._session.sessionFile ?? sessionPath });
    return { cancelled: false };
  }

  async newSession(options?: {
    parentSession?: string;
    setup?: (sm: SessionManager) => Promise<void>;
    withSession?: (ctx: ReplacedSessionContext) => Promise<void> | void;
  }): Promise<{ cancelled: boolean }> {
    this.beforeSessionInvalidate?.();
    if (options?.setup) await options.setup(this._session.sessionManager);
    await this._session._newSession();
    await this.rebindSession?.(this._session);
    await options?.withSession?.({ sessionPath: this._session.sessionFile ?? "" });
    return { cancelled: false };
  }

  async fork(
    entryId: string,
    options?: { position?: "before" | "at"; withSession?: (ctx: ReplacedSessionContext) => Promise<void> | void },
  ): Promise<{ cancelled: boolean; selectedText?: string }> {
    // The harness has no branch tree; fork creates a fresh session. `entryId`
    // and `selectedText` are not meaningful here (documented limitation).
    this.beforeSessionInvalidate?.();
    await this._session._forkSession();
    await this.rebindSession?.(this._session);
    await options?.withSession?.({ sessionPath: this._session.sessionFile ?? "" });
    return { cancelled: false, selectedText: undefined };
  }

  async importFromJsonl(inputPath: string, cwdOverride?: string): Promise<{ cancelled: boolean }> {
    if (cwdOverride) this._services.cwd = cwdOverride;
    // Best-effort: switch the core to the imported file.
    try {
      await this._session._switchSession(inputPath);
    } catch {
      throw new SessionImportFileNotFoundError(inputPath);
    }
    await this.rebindSession?.(this._session);
    return { cancelled: false };
  }

  async dispose(): Promise<void> {
    this._session.dispose();
  }
}

/** Build a runtime from a recreate factory. pi-web's `buildRuntimeFactory` is the
 *  factory; this invokes it (spawning the core) and wraps the result. */
export async function createAgentSessionRuntime(
  createRuntime: CreateAgentSessionRuntimeFactory,
  options: { cwd: string; agentDir: string; sessionManager: SessionManager; sessionStartEvent?: any },
): Promise<AgentSessionRuntime> {
 const result = await createRuntime(options);
  return new AgentSessionRuntime(
    result.session,
    result.services,
    createRuntime,
    [...result.diagnostics],
    result.modelFallbackMessage,
  );
}
