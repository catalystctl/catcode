# Evidence Ledger

Internal document linking every documented surface to its source evidence.
**Not user-facing.** Used for maintenance and gap analysis.

## CLI — TUI binary (`catcode`)

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| `catcode` (no args) | `tui/main.go` | `main()` → `tea.NewProgram(initialSession())` | high |
| `catcode --update` | `tui/update.go` | `handleCLIArgs` switch case `"--update"` → `runUpdate()` | high |
| `catcode --check-update` | `tui/update.go` | `"--check-update"` → `runCheckUpdate()` | high |
| `catcode --version` / `-v` | `tui/update.go` | `"-v", "--version"` → prints `coreVersion` | high |
| `catcode --help` / `-h` | `tui/update.go` | `"-h", "--help"` → `printUsage()` | high |

## CLI — Core binary (`core`)

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| `--workspace` | `core/src/config.rs` | CLI parsing at `"--workspace"` arm | high |
| `--base-url` | `core/src/config.rs` | `"--base-url"` arm | high |
| `--approval` | `core/src/config.rs` | `"--approval"` → `Approval::parse()` | high |
| `--bash-timeout` | `core/src/config.rs` | `"--bash-timeout"` arm | high |
| `--max-bash-timeout` | `core/src/config.rs` | `"--max-bash-timeout"` arm | high |
| `--fetch-timeout` | `core/src/config.rs` | `"--fetch-timeout"` arm | high |
| `--diag-timeout` | `core/src/config.rs` | `"--diag-timeout"` arm | high |
| `--sandbox` | `core/src/config.rs` | `"--sandbox"` → `Sandbox::parse()` | high |
| `--no-network` | `core/src/config.rs` | `"--no-network"` arm | high |
| `--trust-project-plugins` | `core/src/config.rs` | `"--trust-project-plugins"` arm | high |
| `--idle-timeout` | `core/src/config.rs` | `"--idle-timeout"` arm | high |
| `--max-session-tokens` | `core/src/config.rs` | `"--max-session-tokens"` arm | high |
| `--debug-log` | `core/src/config.rs` | `"--debug-log"` arm | high |
| `--session` | `core/src/config.rs` | `"--session"` arm | high |
| `--model` | `core/src/config.rs` | `"--model"` arm | high |
| `--provider` | `core/src/config.rs` | `"--provider"` arm | high |
| `--config` | `core/src/config.rs` | `"--config"` arm | high |
| `-h` / `--help` | `core/src/config.rs` | help = true → `print_help()` | high |
| `-V` / `--version` | `core/src/config.rs` | version = true → prints version | high |

