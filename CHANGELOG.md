# Changelog

## Unreleased

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
