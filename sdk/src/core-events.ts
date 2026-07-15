// Typed catalog of every JSONL event the catalyst-code core can emit.
// Kept in sync with `core/src/**` `Event::new("...")` call sites.
//
// `CoreEvent` is intentionally an open `{ type: string } & Record<string, unknown>`
// at the wire boundary (unknown future events must not break the SDK). The
// `CORE_EVENT_TYPES` list + per-kind interfaces below are the documented,
// autocomplete-friendly surface for known events.

/** Every known core event `type` string (alphabetical). */
export const CORE_EVENT_TYPES = [
  "aborted",
  "agents",
  "approval_changed",
  "approval_request",
  "ask_request",
  "audit",
  "authed",
  "bash_execution",
  "checkpoint_created",
  "checkpoint_restored",
  "checkpoints",
  "compacted",
  "compacting",
  "config_changed",
  "context_breakdown",
  "cost_update",
  "delta",
  "digested",
  "done",
  "error",
  "file_change",
  "goal_phase",
  "goal_plan",
  "goal_state",
  "goal_step_verdict",
  "history",
  "http_retry",
  "info",
  "intercom_message",
  "memory_list",
  "memory_saved",
  "metrics",
  "models",
  "oauth_prompt",
  "plugin_commands",
  "plugin_disabled",
  "plugin_enabled",
  "plugin_error",
  "plugin_installed",
  "plugin_removed",
  "plugin_status",
  "plugins_list",
  "protocol_hello",
  "provider_changed",
  "provider_presets",
  "ready",
  "reflecting",
  "reset",
  "search_key_set",
  "session_change_failed",
  "session_changed",
  "session_deleted",
  "session_pinned",
  "session_renamed",
  "sessions",
  "skills",
  "stats",
  "steer",
  "subagent_done",
  "subagent_message",
  "subagent_progress",
  "subagent_start",
  "subagent_tool_call",
  "subagent_tool_result",
  "sudo_request",
  "thinking",
  "tool_call",
  "tool_call_args",
  "tool_call_name",
  "tool_call_start",
  "tool_result",
  "umans_conc",
  "usage",
  "vision_config",
  "work_state",
  "worktree_cleaned",
  "worktree_promoted",
  "worktree_ready",
] as const;

export type CoreEventType = (typeof CORE_EVENT_TYPES)[number];

export function isKnownCoreEventType(type: string): type is CoreEventType {
  return (CORE_EVENT_TYPES as readonly string[]).includes(type);
}

/** Wire-level core event. Always has `type`; other fields vary by kind. */
export type CoreEvent = { type: string } & Record<string, unknown>;

// ── Narrow helpers for common / newly added events ──

export interface ProtocolHelloEvent {
  type: "protocol_hello";
  version: string;
  min_client: string;
  capabilities: string[];
}

export interface FileChangeEvent {
  type: "file_change";
  path: string;
  unified_diff?: string;
  tool: string;
  agent_id?: string;
  run_id?: string;
}

export interface CheckpointCreatedEvent {
  type: "checkpoint_created";
  id: string;
  label: string;
  kind: string;
  auto?: boolean;
  paths?: string[];
}

export interface CheckpointRestoredEvent {
  type: "checkpoint_restored";
  id: string;
  kind: string;
}

export interface CheckpointsEvent {
  type: "checkpoints";
  checkpoints: Array<Record<string, unknown>>;
}

export interface WorktreeReadyEvent {
  type: "worktree_ready";
  run_id: string;
  path: string;
  branch?: string;
}

export interface WorktreeCleanedEvent {
  type: "worktree_cleaned";
  path: string;
}

export interface WorktreePromotedEvent {
  type: "worktree_promoted";
  run_id: string;
  paths: string[];
}

export interface AuditEvent {
  type: "audit";
  tool: string;
  decision: string;
  actor: string;
}

export interface CostUpdateEvent {
  type: "cost_update";
  tokens_in?: number;
  tokens_out?: number;
  cached_tokens?: number;
  cache_hit_pct?: number | null;
  estimated_usd?: number | null;
  model?: string;
}

export interface GoalStepVerdictEvent {
  type: "goal_step_verdict";
  ok: boolean;
  output: string;
}

export interface GoalStateEvent {
  type: "goal_state";
  [key: string]: unknown;
}

export interface GoalPlanEvent {
  type: "goal_plan";
  summary?: string;
  steps?: unknown[];
  risks?: string[];
  validation?: string[];
  version?: number;
}

export interface GoalPhaseEvent {
  type: "goal_phase";
  from: string;
  to: string;
  message?: string;
}

export interface SubagentStartEvent {
  type: "subagent_start";
  run_id: string;
  mode?: string;
  agent?: string;
  agents?: string[];
  task?: string;
  depth?: number;
  started_at?: number;
}

export interface SubagentDoneEvent {
  type: "subagent_done";
  run_id: string;
  state: string;
  summary?: string;
  ended_at?: number;
}

export interface SubagentProgressEvent {
  type: "subagent_progress";
  run_id: string;
  agent?: string;
  phase?: string;
  detail?: string;
  [key: string]: unknown;
}

export interface ApprovalRequestEvent {
  type: "approval_request";
  request_id: string;
  tool: string;
  args: string;
  diff?: string;
}

export interface AskRequestEvent {
  type: "ask_request";
  request_id: string;
  questions: unknown[];
}

export interface SudoRequestEvent {
  type: "sudo_request";
  request_id: string;
  command: string;
}

export interface MetricsEvent {
  type: "metrics";
  ttft_ms?: number;
  elapsed_ms?: number;
  tokens_in?: number;
  prompt_tokens?: number;
  tokens_out?: number;
  cached_tokens?: number;
  tps?: number;
  tps_est?: number;
  model?: string;
  memory_recall?: unknown;
}

export type NarrowCoreEvent =
  | ProtocolHelloEvent
  | FileChangeEvent
  | CheckpointCreatedEvent
  | CheckpointRestoredEvent
  | CheckpointsEvent
  | WorktreeReadyEvent
  | WorktreeCleanedEvent
  | WorktreePromotedEvent
  | AuditEvent
  | CostUpdateEvent
  | GoalStepVerdictEvent
  | GoalStateEvent
  | GoalPlanEvent
  | GoalPhaseEvent
  | SubagentStartEvent
  | SubagentDoneEvent
  | SubagentProgressEvent
  | ApprovalRequestEvent
  | AskRequestEvent
  | SudoRequestEvent
  | MetricsEvent;