## Slash Commands

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| `/login` | `tui/handlers.go` | `case "/login"` → `openLoginPicker()` | high |
| `/search-key` | `tui/handlers.go` | `case "/search-key"` | high |
| `/logout` | `tui/handlers.go` | `case "/logout"` | high |
| `/oauth-code` | `tui/handlers.go` | `case "/oauth-code"` | high |
| `/model` / `/models` | `tui/handlers.go` | `case "/model", "/models"` | high |
| `/reset` | `tui/handlers.go` | `case "/reset"` | high |
| `/abort` | `tui/handlers.go` | `case "/abort"` | high |
| `/exit` / `/quit` | `tui/handlers.go` | `case "/exit", "/quit"` | high |
| `/steer` | `tui/handlers.go` | `case "/steer"` | high |
| `/approval` / `/approvals` | `tui/handlers.go` | `case "/approval", "/approvals"` | high |
| `/reasoning` | `tui/handlers.go` | `case "/reasoning"` | high |
| `/bash-timeout` | `tui/handlers.go` | `case "/bash-timeout"` | high |
| `/auto-compact` | `tui/handlers.go` | `case "/auto-compact"` | high |
| `/sandbox` | `tui/handlers.go` | `case "/sandbox"` | high |
| `/no-network` | `tui/handlers.go` | `case "/no-network"` | high |
| `/mouse-wheel` | `tui/handlers.go` | `case "/mouse-wheel"` | high |
| `/footer-metrics` | `tui/handlers.go` | `case "/footer-metrics"` | high |
| `/idle-timeout` | `tui/handlers.go` | `case "/idle-timeout"` | high |
| `/max-session-tokens` | `tui/handlers.go` | `case "/max-session-tokens"` | high |
| `/help` / `/?` | `tui/handlers.go` | `case "/help", "/?"` | high |
| `/settings` | `tui/handlers.go` | `case "/settings"` | high |
| `/keybinds` | `tui/handlers.go` | `case "/keybinds"` | high |
| `/theme` | `tui/handlers.go` | `case "/theme"` | high |
| `/copy` | `tui/handlers.go` | `case "/copy"` | high |
| `/attach` | `tui/handlers.go` | `case "/attach"` | high |
| `/clear` | `tui/handlers.go` | `case "/clear"` | high |
| `/undo` | `tui/handlers.go` | `case "/undo"` | high |
| `/compact` | `tui/handlers.go` | `case "/compact"` | high |
| `/context` | `tui/handlers.go` | `case "/context"` | high |
| `/usage` | `tui/handlers.go` | `case "/usage"` | high |
| `/remember` | `tui/handlers.go` | `case "/remember"` | high |
| `/memory` | `tui/handlers.go` | `case "/memory"` | high |
| `/forget` | `tui/handlers.go` | `case "/forget"` | high |
| `/index` | `tui/handlers.go` | `case "/index"` | high |
| `/reflect` | `tui/handlers.go` | `case "/reflect"` | high |
| `/sessions` | `tui/handlers.go` | `case "/sessions"` | high |
| `/new` | `tui/handlers.go` | `case "/new"` | high |
| `/stats` | `tui/handlers.go` | `case "/stats"` | high |
| `/status` | `tui/handlers.go` | `case "/status"` | high |
| `/find` | `tui/handlers.go` | `case "/find"` | high |
| `/plugin-install` | `tui/handlers.go` | `case "/plugin-install"` | high |
| `/plugin-list` / `/plugin-config` / `/plugin-enable` / `/plugin-disable` | `tui/handlers.go` | `case "/plugin-list", "/plugin-config", "/plugin-enable", "/plugin-disable"` | high |
| `/vision` | `tui/handlers.go` | `case "/vision"` | high |
| `/plugin-remove` | `tui/handlers.go` | `case "/plugin-remove"` | high |
| `/plugin-reload` | `tui/handlers.go` | `case "/plugin-reload"` | high |
| `/goal` | `tui/handlers.go` | `case "/goal"` | high |
| `/cancel-goal` | `tui/handlers.go` | `case "/cancel-goal"` | high |
| `/run` | `tui/handlers.go` | `case "/run"` | high |
| `/parallel` | `tui/handlers.go` | `case "/parallel"` | high |
| `/chain` | `tui/handlers.go` | `case "/chain"` | high |
| `/subagents` / `/subagents-list` | `tui/handlers.go` | `case "/subagents", "/subagents-list"` | high |

## Built-in Tools

| Surface | Class | Source | Evidence | Confidence |
|---------|-------|--------|----------|------------|
| `read_file` | ReadOnly | `core/src/tools.rs` | `classify()` → `ToolKind::ReadOnly` | high |
| `edit` | Destructive | `core/src/tools.rs` | falls to default → Destructive | high |
| `write_file` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `delete` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `rename` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `mkdir` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `list_dir` | ReadOnly | `core/src/tools.rs` | `classify()` → ReadOnly | high |
| `grep` | ReadOnly | `core/src/tools.rs` | `classify()` → ReadOnly | high |
| `glob` | ReadOnly | `core/src/tools.rs` | `classify()` → ReadOnly | high |
| `bash` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `todo_write` | Destructive | `core/src/tools.rs` | explicit ReadOnly? No — default Destructive. Check: `classify()` doesn't list `todo_write`. Let me verify. | medium |
| `todo_read` | ReadOnly | `core/src/tools.rs` | `classify()` lists `todo_read` → ReadOnly | high |
| `finish` | ReadOnly | `core/src/tools.rs` | `classify()` lists `finish` �� ReadOnly | high |
| `memory` | ReadOnly | `core/src/tools.rs` | `classify()` lists `memory` → ReadOnly | high |
| `knowledge` | (check classify) | See `classify()` for details | medium |
| `ask` | ReadOnly | `core/src/tools.rs` | `classify()` lists `ask` → ReadOnly | high |
| `load_tools` | ReadOnly | `core/src/tools.rs` | `classify()` lists `load_tools` → ReadOnly | high |
| `subagent` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `patch` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `bulk` | Destructive | `core/src/tools.rs` | deferred, default → Destructive | high |
| `bulk_read` | ReadOnly | `core/src/tools.rs` | `classify()` lists `bulk_read` → ReadOnly | high |
| `bulk_write` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `bulk_edit` | Destructive | `core/src/tools.rs` | default → Destructive | high |
| `goal_write_plan` | (deferred) | `core/src/tools.rs` | deferred_tool_names includes it | high |
| `diagnostics` | ReadOnly | `core/src/tools.rs` | `classify()` lists `diagnostics` → ReadOnly | high |
| `fetch` | (deferred) | `core/src/tools.rs` | deferred_tool_names includes it; classified by `is_parallel_wave_tool` | high |
| `web_search` | ReadOnly | `core/src/tools.rs` | `classify()` lists `web_search` → ReadOnly | high |
| `git_status` | ReadOnly | `core/src/tools.rs` | `classify()` lists `git_status` → ReadOnly | high |
| `git_diff` | ReadOnly | `core/src/tools.rs` | `classify()` lists `git_diff` → ReadOnly | high |
| `git_log` | ReadOnly | `core/src/tools.rs` | `classify()` lists `git_log` → ReadOnly | high |
| `workspace_activity` | ReadOnly | `core/src/tools.rs` | `classify()` lists `workspace_activity` → ReadOnly | high |
| `git_add` | Destructive | `core/src/tools.rs` | deferred, default → Destructive | high |
| `git_commit` | Destructive | `core/src/tools.rs` | deferred, default → Destructive | high |
| `spawn` | Destructive | `core/src/tools.rs` | deferred, default → Destructive | high |
| `test_env` | (deferred) | `core/src/tools.rs` | deferred_tool_names includes it | high |

