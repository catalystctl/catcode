// Shared wire + UI types for the umans-harness web frontend.
//
// The core speaks newline-delimited JSON over stdio. The server bridge forwards
// raw core events to the browser over SSE, and accepts raw core commands via
// POST. This file is the single source of truth for those shapes (a typed subset
// of core/src/protocol.rs + the event payloads constructed in main.rs/provider.rs),
// plus the UI message model the reducer assembles from the event stream.

// ─── Core wire types ────────────────────────────────────────────────────────

export interface ModelInfo {
  id: string;
  name: string;
  reasoning: boolean;
  context_window: number;
  max_tokens: number;
  /** Thinking levels the model advertises (e.g. ["low","medium","high"]). */
  thinking_levels: string[];
  vision: boolean;
}

export interface ReadyPayload {
  type: "ready";
  models: ModelInfo[];
  authed: boolean;
  workspace: string;
  approval: string; // "never" | "destructive" | "always"
  base_url: string;
  provider: string;
  providerKind: string;
  providers: string[];
  bash_timeout_secs: number;
  resumed_messages: number;
}

export interface ApprovalRequest {
  request_id: string;
  tool: string;
  args: string;
  diff?: string;
}

export interface Metrics {
  ttft_ms?: number;
  elapsed_ms?: number;
  tokens_in?: number; // mid-stream = live context; final = in+out (use prompt_tokens)
  prompt_tokens?: number; // final: true input
  tokens_out?: number;
  cached_tokens?: number;
  tps?: number;
  model?: string;
}

export interface SessionEntry {
  name: string;
  mtime: number;
  size?: number;
  /** Auto-derived title (first user message) from the core. May be overridden
   *  by the bridge's session-titles overlay (user-defined rename). */
  title?: string;
  /** Absolute path to the .jsonl session file. */
  path?: string;
  /** Message count in the session file. */
  messages?: number;
  /** True when this is the currently-active session. */
  current?: boolean;
}

export interface Stats {
  type: "stats";
  /** Current real context size (matches the footer) — NOT cumulative. */
  tokens_in: number;
  /** Cumulative output tokens (total produced this session). */
  tokens_out: number;
  /** Cumulative prompt tokens (billing total input; drives cache_hit_ratio). */
  total_in?: number;
  /** Cumulative in + out (billing total). */
  tokens_total: number;
  cached_tokens: number;
  /** cached_tokens / total_in — fraction of cumulative prompt that was a cache hit. */
  cache_hit_ratio?: number;
  turns: number;
  messages: number;
  session_file: string;
}

/** A saved memory note (persisted per-workspace, injected into the system prompt). */
export interface MemoryEntry {
  id: string;
  text: string;
  tags?: string[];
}

/** A loaded plugin. */
export interface PluginEntry {
  name: string;
  enabled: boolean;
  path?: string;
  description?: string;
  error?: string;
}

/** A discoverable skill (project then user scope). `content` is the parsed
 *  SKILL.md body — sent by the core so `/skill:<name>` can apply a skill even
 *  when it lives under ~/.umans-harness/skills (outside the workspace, which
 *  the read_file tool cannot reach). */
export interface SkillInfo {
  name: string;
  description: string;
  location: string;
  content: string;
}

/** An intercom message from a subagent to the orchestrator. `need_decision`
 *  asks are surfaced as a blocking prompt; other traffic is logged. */
export interface IntercomPrompt {
  request_id: string;
  from: string;
  message: string;
  reason?: string;
}

/** A log entry for intercom/subagent activity (kept recent, capped). */
export interface IntercomEntry {
  id: string;
  kind: "ask" | "reply" | "status";
  from?: string;
  to?: string;
  message: string;
  ts: number;
}

/** Vision-handoff configuration (curated vision-capable models + target). */
export interface VisionConfig {
  vision_models: string[];
  vision_model: string | null;
}

