# Codebase Review — Umans Harness

**Method:** 12 parallel `reviewer` subagents on **`deepseek-v4-flash`** (opencode-go), two batches of ≤8 (the `subagent` tool hard-caps parallel tasks at `parallel_max_tasks` = 8). Scope: Rust core (~22.9k LOC, 19 modules), Go TUI (~12.2k LOC), TS SDK (~5.7k LOC), Next.js web (~6.9k LOC), build/release/CI/Docker.

**Verification note:** the two most surprising Critical findings (main.rs `return`-exits-process, config precedence inversion) were independently confirmed by re-reading the source. `deepseek-v4-flash` line references are accurate on the spot-checks, but line numbers may drift by a few lines — re-read before patching.

---

## 🔴 CRITICAL

### C1. `return;` in command handlers exits the entire core process  *(verified)*
`core/src/main.rs` — `Command::SetProvider` (~:1148), `Command::Login` (~:1188), `Command::Logout` (~:1291), `Command::LoginOauth` (~:1341), `Command::SetConfig` (~:1431).
These arms use bare `return;` inside `#[tokio::main] async fn main`'s command loop to mean "skip this command", but `return` resolves the future → the **whole core process exits**. Any unknown provider name, unknown login preset, or unknown config key kills the engine.
**Fix:** replace each `return;` with `continue;`.

### C2. Config precedence is inverted  *(verified)*
`core/src/config.rs:901–944` — CLI args are parsed into `c` first (~:805), then config files are applied (`apply_json`, ~:936), then env vars (~:942+). Effective precedence is **env > files > CLI** — i.e. CLI is the *weakest*, the opposite of the documented "CLI > env > files". `--base-url` is silently overwritten by `settings.json`; `--no-trust-project-plugins` can be overridden by `UMANS_HARNESS_TRUST_PROJECT_PLUGINS=1`.
**Fix:** parse CLI into a temp struct, apply layers in increasing precedence: defaults → managed.d → managed config → settings.json → settings.local.json → env → CLI (CLI last).

### C3. Agent-management actions allow path traversal → arbitrary write/delete
`core/src/subagent.rs` — `create_agent` (~:2408), `update_agent` (~:2443), `delete_agent` (~:2507) build filesystem paths from the unsanitized `name` argument. `name = "../../../.bashrc"` (or absolute) writes/deletes files outside `.umans-harness/agents`.
**Fix:** restrict `name` to a strict slug (`[A-Za-z0-9_-]`); assert the resolved path stays inside the agents dir.

### C4. Firejail profile emits literal `read-write {ws}` instead of the workspace path
`core/src/tools.rs:~1296` — `s.push_str("read-write {ws}\n");` is not `format!`'d, so the directive is the literal string `{ws}`. The workspace is only writable by luck (the `whitelist` line is correct; the existing test only checks the workspace string *appears*).
**Fix:** `s.push_str(&format!("read-write {ws}\n"));`

### C5. `session::ensure_header` check-then-create race can truncate a live session
`core/src/session.rs:~26–35` — `exists()` then `OpenOptions.create(true).truncate(true)`. Since `run_turn` is an async task, `ListSessions`/`Reset`/`Undo`/`Compact`/`LoadSession`/`NewSession` run concurrently with appends; a second caller can see a brand-new file as absent and `truncate` it, discarding messages. `append`/`rewrite` are also mutually unsynchronized, and long JSON lines exceed `PIPE_BUF` so `write_all` isn't atomic across writers.
**Fix:** `create_new(true)` (atomic), or open-append + write header only when file is empty; add a per-file `tokio::sync::Mutex` or advisory lock.

### C6. Dangerous-path blocklist checks the input string, not the canonical path → symlink bypass
`core/src/workspace.rs:~27` + callers (`tools.rs:~646`, `main.rs:~3126`) — `check_dangerous_path` runs on the raw relative string *before* `resolve`. An in-workspace symlink → `.git/config` or `id_rsa` passes `resolve` (target is "in-workspace") and the write proceeds.
**Fix:** re-run `check_dangerous_path` on the canonical path returned by `resolve`.

