# Slash Commands

Inside the TUI, type a slash command at the input prompt to change settings, navigate, manage plugins, or inspect state.

Every command begins with `/`. Aliases are listed alongside the primary name.

---

## Navigation and Help

### `/help`, `/?`

Open the help screen listing all slash commands, keybindings, and common workflows.

**Syntax:** `/help` or `/?`

### `/settings`

Open the settings modal to browse and edit all runtime preferences.

**Syntax:** `/settings`

### `/keybinds`

Open the interactive keybinding editor. All keybindings are customizable and persisted in settings.

**Syntax:** `/keybinds`

### `/status`

Print a status summary to the transcript: active model, provider, approval mode, reasoning effort, theme, mouse-wheel state, performance metrics, and context usage.

**Syntax:** `/status`

### `/sessions`

List all saved sessions. Select one to resume it.

**Syntax:** `/sessions`

### `/new`

Start a new session on a fresh conversation file. The current session is not deleted. Refuses while a turn is in progress.

**Syntax:** `/new`

---

## Conversation

### `/reset`

Wipe the conversation and its session file. Shows a confirmation prompt first (destructive).

**Syntax:** `/reset`

### `/clear`

Clear the in-memory transcript. The session file on disk is preserved so the conversation can still be resumed later.

**Syntax:** `/clear`

### `/undo`

Drop the last assistant turn from the conversation history. The core re-emits the trimmed conversation; the previous state is restored.

**Syntax:** `/undo`

### `/compact`

Force an immediate context compaction. Optionally include preservation instructions that tell the summarizer what to keep.

**Syntax:**

```text
/compact
/compact preserve the project architecture notes above
```

Without arguments, opens a modal to enter preservation instructions.

### `/context`

Request the core to report the current context window usage (token counts and message breakdown). The response is rendered into the transcript.

**Syntax:** `/context`

### `/copy`

Copy the last assistant response to the system clipboard.

**Syntax:** `/copy`

### `/attach`

Attach an image to the next turn. The image is validated (must be a readable image file ≤20 MiB). Optionally include a prompt text that replaces the composer contents.

**Syntax:**

```text
/attach
/attach /path/to/screenshot.png
/attach /path/to/diagram.png "explain this architecture"
```

Without arguments, opens a file-path input modal.

### `/abort`

Cancel the current in-progress turn and clear any queued follow-up or steer prompts. Sends an `abort` command to the core.

**Syntax:** `/abort`

### `/steer`

Send a steering/guidance message that influences the current running turn without waiting for it to complete. Works on terminals that do not support Ctrl+Enter.

**Syntax:**

```text
/steer
/steer focus on error handling
```

Without arguments, opens a modal for typing the steering message.

---

## Provider and Authentication

### `/login`

Open the provider-login picker. Choose a configured provider to authenticate. Supported flows include API-key paste and OAuth device-code (Google/OpenAI).

**Syntax:** `/login`

### `/logout`

Log out of a provider and remove its stored credentials.

**Syntax:**

```text
/logout
/logout <provider-name>
```

Without arguments, opens a picker to choose which provider to log out of.

### `/oauth-code`

Complete a pending OAuth device-code login (used in headless/SSH sessions where the browser cannot open automatically).

**Syntax:**

```text
/oauth-code
/oauth-code ABCD-1234-EFGH-5678
```

Without arguments, opens a modal to paste the code (recommended for long Google codes). With an inline argument, sends the code immediately.

### `/search-key`

Manage search-provider API keys (Exa, Tavily).

**Syntax:**

```text
/search-key                       # open picker to choose provider
/search-key exa                   # open paste modal for Exa key
/search-key exa <your-api-key>    # set inline
/search-key exa --clear           # remove stored key
/search-key tavily                # same for Tavily
```

Supported providers: `exa`, `tavily`. Use `--clear`, `clear`, or `off` to remove a stored key.

### `/model`, `/models`

Set the active model for the conversation.

**Syntax:**

```text
/model                    # open model picker
/model 3                  # select model at index 3 in the list
/model gpt-4              # select the first model whose ID contains "gpt-4"
```

Selection by substring is case-sensitive. When the reasoning level is incompatible with the new model, it is automatically adjusted.