/** Core events (server → client). A typed subset of what the core emits. */
export type CoreEvent =
  | ReadyPayload
  | { type: "models"; models: ModelInfo[] }
  | { type: "authed"; ok: boolean; provider: string }
  | { type: "provider_changed"; provider: string; kind: string; base_url: string; has_key: boolean }
  | { type: "approval_changed"; mode: string } // "destructive" | "always" | "<kind>:always"
  | { type: "delta"; text: string }
  | { type: "thinking"; text: string }
  | { type: "tool_call_start"; id: string; index: number }
  | { type: "tool_call_name"; index: number; name: string }
  | { type: "tool_call_args"; index: number; args: string }
  | { type: "tool_call"; id: string; name: string; args: string }
  | { type: "tool_result"; id: string; ok: boolean; output: string; diff?: string; tool?: string }
  | { type: "approval_request"; request_id: string; tool: string; args: string; diff?: string }
  | { type: "metrics" } & Metrics
  | { type: "compacted"; before_tokens: number; after_tokens: number }
  | { type: "http_retry"; attempt?: number; status?: number; backoff_ms?: number; reason?: string }
  | { type: "sessions"; sessions: SessionEntry[]; files: string[] }
  | Stats
  | { type: "history"; messages: unknown[]; tokens_in?: number }
  | { type: "done" }
  | { type: "aborted" }
  | { type: "reset" }
  | { type: "error"; message: string }
  | { type: "info"; message: string }
  | { type: "steer"; prompt: string }
  // ── Subagent / intercom ──
  | { type: "intercom_message"; request_id: string; from: string; message: string; reason?: string; to?: string }
  | { type: "subagent_progress"; message: string; agent?: string; phase?: string }
  // ── Memory ──
  | { type: "memory_saved"; id?: string; text?: string; deleted?: boolean; message?: string }
  | { type: "memory_list"; memories: MemoryEntry[] }
  // ── Plugins ──
  | { type: "plugins_list"; plugins: PluginEntry[] }
  | { type: "plugin_installed"; name: string; ok: boolean; message?: string }
  | { type: "plugin_removed"; name: string; ok: boolean; message?: string }
  | { type: "plugin_enabled"; name: string; ok: boolean }
  | { type: "plugin_disabled"; name: string; ok: boolean }
  | { type: "plugin_error"; name?: string; message: string }
  // ── Vision ──
  | { type: "vision_config"; vision_models: string[]; vision_model: string | null }
  // ── Projects / workspace ──
  | { type: "projects"; projects: ProjectEntry[] }
  | { type: "workspace_changed"; workspace: string; projects: ProjectEntry[] }
  | { type: "session_renamed"; name: string; title: string }
  // ── Compaction / config ──
  | { type: "digested"; count?: number; what?: string }
  | { type: "config_changed"; key: string; value: string | number }
  // ── Skills ──
  | { type: "skills"; skills: SkillInfo[] };

/** Core commands (client → server → core stdin). A typed subset. */
export type CoreCommand =
  | { type: "send"; prompt: string; model: string; reasoning_effort?: string; images?: string[] }
  | { type: "steer"; prompt: string; model: string; reasoning_effort?: string }
  | { type: "abort" }
  | { type: "reset" }
  | { type: "clear" }
  | { type: "compact" }
  | { type: "approve"; request_id: string; decision: "yes" | "no" | "always" }
  | { type: "set_approval"; mode: "never" | "destructive" | "always" }
  | { type: "set_key"; api_key: string; provider?: string }
  | { type: "set_provider"; name: string }
  | { type: "list_sessions" }
  | { type: "load_session"; path: string }
  | { type: "new_session"; path?: string }
  | { type: "stats" }
  | { type: "set_config"; key: string; value: string | number }
  // ── Turn / history ──
  | { type: "undo" }
  // ── Subagent / intercom ──
  | { type: "intercom_reply"; request_id: string; reply: string }
  // ── Memory ──
  | { type: "save_memory"; text: string; tags?: string[] }
  | { type: "list_memory" }
  | { type: "forget_memory"; id: string }
  | { type: "refresh_memory" }
  // ── Plugins ──
  | { type: "install_plugin"; path: string }
  | { type: "remove_plugin"; name: string }
  | { type: "enable_plugin"; name: string }
  | { type: "disable_plugin"; name: string }
  | { type: "list_plugins" }
  // ── Vision ──
  | { type: "get_vision_config" }
  | { type: "set_vision_config"; vision_models?: string[]; vision_model?: string | null }
  // ── Projects / workspace ──
  | { type: "switch_workspace"; path: string }
  | { type: "rename_session"; name: string; title: string }
  | { type: "list_projects" }
  | { type: "add_project"; path: string }
  | { type: "remove_project"; path: string }
  // ── Session lifecycle ──
  | { type: "delete_session"; path: string }
  // ── Skills ──
  | { type: "list_skills" }
  | { type: "apply_skill"; name: string; task?: string; model: string; reasoning_effort?: string };

