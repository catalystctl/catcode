// Shared wire + UI types for the catalyst-code web frontend.
//
// The core speaks newline-delimited JSON over stdio. The server bridge forwards
// raw core events to the browser over SSE, and accepts raw core commands via
// POST. Wire-level event payloads are sourced from the SDK (@catalyst-code/coding-agent)'s
// typed event catalog (core-events.ts / core-process.ts); the UI message model
// is assembled by the reducer from the event stream.
//
// This file is imported by BOTH server and browser code — use ONLY `import type`
// (no runtime SDK imports).

import type {
  ApprovalRequestEvent,
  CostUpdateEvent,
  FileChangeEvent,
  MetricsEvent,
  ProtocolHelloEvent,
  SudoRequestEvent,
  WorktreeReadyEvent,
} from "@catalyst-code/coding-agent";
import type { ReadyPayload as SDKReadyPayload } from "@catalyst-code/coding-agent";

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
  /** Owning provider name (e.g. "openai", "gemini", "anthropic"), populated by
   * the core's multi-provider aggregation so a turn routes to the right endpoint
   * when multiple providers are logged in. Empty for legacy single-provider models. */
  provider?: string;
}

/** A built-in first-party provider template advertised by the core. */
export interface ProviderPreset {
  id: string;
  label: string;
  kind: string;
  base_url: string;
  envVar: string;
  altEnvs?: string[];
  description: string;
  hasKey: boolean;
  configured: boolean;
  loggedIn: boolean;
  supportsOauth?: boolean;
}

export interface ReadyPayload extends SDKReadyPayload {
  type: "ready";
  models: ModelInfo[];
  providerPresets?: ProviderPreset[];
  /** When true, the core auto-compacts context on thresholds / idle. */
  auto_compact?: boolean;
  /** Configured fractions; runtime thresholds may be lower when response
   * headroom for the selected model requires it. */
  context_compact_at?: number;
  context_digest_at?: number;
  /** Bash hard sandbox: `"none"` | `"firejail"` | `"seatbelt"`. */
  sandbox?: string;
  plugins_skipped?: string[];
}

/** A discoverable subagent (builtin / user / project) from `agents` events. */
export interface AgentInfo {
  name: string;
  description: string;
  source: "builtin" | "user" | "project" | string;
}

/** Latest `/context` payload kept for the Diagnostics panel. */
export interface ContextBreakdown {
  total_tokens: number;
  context_window: number;
  pct: number;
  messages: number;
  system_tokens: number;
  digest_threshold_tokens?: number;
  compact_threshold_tokens?: number;
  hard_limit_tokens?: number;
  response_reserve_tokens?: number;
  safety_margin_tokens?: number;
  by_role: Record<string, number>;
  top_consumers: {
    index: number;
    role: string;
    tokens: number;
    preview: string;
  }[];
}

/** Latest `/usage` payload kept for the Diagnostics panel. */
export interface UsageSnapshot {
  provider: string;
  provider_kind?: string;
  model?: string;
  base_url?: string;
  available: boolean;
  plan?: string;
  message?: string;
  windows: Array<{
    id: string;
    label: string;
    used?: number;
    limit?: number;
    unit: string;
    resets_at?: number;
    detail?: string;
  }>;
}

export type ApprovalRequest = Omit<ApprovalRequestEvent, "type">;

/** Stream + final-turn metrics. Mid-stream: `tokens_in` = live context;
 *  final = `elapsed_ms`/`prompt_tokens` present (true input).
 *  `tps_est` (mid-stream estimate) is mapped to `tps` in the reducer. */
export type Metrics = Omit<MetricsEvent, "type">;

/** Live, account-wide Umans concurrency usage from the gateway's `/v1/usage`
 *  endpoint, polled every few seconds by the core (independent of turns) so the
 *  footer can show a "Conc used/limit" field ahead of tps. `used == null` means
 *  not Umans / fetch failed (hide); `limit == null` means the plan is unlimited
 *  (render ∞). */
export interface UmansConc {
  used: number | null;
  limit: number | null;
  /** The Umans provider name the poll is tracking; the UI only renders the
   *  field when the selected model routes to this provider. */
  provider: string;
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
  /** Pinned in the session picker. */
  pinned?: boolean;
}

