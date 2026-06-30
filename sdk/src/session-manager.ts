// SessionManager — mirrors `pi-coding-agent`'s `core/session-manager.ts`.
//
// The umans-harness core owns session persistence (a flat JSONL file of
// OpenAI-style messages). This manager is a thin handle over the session file
// PATH: it resolves/creates the path and forwards structural operations
// (`load_session`/`new_session`) to the core via the bound AgentSession.
//
// PI's session model is a branching entry tree (ids/parentId). The harness has
// a flat history instead, so the tree methods return minimal/synthetic results
// (documented per-method). `fork` in `AgentSessionRuntime` creates a new
// session rather than a true branch — see its doc comment.

import { join } from "node:path";
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import {
  ensureDir,
  newSessionFilename,
  sessionsDirFor,
} from "./config.js";
import type { AgentMessage, TextContent, ImageContent } from "./types.js";

export const CURRENT_SESSION_VERSION = 3;

export interface SessionHeader {
  type: "session";
  version?: number;
  id: string;
  timestamp: string;
  cwd: string;
  parentSession?: string;
}

export interface NewSessionOptions {
  id?: string;
  parentSession?: string;
}

export interface SessionEntryBase {
  type: string;
  id: string;
  parentId: string | null;
  timestamp: string;
}
export interface SessionMessageEntry extends SessionEntryBase {
  type: "message";
  message: AgentMessage;
}
export interface ThinkingLevelChangeEntry extends SessionEntryBase {
  type: "thinking_level_change";
  thinkingLevel: string;
}
export interface ModelChangeEntry extends SessionEntryBase {
  type: "model_change";
  provider: string;
  modelId: string;
}
export interface CompactionEntry<T = unknown> extends SessionEntryBase {
  type: "compaction";
  summary: string;
  firstKeptEntryId: string;
  tokensBefore: number;
  details?: T;
  fromHook?: boolean;
}
export interface BranchSummaryEntry<T = unknown> extends SessionEntryBase {
  type: "branch_summary";
  fromId: string;
  summary: string;
  details?: T;
  fromHook?: boolean;
}
export interface CustomEntry<T = unknown> extends SessionEntryBase {
  type: "custom";
  customType: string;
  data?: T;
}
export interface LabelEntry extends SessionEntryBase {
  type: "label";
  targetId: string;
  label: string | undefined;
}
export interface SessionInfoEntry extends SessionEntryBase {
  type: "session_info";
  name?: string;
}
export interface CustomMessageEntry<T = unknown> extends SessionEntryBase {
  type: "custom_message";
  customType: string;
  content: string | (TextContent | ImageContent)[];
  details?: T;
  display: boolean;
}
export type SessionEntry =
  | SessionMessageEntry
  | ThinkingLevelChangeEntry
  | ModelChangeEntry
  | CompactionEntry
  | BranchSummaryEntry
  | CustomEntry
  | CustomMessageEntry
  | LabelEntry
  | SessionInfoEntry;
export type FileEntry = SessionHeader | SessionEntry;

export interface SessionContext {
  messages: AgentMessage[];
  thinkingLevel: string;
  model: { provider: string; modelId: string } | null;
}

export interface SessionInfo {
  path: string;
  id: string;
  cwd: string;
  name?: string;
  parentSessionPath?: string;
  created: string;
  modified: string;
  messageCount: number;
  firstMessage: string;
  allMessagesText: string;
}

export type SessionListProgress = (loaded: number, total: number) => void;

export class SessionManager {
  private cwd: string;
  private sessionDir: string;
  private sessionFile: string | undefined;
  private sessionId: string;
  private sessionName: string | undefined;
  private persisted: boolean;

  private constructor(cwd: string, sessionDir: string, sessionFile?: string, persisted = true) {
    this.cwd = cwd;
    this.sessionDir = sessionDir;
    this.sessionFile = sessionFile;
    this.persisted = persisted;
    this.sessionId = sessionFile ? deriveId(sessionFile) : newId();
  }

  /** Create a fresh session for a workspace (most-recent resume handled by caller). */
  static create(cwd: string, sessionDir?: string): SessionManager {
    const dir = sessionDir ?? sessionsDirFor(cwd);
    ensureDir(dir);
    return new SessionManager(cwd, dir, join(dir, newSessionFilename()));
  }

