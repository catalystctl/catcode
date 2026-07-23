# CLI Reference

Catalyst Code ships two binaries:

- **`catcode`** — the terminal UI (Go/Bubble Tea). Interactively spawns and controls the core.
- **`core`** — the engine (Rust). A stdio JSON-RPC server that the TUI or SDK spawns. Not designed for direct interactive use.

---

## `catcode` �� Terminal UI

The TUI binary is the primary user-facing entry point.

### Usage

```text
catcode                       start the interactive TUI
catcode --update              update CLI (and web frontend if installed)
catcode --check-update        report whether an update is available
catcode --version, -v         print version
catcode --help, -h            show help
```

### Flags

| Flag | Purpose | Default |
|---|---|---|
| _(no args)_ | Start the interactive TUI | — |
| `-h`, `--help` | Print usage text and exit | — |
| `-v`, `--version` | Print version and exit | — |
| `--check-update` | Check GitHub Releases for a newer binary. Exits 0 regardless (scripting-friendly). | — |
| `--update`, `-u`, `update` | Download and atomically replace the running binary with the latest release. If the web frontend is installed, also refreshes `core` and the web bundle, then restarts the service. | — |

### Version

The version is the git commit short SHA, injected at build via `-ldflags -X main.coreVersion=<SHA>`. Local (non-release) builds report `dev`.

### Environment Variables

| Variable | Purpose |
|---|---|
| `CATCODE_CORE` | Explicit path to the `core` binary. If set and the file exists, the TUI uses it instead of searching the install layout or PATH. |

### Core Binary Discovery

The TUI locates the `core` binary in this order:

1. `$CATCODE_CORE` (explicit override)
2. `<dir of catcode>/catcode-core[.exe]` (installed layout)
3. `catcode-core` on `PATH`
4. Development fallbacks (only when the TUI is a dev build): `core/target/release/catcode-core`, `../core/target/release/catcode-core`

---

## `core` — Engine

The core is a stdio JSON-RPC server. It reads newline-delimited JSON commands from stdin and
writes newline-delimited JSON events to stdout. The TUI and SDK spawn this process and
communicate over its stdio.

### Usage

```text
core [OPTIONS]
```

### Flags

| Flag | Type | Default | Environment Variable | Purpose |
|---|---|---|---|---|
| `--workspace <DIR>` | path | current directory | `CATALYST_CODE_WORKSPACE` | Workspace root that constrains all file and bash operations. |
| `--base-url <URL>` | url | `https://api.code.umans.ai/v1` | `UMANS_BASE_URL` | OpenAI-compatible API base URL. |
| `--approval <MODE>` | enum | `destructive` | `CATALYST_CODE_APPROVAL` | Approval gate mode: `never` (auto-approve), `destructive` (ask for bash/write/edit), `always` (ask for every tool). |
| `--bash-timeout <SECS>` | integer | `30` | `CATALYST_CODE_BASH_TIMEOUT` | Per-command bash timeout in seconds. |
| `--max-bash-timeout <SECS>` | integer | `600` | `CATALYST_CODE_MAX_BASH_TIMEOUT` | Ceiling for the bash tool's per-call `timeout` override. |
| `--fetch-timeout <SECS>` | integer | `20` | `CATALYST_CODE_FETCH_TIMEOUT` | Wall-clock timeout for the `fetch` tool. |
| `--diag-timeout <SECS>` | integer | `120` | `CATALYST_CODE_DIAG_TIMEOUT` | Diagnostics tool (`cargo check`/`tsc`/`go build`) timeout. |
| `--sandbox <MODE>` | enum | `none` | `CATALYST_CODE_SANDBOX` | Sandbox for agent workloads: `none`, `microsandbox` (runs bash/git/diagnostics/plugins in a Microsandbox microVM on Linux KVM, Apple Silicon macOS, Windows WHP). Legacy `firejail`/`seatbelt` migrate to `microsandbox`. |
| `--no-network` | flag | `false` | `CATALYST_CODE_NO_NETWORK=1` | Block guest network egress (Microsandbox network policy). |
| `--trust-project-plugins` | flag | `false` | `CATALYST_CODE_TRUST_PROJECT_PLUGINS=1` | Load project-scoped plugins (`.catalyst-code/plugins`). Off by default for safety. |
| `--idle-timeout <SECS>` | integer | `120` | `CATALYST_CODE_IDLE_TIMEOUT` | SSE idle timeout. |
| `--max-session-tokens <N>` | integer | `0` (unlimited) | `CATALYST_CODE_MAX_SESSION_TOKENS` | Hard session token budget. `0` = unlimited. |
| `--debug-log <FILE>` | path | none | `CATALYST_CODE_DEBUG_LOG` | Structured JSONL debug log path. |
| `--session <FILE>` | path | none | `CATALYST_CODE_SESSION` | Append-only JSONL session file (resumed on restart). |
| `--model <ID>` | string | none | — | Default model ID. |
| `--provider <NAME>` | string | none | `UMANS_ACTIVE_PROVIDER` | Active provider name (matches a `providers[]` entry in config). |
| `--config <FILE>` | path | `./catalyst-code.json`, `~/.config/catalyst-code/config.json` | — | JSON config file. |
| `-h`, `--help` | flag | — | — | Print help text and exit. |
| `-V`, `--version` | flag | ��� | — | Print version (`CARGO_PKG_VERSION`) and exit. |