## Browser Tools

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| `browser_create` | `core/src/tools.rs` | deferred_tool_names | high |
| `browser_screenshot` | `core/src/tools.rs` + `core/src/browser/` | ReadOnly via `is_browser_readonly` | high |
| All browser tools | `core/src/browser/` | Module exists with 18+ tools | high |

## Configuration

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| Config precedence | `core/src/config.rs` | Comment line 2: CLI > env > settings.local.json > … | high |
| Approval modes | `core/src/config.rs` | `Approval::parse()`, 3 variants | high |
| Sandbox modes | `core/src/config.rs` | `Sandbox::parse()`, 3 variants | high |
| Provider config | `core/src/config.rs` | `ProviderConfig` struct | high |
| Permission rules | `core/src/config.rs` | `PermissionRule` struct + `parse_permission_rule()` | high |
| Workspace confinement | `core/src/workspace.rs` | `resolve()`, `check_dangerous_path()` | high |
| Restricted paths | `core/src/workspace.rs` | `DANGEROUS_PATHS` const | high |

## Plugin System

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| Hook points | `core/src/plugins.rs` | `HOOK_POINTS` const, 18 entries | high |
| Hook policy | `core/src/plugins.rs` | `hook_policy()` function | high |
| Plugin manifest | `core/src/plugins.rs` | `PluginManifest` deserialization | high |
| Tool declarations | `core/src/plugins.rs` | `ToolManifestEntry`, `ToolConfig` structs | high |
| OAuth providers | `core/src/plugins.rs` | Plugin OAuth fields in manifest | high |
| Memory providers | `core/src/plugins.rs` | Plugin memory_provider field | high |

## Goal Mode

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| Phase machine | `core/src/goal.rs` | `GoalPhase` enum, 9 phases | high |
| goal_write_plan | `core/src/goal.rs` + `core/src/tools.rs` | Tool definition + deferred classification | high |
| /goal command | `tui/handlers.go` | `case "/goal"` | high |
| Goal state events | `core/src/goal.rs` | goal_state → `emit()` calls | high |

## Subagents

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| Built-in agents | `core/src/subagent.rs` + `.catalyst-code/agents/` | list_dir shows 22 agent files | high |
| Execution modes | `core/src/subagent.rs` | `single/parallel/chain` tool param enum | high |
| Management actions | `core/src/subagent.rs` | `list/get/create/update/delete/status/…` | high |
| Intercom bus | `core/src/intercom.rs` | Full module with execute functions | high |
| Frontmatter parsing | `core/src/subagent.rs` | `parse_frontmatter()` function | high |

## Wire Protocol

| Surface | Source | Evidence | Confidence |
|---------|--------|----------|------------|
| Command enum | `core/src/protocol/commands.rs` | `Command` enum with serde tags | high |
| Event struct | `core/src/protocol/events.rs` | `Event` struct + constructors | high |
| emit function | `core/src/protocol/events.rs` | centralized sink-backed `emit` | high |

## Existing Documentation Audit

| Document | Classification | Notes |
|----------|----------------|-------|
| `README.md` | accurate | 596 lines, comprehensive. Links to new docs needed. |
| `CONTRIBUTING.md` | accurate | 160 lines, still current |
| `CHANGELOG.md` | accurate | Day-by-day format, current |
| `docs/IDE_PANELS_CONTRACT.md` | accurate | 802 lines, IDE shell contract |
| `docs/SELF_LEARNING.md` | accurate | 878 lines, self-learning design |
| `docs/PLUGINS.md` | accurate | 426 lines, points to plugin-authoring skill |
| `docs/logo.svg` | accurate | SVG logo, not documentation |
| `docs/examples/plugins/README.md` | accurate | 28 lines, plugin examples |
