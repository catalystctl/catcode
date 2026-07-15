// AgentSessionEvent — the event union pi-web's `handleEvent` switch consumes.
// Mirrors `@earendil-works/pi-agent-core` `AgentEvent` (base variants) plus the
// session-specific overrides/additions from `pi-coding-agent`'s `AgentSession`.
//
// Every raw harness JSONL event is ALSO re-emitted as `{ type: "core_event", event }`
// so SDK consumers can observe the full core protocol without dropping PI mapping.

import type {
  AgentMessage,
  AssistantMessageEvent,
  CompactionResult,
  ThinkingLevel,
  ToolResultMessage,
} from "./types.js";
import type { CoreEvent } from "./core-events.js";

export type AgentSessionEventListener = (event: AgentSessionEvent) => void;
export type CoreEventListener = (event: CoreEvent) => void;

export type AgentSessionEvent =
  // ── base AgentEvent variants ──
  | { type: "agent_start" }
  | { type: "turn_start" }
  | { type: "turn_end"; message: AgentMessage; toolResults: ToolResultMessage[] }
  | { type: "message_start"; message: AgentMessage }
  | { type: "message_update"; message: AgentMessage; assistantMessageEvent: AssistantMessageEvent }
  | { type: "message_end"; message: AgentMessage }
  | { type: "tool_execution_start"; toolCallId: string; toolName: string; args: any }
  | {
      type: "tool_execution_update";
      toolCallId: string;
      toolName: string;
      args: any;
      partialResult: any;
    }
  | {
      type: "tool_execution_end";
      toolCallId: string;
      toolName: string;
      result: any;
      isError: boolean;
    }
  // ── session overrides / additions ──
  | { type: "agent_end"; messages: AgentMessage[]; willRetry: boolean }
  | { type: "queue_update"; steering: readonly string[]; followUp: readonly string[] }
  | { type: "compaction_start"; reason: "manual" | "threshold" | "overflow" }
  | {
      type: "compaction_end";
      reason: "manual" | "threshold" | "overflow";
      result: CompactionResult | undefined;
      aborted: boolean;
      willRetry: boolean;
      errorMessage?: string;
    }
  | { type: "session_info_changed"; name: string | undefined }
  | { type: "thinking_level_changed"; level: ThinkingLevel }
  | {
      type: "auto_retry_start";
      attempt: number;
      maxAttempts: number;
      delayMs: number;
      errorMessage: string;
    }
  | { type: "auto_retry_end"; success: boolean; attempt: number; finalError?: string }
  /** Passthrough of every raw core JSONL event (including ones with no PI mapping). */
  | { type: "core_event"; event: CoreEvent };

/** Prompt options — mirrors `pi-coding-agent`'s `PromptOptions`. */
export interface PromptOptions {
  expandPromptTemplates?: boolean;
  images?: import("./types.js").ImageContent[];
  streamingBehavior?: "steer" | "followUp";
  source?: string;
  preflightResult?: (success: boolean) => void;
}

export type InputSource = "interactive" | "rpc" | "api" | "steer" | "followUp";

export interface ModelCycleResult {
  model: import("./types.js").Model<any>;
  thinkingLevel: ThinkingLevel;
  isScoped: boolean;
}
