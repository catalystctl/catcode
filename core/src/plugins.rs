// Plugin system: self-bootstrapping hooks loaded from .catalyst-code/plugins/.
// Each plugin is a subdirectory with a plugin.json manifest and hook scripts.
// Hooks are spawned as subprocesses with stdin JSON context, stdout JSON response.
// Broken hooks never crash the core; timeouts and parse failures are handled gracefully.
use crate::config::{ProviderConfig, ProviderKind, ResolvedProvider};
use crate::oauth::{LoginOutcome, OAuthPrompt, PendingOauth};
use crate::tools::{Outcome, ToolKind};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

// ---- constants ----

/// Injected into the system prompt so agents can self-bootstrap plugins.
/// Explains the plugin directory layout, manifest format, hook contract,
/// and available hook points. Designed for an LLM to read and act on.
pub const PLUGIN_DOCS: &str = r#"## Plugin System

You can extend the harness with plugins. Plugins are self-contained directories
under `.catalyst-code/plugins/`. Each plugin hooks into tool execution and
session lifecycle events to inspect, approve, modify, or log operations.

### Creating a plugin

1. Create a directory: `.catalyst-code/plugins/<plugin-name>/`
2. Write a `plugin.json` manifest (see format below)
3. Write executable hook scripts (bash, python, or any language)
4. Make hook scripts executable (`chmod +x hooks/*.sh`)
5. The core loads new plugins on next restart, or you can call the `plugin` tool
   (if loaded by the TUI) to `install`, `remove`, `enable`, or `disable` plugins
   at runtime.

### plugin.json format

```
{
  "name": "my-plugin",
  "version": "0.1.0",
  "description": "What this plugin does",
  "hooks": {
    "pre_write": {
      "script": "hooks/pre_write.sh",
      "timeout_ms": 5000,
      "pass_args": true
    },
    "post_bash": {
      "script": "hooks/post_bash.py",
      "timeout_ms": 30000,
      "pass_args": false
    }
  }
}
```

Fields:
- `name` (required): unique plugin identifier (directory name must match)
- `version` (required): semver string
- `description` (optional): human-readable summary
- `hooks` (optional): map of hook-point name to config
  - `script` (required): path to executable, relative to the plugin directory
  - `timeout_ms` (optional): override the default hook timeout (default: 5s for pre_*, 30s for post_*)
  - `pass_args` (optional): if true, the hook context JSON includes the tool's `args` object (default: false)

### Hook contract

Each hook script receives a single JSON object on stdin and MUST write a single
JSON object to stdout before exiting. Stderr is captured for error reporting.

**Context (stdin → script):**
```
{
  "hook": "pre_write",
  "tool": "write_file",
  "workspace": "/path/to/workspace",
  "args": { "path": "src/file.rs", "content": "..." },
  "session_id": "abc123.jsonl",
  "timestamp": 1719000000
}
```

**Response (script → stdout):**
```
{
  "allow": true,
  "reason": "File passes lint check",
  "modify": { "content": "reformatted code" }
}
```

- `allow` (required, bool): true to proceed, false to block (pre hooks) or skip result (post hooks)
- `reason` (optional, string): human-readable explanation. For pre hooks it
  is shown to the model — appended to the tool result as a note on `allow`, and
  used as the deny message on `allow:false`. Also logged. For post hooks it is
  appended to the tool result as a note.
- `modify` (optional, object): for pre hooks, a JSON object whose keys are
  **merged over** the original tool args (shallow, per-key override). Return only
  the fields you want to change; everything else is preserved. Examples:
  pre_write `{ "content": "reformatted" }` overrides content but keeps `path`/
  `edits`; pre_bash `{ "command": "fixed command" }` overrides the command;
  pre_read `{ "path": "new/path" }` redirects the read. For **post hooks**,
  `modify` transforms the tool's RESULT: `{ "output": "...", "ok": false,
  "diff": "..." }` replaces the result text, flips success, or replaces/clears
  the diff — e.g. redact a secret, append context, or reformat. (The post
  context includes the current result under the `result` key so the hook can
  read it.)

  Note: pre-hook `modify` runs AFTER the approval gate + diff preview (which use
  the original args), so a rewritten `path`/`command` is NOT re-prompted. File
  tools still re-confine the path internally and `bash` re-checks its denylist,
  so the security boundaries hold — but a plugin that redirects a safe path to a
  sensitive one bypasses the user-facing prompt. Pre-hooks are trusted,
  user-installed code (project hooks gated by `--trust-project-plugins`).

Safety rules enforced by the core:
- pre_* hooks: non-zero exit, timeout, or JSON parse failure → `allow: false` (blocks the tool)
- post_* hooks: non-zero exit, timeout, or JSON parse failure → silently skipped (tool already ran)
- Disabled plugins are never invoked
- Every hook has a hard timeout (5s default for pre_*, 30s default for post_*)
- Hook failures never crash the core

### Available hook points

| Hook point    | Fires when                              | Type |
|---------------|-----------------------------------------|------|
| pre_bash      | Before a bash command executes          | pre  |
| pre_write     | Before a file write/edit                | pre  |
| pre_read      | Before a file is read                   | pre  |
| post_bash     | After a bash command completes          | post |
| post_write    | After a file write/edit completes       | post |
| post_read     | After a file is read                    | post |
| pre_tool      | Before ANY tool executes (catch-all)    | pre  |
| post_tool     | After ANY tool executes (catch-all)     | post |
| session_start | When a session begins (prompt received) | lifecycle |
| session_stop  | When a session ends (done/abort)        | lifecycle |
| pre_compact   | Before conversation compaction         | pre  |
| pre_turn      | Before a model request (advisory)      | pre  |

### pre_turn hook (model handoff)

`pre_turn` fires once per assistant turn, after the user message (including any
attached images) is built and before the first model request. It is advisory:
it can remap the model for the turn but can never block it (a missing/broken
hook or `allow:false` is ignored — the turn proceeds with the original model).

Context `args` (set `pass_args: true` in the manifest):
```
{
  "model": "umans-glm-5.2",
  "has_images": true,
  "image_count": 2,
  "models": [ {"id":"...", "vision":true}, ... ]
}
```
Response: return `modify: { "model": "<new-model-id>" }` to swap the turn's
model. The core validates the id against discovered models and emits an `info`
event on handoff. Use this to route image-bearing turns to a vision-capable
model when the active one lacks vision (see the bundled `vision-handoff` plugin).

### Declaring tools (custom capabilities, no MCP)

A plugin can ALSO declare tools — first-class capabilities the model can call,
defined without MCP and without recompiling. A tool is a JSON Schema (sent to
the model like any built-in tool) plus a handler script the core spawns per
call. This is the no-MCP way to give the agent a new capability (a domain tool,
a CLI wrapper, an internal-API client, …) by dropping files in
`.catalyst-code/plugins/`. (Plugin tools are available to the main agent;
subagents use the built-in tool set.)

Add a `tools` array to `plugin.json` (a plugin may declare only tools, only
hooks, or both):

```
{
  "name": "my-tools",
  "version": "0.1.0",
  "description": "Custom domain tools",
  "tools": [
    {
      "name": "lookup_order",
      "description": "Look up an order by id from the internal API.",
      "parameters": {
        "type": "object",
        "properties": { "order_id": { "type": "string" } },
        "required": ["order_id"]
      },
      "script": "tools/lookup_order.sh",
      "kind": "readonly",
      "timeout_ms": 15000
    }
  ]
}
```

Fields:
- `name` (required): tool name. By default it must not collide with a built-in
  tool (bash, read_file, edit, subagent, …) — a colliding plugin tool is skipped
  and the built-in wins. Set `override: true` (below) to instead REPLACE the
  built-in's implementation with this plugin's handler.
- `description` (optional): shown to the model.
- `parameters` (optional): a JSON Schema for the tool's arguments. Defaults
  to an empty object.
- `script` (required): path to the executable handler, relative to the plugin
  directory (path-confined; `..` escapes are rejected).
- `kind` (optional): `"readonly"` (skips the approval gate) or `"destructive"`
  (prompts under Approval::Destructive — the default). Arbitrary external code
  runs on every call, so default to `destructive`.
- `override` (optional, bool): when `true` AND `name` matches a built-in tool,
  this plugin's handler REPLACES that built-in — the model still sees a tool of
  that name (this plugin's declared `description`/`parameters`), but calls route
  to the plugin script instead of the core handler. This is the no-recompile way
  to fully override a core tool (a sandboxed `bash`, a redacting `read_file`, a
  rate-limited `git_commit`, …). Default `false`: a name collision stays built-in.
- `timeout_ms` (optional): hard per-call timeout (default 30s).

Tool handler contract (one JSON object on stdin, one on stdout):

```
# stdin (→ handler)
{ "args": { "order_id": "12345" }, "workspace": "/abs/path", "session_id": "x.jsonl", "timestamp": 1719000000 }

# stdout (→ core)
{ "ok": true,  "output": "Order #12345: shipped" }
{ "ok": false, "output": "order not found" }   # ok omitted defaults to true
```

- `output` is the text shown to the model as the tool result. `ok:false` (or
  an `error` field) marks the call failed — the conversation continues; the
  model sees the error and can react.
- A bare non-JSON stdout is accepted as `output` with `ok=true`, so a trivial
  `echo` handler works. Prefer the structured form.
- Non-zero exit, timeout, or spawn failure produce an error result (the model
  is told the tool failed); they never crash the core or the turn.

Safety (identical to hooks): tool scripts are path-confined to the plugin
directory, must be executable, and run with the same `trust_project_plugins`
gate — a repo's project-scoped tools load only after you opt in with
`--trust-project-plugins` (built-in + `~/.catalyst-code/plugins` tools load
always). Tool calls honor the approval gate and your allow/deny permission
rules like any tool.

`.catalyst-code/plugins/my-tools/tools/lookup_order.sh`:
```bash
#!/bin/bash
input=$(cat)
id=$(echo "$input" | jq -r '.args.order_id')
# …call your internal API…
jq -n --arg o "Order #$id: shipped" '{ "ok": true, "output": $o }'
```

Remember: `chmod +x` the handler.

### Add, override, and remove core behavior

A plugin can do everything a direct core edit can — add, override, and remove
behavior — without recompiling. Five mechanisms cover the full surface:

| Operation | Mechanism | Notes |
|-----------|-----------|-------|
| **ADD a tool** | `tools` array | A new capability the model can call. |
| **OVERRIDE a tool** | a tool with `override: true` | Replaces a built-in's implementation (the model still sees that tool name; calls route to the plugin script). |
| **REMOVE a tool** | `disable_tools` | Drops the named tool from the model's toolset entirely (built-in or override). The strongest lever — wins over `override`. |
| **MODIFY tool input** | `pre_bash`/`pre_write`/`pre_read`/`pre_tool` `modify` | Override specific args before execution. |
| **MODIFY tool output** | `post_*`/`post_tool` `modify` | Replace the result text / flip success / change the diff after execution. |
| **MODIFY the model** | `pre_turn` `modify.model` | Remap the turn's model (advisory). |
| **ADD to the system prompt** | `system_prompt` field | Static text appended to the system prompt. |

#### `disable_tools` — remove a capability

```json
{
  "name": "no-bash",
  "version": "1.0.0",
  "disable_tools": ["bash", "git_commit"]
}
```

The listed tool names vanish from the model's toolset — it can never call them
(this is stronger than a per-call `pre_bash` deny). Composes across plugins (the
union is removed). Applied as a final filter, so it also removes a tool another
plugin `override`s.

#### `system_prompt` — inject context

```json
{
  "name": "domain-rules",
  "version": "1.0.0",
  "system_prompt": "All database access must go through the `db_query` tool. Never construct raw SQL in bash."
}
```

The text is appended to the system prompt (after the plugin docs), framed with
the plugin name + version — the same surface a core edit of the system prompt
touches. Empty by default, so the prompt + its prefix cache are untouched when no
plugin declares one. (Main agent only; subagents use the built-in tool set.)

#### `override: true` — replace a core tool

```json
{
  "name": "sandboxed-bash",
  "version": "1.0.0",
  "tools": [
    {
      "name": "bash",
      "override": true,
      "description": "Run a command in the project sandbox.",
      "parameters": {"type":"object","properties":{"command":{"type":"string"}},"required":["command"]},
      "script": "tools/bash.sh",
      "kind": "destructive"
    }
  ]
}
```

The model calls `bash` as usual, but the call routes to `tools/bash.sh` instead
of the core handler — and the plugin controls the description/schema. The tool's
`kind` (approval gate) is the plugin's. The specific `pre_bash`/`post_bash`
hooks still fire (keyed on the tool name), and `pre_tool`/`post_tool` fire too.

### Declaring an OAuth provider (subscription auth)

A plugin can add a subscription-OAuth provider — the same mechanism the
built-in OpenAI (ChatGPT), Google (Gemini), and Anthropic (Claude) providers
use, but for any vendor — with no recompile. The plugin supplies ONE script
that handles four actions (`login`, `complete`, `token`, `clear`); the
harness owns the loopback redirect server (web flow) and the `/oauth-code`
paste path (manual/device flow), exactly like the built-in flows.

Add an `oauth` block to `plugin.json`:

```json
{
  "name": "grok-oauth",
  "version": "0.1.0",
  "oauth": {
    "provider_id": "grok",
    "label": "Grok (xAI)",
    "kind": "openai",
    "base_url": "https://api.x.ai/v1",
    "description": "Grok via xAI device-code OAuth.",
    "headers": [["X-Source", "catalyst-code"]],
    "token_path": "grok.json",
    "script": "oauth/oauth.sh",
    "login_timeout_ms": 180000,
    "token_timeout_ms": 30000
  }
}
```

Fields:
- `provider_id` (required): the provider identity. The harness creates the
  provider config with this `name` on a successful `/login`, and `/oauth-code`
  + `/logout` dispatch on it.
- `label` (optional): shown in the `/login` picker (defaults to provider_id).
- `kind` (optional, default `"openai"`): `"openai"` (OpenAI-compatible
  `/chat/completions`) or `"anthropic"` (`/v1/messages`). Decides the wire
  protocol + auth header.
- `base_url` (required): the endpoint, including `/v1`. Paths are appended
  directly.
- `description` (optional): shown in the picker.
- `headers` (optional): extra HTTP headers on every request, `[[key,val],…]`.
  These are persisted into the provider config.
- `token_path` (optional, default `<provider_id>.json`): the token-file name,
  relative to `~/.config/catalyst-code/oauth/`. The plugin owns the token's
  on-disk format; the harness only checks existence (for the "logged in"
  status) and passes the absolute path to the script.
- `script` (required unless every action has an explicit override): the script
  handling ALL actions, dispatched by the `action` field in stdin.
- `login_script` / `complete_script` / `token_script` (optional): per-action
  overrides. When absent, the action falls back to `script`.
- `login_timeout_ms` (optional, default 120000): timeout for `login` +
  `complete`.
- `token_timeout_ms` (optional, default 30000): timeout for `token` + `clear`.

#### Script action contract

Every invocation receives ONE JSON object on stdin (with `action` + the base
context) and MUST write ONE JSON object to stdout. The base context always
includes `action`, `provider_id`, `token_path` (absolute), `workspace`, and
`timestamp`; each action adds its own fields.

**`login`** — build the authorize/verify URL. Input adds `headless` (bool) and,
for the web flow, `redirect_uri` (a `http://localhost:<port>/callback` the
harness already bound — embed it verbatim in your authorize URL). Output:
```json
{ "url": "https://auth.example.com/device?...", "code": "ABCD-EFGH",
  "message": "Open the URL and enter the code",
  "flow": "web" | "manual",
  "state": "<csrf>", "pending": { "verifier": "...", "device_id": "..." } }
```
- `flow`: `"web"` = the harness waits for the loopback redirect (local
  machine); `"manual"` = the user pastes a code back via `/oauth-code`
  (SSH/headless, or device-code flows). Honor `headless`: return `"manual"`
  when there is no usable browser.
