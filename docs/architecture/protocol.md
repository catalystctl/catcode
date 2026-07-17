# Wire Protocol Reference

Catalyst Code components communicate over **newline-delimited JSON on stdio**.
Commands flow from the frontend (TUI/web/SDK) to the core on stdin; events
flow from the core to the frontend on stdout.

This document describes every command and event type defined in the wire
protocol. Source: `core/src/protocol.rs` (/core/src/protocol.rs).

---

## Table of Contents

- [Transport Conventions](#transport-conventions)
- [Common Types](#common-types)
- [Commands (Frontend → Core)](#commands-frontend--core)
- [Events (Core → Frontend)](#events-core--frontend)
- [Typical Session Flow](#typical-session-flow)
- [Error Handling](#error-handling)

---

## Transport Conventions

**Encoding:** UTF-8 JSON, exactly one JSON object per line (`\n`).

**Direction:**
- `stdin` — Frontend writes [`Command`](#commands-frontend--core) objects.
- `stdout` — Core writes [`Event`](#events-core--frontend) objects.

**Tagged union:** Every message carries a `"type"` field that identifies the
variant. Commands use `#[serde(tag = "type")]`; events use an explicit
`"type"` field in the JSON.

**Buffering:** Frontends should either flush stdin after each command or set
line-buffered mode. The core reads stdin line-by-line with a read buffer.

**Thread safety:** The core's `emit()` function locks stdout per-line.

---

## Common Types

### `ModelInfo`

Returned in `ready` and `models` events.

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Model identifier (e.g. `"glm-5.2"`) |
| `name` | string | Human-readable name |
| `reasoning` | boolean | Whether the model supports reasoning/thinking |
| `context_window` | integer | Context window in tokens |
| `max_tokens` | integer | Maximum output tokens |
| `thinking_levels` | string[] | Reasoning effort levels (`["low","medium","high"]`) |
| `vision` | boolean | Whether the model accepts image inputs |
| `provider` | string | Provider name that owns this model (for multi-provider per-turn routing) |

### `Event`

Every event is an object with a `"type"` field plus additional data fields:

```json
{"type": "event_name", "field1": "value1", "field2": "value2"}
```

Construction in the codebase uses the builder pattern:

```rust
Event::new("event_name")
    .with("field", json!(value))
```

Source: `Event` (/core/src/protocol.rs), line ~385.

---

## Commands (Frontend → Core)

All commands are deserialized via `#[serde(tag = "type")]`. The `type` field
determines which variant is parsed.

### Initialization and Lifecycle

#### `init`

Initialize a new core session. Sent once at startup. The core responds with a
[`ready`](#ready) event containing the initial model list and config.

```json
{"type": "init"}
```

No extra fields.

#### `reset`

Full reset: clear the in-memory conversation **and** the session file. Re-emits
a `reset` event.

```json
{"type": "reset"}
```

#### `clear`

Clear only the in-memory conversation. The session file is preserved so a
restart can resume.

```json
{"type": "clear"}
```

#### `undo`

Drop the last turn (user prompt + assistant reply + tool calls/results). Also
restores the latest auto filesystem checkpoint when one exists.

```json
{"type": "undo"}
```

#### `abort`

Abort the currently running turn **and** drop any queued prompt. The core emits
`aborted` then `done`.

```json
{"type": "abort"}
```

#### `clear_queue`

Drop a queued follow-up/steer prompt **without** aborting the running turn.
Useful for the TUI's Esc key to cancel just the queued message.

```json
{"type": "clear_queue"}
```

#### `stats`

Request a session statistics summary. Returns a [`stats`](#stats) event.

```json
{"type": "stats"}
```

#### `context`

Request a token-usage breakdown of the current context. Returns a
[`context_breakdown`](#context_breakdown) event with total tokens, context
window usage, per-role buckets, and the top token consumers.

```json
{"type": "context"}
```

#### `usage`

Request provider plan/rate-limit usage for the currently selected model. Each
provider implements its own stats. Returns a [`usage`](#usage) event.

```json
{"type": "usage", "model": "glm-5.2"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | no | Override the last-used model for routing |

---

### Conversation

#### `send`

Send a user prompt and start an assistant turn. This is the primary way to
talk to the model.

```json
{
  "type": "send",
  "prompt": "Write a Rust function that reads a file",
  "model": "glm-5.2",
  "reasoning_effort": "high",
  "images": ["data:image/png;base64,..."]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `prompt` | string | yes | The user's message |
| `model` | string | yes | Model ID to use for this turn |
| `reasoning_effort` | string | no | Reasoning/thinking effort level (e.g. `"low"`, `"high"`) |
| `images` | string[] | no | Image data URLs (`data:image/...;base64,...`) or absolute file paths |

#### `steer`

Interrupt an in-flight turn and redirect it with a new prompt. If no turn is
running, behaves like `send`.

```json
{
  "type": "steer",
  "prompt": "Actually, use async instead",
  "model": "glm-5.2",
  "reasoning_effort": "high"
}
```

Same fields as `send`. Emits a `steer` event.

---

### Model and Provider Management

#### `set_key`

Apply an API key to a provider at runtime. Overrides both the config file
`api_key` and `api_key_env` for that provider.

```json
{"type": "set_key", "api_key": "sk-...", "provider": "umans"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `api_key` | string | yes | The API key |
| `provider` | string | no | Provider name; omitted = apply to currently active provider ("default" slot) |

Emits `authed` event. The provider's models are refreshed.

#### `set_search_key`

Set or clear a search-tool API key (Exa / Tavily) for `web_search`.

```json
{"type": "set_search_key", "provider": "exa", "api_key": "sk-..."}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `provider` | string | yes | `"exa"` or `"tavily"` |
| `api_key` | string | yes | Empty string clears the stored key |

Persisted to `config.json` `search_keys`. Emits `search_key_set`.

#### `set_provider`

Switch the active model provider at runtime.

```json
{"type": "set_provider", "name": "opencode-go"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Provider name from config; unknown names are ignored |

Re-resolves base URL, key, and wire protocol. Re-discovers models. Emits
`provider_changed` and (when keyed) `authed`.

#### `list_provider_presets`

List the built-in provider presets (Umans, OpenCode Go, OpenRouter) plus plugin
OAuth providers. Emits `provider_presets`.

```json
{"type": "list_provider_presets"}
```

#### `login`

Log in to a first-party provider preset with an API key.

```json
{"type": "login", "preset": "umans", "api_key": "sk-..."}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `preset` | string | yes | Preset ID (`"umans"`, `"opencode-go"`, `"openrouter"`) |
| `api_key` | string | no | API key. Required when the preset has no `api_key_env` set. |

Creates the provider config, sets its API key, persists, re-aggregates models.
Multiple providers can be logged in simultaneously. Emits `provider_changed`,
`authed`, and `info` events.

#### `logout`

Log out of a provider: drop its runtime key and remove it from the configured
providers.

```json
{"type": "logout", "provider": "umans"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `provider` | string | yes | Provider name to log out |

Emits `info`, `provider_changed`, and `authed` events. No-op with `error`
event when not logged in.

#### `login_oauth`

Start plugin-based OAuth login for a plugin-declared `provider_id`.

```json
{"type": "login_oauth", "preset": "chatgpt"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `preset` | string | yes | Provider ID matching a plugin's `oauth.provider_id` |

Emits `oauth_prompt` events for the user to authorize, then process the code.

#### `oauth_code`

Complete a pending plugin OAuth login by submitting the authorization/user code
from a prior `oauth_prompt`.

```json
{"type": "oauth_code", "code": "...ABC123..."}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `code` | string | yes | The authorization code obtained from the OAuth provider |

#### `set_approval`

Change the approval mode at runtime.

```json
{"type": "set_approval", "mode": "always"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `mode` | string | yes | `"never"`, `"destructive"`, or `"always"` |

Emits `approval_changed`.

#### `set_config`

Change a runtime config knob.

```json
{"type": "set_config", "key": "bash_timeout_secs", "value": 60}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | string | yes | Recognized keys: `bash_timeout_secs`, `sandbox`, `auto_compact` |
| `value` | any | yes | Coerced from the JSON type |

Emits `config_changed`.

#### `get_vision_config`

Get the current vision-handoff configuration. Emits `vision_config`.

```json
{"type": "get_vision_config"}
```

#### `set_vision_config`

Set the vision-handoff configuration and persist to `.catalyst-code/vision.json`.

```json
{
  "type": "set_vision_config",
  "enabled": true,
  "vision_model": "glm-5.2-vision",
  "vision_models": ["glm-5.2-vision", "gpt-4o"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `enabled` | boolean | yes | Enable vision handoff (default: true when absent) |
| `vision_model` | string | no | Preferred handoff target; empty = cheapest same-provider |
| `vision_models` | string[] | no | Curated list of vision-capable models |

Emits `vision_config`.

---

### Session Management

#### `list_sessions`

List available session files. Emits `sessions`.

```json
{"type": "list_sessions"}
```

#### `load_session`

Load a specific session file, replacing the current conversation.

```json
{"type": "load_session", "path": "sessions/2026-07-15.jsonl"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Path to the session file |

#### `rename_session`

Set a human-readable title for a saved session.

```json
{"type": "rename_session", "path": "sessions/2026-07-15.jsonl", "title": "Auth refactor"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Path to the session file |
| `title` | string | yes | New title string |

#### `delete_session`

Delete a non-active saved session and its metadata.

```json
{"type": "delete_session", "path": "sessions/old-session.jsonl"}
```

#### `pin_session`

Pin or unpin a session in the picker.

```json
{"type": "pin_session", "path": "sessions/important.jsonl", "pinned": true}
```

#### `new_session`

Start a fresh session file in the same project directory. An optional `path` (a
filename, not a full path) overrides the auto-generated name.

```json
{"type": "new_session", "path": "refactor-auth.jsonl"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | no | Custom session filename |

---

### Checkpoints

#### `create_checkpoint`

Create a hybrid filesystem checkpoint (git stash ref or file snapshot).

```json
{"type": "create_checkpoint", "label": "before-auth-refactor", "paths": ["src/auth.rs"]}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `label` | string | no | Human-readable label |
| `paths` | string[] | no | Specific paths to snapshot (omitted = snapshot all) |

#### `list_checkpoints`

List known checkpoints for this session/workspace. Emits `checkpoints`.

```json
{"type": "list_checkpoints"}
```

#### `restore_checkpoint`

Restore a checkpoint by id (filesystem only; conversation unchanged).

```json
{"type": "restore_checkpoint", "id": "ck-abc123"}
```

---

### Compaction

#### `compact`

Force a context compaction now, regardless of the threshold.

```json
{"type": "compact", "instructions": "Focus on code samples and API usage"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `instructions` | string | no | Override `compact_instructions` for this call only |

Emits `compacting`, then `compacted`, then optionally `error` if still over
limit.

---

### Memory

#### `save_memory`

Save a durable memory note (persisted across sessions). Core generates a name,
saves it, and refreshes system-prompt injection. Emits `memory_saved`.

```json
{"type": "save_memory", "text": "User prefers async Rust patterns", "tags": ["rust", "async"], "scope": "workspace"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `text` | string | yes | Memory content |
| `tags` | string[] | no | Optional tags for retrieval |
| `scope` | string | no | `"workspace"` (default) or `"global"` |

#### `list_memory`

List saved memories (both scopes). Emits `memory_list`.

```json
{"type": "list_memory"}
```

#### `forget_memory`

Delete a memory by its id. Emits `memory_saved` describing the outcome.

```json
{"type": "forget_memory", "id": "my-memory-slug", "scope": "workspace"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Memory id (slug or name) |
| `scope` | string | no | Scope to search; omitted searches both |

#### `refresh_memory`

Ask core to re-inject memories into the system prompt (called after saving a
memory externally or to force a refresh).

```json
{"type": "refresh_memory"}
```

---

### Plugin Lifecycle

#### `install_plugin`

Install a plugin from a local directory or a GitHub Release.

```json
{"type": "install_plugin", "path": "owner/repo@v1.0.0", "scope": "global"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Local directory, `owner/repo[@tag]`, or full GitHub URL |
| `scope` | string | no | `"global"` (default — `~/.catalyst-code/plugins`, every workspace) or `"workspace"` |

#### `remove_plugin`

Remove a named plugin.

```json
{"type": "remove_plugin", "name": "my-plugin"}
```

#### `enable_plugin`

Re-enable a disabled plugin.

```json
{"type": "enable_plugin", "name": "my-plugin"}
```

#### `disable_plugin`

Disable a plugin without removing it.

```json
{"type": "disable_plugin", "name": "my-plugin"}
```

#### `list_plugins`

List all installed plugins with their enabled/disabled status. Emits
`plugin_list`.

```json
{"type": "list_plugins"}
```

#### `reload_plugins`

Re-scan plugin directories, preserving enabled/disabled flags.

```json
{"type": "reload_plugins"}
```

#### `plugin_command`

Run a plugin-declared slash command by name.

```json
{"type": "plugin_command", "name": "my-command", "args": "--flag value"}
```

#### `list_plugin_commands`

List slash commands declared by enabled plugins.

```json
{"type": "list_plugin_commands"}
```

---

### Agents, Skills, and Goals

#### `list_agents`

Re-discover available subagents (builtin + user + project) and emit an `agents`
event.

```json
{"type": "list_agents"}
```

#### `list_skills`

List discoverable skills (project then user scope). Emits a `skills` event with
each skill's name, description, and location.

```json
{"type": "list_skills"}
```

#### `apply_skill`

Invoke a skill by name: the core reads the matching `SKILL.md`, builds a
prompt, and runs a normal assistant turn.

```json
{"type": "apply_skill", "name": "repository-documentation-factory", "task": "document the CLI", "model": "glm-5.2"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Skill name (resolved project > user scope) |
| `task` | string | no | Optional follow-up appended to the skill instructions |
| `model` | string | yes | Model to use |
| `reasoning_effort` | string | no | Reasoning effort level |

#### `start_goal`

Start goal mode: plan then deploy subagents under the given configuration.

```json
{
  "type": "start_goal",
  "goal": "Refactor the auth module",
  "concurrency": 4,
  "max_tasks": 12,
  "allowed_models": ["glm-5.2", "claude-sonnet-4"],
  "auto_deploy": true,
  "planner_model": "glm-5.2",
  "worker_model": "claude-sonnet-4",
  "model_concurrency": {"glm-5.2": 2, "claude-sonnet-4": 4}
}
```

Fields: `goal` (required), `concurrency`, `max_tasks`, `allowed_models`,
`allowed_providers`, `auto_deploy`, `planner_model`, `worker_model`,
`reviewer_model`, `model_concurrency`, `model`, `reasoning_effort`.

#### `cancel_goal`

Cancel the active goal (interrupts planning/deploy runs).

```json
{"type": "cancel_goal"}
```

#### `goal_status`

Re-emit the current `goal_state` (+ `goal_plan` if present).

```json
{"type": "goal_status"}
```

#### `approve_goal_plan`

Approve a plan that is waiting at `plan_ready` (when `auto_deploy` was false).

```json
{"type": "approve_goal_plan"}
```

#### `revise_goal`

Re-enter planning with user feedback (from `plan_ready` / `failed`).

```json
{"type": "revise_goal", "feedback": "Add validation checks", "model": "glm-5.2"}
```

---

### Approval and Interaction

#### `approve`

Respond to a pending approval request.

```json
{"type": "approve", "request_id": "req-123", "decision": "yes", "pattern": "//src/**"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_id` | string | yes | ID from the `approval_request` event |
| `decision` | string | yes | `"yes"`, `"no"`, `"always"`, `"allow_session"`, `"allow_pattern"` |
| `pattern` | string | no | Path/command glob for `allow_pattern`; defaults to the tool's path arg |

#### `ask_reply`

Reply to a pending `ask_request` (the `ask` tool).

```json
{"type": "ask_reply", "request_id": "ask-456", "answers": {"q1": "Use async-std", "q2": "Yes"}}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_id` | string | yes | ID from the `ask_request` event |
| `answers` | object/null | yes | Map of question id → answer string, or `null` to skip questions |

#### `sudo_reply`

Reply to a pending `sudo_request` (a bash command that invokes `sudo`).

```json
{"type": "sudo_reply", "request_id": "sudo-789", "approved": true, "password": "hunter2"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_id` | string | yes | ID from the `sudo_request` event |
| `approved` | boolean | no | `true` to run the command with `sudo -S` |
| `password` | string | no | Password fed to `sudo -S` stdin (used once, not stored) |

#### `intercom_reply`

Reply to a subagent's `contact_supervisor` need_decision ask.

```json
{"type": "intercom_reply", "request_id": "inter-111", "reply": "Use the existing adapter"}
```

---

### User Commands

#### `user_bash`

User-initiated bash from the composer (`!cmd` / `!!cmd`), PI-compatible. Runs
in the workspace with the same sandbox/denylist as the agent `bash` tool.

```json
{"type": "user_bash", "command": "git status", "exclude_from_context": false}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `command` | string | yes | Shell command to run |
| `exclude_from_context` | boolean | no | `true` for `!!cmd` — shows output but does not add to LLM context |

Emits `bash_execution` events for the UI.

---

## Events (Core → Frontend)

All events are JSON objects with a `"type"` string field plus additional data.

### Lifecycle

#### `ready`

Emitted once after `init` completes. Carries the full initial state.

```json
{
  "type": "ready",
  "models": [ /* ModelInfo[] */ ],
  "authed": false,
  "workspace": "/home/user/project",
  "approval": "destructive",
  "base_url": "https://api.code.umans.ai/v1",
  "provider": "umans",
  "provider_kind": "openai",
  "has_key": false,
  "has_vision_config": false,
  "idle_timeout_secs": 120,
  "bash_timeout_secs": 30,
  "auto_compact": true,
  "context_compact_at": 0.9,
  "context_digest_at": 0.7,
  "sandbox": "none",
  "resumed_messages": 0,
  "plugins": [],
  "plugins_skipped": []
}
```

#### `reset`

Emitted after `reset` or `clear`. No extra fields.

```json
{"type": "reset"}
```

---

### Turn Streaming

#### `delta`

A text delta from the model's response stream. Multiple `delta` events are
emitted per turn, one per chunk.

```json
{"type": "delta", "text": "async fn read_file"}
```

#### `thinking`

A thinking/reasoning text delta from the model. Interleaved with `delta` events.

```json
{"type": "thinking", "text": "I need to understand the file structure first..."}
```

#### `tool_call`

A tool call requested by the model.

```json
{
  "type": "tool_call",
  "id": "call_abc123",
  "name": "bash",
  "args": "{\"command\": \"ls -la\"}"
}
```

#### `tool_result`

The result of a tool execution.

```json
{
  "type": "tool_result",
  "id": "call_abc123",
  "ok": true,
  "name": "bash",
  "output": "total 24\ndrwxrwxr-x ..."
}
```

Fields: `id`, `ok` (boolean), `name`, `output` (truncated to 32 KiB for bash,
bounded for other tools). On error: `ok: false` with error message in `output`.

#### `aborted`

Emitted when a turn is aborted (via `abort` command, user denial, or internal
error). Usually followed by `done`.

```json
{"type": "aborted"}
```

#### `done`

Emitted when a turn finishes (successfully, aborted, or errored).

```json
{"type": "done"}
```

#### `steer`

Emitted when a `steer` command is received, before the turn is redirected.

```json
{"type": "steer", "prompt": "Actually, use async instead"}
```

---

### Provider and Model Events

#### `models`

Full model list update. Emitted after login, logout, `set_provider`, and
`refresh_models`.

```json
{"type": "models", "models": [ /* ModelInfo[] */ ]}
```

#### `provider_presets`

List of available provider presets (built-in + plugin OAuth).

```json
{
  "type": "provider_presets",
  "presets": [
    {"id": "umans", "label": "Umans (GLM-5.2)", "kind": "openai", "has_key": true, "can_oauth": false},
    {"id": "opencode-go", "label": "OpenCode Go", "kind": "openai", "has_key": false, "can_oauth": false}
  ]
}
```

#### `provider_changed`

Emitted when the active provider switches (login, logout, `set_provider`).

```json
{"type": "provider_changed", "provider": "umans", "kind": "openai", "base_url": "https://api.code.umans.ai/v1", "has_key": true}
```

#### `authed`

Authentication status change. Emitted on login, logout, and `set_key`.

```json
{"type": "authed", "ok": true, "provider": "umans"}
```

#### `search_key_set`

Search key change confirmation.

```json
{"type": "search_key_set", "provider": "exa", "has_key": true}
```

---

### OAuth Events

#### `oauth_prompt`

Emitted during plugin OAuth login to ask the user to authorize and enter a code.

```json
{
  "type": "oauth_prompt",
  "url": "https://provider.com/authorize?code=ABC",
  "code": "ABC123",
  "message": "Open this URL in your browser, then paste the code"
}
```

---

### Approval Events

#### `approval_request`

A tool call is waiting for human approval.

```json
{
  "type": "approval_request",
  "request_id": "req-123",
  "tool": "write_file",
  "args": "{\"path\": \"src/main.rs\", \"content\": \"...\"}",
  "diff": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,5 @@..."
}
```

`diff` is present only for `write_file`, `edit`, and `patch` (generated before
the tool executes, so the user sees what will change).

#### `approval_changed`

Emitted after `set_approval`.

```json
{"type": "approval_changed", "mode": "always"}
```

#### `config_changed`

Emitted after `set_config`.

```json
{"type": "config_changed", "key": "bash_timeout_secs", "value": 60}
```

---

### User Interaction Events

#### `ask_request`

The `ask` tool is waiting for user input.

```json
{
  "type": "ask_request",
  "request_id": "ask-456",
  "questions": [
    {"id": "q1", "type": "text", "label": "Which library should I use?"},
    {"id": "q2", "type": "select", "label": "Async runtime?", "options": ["tokio", "async-std", "smol"]}
  ]
}
```

#### `sudo_request`

A bash command that invokes `sudo` is waiting for a password.

```json
{
  "type": "sudo_request",
  "request_id": "sudo-789",
  "command": "apt install -y postgresql"
}
```

#### `intercom_message`

A subagent has sent a `contact_supervisor` message to the orchestrator.

```json
{
  "type": "intercom_message",
  "request_id": "inter-111",
  "from_agent": "worker-1",
  "message": "Need decision: use adapter A or B?"
}
```

---

### Session and Metrics Events

#### `sessions`

Emitted after `list_sessions`. Contains available session files.

```json
{
  "type": "sessions",
  "sessions": [
    {"path": "sessions/2026-07-15.jsonl", "title": "Auth refactor", "pinned": true, "mtime": "..."}
  ]
}
```

#### `history`

Emitted after `undo`. Contains the truncated conversation history.

```json
{"type": "history", "messages": [ /* ... */ ], "tokens_in": 1234}
```

#### `stats`

Emitted after `stats` command.

```json
{
  "type": "stats",
  "tokens_in": 15000,
  "tokens_out": 3200,
  "total_in": 15000,
  "total_out": 3200,
  "auto_compactions": 2,
  "human_corrections": 0,
  "subagent_calls": 5,
  "tool_calls": 47,
  "bash_calls": 12,
  "edit_calls": 8
}
```

#### `context_breakdown`

Emitted after `context` command. Shows token usage breakdown.

```json
{
  "type": "context_breakdown",
  "total": 15000,
  "context_window": 128000,
  "pct": 11.7,
  "roles": {"system": 5000, "user": 6000, "assistant": 4000},
  "top_consumers": [
    {"role": "user", "content_preview": "Please review this large file...", "tokens": 3000}
  ],
  "model_id": "glm-5.2"
}
```

#### `usage`

Emitted after `usage` command. Provider-specific usage data.

```json
{
  "type": "usage",
  "provider": "umans",
  "provider_kind": "openai",
  "model": "glm-5.2",
  "usage": { "concurrent": 1, "requests_this_hour": 25, "requests_limit": 100 }
}
```

---

### Memory Events

#### `memory_saved`

Confirmation of a memory save or forget.

```json
{"type": "memory_saved", "id": "my-memory-slug", "message": "memory saved"}
```

#### `memory_list`

Emitted after `list_memory`.

```json
{
  "type": "memory_list",
  "entries": [ /* memory objects */ ],
  "count": 5
}
```

---

### Checkpoint Events

#### `checkpoints`

Emitted after `list_checkpoints`.

```json
{
  "type": "checkpoints",
  "checkpoints": [
    {"id": "ck-abc", "label": "before-auth-refactor", "kind": "git-stash", "created_at": "..."}
  ]
}
```

---

### Compaction Events

#### `compacting`

Emitted when compaction begins.

```json
{"type": "compacting", "before_tokens": 115000, "trigger": "auto"}
```

#### `compacted`

Emitted when compaction completes.

```json
{
  "type": "compacted",
  "before_tokens": 115000,
  "after_tokens": 45000,
  "before_messages": 120,
  "after_messages": 45
}
```

---

### Goal Events

#### `goal_state`

Emitted during goal mode to report phase changes.

```json
{
  "type": "goal_state",
  "goal": "Refactor the auth module",
  "phase": "planning",
  "progress": 0.3,
  "total_tasks": 12,
  "completed": 4,
  "failed": 0,
  "active": 3
}
```

#### `goal_plan`

Emitted when a goal plan is ready.

```json
{
  "type": "goal_plan",
  "prompts": [ /* step-by-step plan */ ],
  "auto_deploy": false
}
```

---

### Plugin Events

#### `plugin_list`

Emitted after `list_plugins`.

```json
{
  "type": "plugin_list",
  "plugins": [
    {"name": "catcode-chatgpt-provider", "enabled": true, "version": "1.0.0"}
  ]
}
```

#### `agents`

Emitted after `list_agents`.

```json
{
  "type": "agents",
  "agents": [
    {"name": "scout", "label": "Scout", "description": "Quickly explore an unfamiliar codebase..."},
    {"name": "planner", "label": "Planner", "description": "Decompose a goal into sub-steps..."}
  ]
}
```

#### `skills`

Emitted after `list_skills`.

```json
{
  "type": "skills",
  "skills": [
    {"name": "repository-documentation-factory", "description": "Create/proofread/repair docs", "location": "user"}
  ]
}
```

---

### Vision Events

#### `vision_config`

Emitted after `get_vision_config` or `set_vision_config`.

```json
{
  "type": "vision_config",
  "enabled": true,
  "vision_model": "glm-5.2-vision",
  "vision_models": ["glm-5.2-vision", "gpt-4o"]
}
```

---

### Error and Info Events

#### `error`

A non-fatal error occurred.

```json
{"type": "error", "message": "unknown provider preset 'foo'; available: umans, opencode-go, openrouter"}
```

#### `info`

An informational message.

```json
{"type": "info", "message": "logged into Umans."}
```

#### `bash_execution`

Emitted after `user_bash` command.

```json
{
  "type": "bash_execution",
  "command": "git status",
  "exit_code": 0,
  "stdout": "On branch master\nnothing to commit..."
}
```

---

## Typical Session Flow

```
Frontend                          Core
   |                               |
   |-------- init ---------------->|
   |                               |
   |<------- ready ----------------|  (models, config, provider state)
   |                               |
   |---- set_key / login --------->|  (optional)
   |<-- authed / provider_changed -|
   |                               |
   |-------- send ----------------->|
   |<--- thinking (stream) --------|
   |<--- delta (stream) -----------|
   |<--- tool_call ----------------|
   |<--- approval_request ---------|
   |--- approve (yes)------------->|
   |<--- tool_result --------------|
   |<--- delta (stream) -----------|
   |<--- done ---------------------|
   |                               |
   |-------- stats --------------->|
   |<------- stats ----------------|
```

---

## Error Handling

Errors are reported as `{"type": "error", "message": "..."}` events. These are
**non-fatal** — the core continues running after emitting an error. Common
error scenarios:

| Scenario | Error Message |
|----------|--------------|
| Unknown login preset | `"unknown provider preset '{name}'; available: ..."` |
| Missing API key on login | `"no API key provided for '{preset}' — paste a key via /login..."` |
| Logout of non-logged-in provider | `"not logged into '{provider}'"` |
| Unknown set_provider name | `"unknown provider '{name}'; not switching"` |
| Unknown model in send | `"unknown model: {model}"` |
| OAuth with no plugin | `"'{preset}' has no plugin OAuth login..."` |
| OAuth code without pending login | `"No pending OAuth login..."` |
| Invalid set_search_key provider | `"set_search_key: unknown provider '{provider}'..."` |

Fatal errors (e.g., panic recovery) cause the core to emit an `error` event,
then `done`, and continue with the next input.

Source: Error handling throughout `main.rs` (/core/src/main.rs) event dispatch.
