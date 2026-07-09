# Changelog

## Unreleased

### Fixed — TUI modified Enter in SSH/Konsole
- **Shift+Enter** now inserts a newline reliably in terminals that support
  enhanced keyboard reporting (including Konsole over SSH). The TUI enables
  xterm `modifyOtherKeys` level 2 and Kitty progressive keyboard reporting
  after Bubble Tea enters the alternate screen, so modified Enter keys are
  reported distinctly instead of arriving as plain Enter or leaking `OM` into
  the input.
- **Ctrl+Enter** now steers instead of queueing a follow-up when the terminal
  reports the modifier. Enhanced Ctrl+letter reports are translated back into
  Bubble Tea's normal key messages, preserving existing bindings such as
  Ctrl+C, Ctrl+P, Ctrl+K, Ctrl+T, and Ctrl+O.
- Added regression coverage for modified-Enter CSI sequences and the legacy
  SS3 keypad-Enter fallback.

### Added — cross-platform installers & standalone executables
- All three desktop platforms now ship BOTH a native installer AND a
  self-contained standalone executable, so `catcode` runs from the terminal with
  no install on any OS:
  - **Windows** — `catcode-<ver>-windows.msi` (per-user MSI, adds `catcode` to PATH,
    no admin) **and** `catcode-<ver>-windows-x86_64.exe` (single file, Rust core
    embedded via `go:embed` `-tags embed_core`; extracts its core to
    `%LOCALAPPDATA%\catalyst-code` on first run). `release-windows.sh` now
    emits both (previously MSI + zip only).
  - **macOS** — `catcode-<ver>-macos-{arm64,x86_64}.dmg` (disk-image installer:
    mount + double-click `Install catcode.command` to put `catcode` on `/usr/local/bin`)
    **and** the existing `catcode-<ver>-macos-{arm64,x86_64}` standalone.
    `release-macos.sh` now builds the `.dmg` too (`hdiutil` real UDIF on macOS,
    `xorriso` HFS+ hybrid when cross-built on Linux).
  - **Linux** — `catcode-<ver>-<arch>.AppImage` (self-contained AppImage wrapping
    the embedded-core standalone) **and** `catcode-<ver>-linux-<arch>`
    standalone. New `release-linux.sh` builds the core natively, embeds it into
    the TUI, generates a terminal-prompt icon, and packages the AppImage with
    `appimagetool` (fetched on demand; runs without FUSE via
    `APPIMAGE_EXTRACT_AND_RUN=1`).
  - New `release-all.sh` runs all three platform scripts and reports
    per-platform pass/fail, so a partial toolchain still ships what it can.
  - Packaging assets: `packaging/linux/{AppRun,catcode.desktop,make-icon.py}` and
    `packaging/macos/{install.command,README.txt}`.
- `tui/embed_core.go`: the extracted embedded core now gets the platform exe
  suffix (`coreExeSuffix()`), so on Windows it caches as
  `catcode-core-<ver>-windows-amd64.exe` (a proper PE name CreateProcess exec's
  and AV tools recognize); unchanged on macOS/Linux.
- Fixed a latent `go build -ldflags` bug: `release-windows.sh` and `release.sh`
  used `-X main/coreVersion` with a slash (`main/coreVersion`), which the Go
  1.24 linker rejects. Corrected to `-X main.coreVersion` (dot), matching
  `release-macos.sh`.

### Added — core tooling, compaction & approval improvements
- **`fetch` tool** (native HTTP GET): a first-class read-only tool that fetches
  a URL and returns lightly HTML-stripped text (bounded to `fetch_max_bytes`,
  default 256 KiB). Unlike `bash curl`, it is NOT subject to the bash sandbox, so
  it still works under `--no-network` where bash egress is dead — but it honors
  `--no-network` unless you explicitly populate `fetch_allowlist` (so it can't
  surprise-bypass an egress block; the allowlist is the opt-in). A non-empty
  `fetch_allowlist` (host globs like `*.rust-lang.org`, `docs.rs`) restricts
  egress; empty = any host when network is enabled. Closes the gap where the
  `researcher` agent couldn't look anything up under the hard-security config.
  New config: `fetch_allowlist`, `fetch_timeout_secs` (20), `fetch_max_bytes`
  (262144); CLI `--fetch-timeout`; env `CATALYST_CODE_FETCH_ALLOWLIST`,
  `CATALYST_CODE_FETCH_TIMEOUT`, `CATALYST_CODE_FETCH_MAX_BYTES`. Also usable
  via `bulk`.
- **`edit`: `replace_all` and `normalize_whitespace`** per edit. `replace_all`
  replaces every occurrence instead of erroring on a non-unique match.
  `normalize_whitespace` matches on whitespace-collapsed text (runs of
  whitespace become a single space) so indentation/spacing drift still lands —
  the replacement edits the real text region, projected back via a char map. On
  a failed search, a closest-line hint (line number + token overlap) lets the
  model self-correct in one shot instead of re-reading the whole file. The
  matching core is refactored into a non-writing `plan_edit` shared by
  `execute_edit` and the new approval preview.
