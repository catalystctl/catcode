# Plugin System

Plugins extend the Catalyst Code harness **without recompiling the core**. A
plugin is a directory under `.catalyst-code/plugins/<name>` (project-scoped) or
`~/.catalyst-code/plugins/<name>` (global) containing a `plugin.json` manifest
and executable scripts. No MCP, no build step, no external registry.

---

## Capabilities

A single plugin can provide any combination of:

| Capability | Description |
|------------|-------------|
| **Hooks** | Intercept tool calls and lifecycle events. Pre-hooks can modify args or deny the operation; post-hooks can read/modify results. |
| **Custom tools** | New function-calling tools visible to the model, run via a handler script. No recompile. |
| **OAuth providers** | Add a subscription-based provider (login flow + token resolution) like built-in OpenAI/Claude/Gemini. |
| **Memory providers** | Replace the built-in markdown memory store for standing-prompt injection, compaction extracts, and `/memory` commands. |
| **System-prompt injection** | Static text appended to the system prompt every turn. |
| **Slash commands** | Custom `/name` handlers callable from the chat. |
| **Disable built-in tools** | Remove a built-in tool from the model's toolset entirely. |
| **Tool override** | Replace a built-in tool's handler (e.g. a sandboxed `bash`, a redacting `read_file`) — the model keeps calling the same tool name. |

**Source:** `core/src/plugins.rs`, `PluginManifest` struct (line 156) and
`Plugin` struct (line 324).

---

## Discovery

The harness scans two directories at startup (and on `/plugin-reload`):

1. **Global, user-owned** — `~/.catalyst-code/plugins/` (staged on first run).
   Loads unconditionally. These are plugins *you* installed and trust in every
   project.
2. **Project-scoped** — `<workspace>/.catalyst-code/plugins/`. Gated by
   [`--trust-project-plugins`](#permission-model) — a repository you `cd` into
   must not run arbitrary hook scripts without opt-in.

A project plugin with the same name as a global one overrides it (matching the
agent/skill override model).

Plugins are loaded from subdirectories containing a `plugin.json` manifest.
Invalid plugins are skipped with a log message but never crash the harness.

**Source:** `PluginManager::scan_and_load`, `core/src/plugins.rs` line 620.

---

## Manifest Format (`plugin.json`)

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "What this plugin does",
  "system_prompt": "Optional static text injected into every system prompt.",
  "hooks": {
    "pre_bash": { "script": "hooks/pre-bash.sh", "timeout_ms": 5000 },
    "post_write": { "script": "hooks/post-write.py", "timeout_ms": 30000 }
  },
  "tools": [
    {
      "name": "my_tool",
      "description": "Does something useful",
      "parameters": { "type": "object", "properties": { ... } },
      "script": "tools/my-tool.sh",
      "kind": "readonly",
      "timeout_ms": 30000
    }
  ],
  "commands": [
    {
      "name": "my-command",
      "description": "A custom slash command",
      "script": "commands/my-command.sh",
      "timeout_ms": 15000
    }
  ],
  "disable_tools": ["bash"],
  "oauth": {
    "provider_id": "my-service",
    "label": "My Service",
    "kind": "openai",
    "base_url": "https://api.example.com/v1",
    "script": "oauth/handler.sh"
  },
  "memory_provider": {
    "script": "memory/provider.sh",
    "timeout_ms": 30000
  }
}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Plugin identifier (must be non-empty, unique) |
| `version` | string | yes | Semver string |
| `description` | string | no | Human-readable summary |
| `system_prompt` | string | no | Static text appended to the system prompt every turn |
| `hooks` | object | no | Hook point name → `{ script, timeout_ms?, pass_args? }` |
| `tools` | array | no | Custom tool declarations |
| `commands` | array | no | Custom slash commands |
| `disable_tools` | array | no | Built-in tool names to remove from the model's toolset |
| `oauth` | object | no | OAuth subscription provider definition |
| `memory_provider` | object | no | Memory backend definition |

**Source:** `core/src/plugins.rs`, `PluginManifest` (line 156), `ToolManifestEntry`
(line 241), `OauthManifestEntry` (line 267), `MemoryProviderManifestEntry` (line 225).

---

## Hook Points

There are **18** hook points. Each is classified by time of fire (pre/post/lifecycle)
and failure policy.