/** Cumulative / turn cost from core `cost_update` events. */
export type CostUpdate = Omit<CostUpdateEvent, "type">;

export type ProtocolHello = Omit<ProtocolHelloEvent, "type">;

export interface CheckpointInfo {
  id: string;
  label?: string;
  kind?: string;
  auto?: boolean;
  paths?: string[];
  created_at?: number;
  [key: string]: unknown;
}

export type WorktreeInfo = Omit<WorktreeReadyEvent, "type">;

export type FileChangeRecord = Omit<FileChangeEvent, "type"> & { ts: number };

export type ApproveDecision =
  | "yes"
  | "no"
  | "always"
  | "allow_session"
  | "allow_pattern";

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

/** A saved memory note (persisted per-workspace, injected into the system prompt).
 *  The core emits the full memory text under `content`, a one-line `description`,
 *  a `name` (slug/title), and `scope` ("workspace" | "global"). `text` is an
 *  alias for `name` kept for TUI parity; `tags` surfaces the memory type. */
export interface MemoryEntry {
  id: string;
  /** Memory slug/title (the core generates one on save). */
  name?: string;
  /** One-line description shown as a subtitle. */
  description?: string;
  /** Full memory text — the actual content the agent remembers. */
  content?: string;
  /** "workspace" or "global". */
  scope?: string;
  /** Memory type label (e.g. "note", "convention", "decision"). */
  type?: string;
  /** Alias for `name` (TUI parity). Kept for backward compat. */
  text: string;
  tags?: string[];
}

/** A loaded plugin. The core emits `name`, `version`, `enabled`, `description`,
 *  and `hooks` (the list of hook-point names the plugin registers). */
export interface PluginEntry {
  name: string;
  enabled: boolean;
  version?: string;
  path?: string;
  description?: string;
  /** Hook-point names this plugin registers (e.g. ["pre_write","post_bash"]). */
  hooks?: string[];
  error?: string;
}

/** A discoverable skill (project then user scope). `content` is the parsed
 *  SKILL.md body — sent by the core so `/skill:<name>` can apply a skill even
 *  when it lives under ~/.catalyst-code/skills (outside the workspace, which
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

/** One question in an `ask` tool call: a multiple-choice selection or a
 *  free-text box. `options` is required for type "select". */
export interface AskQuestion {
  id: string;
  prompt: string;
  type: "select" | "text";
  options?: string[];
  allowCustom?: boolean;
  required?: boolean;
  placeholder?: string;
}

/** A pending `ask` tool call — the model asked the user one or more questions
 *  and is blocking until they answer or skip. */
export interface AskPrompt {
  request_id: string;
  questions: AskQuestion[];
}

/** A pending sudo_request: the agent wants to run a bash command that invokes
 *  `sudo`. The user must approve (with a password) or decline. */
export type SudoPrompt = Omit<SudoRequestEvent, "type">;

/** A log entry for intercom/subagent activity (kept recent, capped). */
export interface IntercomEntry {
  id: string;
  kind: "ask" | "reply" | "status";
  from?: string;
  to?: string;
  message: string;
  ts: number;
}

/** A single item in a subagent run's live chat transcript: either a
 *  user/assistant message or a tool call with its (later-arriving) result. */
export interface SubagentChatItem {
  /** Generated unique id (NOT the tool call_id, which can be empty/duplicate). */
  id: string;
  kind: "message" | "tool";
  ts: number;
  // message
  role?: "user" | "assistant";
  content?: string;
  // tool
  callId?: string;
  name?: string;
  args?: Record<string, unknown>;
  result?: string;
  ok?: boolean;
}

/** A live subagent run: lifecycle metadata + a per-run chat transcript the
 *  SubagentsPanel drills into. Keyed by run_id in `AgentState.subagentRuns`. */
export interface SubagentRunView {
  id: string;
  mode: string; // single | parallel | chain
  agent?: string;
  agents: string[];
  task: string;
  state: string; // running | completed | failed | paused
  depth: number;
  startedAt: number;
  endedAt?: number;
  summary?: string;
  phase?: string; // last progress phase
  tool?: string; // current tool name
  toolCount: number;
  tokensIn: number;
  tokensOut: number;
  elapsedMs: number;
  items: SubagentChatItem[];
}