- **`bash`: per-call `timeout`** override (clamped to `[1, max_bash_timeout_secs]`,
  default ceiling 600s) so a slow `cargo build --release`/`npm install`/test
  suite can get more time for one command without raising the global default.
  New config: `max_bash_timeout_secs`; CLI `--max-bash-timeout`; env
  `CATALYST_CODE_MAX_BASH_TIMEOUT`.
- **`bash`: smarter output truncation.** When over the 32 KiB cap, the tail is
  kept AND error/warning lines are salvaged from the dropped head — a compile
  error in the middle of a huge build log no longer vanishes under a pure tail
  truncation.
- **Approval diff preview.** For `write_file`/`edit`/`patch`, the
  `approval_request` event now carries a `diff` field: the unified diff the
  call *would* produce (computed without writing). The TUI renders it under the
  approval banner (and in the history block), so the human approves the actual
  resulting change, not just the raw search/replace blobs. Layout reserves the
  real banner height so the diff never overlaps the viewport.
- **Subagent per-agent depth.** An agent's `maxSubagentDepth` frontmatter now
  actually caps its subtree (it was dead code): `run_single` computes
  `child_max_depth(global, agent.max_subagent_depth)`, so e.g. a constrained
  agent can't fan out deeper than its declared ceiling even when the global
  cap is higher.
- **Token-budget-aware compaction tail.** The verbatim tail kept on compaction
  is now sized by a token budget (25% of the context window, floored at 6k)
  with a 6-message minimum, instead of a fixed 8/10 messages — so a quiet
  stretch isn't over-kept and a huge tool result doesn't eat the whole tail.
- **`session-extract` memory accumulates.** Auto-extracted durable facts now
  APPEND across compactions (with a rolling 16 KiB cap, oldest trimmed) instead
  of overwriting, so early-session facts survive later compactions.

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

### Fixed — per-model reasoning levels from /models/info
- The core's `/models/info` parser looked for reasoning levels under the wrong
  field (`capabilities.thinking_levels` / `reasoning_levels` / `reasoning_efforts`),
  but the endpoint nests them under `capabilities.reasoning.levels`. Every
  live-discovered model therefore got **empty** levels and the TUI fell back to
  the hardcoded `low/medium/high` trio for all models. The parser now reads the
  real `capabilities.reasoning.levels` (with `reasoning.supported` for the flag),
  so each model advertises the efforts it actually accepts — e.g. GLM 5.2
  `none/high/max`, flash & qwen `none/low/medium/high`, kimi/coder `[]`
  (keep the low/med/high fallback). Flat capability fields remain as a fallback
  for other OpenAI-compatible endpoints.
- Bumped the models cache schema to v3 (with a `version` gate on read) so stale
  caches written by the older parser — which stored empty `thinking_levels` and
  wrong `vision` flags — are treated as a miss and refreshed instead of masking
  the fixes for the 8h TTL.
- TUI: the `/reasoning` + `ctrl+r` hints no longer hardcode `(low/med/high)`;
  the picker already renders the selected model's actual advertised levels.

### Fixed — vision capability from /models/info
- The `/models/info` parser read vision from the wrong field
  (`capabilities.vision`), but the endpoint exposes it as
  `capabilities.supports_vision`, encoded as `true` / `false` / `"via-handoff"`.
  Every live model therefore reported `vision=false`, so the vision-handoff
  plugin always handed image turns off even from natively vision-capable models
  (kimi/coder/flash/qwen). The parser now reads `capabilities.supports_vision`;
  only boolean `true` counts as native client-side vision, so `"via-handoff"`
  (GLM 5.2, whose vision only works on `/v1/messages`, which the harness doesn't
  use) maps to `false` and the plugin routes its image turns to a native model.

### Added — macOS standalone executable
- `release-macos.sh` cross-compiles the harness into a single self-contained
  macOS executable per arch (arm64 + x86_64): the Rust core is built with
  `cargo zigbuild` (zig as the macOS linker; pure-Rust `rustls-tls` so no
  macOS SDK is needed), then embedded into the Go TUI via `go:embed`
  (`-tags embed_core`). Each output file runs from any CWD — it extracts its
  bundled core to `~/Library/Caches/catalyst-code` on first run and launches
  the harness in that directory, with no separate `catcode-core` and no install.
- TUI: new `embeddedCorePath()` (build-tagged `embed_core`) is wired into
  `coreBinaryPath()` ahead of the usual `$CATCODE_CORE`/dev/installed search;
  it's a no-op stub in normal builds, so dev, Linux, and the Windows MSI
  layout are unchanged.

### Changed
- Removed the fixed agentic turn cap (`--max-turns` / `max_turns`, default 200)
  and the `spawn` sub-agent turn cap (`spawn_max_turns`, default 10). Turns are
  now bounded only by the session token budget (`--max-session-tokens`, 0 =
  unlimited), the `finish` tool, abort, or the model stopping. Removed: the
  `--max-turns` flag, `CATALYST_CODE_MAX_TURNS` env var, `max_turns` config
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
  `~/.config/catalyst-code/sessions/`.
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