### `/usage`

Request provider plan usage/rate-limit information for the currently selected model. Opens a loading modal and displays the result.

**Syntax:** `/usage`

---

## Settings

All setting commands support a bare form (opens a picker or modal) and an inline form (sets the value directly).

### `/approval`, `/approvals`

Set the tool-approval mode.

**Syntax:**

```text
/approval                 # open picker
/approval never            # auto-approve everything
/approval destructive      # ask only for bash/write/edit (default)
/approval always           # ask for every tool call
```

### `/reasoning`

Set the reasoning-effort level (when supported by the model).

**Syntax:**

```text
/reasoning                # open picker with available levels
/reasoning low
/reasoning medium
/reasoning high
```

Available levels depend on the active model.

### `/bash-timeout`

Set the per-command bash timeout in seconds.

**Syntax:**

```text
/bash-timeout             # open modal
/bash-timeout 60          # set to 60 seconds
```

Must be a positive integer.

### `/auto-compact`

Toggle automatic context compaction.

**Syntax:**

```text
/auto-compact             # open picker
/auto-compact on
/auto-compact off
```

Accepts `on`/`off`, `true`/`false`, `1`/`0`.

### `/sandbox`

Inspect and control the Microsandbox microVM sandbox.

**Syntax:**

```text
/sandbox                 # open status / settings view
/sandbox status          # show platform, image, limits, readiness
/sandbox enable          # enable Microsandbox (runs preflight first)
/sandbox disable         # explicitly disable (set to none)
/sandbox setup           # prepare user-space runtime/image assets
/sandbox recheck         # re-run preflight after setup
/sandbox reset           # destroy and recreate an unhealthy sandbox
```

Enabling requires a healthy preflight (Linux KVM, Apple Silicon macOS, or
Windows WHP). Changes that need a core restart prompt to restart. See the
[Sandbox Guide](../guides/sandbox.md).

### `/no-network`

Toggle bash network egress blocking.

**Syntax:**

```text
/no-network               # open picker
/no-network on
/no-network off
```

Changes require a core restart.

### `/mouse-wheel`

Toggle mouse-wheel support for scrolling.

**Syntax:**

```text
/mouse-wheel              # open picker
/mouse-wheel on            # Shift+click to select text
/mouse-wheel off           # click-drag to select text
```

### `/footer-metrics`

Toggle the real-time performance metrics footer (token rates, latency, memory).

**Syntax:**

```text
/footer-metrics           # open picker
/footer-metrics on
/footer-metrics off
```

### `/idle-timeout`

Set the SSE idle timeout in seconds (must be ≥10).

**Syntax:**

```text
/idle-timeout             # open modal
/idle-timeout 300
```

Changes require a core restart.

### `/max-session-tokens`

Set the hard session token budget.

**Syntax:**

```text
/max-session-tokens       # open modal
/max-session-tokens 0      # unlimited
/max-session-tokens 32000  # limit to 32K tokens
```

`0` means unlimited. Changes require a core restart.

### `/theme`

