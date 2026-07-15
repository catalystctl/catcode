// Compatibility probe — imports the EXACT symbol set pi-web uses from
// @earendil-works/pi-coding-agent + @earendil-works/pi-ai, but from this
// package, and exercises the call patterns from pi-web's `pi-agent.ts`.
// If this type-checks, pi-web can swap its imports to @catalyst-code/coding-agent.

import {
  AuthStorage,
  ModelRegistry,
  SessionManager,
  Theme,
  createAgentSessionServices,
  createAgentSessionFromServices,
  createAgentSessionRuntime,
  getAgentDir,
  initTheme,
  getSupportedThinkingLevels,
  loadSkills,
  SettingsManager,
  type AgentSessionServices,
  type AgentSession,
  type AgentSessionEvent,
  type AgentSessionRuntime,
  type CreateAgentSessionRuntimeFactory,
  type ExtensionUIContext,
  type ThemeColor,
  type Model,
  type SessionEntry,
  type SessionHeader,
  type SessionMessageEntry,
  type UserMessage,
  type AssistantMessage,
  type ToolResultMessage,
  type TextContent,
  type ImageContent,
  type ThinkingContent,
  type ToolCall,
} from "./dist/index.js";

// ── pi-agent.ts patterns ──
const authStorage = AuthStorage.create();
const modelRegistry = ModelRegistry.create(authStorage);
modelRegistry.refresh();
const available: Model[] = modelRegistry.getAvailable();
const found = modelRegistry.find("default", "umans-glm-5.2");

const cwd = process.cwd();
const agentDir = getAgentDir();
const sessionManager = SessionManager.create(cwd);
const sessionManagerOpen = SessionManager.open("some.jsonl");

const servicesP = createAgentSessionServices({ cwd, agentDir, authStorage, modelRegistry });

const factory: CreateAgentSessionRuntimeFactory = async ({ cwd, agentDir, sessionManager, sessionStartEvent }) => {
  const services = await createAgentSessionServices({ cwd, agentDir, authStorage, modelRegistry });
  return createAgentSessionFromServices({ services, sessionManager, sessionStartEvent });
};

async function main() {
  const runtime = await createAgentSessionRuntime(factory, { cwd, agentDir, sessionManager });
  runtime.setRebindSession((session) => Promise.resolve());
  runtime.setBeforeSessionInvalidate(() => {});

  const session: AgentSession = runtime.session;
  const unsub = session.subscribe((event: AgentSessionEvent) => {
    switch (event.type) {
      case "agent_start": break;
      case "agent_end": console.log(event.messages.length); break;
      case "message_update": {
        const d = event.assistantMessageEvent;
        if (d.type === "text_delta") process.stdout.write(d.delta);
        break;
      }
      case "tool_execution_start": console.log(event.toolName, event.args); break;
      case "tool_execution_end": console.log(event.isError); break;
      case "compaction_end": console.log(event.result?.tokensBefore); break;
      case "thinking_level_changed": console.log(event.level); break;
      case "session_info_changed": console.log(event.name); break;
      case "queue_update": console.log(event.steering.length); break;
      case "core_event": console.log(event.event.type); break;
    }
  });
  const unsubCore = session.subscribeCore((ev) => {
    console.log("core", ev.type);
  });
  void unsubCore;
  // model/thinking interception (pi-web monkey-patches extensionRunner.emit)
  const runner = session.extensionRunner;
  const original = runner.emit.bind(runner);
  runner.emit = async (event: any) => {
    if (event?.type === "model_select") console.log(event.model.provider, event.model.id);
    if (event?.type === "thinking_level_select") console.log(event.level);
    return original(event);
  };

  const uiContext: ExtensionUIContext = {
    select: async () => undefined,
    confirm: async () => true,
    input: async () => undefined,
    notify: () => {},
    onTerminalInput: () => () => {},
    setStatus: () => {},
    setWorkingMessage: () => {},
    setWorkingVisible: () => {},
    setWorkingIndicator: () => {},
    setHiddenThinkingLabel: () => {},
    setWidget: () => {},
    setFooter: () => {},
    setHeader: () => {},
    setTitle: () => {},
    custom: async () => { throw new Error("unsupported"); },
    pasteToEditor: () => {},
    setEditorText: () => {},
    getEditorText: () => "",
    editor: async () => undefined,
    addAutocompleteProvider: () => {},
    setEditorComponent: () => {},
    getEditorComponent: () => undefined,
    theme: new Theme({} as any, {} as any, "truecolor"),
    getAllThemes: () => [],
    getTheme: () => new Theme({} as any, {} as any, "truecolor"),
    setTheme: () => ({ success: false, error: "no themes" }),
    getToolsExpanded: () => false,
    setToolsExpanded: () => {},
  };
  await session.bindExtensions({ uiContext, onError: (e) => console.error(e.error) });

  if (found) await session.setModel(found);
  await session.prompt("hello", { source: "rpc", preflightResult: (ok) => console.log(ok) });
  await session.steer("actually do X");
  await session.followUp("then Y");
  await session.abort();
  await session.compact("keep recent");
  await session.executeBash("ls");
  const stats = await session.getSessionStats();
  console.log(stats.tokens.input);
  console.log(session.messages.length, session.isStreaming, session.sessionFile, session.sessionId, session.sessionName, session.model?.id, session.thinkingLevel, session.pendingMessageCount);
  console.log(session.getLastAssistantText());
  console.log(session.getUserMessagesForForking().length);
  session.setSessionName("demo");
  const htmlPath = await session.exportToHtml();
  console.log(htmlPath);

  await runtime.newSession({ withSession: async () => {} });
  await runtime.switchSession("other.jsonl", { withSession: async () => {} });
  await runtime.fork("entry-1", { position: "at", withSession: async () => {} });
  await runtime.dispose();
  unsub();

  // ── pi-sessions.ts patterns ──
  const mgr = SessionManager.open("x.jsonl");
  mgr.appendSessionInfo("name");
  const leaf = sessionManagerOpen.getLeafId();
  console.log(leaf);

  // ── pi-skills.ts patterns ──
  const sm = SettingsManager.create(cwd, agentDir);
  sm.setSkillPaths(["a"]);
  sm.setProjectSkillPaths(["b"]);
  const paths = sm.getSkillPaths();
  const loaded = loadSkills({ cwd, agentDir, skillPaths: paths, includeDefaults: true });
  console.log(loaded.skills.map((s) => s.name));

  // types are usable
  const u: UserMessage = { role: "user", content: "hi", timestamp: 0 };
  const t: TextContent = { type: "text", text: "x" };
  const tc: ToolCall = { type: "toolCall", id: "1", name: "bash", arguments: {} };
  void u; void t; void tc;
}

void main;
void main();

// ThemeColor index usage (pi-web builds Record<ThemeColor, ...> + Record<ThemeBg, ...>)
import type { ThemeBg } from "./dist/index.js";
const fg: Record<ThemeColor, string | number> = {} as any;
const bg: Record<ThemeBg, string | number> = {} as any;
const _t = new Theme(fg, bg, "truecolor");
initTheme();