  /** Open an existing session file by path (relative to the sessions dir). */
  static open(path: string, sessionDir?: string, cwdOverride?: string): SessionManager {
    const cwd = cwdOverride ?? process.cwd();
    const dir = sessionDir ?? sessionsDirFor(cwd);
    const abs = existsSync(path) ? path : join(dir, path);
    return new SessionManager(cwd, dir, abs);
  }

  static continueRecent(cwd: string, sessionDir?: string): SessionManager {
    const dir = sessionDir ?? sessionsDirFor(cwd);
    ensureDir(dir);
    const recent = mostRecentSession(dir);
    return recent ? SessionManager.open(recent, dir, cwd) : SessionManager.create(cwd, dir);
  }

  static inMemory(cwd?: string): SessionManager {
    const c = cwd ?? process.cwd();
    const sm = new SessionManager(c, "", undefined, false);
    return sm;
  }

  static forkFrom(sourcePath: string, targetCwd: string, sessionDir?: string): SessionManager {
    const dir = sessionDir ?? sessionsDirFor(targetCwd);
    ensureDir(dir);
    const target = join(dir, newSessionFilename());
    if (existsSync(sourcePath)) writeFileSync(target, readFileSync(sourcePath));
    return new SessionManager(targetCwd, dir, target);
  }

  static async list(cwd: string, sessionDir?: string, onProgress?: SessionListProgress): Promise<SessionInfo[]> {
    const dir = sessionDir ?? sessionsDirFor(cwd);
    return listSessionsIn(dir, onProgress);
  }

  static async listAll(onProgress?: SessionListProgress): Promise<SessionInfo[]> {
    // Best-effort across the sessions root.
    return listSessionsIn(join(sessionsDirFor(process.cwd()), ".."), onProgress);
  }

  setSessionFile(sessionFile: string): void {
    this.sessionFile = sessionFile;
    this.sessionId = deriveId(sessionFile);
    this.persisted = true;
  }

  newSession(options?: NewSessionOptions): string | undefined {
    const dir = this.sessionDir || sessionsDirFor(this.cwd);
    ensureDir(dir);
    const file = options?.id ? join(dir, `${options.id}.jsonl`) : join(dir, newSessionFilename());
    this.sessionFile = file;
    this.sessionId = options?.id ?? deriveId(file);
    return this.sessionId;
  }

  isPersisted(): boolean {
    return this.persisted;
  }
  getCwd(): string {
    return this.cwd;
  }
  getSessionDir(): string {
    return this.sessionDir;
  }
  getSessionId(): string {
    return this.sessionId;
  }
  getSessionFile(): string | undefined {
    return this.sessionFile;
  }

  // ── append (return synthetic entry ids) ──
  appendMessage(message: any): string {
    return appendEntry(this, { type: "message", message });
  }
  appendThinkingLevelChange(thinkingLevel: string): string {
    return appendEntry(this, { type: "thinking_level_change", thinkingLevel });
  }
  appendModelChange(provider: string, modelId: string): string {
    return appendEntry(this, { type: "model_change", provider, modelId });
  }
  appendCompaction<T = unknown>(
    summary: string,
    firstKeptEntryId: string,
    tokensBefore: number,
    details?: T,
    fromHook?: boolean,
  ): string {
    return appendEntry(this, { type: "compaction", summary, firstKeptEntryId, tokensBefore, details, fromHook });
  }
  appendCustomEntry(customType: string, data?: unknown): string {
    return appendEntry(this, { type: "custom", customType, data });
  }
  appendSessionInfo(name: string): string {
    this.sessionName = name;
    return appendEntry(this, { type: "session_info", name });
  }
  appendCustomMessageEntry<T = unknown>(
    customType: string,
    content: string | (TextContent | ImageContent)[],
    display: boolean,
    details?: T,
  ): string {
    return appendEntry(this, { type: "custom_message", customType, content, display, details });
  }
  appendLabelChange(targetId: string, label: string | undefined): string {
    return appendEntry(this, { type: "label", targetId, label });
  }

