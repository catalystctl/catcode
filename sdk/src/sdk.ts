// Top-level `createAgentSession` + tool factory stubs — mirrors
// `pi-coding-agent`'s `core/sdk.ts`. The tool factories are no-ops: the umans-harness
// core owns the built-in toolset (read_file/edit/write_file/grep/glob/bash/...).

import { getAgentDir } from "./config.js";
import { AuthStorage } from "./auth-storage.js";
import { ModelRegistry, getSharedAuth, getSharedModelRegistry } from "./model-registry.js";
import { SettingsManager } from "./settings-manager.js";
import { SessionManager } from "./session-manager.js";
import { DefaultResourceLoader } from "./resource-loader.js";
import { AgentSession, type AgentSessionServices } from "./agent-session.js";
import {
  createAgentSessionServices,
  createAgentSessionFromServices,
  type CreateAgentSessionResult,
  type CreateAgentSessionServicesOptions,
  type CreateAgentSessionFromServicesOptions,
} from "./agent-session-services.js";
import type { Model, ThinkingLevel, ToolCall, ImageContent, TextContent } from "./types.js";

export interface CreateAgentSessionOptions {
  cwd?: string;
  agentDir?: string;
  authStorage?: AuthStorage;
  modelRegistry?: ModelRegistry;
  model?: Model<any>;
  thinkingLevel?: ThinkingLevel;
  scopedModels?: Array<{ model: Model<any>; thinkingLevel?: ThinkingLevel }>;
  noTools?: "all" | "builtin";
  tools?: string[];
  customTools?: any[];
  resourceLoader?: any;
  sessionManager?: SessionManager;
  settingsManager?: SettingsManager;
  sessionStartEvent?: any;
}

/** Convenience entry: build services + session manager, then a session. */
export async function createAgentSession(
  options: CreateAgentSessionOptions = {},
): Promise<CreateAgentSessionResult> {
  const cwd = options.cwd ?? process.cwd();
  const agentDir = options.agentDir ?? getAgentDir();
  const authStorage = options.authStorage ?? getSharedAuth();
  const modelRegistry = options.modelRegistry ?? getSharedModelRegistry();
  const settingsManager = options.settingsManager ?? SettingsManager.create(cwd, agentDir);
  const resourceLoader = options.resourceLoader ?? new DefaultResourceLoader({ cwd, agentDir, settingsManager });
  await resourceLoader.reload();
  const services: AgentSessionServices = {
    cwd,
    agentDir,
    authStorage,
    settingsManager,
    modelRegistry,
    resourceLoader,
    diagnostics: [],
  };
  const sessionManager = options.sessionManager ?? SessionManager.create(cwd);
  return createAgentSessionFromServices({
    services,
    sessionManager,
    model: options.model,
    thinkingLevel: options.thinkingLevel,
    scopedModels: options.scopedModels,
    tools: options.tools,
    noTools: options.noTools,
    customTools: options.customTools,
    sessionStartEvent: options.sessionStartEvent,
  });
}

// ── Tool factory stubs (the core owns the built-in tools) ──

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: any;
}

export function defineTool(def: ToolDefinition): ToolDefinition {
  return def;
}

export function createReadTool(): any {
  return { name: "read_file" };
}
export function createBashTool(): any {
  return { name: "bash" };
}
export function createEditTool(): any {
  return { name: "edit" };
}
export function createWriteTool(): any {
  return { name: "write_file" };
}
export function createGrepTool(): any {
  return { name: "grep" };
}
export function createFindTool(): any {
  return { name: "glob" };
}
export function createLsTool(): any {
  return { name: "list_dir" };
}
export function createCodingTools(): any[] {
  return [createReadTool(), createBashTool(), createEditTool(), createWriteTool(), createGrepTool(), createFindTool(), createLsTool()];
}
export function createReadOnlyTools(): any[] {
  return [createReadTool(), createGrepTool(), createFindTool(), createLsTool()];
}
export function withFileMutationQueue<T>(fn: () => T): T {
  return fn();
}

export type { CreateAgentSessionServicesOptions, CreateAgentSessionFromServicesOptions };
export type { ImageContent, TextContent, ToolCall };
