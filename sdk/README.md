# @catalyst-code/coding-agent

A **pi-coding-agent-compatible** TypeScript SDK for the
[catalyst-code](..) core. It is a thin adapter — it spawns the Rust `core`
binary, speaks its JSONL stdio protocol, and exposes the
[`@earendil-works/pi-coding-agent`](https://www.npmjs.com/package/@earendil-works/pi-coding-agent)
API surface (plus the `@earendil-works/pi-ai` subset that pi-web uses) so a
consumer written against the PI SDK runs unchanged against the Catalyst Code harness.

> **No agent-loop reimplementation.** Model inference, tool execution, session
> persistence and auto-compaction all run in the Rust `core`. This package only
> spawns it and translates the protocol — exactly the "wrapper" contract.

## Swap into pi-web

pi-web imports from two packages:

```ts
import { … } from "@earendil-works/pi-coding-agent";   // pi-agent.ts, pi-sessions.ts, pi-skills.ts
import { getSupportedThinkingLevels, type Model, … } from "@earendil-works/pi-ai"; // pi-agent.ts, pi-sessions.ts
```

This package re-exports everything pi-web uses from **both**, so the swap is a
single import change — point both specifiers at this package:

```bash
# in pi-web/packages/server
npm i ../../glm-5.2-ai-harnesss/sdk
```

```ts
// before
import { … } from "@earendil-works/pi-coding-agent";
import { getSupportedThinkingLevels, type Model } from "@earendil-works/pi-ai";

// after — one package
import { …, getSupportedThinkingLevels, type Model } from "@catalyst-code/coding-agent";
```

No other pi-web changes are required: `AuthStorage`, `ModelRegistry`,
`SessionManager`, `SettingsManager`, `createAgentSessionServices`,
`createAgentSessionFromServices`, `createAgentSessionRuntime`, `AgentSession`,
`AgentSessionEvent`, `ExtensionUIContext`, `Theme`/`initTheme`, `loadSkills`,
`getAgentDir`, the message types and the runtime call patterns all match.

> Verified: pi-web's `packages/server/src` (`pi-agent.ts`, `pi-sessions.ts`,
> `pi-skills.ts`) type-checks unchanged when both PI specifiers are aliased to
> this package (see `compat-probe.ts` for the exercised surface).

## Quick start

```ts
import {
  AuthStorage, ModelRegistry, SessionManager, Theme, initTheme, getAgentDir,
  createAgentSessionServices, createAgentSessionFromServices, createAgentSessionRuntime,
} from "@catalyst-code/coding-agent";

const cwd = process.cwd();
const authStorage = AuthStorage.create();
const modelRegistry = ModelRegistry.create(authStorage);
initTheme();

const factory = async ({ cwd, agentDir, sessionManager, sessionStartEvent }) => {
  const services = await createAgentSessionServices({ cwd, agentDir, authStorage, modelRegistry });
  return createAgentSessionFromServices({ services, sessionManager, sessionStartEvent });
};

const runtime = await createAgentSessionRuntime(factory, {
  cwd,
  agentDir: getAgentDir(),
  sessionManager: SessionManager.create(cwd),
});

runtime.session.subscribe((event) => {
  if (event.type === "message_update" && event.assistantMessageEvent.type === "text_delta") {
    process.stdout.write(event.assistantMessageEvent.delta);
  }
});

await runtime.session.prompt("explain this repo");
await runtime.dispose();
```

### Core binary resolution

`AgentSession` spawns `catcode-core` (the Rust binary). It is resolved, in order:

1. `CATCODE_CORE` env var (absolute path — use this in production).
2. Dev build at `core/target/release/catcode-core` / `…/core` (repo-relative).
3. `catcode-core` on `PATH` (installed layout).

API keys are not auto-injected by the SDK. Paste a key via `/login` or complete
this app's OAuth flow; you can also push a runtime key via
`authStorage.setRuntimeApiKey(provider, key)` (forwarded as the `set_key`
command). For a provider explicitly configured with `api_key_env`, the core
reads the named env var at request time.

## Protocol mapping

The core speaks newline-delimited JSON over stdin/stdout. `AgentSession`
translates it into the PI event stream:

| Harness event        | PI `AgentSessionEvent`(s)                                              |
|----------------------|------------------------------------------------------------------------|
| `ready` / `models`   | (populates the shared `ModelRegistry`; no PI event)                    |
| `delta`              | `agent_start` → `turn_start` → `message_start` → `message_update` (`text_delta`) |
| `thinking`           | `message_update` (`thinking_delta`)                                   |
| `tool_call`          | `message_update` (`toolcall_end`) → `message_end` → `tool_execution_start` |
| `tool_result`        | `tool_execution_end` (+ `toolResultMessage`)                           |
| (next model request) | `turn_end` → `turn_start` → `message_start` (loop)                     |
| `done`               | `turn_end` → `agent_end`                                              |
| `aborted`            | `message_end`? → `turn_end` → `agent_end`                            |
| `http_retry`         | `auto_retry_start` / `auto_retry_end`                                  |
| `compacted`          | `compaction_start` → `compaction_end`                                 |
| `approval_request`  | routed through bound `ExtensionUIContext.confirm` → `approve`          |
| `intercom_message`   | routed through `ExtensionUIContext.input` → `intercom_reply`           |
| `history`            | rebuilds `session.messages` from the resumed transcript               |

Commands sent to the core: `init`, `send`/`steer` (with `model` +
`reasoning_effort`), `abort`, `compact`, `set_model`/`set_provider`/`set_key`,
`stats`, `new_session`/`load_session`, `approve`, `intercom_reply`.

## Package layout

```
src/
  config.ts                 # getAgentDir, configDir, core-binary resolution
  types.ts                  # Model, messages, AgentState, … (PI-compatible)
  ai.ts                     # pi-ai subset: getSupportedThinkingLevels, getModel, …
  events.ts                 # AgentSessionEvent union + PromptOptions
  theme.ts                  # Theme, initTheme, ThemeColor, ThemeBg
  auth-storage.ts           # AuthStorage (+ runtime-key forwarding)
  model-registry.ts         # ModelRegistry + shared singletons
  session-manager.ts        # SessionManager (flat-history; PI entry-tree types)
  settings-manager.ts       # SettingsManager (skill/extension/prompt paths)
  resource-loader.ts        # DefaultResourceLoader, loadSkills, Skill
  extension-runner.ts       # ExtensionRunner (mutable emit), ExtensionUIContext
  core-process.ts           # CoreProcess — spawns catcode-core, JSONL I/O
  agent-session.ts          # AgentSession — harness→PI event translation
  agent-session-services.ts # createAgentSessionServices / FromServices
  agent-session-runtime.ts  # AgentSessionRuntime, createAgentSessionRuntime
  sdk.ts                    # createAgentSession + tool-factory stubs
  index.ts                  # barrel
```

## Known adaptation points

These are inherent differences between the Catalyst Code harness and the PI session
model. They compile and run, but behave slightly differently — documented so
pi-web can adapt if exact parity is needed:

- **Branch tree / fork**: the harness keeps a *flat* transcript (no parent/child
  entry ids). `SessionManager.getLeafId()`/`getBranch()`/`getChildren()` return
  synthetic/flat results, and `AgentSessionRuntime.fork()` starts a fresh session
  rather than a true branch. `switchSession`/`newSession` reuse the *same* core
  process (the harness repoints to a different session file) but still invoke the
  `rebindSession` / `beforeSessionInvalidate` / `withSession` hooks.
- **Session file format**: the core writes OpenAI-style JSONL, not PI's
  entry-tree. `SessionManager` exposes the PI entry types for compatibility, but
  routes that parse raw session files directly (e.g. pi-web's `pi-sessions.ts`
  history export) may need to read via `AgentSession.messages` /
  `getSessionStats()` instead.
- **`executeBash`** runs via the core `user_bash` command (same sandbox/denylist
  as the agent `bash` tool). Used for PI-compatible `!cmd` / `!!cmd` bang
  commands; `!!` sets `excludeFromContext` so output is shown but not added to
  the model transcript.
- **`exportToHtml`** produces a minimal transcript HTML.
- **Model discovery**: models arrive from the core's `ready`/`models` events
  (dynamic), so `ModelRegistry.getAvailable()` is empty until the first core
  starts; `setModel` applies per-turn via `send`.

## Scripts

```bash
npm run build       # tsc → dist/
npm run typecheck   # tsc --noEmit
```