### Environment Variables (no CLI flag equivalent)

These configuration keys can only be set via environment variable or config file:

| Variable | Type | Default | Purpose |
|---|---|---|---|
| `CATALYST_CODE_FETCH_MAX_BYTES` | integer | `262144` (256 KiB) | Maximum response body size for the `fetch` tool. |
| `CATALYST_CODE_FETCH_ALLOWLIST` | comma-separated hostnames | empty (any host allowed) | Restrict `fetch` to specific hostnames. |
| `CATALYST_CODE_AUTO_COMPACT` | bool | `true` | Automatically compact conversation when approaching context limit. |
| `CATALYST_CODE_COMPACT_INSTRUCTIONS` | string | none | Custom instructions preserved through compaction. |
| `CATALYST_CODE_AUTO_REFLECT` | bool | `true` | Automatically reflect after tool turns to persist learnings. |
| `CATALYST_CODE_AUTO_REFLECT_MIN_TOOL_CALLS` | integer | `1` | Minimum tool calls before auto-reflect triggers. |
| `UMANS_PROVIDERS` | JSON array | none | Inline provider definitions (same schema as `providers[]` in config file). |

### Sandbox Mode Aliases

| CLI value | Aliases | Effective Mode |
|---|---|---|
| `none` | `off`, `false`, `disabled` | No sandboxing (host execution) |
| `microsandbox` | `msb`, `on`, `true`, `enabled` | Microsandbox microVM |
| (legacy) `firejail` | `fj` | migrated to `microsandbox` (deprecation notice) |
| (legacy) `seatbelt` | `macos`, `sandbox-exec` | migrated to `microsandbox` (deprecation notice) |

See the [Sandbox Guide](../guides/sandbox.md) for platform requirements. Legacy
values never silently downgrade to `none`; if the environment cannot run
Microsandbox, the sandbox fails closed.

### Approval Mode Values

| Value | Behavior |
|---|---|
| `never` | Auto-approve every tool call (fully trust the model). |
| `destructive` | Ask for confirmation only on bash, write_file, and edit operations (default). |
| `always` | Ask for confirmation on every tool call. |

### Config Precedence

Values are resolved lowest-to-highest:

1. Built-in defaults
2. Config files (managed config dir, `settings.json`, `catalyst-code.json`, `--config` path)
3. Environment variables
4. CLI flags (highest — applied last so the TUI's `--approval` always wins)

### Config File Location

The core searches these paths in order (earlier files win unless overridden by env/CLI):

- `<managed config dir>/catalyst-code.json` (platform-dependent; see `dirs::config_dir`)
- Files in `<managed config dir>/catalyst-code.d/` (sorted lexicographically)
- `~/.config/catalyst-code/settings.json`
- `./settings.local.json`
- `./settings.json`
- `--config <FILE>` path (if specified, only that file is loaded; no fallbacks)

---

## Protocol

Both binaries use newline-delimited JSON over stdio:

```jsonl
{"type": "send", "prompt": "hello", "model": "gpt-4o"}
{"type": "event", ...}
```

- **Commands** (TUI → core): type-tagged JSON objects on stdin.
- **Events** (core �� TUI): type-tagged JSON objects on stdout.
- The TUI parses events and renders them in the terminal.
- There is no HTTP server, REST API, or gRPC endpoint in the default configuration.
- The `--base-url` flag points to an upstream OpenAI-compatible API, not to a local service.

---

## Examples

### Start the TUI (development)

```bash
catcode
```

### Run core directly (debugging)

```bash
core --workspace /path/to/project --approval never
```

### Restrict workspace and enable sandbox

```bash
core --workspace /home/user/project --sandbox microsandbox
```

### Set provider and model from environment

```bash
export UMANS_BASE_URL="https://api.openai.com/v1"
export UMANS_ACTIVE_PROVIDER="openai"
core --model gpt-4o
```

### Disable network and run fully isolated (auto-approve, no egress)

```bash
core --sandbox microsandbox --no-network --approval never --workspace /tmp/sandbox
```

### Check for TUI update

```bash
catcode --check-update
```

### Update the TUI

```bash
catcode --update
```

---

## Related

- [Slash Commands](slash-commands.md) — Interactive commands available inside the TUI.
- [Configuration](../configuration/index.md) — Full config file schema.
