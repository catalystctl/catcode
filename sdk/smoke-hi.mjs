// High-level API smoke: build services + runtime + session the way pi-web does,
// subscribe, confirm the shared registry is populated from `ready`, then dispose.
import {
  AuthStorage, ModelRegistry, SessionManager,
  createAgentSessionServices, createAgentSessionFromServices, createAgentSessionRuntime,
  getAgentDir, getSharedModelRegistry,
} from "./dist/index.js";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.UMANS_CORE = process.env.UMANS_CORE || join(process.cwd(), "core", "target", "release", "core");
const cwd = mkdtempSync(join(tmpdir(), "umans-sdk-hi-"));

const authStorage = AuthStorage.create();
const modelRegistry = ModelRegistry.create(authStorage);
modelRegistry.refresh();

const factory = async ({ cwd, agentDir, sessionManager, sessionStartEvent }) => {
  const services = await createAgentSessionServices({ cwd, agentDir, authStorage, modelRegistry });
  return createAgentSessionFromServices({ services, sessionManager, sessionStartEvent });
};

const runtime = await createAgentSessionRuntime(factory, {
  cwd,
  agentDir: getAgentDir(),
  sessionManager: SessionManager.create(cwd),
});

let rebindCount = 0;
runtime.setRebindSession(() => { rebindCount++; });
runtime.setBeforeSessionInvalidate(() => {});

const session = runtime.session;
const events = [];
const unsub = session.subscribe((e) => events.push(e.type));

// monkey-patch extensionRunner.emit like pi-web
const orig = session.extensionRunner.emit.bind(session.extensionRunner);
session.extensionRunner.emit = async (ev) => { events.push(`ext:${ev.type}`); return orig(ev); };

await session.bindExtensions({ uiContext: undefined, onError: () => {} });

console.log("models in passed registry:", modelRegistry.getAvailable().length);
console.log("models in shared singleton:", getSharedModelRegistry().getAvailable().length);
console.log("session.model:", session.model?.id ?? "(default)");
console.log("session.sessionId:", session.sessionId);
console.log("session.isStreaming:", session.isStreaming);
console.log("messages:", session.messages.length);

await session.dispose();
unsub();
console.log("events seen:", events);
console.log("rebind callbacks:", rebindCount);
process.exit(0);