- `state` (web flow): the CSRF state you put in the authorize URL, so the
  harness can verify the redirect.
- `pending` (both flows): an opaque JSON blob you need to carry to `complete`
  (e.g. a PKCE verifier, a device-auth id). The harness stashes it and passes
  it back verbatim.
- `code` (optional, manual/device flow): a user-code to display.

**`complete`** — exchange the code for a token and WRITE it to `token_path`.
Input adds `code` (the pasted/redirected code) and `redirect_uri` (web flow) or
`pending` (manual flow). Output:
```json
{ "ok": true }
{ "ok": false, "error": "expired code" }
```

**`token`** — resolve/refresh the access token. Read `token_path`; if expired,
refresh (make your own HTTP call) and write the updated token back. Output:
```json
{ "access_token": "<bearer>", "expires_at": 1719003600 }
{ "access_token": null }
{ "ok": false, "error": "refresh failed" }
```
`expires_at` is unix seconds (optional; if 0/absent the harness caches for ~5
min). This runs on the per-turn hot path, so it is cached until near expiry.

**`clear`** — delete any credentials + extra state you manage. The harness
ALSO deletes `token_path`, so this is optional (use it for sidecar files).
Output: `{ "ok": true }`.

#### How it fits together

- `/login <provider_id>`: the harness runs `login`, emits the URL as an
  `oauth_prompt`, and either waits for the redirect (web) or stashes `pending`
  and waits for `/oauth-code` (manual). On success it creates the provider
  config (name = provider_id, your base_url/kind/headers, no api_key) and
  refreshes `/models`.
- At turn + discovery time: the harness runs `token` (cached), injects the
  access token as `Authorization: Bearer`, and routes the turn to your
  `base_url` over your declared `kind`.
- `/logout <provider_id>`: deletes `token_path` + runs `clear` + drops the
  provider config.

The plugin's token format is entirely its own — the harness never parses it.

### Example: a pre_write linter plugin

`.catalyst-code/plugins/lint-check/plugin.json`:
```
{
  "name": "lint-check",
  "version": "0.1.0",
  "description": "Run cargo fmt on Rust files before writing",
  "hooks": {
    "pre_write": {
      "script": "hooks/pre_write.sh",
      "timeout_ms": 10000,
      "pass_args": true
    }
  }
}
```

`.catalyst-code/plugins/lint-check/hooks/pre_write.sh`:
```bash
#!/bin/bash
input=$(cat)
path=$(echo "$input" | jq -r '.args.path // ""')
content=$(echo "$input" | jq -r '.args.content // ""')

if [[ "$path" == *.rs ]] && command -v rustfmt &>/dev/null; then
  formatted=$(echo "$content" | rustfmt --edition 2021 2>/dev/null)
  if [ $? -eq 0 ] && [ -n "$formatted" ]; then
    jq -n --arg c "$formatted" '{ "allow": true, "reason": "rustfmt applied", "modify": { "content": $c } }'
    exit 0
  fi
fi
echo '{"allow": true}'
```

Remember: `chmod +x .catalyst-code/plugins/lint-check/hooks/pre_write.sh`
"#;

/// Valid hook point names. Plugins can register for any of these.
pub const HOOK_POINTS: &[&str] = &[
    "pre_bash",
    "pre_write",
    "pre_read",
    "post_bash",
    "post_write",
    "post_read",
    "session_start",
    "session_stop",
    "pre_compact",
    "pre_turn",
    // Catch-all hooks that fire for EVERY tool call (in addition to the
    // specific pre_bash/pre_write/pre_read). They cover tools with no
    // dedicated hook (memory, todo_write, git_*, subagent, plugin tools, …)
    // so a plugin can audit/modify/deny ANY tool — the same reach a core edit
    // of the dispatch loop has. pre_tool runs after the specific pre-hook;
    // post_tool runs after the specific post-hook.
    "pre_tool",
    "post_tool",
];

/// Default timeout in milliseconds for pre_* hooks (blocking — keep short).
pub const DEFAULT_PRE_TIMEOUT_MS: u64 = 5_000;

/// Default timeout in milliseconds for post_* and lifecycle hooks.
pub const DEFAULT_POST_TIMEOUT_MS: u64 = 30_000;

// ---- manifest deserialization (plugin.json) ----

#[derive(Deserialize, Debug, Clone)]
struct PluginManifest {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    hooks: HashMap<String, HookManifestEntry>,
    /// Optional user-declared tools (custom capabilities, no MCP needed).
    #[serde(default)]
    tools: Vec<ToolManifestEntry>,
    /// Built-in/plugin tool names to REMOVE from the model's toolset.
    #[serde(default)]
    disable_tools: Vec<String>,
    /// Static text injected into the system prompt (empty = none).
    #[serde(default)]
    system_prompt: String,
    /// Optional OAuth subscription provider this plugin adds (login flow +
    /// token resolution), mirroring the built-in OpenAI/Claude/Gemini OAuth.
    #[serde(default)]
    oauth: Option<OauthManifestEntry>,
}

#[derive(Deserialize, Debug, Clone)]
struct HookManifestEntry {
    script: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    pass_args: bool,
}

/// A tool declared in a plugin manifest (the `tools` array). Each entry becomes
/// a tool the model can call; the `script` handler is spawned per call.
#[derive(Deserialize, Debug, Clone)]
struct ToolManifestEntry {
    name: String,
    #[serde(default)]
    description: String,
    /// JSON Schema for the tool's parameters (sent to the model as-is).
    #[serde(default)]
    parameters: Value,
    script: String,
    /// "readonly" (skip the approval gate) or "destructive" (prompt; default).
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// When true AND `name` matches a built-in tool, this plugin's handler
    /// REPLACES the built-in's implementation: the model still sees a tool of
    /// that name (the plugin's declared schema), but calls route to the plugin
    /// script instead of the core handler. Lets a plugin fully override a
    /// core tool (a sandboxed bash, a redacting read_file, …) without
    /// recompiling. Default false: a name collision stays built-in (unchanged).
    #[serde(default, rename = "override")]
    override_builtin: bool,
}

/// An OAuth provider declared by a plugin manifest's `oauth` block. Lets a
/// plugin add a subscription-OAuth provider (login flow + token resolution)
/// the same way the built-in OpenAI/Claude/Gemini providers work — no
/// recompile. The plugin supplies ONE script that handles four actions
/// (`login`, `complete`, `token`, `clear`) dispatched by an `action` field in
/// the stdin context; per-action script overrides are optional. See
/// PLUGIN_DOCS for the full contract.
#[derive(Deserialize, Debug, Clone)]
struct OauthManifestEntry {
    /// The provider identity. Must match the provider-config `name` created on
    /// `/login` (the harness creates the config with this name). Also the key
    /// `/oauth-code` and `/logout` dispatch on.
    provider_id: String,
    /// Human label shown in the `/login` picker (defaults to provider_id).
    #[serde(default)]
    label: Option<String>,
    /// Wire protocol: "openai" (default) or "anthropic".
    #[serde(default)]
    kind: Option<String>,
    /// The endpoint base URL (include `/v1`; paths appended directly).
    base_url: String,
    #[serde(default)]
    description: Option<String>,
    /// Extra HTTP headers appended to every request, `[[key,val],…]`.
    #[serde(default)]
    headers: Vec<(String, String)>,
    /// Token-file name, relative to `~/.config/catalyst-code/oauth/`. Defaults
    /// to `<provider_id>.json`. The harness passes the ABSOLUTE resolved path to
    /// every script invocation, so the plugin owns the token's on-disk format.
    #[serde(default)]
    token_path: Option<String>,
    /// The script handling ALL actions (login/complete/token/clear). Required
    /// unless every action has an explicit override.
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    login_script: Option<String>,
    #[serde(default)]
    complete_script: Option<String>,
    #[serde(default)]
    token_script: Option<String>,
    /// Timeout for the login + complete actions (default 120s).
    #[serde(default)]
    login_timeout_ms: Option<u64>,
    /// Timeout for the token (resolve/refresh) action (default 30s).
    #[serde(default)]
    token_timeout_ms: Option<u64>,
}

// ---- public types ----

/// A loaded plugin with its registered hooks and declared tools.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    /// Absolute path to the plugin directory on disk.
    pub source_path: PathBuf,
    /// Hook name → config map.
    pub hooks: HashMap<String, HookConfig>,
    /// Tools this plugin declares (custom capabilities; no MCP needed).
    pub tools: Vec<ToolConfig>,
    /// Built-in/plugin tool names to REMOVE from the model's toolset.
    pub disable_tools: Vec<String>,
    /// Static text injected into the system prompt (empty = none).
    pub system_prompt: String,
    /// OAuth subscription provider this plugin declares, if any.
    pub oauth: Option<PluginOauthConfig>,
}

/// Configuration for one hook within a plugin.
#[derive(Clone, Debug)]
pub struct HookConfig {
    /// Absolute path to the executable hook script.
    pub script: PathBuf,
    /// Hard timeout in milliseconds for this hook.
    pub timeout_ms: u64,
    /// Whether to include tool args in the hook context JSON.
    pub pass_args: bool,
}

/// Configuration for one user-declared tool within a plugin.
#[derive(Clone, Debug)]
pub struct ToolConfig {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters (sent to the model verbatim).
    pub parameters: Value,
    /// Absolute path to the executable handler script.
    pub script: PathBuf,
    /// Hard timeout in milliseconds for a single tool call.
    pub timeout_ms: u64,
    /// Approval classification: ReadOnly skips the gate, Destructive prompts.
    pub kind: ToolKind,
    /// True → this tool's handler replaces the built-in of the same name.
    pub override_builtin: bool,
}

/// A loaded OAuth-provider declaration (manifest `oauth` block with script
/// paths resolved to absolute, path-confined, executable files). The plugin
/// owns the token's on-disk format; the harness owns the loopback redirect
/// server (web flow) and the `/oauth-code` paste path (manual flow).
#[derive(Clone, Debug)]
pub struct PluginOauthConfig {
    pub provider_id: String,
    pub label: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub description: String,
    pub headers: Vec<(String, String)>,
    /// Absolute path the plugin reads/writes its token at.
    pub token_path: PathBuf,
    /// Resolved absolute script paths per action (override else the default).
    pub scripts: HashMap<String, PathBuf>,
    pub login_timeout_ms: u64,
    pub token_timeout_ms: u64,
}

impl PluginOauthConfig {
    /// Resolve which script runs `action` (the action-specific override, else
    /// the shared `script` fallback).
    pub fn script_for(&self, action: &str) -> Option<&Path> {
        self.scripts
            .get(action)
            .or_else(|| self.scripts.get("*"))
            .map(|p| p.as_path())
    }
}

/// A cached OAuth access token + its absolute-seconds expiry, keyed by
/// provider_id in the PluginManager. Keeps the per-turn hot path (enrich_oauth)
/// from spawning the token script on every request.
#[derive(Clone)]
struct CachedToken {
    token: String,
    expires_at: u64,
}

/// Result returned from executing a hook.
#[derive(Clone, Debug)]
pub struct HookResult {
    /// Whether the operation is allowed to proceed.
    pub allow: bool,
    /// Human-readable explanation from the hook.
    pub reason: String,
    /// Optional modified arguments (pre hooks only; ignored for post hooks).
    pub modify: Option<Value>,
}

// ---- PluginManager ----

/// Manages the lifecycle of all installed plugins.
/// Holds an in-memory registry behind a `RwLock`.
pub struct PluginManager {
    plugins_dir: PathBuf,
    /// Optional **global, user-owned** plugins dir (`~/.catalyst-code/plugins`)
    /// scanned before the project dir so globally-staged plugins load across
    /// every project. `None` for the isolated `new()` constructor (used by
    /// tests); `Some` for `new_with_global_plugins()` (used by the core at
    /// startup). A project plugin with the same name overrides the global one.
    user_plugins_dir: Option<PathBuf>,
    /// Workspace root — used to decide whether a plugin dir is project-scoped
    /// (inside the workspace) vs user-installed (outside it).
    workspace: PathBuf,
    /// When false (the secure default), project-scoped plugins under the
    /// workspace's `.catalyst-code/plugins` are NOT auto-loaded — a repo you
    /// `cd` into must not run hook scripts with your privileges without opt-in.
    trust_project: bool,
    plugins: RwLock<HashMap<String, Plugin>>,
    /// Project-scoped plugin names skipped because trust_project is false.
    skipped_project: Mutex<Vec<String>>,
    /// In-memory cache of resolved OAuth access tokens (provider_id → token),
    /// so the per-turn hot path (`enrich_oauth`) doesn't spawn the token script
    /// on every request. Refreshed when near expiry.
    token_cache: Mutex<HashMap<String, CachedToken>>,
}