### C7. Concurrent `prompt()`/`followUp()` collide on `turnResolver`
`sdk/src/agent-session.ts:~301–319` — when a follow-up is queued while streaming, a fresh `turnResolver` is installed unconditionally; when the current turn finishes it resolves the *new* resolver belonging to the follow-up before that turn runs. Basic prompt lifecycle breaks.
**Fix:** track resolvers per turn identity, or reject a new `prompt()` while one is pending.

### C8. `abort()` leaves the original `prompt()` promise dangling forever
`sdk/src/agent-session.ts:~399–405` — saves `prev = this.turnResolver`, installs a wrapper, then `void prev;` (never resolved/rejected). `await session.prompt()` hangs indefinitely after abort.
**Fix:** resolve/reject `prev` (resolve on clean abort, reject with a cancellation error if callers should see it).

### C9. `pendingLogin` leaks on ESC → commits a settings value as an API key
`tui/modal.go` — `selectProviderItem` (~:526) sets `pendingLogin` + `editing`; `handleSettingsEditKey` close branch (~:638) and `closeModal` (~:203) never clear it; `commitEditField` (~:1058) branches on `pendingLogin != ""` *before* checking the field. After `/login` → pick → Esc → Esc, editing *any* settings field (e.g. Bash Timeout) and pressing Enter sends that value as a key for the leaked preset.
**Fix:** clear `s.pendingLogin` in both the close branch and `closeModal`.

### C10. `save_providers_config` claims 0600 perms but never sets them
`core/src/config.rs:~554–580` — doc says "0600 perms", but `set_permissions` is never called and `provider_to_json` serializes literal `api_key` values. With default umask `022`, `~/.config/umans-harness/config.json` is `0644` — world-readable secrets.
**Fix:** `std::fs::set_permissions(&tmp, Permissions::from_mode(0o600))` (Unix) before rename.

---

## 🟠 HIGH

### Rust core
- **main.rs** — queued follow-ups silently drop attached `images` (`QueuedPrompt` has no images field); **allow-rules override deny-rules** contradicting the documented "deny wins" (~:3046); conversation `Mutex` held across `await`/sync-IO at ~14 append sites + `refresh_memory_injection`; `image_to_data_url` loads arbitrary files with no size cap (OOM); panic path leaves stale queued prompt; TOCTOU lets a new turn spawn during drain.
- **tools.rs** — bash denylist `contains_at_boundary` only treats space/EOF as boundary, so `rm -rf /;echo` / `rm -rf ~&` are **not blocked** (shell metachars bypass); `diagnostics` is classified `ReadOnly` yet runs `npm run build` (executes arbitrary package.json scripts, writes files — **approval-gate bypass**); `grep` `context` is `u64`→`usize` with no clamp → overflow panic/UB; edit/patch/write_file are non-atomic (half-written file on crash); `read_file` TOCTOU size race.
- **provider.rs** — OpenAI streaming path **ignores `max_tokens`** (output uncapped); OpenAI path **ignores `provider.headers`** (gateway/proxy headers dropped, Anthropic path has them); Anthropic streaming `tool_use` seeds `tool_args` with the empty `input: {}` then appends deltas → invalid JSON `{}{...}`; unvalidated streaming `index` grows a `Vec` to a huge `u64` → OOM/panic; per-chunk `from_utf8_lossy` corrupts multi-byte chars split across chunk boundaries; no total turn/stream timeout (slow-drip defeats idle timeout); model cache drops `reasoning`+`provider` and hardcodes `reasoning:true` on reload (non-reasoning models mislabeled).
- **subagent.rs/intercom.rs** — `run_chain` fails to finalize on step-failure/abort → **re-leaks `SubagentRun`** (reintroduces the leak `prune_terminal_runs` fixed); `max_subagent_depth` not inherited across nesting (children can spawn deeper than the parent allowed); `finish`/`contact_supervisor`/`intercom` are injected into the child's definitions but `dispatch_subagent_tool` can reject them (advertised-but-unusable); `intercom reply` resolves any ask by id without verifying the recipient (a child can inject a reply into a peer's/orchestrator's question); `IntercomBus::post` double-lock TOCTOU can drop messages (5-min timeout stall).
- **config.rs/plugins.rs** — documented merge semantics (array concat+dedup, deep-merge, null-delete) are **not implemented** (shallow replace); duplicate provider names from lower-precedence files win (no dedup in file path); **empty `UMANS_HARNESS_TRUST_PROJECT_PLUGINS` enables project plugins** (security footgun — `v.is_empty()` treated as on); plugin `install`/`remove` path traversal via manifest `name` (`"../../../evil"`); `context_compact_at` not configurable from JSON.
- **session/memory/workspace** — memory files **not fsync'd** (durability claim false); `WRITE_LOCK` is process-local only (multi-core/multi-session races drop facts); `memory_injection` loads *all* memory files with no count cap (unbounded system prompt); atomic renames lack directory fsync (POSIX rename durability); `load` silently drops sessions on permission errors; `load` reads the whole file into memory (OOM before compaction).