/** Vision-handoff configuration (curated vision-capable models + target). */
export interface VisionConfig {
  /** Auto handoff on image turns (default true / recommended ON). */
  enabled: boolean;
  vision_models: string[];
  vision_model: string | null;
}

/** An OAuth authorization prompt from the core: the user must visit `url`
 *  (and, for the device flow, enter `code`) to complete a provider login. For
 *  the no-browser flow the user pastes the resulting code/callback URL back via
 *  the `oauth_code` command. */
export interface OauthPrompt {
  url: string;
  code?: string;
  message?: string;
}

/** Rolling work-state summary the core maintains from conversation signals
 *  (goal / done / in-progress / next / recent files / last activity). Emitted
 *  via `work_state` so the UI can render a live status panel. */
export interface WorkState {
  version: number;
  goal: string;
  done: string[];
  in_progress: string[];
  next: string[];
  recent_files: string[];
  last_activity: string;
}

/** First-class goal mode snapshot from `goal_state` events. */
export interface GoalModeState {
  id: string;
  goal: string;
  phase: string;
  concurrency: number;
  max_tasks: number;
  allowed_models: string[];
  allowed_providers: string[];
  auto_deploy: boolean;
  role_models?: {
    planner?: string | null;
    worker?: string | null;
    reviewer?: string | null;
  };
  model_concurrency?: Record<string, number>;
  prompts: GoalPrompt[];
  active_run_ids: string[];
  version: number;
  error: string | null;
  parent_model: string;
}

export interface GoalPrompt {
  step_id: string;
  agent: string;
  title: string;
  task: string;
  model?: string | null;
  status: string;
  run_id?: string | null;
  summary?: string | null;
}

export interface GoalPlan {
  id: string;
  summary: string;
  steps: Array<{
    id: string;
    agent: string;
    title: string;
    task: string;
    model?: string;
    depends_on?: string[];
    parallel_group?: string;
  }>;
  risks: string[];
  validation: string[];
  version: number;
}