impl PluginManager {
    /// Create a new manager and scan/load all plugins from `plugins_dir` only
    /// (the project plugins dir). This is the **isolated** constructor used by
    /// tests; it does NOT scan the global `~/.catalyst-code/plugins` dir, so
    /// tests are unaffected by the developer's real global plugins.
    ///
    /// Production code should use [`PluginManager::new_with_global_plugins`]
    /// instead, which also loads globally-staged plugins.
    pub fn new(plugins_dir: PathBuf, workspace: PathBuf, trust_project: bool) -> Self {
        let mgr = PluginManager {
            plugins_dir,
            user_plugins_dir: None,
            workspace,
            trust_project,
            plugins: RwLock::new(HashMap::new()),
            skipped_project: Mutex::new(Vec::new()),
            token_cache: Mutex::new(HashMap::new()),
        };
        mgr.scan_and_load();
        mgr
    }

    /// Production constructor: like [`PluginManager::new`] but ALSO scans the
    /// global, user-owned plugins dir (`~/.catalyst-code/plugins`, staged on
    /// first run) before the project dir. Globally-staged plugins (e.g. the
    /// vision-handoff plugin) therefore load in every project without any
    /// per-project setup; a same-named project plugin overrides the global one.
    pub fn new_with_global_plugins(
        plugins_dir: PathBuf,
        workspace: PathBuf,
        trust_project: bool,
    ) -> Self {
        let user_plugins_dir = crate::config::home_dir().map(|h| h.join(".catalyst-code/plugins"));
        let mgr = PluginManager {
            plugins_dir,
            user_plugins_dir,
            workspace,
            trust_project,
            plugins: RwLock::new(HashMap::new()),
            skipped_project: Mutex::new(Vec::new()),
            token_cache: Mutex::new(HashMap::new()),
        };
        mgr.scan_and_load();
        mgr
    }

    /// Test-only constructor that scans an explicit user (global) plugins dir
    /// in addition to the project dir, so global-scan behavior can be exercised
    /// deterministically without touching the real `~/.catalyst-code/plugins`.
    #[cfg(test)]
    fn new_with_user_plugins_dir(
        plugins_dir: PathBuf,
        user_plugins_dir: Option<PathBuf>,
        workspace: PathBuf,
        trust_project: bool,
    ) -> Self {
        let mgr = PluginManager {
            plugins_dir,
            user_plugins_dir,
            workspace,
            trust_project,
            plugins: RwLock::new(HashMap::new()),
            skipped_project: Mutex::new(Vec::new()),
            token_cache: Mutex::new(HashMap::new()),
        };
        mgr.scan_and_load();
        mgr
    }

    /// Names of project-scoped plugins skipped because `trust_project` is false.
    /// The caller surfaces these to the user so the opt-in is discoverable.
    pub fn skipped_project_plugins(&self) -> Vec<String> {
        self.skipped_project.lock().unwrap().clone()
    }

    /// Re-scan the plugin directories and load/reload all valid plugins.
    ///
    /// Two directories are scanned:
    /// 1. the **global, user-owned** plugins dir `~/.catalyst-code/plugins`
    ///    (staged on first run; shared across every project; loads always —
    ///    these are plugins *you* installed, outside the workspace), then
    /// 2. the **project** plugins dir (`plugins_dir`, default
    ///    `.catalyst-code/plugins` inside the workspace; gated by
    ///    `trust_project`).
    ///
    /// On a name collision the project plugin wins, so a project's own
    /// `.catalyst-code/plugins/<name>` overrides the global one for that
    /// project only — matching the agent/skill override model.
    ///
    /// Invalid plugins are skipped with a log message to stderr but never
    /// crash. Project-scoped plugins (dir inside the workspace) are skipped
    /// unless `trust_project` is true; their names are recorded in
    /// `skipped_project`.
    fn scan_and_load(&self) {
        let canon_ws =
            std::fs::canonicalize(&self.workspace).unwrap_or_else(|_| self.workspace.clone());
        let mut plugins = self.plugins.write().unwrap();
        plugins.clear();
        let mut skipped_local: Vec<String> = Vec::new();

        // 1) Global, user-owned plugins (~/.catalyst-code/plugins) when this
        //    manager was constructed to scan them (production). Outside the
        //    workspace, so `is_project` is false and they load unconditionally.
        //    Skipped entirely for the isolated `new()` constructor (tests).
        if let Some(ref user_dir) = self.user_plugins_dir {
            self.scan_dir(user_dir, &canon_ws, &mut plugins, &mut skipped_local);
        }

        // 2) Project plugins. Scanned last so a same-named project plugin
        //    overrides the global one. Created on demand so the dir existing
        //    never errors.
        let _ = std::fs::create_dir_all(&self.plugins_dir);
        self.scan_dir(
            &self.plugins_dir,
            &canon_ws,
            &mut plugins,
            &mut skipped_local,
        );

        *self.skipped_project.lock().unwrap() = skipped_local;
    }

