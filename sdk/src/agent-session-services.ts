// Service + session factories — mirrors `pi-coding-agent`'s
// `core/agent-session-services.ts` / `core/sdk.ts` factory split that pi-web uses.

import { getAgentDir } from "./config.js";
import { AuthStorage } from "./auth-storage.js";
import { ModelRegistry, getSharedAuth, getSharedModelRegistry } from "./model-registry.js";
import { SettingsManager } from "./settings-manager.js";
import { DefaultResourceLoader, type ResourceLoader } from "./resource-loader.js";
import {
  AgentSession,
  type AgentSessionServices,
  type AgentSessionRuntimeDiagnostic,
} from "./agent-session.js";
import type { Model, ThinkingLevel } from "./types.js";
import type { SessionManager } from "./session-manager.js";

export type { AgentSessionServices, AgentSessionRuntimeDiagnostic } from "./agent-session.js";

export interface CreateAgentSessionServicesOptions {
  cwd: string;
  agentDir?: string;
  authStorage?: AuthStorage;
  settingsManager?: SettingsManager;
  modelRegistry?: ModelRegistry;
  extensionFlagValues?: Map<string, boolean | string>;
  resourceLoaderOptions?: Record<string, unknown>;
}

export interface CreateAgentSessionFromServicesOptions {
  services: AgentSessionServices;
  sessionManager: SessionManager;
  sessionStartEvent?: any;
  model?: Model<any>;
  thinkingLevel?: ThinkingLevel;
  scopedModels?: Array<{ model: Model<any>; thinkingLevel?: ThinkingLevel }>;
  tools?: string[];
  noTools?: "all" | "builtin";
  customTools?: any[];
}

export interface CreateAgentSessionResult {
  session: AgentSession;
  extensionsResult: { extensions: any[]; diagnostics: any[] };
  modelFallbackMessage?: string;
}

export interface CreateAgentSessionRuntimeResult extends CreateAgentSessionResult {
  services: AgentSessionServices;
  diagnostics: AgentSessionRuntimeDiagnostic[];
}

/** Build cwd-bound services (auth/registry/settings/resource loader). Does NOT
 *  spawn a core — that happens when a session is created from these services. */
export async function createAgentSessionServices(
  options: CreateAgentSessionServicesOptions,
): Promise<AgentSessionServices> {
  const cwd = options.cwd;
  const agentDir = options.agentDir ?? getAgentDir();
  const authStorage = options.authStorage ?? getSharedAuth();
  const modelRegistry = options.modelRegistry ?? getSharedModelRegistry();
  const settingsManager = options.settingsManager ?? SettingsManager.create(cwd, agentDir);
  const resourceLoader: ResourceLoader =
    new DefaultResourceLoader({ cwd, agentDir, settingsManager });
  await resourceLoader.reload();
  const diagnostics: AgentSessionRuntimeDiagnostic[] = [];
  return { cwd, agentDir, authStorage, settingsManager, modelRegistry, resourceLoader, diagnostics };
}

/** Construct an `AgentSession` from already-built services. Spawns the core,
 *  awaits `ready`, populates the model registry, and returns the bound session. */
export async function createAgentSessionFromServices(
  options: CreateAgentSessionFromServicesOptions,
): Promise<CreateAgentSessionRuntimeResult> {
  const { services, sessionManager, model, thinkingLevel, tools, customTools, sessionStartEvent } = options;
  const session = new AgentSession({
    services,
    sessionManager,
    model,
    thinkingLevel,
    tools,
    customTools,
    sessionStartEvent,
  });
  // Resolve a runtime API key for the model's provider (if any) to push to the core.
  const provider = model?.provider ?? services.modelRegistry.getProviderDisplayName("default");
  const apiKey = await services.authStorage.getApiKey(provider);
  await session.init(model, model?.provider, apiKey);
  return {
    session,
    extensionsResult: { extensions: [], diagnostics: [] },
    services,
    diagnostics: services.diagnostics,
  };
}
