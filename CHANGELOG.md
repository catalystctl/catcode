# Changelog

## Unreleased

### Fixed - plugin hook dispatch for write/edit/bash/read
- Pre-execution hooks (`pre_write`/`pre_bash`/`pre_read`) ran **twice** per
  tool call: a leftover dead loop re-executed every hook, so on denial the model
  received two "denied" tool-result messages (double token counting and
  duplicate context pollution) and any side-effectful hook fired twice. The
  dispatch is now a single pass.
- A hook's `modify` now **merges** over the original tool args (per-key) instead
  of replacing them wholesale. Previously a `pre_write` hook returning
  `{"content": "..."}` (as in the bundled lint-check example) dropped `path`, so
  the write targeted the wrong/empty path. `path`/`edits`/`command`/etc. are now
  preserved and hooks compose (each sees earlier amendments). New helper
  `plugins::apply_modify` (with unit tests) locks the contract.
- Hook `reason` is now surfaced to the model: on `allow` it is appended to the
  tool result as a `Plugin hooks:` note, and on deny it is the deny message - so
  the model knows its write/edit/bash/read was inspected or modified and can
  react. `PLUGIN_DOCS` updated to match (merge semantics + reason surfacing).
- Subagent tool dispatch now applies the same dangerous-path guard
  (`workspace::check_dangerous_path`) to `write_file`/`edit`/`patch` as the main
  loop, so scoped subagent writes to `.git/**`, `**/.ssh/**`, `**/.env*`, etc.
  are blocked and the subagent model is told why.

### Added — macOS standalone executable
- `release-macos.sh` cross-compiles the harness into a single self-contained
  macOS executable per arch (arm64 + x86_64): the Rust core is built with
  `cargo zigbuild` (zig as the macOS linker; pure-Rust `rustls-tls` so no
  macOS SDK is needed), then embedded into the Go TUI via `go:embed`
  (`-tags embed_core`). Each output file runs from any CWD — it extracts its
  bundled core to `~/Library/Caches/umans-harness` on first run and launches
  the harness in that directory, with no separate `umans-core` and no install.
- TUI: new `embeddedCorePath()` (build-tagged `embed_core`) is wired into
  `coreBinaryPath()` ahead of the usual `$UMANS_CORE`/dev/installed search;
  it's a no-op stub in normal builds, so dev, Linux, and the Windows MSI
  layout are unchanged.

### Changed
- Removed the fixed agentic turn cap (`--max-turns` / `max_turns`, default 200)
  and the `spawn` sub-agent turn cap (`spawn_max_turns`, default 10). Turns are
  now bounded only by the session token budget (`--max-session-tokens`, 0 =
  unlimited), the `finish` tool, abort, or the model stopping. Removed: the
  `--max-turns` flag, `UMANS_HARNESS_MAX_TURNS` env var, `max_turns` config
  key, `ready` event's `max_turns` field, the `set_config max_turns` knob, and
  the TUI "Max Turns" setting. Stale `max_turns` config values are ignored.