    /// Scan one plugin directory and load every valid plugin in it into
    /// `plugins` (later inserts override earlier ones on name collision).
    /// `skipped` collects project-scoped plugin names that were gated off by
    /// `trust_project`. A missing or unreadable directory is a silent no-op.
    fn scan_dir(
        &self,
        dir: &std::path::Path,
        canon_ws: &std::path::Path,
        plugins: &mut HashMap<String, Plugin>,
        skipped: &mut Vec<String>,
    ) {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.json");
            if !manifest_path.exists() {
                continue;
            }
            // Project-scoped gating: a plugin dir inside the workspace (e.g.
            // `.catalyst-code/plugins/*` shipped by the repo) is treated as
            // untrusted unless the user opted in via `trust_project`. This stops
            // a repo from auto-running hook scripts (which see every tool's
            // args, including bash commands + file contents) with your
            // privileges. User-installed plugins (outside the workspace) load
            // regardless. `trust_project` is read only from env/CLI, never a
            // project config file, so a repo can't self-enable.
            let canon_plugin = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            let is_project = canon_plugin.starts_with(canon_ws);
            if is_project && !self.trust_project {
                let name = std::fs::read_to_string(&manifest_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .map(String::from)
                            .unwrap_or_default()
                    });
                eprintln!(
                    "[plugins] skipping project-scoped plugin '{name}' (in {canon_plugin:?}); set --trust-project-plugins / CATALYST_CODE_TRUST_PROJECT_PLUGINS=1 to enable"
                );
                skipped.push(name);
                continue;
            }
            match Self::load_plugin_from_dir(&path) {
                Ok(plugin) => {
                    plugins.insert(plugin.name.clone(), plugin);
                }
                Err(e) => {
                    eprintln!(
                        "[plugins] failed to load plugin in {:?}: {e}",
                        path.file_name().unwrap_or_default()
                    );
                }
            }
        }
    }

    /// Load a single plugin from a directory containing plugin.json.
    fn load_plugin_from_dir(dir: &Path) -> Result<Plugin, String> {
        let manifest_path = dir.join("plugin.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read plugin.json: {e}"))?;

        let manifest: PluginManifest =
            serde_json::from_str(&raw).map_err(|e| format!("plugin.json parse error: {e}"))?;

        if manifest.name.is_empty() {
            return Err("plugin name is empty".into());
        }

        // Canonicalize the plugin directory for path-confinement checks.
        let canon_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());

        let mut hooks: HashMap<String, HookConfig> = HashMap::new();
        for (hook_name, entry) in &manifest.hooks {
            if !HOOK_POINTS.contains(&hook_name.as_str()) {
                eprintln!(
                    "[plugins] unknown hook point '{}' in plugin '{}'; skipping",
                    hook_name, manifest.name
                );
                continue;
            }

            let script_rel = Path::new(&entry.script);

            // Reject `..` escapes in the relative path before any join.
            {
                use std::path::Component;
                for comp in script_rel.components() {
                    if let Component::ParentDir = comp {
                        return Err(format!(
                            "hook script {:?} escapes the plugin directory",
                            entry.script
                        ));
                    }
                }
            }

            let script_abs = canon_dir.join(script_rel);

            // Canonicalize if possible to catch symlink escapes.
            let canon_script =
                std::fs::canonicalize(&script_abs).unwrap_or_else(|_| script_abs.clone());
            if !canon_script.starts_with(&canon_dir) {
                return Err(format!(
                    "hook script {:?} escapes the plugin directory",
                    entry.script
                ));
            }

            if !canon_script.exists() {
                return Err(format!("hook script {:?} does not exist", entry.script));
            }

            // Cross-platform executable check (Unix permission bit, or
            // extension/presence on Windows where there is no exec bit).
            let is_exe = is_executable(&canon_script);
            if !is_exe {
                return Err(format!(
                    "hook script {:?} is not executable (try chmod +x)",
                    entry.script
                ));
            }

            let timeout_ms = entry
                .timeout_ms
                .unwrap_or_else(|| default_hook_timeout(hook_name));

            hooks.insert(
                hook_name.clone(),
                HookConfig {
                    script: canon_script,
                    timeout_ms,
                    pass_args: entry.pass_args,
                },
            );
        }

        // --- plugin-declared tools (custom capabilities, no MCP needed) ---
        // Each tool's handler script gets the same path-confinement +
        // executable checks as a hook script, so a plugin can't reach outside
        // its directory or run a non-executable file. Reserved-name filtering
        // (collisions with built-in tools) is done by the caller at merge time,
        // not here — keep loading decoupled from the built-in tool set.
        let mut tools_vec: Vec<ToolConfig> = Vec::new();
        for t in &manifest.tools {
            if t.name.is_empty() {
                return Err("plugin declares a tool with an empty name".into());
            }
            let canon_script = validate_plugin_script(&canon_dir, &t.script)?;
            let timeout_ms = t.timeout_ms.unwrap_or(DEFAULT_POST_TIMEOUT_MS);
            let kind = match t.kind.as_deref().unwrap_or("destructive") {
                "readonly" => ToolKind::ReadOnly,
                "destructive" => ToolKind::Destructive,
                other => {
                    return Err(format!(
                        "tool '{}' has invalid kind '{}' (use 'readonly' or 'destructive')",
                        t.name, other
                    ))
                }
            };
            let parameters = if t.parameters.is_object() {
                t.parameters.clone()
            } else {
                json!({ "type": "object", "properties": {} })
            };
            tools_vec.push(ToolConfig {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters,
                script: canon_script,
                timeout_ms,
                kind,
                override_builtin: t.override_builtin,
            });
        }

        // --- plugin-declared OAuth provider (subscription auth, no recompile) ---
        let oauth_config = match manifest.oauth {
            Some(entry) => Some(load_oauth_entry(&canon_dir, entry)?),
            None => None,
        };

        Ok(Plugin {
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            enabled: true,
            source_path: canon_dir,
            hooks,
            tools: tools_vec,
            disable_tools: manifest.disable_tools,
            system_prompt: manifest.system_prompt,
            oauth: oauth_config,
        })
    }

    /// Install a plugin from `source_path` (a directory containing plugin.json).
    /// The plugin directory is copied into the managed plugins directory and
    /// registered. Returns an error if a plugin with the same name already exists
    /// or if validation fails.
    pub fn install(&self, source_path: &Path) -> Result<Plugin, String> {
        let source = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(source_path)
        };
        if !source.is_dir() {
            return Err(format!("{:?} is not a directory", source_path));
        }
        let manifest_path = source.join("plugin.json");
        if !manifest_path.exists() {
            return Err(format!("no plugin.json found in {:?}", source_path));
        }

        // Pre-validate the plugin from source before copying.
        let plugin = Self::load_plugin_from_dir(&source)?;

        // Check for name collision.
        {
            let plugins = self.plugins.read().unwrap();
            if plugins.contains_key(&plugin.name) {
                return Err(format!(
                    "plugin '{}' is already installed; remove it first or use a different name",
                    plugin.name
                ));
            }
        }

        let dest_dir = self.plugins_dir.join(&plugin.name);
        if dest_dir.exists() {
            let _ = std::fs::remove_dir_all(&dest_dir);
        }

        copy_dir(&source, &dest_dir)?;

        // Re-load from the copied location so paths point to the managed dir.
        let installed = Self::load_plugin_from_dir(&dest_dir)?;

        self.plugins
            .write()
            .unwrap()
            .insert(installed.name.clone(), installed.clone());

        Ok(installed)
    }

    /// Remove a plugin by name. Deletes the plugin directory from disk and
    /// unregisters it from the in-memory registry.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.remove(name) {
            let _ = std::fs::remove_dir_all(&plugin.source_path);
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Enable a previously-disabled plugin by name.
    pub fn enable(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.enabled = true;
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Disable a plugin by name (keeps it on disk, stops invoking hooks).
    pub fn disable(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.enabled = false;
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Return a snapshot of all registered plugins (name → Plugin).
    pub fn list(&self) -> HashMap<String, Plugin> {
        self.plugins.read().unwrap().clone()
    }

    /// Get all enabled hook configs for a given hook point name.
    /// Returns a vec of (plugin_name, HookConfig) pairs so the caller can
    /// iterate and merge results.
    pub fn get_hook_configs(&self, hook_name: &str) -> Vec<(String, HookConfig)> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .filter_map(|p| p.hooks.get(hook_name).map(|c| (p.name.clone(), c.clone())))
            .collect()
    }

    /// Cheap existence check (no config clone): does any enabled plugin register
    /// this hook point? Used to decide whether to clone tool args before the
    /// pre-hook phase without paying for a full `get_hook_configs`.
    pub fn has_hook(&self, hook_name: &str) -> bool {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .any(|p| p.hooks.contains_key(hook_name))
    }

    /// Look up a single plugin by name.
    pub fn get_plugin(&self, name: &str) -> Option<Plugin> {
        self.plugins.read().unwrap().get(name).cloned()
    }

    /// OpenAI function-calling tool definitions for every tool declared by
    /// ENABLED plugins. Built-in tools are NOT included here; the caller merges
    /// them and filters name collisions (a plugin tool may never shadow a
    /// built-in). Empty when no plugin declares tools.
    pub fn tool_definitions(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for p in self.plugins.read().unwrap().values().filter(|p| p.enabled) {
            for t in &p.tools {
                out.push(json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                }));
            }
        }
        out
    }

    /// Look up a plugin-declared tool's config by tool name (enabled plugins
    /// only). Returns None for built-in tools.
    pub fn tool_config(&self, name: &str) -> Option<ToolConfig> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .find_map(|p| p.tools.iter().find(|t| t.name == name).cloned())
    }

    /// Approval classification for a plugin-declared tool, if it exists.
    pub fn tool_kind(&self, name: &str) -> Option<ToolKind> {
        self.tool_config(name).map(|t| t.kind)
    }

    /// Union of tool names every enabled plugin asks to disable (the
    /// `disable_tools` manifest field). Applied as a FINAL filter on the
    /// model's tool list, so a disabled name is gone whether it's a built-in
    /// or an override — `disable_tools` is the strongest "remove a feature"
    /// lever and always wins over `override`.
    pub fn disabled_tools(&self) -> std::collections::HashSet<String> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .flat_map(|p| p.disable_tools.iter().cloned())
            .collect()
    }

    /// Built-in tool names for which an enabled plugin declares an
    /// `override: true` tool — the plugin's handler replaces the built-in's
    /// implementation. A plugin tool named like a built-in WITHOUT
    /// `override: true` does NOT appear here (it stays a no-op collision,
    /// built-in wins — unchanged behavior).
    pub fn overridden_tool_names(&self) -> std::collections::HashSet<String> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .flat_map(|p| p.tools.iter())
            .filter(|t| t.override_builtin && crate::tools::is_builtin(&t.name))
            .map(|t| t.name.clone())
            .collect()
    }

    /// Concatenated `system_prompt` text from every enabled plugin that
    /// declares one, each framed with its plugin name + version. Empty (so the
    /// system prompt + its prefix cache are untouched) when no plugin declares
    /// any. Lets a plugin inject domain rules / persona / context into the
    /// system prompt — the same surface a core edit of SYSTEM_PROMPT_BASE
    /// touches.
    pub fn system_prompt_injection(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for p in self.plugins.read().unwrap().values().filter(|p| p.enabled) {
            let s = p.system_prompt.trim();
            if !s.is_empty() {
                parts.push(format!("# Plugin: {} (v{})\n{}", p.name, p.version, s));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("\n\n## Plugin-injected context\n\n{}", parts.join("\n\n"))
        }
    }

    // ---- OAuth provider declarations (subscription auth, no recompile) ----

    /// All OAuth providers declared by enabled plugins (one per plugin that
    /// declares an `oauth` block). Used to populate the `/login` picker and to
    /// dispatch `/login` / `/oauth-code` / `/logout`.
    pub fn oauth_configs(&self) -> Vec<PluginOauthConfig> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .filter_map(|p| p.oauth.clone())
            .collect()
    }

    /// Look up a plugin-declared OAuth provider by its provider_id.
    pub fn oauth_config(&self, provider_id: &str) -> Option<PluginOauthConfig> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .find_map(|p| {
                p.oauth
                    .as_ref()
                    .filter(|o| o.provider_id == provider_id)
                    .cloned()
            })
    }

    /// Find the plugin OAuth provider that should authenticate a resolved
    /// provider at turn time. Matches by provider-config name == provider_id
    /// first (the `/login` flow creates the config with that name), then by
    /// base_url host (a manually-configured provider pointing at the plugin's
    /// declared endpoint).
    pub fn oauth_config_for_provider(&self, rp: &ResolvedProvider) -> Option<PluginOauthConfig> {
        let configs = self.oauth_configs();
        if let Some(c) = configs.iter().find(|c| c.provider_id == rp.name) {
            return Some(c.clone());
        }
        if let (Some(host),) = (url_host(&rp.base_url),) {
            if let Some(c) = configs
                .iter()
                .find(|c| url_host(&c.base_url).as_deref() == Some(host.as_str()))
            {
                return Some(c.clone());
            }
        }
        None
    }

    /// Build the `ProviderConfig` to create on a successful `/login` for a
    /// plugin OAuth provider (no api_key — the token is resolved + refreshed at
    /// turn time). `finalize_oauth` uses this in place of a built-in preset.
    pub fn oauth_provider_config(&self, provider_id: &str) -> Option<ProviderConfig> {
        let cfg = self.oauth_config(provider_id)?;
        Some(ProviderConfig {
            name: cfg.provider_id.clone(),
            kind: cfg.kind,
            base_url: cfg.base_url.clone(),
            api_key: None,
            api_key_env: None,
            headers: cfg.headers.clone(),
        })
    }

    /// True when a plugin declares an OAuth login flow for `provider_id` (the
    /// login action has a resolvable script).
    pub fn supports_oauth_login(&self, provider_id: &str) -> bool {
        let Some(cfg) = self.oauth_config(provider_id) else {
            return false;
        };
        cfg.script_for("login").is_some()
    }

    /// Cheap sync check (no subprocess): does the plugin's token file exist?
    /// Used to gate an OAuth-only provider into model aggregation so `/models`
    /// shows it without an API key.
    pub fn has_oauth_creds(&self, provider_id: &str) -> bool {
        self.oauth_config(provider_id)
            .map(|c| c.token_path.exists())
            .unwrap_or(false)
    }

    /// Delete the plugin's stored token + invalidate the cache. Called by
    /// `/logout` so the provider is fully logged out (not just its config).
    /// Best-effort: also invokes the plugin's `clear` action so the plugin can
    /// tear down any extra state it manages.
    pub async fn clear_oauth(&self, provider_id: &str) {
        if let Some(cfg) = self.oauth_config(provider_id) {
            let _ = std::fs::remove_file(&cfg.token_path);
            if let Some(script) = cfg.script_for("clear") {
                let ctx =
                    self.oauth_action_ctx("clear", provider_id, &cfg.token_path.to_string_lossy());
                let _ = self
                    .execute_oauth_script(script, ctx, cfg.token_timeout_ms)
                    .await;
            }
        }
        if let Ok(mut cache) = self.token_cache.lock() {
            cache.remove(provider_id);
        }
    }

    /// Resolve a fresh (cached) OAuth access token for `provider_id` at turn /
    /// discovery time. Spawns the plugin's `token` action only when the cached
    /// token is missing or near expiry. Returns None when no creds exist or the
    /// script fails — callers fall back to the API-key path (no regression).
    pub async fn resolve_oauth_token(&self, provider_id: &str) -> Option<String> {
        let cfg = self.oauth_config(provider_id)?;
        // Cache hit?
        {
            let cache = self.token_cache.lock().ok()?;
            if let Some(c) = cache.get(provider_id) {
                let now = now_secs();
                if c.expires_at == 0 || c.expires_at > now + 60 {
                    return Some(c.token.clone());
                }
            }
        }
        let script = cfg.script_for("token")?;
        let ctx = self.oauth_action_ctx("token", provider_id, &cfg.token_path.to_string_lossy());
        let resp = self
            .execute_oauth_script(script, ctx, cfg.token_timeout_ms)
            .await
            .ok()?;
        let token = resp
            .get("access_token")
            .and_then(|t| t.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        if let Some(t) = &token {
            let now = now_secs();
            let exp = resp
                .get("expires_at")
                .and_then(|e| e.as_u64())
                .filter(|e| *e > 0)
                .unwrap_or(now + 300);
            if let Ok(mut cache) = self.token_cache.lock() {
                cache.insert(
                    provider_id.to_string(),
                    CachedToken {
                        token: t.clone(),
                        expires_at: exp,
                    },
                );
            }
        }
        token
    }

    /// Drive the interactive OAuth login for a plugin provider. Picks the flow
    /// from the script's returned `flow` field:
    ///  - `web` (default for a local machine): the harness binds a loopback
    ///    redirect, the script builds the authorize URL with that redirect_uri,
    ///    the harness waits for the browser redirect, then calls `complete`.
    ///  - `manual` (default for SSH/headless, or when the script chooses it):
    ///    the script returns a URL + an opaque `pending` blob; the user pastes
    ///    the code back via `/oauth-code`, which calls `complete`.
    pub async fn oauth_login(
        &self,
        provider_id: &str,
        emit: &(dyn Fn(OAuthPrompt) + Send + Sync),
    ) -> Result<LoginOutcome, String> {
        let cfg = self
            .oauth_config(provider_id)
            .ok_or_else(|| format!("'{provider_id}' has no plugin OAuth login flow"))?;
        let token_path = cfg.token_path.to_string_lossy().to_string();
        let headless = crate::oauth::likely_headless();

        if !headless {
            // Web flow: bind a loopback redirect the script embeds in its URL.
            let (listener, listener_v6, port) = crate::oauth::bind_loopback(0).await?;
            let redirect_uri = format!("http://localhost:{port}/callback");
            let mut ctx = self.oauth_action_ctx("login", provider_id, &token_path);
            ctx["headless"] = json!(false);
            ctx["redirect_uri"] = json!(redirect_uri);
            let resp = self
                .execute_oauth_script(
                    cfg.script_for("login").ok_or("no login script")?,
                    ctx,
                    cfg.login_timeout_ms,
                )
                .await?;
            let url = resp
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("login script did not return a url")?
                .to_string();
            let flow = resp
                .get("flow")
                .and_then(|v| v.as_str())
                .unwrap_or("web")
                .to_string();
            let code = resp.get("code").and_then(|v| v.as_str()).map(String::from);
            let state = resp
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let message = resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Open the URL to log in.")
                .to_string();
            let pending = resp.get("pending").cloned();
            emit(OAuthPrompt {
                url: url.clone(),
                code,
                message,
            });
            let _ = crate::oauth::open_browser(&url);

            if flow == "manual" {
                // The script insisted on the manual flow even locally.
                return Ok(LoginOutcome::AwaitingCode {
                    pending: PendingOauth::plugin(provider_id, state, pending),
                });
            }
            // Wait for the browser redirect, then complete the exchange.
            let code =
                crate::oauth::await_redirect_dual(listener, listener_v6, &state, None).await?;
            let mut ctx = self.oauth_action_ctx("complete", provider_id, &token_path);
            ctx["code"] = json!(code);
            ctx["redirect_uri"] = json!(redirect_uri);
            if let Some(p) = &pending {
                ctx["pending"] = p.clone();
            }
            let resp = self
                .execute_oauth_script(
                    cfg.script_for("complete").ok_or("no complete script")?,
                    ctx,
                    cfg.login_timeout_ms,
                )
                .await?;
            if !resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                let err = resp
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(format!("OAuth complete failed: {err}"));
            }
            // Invalidate the cache so the next turn resolves the fresh token.
            if let Ok(mut cache) = self.token_cache.lock() {
                cache.remove(provider_id);
            }
            Ok(LoginOutcome::Done)
        } else {
            // Manual / device-code flow: emit the URL, stash `pending`, wait
            // for the user to paste the code via `/oauth-code`.
            let mut ctx = self.oauth_action_ctx("login", provider_id, &token_path);
            ctx["headless"] = json!(true);
            let resp = self
                .execute_oauth_script(
                    cfg.script_for("login").ok_or("no login script")?,
                    ctx,
                    cfg.login_timeout_ms,
                )
                .await?;
            let url = resp
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("login script did not return a url")?
                .to_string();
            let code = resp.get("code").and_then(|v| v.as_str()).map(String::from);
            let message = resp
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Open the URL, approve, then paste the code via /oauth-code.")
                .to_string();
            let pending = resp.get("pending").cloned();
            emit(OAuthPrompt { url, code, message });
            Ok(LoginOutcome::AwaitingCode {
                pending: PendingOauth::plugin(provider_id, String::new(), pending),
            })
        }
    }

    /// Complete a pending manual (paste-code) plugin OAuth login: exchange the
    /// code for a token via the plugin's `complete` action. The plugin writes
    /// the token to its token_path; the harness never parses the token format.
    pub async fn oauth_complete(
        &self,
        provider_id: &str,
        pending: &PendingOauth,
        code: &str,
    ) -> Result<(), String> {
        let cfg = self
            .oauth_config(provider_id)
            .ok_or_else(|| format!("'{provider_id}' has no plugin OAuth flow"))?;
        let script = cfg
            .script_for("complete")
            .ok_or_else(|| format!("'{provider_id}' has no complete script"))?;
        let token_path = cfg.token_path.to_string_lossy().to_string();
        let mut ctx = self.oauth_action_ctx("complete", provider_id, &token_path);
        ctx["code"] = json!(code);
        if let Some(p) = &pending.plugin_pending {
            ctx["pending"] = p.clone();
        }
        let resp = self
            .execute_oauth_script(script, ctx, cfg.login_timeout_ms)
            .await?;
        if !resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("OAuth complete failed: {err}"));
        }
        // Invalidate the cache so the next turn resolves the fresh token.
        if let Ok(mut cache) = self.token_cache.lock() {
            cache.remove(provider_id);
        }
        Ok(())
    }

    /// Build the base context JSON passed to every OAuth script invocation
    /// (action-specific fields are added by the caller via `ctx["..."] = ...`).
    fn oauth_action_ctx(&self, action: &str, provider_id: &str, token_path: &str) -> Value {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        json!({
            "action": action,
            "provider_id": provider_id,
            "token_path": token_path,
            "workspace": self.workspace.to_string_lossy(),
            "timestamp": timestamp,
        })
    }

    /// Spawn an OAuth script, write the context JSON to its stdin, read one
    /// JSON object from stdout. Bounded by `timeout_ms` (stdin-write + wait).
    /// Mirrors `execute_hook` / `execute_plugin_tool` (kill_on_drop, bounded
    /// stdin write, timeout). Non-zero exit / timeout / parse failure → Err.
    async fn execute_oauth_script(
        &self,
        script: &Path,
        context: Value,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        let ctx_bytes = serde_json::to_vec(&context).unwrap_or_default();
        let mut child = match hook_command(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Err(format!("failed to spawn oauth script {script:?}: {e}")),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let stdin_timeout = Duration::from_millis(timeout_ms.max(1000));
            let write_fut = async {
                let _ = stdin.write_all(&ctx_bytes).await;
                let _ = stdin.shutdown().await;
            };
            if tokio::time::timeout(stdin_timeout, write_fut)
                .await
                .is_err()
            {
                let _ = child.start_kill();
                return Err(format!(
                    "oauth script did not consume stdin within {}ms",
                    stdin_timeout.as_millis()
                ));
            }
        }
        let timeout_dur = Duration::from_millis(timeout_ms);
        match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!(
                        "oauth script exited with {}: {}",
                        output.status,
                        stderr.trim()
                    ));
                }
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if stdout.is_empty() {
                    return Err("oauth script returned empty stdout".into());
                }
                serde_json::from_str::<Value>(&stdout)
                    .map_err(|e| format!("oauth script returned invalid JSON: {e}"))
            }
            Ok(Err(e)) => Err(format!("oauth script wait error: {e}")),
            Err(_) => Err(format!("oauth script timed out after {}ms", timeout_ms)),
        }
    }
}

// ---- hook execution ----

