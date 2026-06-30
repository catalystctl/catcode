// @umans-harness/coding-agent — a pi-coding-agent-compatible TypeScript SDK
// that wraps the umans-harness core binary. Drop-in replacement for
// `@earendil-works/pi-coding-agent` (plus the `pi-ai` subset pi-web uses).
//
// This is a thin adapter: the agent loop, model inference, tool execution and
// session persistence all run in the Rust `core`; this package only spawns it,
// speaks its JSONL protocol, and exposes the PI-compatible API surface.

export { VERSION } from "./version.js";
export { getAgentDir, configDir, resolveCoreBinary } from "./config.js";

// pi-ai subset
export {
  getSupportedThinkingLevels,
  clampThinkingLevel,
  modelsAreEqual,
  getModel,
  getProviders,
  getModels,
  calculateCost,
} from "./ai.js";

// Shared types
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

// Events
export type {
  AgentSessionEvent,
  AgentSessionEventListener,
  PromptOptions,
  InputSource,
  ModelCycleResult,
} from "./events.js";

// Theme
export {
  Theme,
  initTheme,
  getHeadlessTheme,
  type ThemeColor,
  type ThemeBg,
  type ColorTable,
  type ColorBgTable,
  type ColorMode,
} from "./theme.js";

// Auth
export { AuthStorage, type RuntimeKeySink } from "./auth-storage.js";
export type {
  ApiKeyCredential,
  OAuthCredential,
  AuthCredential,
  AuthStatus,
  AuthStorageData,
  AuthStorageBackend,
} from "./auth-types.js";
export { FileAuthStorageBackend, InMemoryAuthStorageBackend } from "./auth-types.js";

// Model registry
export { ModelRegistry, getSharedAuth, getSharedModelRegistry, _resetSharedRegistries, type ResolvedRequestAuth } from "./model-registry.js";

// Session manager
export {
  SessionManager,
  CURRENT_SESSION_VERSION,
  type SessionHeader,
  type NewSessionOptions,
  type SessionEntryBase,
  type SessionMessageEntry,
  type ThinkingLevelChangeEntry,
  type ModelChangeEntry,
  type CompactionEntry,
  type BranchSummaryEntry,
  type CustomEntry,
  type LabelEntry,
  type SessionInfoEntry,
  type CustomMessageEntry,
  type SessionEntry,
  type FileEntry,
  type SessionContext,
  type SessionInfo,
  type SessionListProgress,
} from "./session-manager.js";

// Settings
export { SettingsManager, type CompactionSettings, type RetrySettings, type ImageSettings, type PackageSource, type TransportSetting } from "./settings-manager.js";

// Skills / resource loader
export {
  DefaultResourceLoader,
  loadSkills,
  loadSkillsFromDir,
  formatSkillsForPrompt,
  parseFrontmatter,
  stripFrontmatter,
  loadProjectContextFiles,
  type Skill,
  type SkillFrontmatter,
  type LoadSkillsResult,
  type LoadSkillsOptions,
  type LoadSkillsFromDirOptions,
  type PromptTemplate,
  type ResourceLoader,
  type ResourceDiagnostic,
  type SourceInfo,
  type DefaultResourceLoaderOptions,
} from "./resource-loader.js";

// Extensions
export {
  ExtensionRunner,
  type ExtensionUIContext,
  type ExtensionBindings,
  type ExtensionError,
  type ExtensionErrorListener,
  type RegisteredCommand,
  type ExtensionEvent,
  type ExtensionUIDialogOptions,
  type ExtensionWidgetOptions,
  type WidgetPlacement,
} from "./extension-runner.js";

// AgentSession + services + runtime + factories
export { AgentSession, type BashResult, type AgentSessionConfig, type AgentSessionServices, type AgentSessionRuntimeDiagnostic, type ReplacedSessionContext } from "./agent-session.js";
export {
  createAgentSessionServices,
  createAgentSessionFromServices,
  type CreateAgentSessionServicesOptions,
  type CreateAgentSessionFromServicesOptions,
  type CreateAgentSessionResult,
  type CreateAgentSessionRuntimeResult,
} from "./agent-session-services.js";
export {
  AgentSessionRuntime,
  createAgentSessionRuntime,
  type CreateAgentSessionRuntimeFactory,
  SessionImportFileNotFoundError,
  MissingSessionCwdError,
} from "./agent-session-runtime.js";
export {
  createAgentSession,
  defineTool,
  createReadTool,
  createBashTool,
  createEditTool,
  createWriteTool,
  createGrepTool,
  createFindTool,
  createLsTool,
  createCodingTools,
  createReadOnlyTools,
  withFileMutationQueue,
  type ToolDefinition,
  type CreateAgentSessionOptions,
} from "./sdk.js";

// Low-level process bridge (advanced usage / testing).
export { CoreProcess, type CoreProcessOptions, type ReadyPayload, type CoreEvent, type CoreCommand } from "./core-process.js";