### Web frontend
- **reducer.ts:57–60, 313–320** — `reduce()` is **impure**: global `counter` + `Date.now()` inside `setState`. React 18/19 Strict Mode double-invokes updaters → duplicate user messages/toasts, key collisions.
- **components/markdown.tsx:54–59** — `a` spreads raw `href` from model output with no sanitization → `[x](javascript:alert(...))` renders a clickable `javascript:` URI (**XSS**).
- **app/api/files/route.ts:44–52 + app/api/command/route.ts:46–53** — path traversal: `normalize` (not `realpath`) + relative `workspace` lets `..`/symlinks read outside the root; `switch_workspace`/`add_project` pass arbitrary `path` straight to `spawn(cwd)`. Any multi-tenant/leaked-token scenario runs the agent in `/etc`, `/home/other`.
- **api/stream/route.ts:51–58** — `enqueue` return value ignored → no SSE backpressure; large tool results balloon server memory when the browser is slow.
- **core-bridge.ts/live-session.ts** — no hard cap on live sessions (idle GC is 2h, active-viewer sessions never reaped) → resource exhaustion; dead sessions linger in the map until idle GC.

### TypeScript SDK
- **agent-session.ts:1193–1207** — approval **defaults to `"yes"` when no UI is bound** (headless consumers auto-approve destructive tools — security footgun).
- **agent-session.ts:1169–1186** — `compacted` updates token counts but never truncates `_messages` → `session.messages` is stale post-compaction.
- **session-manager.ts:331–344 + agent-session.ts:204–206** — `SessionManager.append*` writes PI-style entry-tree JSONL to the **same path** the Rust core uses for OpenAI-style messages → **corrupts the session file** the core resumes from.
- **model-registry.ts:47–48 / agent-session.ts:233,790** — shared global `_setModels()` overwrites on every `ready`/`models`; `_mergeModels()` exists but is unused → multi-provider/multi-session loses discovered models.

### Go TUI
- **handlers.go:390–410** — crash-restart budget (`coreRestarts`) is reset **only** when no turn is queued; a crash during a queued follow-up/steer continuation kills the session immediately, contradicting the "per-incident" intent.
- **handlers.go:662–671** — legacy `APIKey` fallback in `providerKey()` sends the **previous provider's secret** to a new provider after a switch (or on `/logout` if `APIKey` isn't cleared).

### Build / CI / Docker
- **Dockerfile:25–27** — runtime image lacks **`git`** (breaks `git_status`/`git_diff`/`git_log` + git context) and **`python3`** (bundled `vision-handoff` + `telemetry` plugins are `.py` and silently fail).
- **Dockerfile:29→35** — `/workspace` created root-owned, then `USER harness` without `chown` → bash tool can't write the CWD.
- **Dockerfile:25,36,51** — defaults to `UMANS_HARNESS_SANDBOX=firejail` which silently no-ops in a plain `docker run` (needs `--cap-add SYS_ADMIN --security-opt apparmor=unconfined`); users believe bash is firejailed when it isn't.
- **ci.yml** — no `web` or `sdk` jobs (web/SDK breakage ships undetected); the `docker` job builds the image but **never runs it**, so the missing git/python3 above isn't caught.
- **release-macos.sh:84 + Dockerfile:14** — omit `--locked` (reproducibility gap vs Linux/Windows); **build.sh:7** also omits it.