/** Synthetic events produced by the bridge/client (not from the core). */
export type SyntheticEvent =
  // A user message was sent (added optimistically by the client; tracked by the
  // bridge for snapshot hydration). `model` records the model used for the turn.
  | { type: "_user"; text: string; model?: string; steer?: boolean }
  // The selected model / thinking level changed in the UI.
  | { type: "_select_model"; id: string }
  | { type: "_set_thinking"; level: string }
  // A toast was dismissed by the UI.
  | { type: "_dismiss_toast"; id: string }
  // Optimistic: the bridge is (re)spawning the core after a workspace switch.
  | { type: "_set_switching"; switching: boolean }
  // A custom session title was set/removed (web-layer rename overlay).
  | { type: "_session_title"; name: string; title: string };

export type AgentEvent = CoreEvent | SyntheticEvent;

// ─── UI message model ───────────────────────────────────────────────────────

export interface ToolResult {
  ok: boolean;
  output: string;
  diff?: string;
  /** True when the result was reconstructed from session history (no live
   *  ok/error known). Renders a neutral badge instead of green "ok". */
  unknown?: boolean;
}

export interface UIToolCall {
  id: string;
  name: string;
  args: Record<string, unknown>;
  argString: string;
  /** Present once the tool_result event arrives. */
  result?: ToolResult;
}

export interface UserMsg {
  id: string;
  role: "user";
  text: string;
  ts: number;
  /** True when this message was a steer (redirect of an in-flight turn),
   *  not a fresh prompt. Drives a visual "steering" badge. */
  steer?: boolean;
  /** Images attached to this message (data URLs). */
  images?: string[];
}

export interface AssistantMsg {
  id: string;
  role: "assistant";
  text: string;
  thinking: string;
  toolCalls: UIToolCall[];
  model?: string;
  /** True while this assistant message is still receiving deltas. */
  streaming: boolean;
  usage?: Metrics;
  ts: number;
}

export interface ToolMsg {
  id: string;
  role: "tool";
  toolCallId: string;
  toolName: string;
  output: string;
  ok: boolean;
  diff?: string;
  ts: number;
}

export type UIMessage = UserMsg | AssistantMsg | ToolMsg;

export interface Toast {
  id: string;
  kind: "info" | "error" | "success";
  message: string;
}

/** A workspace file entry for the @-mention flyout. */
export interface FileEntry {
  /** Path relative to the workspace root. */
  path: string;
  /** Just the filename. */
  name: string;
  /** True if this is a directory. */
  dir: boolean;
}

// ─── Agent state (the reducer output) ───────────────────────────────────────

export interface ProjectEntry {
  /** Absolute workspace path. */
  path: string;
  /** Display name (basename). */
  name: string;
  /** Last-accessed timestamp (ms). */
  lastUsed: number;
}

export interface AgentState {
  ready: ReadyPayload | null;
  models: ModelInfo[];
  authed: boolean | null;
  provider: string;
  providerKind: string;
  approvalMode: string;
  escalatedKinds: string[];
  workspace: string;
  /** Known workspace projects (for the project picker). */
  projects: ProjectEntry[];
  selectedModel: string | null;
  thinkingLevel: string;
  messages: UIMessage[];
  currentAssistantId: string | null;
  streaming: boolean;
  retrying: boolean;
  pendingApproval: ApprovalRequest | null;
  metrics: Metrics | null;
  sessions: SessionEntry[];
  currentSessionFile: string | null;
  stats: Stats | null;
  toasts: Toast[];
  memories: MemoryEntry[];
  plugins: PluginEntry[];
  skills: SkillInfo[];
  pendingIntercom: IntercomPrompt | null;
  intercomLog: IntercomEntry[];
  visionConfig: VisionConfig | null;
  /** True while the bridge is (re)spawning the core after a workspace switch. */
  switching: boolean;
}

/** Sent to a freshly-connected client to hydrate the full current state. */
export interface SnapshotEvent {
  type: "_snapshot";
  state: AgentState;
}

export type ServerToClient = CoreEvent | SnapshotEvent;