/** Core events (server → client). A typed subset of what the core emits. */
export type CoreEvent =
  | ReadyPayload
  | { type: "models"; models: ModelInfo[] }
  | { type: "provider_presets"; presets: ProviderPreset[] }
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
  /** User-initiated `!cmd` / `!!cmd` (PI-compatible bang bash). */
  | {
      type: "bash_execution";
      command: string;
      output: string;
      ok: boolean;
      exclude_from_context?: boolean;
    }
  | { type: "approval_request"; request_id: string; tool: string; args: string; diff?: string }
  | {
      type: "protocol_hello";
      version: string;
      min_client: string;
      capabilities: string[];
    }
  | {
      type: "file_change";
      path: string;
      unified_diff?: string;
      tool: string;
      agent_id?: string;
      run_id?: string;
    }
  | {
      type: "checkpoint_created";
      id: string;
      label: string;
      kind: string;
      auto?: boolean;
      paths?: string[];
    }
  | { type: "checkpoint_restored"; id: string; kind: string }
  | { type: "checkpoints"; checkpoints: CheckpointInfo[] }
  | { type: "worktree_ready"; run_id: string; path: string; branch?: string }
  | { type: "worktree_cleaned"; path: string }
  | { type: "worktree_promoted"; run_id: string; paths: string[] }
  | { type: "audit"; tool: string; decision: string; actor: string }
  | ({ type: "cost_update" } & CostUpdate)
  | { type: "goal_step_verdict"; ok: boolean; output: string }
  | { type: "search_key_set"; provider: string; has_key: boolean }
  | { type: "plugin_commands"; commands: unknown[] }
  | { type: "plugin_status"; plugin: string; text: string }
  | { type: "session_changed"; path: string; new?: boolean }
  | { type: "session_change_failed"; path: string; message: string }
  | { type: "session_deleted"; path: string }
  | { type: "session_pinned"; path: string; pinned: boolean }
  | { type: "ask_request"; request_id: string; questions: AskQuestion[] }
  | { type: "sudo_request"; request_id: string; command: string }
  | { type: "metrics" } & Metrics
  | { type: "umans_conc"; used: number | null; limit: number | null; provider: string }
  | {
      type: "compacted";
      before_tokens: number;
      after_tokens: number;
      summary_chars?: number;
      context_window?: number;
      threshold_tokens?: number;
      hard_limit_tokens?: number;
      within_limit?: boolean;
      scope?: string;
    }
  | {
      type: "compacting";
      before_tokens: number;
      trigger: string;
      context_window?: number;
      threshold_tokens?: number;
      hard_limit_tokens?: number;
      response_reserve_tokens?: number;
      safety_margin_tokens?: number;
      utilization_pct?: number;
    }
  | ({ type: "context_breakdown" } & ContextBreakdown)
  | ({ type: "usage" } & UsageSnapshot)
  | { type: "agents"; agents: AgentInfo[] }
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
  | { type: "intercom_message"; id: string; from: string; message: string; reason?: string; to?: string }
  | { type: "subagent_progress"; run_id: string; agent: string; phase: string; tool: string; tool_count: number; tokens_in: number; tokens_out: number; elapsed_ms: number; ok: boolean }
  | { type: "subagent_start"; run_id: string; mode: string; agent?: string; agents: string[]; task: string; depth: number; started_at: number }
  | { type: "subagent_message"; run_id: string; role: string; content: string }
  | { type: "subagent_tool_call"; run_id: string; call_id: string; name: string; args: Record<string, unknown>; tool_count: number }
  | { type: "subagent_tool_result"; run_id: string; call_id: string; name: string; result: string; ok: boolean }
  | { type: "subagent_done"; run_id: string; state: string; summary?: string; ended_at: number }
  // ── Memory ──
  | { type: "memory_saved"; id?: string; text?: string; deleted?: boolean; message?: string }
  | { type: "memory_list"; entries: MemoryEntry[] }
  // ── Plugins ──
  | { type: "plugins_list"; plugins: PluginEntry[] }
  | { type: "plugin_installed"; name: string; ok?: boolean; message?: string }
  | { type: "plugin_removed"; name: string; ok?: boolean; message?: string }
  | { type: "plugin_enabled"; name: string; ok?: boolean }
  | { type: "plugin_disabled"; name: string; ok?: boolean }
  | { type: "plugin_error"; name?: string; message: string }
  // ── Vision ──
  | { type: "vision_config"; enabled?: boolean; vision_models: string[]; vision_model: string | null }
  // ── Projects / workspace ──
  | { type: "projects"; projects: ProjectEntry[] }
  | { type: "workspace_changed"; workspace: string; projects: ProjectEntry[] }
  | { type: "session_renamed"; name: string; title: string }
  // ── Compaction / config ──
  | {
      type: "digested";
      results: number;
      before_tokens?: number;
      after_tokens?: number;
      trigger?: string;
      context_window?: number;
      threshold_tokens?: number;
      hard_limit_tokens?: number;
      utilization_pct?: number;
      scope?: string;
    }
  | { type: "config_changed"; key: string; value: string | number | boolean }
  // ── OAuth / lifecycle status ──
  | { type: "oauth_prompt"; url: string; code?: string; message?: string }
  | { type: "reflecting"; recurrence: number | string }
  | { type: "work_state"; version: number; goal: string; done: string[]; in_progress: string[]; next: string[]; recent_files: string[]; last_activity: string }
  // ── Goal mode ──
  | ({ type: "goal_state" } & GoalModeState)
  | ({ type: "goal_plan" } & GoalPlan)
  | { type: "goal_phase"; from: string; to: string; message?: string }
  // ── Skills ──
  | { type: "skills"; skills: SkillInfo[] };