/// Execute a single hook script and return its result.
///
/// The hook receives `context` JSON on stdin. It must write a JSON response
/// (see PLUGIN_DOCS for schema) to stdout. The function handles timeouts,
/// non-zero exits, and parse failures according to the safety rules:
///
/// - **pre_* hooks**: non-zero exit, timeout, or parse failure → deny
/// - **post_* / lifecycle hooks**: non-zero exit, timeout, or parse failure → skip
///
/// The `hook_name` prefix ("pre_" vs "post_" etc.) determines the safety rule.
/// Disabled plugin checks are handled before calling this function.
pub async fn execute_hook(
    hook_name: &str,
    plugin_name: &str,
    config: &HookConfig,
    context: &Value,
) -> HookResult {
    let is_pre = hook_name.starts_with("pre_");

    let deny = |reason: String| HookResult {
        allow: false,
        reason,
        modify: None,
    };

    let skip = |reason: String| HookResult {
        allow: true,
        reason: format!("[{plugin_name}] {reason}"),
        modify: None,
    };

    // Spawn the hook script.
    let script_path = &config.script;
    let mut child = match hook_command(script_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to spawn hook script {:?}: {e}", script_path);
            return if is_pre { deny(msg) } else { skip(msg) };
        }
    };

    // Write the context JSON to stdin, then close it so the script can proceed.
    // The write can block indefinitely if a hook with pass_args doesn't drain
    // its stdin (a >64KB payload exceeds the pipe buffer). Bound it by the
    // hook's own timeout so a wedged hook can't hang the turn forever; on
    // timeout kill the child and deny/skip (P1-9).
    let context_bytes = serde_json::to_vec(context).unwrap_or_default();
    if let Some(mut stdin) = child.stdin.take() {
        let stdin_timeout = Duration::from_millis(config.timeout_ms.max(1000));
        let write_fut = async {
            let _ = stdin.write_all(&context_bytes).await;
            let _ = stdin.shutdown().await;
        };
        if tokio::time::timeout(stdin_timeout, write_fut)
            .await
            .is_err()
        {
            let _ = child.start_kill();
            let msg = format!(
                "hook '{}' did not consume stdin within {}ms",
                hook_name,
                stdin_timeout.as_millis()
            );
            return if is_pre { deny(msg) } else { skip(msg) };
        }
    }

    let timeout_dur = Duration::from_millis(config.timeout_ms);
    let output_result = tokio::time::timeout(timeout_dur, child.wait_with_output()).await;

    match output_result {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let msg = format!(
                    "hook '{}' exited with {}: {}",
                    hook_name,
                    output.status,
                    stderr.trim()
                );
                return if is_pre { deny(msg) } else { skip(msg) };
            }

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stdout.is_empty() {
                let msg = format!("hook '{}' returned empty stdout", hook_name);
                return if is_pre { deny(msg) } else { skip(msg) };
            }

            let response: Value = match serde_json::from_str(&stdout) {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("hook '{}' returned invalid JSON: {e}", hook_name);
                    return if is_pre { deny(msg) } else { skip(msg) };
                }
            };

            let allow = response
                .get("allow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let reason = response
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let modify = response.get("modify").cloned();

            // For post hooks, we never block — "allow: false" just means
            // the hook observed an issue, it doesn't roll back the operation.
            if !is_pre && !allow {
                return HookResult {
                    allow: true,
                    reason: format!("[{plugin_name}] {reason}"),
                    modify: None,
                };
            }

            HookResult {
                allow,
                reason,
                modify,
            }
        }
        Ok(Err(e)) => {
            let msg = format!("hook '{}' wait error: {e}", hook_name);
            if is_pre {
                deny(msg)
            } else {
                skip(msg)
            }
        }
        Err(_elapsed) => {
            let msg = format!(
                "hook '{}' timed out after {}ms",
                hook_name, config.timeout_ms
            );
            if is_pre {
                deny(msg)
            } else {
                skip(msg)
            }
        }
    }
}

/// Build the standard hook context JSON object.
///
/// The caller supplies the hook point name, tool name (empty string for
/// lifecycle hooks), workspace path, optional tool args, and session id.
/// If `pass_args` is false, the `args` field is omitted from the context.
pub fn build_context(
    hook_name: &str,
    tool_name: &str,
    workspace: &str,
    args: Option<&Value>,
    session_id: &str,
    pass_args: bool,
) -> Value {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut ctx = serde_json::json!({
        "hook": hook_name,
        "tool": tool_name,
        "workspace": workspace,
        "session_id": session_id,
        "timestamp": timestamp,
    });

    if pass_args {
        if let Some(a) = args {
            if let Some(obj) = ctx.as_object_mut() {
                obj.insert("args".to_string(), a.clone());
            }
        }
    }

    ctx
}

/// Merge a hook's `modify` object over the running tool args, in place.
///
/// `modify` is a JSON object whose keys override the corresponding keys in
/// `args` (shallow, per-key). Keys absent from `modify` are left untouched, so
/// a pre_write hook can return `{"content": "..."}` to reformat content while
/// preserving `path`/`edits`, a pre_bash hook can return `{"command": "..."}`
/// to fix a command, and a pre_read hook can return `{"path": "..."}` to
/// redirect a read. Non-object `modify` (or non-object `args`) is a no-op so a
/// malformed hook never corrupts the tool call.
pub fn apply_modify(args: &mut Value, modify: &Value) {
    if let (Some(base), Some(over)) = (args.as_object_mut(), modify.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
}

/// Execute a plugin-declared tool by spawning its handler script.
///
/// The handler receives one JSON object on stdin:
/// ```json
/// { "args": {…}, "workspace": "/abs/path", "session_id": "x.jsonl", "timestamp": 1719000000 }
/// ```
/// It must write one JSON object to stdout:
/// ```json
/// { "ok": true,  "output": "result text shown to the model" }
/// { "ok": false, "output": "error message" }      // `ok` omitted defaults to true
/// ```
/// A bare non-JSON stdout is accepted as the output text with `ok=true` (so a
/// trivial `echo` handler works). Non-zero exit, timeout, or spawn failure
/// produce an error `Outcome` — the conversation continues; the tool call
/// failed from the model's point of view. Safety mirrors `execute_hook`:
/// stdin-write is bounded by the tool's timeout, and the child is
/// `kill_on_drop` so a dropped future frees it.
pub async fn execute_plugin_tool(
    tool_name: &str,
    config: &ToolConfig,
    args: &Value,
    workspace: &str,
    session_id: &str,
) -> Outcome {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ctx = json!({
        "args": args,
        "workspace": workspace,
        "session_id": session_id,
        "timestamp": timestamp,
    });
    let ctx_bytes = serde_json::to_vec(&ctx).unwrap_or_default();

    let mut child = match hook_command(&config.script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Outcome::err(format!(
                "plugin tool '{}' failed to spawn handler: {e}",
                tool_name
            ))
        }
    };

    // Write the args context to stdin, bounded by the tool's timeout so a
    // handler that never drains its stdin can't hang the turn forever.
    if let Some(mut stdin) = child.stdin.take() {
        let stdin_timeout = Duration::from_millis(config.timeout_ms.max(1000));
        let write_fut = async {
            let _ = stdin.write_all(&ctx_bytes).await;
            let _ = stdin.shutdown().await;
        };
        if tokio::time::timeout(stdin_timeout, write_fut)
            .await
            .is_err()
        {
            let _ = child.start_kill();
            return Outcome::err(format!(
                "plugin tool '{}' handler did not consume stdin within {}ms",
                tool_name,
                stdin_timeout.as_millis()
            ));
        }
    }

    let timeout_dur = Duration::from_millis(config.timeout_ms);
    match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Outcome::err(format!(
                    "plugin tool '{}' handler exited with {}: {}",
                    tool_name,
                    output.status,
                    stderr.trim()
                ));
            }
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stdout.is_empty() {
                return Outcome::err(format!(
                    "plugin tool '{}' handler returned empty stdout",
                    tool_name
                ));
            }
            // Structured {ok, output} | {error} | {result}; else accept raw text.
            match serde_json::from_str::<Value>(&stdout) {
                Ok(v) if v.is_object() => {
                    if let Some(err) = v
                        .get("error")
                        .and_then(|e| e.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        Outcome::err(format!("plugin tool '{}': {}", tool_name, err))
                    } else {
                        let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(true);
                        let output_text = v
                            .get("output")
                            .or_else(|| v.get("result"))
                            .and_then(|o| o.as_str())
                            .map(String::from)
                            .unwrap_or_else(|| stdout.clone());
                        if ok {
                            Outcome::ok(output_text)
                        } else {
                            Outcome::err(output_text)
                        }
                    }
                }
                Ok(_) => Outcome::ok(stdout),
                Err(_) => Outcome::ok(stdout),
            }
        }
        Ok(Err(e)) => Outcome::err(format!(
            "plugin tool '{}' handler wait error: {e}",
            tool_name
        )),
        Err(_) => Outcome::err(format!(
            "plugin tool '{}' handler timed out after {}ms",
            tool_name, config.timeout_ms
        )),
    }
}

// ---- helpers ----

/// Return the default timeout for a hook point.
fn default_hook_timeout(hook_name: &str) -> u64 {
    if hook_name.starts_with("pre_") {
        DEFAULT_PRE_TIMEOUT_MS
    } else {
        DEFAULT_POST_TIMEOUT_MS
    }
}

/// Validate a script path declared by a plugin (hook or tool): reject `..`
/// escapes, require the canonicalized path to stay within the plugin's
/// canonical directory, and confirm it exists and is executable. Returns the
/// canonical script path on success. Used for plugin-declared tools; the hook
/// loader does the same checks inline (kept separate to avoid disturbing the
/// proven hook path + its exact error messages).
fn validate_plugin_script(canon_dir: &Path, script_rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(script_rel);
    {
        use std::path::Component;
        for comp in rel.components() {
            if let Component::ParentDir = comp {
                return Err(format!(
                    "script {:?} escapes the plugin directory",
                    script_rel
                ));
            }
        }
    }
    let abs = canon_dir.join(rel);
    let canon = std::fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
    if !canon.starts_with(canon_dir) {
        return Err(format!(
            "script {:?} escapes the plugin directory",
            script_rel
        ));
    }
    if !canon.exists() {
        return Err(format!("script {:?} does not exist", script_rel));
    }
    if !is_executable(&canon) {
        return Err(format!(
            "script {:?} is not executable (try chmod +x)",
            script_rel
        ));
    }
    Ok(canon)
}

/// Resolve a plugin manifest `oauth` block into a loaded [`PluginOauthConfig`]:
/// validates the provider_id/base_url/kind, resolves the token-file path under
/// `~/.config/catalyst-code/oauth/`, and resolves every declared script (shared
/// `script` default + per-action overrides) to an absolute, path-confined,
/// executable file. Token resolution is mandatory (a provider that can never
/// produce a token is useless); login/complete fall back to the shared script
/// and error at runtime only if neither exists.
fn load_oauth_entry(
    canon_dir: &Path,
    entry: OauthManifestEntry,
) -> Result<PluginOauthConfig, String> {
    let provider_id = entry.provider_id.clone();
    if provider_id.is_empty() {
        return Err("oauth provider_id is empty".into());
    }
    if entry.base_url.is_empty() {
        return Err(format!(
            "oauth provider '{provider_id}' has an empty base_url"
        ));
    }
    let kind = match entry.kind.as_deref().unwrap_or("openai") {
        "openai" => ProviderKind::OpenAI,
        "anthropic" => ProviderKind::Anthropic,
        other => {
            return Err(format!(
                "oauth provider '{provider_id}' has invalid kind '{other}' (use 'openai' or 'anthropic')"
            ))
        }
    };
    // Token file lives under ~/.config/catalyst-code/oauth/ (created lazily by
    // the plugin's complete/token scripts on first write).
    let token_dir = crate::config::home_dir()
        .map(|h| h.join(".config/catalyst-code/oauth"))
        .unwrap_or_else(|| PathBuf::from(".config/catalyst-code/oauth"));
    let token_name = entry
        .token_path
        .clone()
        .unwrap_or_else(|| format!("{provider_id}.json"));
    let token_path = token_dir.join(&token_name);

    // Resolve the shared default (keyed "*") + per-action overrides.
    let mut scripts: HashMap<String, PathBuf> = HashMap::new();
    if let Some(s) = &entry.script {
        scripts.insert("*".to_string(), validate_plugin_script(canon_dir, s)?);
    }
    for (action, opt) in [
        ("login", &entry.login_script),
        ("complete", &entry.complete_script),
        ("token", &entry.token_script),
    ] {
        if let Some(s) = opt {
            scripts.insert(action.to_string(), validate_plugin_script(canon_dir, s)?);
        }
    }
    // Token resolution is essential — without it the provider can never
    // authenticate a turn.
    if scripts.get("token").or_else(|| scripts.get("*")).is_none() {
        return Err(format!(
            "oauth provider '{provider_id}' has no token script: set 'script' (handles all actions) or 'token_script'"
        ));
    }

    let label = entry.label.unwrap_or_else(|| provider_id.clone());
    Ok(PluginOauthConfig {
        provider_id,
        label,
        kind,
        base_url: entry.base_url,
        description: entry.description.unwrap_or_default(),
        headers: entry.headers,
        token_path,
        scripts,
        login_timeout_ms: entry.login_timeout_ms.unwrap_or(120_000),
        token_timeout_ms: entry.token_timeout_ms.unwrap_or(30_000),
    })
}

/// Cross-platform check for whether a hook script is executable.
/// - Unix: any executable permission bit set (owner/group/other).
/// - Windows / non-Unix: no permission bit exists, so any file that exists
///   counts as executable (the OS governs launch by extension; a bad or
///   missing interpreter surfaces as a spawn error at hook execution time).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Current unix time in seconds (0 on clock error). Used by the OAuth token
/// cache expiry check.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Extract the lowercased host from a URL (best-effort, no `url` crate dep).
/// `https://api.x.ai/v1` → `api.x.ai`. Used to match a manually-configured
/// provider to a plugin OAuth declaration by endpoint host.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, h)| h).unwrap_or(url);
    let host = rest.split(['/', ':']).next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Pick a Python interpreter, preferring `python3` then `python`, cached.
/// Falls back to `python3` (which will fail-to-spawn gracefully if truly
/// absent) so a missing interpreter never panics.
fn python_interpreter() -> String {
    use std::sync::OnceLock;
    static INTERP: OnceLock<String> = OnceLock::new();
    INTERP
        .get_or_init(|| {
            for cand in ["python3", "python"] {
                if let Ok(o) = std::process::Command::new(cand).arg("--version").output() {
                    if o.status.success() {
                        return cand.to_string();
                    }
                }
            }
            "python3".to_string()
        })
        .clone()
}