  // ── read ──
  getSessionName(): string | undefined {
    return this.sessionName;
  }
  /** Synthetic: the harness has no branch tree. Returns the latest entry id. */
  getLeafId(): string | null {
    const entries = this.getEntries();
    return entries.length ? entries[entries.length - 1].id : null;
  }
  getLeafEntry(): SessionEntry | undefined {
    const entries = this.getEntries();
    return entries.length ? entries[entries.length - 1] : undefined;
  }
  getEntry(id: string): SessionEntry | undefined {
    return this.getEntries().find((e) => e.id === id);
  }
  getChildren(_parentId: string): SessionEntry[] {
    return []; // flat history — no children.
  }
  getLabel(_id: string): string | undefined {
    return undefined;
  }
  getBranch(_fromId?: string): SessionEntry[] {
    return this.getEntries();
  }
  buildSessionContext(): SessionContext {
    return { messages: [], thinkingLevel: "medium", model: null };
  }
  getHeader(): SessionHeader | null {
    return null;
  }
  getEntries(): SessionEntry[] {
    if (!this.sessionFile || !existsSync(this.sessionFile)) return [];
    try {
      const lines = readFileSync(this.sessionFile, "utf8").split("\n").filter(Boolean);
      return lines
        .map((l) => safeParse(l))
        .filter((e): e is SessionEntry => !!e && typeof e === "object" && "type" in e);
    } catch {
      return [];
    }
  }
  getTree(): any[] {
    return this.getEntries().map((entry) => ({ entry, children: [] }));
  }

  // ── branching (no-ops for the flat-history harness) ──
  branch(_branchFromId: string): void {}
  resetLeaf(): void {}
  branchWithSummary(branchFromId: string | null, summary: string, details?: unknown, fromHook?: boolean): string {
    return appendEntry(this, { type: "branch_summary", fromId: branchFromId ?? "", summary, details, fromHook });
  }
  createBranchedSession(leafId: string): string | undefined {
    // Clone semantics: copy the current file to a new session file.
    if (!this.sessionFile || !existsSync(this.sessionFile)) return undefined;
    const dir = this.sessionDir || sessionsDirFor(this.cwd);
    ensureDir(dir);
    const target = join(dir, newSessionFilename());
    writeFileSync(target, readFileSync(this.sessionFile));
    return target;
  }
}

// ── helpers ──

function deriveId(file: string): string {
  const base = file.split(/[\\/]/).pop() ?? file;
  return base.replace(/\.jsonl$/i, "");
}

function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function appendEntry(sm: SessionManager, entry: Record<string, any>): string {
  const id = newId();
  const full: SessionEntry = {
    id,
    parentId: null,
    timestamp: new Date().toISOString(),
    ...entry,
  } as unknown as SessionEntry;
  const file = sm.getSessionFile();
  if (file && sm.isPersisted()) {
    try {
      mkdirSync(sm.getSessionDir() || join(file, ".."), { recursive: true });
      writeFileSync(file, JSON.stringify(full) + "\n", { flag: "a" });
    } catch {
      /* best-effort */
    }
  }
  return id;
}

function safeParse(line: string): any {
  try {
    return JSON.parse(line);
  } catch {
    return undefined;
  }
}

function mostRecentSession(dir: string): string | undefined {
  if (!existsSync(dir)) return undefined;
  let best: string | undefined;
  let bestMtime = 0;
  for (const name of readdirSync(dir)) {
    if (!name.endsWith(".jsonl")) continue;
    const p = join(dir, name);
    try {
      const mt = statSync(p).mtimeMs;
      if (mt > bestMtime) {
        bestMtime = mt;
        best = p;
      }
    } catch {
      /* skip */
    }
  }
  return best;
}

function listSessionsIn(dir: string, _onProgress?: SessionListProgress): SessionInfo[] {
  if (!existsSync(dir)) return [];
  const out: SessionInfo[] = [];
  for (const name of readdirSync(dir)) {
    if (!name.endsWith(".jsonl")) continue;
    const p = join(dir, name);
    try {
      const st = statSync(p);
      const lines = readFileSync(p, "utf8").split("\n").filter(Boolean);
      const first = lines.length ? safeParse(lines[0]) : undefined;
      const firstText = first?.message?.content ?? first?.content ?? "";
      out.push({
        path: p,
        id: deriveId(p),
        cwd: dir,
        created: st.birthtime.toISOString(),
        modified: st.mtime.toISOString(),
        messageCount: lines.length,
        firstMessage: typeof firstText === "string" ? firstText : "",
        allMessagesText: "",
      });
    } catch {
      /* skip unreadable */
    }
  }
  return out.sort((a, b) => (a.modified < b.modified ? 1 : -1));
}