/** Core commands (client → server → core stdin). A typed subset. */
export type CoreCommand =
  | { type: "send"; prompt: string; model: string; reasoning_effort?: string; images?: string[] }
  | { type: "steer"; prompt: string; model: string; reasoning_effort?: string }
  | { type: "abort" }
  | { type: "clear_queue" }
  | { type: "user_bash"; command: string; exclude_from_context?: boolean }
  | { type: "reset" }
  | { type: "clear" }
  | { type: "compact"; instructions?: string }
  | { type: "context" }
  | { type: "usage"; model?: string }
  | {
      type: "approve";
      request_id: string;
      decision: ApproveDecision;
      pattern?: string;
    }
  | { type: "set_approval"; mode: "never" | "destructive" | "always" }
  | { type: "create_checkpoint"; label?: string; paths?: string[] }
  | { type: "list_checkpoints" }
  | { type: "restore_checkpoint"; id: string }
  | { type: "pin_session"; path: string; pinned: boolean }
  | { type: "set_key"; api_key: string; provider?: string }
  | { type: "set_search_key"; provider: string; api_key: string }
  | { type: "set_provider"; name: string }
  | { type: "list_provider_presets" }
  | { type: "login"; preset: string; api_key?: string }
  | { type: "login_oauth"; preset: string }
  | { type: "logout"; provider: string }
  | { type: "oauth_code"; code: string }
  | { type: "list_sessions" }
  | { type: "load_session"; path: string }
  | { type: "new_session"; path?: string }
  | { type: "stats" }
  | { type: "set_config"; key: string; value: string | number | boolean }
  // ── Turn / history ──
  | { type: "undo" }
  // ── Subagent / intercom ──
  | { type: "intercom_reply"; request_id: string; reply: string }
  // ── Ask tool ──
  | { type: "ask_reply"; request_id: string; answers: Record<string, string> | null }
  // ── Sudo passthrough (bash command invokes sudo) ──
  | { type: "sudo_reply"; request_id: string; approved: boolean; password?: string }
  // ── Memory ──
  | { type: "save_memory"; text: string; tags?: string[]; scope?: "workspace" | "global" }
  | { type: "list_memory" }
  | { type: "forget_memory"; id: string }
  | { type: "refresh_memory" }
  // ── Plugins ──
  | { type: "install_plugin"; path: string; scope?: "workspace" | "global" }
  | { type: "remove_plugin"; name: string }
  | { type: "enable_plugin"; name: string }
  | { type: "disable_plugin"; name: string }
  | { type: "list_plugins" }
  | { type: "list_agents" }
  // ── Vision ──
  | { type: "get_vision_config" }
  | {
      type: "set_vision_config";
      enabled?: boolean;
      vision_models?: string[];
      vision_model?: string | null;
    }
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
  | { type: "apply_skill"; name: string; task?: string; model: string; reasoning_effort?: string }
  // ── Goal mode ──
  | {
      type: "start_goal";
      goal: string;
      concurrency?: number;
      max_tasks?: number;
      allowed_models?: string[];
      allowed_providers?: string[];
      auto_deploy?: boolean;
      planner_model?: string;
      worker_model?: string;
      reviewer_model?: string;
      model_concurrency?: Record<string, number>;
      model: string;
      reasoning_effort?: string;
    }
  | { type: "cancel_goal" }
  | { type: "goal_status" }
  | { type: "approve_goal_plan" }
  | { type: "revise_goal"; feedback: string; model: string; reasoning_effort?: string };

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
  | { type: "_session_title"; name: string; title: string }
  /** Client-side undo: drop the last turn locally; next `reset` keeps messages. */
  | { type: "_undo_local" };

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

/** User-initiated bang bash (`!` / `!!`) shown in the transcript. */
export interface BashMsg {
  id: string;
  role: "bash";
  command: string;
  output: string;
  ok: boolean;
  /** True for `!!cmd` — output was not added to model context. */
  excludeFromContext?: boolean;
  ts: number;
}