### Fixed
- Tokens-per-second (`tps` in the `metrics` event, shown as "tok/s" in the
  footer) was wrong: it divided the *last* request's output tokens by the
  whole turn's wall time — including every tool call's execution/wait time
  and every prefill. It now divides total output tokens across all requests
  by accumulated *generation time* (each request's first-token → end
  window), so tool-call waits and prefill (TTFT) are excluded — pure model
  throughput, not end-to-end wall time. The live mid-stream TPS now times
  each request from its own first token (not the turn's), and
  `mark_first_token` fires on the first reasoning chunk as well as content,
  so reasoning-model (GLM @ high) TPS is accurate.

### Added — in-flight steer, follow-up & commands
- While a turn is running the input now stays live so you can compose a
  follow-up, steer the model, or run commands without waiting:
  - **Enter** queues a follow-up message (runs after the current turn); the
    core's one-deep buffer drains it automatically.
  - **Ctrl+Enter** steers: interrupts the running turn and redirects it with
    the typed message (new `steer` command). On terminals where Ctrl+Enter
    isn't reported as a distinct key, `/steer <msg>` works everywhere.
  - **Esc** aborts the turn (and drops any queued follow-up/steer).
    `/abort` still works.
  - Slash commands and the palette (`ctrl+p`/`ctrl+k`) are usable mid-turn.
- A `queuedNext` flag keeps the TUI streaming across the steer/follow-up
  hand-off so the footer never flashes "ready" between chained turns.
- Core: the run-loop drain (`run_turn_and_drain`) is shared by `send` and
  `steer`; `abort` now also clears the queued prompt.

### Added — reasoning level control
- Models now advertise their supported thinking levels via a new
  `thinking_levels` field on `ModelInfo` (e.g. `["low","medium","high"]`).
  Levels are read from `/models/info` (`capabilities.thinking_levels`,
  `reasoning_levels`, or `reasoning_efforts`) when the endpoint provides them,
  and fall back to the built-in snapshot otherwise.
- `reasoning_effort` is validated against the selected model's levels before
  each request (`resolve_effort`): an unsupported effort is clamped to the
  closest preferred level (high → medium → low → … → first). This replaces
  the hardcoded `model.contains("glm")` sniff — GLM now advertises only
  `["high"]` and is clamped data-driven-ly. The core emits an `info` event
  when clamping occurs.
- TUI: the settings "Reasoning" field cycles the *selected model's* levels
  (not a fixed low/medium/high), and the effort is clamped on model load,
  model switch, and `/model` so the displayed value always matches the wire
  field. The model picker now shows `think:low/medium/high`.

## 0.2.0 — 2026-06-19 (production hardening)

### Fixed
- `bulk_edit` called `execute(path, …)` with the *path* as the tool name;
  every multi-file edit returned "unknown tool: <path>". Now passes `"edit"`.
- Settings modal sent `set_config` but the core had no such command; bash-timeout
  and max-turns edits were dead. Added `SetConfig` + `config_changed` event;
  `ready` now emits `bash_timeout_secs` / `max_turns`.
- TUI never passed `--session`, so persistent chats were unreachable.
  The TUI now writes one JSONL per workspace under
  `~/.config/umans-harness/sessions/`.
- Flaky `workspace::tests::relative_inside_ok` under parallel `cargo test`
  (shared fixed temp dir). Now uses a unique dir per call.

### Added — agentic loops / infinite turns
- `max_turns` default 25 → 200; real ceiling is the session token budget.
- `finish` tool lets the model exit the loop cleanly.
- `spawn` tool: nested agentic turn with a fresh sub-conversation (depth 1),
  bounded by `--max-turns` for the sub-agent (`spawn_max_turns`, default 10).
- `todo_write` / `todo_read`: persistent plan that survives context compaction.
- `patch` tool: unified-diff applier for larger refactors than `edit`.
- `diagnostics` tool: runs `cargo check` / `tsc --noEmit` / `go build` /
  `py_compile` so the model can type-check its work.
- Summarizing compaction: dropped turns are summarized by a model call
  (`summarize_on_compact`, default on) instead of vanishing. Falls back to the
  drop-oldest marker if the summarize call fails.
- `--max-session-tokens` hard session budget; trips before the request.
- Read-file pagination (`offset`/`limit`); limits raised to 5 MB / 10 000 lines.
- `bash` output cap 8 KB → 32 KB, truncating the *head* so errors survive.
- `grep` size guard (5 MB/file) + binary sniff (NUL bytes).

### Added — persistent chats
- Session JSONL is now versioned (`_session_version` header) + fsync'd.
- `/clear` (in-memory only, keep file) vs `/reset` (wipe both).
- `/undo` drops the last turn. `/compact` forces compaction.
- `/sessions` lists saved sessions; `/load_session` loads one.
- `/stats` reports token + turn totals.

### Added — security
- `--sandbox firejail`: wraps bash in a generated firejail profile that
  whitelists only the workspace + shell paths.
- `--no-network`: `unshare -n` so bash can't phone home.
- Per-tool-*kind* approval escalation: "always" now un-gates only the matched
  kind, not the whole session.

### Added — reliability
- `--idle-timeout` (default 120s, was hard-coded 60s); reasoning models that
  think >60s before the first token no longer abort.
- `reasoning_effort` + `reasoning_content` replay gated on Umans base URL;
  other OpenAI-compatible servers no longer 400 on the unknown fields.
- TUI auto-restarts the core once on unexpected exit (crash recovery), with
  a persisted-key re-auth.
- `send` while a turn is running queues (one-deep) instead of dropping the
  prompt.

### Added — tooling / UX
- Image/vision input: `--allow-vision`, `/attach <path>` command, multimodal
  user messages (data URLs, with magic-byte mime sniffing).
- New slash commands in the palette + help: `/clear`, `/undo`, `/compact`,
  `/sessions`, `/stats`, `/attach`.
- Settings modal: Sandbox, No Network, Idle Timeout, Max Session Tokens.

### Packaging
- `Dockerfile` (multi-stage, firejail + ca-certs in the runtime image).
- `release.sh` builds + tars + checksums a versioned release.
- This `CHANGELOG.md`.