/// Build the command to run a hook script, selecting the right interpreter by
/// extension so plugins work cross-platform. On Unix a shebang handles `*.sh`;
/// on Windows `.bat`/`.cmd`/`.exe` launch directly, `.ps1` uses powershell,
/// `.py` uses python, and `.sh`/`.bash` use `bash` (Git Bash/WSL) when present.
/// `CATALYST_CODE_SHELL` overrides the interpreter for `.sh`/`.bash`.
fn hook_command(script: &Path) -> Command {
    let ext = script
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "bat" | "cmd" | "exe" | "com" => Command::new(script),
        "ps1" => {
            let mut c = Command::new("powershell");
            c.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(script);
            c
        }
        "py" => {
            // Prefer `python3` (present on most Linux/macOS); fall back to
            // `python` (common on Windows / some distros). Launching a `.py`
            // hook as `python` on a python3-only system fails to spawn, and for
            // the advisory pre_turn hook that silently skips it — so vision
            // handoff would never run. Probe once and cache the interpreter.
            let mut c = Command::new(python_interpreter());
            c.arg(script);
            c
        }
        "sh" | "bash" => {
            // Prefer an explicit override, then bash (Git Bash/WSL on Windows).
            // On bare Windows without bash the spawn fails → graceful pre-hook deny.
            if let Ok(shell) = std::env::var("CATALYST_CODE_SHELL") {
                let mut c = Command::new(shell);
                c.arg(script);
                c
            } else {
                let mut c = Command::new("bash");
                c.arg(script);
                c
            }
        }
        _ => Command::new(script),
    }
}