| Hook Point | When It Fires | Policy | Can Deny | Can Modify |
|-----------|--------------|--------|----------|------------|
| `pre_bash` | Before every `bash` tool call | Blocking | Yes | Yes |
| `pre_write` | Before `edit`, `write_file`, `patch` | Blocking | Yes | Yes |
| `pre_read` | Before `read_file`, `grep`, `glob` | Blocking | Yes | Yes |
| `pre_tool` | After the specific pre-hook, for **every** tool call (catch-all) | Blocking | Yes | Yes |
| `pre_input` | Before user input is sent to the model | Blocking | Yes | Yes |
| `post_bash` | After every `bash` tool call | Best-effort | No | Yes |
| `post_write` | After `edit`, `write_file`, `patch` | Best-effort | No | Yes |
| `post_read` | After `read_file`, `grep`, `glob` | Best-effort | No | Yes |
| `post_tool` | After the specific post-hook, for **every** tool call (catch-all) | Best-effort | No | Yes |
| `session_start` | When a session is created | Advisory | No | Yes |
| `session_stop` | When a session ends | Advisory | No | Yes |
| `pre_compact` | Before session compaction | Advisory | No | Yes |
| `pre_turn` | Before an agent turn starts | Advisory | No | Yes |
| `pre_agent_start` | Before the subagent loop starts | Advisory | No | Yes |
| `pre_context` | Before context construction | Advisory | No | Yes |
| `turn_start` | At the start of each agent turn | Advisory | No | Yes |
| `turn_end` | At the end of each agent turn | Advisory | No | Yes |
| `session_shutdown` | Harness shutdown | Advisory | No | Yes |

**Blocking** pre-hooks: failure (non-zero exit, timeout, parse error) denies the
operation with an error message. **Best-effort** post-hooks: failure silently
skips the hook result. **Advisory** lifecycle hooks: failure is logged but never
blocks the harness.

The `pre_tool` / `post_tool` catch-all hooks fire for **every** tool call,
including `memory`, `todo_write`, `git_*`, `subagent`, and plugin-declared tools
— giving a plugin the same reach over the dispatch loop that a core edit has.

**Source:** `core/src/plugins.rs`, `HOOK_POINTS` constant (line 37), `hook_policy`
(line 93).

### Hook Dispatch

Each hook fires as a **subprocess**. The harness:

1. Constructs a JSON context object.
2. Writes it to the hook script's stdin (bounded by the hook's timeout).
3. Reads one JSON object from stdout: `{ "allow": bool, "reason"?: string, "modify"?: object, "notify"?: string, "status"?: string }`.
4. Enforces the hook policy (deny vs skip, modify vs ignore).

**Context JSON structure:**

```json
{
  "hook": "pre_bash",
  "tool": "bash",
  "workspace": "/abs/path/to/workspace",
  "session_id": "session.jsonl",
  "timestamp": 1719000000,
  "args": { "command": "rm -rf /" }
}
```

The `args` field is only present when the hook's `pass_args` is `true`.

**Response fields:**

| Field | Type | Required | Meaning |
|-------|------|----------|---------|
| `allow` | bool | yes | `true` = proceed, `false` = deny (blocking hooks only) |
| `reason` | string | no | Human-readable explanation; shown on deny or as plugin note |
| `modify` | object | no | Keys merged over the original tool args (shallow, per-key) |
| `notify` | string | no | UI notification text |
| `status` | string | no | Status bar text (`""` clears, absent is a no-op) |

Broken hooks never crash the core: timeouts, parse failures, and non-zero exits
are caught and handled according to the hook policy.

**Source:** `execute_hook` (line 1731), `build_context` (line 1928), `apply_modify`
(line 1960) in `core/src/plugins.rs`.

---

## Custom Tools

