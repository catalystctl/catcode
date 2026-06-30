// pi-ai subset — the symbols pi-web imports from `@earendil-works/pi-ai`.
// Re-exported from the package barrel so consumers can switch a single import.

import type {
  Api,
  KnownProvider,
  Model,
  ModelThinkingLevel,
  ThinkingLevelMap,
} from "./types.js";

export type {
  Api,
  Provider,
  KnownProvider,
  Model,
  ModelThinkingLevel,
  ThinkingLevel,
  ThinkingLevelMap,
  TextContent,
  ThinkingContent,
  ImageContent,
  ToolCall,
  ContentBlock,
  Usage,
  StopReason,
  UserMessage,
  AssistantMessage,
  ToolResultMessage,
  Message,
  AgentMessage,
  AssistantMessageEvent,
  AgentState,
  ContextUsage,
  CompactionResult,
  SessionStats,
} from "./types.js";

const DEFAULT_LEVELS: ModelThinkingLevel[] = ["off", "low", "medium", "high"];

/** Thinking levels a model advertises. Mirrors `pi-ai`'s `getSupportedThinkingLevels`. */
export function getSupportedThinkingLevels<TApi extends Api = Api>(
  model: Model<TApi>,
): ModelThinkingLevel[] {
  const levels = (model.thinkingLevels as ModelThinkingLevel[] | undefined) ?? [];
  if (levels.length > 0) {
    // Ensure "off" is always offered (PI includes it when the model supports reasoning).
    return levels.includes("off") ? levels : (["off", ...levels] as ModelThinkingLevel[]);
  }
  return DEFAULT_LEVELS;
}

/** Clamp a requested level to one the model supports. Mirrors `pi-ai`'s `clampThinkingLevel`. */
export function clampThinkingLevel<TApi extends Api = Api>(
  model: Model<TApi>,
  level: ModelThinkingLevel,
): ModelThinkingLevel {
  const supported = getSupportedThinkingLevels(model);
  if (supported.includes(level)) return level;
  // Fall back to the closest supported level.
  if (level === "off") return "off";
  const order: ModelThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];
  const idx = order.indexOf(level);
  for (let i = idx; i >= 0; i--) {
    if (supported.includes(order[i])) return order[i];
  }
  return supported[0] ?? "medium";
}

/** Equality by id + provider. Mirrors `pi-ai`'s `modelsAreEqual`. */
export function modelsAreEqual<TApi extends Api = Api>(
  a: Model<TApi> | null | undefined,
  b: Model<TApi> | null | undefined,
): boolean {
  if (!a || !b) return false;
  return a.id === b.id && a.provider === b.provider;
}

/** Find a built-in/registered model by provider + id.
 *
 * Unlike the real `pi-ai` (which reads a generated model table), this resolves
 * against the shared `ModelRegistry` populated from the harness core's
 * `ready`/`models` events. Returns `undefined` if no core has discovered models
 * yet. */
export function getModel<TProvider extends KnownProvider = KnownProvider, TModelId extends string = string>(
  provider: TProvider,
  modelId: TModelId,
): Model<any> | undefined {
  // Lazy import to avoid a circular module-load dependency at import time.
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const { getSharedModelRegistry } = require("./model-registry.js") as typeof import("./model-registry.js");
  return getSharedModelRegistry().find(provider, modelId);
}

export function getProviders(): KnownProvider[] {
  const { getSharedModelRegistry } = require("./model-registry.js") as typeof import("./model-registry.js");
  return Array.from(new Set(getSharedModelRegistry().getAll().map((m) => m.provider)));
}

export function getModels<TProvider extends KnownProvider = KnownProvider>(
  provider: TProvider,
): Model<any>[] {
  const { getSharedModelRegistry } = require("./model-registry.js") as typeof import("./model-registry.js");
  return getSharedModelRegistry().getAll().filter((m) => m.provider === provider);
}

export function calculateCost<TApi extends Api = Api>(_model: Model<TApi>, _usage: any): number {
  return 0; // cost accounting is provider-specific; not tracked by the wrapper.
}