/// Recursively copy a directory from `src` to `dst`.
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {:?}: {e}", dst))?;

    let rd = std::fs::read_dir(src).map_err(|e| format!("read_dir {:?}: {e}", src))?;

    for entry in rd {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type error: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ft.is_dir() {
            copy_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("copy {:?} -> {:?}: {e}", src_path, dst_path))?;
        }
    }
    Ok(())
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {
        // No executable bit on Windows; hooks launch by extension.
    }

    /// Create a temporary directory that is cleaned up on drop.
    struct TmpDir {
        path: PathBuf,
    }

    impl TmpDir {
        fn new(prefix: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let path =
                std::env::temp_dir().join(format!("catalyst_code_plugin_test_{}_{}", prefix, n));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TmpDir { path }
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Write a minimal executable shell script that outputs the given JSON.
    fn write_hook_script(dir: &Path, name: &str, stdout_json: &str, exit_code: u32) -> PathBuf {
        let script = dir.join(name);
        let content = format!("#!/bin/sh\necho '{}'\nexit {}\n", stdout_json, exit_code);
        fs::write(&script, &content).unwrap();
        make_executable(&script);
        script
    }

    /// Write a complete plugin to a directory.
    fn write_plugin(dir: &Path, name: &str, version: &str, hooks_json: &str) {
        let manifest = format!(
            r#"{{
  "name": "{}",
  "version": "{}",
  "description": "Test plugin",
  "hooks": {}
}}"#,
            name, version, hooks_json
        );
        fs::write(dir.join("plugin.json"), manifest).unwrap();
    }

    // ---- manifest loading ----

    #[test]
    fn load_minimal_plugin() {
        let tmp = TmpDir::new("load_minimal");
        write_plugin(&tmp.path, "minimal", "1.0.0", "{}");
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(plugin.name, "minimal");
        assert_eq!(plugin.version, "1.0.0");
        assert_eq!(plugin.hooks.len(), 0);
        assert!(plugin.enabled);
    }

    #[test]
    fn load_plugin_with_hooks() {
        let tmp = TmpDir::new("load_with_hooks");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        let pre_script = write_hook_script(&hooks_dir, "pre_write.sh", r#"{"allow":true}"#, 0);
        let post_script = write_hook_script(&hooks_dir, "post_bash.sh", r#"{"allow":true}"#, 0);

        // Use relative paths in the manifest.
        write_plugin(
            &tmp.path,
            "with-hooks",
            "0.2.0",
            r#"{
          "pre_write": { "script": "hooks/pre_write.sh", "timeout_ms": 7000, "pass_args": true },
          "post_bash": { "script": "hooks/post_bash.sh" }
        }"#,
        );

        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(plugin.hooks.len(), 2);

        let pre = plugin.hooks.get("pre_write").unwrap();
        assert_eq!(pre.script, std::fs::canonicalize(&pre_script).unwrap());
        assert_eq!(pre.timeout_ms, 7000);
        assert!(pre.pass_args);

        let post = plugin.hooks.get("post_bash").unwrap();
        assert_eq!(post.script, std::fs::canonicalize(&post_script).unwrap());
        assert_eq!(post.timeout_ms, DEFAULT_POST_TIMEOUT_MS);
        assert!(!post.pass_args);
    }

    #[test]
    fn load_rejects_missing_script() {
        let tmp = TmpDir::new("load_missing_script");
        write_plugin(
            &tmp.path,
            "bad",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/nonexistent.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn load_rejects_non_executable() {
        let tmp = TmpDir::new("load_not_exe");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        let script = hooks_dir.join("hook.sh");
        fs::write(&script, "#!/bin/sh\necho ok\n").unwrap();
        // Leave without +x.
        write_plugin(
            &tmp.path,
            "not-exe",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/hook.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not executable"));
    }

    #[test]
    fn load_rejects_script_escape() {
        let tmp = TmpDir::new("load_escape");
        write_plugin(
            &tmp.path,
            "escape-artist",
            "1.0.0",
            r#"{"pre_write": {"script": "../hooks/outside.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes"));
    }

    #[test]
    fn load_skips_unknown_hook() {
        let tmp = TmpDir::new("load_unknown_hook");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        write_hook_script(&hooks_dir, "hook.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &tmp.path,
            "weird",
            "1.0.0",
            r#"{"pre_launch_missiles": {"script": "hooks/hook.sh"}}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert!(plugin.hooks.is_empty()); // unknown hook skipped
    }

    #[test]
    fn load_rejects_bad_json() {
        let tmp = TmpDir::new("load_bad_json");
        fs::write(tmp.path.join("plugin.json"), "not valid {{{").unwrap();
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
    }

    #[test]
    fn load_rejects_empty_name() {
        let tmp = TmpDir::new("load_empty_name");
        write_plugin(&tmp.path, "", "1.0.0", "{}");
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
    }

    // ---- PluginManager lifecycle ----

    #[test]
    fn manager_loads_plugins_on_new() {
        let tmp = TmpDir::new("mgr_loads");
        let plugin_dir = tmp.path.join("test-plugin");
        fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
        write_hook_script(&plugin_dir.join("hooks"), "hook.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &plugin_dir,
            "test-plugin",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/hook.sh"}}"#,
        );

        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__pm_test_ws__"), true);
        let plugins = mgr.list();
        assert_eq!(plugins.len(), 1);
        assert!(plugins.contains_key("test-plugin"));
    }

    #[test]
    fn project_plugins_skipped_unless_trusted() {
        // A plugin shipped inside the workspace (project-scoped) must be skipped
        // when trust_project is false — a repo must not auto-run its own hooks.
        let tmp = TmpDir::new("proj_skip");
        // workspace == the tmp dir; plugin under <tmp>/.catalyst-code/plugins/x
        let plugins_dir = tmp.path.join(".catalyst-code/plugins");
        let plugin_dir = plugins_dir.join("shady");
        fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
        write_hook_script(&plugin_dir.join("hooks"), "hook.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &plugin_dir,
            "shady",
            "1.0.0",
            r#"{"pre_bash":{"script":"hooks/hook.sh"}}"#,
        );
        let mgr = PluginManager::new(plugins_dir.clone(), tmp.path.clone(), false);
        assert!(
            mgr.list().is_empty(),
            "project plugin should be skipped when trust=false"
        );
        let skipped = mgr.skipped_project_plugins();
        assert_eq!(skipped, vec!["shady".to_string()]);

        // Same plugin loads when trust_project is true.
        let mgr2 = PluginManager::new(plugins_dir, tmp.path.clone(), true);
        assert_eq!(mgr2.list().len(), 1);
        assert!(mgr2.skipped_project_plugins().is_empty());
    }

    #[test]
    fn global_user_plugins_load_alongside_project() {
        // A plugin in the user (global) plugins dir loads regardless of
        // trust_project (it lives outside the workspace), and a same-named
        // project plugin overrides it.
        let ws = TmpDir::new("glob_ws");
        let global = TmpDir::new("glob_user_plugins");
        let proj_plugins = ws.path.join(".catalyst-code/plugins");

        // Global plugin "vision-fake" (outside the workspace).
        let gdir = global.path.join("vision-fake");
        fs::create_dir_all(gdir.join("hooks")).unwrap();
        write_hook_script(&gdir.join("hooks"), "h.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &gdir,
            "vision-fake",
            "1.0.0",
            r#"{"pre_turn":{"script":"hooks/h.sh"}}"#,
        );

        // trust_project=false, no project plugins present: global loads, none skipped.
        let mgr = PluginManager::new_with_user_plugins_dir(
            proj_plugins.clone(),
            Some(global.path.clone()),
            ws.path.clone(),
            false,
        );
        assert!(mgr.list().contains_key("vision-fake"));
        assert!(mgr.skipped_project_plugins().is_empty());

        // Add a same-named project plugin inside the workspace.
        let pdir = proj_plugins.join("vision-fake");
        fs::create_dir_all(pdir.join("hooks")).unwrap();
        write_hook_script(&pdir.join("hooks"), "h.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &pdir,
            "vision-fake",
            "9.9.9",
            r#"{"pre_turn":{"script":"hooks/h.sh"}}"#,
        );

        // trust_project=true: project plugin loads and OVERRIDES the global one.
        let mgr2 = PluginManager::new_with_user_plugins_dir(
            proj_plugins.clone(),
            Some(global.path.clone()),
            ws.path.clone(),
            true,
        );
        assert_eq!(mgr2.list().get("vision-fake").unwrap().version, "9.9.9");

        // trust_project=false: the project plugin is skipped (recorded), and
        // the global one still loads.
        let mgr3 = PluginManager::new_with_user_plugins_dir(
            proj_plugins,
            Some(global.path.clone()),
            ws.path.clone(),
            false,
        );
        assert_eq!(mgr3.list().get("vision-fake").unwrap().version, "1.0.0");
        assert_eq!(
            mgr3.skipped_project_plugins(),
            vec!["vision-fake".to_string()]
        );
    }

    #[test]
    fn install_and_remove_plugin() {
        let tmp = TmpDir::new("mgr_install");
        let mgr = PluginManager::new(
            tmp.path.join("managed"),
            PathBuf::from("/__pm_test_ws__"),
            true,
        );

        // Create a plugin source dir. (install target is outside the test
        // workspace dummy, so it loads regardless of trust_project.)
        let src = TmpDir::new("install_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(&src.path.join("hooks"), "hook.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &src.path,
            "fresh",
            "2.0.0",
            r#"{"post_write": {"script": "hooks/hook.sh"}}"#,
        );

        let installed = mgr.install(&src.path).unwrap();
        assert_eq!(installed.name, "fresh");
        assert_eq!(installed.version, "2.0.0");

        // Check that it was copied into the managed dir.
        assert!(mgr.list().contains_key("fresh"));
        assert!(tmp.path.join("managed/fresh/plugin.json").exists());

        // Remove it.
        mgr.remove("fresh").unwrap();
        assert!(mgr.list().is_empty());
        assert!(!tmp.path.join("managed/fresh").exists());
    }

    #[test]
    fn install_rejects_duplicate() {
        let tmp = TmpDir::new("mgr_dup");
        let mgr = PluginManager::new(
            tmp.path.join("managed"),
            PathBuf::from("/__pm_test_ws__"),
            true,
        );

        let src = TmpDir::new("dup_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(&src.path.join("hooks"), "h.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &src.path,
            "dup",
            "1.0.0",
            r#"{"pre_read": {"script": "hooks/h.sh"}}"#,
        );

        mgr.install(&src.path).unwrap();
        let result = mgr.install(&src.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already installed"));
    }

    #[test]
    fn enable_disable_toggle() {
        let tmp = TmpDir::new("mgr_toggle");
        let mgr = PluginManager::new(
            tmp.path.join("managed"),
            PathBuf::from("/__pm_test_ws__"),
            true,
        );

        let src = TmpDir::new("toggle_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(&src.path.join("hooks"), "h.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &src.path,
            "toggle-me",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/h.sh"}}"#,
        );

        mgr.install(&src.path).unwrap();

        // Initially enabled.
        assert!(mgr.get_plugin("toggle-me").unwrap().enabled);

        mgr.disable("toggle-me").unwrap();
        assert!(!mgr.get_plugin("toggle-me").unwrap().enabled);

        mgr.enable("toggle-me").unwrap();
        assert!(mgr.get_plugin("toggle-me").unwrap().enabled);

        // Disabled plugins are excluded from hook configs.
        mgr.disable("toggle-me").unwrap();
        let configs = mgr.get_hook_configs("pre_write");
        assert!(configs.is_empty());
    }

    #[test]
    fn enable_disable_unknown_is_error() {
        let tmp = TmpDir::new("mgr_unknown");
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__pm_test_ws__"), true);
        assert!(mgr.enable("nope").is_err());
        assert!(mgr.disable("nope").is_err());
        assert!(mgr.remove("nope").is_err());
    }

    // ---- execute_hook ----

    #[tokio::test]
    async fn execute_hook_allow() {
        let tmp = TmpDir::new("exec_allow");
        let script = write_hook_script(
            &tmp.path,
            "allow.sh",
            r#"{"allow": true, "reason": "all good"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.reason, "all good");
        assert!(result.modify.is_none());
    }

    #[tokio::test]
    async fn execute_hook_deny() {
        let tmp = TmpDir::new("exec_deny");
        let script = write_hook_script(
            &tmp.path,
            "deny.sh",
            r#"{"allow": false, "reason": "blocked"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert_eq!(result.reason, "blocked");
    }

    #[test]
    fn apply_modify_overrides_only_listed_keys() {
        // pre_write: reformat content, keep path/edits.
        let mut args = json!({ "path": "src/lib.rs", "content": "fn main(){}" });
        let modify = json!({ "content": "fn main() {\n}\n" });
        apply_modify(&mut args, &modify);
        assert_eq!(args["path"], json!("src/lib.rs"));
        assert_eq!(args["content"], json!("fn main() {\n}\n"));
    }

    #[test]
    fn apply_modify_pre_bash_replaces_command_only() {
        let mut args = json!({ "command": "rm -rf /", "timeout": 30 });
        let modify = json!({ "command": "echo safe" });
        apply_modify(&mut args, &modify);
        assert_eq!(args["command"], json!("echo safe"));
        assert_eq!(
            args["timeout"],
            json!(30),
            "unrelated keys must be preserved"
        );
    }

    #[test]
    fn apply_modify_pre_read_redirects_path() {
        let mut args = json!({ "path": "a.txt", "context": 3 });
        let modify = json!({ "path": "b.txt" });
        apply_modify(&mut args, &modify);
        assert_eq!(args["path"], json!("b.txt"));
        assert_eq!(args["context"], json!(3));
    }

    #[test]
    fn apply_modify_composes_across_hooks() {
        // Two hooks amend different fields; both survive.
        let mut args = json!({ "path": "f", "content": "x" });
        apply_modify(&mut args, &json!({ "content": "y" }));
        apply_modify(&mut args, &json!({ "path": "g" }));
        assert_eq!(args, json!({ "path": "g", "content": "y" }));
    }

    #[test]
    fn apply_modify_non_object_modify_is_noop() {
        let mut args = json!({ "path": "f", "content": "x" });
        apply_modify(&mut args, &json!("not an object"));
        apply_modify(&mut args, &json!(42));
        apply_modify(&mut args, &json!(null));
        assert_eq!(args, json!({ "path": "f", "content": "x" }));
    }

    #[test]
    fn apply_modify_non_object_args_is_noop() {
        let mut args = json!("scalar args");
        apply_modify(&mut args, &json!({ "content": "y" }));
        assert_eq!(args, json!("scalar args"));
    }

    #[tokio::test]
    async fn execute_hook_with_modify() {
        let tmp = TmpDir::new("exec_modify");
        let response = r#"{"allow": true, "reason": "reformatted", "modify": {"content": "new"}}"#;
        let script = write_hook_script(&tmp.path, "modify.sh", response, 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.reason, "reformatted");
        assert_eq!(result.modify, Some(json!({"content": "new"})));
    }

    #[tokio::test]
    async fn execute_hook_nonzero_exit_pre_denies() {
        let tmp = TmpDir::new("exec_exit_pre");
        let script = write_hook_script(
            &tmp.path,
            "fail.sh",
            r#"{"allow": true}"#,
            1, // exits with 1
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("exited"));
    }

    #[tokio::test]
    async fn execute_hook_nonzero_exit_post_skips() {
        let tmp = TmpDir::new("exec_exit_post");
        let script = write_hook_script(&tmp.path, "fail.sh", r#"{"allow": true}"#, 1);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("post_bash", "bash", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("post_bash", "test-plugin", &config, &ctx).await;
        // post hooks: non-zero exit is skipped, operation continues.
        assert!(result.allow);
        assert!(result.reason.contains("exited"));
        assert!(result.modify.is_none());
    }

    #[tokio::test]
    async fn execute_hook_bad_json_pre_denies() {
        let tmp = TmpDir::new("exec_bad_json");
        let script = write_hook_script(&tmp.path, "bad.sh", "NOT JSON", 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn execute_hook_timeout_pre_denies() {
        let tmp = TmpDir::new("exec_timeout");
        // Script sleeps long enough to trigger the timeout.
        let script = tmp.path.join("sleep.sh");
        fs::write(&script, "#!/bin/sh\nsleep 10\necho '{\"allow\":true}'\n").unwrap();
        make_executable(&script);

        let config = HookConfig {
            script,
            timeout_ms: 200, // very short timeout
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_hook_post_always_allows_even_on_deny() {
        // For post hooks, even if the hook returns allow:false, we don't block.
        let tmp = TmpDir::new("exec_post_deny");
        let script = write_hook_script(
            &tmp.path,
            "deny.sh",
            r#"{"allow": false, "reason": "saw an issue"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("post_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("post_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.modify, None);
    }

    #[tokio::test]
    async fn execute_hook_empty_stdout_pre_denies() {
        let tmp = TmpDir::new("exec_empty");
        let script = write_hook_script(&tmp.path, "empty.sh", "", 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_bash", "bash", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_bash", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("empty stdout"));
    }

    // ---- build_context ----

    #[test]
    fn build_context_structure() {
        let ctx = build_context(
            "pre_write",
            "write_file",
            "/home/user/project",
            Some(&json!({"path": "src/main.rs", "content": "fn main() {}"})),
            "session_123.jsonl",
            true,
        );

        assert_eq!(ctx["hook"], "pre_write");
        assert_eq!(ctx["tool"], "write_file");
        assert_eq!(ctx["workspace"], "/home/user/project");
        assert_eq!(ctx["session_id"], "session_123.jsonl");
        assert!(ctx["timestamp"].as_u64().is_some());
        assert_eq!(ctx["args"]["path"], "src/main.rs");
        assert_eq!(ctx["args"]["content"], "fn main() {}");
    }

    #[test]
    fn build_context_omits_args_when_pass_args_false() {
        let ctx = build_context(
            "pre_write",
            "write_file",
            "/ws",
            Some(&json!({"secret": "value"})),
            "sess.jsonl",
            false,
        );
        assert!(ctx.get("args").is_none());
    }

    #[test]
    fn build_context_handles_none_args() {
        let ctx = build_context("session_start", "", "/ws", None, "sess.jsonl", true);
        assert!(ctx.get("args").is_none());
    }

    // ---- default timeouts ----

    #[test]
    fn pre_hooks_get_short_timeout() {
        assert_eq!(default_hook_timeout("pre_bash"), 5_000);
        assert_eq!(default_hook_timeout("pre_write"), 5_000);
        assert_eq!(default_hook_timeout("pre_read"), 5_000);
        assert_eq!(default_hook_timeout("pre_compact"), 5_000);
    }

    #[test]
    fn post_hooks_get_long_timeout() {
        assert_eq!(default_hook_timeout("post_bash"), 30_000);
        assert_eq!(default_hook_timeout("post_write"), 30_000);
        assert_eq!(default_hook_timeout("session_start"), 30_000);
        assert_eq!(default_hook_timeout("session_stop"), 30_000);
    }

    // ---- plugin-declared tools ----

    /// Write a plugin that declares one tool whose handler is a minimal
    /// executable script, into a `tools-plugin/` SUBDIRECTORY of `dir` (so a
    /// `PluginManager::new(dir, …)` scan finds it — the scanner loads each
    /// subdirectory, not `dir` itself). Returns the plugin directory path so
    /// `load_plugin_from_dir` callers can target it directly. `extra` is
    /// spliced into the tool object as extra fields (e.g.
    /// `"kind":"readonly","timeout_ms":12345`).
    fn write_plugin_with_tool(dir: &Path, tool_name: &str, extra: &str) -> PathBuf {
        let plugin_dir = dir.join("tools-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        let tools_dir = plugin_dir.join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        let script = tools_dir.join("run.sh");
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        make_executable(&script);
        let tool_extra = if extra.is_empty() {
            String::new()
        } else {
            format!(", {}", extra)
        };
        let manifest = format!(
            r#"{{
  "name": "tools-plugin",
  "version": "1.0.0",
  "tools": [
    {{ "name": "{name}", "description": "a tool", "parameters": {{"type":"object","properties":{{}}}}, "script": "tools/run.sh"{extra} }}
  ]
}}"#,
            name = tool_name,
            extra = tool_extra,
        );
        fs::write(plugin_dir.join("plugin.json"), &manifest).unwrap();
        plugin_dir
    }

    #[test]
    fn load_plugin_with_tools() {
        let tmp = TmpDir::new("load_tools");
        let pdir = write_plugin_with_tool(&tmp.path, "my_tool", "");
        let plugin = PluginManager::load_plugin_from_dir(&pdir).unwrap();
        assert_eq!(plugin.tools.len(), 1);
        let t = &plugin.tools[0];
        assert_eq!(t.name, "my_tool");
        assert_eq!(t.description, "a tool");
        assert_eq!(t.timeout_ms, DEFAULT_POST_TIMEOUT_MS);
        assert_eq!(t.kind, ToolKind::Destructive); // default
        assert!(t.parameters.is_object());
        assert!(t.script.exists());
    }

    #[test]
    fn load_tool_readonly_kind() {
        let tmp = TmpDir::new("tool_readonly");
        let pdir = write_plugin_with_tool(&tmp.path, "ro_tool", r#""kind":"readonly""#);
        let plugin = PluginManager::load_plugin_from_dir(&pdir).unwrap();
        assert_eq!(plugin.tools[0].kind, ToolKind::ReadOnly);
    }

    #[test]
    fn load_tool_explicit_destructive_kind() {
        let tmp = TmpDir::new("tool_destructive");
        let pdir = write_plugin_with_tool(&tmp.path, "d_tool", r#""kind":"destructive""#);
        let plugin = PluginManager::load_plugin_from_dir(&pdir).unwrap();
        assert_eq!(plugin.tools[0].kind, ToolKind::Destructive);
    }

    #[test]
    fn load_tool_custom_timeout() {
        let tmp = TmpDir::new("tool_timeout");
        let pdir = write_plugin_with_tool(&tmp.path, "t_tool", r#""timeout_ms": 12345"#);
        let plugin = PluginManager::load_plugin_from_dir(&pdir).unwrap();
        assert_eq!(plugin.tools[0].timeout_ms, 12345);
    }

    #[test]
    fn load_tool_rejects_missing_script() {
        let tmp = TmpDir::new("tool_missing");
        fs::write(
            tmp.path.join("plugin.json"),
            r#"{"name":"p","version":"1.0.0","tools":[{"name":"x","script":"tools/nope.sh"}]}"#,
        )
        .unwrap();
        let r = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn load_tool_rejects_path_escape() {
        let tmp = TmpDir::new("tool_escape");
        fs::write(
            tmp.path.join("plugin.json"),
            r#"{"name":"p","version":"1.0.0","tools":[{"name":"x","script":"../escape.sh"}]}"#,
        )
        .unwrap();
        let r = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("escapes"));
    }

    #[test]
    fn load_tool_rejects_invalid_kind() {
        let tmp = TmpDir::new("tool_bad_kind");
        let pdir = write_plugin_with_tool(&tmp.path, "bad", r#""kind":"weird""#);
        let r = PluginManager::load_plugin_from_dir(&pdir);
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("invalid kind"));
        assert!(e.contains("weird"));
    }

    #[test]
    fn load_tool_rejects_empty_name() {
        let tmp = TmpDir::new("tool_empty_name");
        fs::write(
            tmp.path.join("plugin.json"),
            r#"{"name":"p","version":"1.0.0","tools":[{"name":"","script":"tools/run.sh"}]}"#,
        )
        .unwrap();
        let r = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("empty name"));
    }

    #[test]
    fn tool_definitions_schema() {
        let tmp = TmpDir::new("tool_defs");
        write_plugin_with_tool(&tmp.path, "echo_tool", r#""timeout_ms":5000"#);
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        let defs = mgr.tool_definitions();
        assert_eq!(defs.len(), 1);
        let f = defs[0].get("function").unwrap();
        assert_eq!(f.get("name").and_then(|v| v.as_str()), Some("echo_tool"));
        assert_eq!(
            f.get("description").and_then(|v| v.as_str()),
            Some("a tool")
        );
        assert!(f.get("parameters").unwrap().is_object());
    }

    #[test]
    fn tool_definitions_excludes_disabled() {
        let tmp = TmpDir::new("tool_disabled");
        write_plugin_with_tool(&tmp.path, "t", "");
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert_eq!(mgr.tool_definitions().len(), 1);
        mgr.disable("tools-plugin").unwrap();
        assert_eq!(mgr.tool_definitions().len(), 0);
        assert!(mgr.tool_config("t").is_none());
        assert!(mgr.tool_kind("t").is_none());
    }

    #[test]
    fn tool_config_and_kind_lookup() {
        let tmp = TmpDir::new("tool_lookup");
        write_plugin_with_tool(&tmp.path, "lookup", r#""kind":"readonly""#);
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        let tc = mgr.tool_config("lookup").unwrap();
        assert_eq!(tc.name, "lookup");
        assert_eq!(tc.kind, ToolKind::ReadOnly);
        assert!(mgr.tool_config("nope").is_none());
        assert_eq!(mgr.tool_kind("lookup"), Some(ToolKind::ReadOnly));
        // A built-in tool name is not a plugin tool → None.
        assert!(mgr.tool_kind("read_file").is_none());
    }

    #[test]
    fn builtin_name_collision_never_hijacks() {
        // A plugin MAY declare a tool whose name collides with a built-in. It
        // still loads (tool_config finds it), but the registry merge hides it
        // from the model's tool list (built-in wins) and the dispatch +
        // classify guards drop it via `is_builtin` — so the built-in always
        // runs, never the same-named plugin tool. This test pins that guard
        // composition so a future change can't reintroduce the hijack.
        let tmp = TmpDir::new("tool_collision");
        write_plugin_with_tool(&tmp.path, "read_file", r#""kind":"readonly""#);
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        // Loadable + findable by the manager …
        assert!(mgr.tool_config("read_file").is_some());
        assert_eq!(mgr.tool_kind("read_file"), Some(ToolKind::ReadOnly));
        // … but the dispatch guard filters it out (it's a built-in name), so a
        // call to "read_file" routes to the built-in, not the plugin.
        assert!(mgr
            .tool_config("read_file")
            .filter(|_| !crate::tools::is_builtin("read_file"))
            .is_none());
        // And a genuinely-custom name is NOT filtered — it dispatches normally.
        let tmp2 = TmpDir::new("tool_no_collision");
        write_plugin_with_tool(&tmp2.path, "my_domain_tool", r#""kind":"readonly""#);
        let mgr2 = PluginManager::new(tmp2.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr2
            .tool_config("my_domain_tool")
            .filter(|_| !crate::tools::is_builtin("my_domain_tool"))
            .is_some());
        // is_builtin itself: known built-ins true, arbitrary/empty false.
        assert!(crate::tools::is_builtin("read_file"));
        assert!(crate::tools::is_builtin("bash"));
        assert!(!crate::tools::is_builtin("my_domain_tool"));
        assert!(!crate::tools::is_builtin(""));
    }

    // ---- execute_plugin_tool ----

    /// Write a `plugin.json` with arbitrary manifest JSON into `dir`.
    fn write_manifest(dir: &Path, manifest: &str) {
        fs::write(dir.join("plugin.json"), manifest).unwrap();
    }

    #[test]
    fn hook_points_include_catch_all() {
        assert!(HOOK_POINTS.contains(&"pre_tool"));
        assert!(HOOK_POINTS.contains(&"post_tool"));
        // pre_* get the short timeout; post_* the long one.
        assert_eq!(default_hook_timeout("pre_tool"), DEFAULT_PRE_TIMEOUT_MS);
        assert_eq!(default_hook_timeout("post_tool"), DEFAULT_POST_TIMEOUT_MS);
    }

    #[test]
    fn disable_tools_manifest_loaded() {
        let tmp = TmpDir::new("disable_loaded");
        write_manifest(
            &tmp.path,
            r#"{"name":"no-bash","version":"1.0.0","disable_tools":["bash","git_commit"]}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(
            plugin.disable_tools,
            vec!["bash".to_string(), "git_commit".to_string()]
        );
    }

    #[test]
    fn system_prompt_manifest_loaded() {
        let tmp = TmpDir::new("sysprompt_loaded");
        write_manifest(
            &tmp.path,
            r#"{"name":"rules","version":"2.0.0","system_prompt":"Never run raw SQL."}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(plugin.system_prompt, "Never run raw SQL.");
    }

    #[test]
    fn override_field_loaded() {
        let tmp = TmpDir::new("override_loaded");
        let pdir = write_plugin_with_tool(&tmp.path, "bash", r#""override":true"#);
        let plugin = PluginManager::load_plugin_from_dir(&pdir).unwrap();
        assert!(plugin.tools[0].override_builtin);

        // Without override, it stays false.
        let tmp2 = TmpDir::new("override_false");
        let pdir2 = write_plugin_with_tool(&tmp2.path, "my_tool", "");
        let plugin2 = PluginManager::load_plugin_from_dir(&pdir2).unwrap();
        assert!(!plugin2.tools[0].override_builtin);
    }

    // ---- plugin-declared OAuth provider loading ----

    #[test]
    fn load_oauth_minimal() {
        let tmp = TmpDir::new("oauth_minimal");
        let odir = tmp.path.join("oauth");
        fs::create_dir_all(&odir).unwrap();
        write_hook_script(&odir, "oauth.sh", r#"{"access_token":null}"#, 0);
        write_manifest(
            &tmp.path,
            r#"{"name":"grok-oauth","version":"0.1.0","oauth":{
               "provider_id":"grok",
               "base_url":"https://api.x.ai/v1",
               "script":"oauth/oauth.sh"
            }}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        let oauth = plugin.oauth.expect("oauth config loaded");
        assert_eq!(oauth.provider_id, "grok");
        assert_eq!(oauth.base_url, "https://api.x.ai/v1");
        assert_eq!(oauth.kind, ProviderKind::OpenAI);
        assert_eq!(oauth.label, "grok"); // defaults to provider_id
                                         // token_path defaults to <provider_id>.json under the oauth dir.
        assert!(oauth.token_path.ends_with("grok.json"));
        // The shared script resolves for every action.
        assert!(oauth.script_for("login").is_some());
        assert!(oauth.script_for("complete").is_some());
        assert!(oauth.script_for("token").is_some());
        assert!(oauth.script_for("clear").is_some());
    }

    #[test]
    fn load_oauth_rejects_missing_token_script() {
        let tmp = TmpDir::new("oauth_no_token");
        write_manifest(
            &tmp.path,
            r#"{"name":"bad","version":"0.1.0","oauth":{
               "provider_id":"bad","base_url":"https://x.example/v1"
            }}"#,
        );
        let err = PluginManager::load_plugin_from_dir(&tmp.path).unwrap_err();
        assert!(err.contains("no token script"), "got: {err}");
    }

    #[test]
    fn load_oauth_rejects_invalid_kind() {
        let tmp = TmpDir::new("oauth_bad_kind");
        let odir = tmp.path.join("oauth");
        fs::create_dir_all(&odir).unwrap();
        write_hook_script(&odir, "oauth.sh", r#"{"access_token":null}"#, 0);
        write_manifest(
            &tmp.path,
            r#"{"name":"bad","version":"0.1.0","oauth":{
               "provider_id":"bad","base_url":"https://x.example/v1",
               "kind":"weird","script":"oauth/oauth.sh"
            }}"#,
        );
        let err = PluginManager::load_plugin_from_dir(&tmp.path).unwrap_err();
        assert!(err.contains("invalid kind"), "got: {err}");
    }

    #[test]
    fn load_oauth_per_action_overrides() {
        let tmp = TmpDir::new("oauth_overrides");
        let odir = tmp.path.join("oauth");
        fs::create_dir_all(&odir).unwrap();
        write_hook_script(&odir, "login.sh", r#"{"url":"https://x"}"#, 0);
        write_hook_script(&odir, "complete.sh", r#"{"ok":true}"#, 0);
        write_hook_script(&odir, "token.sh", r#"{"access_token":"t"}"#, 0);
        write_manifest(
            &tmp.path,
            r#"{"name":"ov","version":"0.1.0","oauth":{
               "provider_id":"ov","base_url":"https://x.example/v1","kind":"anthropic",
               "login_script":"oauth/login.sh","complete_script":"oauth/complete.sh",
               "token_script":"oauth/token.sh"
            }}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        let oauth = plugin.oauth.unwrap();
        assert_eq!(oauth.kind, ProviderKind::Anthropic);
        // No shared script → only the per-action overrides resolve.
        assert!(oauth.script_for("login").is_some());
        assert!(oauth.script_for("complete").is_some());
        assert!(oauth.script_for("token").is_some());
        assert!(oauth.script_for("clear").is_none());
    }

    #[test]
    fn oauth_provider_config_builds_config() {
        let tmp = TmpDir::new("oauth_provider_cfg");
        // PluginManager::new scans SUBDIRS of plugins_dir, so the plugin lives
        // in its own subdir.
        let pdir = tmp.path.join("grok-oauth");
        let odir = pdir.join("oauth");
        fs::create_dir_all(&odir).unwrap();
        write_hook_script(&odir, "oauth.sh", r#"{"access_token":null}"#, 0);
        write_manifest(
            &pdir,
            r#"{"name":"grok-oauth","version":"0.1.0","oauth":{
               "provider_id":"grok","base_url":"https://api.x.ai/v1",
               "headers":[["X-Source","cc"]],"script":"oauth/oauth.sh"
            }}"#,
        );
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr.supports_oauth_login("grok"));
        let pc = mgr.oauth_provider_config("grok").expect("provider config");
        assert_eq!(pc.name, "grok");
        assert_eq!(pc.base_url, "https://api.x.ai/v1");
        assert_eq!(pc.kind, ProviderKind::OpenAI);
        assert!(pc.api_key.is_none());
        assert_eq!(pc.headers, vec![("X-Source".to_string(), "cc".to_string())]);
        // Unknown provider_id → None / not supported.
        assert!(mgr.oauth_provider_config("nope").is_none());
        assert!(!mgr.supports_oauth_login("nope"));
    }

    #[test]
    fn manager_disabled_tools_unions_across_plugins() {
        let tmp = TmpDir::new("disable_union");
        let a = tmp.path.join("a");
        let b = tmp.path.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        write_manifest(
            &a,
            r#"{"name":"a","version":"1.0.0","disable_tools":["bash"]}"#,
        );
        write_manifest(
            &b,
            r#"{"name":"b","version":"1.0.0","disable_tools":["bash","edit"]}"#,
        );
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        let disabled = mgr.disabled_tools();
        assert!(disabled.contains("bash"));
        assert!(disabled.contains("edit"));
        assert_eq!(disabled.len(), 2);
    }

    #[test]
    fn overridden_tool_names_only_when_override_and_builtin() {
        // override:true on a built-in name → overridden. override:false (or a
        // custom name) → NOT overridden.
        let tmp = TmpDir::new("override_names");
        write_plugin_with_tool(&tmp.path, "bash", r#""override":true"#);
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        let names = mgr.overridden_tool_names();
        assert!(names.contains("bash"));

        // override:true on a NON-built-in name → not in overridden set (there's
        // nothing to override; it's just a custom tool).
        let tmp2 = TmpDir::new("override_custom");
        write_plugin_with_tool(&tmp2.path, "my_domain_tool", r#""override":true"#);
        let mgr2 = PluginManager::new(tmp2.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr2.overridden_tool_names().is_empty());

        // A plain collision (no override) on a built-in → NOT overridden.
        let tmp3 = TmpDir::new("override_none");
        write_plugin_with_tool(&tmp3.path, "read_file", r#""kind":"readonly""#);
        let mgr3 = PluginManager::new(tmp3.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr3.overridden_tool_names().is_empty());
    }

    #[test]
    fn system_prompt_injection_concat_and_framed() {
        let tmp = TmpDir::new("sysprompt_inject");
        let a = tmp.path.join("a");
        let b = tmp.path.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        write_manifest(
            &a,
            r#"{"name":"alpha","version":"1.0.0","system_prompt":"rule A"}"#,
        );
        write_manifest(
            &b,
            r#"{"name":"beta","version":"2.0.0","system_prompt":"rule B"}"#,
        );
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        let inj = mgr.system_prompt_injection();
        assert!(inj.starts_with("\n\n## Plugin-injected context\n\n"));
        assert!(inj.contains("# Plugin: alpha (v1.0.0)\nrule A"));
        assert!(inj.contains("# Plugin: beta (v2.0.0)\nrule B"));

        // Empty when no plugin declares one (prefix-cache-safe).
        let tmp2 = TmpDir::new("sysprompt_empty");
        write_manifest(&tmp2.path, r#"{"name":"plain","version":"1.0.0"}"#);
        let mgr2 = PluginManager::new(tmp2.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr2.system_prompt_injection().is_empty());
    }

    #[test]
    fn has_hook_existence_check() {
        let tmp = TmpDir::new("has_hook");
        // PluginManager::new scans SUBDIRECTORIES of its root for plugins,
        // so the plugin must live in a subdir.
        let pdir = tmp.path.join("h");
        let hooks_dir = pdir.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        write_hook_script(&hooks_dir, "h.sh", r#"{"allow":true}"#, 0);
        write_manifest(
            &pdir,
            r#"{"name":"h","version":"1.0.0","hooks":{"pre_tool":{"script":"hooks/h.sh"}}}"#,
        );
        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr.has_hook("pre_tool"));
        assert!(!mgr.has_hook("pre_bash"));
        // Disabled plugin is excluded.
        mgr.disable("h").unwrap();
        assert!(!mgr.has_hook("pre_tool"));
    }

    #[test]
    fn disabled_plugin_excluded_from_new_capabilities() {
        // A disabled plugin contributes nothing to disable_tools / overrides /
        // system_prompt — mirroring how disabled plugins are excluded from
        // hook configs and tool definitions.
        let tmp = TmpDir::new("disabled_excluded");
        let pdir = write_plugin_with_tool(&tmp.path, "bash", r#""override":true"#);
        // Augment with disable_tools + system_prompt.
        let manifest = fs::read_to_string(pdir.join("plugin.json")).unwrap();
        let manifest = manifest.trim_end_matches('}').to_string()
            + r#","disable_tools":["edit"],"system_prompt":"ctx"}"#;
        fs::write(pdir.join("plugin.json"), &manifest).unwrap();

        let mgr = PluginManager::new(tmp.path.clone(), PathBuf::from("/__t_ws__"), true);
        assert!(mgr.overridden_tool_names().contains("bash"));
        assert!(mgr.disabled_tools().contains("edit"));
        assert!(mgr.system_prompt_injection().contains("ctx"));

        mgr.disable("tools-plugin").unwrap();
        assert!(mgr.overridden_tool_names().is_empty());
        assert!(mgr.disabled_tools().is_empty());
        assert!(mgr.system_prompt_injection().is_empty());
    }

    fn tool_config_for(script: PathBuf, timeout_ms: u64, kind: ToolKind) -> ToolConfig {
        ToolConfig {
            name: "ut".into(),
            description: "".into(),
            parameters: json!({}),
            script,
            timeout_ms,
            kind,
            override_builtin: false,
        }
    }

    #[tokio::test]
    async fn execute_plugin_tool_json_output() {
        let tmp = TmpDir::new("ept_json");
        let script = write_hook_script(&tmp.path, "t.sh", r#"{"ok":true,"output":"hi there"}"#, 0);
        let tc = tool_config_for(script, 5000, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({"x":1}), "/ws", "s.jsonl").await;
        assert!(out.ok);
        assert_eq!(out.output, "hi there");
    }

    #[tokio::test]
    async fn execute_plugin_tool_raw_output() {
        let tmp = TmpDir::new("ept_raw");
        let script = write_hook_script(&tmp.path, "t.sh", "plain result", 0);
        let tc = tool_config_for(script, 5000, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({}), "/ws", "s.jsonl").await;
        assert!(out.ok);
        assert_eq!(out.output, "plain result");
    }

    #[tokio::test]
    async fn execute_plugin_tool_error_output() {
        let tmp = TmpDir::new("ept_err");
        let script = write_hook_script(&tmp.path, "t.sh", r#"{"ok":false,"output":"boom"}"#, 0);
        let tc = tool_config_for(script, 5000, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({}), "/ws", "s.jsonl").await;
        assert!(!out.ok);
        assert_eq!(out.output, "boom");
    }

    #[tokio::test]
    async fn execute_plugin_tool_error_field() {
        let tmp = TmpDir::new("ept_errfield");
        let script = write_hook_script(&tmp.path, "t.sh", r#"{"error":"something failed"}"#, 0);
        let tc = tool_config_for(script, 5000, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({}), "/ws", "s.jsonl").await;
        assert!(!out.ok);
        assert!(out.output.contains("something failed"));
    }

    #[tokio::test]
    async fn execute_plugin_tool_nonzero_exit() {
        let tmp = TmpDir::new("ept_exit");
        let script = write_hook_script(&tmp.path, "t.sh", r#"{"ok":true,"output":"x"}"#, 3);
        let tc = tool_config_for(script, 5000, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({}), "/ws", "s.jsonl").await;
        assert!(!out.ok);
        assert!(out.output.contains("exited"));
    }

    #[tokio::test]
    async fn execute_plugin_tool_timeout() {
        let tmp = TmpDir::new("ept_timeout");
        // Handler sleeps well past the tool's timeout.
        let script = tmp.path.join("slow.sh");
        fs::write(
            &script,
            "#!/bin/sh\nsleep 2\necho '{\"ok\":true,\"output\":\"late\"}'\n",
        )
        .unwrap();
        make_executable(&script);
        let tc = tool_config_for(script, 300, ToolKind::Destructive);
        let out = execute_plugin_tool("ut", &tc, &json!({}), "/ws", "s.jsonl").await;
        assert!(!out.ok);
        assert!(out.output.contains("timed out"));
    }
}