export type UIMessage = UserMsg | AssistantMsg | ToolMsg | BashMsg;

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
  /** First-party provider presets (Umans, OpenCode Go, OpenRouter) from the
   * core, used by the /login + /logout pickers. */
  providerPresets: ProviderPreset[];
  selectedModel: string | null;
  thinkingLevel: string;
  messages: UIMessage[];
  currentAssistantId: string | null;
  streaming: boolean;
  retrying: boolean;
  pendingApproval: ApprovalRequest | null;
  pendingAsk: AskPrompt | null;
  pendingSudo: SudoPrompt | null;
  metrics: Metrics | null;
  /** Live Umans concurrency (used/limit) from the `/v1/usage` poll; null when
   *  not Umans / fetch failed. Drives a footer field ahead of tps. */
  umansConc: UmansConc | null;
  sessions: SessionEntry[];
  currentSessionFile: string | null;
  stats: Stats | null;
  toasts: Toast[];
  memories: MemoryEntry[];
  plugins: PluginEntry[];
  skills: SkillInfo[];
  /** Discoverable subagents from core `agents` events. */
  availableAgents: AgentInfo[];
  pendingIntercom: IntercomPrompt | null;
  pendingOauth: OauthPrompt | null;
  intercomLog: IntercomEntry[];
  /** Live subagent runs keyed by run_id — the SubagentsPanel list + drill-in chat. */
  subagentRuns: Record<string, SubagentRunView>;
  visionConfig: VisionConfig | null;
  /** Last `/context` breakdown for the Diagnostics panel. */
  contextBreakdown: ContextBreakdown | null;
  /** Last `/usage` snapshot for the Diagnostics panel. */
  usageSnapshot: UsageSnapshot | null;
  /** Rolling work-state summary (goal/done/doing/next/recent files) from
   *  `work_state` events — drives the ambient status panel. */
  workState: WorkState | null;
  /** Active goal mode (plan → deploy). Null when idle. */
  goalMode: GoalModeState | null;
  /** Last structured plan from `goal_plan` (for plan-ready review). */
  goalPlan: GoalPlan | null;
  /** Protocol capabilities handshake from `protocol_hello`. */
  protocolHello: ProtocolHello | null;
  /** Latest `cost_update` totals (session-scoped estimate). */
  cost: CostUpdate | null;
  /** Known hybrid checkpoints from `checkpoints` / create events. */
  checkpoints: CheckpointInfo[];
  /** Monotonic counter bumped on every `file_change` (IDE refresh signal). */
  fileChangeSeq: number;
  /** Recent agent file mutations (newest first, capped). */
  recentFileChanges: FileChangeRecord[];
  /** Active parallel-subagent worktrees. */
  worktrees: WorktreeInfo[];
  /** Slash commands contributed by plugins. */
  pluginCommands: unknown[];
  /** Search-provider API key presence (`provider` → has_key). */
  searchKeys: Record<string, boolean>;
  /** Last goal wave verifier result. */
  lastGoalVerdict: { ok: boolean; output: string } | null;
  /** True while the bridge is (re)spawning the core after a workspace switch. */
  switching: boolean;
  /** True when the core has a one-deep follow-up/steer queued behind the live turn. */
  followUpQueued: boolean;
  /** After `_undo_local`, the next core `reset` must not wipe the trimmed transcript. */
  pendingUndo: boolean;
}

/** Sent to a freshly-connected client to hydrate the full current state. */
export interface SnapshotEvent {
  type: "_snapshot";
  state: AgentState;
}

export type ServerToClient = CoreEvent | SnapshotEvent;

// ─── IDE panel types (client-only + API DTOs) ──────────────────────────────
// Per docs/IDE_PANELS_CONTRACT.md §2. Client-only layout state + API DTOs;
// NEVER reduced into AgentState / never sent over SSE.

/** A panel the IDE shell can show. "copilot" is handled separately (the dock). */
export type IdePanelId = "explorer" | "git" | "terminal" | "preview";

/** Panels that can be moved between IDE dock zones. */
export type MovablePanelId = "chat" | "git" | "terminal" | "preview";

/** A drop target around (or in place of) the fixed editor work area. */
export type DockPosition = "left" | "right" | "bottom" | "main";

/** One entry in the file-explorer tree (one level of a directory). */
export interface FileNode {
  /** Workspace-relative path with forward slashes (e.g. "src/lib/foo.ts"). */
  path: string;
  /** Just the basename. */
  name: string;
  /** True if this is a directory. */
  dir: boolean;
  /** File size in bytes (0 for dirs). */
  size?: number;
  /** mtime in ms (for change detection / refresh). */
  mtime?: number;
  /** True if the entry is a symlink (rendered with an arrow). */
  symlink?: boolean;
}

/** One row of `git status --porcelain=v2`. */
export interface GitStatusEntry {
  /** Workspace-relative path. For renames: "old -> new". */
  path: string;
  /** Original path for renames, else null. */
  oldPath?: string | null;
  /** XY status codes from porcelain v2 (e.g. "M ", " M", "A ", "??", "R "). */
  xy: string;
  /** Human label. */
  status: "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";
  /** Staged (index) vs unstaged (worktree). */
  staged: boolean;
}

