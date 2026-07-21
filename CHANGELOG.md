# Changelog

All notable changes to **Catalyst Code** (formerly Umans Harness), day by day from first commit.

## 2026-07-21

- **feat(packaging): add Homebrew tap (cask + formula) with release automation** [1cf1193]
  Catalyst Code is now installable via Homebrew from a `catalystctl/homebrew-catcode`
  tap. Ships both an arch-aware (arm64/x86_64) cask (installs the per-arch `.dmg`
  via the `binary` stanza) and a formula (raw prebuilt binary) for the standalone
  `catcode` CLI; the optional web frontend is wired via `caveats` pointing at
  `install-web.sh` (it needs Node + a launchd service, so it isn't a cask/formula).
  Placeholder templates live at `packaging/homebrew/{Casks,Formula}/`; a
  `homebrew-tap.yml` workflow renders them with the real version + per-arch sha256
  on each `v*` release and pushes to the tap via a `HOMEBREW_TAP_TOKEN` secret
  (gated to `v*` tags — SHA versions aren't monotonic, so `brew upgrade` can't
  rank them). Also fixes `release.yml` so `v*` tags name artifacts with the semver
  instead of the commit SHA — was breaking `install.sh` on tagged releases and
  brew version↔url consistency. One-time setup: create the empty public
  `catalystctl/homebrew-catcode` repo and add the `HOMEBREW_TAP_TOKEN` secret.

## 2026-07-12

- **feat(core): compaction reliability + deferred tool-schema staging** [6a48bd4]
  Complete overhaul of the compaction system: pre-digests the middle of the
  conversation, truncates oversized tool payloads, map-reduces chunks over
  100k chars, and merges summary + durable facts into one model call (max 3072
  tokens out). Soft digest uses a token-budgeted keep window (20% of context),
  digests oversized `write_file`/`edit` call arguments, and budget-reclaims
  anything still over 50% of the window. Post-compact target lowered to 35%.
  Tool schemas are now staged — core tools always present, deferred tools
  (`git_*`, `fetch`, `web_search`, `bulk_*`, `diagnostics`, `spawn`) load via
  `load_tools`. Ingress prefers smart-truncation (24KB) over opaque digests;
  cache restores capped (16KB); identical re-reads dedupe.
- **skill: document the review-uncommitted-changes workflow** [947aaae]

## 2026-07-11

- **feat: remove /key command, consolidate OAuth to app-owned store, fix review findings** [3e54128]
  The `/key` convenience command is removed — `/login` already covers API-key
  paste, key override, and OAuth; `/logout` clears credentials. OAuth tokens
  moved to the app's own config directory, not per-workspace files. Auth error
  / first-run copy now points at `/login`. The `set_key` protocol remains for
  SDK/web/reauth.

## 2026-07-10

- **feat: GitHub plugin installs, tool cache, models.dev, and model picker** [8f5d083]
  Plugins can now be installed directly from GitHub repositories. Tool cache
  for improved performance, `models.dev` API for model metadata, and a proper
  model picker in the TUI.
- **feat: PI-compatible bang bash, plugin memory_provider, install scopes, CLI precedence fix** [1c08256]
  `!command` / `!!command` support in the TUI and web composer for running
  bash inline. Core `user_bash` command with the same sandbox/denylist as the
  agent `bash` tool. Plugins can now provide `memory_provider` hooks (e.g.
  SQLite-backed memory). Install scopes added. CLI argument precedence fixed.
- **fix(web): mkdir auth config dir before opening SQLite** [77b9000]
- **fix(install): preserve commit-SHA release tags on Windows** [e3365a3]
- **style: rustfmt + gofmt to unblock CI** [e801999]
- **docs(skill): document hunk-level staging for entangled concurrent edits** [78004d4]

## 2026-07-09

- **feat: Add /goal mode for plan-then-deploy subagent orchestration** [2480b53]
  Core goal protocol with full phase machine: planning → plan_ready →
  deploying → running → done|failed. Planning turns use `goal_write_plan` to
  submit structured plans; core materializes per-subagent prompts and deploys
  under user concurrency and model/provider allowlists. TUI has multi-field
  `/goal` modal with plan review/approve/revise. Web has matching Goal modal
  and status chip. Advanced mode pins models for planner/worker/reviewer.
- **feat: Add multi-provider catalog, Antigravity OAuth, web auth, installer menu** [0e60400]
  Multi-provider catalog system for defining providers in config.json. New
  OAuth provider for Antigravity. Web auth support added. Installer menu for
  cross-platform setup.
- **feat: Add sudo passthrough — intercept sudo commands as blocking password flyout** [52be092]
  When the model runs `sudo`, the TUI intercepts it as a blocking password
  flyout �� the password is forwarded to the command without leaking into the
  conversation history.
- **fix(core): close critical/high security + long-chat durability gaps** [ed7c7c0]
  Security audit fixes: critical and high-severity gaps closed. Long-chat
  durability improvements for conversations that span many turns.
- **tui: replace settings field editor with dedicated modals** [6613273]
  Settings UI overhauled: each setting category now has its own modal instead
  of a shared field editor.
- **tui: migrate to Bubble Tea v2 (Go 1.25, declarative View, KeyPressMsg)** [f3e5dc1]
  Major TUI framework upgrade to Bubble Tea v2, leveraging Go 1.25's
  declarative View pattern and typed `KeyPressMsg`.
- **core: plugins can declare OAuth providers; auto-reflect fires before summary** [714cbd4]
  Plugins can now declare OAuth providers the harness can use for auth flows.
  Auto-reflect reordered to fire before the model's completion summary, so the
  summary is the last thing the user reads.
- **fix(tui): support modified enter in terminals** [ddabd5d]
  Shift+Enter now inserts a newline reliably in terminals with enhanced
  keyboard reporting (including Konsole over SSH). Ctrl+Enter steers instead
  of queueing a follow-up.
- **skill: note concurrent-session staging isolation** [8480161]

## 2026-07-08

- **feat(core): plugin feature parity + context management UX** [3c34a1b]
  Plugin system brought to feature parity: approvals hardened, intercom
  interactions, diagnostics tool scheduling, plugin lifecycle management.
- **core: cross-session workspace presence + awareness** [d105507]
  Core now tracks concurrent sessions in the same workspace, surfacing presence
  info to avoid conflicts.
- **fix: production-readiness hardening + lifecycle/SSRF fixes** [3383c6f]
  Production hardening pass: server-side request forgery (SSRF) protections,
  lifecycle edge cases addressed, and reliability improvements.
- **fix(core): cross-process-safe file writes via fsutil module** [cfcc07d]
  New `fsutil` module implements atomic writes with unique temp names,
  preventing cross-process file corruption that the old in-process mutex
  couldn't protect against.
- **docs: rewrite README + add top-level Windows installer** [b6b19bd]
- **chore: git-commit-all skill prefers exact CI gates** [5da7ce2]
- **ci: merge self-hosted Podman runners, fix flaky core test + remove docker CI** [2da1723, 1467d8b]
  CI migrated from Docker to self-hosted Podman runners. Flaky core test fixed
  by using unique temp dirs per call. Docker CI removed.
- **tui: add catcode self-updater (launch-time banner + catcode --update)** [4a0e52a]
  Built-in self-updater: launch-time update check with 6h cache TTL, and
  `catcode --update` CLI flag. Cache-backed, non-blocking, silent on failure.
- **fix(tui): wire up the ask tool flyout (was entirely dead code)** [2c92b9c]
  The `ask` tool's TUI flyout was never connected — models could call `ask`
  but the user never saw the question. Now it wires through properly.
- **docs(skill): add 'watch CI to green' step to git-commit-all** [1a0228e]

## 2026-07-07

- **web: adopt Catalyst (Obsidian) design system + add Linux installer** [67223cd]
  Major visual overhaul adopting the Catalyst/Obsidian design system. Linux
  installer added for the TUI binary.
- **Rename: Umans Harness → Catalyst Code** [8372ba8, 1b4703a]
  Full tree rename from `umans-harness` to `catalyst-code`. All paths,
  binaries, env vars, and docs updated. The "Obsidian" design system becomes
  the default visual identity.
- **cross-platform install: fix broken install.sh, add macOS launchd + Windows web service** [a7ff434]
  Cross-platform install script fixed. macOS launchd plist for auto-start.
  Windows web service for background operation.
- **core: re-present interrupted ask questions after a core restart** [728a8c4]
  If the core restarts while an `ask` question is pending, the question is
  re-presented so the user doesn't lose the prompt.
- **tui: keep command label visible when a list-row description overflows** [1fe4607]
- **feat(core): plugins can declare custom tools at runtime (no MCP)** [270b082]
  Major plugin capability: plugins can add arbitrary custom tools via a `tools`
  hook at runtime, no MCP server needed. Tools get full schema, dispatch,
  approval, and result handling.
- **feat(install): download prebuilt binaries + commit-SHA releases (no compile)** [6fafe02]
  Installer now downloads prebuilt binaries from GitHub Releases instead of
  compiling from source. Commit-SHA release tags allow pinning to any commit.
- **docs: README overhaul** [9e0b700, 98b3311, 7e2e0ef, 9fecd6b, 1be7ae0]
  Skills published: `publish-to-new-github-repo`, `git-commit-all` with
  conditional push and workflow reporting. README refactored to lead with
  recommended install scripts per platform, added logo and structured sections.
- **ci: Dockerfile fixes + self-hosted Podman runner setup** [2f51b11, 020091f, 4b5f3aa, 3d4a791, 65e8be5, c3da32c]
  Dockerfile parse errors fixed. `FROM golang:1.24-slim` changed to
  `golang:1.24-bookworm` (no `-slim` tag). `cargo build --locked` for
  deterministic builds. AppImage/MSI made optional so releases publish
  without `wixl`/`appimagetool`. Self-hosted Podman runners deployed.

## 2026-07-06

- **prep for first GitHub push: complete .gitignore, untrack scratch artifacts, add MIT license** [e1c8c68]
  Repository prepared for public push: comprehensive `.gitignore` that tracks
  only source + shipped agent/plugin/skill definitions. MIT license added.
  All scratch artifacts untracked.
- **web: OAuth manual-login banner + live work-state panel; add add-key-provider skill** [f897f75]
  Web UI gets an OAuth manual-login banner for providers that need browser
  flow. Live work-state panel shows what the model is doing. New
  `add-key-provider` skill for the harness.
- **feat(memory): add global cross-codebase memory scope** [de9e0b2]
  Memory system extended with a `global` scope — facts persist across
  different workspace boundaries.
- **Add live Umans concurrency (used/limit) to footer** [844897b]
  TUI footer now shows live API concurrency usage (used/limit) from the Umans
  backend.
- **feat: ask tool, subagent peek/steer, restricted-path approval-gating, intercom empty-enter fix** [565a39d]
  `ask` tool for interactive model→user questions. Subagent peek/steer for
  inspecting and redirecting child agents mid-flight. Restricted-path approval
  gating for `.git/**`, `.ssh/**`, `.env*`, etc. Intercom empty-enter crash
  fixed.
- **fix(long-term-usage): plug leaks, races, deadlock, SSRF, corruption paths** [80815f3]
  Comprehensive reliability audit for long-running sessions: memory leaks,
  race conditions, deadlocks, SSRF vectors, and file corruption paths all
  addressed.
- **web: inflight composer ring, ambient work-state, code-copy fix, run pruning** [bdc640a]
  Web UI improvements: spinning indicator while composer is in-flight, ambient
  work-state that stays visible, code-copy button fixed, old runs pruned.

## 2026-07-05

- **feat: multi-provider OAuth, TUI keybinds, auto-reflect, subagent observability, and reviewer agents** [ba39dc8]
  Multi-provider OAuth support for Anthropic and Google. New TUI keybindings
  for power users. Auto-reflect system that learns from each conversation turn.
  Subagent observability: peek into what child agents are doing. Reviewer
  agents for code review workflows.
- **Typed Message, Claude OAuth, Gemini Code Assist API, always-run sanitizer, TPS accuracy** [60bd9a1]
  Typed message protocol for structured communication. Claude OAuth matches
  the official `claude-code` CLI byte-for-byte. Gemini Code Assist API OAuth
  matches `gemini-cli` flow. Always-run sanitizer for tool outputs. TPS
  metric fixed: now divides by generation time (not wall clock), excluding
  tool-call waits and prefill.

## 2026-07-02

- **Add KV-cache-aware rolling work-state, persisted session stats, /stats overhaul, and TUI polish** [5cb2947]
  Work-state now rolling with KV-cache awareness (older states evicted by
  token budget). Session stats persisted to disk and survive restarts. `/stats`
  command overhauled with richer data. TUI polish pass.
- **feat: add web_search tool + multi-provider /login & /logout** [77ec46e]
  `web_search` tool for looking up current information. Unified `/login` and
  `/logout` commands that work across all providers (API key and OAuth).

## 2026-07-01

- **Fix CRLF frontmatter parsing and chain run cancel scoping** [95816e1]
  Chain runs (multi-step subagent sequences) now scope cancellation correctly.
  Frontmatter parsing handles Windows CRLF line endings.
- **Add Next.js web frontend (SSE bridge to core)** [b9a5022]
  First public web frontend: Next.js app that connects to the core via
  Server-Sent Events. Real-time chat, tool rendering, and session management.
- **Add self-learning system and web frontend enhancements** [4560c4d]
  Self-learning system that extracts durable facts from conversations and
  persists them for future sessions. Web UI enhancements for the learning
  features.
- **Redesign TUI tool-call rendering into per-tool dispatchers** [0f6a29c]
  TUI tool-call blocks completely redesigned: each tool type gets its own
  dedicated renderer, making the output cleaner and more informative.
- **Add real-usage token anchoring, skills, telemetry, multi-session web bridge** [1a3bf01]
  Token anchoring uses real API usage data instead of estimates. Skills system
  introduces composable capabilities. Telemetry for usage insights.
  Multi-session web bridge lets the web UI manage multiple sessions.

## 2026-06-30

- **Harden approvals, plugins, and intercom; add diagnostics tool** [a3624b4]
  Approval system hardened with better context and controls. Plugin system
  strengthened. Intercom (model↔model communication) made more reliable.
  `diagnostics` tool added for running `cargo check` / `tsc --noEmit` /
  `go build` / `py_compile` to type-check work.
- **Add memory/git tools, diff view, per-model reasoning; cross-platform installers** [a34fcbe]
  `memory` tool for persistent durable facts. `git_*` tool suite for version
  control operations (status, diff, log, add, commit). Unified diff preview
  in approval requests. Per-model reasoning levels read from `/models/info`.
  Cross-platform installer builds (Windows MSI, Linux AppImage, macOS DMG).
- **Add fetch tool, edit/bash/approval improvements, and TypeScript SDK** [1055559]
  `fetch` tool: native HTTP GET with HTML stripping. `edit`: `replace_all`
  and `normalize_whitespace` options. `bash`: per-call timeout override.
  Approval diff preview shows the exact changes before approval. TypeScript
  SDK (`@catalyst-code/sdk`) published with full tool set.

## 2026-06-29 — Initial commit

- **Initial commit: umans-harness v0.2.0** [7e95016]
  First public release of the assistant coding harness (named "umans-harness"
  at the time). Core agent loop with multi-turn conversations, tool execution,
  and model provider integration.
- **Add subagents & intercom system; drop fixed max-turns cap** [3e9b8be]
  Subagent system: agents can spawn child agents with fresh contexts for
  parallel or sequential work. Intercom system for model↔model communication.
  Fixed `--max-turns` cap removed — sessions bounded only by token budget.
- **Add macOS standalone & Windows installer builds, vision handoff, image attachments; fix TPS metric** [9860270]
  macOS standalone executable (Rust core embedded in Go TUI via `go:embed`).
  Windows MSI installer. Vision handoff: image-capable models handle vision
  requests, and a plugin routes to them when the active model isn't
  vision-capable. TPS metric accuracy fixed.
- **Fix plugin hook dispatch, add subagents/plugins/skills, and CI workflow** [8083e55]
  Plugin hook dispatch fixed: pre-execution hooks (`pre_write`/`pre_bash`)
  no longer run twice per call. Hook `modify` now merges args per-key instead
  of replacing them. Subagents, plugins, and skills directories established.
  CI workflow with build + test matrix.
