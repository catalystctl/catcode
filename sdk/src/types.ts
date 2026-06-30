// Shared, framework-agnostic types — a PI-compatible surface.
//
// These mirror `@earendil-works/pi-ai` (`types.ts`) and
// `@earendil-works/pi-agent-core` (`types.ts`) so that consumers written against
// the PI SDK compile unchanged against this package. They are *types only*; the
// runtime behaviour is provided by the wrapper classes in the other modules.

// ─── Reasoning / thinking ───────────────────────────────────────────────────

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
export type ModelThinkingLevel = ThinkingLevel; // PI: "off" | ThinkingLevel
export type ThinkingLevelMap = Partial<Record<ModelThinkingLevel, string | null>>;

// ─── Content blocks ─────────────────────────────────────────────────────────

export interface TextContent {
  type: "text";
  text: string;
  textSignature?: string;
}

export interface ThinkingContent {
  type: "thinking";
  thinking: string;
  thinkingSignature?: string;
  redacted?: boolean;
}

export interface ImageContent {
  type: "image";
  data: string;
  mimeType: string;
}

export interface ToolCall {
  type: "toolCall";
  id: string;
  name: string;
  arguments: Record<string, any>;
  thoughtSignature?: string;
}

export type ContentBlock = TextContent | ThinkingContent | ImageContent | ToolCall;

// ─── Usage / stop ────────────────────────────────────────────────────────────

export interface Usage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  totalTokens: number;
  cost: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    total: number;
  };
}

export type StopReason = "stop" | "length" | "toolUse" | "error" | "aborted";

// ─── Messages ────────────────────────────────────────────────────────────────

export interface UserMessage {
  role: "user";
  content: string | (TextContent | ImageContent)[];
  timestamp: number;
}

export interface AssistantMessage {
  role: "assistant";
  content: (TextContent | ThinkingContent | ToolCall)[];
  api: Api;
  provider: Provider;
  model: string;
  responseModel?: string;
  responseId?: string;
  diagnostics?: any[];
  usage: Usage;
  stopReason: StopReason;
  errorMessage?: string;
  timestamp: number;
}

export interface ToolResultMessage<TDetails = any> {
  role: "toolResult";
  toolCallId: string;
  toolName: string;
  content: (TextContent | ImageContent)[];
  details?: TDetails;
  isError: boolean;
  timestamp: number;
}

export type Message = UserMessage | AssistantMessage | ToolResultMessage;

// Extension point (declaration-merge friendly, like PI's CustomAgentMessages).
export interface CustomAgentMessages {}

/** AgentMessage = LLM messages + any registered custom message types. */
export type AgentMessage = Message | CustomAgentMessages[keyof CustomAgentMessages];

// ─── Api / Provider placeholders ─────────────────────────────────────────────
//
// The real PI `Api`/`Provider` are large string unions. We use a permissive
// alias so `Model<any>` and `AssistantMessage.api/provider` type-check against
// any concrete value without forcing consumers to pin a literal.

export type Api = string;
export type Provider = string;
export type KnownProvider = string;

// ─── Model ──────────────────────────────────────────────────────────────────

export interface ModelCost {
  input: number;
  output: number;
  cacheRead?: number;
  cacheWrite?: number;
}

export interface ModelInputCapabilities {
  text: boolean;
  image: boolean;
}

export interface Model<TApi extends Api = Api> {
  id: string;
  name: string;
  provider: string;
  api: TApi;
  contextWindow: number;
  maxTokens: number;
  reasoning: boolean;
  vision?: boolean;
  thinkingLevelMap?: ThinkingLevelMap;
  thinkingLevels?: ModelThinkingLevel[];
  input?: ModelInputCapabilities;
  cost?: ModelCost;
  // Allow provider-specific extras without breaking structural typing.
  [key: string]: unknown;
}

// ─── AssistantMessageEvent (stream deltas) ──────────────────────────────────

export type AssistantMessageEvent =
  | { type: "start"; partial: AssistantMessage }
  | { type: "text_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "text_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "text_end"; contentIndex: number; content: string; partial: AssistantMessage }
  | { type: "thinking_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "thinking_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "thinking_end"; contentIndex: number; content: string; partial: AssistantMessage }
  | { type: "toolcall_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "toolcall_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "toolcall_end"; contentIndex: number; toolCall: ToolCall; partial: AssistantMessage }
  | { type: "done"; reason: Extract<StopReason, "stop" | "length" | "toolUse">; message: AssistantMessage }
  | { type: "error"; reason: Extract<StopReason, "aborted" | "error">; error: AssistantMessage };

// ─── Agent state ─────────────────────────────────────────────────────────────

export interface AgentState {
  systemPrompt: string;
  model: Model<any>;
  thinkingLevel: ThinkingLevel;
  tools: any[];
  messages: AgentMessage[];
  readonly isStreaming: boolean;
  readonly streamingMessage?: AgentMessage;
  readonly pendingToolCalls: ReadonlySet<string>;
  readonly errorMessage?: string;
}

// ─── Context usage ───────────────────────────────────────────────────────────

export interface ContextUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  contextWindow: number;
  percentage: number;
}

// ─── Compaction / session stats ──────────────────────────────────────────────

export interface CompactionResult {
  summary: string;
  tokensBefore: number;
  tokensAfter: number;
  firstKeptEntryId?: string;
}

export interface SessionStats {
  sessionFile: string | undefined;
  sessionId: string;
  userMessages: number;
  assistantMessages: number;
  toolCalls: number;
  toolResults: number;
  totalMessages: number;
  tokens: {
    input: number;
    output: number;
    cacheRead: number;
    cacheWrite: number;
    total: number;
  };
  cost: number;
  contextUsage?: ContextUsage;
}