/** Aggregate git state for the git panel + status bar. */
export interface GitStatus {
  /** Current branch name, or "HEAD (detached)". */
  branch: string;
  /** Commits ahead of upstream (0 if no upstream). */
  ahead: number;
  /** Commits behind upstream. */
  behind: number;
  /** All changed entries (staged + unstaged + untracked). */
  entries: GitStatusEntry[];
  /** HEAD commit short oid, or null if no commits. */
  head: { oid: string; message: string; author: string; ts: number } | null;
  /** True if the workspace is not a git repo (panel shows "initialize" CTA). */
  bare: boolean;
  /** Configured upstream for the current branch, if any. */
  upstream?: string | null;
  /** Local and remote branches. */
  branches?: GitBranch[];
  /** Recent commits across all refs. */
  commits?: GitCommit[];
  /** Saved worktree snapshots. */
  stashes?: GitStash[];
  /** Repository tags. */
  tags?: GitTag[];
  /** Configured remotes and their fetch/push URLs. */
  remotes?: GitRemote[];
}

export interface GitBranch {
  name: string;
  oid: string;
  current: boolean;
  remote: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
}

export interface GitCommit {
  oid: string;
  shortOid: string;
  parents: string[];
  subject: string;
  author: string;
  email: string;
  ts: number;
  refs: string[];
}

export interface GitStash {
  ref: string;
  oid: string;
  subject: string;
  ts: number;
}

export interface GitTag {
  name: string;
  oid: string;
  subject: string;
}

export interface GitRemote {
  name: string;
  fetchUrl: string;
  pushUrl: string;
}

/** A live, persistent PTY session rendered by Ghostty in the browser.
 *  Session metadata is persisted per project; the server PTY survives refresh
 *  until the user closes the tab or the web process restarts. */
export interface TerminalSession {
  /** Client-generated id (e.g. "term_<ts>_<n>"). */
  id: string;
  /** Display title (defaults to shell name; user-renamable). */
  title: string;
  /** Workspace-relative or absolute cwd the shell started in. */
  cwd: string;
  /** True while the shell process is alive. */
  alive: boolean;
  /** Last exit code (null while alive / not yet exited). */
  exitCode: number | null;
}

/** Preview panel state. */
export interface PreviewState {
  /** What is being previewed. */
  kind: "file" | "url" | "none";
  /** Workspace-relative file path (kind="file") or absolute URL (kind="url"). */
  target: string;
  /** Optional query/anchor to append. */
  query?: string;
}

/** A tab in the main work area (open file / preview / terminal-host). */
export interface IdeTab {
  /** Unique id (path for files, "preview:<target>", "term:<id>"). */
  id: string;
  kind: "file" | "preview" | "terminal";
  /** Workspace-relative path (file) or target (preview) or terminal id. */
  target: string;
  /** Display label (basename for files). */
  label: string;
  /** Dirty flag (unsaved editor changes). */
  dirty: boolean;
  /** Detected language id for the editor (e.g. "typescript", "markdown"). */
  language?: string;
}

/** Client-only IDE layout state. NEVER sent over SSE / never in AgentState. */
export interface IdeLayoutState {
  /** Which panel's sidebar is shown in PrimarySidebar. */
  activePanel: IdePanelId;
  /** Open tabs in the main work area (ordered). */
  openTabs: IdeTab[];
  /** id of the active tab (null = none). */
  activeTabId: string | null;
  /** PrimarySidebar width in px. */
  sidebarWidth: number;
  /** True when PrimarySidebar is collapsed (hidden). */
  sidebarCollapsed: boolean;
  /** Bottom panel height in px (0 = collapsed). */
  bottomPanelHeight: number;
  /** True when the bottom panel is visible. */
  bottomPanelVisible: boolean;
  /** True when the copilot (Chat) dock is visible. */
  copilotVisible: boolean;
  /** Copilot dock width in px. */
  copilotWidth: number;
  /** Current dock position for every movable panel. */
  panelLocations: Record<MovablePanelId, DockPosition>;
  /** Panels remain mounted only while visible. */
  panelVisibility: Record<MovablePanelId, boolean>;
  /** Selected panel when several panels share a dock zone. */
  activeDockPanels: Record<DockPosition, MovablePanelId | null>;
  /** Shared width of the optional dock on the editor's left edge. */
  leftDockWidth: number;
  /** Live terminal sessions. */
  terminals: TerminalSession[];
  /** Active terminal session id (null = none). */
  activeTerminalId: string | null;
  /** Last-known git status (null until first refresh). */
  gitStatus: GitStatus | null;
  /** Current preview target. */
  preview: PreviewState;
  /** File-tree expanded directory paths (set, persisted across reloads). */
  expandedDirs: string[];
}