Open the theme picker to switch between available color themes (e.g., `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `monokai`).

**Syntax:** `/theme`

---

## Memory

### `/remember`

Save a durable memory to the knowledge store.

**Syntax:**

```text
/remember                         # opens a modal to write the memory text
/remember The project uses Go 1.24
```

The memory is persisted and available across sessions.

### `/memory`

Open the memory list picker. Select a memory to view details or forget it.

**Syntax:** `/memory`

### `/forget`

Forget (permanently delete) a memory by name.

**Syntax:**

```text
/forget                   # opens the memory picker to choose
/forget <memory-name>     # forget immediately (with confirmation)
```

Shows a destructive confirmation prompt first.

### `/index`

Bootstrap or update the repository knowledge index. Scans the project structure and persists
architecture, conventions, APIs, and gotchas as durable memories. Also identifies candidate
reusable skills.

**Syntax:**

```text
/index                    # full index (walk entire repo)
/index --full, -f         # explicit full index
/index --incremental, -i  # only changed areas (uses git diff)
```

Delegates to the orchestrator agent; no core command is needed.

### `/reflect`

Deliberate end-of-task learning pass. Reviews work done in the current session and persists durable takeaways as memories. Creates candidate skills for repetitive patterns.

**Syntax:** `/reflect`

Delegates to the orchestrator agent.

---

## Plugins

### `/plugin-install`

Install a plugin from a local path or remote source.

**Syntax:**

```text
/plugin-install                               # opens a path-input modal
/plugin-install /path/to/plugin/dir
/plugin-install /path/to/plugin/dir --user     # user scope (global)
/plugin-install /path/to/plugin/dir --project  # project scope
```

Without a scope flag, opens a picker to choose user or project scope.

### `/plugin-list`, `/plugin-config`, `/plugin-enable`, `/plugin-disable`

Browse and toggle installed plugins. `plugin-enable`/`plugin-disable` accept an optional plugin name to act immediately.

**Syntax:**

```text
/plugin-list              # list installed plugins
/plugin-config            # open the plugin toggle picker (list + enable/disable)
/plugin-enable <name>     # enable a specific plugin
/plugin-disable <name>    # disable a specific plugin
```

Bare `/plugin-enable` or `/plugin-disable` opens the same toggle picker as `/plugin-config`.

### `/plugin-remove`

Uninstall a plugin and delete its files.

**Syntax:**

```text
/plugin-remove            # open picker to choose plugin
/plugin-remove <name>     # uninstall immediately (with confirmation)
```

Shows a destructive confirmation prompt first.

### `/plugin-reload`

Reload all plugins from disk without restarting the core.

**Syntax:** `/plugin-reload`

### Plugin Slash Commands

Third-party plugins can register custom slash commands. If a typed `/name` does not match a
built-in command, the TUI checks plugin-registered commands. Unknown commands produce
`unknown command: /<name>`.

---

## Goals

### `/goal`

Set a session goal. The goal appears as a pinned panel and guides the agent's behavior.

**Syntax:**

```text
/goal                              # opens a modal to write the goal
/goal refactor the authentication module
```

### `/cancel-goal`

Cancel the active goal. Sends a `cancel_goal` command to the core.

**Syntax:** `/cancel-goal`

---

## Subagents

### `/run`

Run a single subagent with an inline prompt.

**Syntax:**

```text
/run <subagent-name> <prompt>
/run "Check code style" review the handlers.go file
```

Delegates to `runSubagentCommand` with mode `single`.

### `/parallel`

Run multiple subagents in parallel.

**Syntax:**

```text
/parallel <subagent-name> <prompt>
```

Delegates to `runSubagentCommand` with mode `parallel`.

### `/chain`

Run subagents sequentially, passing results between them.

**Syntax:**

```text
/chain <subagent-name> <prompt>
```

Delegates to `runSubagentCommand` with mode `chain`.

### `/subagents`, `/subagents-list`

List all available subagents with their descriptions and capabilities.

**Syntax:** `/subagents` or `/subagents-list`

Delegates to the orchestrator's `subagent({ action: "list" })`.

### `/subagents-doctor`

Run subagent setup diagnostics and show configuration status for each built-in and user-defined subagent.

**Syntax:** `/subagents-doctor`

Delegates to `subagent({ action: "doctor" })`.

### `/subagents-status`

Show the status of active subagent runs (in-progress, completed, failed).

**Syntax:** `/subagents-status`

Delegates to `subagent({ action: "status" })`.

### `/subagents-models`

Show the runtime model mapping for built-in subagents (which model each agent uses).

**Syntax:** `/subagents-models`

Delegates to `subagent({ action: "models" })`.

---

## Other

### `/exit`, `/quit`

Quit the application gracefully. Performs the same clean teardown as the quit keybinding.

**Syntax:** `/exit` or `/quit`

### `/stats`

Request core telemetry and statistics. The core responds with usage counts, timing, and error rates.

**Syntax:** `/stats`

### `/find`

Search the transcript for matching messages and tool output.

**Syntax:**

```text
/find                      # opens a search-input modal
/find authentication error # highlights matching lines
```

Results are highlighted in the transcript view.

### `/vision`

Open the vision-image picker to attach one or more images for the next turn. Supports the same validation as `/attach`.

**Syntax:** `/vision`

---

## Command Reference Table

| Command | Category | Inline Args | Modal/Picker | Restart Required |
|---|---|---|---|---|
| `/help`, `/?` | Navigation | — | Help screen | — |
| `/settings` | Navigation | — | Settings modal | — |
| `/keybinds` | Navigation | — | Keybind editor | — |
| `/status` | Navigation | — | (logs to transcript) | — |
| `/sessions` | Navigation | — | Sessions picker | — |
| `/new` | Navigation | — | — | — |
| `/reset` | Conversation | — | Confirmation | — |
| `/clear` | Conversation | — | — | — |
| `/undo` | Conversation | — | — | — |
| `/compact` | Conversation | preservation instructions | Modal (if no args) | �� |
| `/context` | Conversation | — | (logs to transcript) | — |
| `/copy` | Conversation | — | — | — |
| `/attach` | Conversation | path + prompt | Path modal (if no args) | — |
| `/abort` | Conversation | — | — | — |
| `/steer` | Conversation | steering text | Modal (if no args) | — |
| `/login` | Auth | — | Provider picker | — |
| `/logout` | Auth | provider name | Provider picker (if no args) | — |
| `/oauth-code` | Auth | OAuth code | Code paste modal (if no args) | — |
| `/search-key` | Auth | provider + key/--clear | Provider picker (if no args) | — |
| `/model`, `/models` | Auth | index or substring | Model picker (if no args) | — |
| `/usage` | Auth | — | Usage modal | — |
| `/approval` | Settings | mode | Picker (if no args) | — |
| `/reasoning` | Settings | level | Picker (if no args) | — |
| `/bash-timeout` | Settings | seconds | Modal (if no args) | — |
| `/auto-compact` | Settings | on/off | Picker (if no args) | — |
| `/sandbox` | Settings | mode | Picker (if no args) | Yes |
| `/no-network` | Settings | on/off | Picker (if no args) | Yes |
| `/mouse-wheel` | Settings | on/off | Picker (if no args) | — |
| `/footer-metrics` | Settings | on/off | Picker (if no args) | — |
| `/idle-timeout` | Settings | seconds ≥10 | Modal (if no args) | Yes |
| `/max-session-tokens` | Settings | tokens (0=unlimited) | Modal (if no args) | Yes |
| `/theme` | Settings | — | Theme picker | — |
| `/remember` | Memory | text | Modal (if no args) | — |
| `/memory` | Memory | — | Memory list picker | — |
| `/forget` | Memory | memory name | Memory picker (if no args) | ��� |
| `/index` | Memory | `--full`/`--incremental` | ��� | — |
| `/reflect` | Memory | — | — | — |
| `/plugin-install` | Plugins | path + scope | Picker (if no scope) | — |
| `/plugin-list` | Plugins | — | Plugin list | — |
| `/plugin-config` | Plugins | — | Toggle picker | — |
| `/plugin-enable` | Plugins | plugin name | Toggle picker (if no args) | �� |
| `/plugin-disable` | Plugins | plugin name | Toggle picker (if no args) | ��� |
| `/plugin-remove` | Plugins | plugin name | Picker (if no args) | — |
| `/plugin-reload` | Plugins | — | — | — |
| `/goal` | Goals | goal text | Modal (if no args) | — |
| `/cancel-goal` | Goals | — | — | — |
| `/run` | Subagents | name + prompt | — | — |
| `/parallel` | Subagents | name + prompt | — | — |
| `/chain` | Subagents | name + prompt | — | — |
| `/subagents` | Subagents | — | Delegation | — |
| `/subagents-doctor` | Subagents | — | Delegation | — |
| `/subagents-status` | Subagents | — | Delegation | — |
| `/subagents-models` | Subagents | — | Delegation | — |
| `/exit`, `/quit` | Other | — | ��� | — |
| `/stats` | Other | — | (logs to transcript) | — |
| `/find` | Other | search query | Modal (if no args) | — |
| `/vision` | Other | — | Vision picker | — |

---

## Related

- [CLI Reference](cli.md) — Flags for the `catcode` and `core` binaries.
- [Configuration](../configuration/index.md) — Settings persisted across sessions.