Plugin-declared tools extend the model's function-calling toolset. Each entry
in the `tools` array becomes a tool the model can invoke.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | — | Tool name (must not collide with built-in tools) |
| `description` | string | no | `""` | Description shown to the model |
| `parameters` | object | no | `{}` | JSON Schema describing the tool's parameters |
| `script` | string | yes | — | Relative path to the handler script (must be executable, confined to plugin dir) |
| `kind` | string | no | `"destructive"` | `"readonly"` (no approval gate) or `"destructive"` (requires approval) |
| `timeout_ms` | number | no | `30000` | Hard timeout per call |
| `override` | bool | no | `false` | When `true`, replaces a built-in tool of the same name (model sees the plugin's schema, calls route to plugin script) |

The handler receives one JSON object on stdin:

```json
{ "args": {…}, "workspace": "/abs/path", "session_id": "x.jsonl", "timestamp": 1719000000 }
```

It must write one JSON object to stdout:

```json
{ "ok": true, "output": "result text shown to the model" }
```

A bare non-JSON stdout is accepted as the output text with `ok=true` (so a
trivial `echo` handler works). Non-zero exit, timeout, or spawn failure produce
an error outcome — the conversation continues, but the tool call failed.

**Source:** `ToolManifestEntry` (line 241), `execute_plugin_tool` (line 1989) in
`core/src/plugins.rs`.

---

## OAuth Providers

A plugin can add a subscription-based provider (login flow + token resolution).
The plugin supplies a script that handles four actions dispatched by an
`action` field in the stdin context:

| Action | Purpose |
|--------|---------|
| `login` | Start the login flow (returns URL + optional auto-open flag) |
| `complete` | Complete a manual paste-code flow |
| `token` | Resolve or refresh the access token |
| `clear` | Clear stored credentials |

The script receives:

```json
{ "action": "token", "provider_id": "my-service", "token_path": "/abs/path/to/token.json" }
```

The harness owns the loopback redirect server (browser flow) and the
`/oauth-code` paste path (manual flow). The plugin owns the token's on-disk
format.

Once registered, the provider appears in the `/login` picker and works like any
built-in provider: the harness enriches outgoing API requests with the resolved
access token and provider-specific headers. Tokens are cached in memory for the
duration of the session and refreshed near expiry.

**Source:** `OauthManifestEntry` (line 267), `oauth_login` (line 1483),
`resolve_oauth_creds` (line 1419) in `core/src/plugins.rs`.

---

## Memory Providers

A plugin can replace the built-in markdown memory store with a custom backend.
The script receives `action`, `args`, and responds with memory entries for
standing-prompt injection, compaction extracts, and slash memory commands.

| `action` | Purpose |
|----------|---------|
| `standing_prompt` | Return entries to inject into the system prompt |
| `write` | Write a memory entry |
| `list` | List recent entries |
| `forget` | Delete an entry |
| `search` | Search entries |

**Source:** `MemoryProviderManifestEntry` (line 225), `execute_memory_provider`
(line 2271) in `core/src/plugins.rs`.

---

## Slash Commands

Plugins can declare custom `/name` slash commands. The command name must not
collide with built-in reserved commands (which include `help`, `login`, `goal`,
`plugin-install`, and others — see `RESERVED_COMMAND_NAMES` in
`core/src/plugins.rs` line 126).

The handler receives one JSON object on stdin:

```json
{
  "command": "my-command",
  "args": "...",
  "workspace": "/abs/path",
  "session_id": "session.jsonl",
  "timestamp": 1719000000,
  "plugin": "my-plugin"
}
```

Stdout JSON:

```json
{ "ok": true, "output": "Command output" }
```

**Source:** `CommandManifestEntry` (line 216), `execute_plugin_command` (line 2119)
in `core/src/plugins.rs`.

---

## Permission Model

By default, **project-scoped plugins** (shipped inside a repository's
`.catalyst-code/plugins/`) are **not loaded**. This prevents a repo you `cd`
into from automatically running hook scripts (which see every tool's arguments,
including bash commands and file contents) with your privileges.

To enable them, pass:

```bash
catcode --trust-project-plugins
# or
CATALYST_CODE_TRUST_PROJECT_PLUGINS=1 catcode
```

The flag is read **only** from CLI args or environment variables — never from a
project config file, so a repository cannot self-enable its own hooks.

**User-installed** plugins (installed with `/plugin-install`) carry an on-disk
marker and load unconditionally, even inside the workspace. Global plugins
under `~/.catalyst-code/plugins/` also load unconditionally.

Disabled plugins remain on disk but are not invoked.

**Source:** `config.rs` line 170, `PluginManager::scan_dir` (line 718) in
`core/src/plugins.rs`.

---

## Commands

| Slash Command | Protocol Command | Action |
|---------------|------------------|--------|
| `/plugin-install <source> [scope]` | `install_plugin { path, scope? }` | Install a plugin from a local path or GitHub `owner/repo` (latest release). Scope: `global` (default) or `workspace`. |
| `/plugin-list` | `list_plugins` | List all installed plugins with hooks, tools, commands, and scope. |
| `/plugin-enable <name>` | `enable_plugin { name }` | Enable a disabled plugin. |
| `/plugin-disable <name>` | `disable_plugin { name }` | Disable a plugin without removing it. |
| `/plugin-remove <name>` | `remove_plugin { name }` | Remove a plugin entirely (from disk). |
| `/plugin-reload` | `reload_plugins` | Re-scan plugin directories (preserves enabled/disabled flags). |
| — | `plugin_command { name, args }` | Run a plugin-declared slash command programmatically. |
| — | `list_plugin_commands` | List all plugin-declared slash commands. |

**Protocol source:** `core/src/protocol.rs` lines 221–250.

---

## Example Workflow

```bash
# 1. Install a plugin from GitHub (latest release)
/plugin-install catalystctl/vision-handoff

# 2. List installed plugins
/plugin-list
# → name: vision-handoff, hooks: [pre_bash, post_read], tools: [my_tool], scope: global

# 3. Disable temporarily (keeps on disk)
/plugin-disable vision-handoff

# 4. Reload after editing plugin files
/plugin-reload

# 5. Re-enable
/plugin-enable vision-handoff

# 6. Remove entirely
/plugin-remove vision-handoff
```

---

## Full Authoring Contract

For the complete plugin authoring guide — including the exact hook protocol,
OAuth flow specification, memory provider contract, tool override semantics,
and debugging instructions — apply the `plugin-authoring` skill:

```
/skill:plugin-authoring
```

The skill is staged to `~/.catalyst-code/skills/plugin-authoring/SKILL.md` on
first run and is also available in the repository at
`.catalyst-code/skills/plugin-authoring/SKILL.md`.