---

## 🟡 MEDIUM (selected — full list in per-component sections below)

**Rust core**
- main.rs: panic path doesn't clear `queued`; `bulk_write` touches lost from `WorkState` (reads `edits` not `files`); real-token baseline counts the transient work-state message; reset/clear/new-session don't reset `estimated_tokens`/`last_model`; `Command::Login` indexes `configs[0]` without length check (panic).
- tools.rs: `bulk` schema says "any built-in tool" but enum is limited; `subagent` schema has no `required`; whitespace-only `search` gives misleading "not found"; `web_search` `region` not URL-encoded; `git_*` use weaker path validation than file tools; `glob` `**` in the middle not supported.
- provider.rs: `openai_complete` has no retry/timeout/headers/max_tokens; `send_with_retry` sends `Authorization: Bearer ` even with empty key; `parse_models_response` defaults `reasoning:true` when capability object missing.
- subagent/intercom: `receive` and `poll` both drain the mailbox (poll isn't a peek); `run_parallel` marks parent "completed" even on child failure; `apply_overrides` only for builtins (not user/project agents); `completionGuard` frontmatter parsed inverted (`== "false"`); `std::sync::Mutex` poison risk in `IntercomBus`; `resume_action` doesn't set state `running`; `find_run_prefix` matches ambiguous prefixes (could target wrong run); runs don't use per-model provider routing; vacuous intercom test assertion (always true).
- session/memory/protocol: `append_memory` cap overshoots `max_bytes`; fixed `.tmp` filenames collide under concurrent writers; `describe` clones the full first message before truncating; `ModelInfo` numeric fields lack `#[serde(default)]`; `Event::with` allows reserved `type`-key collision; `staging.rs` test mutates global `HOME` without isolation.

**Go TUI rendering**
- render.go:441–478 — input soft-wrap **draws text after a newline on the cursor line** (reviewer verified with a temp test: `line1\nline2` renders as `│ line1 line2 │` with a blank row 2).
- render.go:71–82 — double rendering per frame (`layout()` renders components to measure height, `View()` renders again) — O(input length) per keystroke.
- tool_blocks.go:410–435 — git status rename (`R100 old -> new`) parsed as 2-char code → misparsed.
- tool_blocks.go:236–247 — diff body not tinted red on failed edit/patch.
- **`web_search` tool entirely missing** from TUI dispatch/icon/kind/display-name map (defaults to destructive amber).
- styles.go — all width math uses rune counts, not display-cell widths (CJK/emoji overflow).
- blocks.go:115–122 — `tool_result` invalidates the **entire** block cache (O(total history) per event).

**Go TUI modal/settings**
- modal.go:918–922 — settings "API Key" field reads only `s.settings.APIKey`, ignores per-provider keys (shows "(not set)" after `/login`).
- modal.go:1081–1092, handlers.go:322–324 — approval state diverges between `s.settings.Approval` and runtime `s.approvalModeStr`; `/approval <foo>` accepts arbitrary input.
- Non-keybinds modals can **trap focus** if the `close` binding is cleared (only `/keybinds` hardcodes `esc`).
- modal.go:967–992 — settings modal not scrollable → lower fields unreachable on short terminals.
- render.go:85 — header says `? for help` but `?` is unbound.

**Web**
- reducer.ts:456–463 — `history` event clobbers in-flight UI state (active approval/intercom prompt vanishes).
- use-agent.ts:126–163 — session switch can reduce stale old-session events into the new session (events carry no session id; no client guard).
- composer.tsx:150–165 — imperative `setSelectionRange` in rAF races React 19 concurrent render.
- use-agent.ts:176–189 — `set_approval` restore effect fires on mount against the default workspace, not the later-loaded session.

**SDK**
- core-process.ts:202 — `send()` throws on a dead process (`ERR_STREAM_WRITE_AFTER_END`); :258–277 `dispose()` waits 3s for an already-dead process; :80 no ready timeout (init hangs forever); :91 `debugFd` leaks on spawn fail; :161 malformed JSON silently dropped.
- agent-session.ts — `provider_changed` ignored (stale `_provider`); tool turns split a single PI turn into multiple; `steer()` doesn't record the user message; `sendUserMessage()` drops images; `AgentSession.state` casts `model as Model` (may be undefined); **no unit/integration tests** (only smoke.mjs needing a real binary).

**Build**
- release-linux.sh:94–110 — `appimagetool` downloaded from `continuous` tag with no checksum/version pin (supply-chain risk).
- packaging/windows/ucli.wxs:18 — MSI `ProductVersion` can't accept semver prerelease tags (`0.3.0-beta.1` breaks).
- `.intentionally-empty-file.o` is both **tracked and gitignored** (0 bytes — confusing).
- release-all.sh:10 missing `set -e`; macOS install hardcodes `/usr/local/bin`; `.gitignore` misses `tui/tui.exe`; typo `.dgm`→`.dmg`; base images not digest-pinned.

---

## 🟢 LOW (one-liners)

- main.rs: `protocol::emit` swallows stdout errors; `--max-session-tokens` is cumulative cost not current-context (document).
- tools.rs: `memory` classified ReadOnly despite writing files (intentional?); `fetch` host parser doesn't strip userinfo; search fallback includes sponsored results.
- provider.rs: `parse_retry_after` treats past HTTP-date as fallback; OAuth `await_redirect_code` doesn't URL-decode; `google_token_from_adc` may return expired token; `VisionConfig::save` non-atomic.
- subagent.rs: `run_chain` temp dir never cleaned; `bulk` arms unreachable (no `bulk` tool definition).
- TUI: `s.input.Width` set but ignored; `renderGitCommitBlock` doesn't escape quotes; `renderQueueBanner` uses byte length for emoji labels; diagnostics counter is a substring heuristic; several `.go` files need `gofmt` (CI only runs `go vet`).
- SDK: `customTools` accepted but unused; `executeBash` discards stderr, always reports `truncated:false`; `clearQueue` always reports empty arrays.
- web: auth cookie parsing fragile (string compare, no rotation); `cachedEnv` never invalidated.

---

## ✅ What's working well (positives)

- Rust: `run_turn` is `catch_unwind`-wrapped and always clears `current` (no wedged UI); real-token baseline invalidated on every history rewrite; session rewrites use temp+fsync+rename; `subagent_runs` pruned; `grounded_estimate` clamps stale lengths; JSON deserialization is consistently guarded (no panics on malformed input in the smaller modules).
- TUI: `/key` restored; Enter-selects-even-when-`select`-unbound fallback fixed + tested; core binary resolution + backpressure + restart-once correct; keymap plumbing + modal overflow math solid.
- Web: per-session model fixed the old single-core leak; tool-call/result ordering stable; no `dangerouslySetInnerHTML`; argv (not shell-string) spawning; `intercom_reply` id field now correct.
- Build: no secrets in workflows; `permissions: contents: read`; good cross-compile target choices (static rustls, cargo-zigbuild); release scripts clean up injected cores via `trap`.

---

## 🎯 Recommended fix priority

1. **C1** (`return`→`continue`) — one-line fixes, highest blast radius (process exits on benign input).
2. **C2** (config precedence) — re-layer CLI last; security-relevant (`--no-trust-project-plugins`).
3. **C3 / C6** (path traversal: agent `name` + dangerous-path symlink bypass) — security.
4. **C7 / C8** (SDK `turnResolver` collision + dangling abort promise) — breaks the basic prompt lifecycle.
5. **C9 / C10** (TUI `pendingLogin` leak + world-readable provider config) — secret leak + state corruption.
6. **High-security set**: bash denylist metachar boundary; `diagnostics` `npm run build` misclassification; web markdown XSS + `/api/files`+`switch_workspace` realpath; SDK approval default `"no"`; SDK session-file ownership split.
7. **High-correctness set**: provider OpenAI `max_tokens`+`provider.headers`+Anthropic tool_use JSON; `run_chain` finalization leak; reducer purity; Dockerfile git/python3/chown.
8. **Build/CI**: add `web`+`sdk` CI jobs + Docker smoke test; `--locked` everywhere; pin/checksum `appimagetool`.

> Full per-component detail with all file:line references is in the raw subagent outputs; this document is the deduplicated, prioritized synthesis.
