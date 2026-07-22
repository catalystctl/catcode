# Changelog

All notable changes to **Catalyst Code** (formerly Umans Harness), day by day from first commit.

## 2026-07-22

- **feat(grep): enrich grep tool with -v/-F/-w/-A/-B, glob negation, multi-file paths** [be0174c]
  A session audit (661 sessions) showed the agent reached for bash grep/rg 1.75×
  more than the native grep tool — driven by missing flags (-v/-F/-w/-A), the
  `| head` pagination habit, and outside-workspace paths. Added `invert` (-v;
  content/count emit non-matching lines, files_with_matches+invert = grep -L),
  `fixed_string` (-F; literal match via regex::escape), `word` (-w), and
  `after`/`before` (-A/-B, merged with `context`/`-C`) to both the rg path and
  the pure-Rust fallback. `glob` now accepts a string or array with `!`-prefixed
  exclusions (rg --glob semantics, mirrored in pure-Rust via glob_filter_passes),
  and a new `paths[]` param searches a specific set of files/dirs (skipping the
  rg fast-path for multiple roots). Schema description rewritten to surface that
  output already includes line numbers and covers -n/-l/-c/-i/-C, nudging off
  `| head`/`grep -n`. Workspace confinement unchanged. 21/21 grep tests pass.

- **docs(skills): add audit-tool-usage-from-sessions skill** [eef7dee]
  Reusable workflow for diagnosing why the agent prefers bash over a native tool
  by auditing session tool_use logs — parse assistant tool_calls, classify bash
  search invocations by reason (PIPE/OUTSIDE_WS/FLAG_V/…), then fix real gaps in
  the native tool and surface already-supported features in the schema.

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

- **feat(install): adjustable web-service envs + CORS expose/origin/trusted-origins** [9631a7a]
  install.sh and install.ps1 now let every web-service env be set at install time
  (flag + interactive prompt + persisted in installer.state + stamped into the
  service unit), aligned 1:1 across systemd/launchd and NSSM/scheduled-task. Adds
  `--expose local|intranet|public` (default intranet; auto-detects the LAN IP)
  and `--origin` (canonical origin → CATCODE_WEB_ORIGIN, drives better-auth
  baseURL/trustedOrigins/passkey rpID), plus `--trusted-origins` for multi-domain
  proxies — fixing the gap where the default 0.0.0.0 bind exposed the panel but
  better-auth CSRF-rejected every non-loopback authenticated POST. `--port`/`--host`
  now survive `--update` (were clobbered by state).
- **fix(ci): unblock master CI — track check scripts + isolate protocol-harness config** [7b75f03]
  Two regressions from the protocol-v2/CI batch surfaced only once the self-hosted
  runner came back online: `web/scripts/check-{protocol-schema,architecture}.mjs`
  were gitignored under the scripts scratch policy (→ MODULE_NOT_FOUND in CI), so
  force-track both; and `core/tests/protocol_harness.rs` only passed on hosts with
  an existing ~/.config/catalyst-code (core's first-run staged a default provider
  shadowing the test mock in CI's clean HOME), so inject the mock as an explicit
  UMANS_PROVIDERS provider.
- **feat(tui): working-wave busy indicator above the composer** [258c4dd]
  render.go draws a full-width animated sparkline pulse (two traveling sines +
  amplitude breath) directly above the input box while busy, chrome-cached per
  View and wired into relayoutHeights(), with a reduced-motion static fallback.

## 2026-07-20

- **protocol: v2 wire schema, handshake, and event catalog across the stack** [dc611ce, 11b2f1a, 8d47fe8, 473f3bd]
  Introduces a versioned JSONL stdio protocol: a checked-in JSON Schema
  (protocol.schema.json) + canonical fixtures under protocol/, a v2 init
  handshake (protocol_version: 2 + client name/version + capabilities
  run_ids/session_ids/event_sequence), and new lifecycle events
  (approval_expired, run_cancelled, runtime_status, session_recovered,
  worktree_seeded, tool_result.status). Adopted in the SDK (typed catalog +
  CoreProcess handshake + bun:test fixture suite), the web reducer (toasts /
  dismiss / terminal semantics), and the architecture docs.
- **refactor(core): split monolithic modules into focused subdirectories** [797b2da]
  Breaks the oversized main.rs/provider.rs/tools.rs/protocol.rs into a modular
  tree — agent/, commands/, providers/, protocol/, runtime/ (coordinator +
  event_sink + lifecycle + resources + run), tooling/ (approval/execution/
  scheduler/schema/policy/builtin), and memory_eval.rs. Behavior unchanged;
  main.rs shrinks to wiring + State, the rest re-export from their new submodules.
- **tui: visual redesign with derived surface tones and composer card** [4e0841c]
  Reworks the transcript chrome around a Catalyst-style depth language: surface /
  sunken / rail / soft-fill tones derived from each theme's authored palette so
  light + dark stay self-consistent; user turns render as right-aligned surface
  bubbles, assistant turns as flush prose under a quiet model tag with a thin
  accent rail; rounded composer card with an inline ❯ prompt and solid block
  cursor. Adds type-to-filter (no leading "/") for the picker.
- **ci: release smoke + protocol/architecture/SDK checks** [9bcc3d6]
  Core job builds a release artifact and smoke-tests --version/--help; web job runs
  check-protocol-schema.mjs (Rust vs JSON Schema vs SDK catalog vs fixtures) and
  check-architecture.mjs (no-growth spawn/megamodule enforcement) before the build;
  adds SDK install/typecheck/test steps.
- **skills: codebase-quality-opinion + tui-visual-iteration** [88b8fe4]
  Adds two opt-in harness skills: a structured "is this a good codebase?" verdict
  workflow, and a workflow for visually iterating on the Go TUI by rendering and
  screenshotting realistic frames.
- **chore(install-web): drop trailing newline** [f5be59b]
  Normalize the installer script to end without a trailing newline for a clean
  curl-piped invocation.

## 2026-07-18

- **feat(tui): application-managed mouse selection + protocol v2 handshake** [129c711]
  Cell-based drag selection over the transcript (painted on the cropped viewport
  so drag cost scales with terminal height; coalesced to ~30fps) and over the
  modal overlay canvas. Mouse tracking is now always on (cell-motion) since
  selection is app-managed, so drag-to-copy works without disabling tracking;
  /mouse-wheel becomes a backward-compat no-op. The init handshake now sends
  protocol_version: 2 + client capabilities, with a Go fixture compat test. Paste
  inserts at the cursor instead of always appending.

## 2026-07-17

- **feat: Charm TUI polish, preview SPA proxy, parallel releases** [4132156]
  TUI adopts glamour/huh/filepicker and refines ask/modals/markdown; the web proxy
  scopes cookies and patches history/location/WS for SPAs; core requires ask when
  underspecified and simplifies failure_atlas; CI/release moves to matrix builds
  with concurrency cancel + bun pin and caches.
- **web: ship one cross-platform bundle via zigpty** [1d6a331]
  Replace @lydell/node-pty with zigpty so the release keeps a real PTY while
  shipping a single catcode-web-<ver>.tar.gz; collapse per-OS web CI jobs and
  point install.sh / install.ps1 at the universal tarball.
- **tui: adopt more Charm bubbles and polish goal UX** [f20649c]
  Replace the composer textinput hack with textarea; use progress/help/key for
  meters and hints; swap the spinner for an explicit busy tick; improve the goal
  progress panel.
- **web: control center and richer goal/reducer state** [7f96160]
  Wire a control-center UI, expand reducer/types for the goal lifecycle, and
  simplify native-pty packaging for the Next web app.
- **sdk: expose goal iteration and verdict events** [43b95e8]
  Add typed goal_iteration, review/verify verdict, and certified events so web and
  clients can track goal certification progress.
- **core: deepen goal orchestration and learning retrieval** [2d7bf44]
  Extend the goal lifecycle / CEO planning, strengthen learning/memory retrieval,
  and harden the browser/plugin surfaces that feed autonomous runs.
- **packaging: harden installers and release web workflow** [49266c1]
  Improve Linux/Windows install paths and keep release-web/update-web aligned
  with the Windows web-bundle packaging flow.
- **chore(skills): expand agent skills and authoring guidance** [c00dc7b]
  Add a documentation-factory and related skills; tighten reviewer/worker agent
  prompts and plugin-authoring skill coverage.
- **docs: user guides, TUI performance notes, README ToC → docs index** [b962e5b, c386114]
  Publish the docs site structure (install, architecture, tools, plugins) and
  capture TUI render-path findings for later perf work; link the README table of
  contents to the full docs index.

## 2026-07-16

- **fix(windows): installer + web-bundle hardening** [7f34874, 81b6f02, c6af9a3, 8595b48, eac639f, eb9f1de, 48ba28a]
  A batch of Windows reliability fixes: make install.ps1 ASCII-safe for the
  legacy PowerShell 5.1 parser (UTF-8 em-dashes/box-drawing broke it under
  irm|iex); keep the PowerShell window open on installer errors (throw instead
  of exit 1); tolerate a missing schtasks on first web install (wrap native
  probes); rebuild native modules (better-sqlite3/node-pty) after extracting the
  Linux-built bundle so Windows stops hitting "not a valid Win32 application";
  build and prefer a Windows-specific web bundle (catcode-web-<ver>-windows.tar.gz);
  sanitize WindowsApps out of PATH before the Windows web-bundle build (Next
  tracing hit EACCES on ActionsMcpHost.exe); unify the Windows web install into
  the root install.ps1 (-WithWeb/-Update).
- **core: tool/provider security hardening** [370661a, adf342c, 2e42adf, ea28e10, 6e36cf9]
  Route writes through unique-temp atomic_write_str, bound rg/grep, match the bash
  regex denylist on normalized input, isolate sandbox profile names by network
  mode, and truncate large diagnostics/git output; treat OpenAI SSE error frames
  after HTTP 200 as provider failures; close a fetch SSRF via userinfo stripping
  (foo@169.254… bypass) and treat 100.64.0.0/10 (CGNAT) as blocked; skip dotenv
  secrets and .runtime when copying plugins; restore plugin providers before
  startup discovery.
- **core: codebase intelligence + native browser tools** [dd3f156]
  Introduce project-scoped learning (index, episodes, knowledge tool, memory
  status) and optional WRY native-browser deferred tools behind native-browser.
- **core: goal lifecycle — validate after final wave, seed worktrees post-promote** [c18be20, 44f78ec]
  Defer Running until a concurrency slot is acquired and run plan validation once
  at the terminal wave; mirror uncommitted main→worktree (including deletions) so
  dependent goal waves see prior promotions.
- **goal progress across the stack** [924a127, b46d0c7, 11da923, 838440e, c0df354]
  Emit lasting goal step/completion signals, map finish to a human-readable tool
  result, harden subagent summaries, and preserve live Cursor model fields (core);
  type goal_step_complete / goal_completion_summary events (sdk); persist goal
  progress in the transcript and render finish tool blocks (tui); render durable
  goal cards + work-state panels and tighten IDE chrome (web); dedupe goal
  complete and show plan_ready progress (tui).
- **web: file explorer, Monaco DiffEditor, in-app self-update, docked chat polish** [c426a7a, 06cb75f, 3bba703, a20d738]
  Polish the file explorer and editor dirty tracking; open git diffs in Monaco
  DiffEditor tabs; offer in-app CLI + frontend self-update from About; tighten
  the docked chat empty state and composer layout.
- **tui: refresh core + web companions via catcode --update** [be505e9]
- **packaging: Fedora install-test image for clean-env install.sh** [9d7648f]
- **ci: audit-path + digest_to_budget test fix, rustfmt/gofmt + gate UnixStream for Windows** [ebd8f6b, 1df5401]
- **docs: Cursor-routed models via catcode-cursor-provider** [602c9eb]
  Document installing the loopback Cursor SDK plugin and how Catalyst keeps
  ownership of tools, approvals, and workspace confinement.

## 2026-07-15

- **core: audit/checkpoint/embed/worktree modules + goal-deploy wrap-up** [dbb9a49]
  New modules — append-only security audit sidecar (tool decisions, arg/diff
  hashes); hybrid fs checkpoints for undo/rewind (git stash + ref, or non-git copy
  under .catalyst-code/checkpoints/); local hashing-sketch recall for memory
  retrieval; git worktree helpers for parallel subagent isolation. Goal mode
  finalizes the post-deploy synthesizing turn on every exit path so a fast deploy
  can't race the planning turn's drain; adds deploy/running/synthesizing busy-state
  and concurrency profiles. Also: auto-checkpoint before the first destructive
  mutation in a turn; speculative tool-cache warming; vision plugin handoff.
- **web: IDE shell polish, session management, and core-event reducer** [aaed8ea]
  File-tree drag-and-drop docking, breadcrumbs, command palette, panel headers;
  agent file_change / worktree promote refreshes the explorer; pin/archive/group
  sessions with layout prefs + terminal metadata persisted to localStorage;
  authorized-workspace file API routes; reducer handles the full SDK core-event
  catalog (compile-time anchored to CoreEventType).
- **sdk: expose raw core event catalog via core_event passthrough** [e2c3f74]
  Typed catalog of every harness JSONL event kind; re-emit every raw event as
  { type: "core_event", event } so all core kinds reach consumers, not just the
  canonical PI-mapped set; add subscribeCore / CoreEventListener API.
- **tui: goal-deploy/checkpoint/worktree events + concurrency profiles + keyless login** [7634444]
  Handle protocol_hello/file_change/checkpoint/worktree/audit/cost_update events
  quietly; goalKeepsBusy spans deploy/running/synthesizing; goal concurrency
  profiles scale worker count + model by concurrency; a plugin/local preset with
  an empty EnvVar commits a keyless login on blank-key Enter.
- **build: web release bundling + one-command web installer** [719133f]
  release-web.sh recursively copies each external runtime package + its full
  dependency closure into the standalone stage (dereferences the workspace SDK
  symlink); install-web.sh is a one-command curl|bash installer forwarding to
  install.sh --with-web; install.ps1 gains Windows web parity.
- **web: preview proxy, screen panel, and IDE git/settings polish** [5b8e315]
  Add core test_env tooling with Linux/Windows VM image packaging, provider
  onboarding in the standing prompt, and related agent skills.
- **docs: README refresh, vision-handoff plugin, add-key-provider skill** [352aef8]

## 2026-07-14

- **web: VSCode-class IDE shell (4 panels + copilot chat), Monaco loader, project switcher** [557098f]
  IdeShell with file explorer/editor, terminal, git, and preview panels; useIde()
  hook (localStorage-persisted, separate from AgentState); lazy multi-model Monaco
  loader; project switcher (list + create + activate); markdown viewer; preview
  (localStorage-saved HTML + iframe srcdoc); CSP + CORS tuned for IDE routes.
- **TUI visuals + web IDE for mobile and git workflows** [6ade8e2]
  Harden TUI rendering/themes with golden tests; add a responsive IDE shell, git
  API, terminal/PTY setup, and mobile audit support on the web side.
- **Fix sudo prompting + clean up provider plugins** [780c660]

## 2026-07-13

- **feat: plugin-only OAuth, memory tooling, and install scope prompt** [b6d518e]
  Move subscription OAuth out of core into plugins (API-key presets stay); prompt
  global vs workspace on /plugin-install; expand the memory tool surface and
  authoring docs.
- **fix(tui): deadlock on launch when the update cache reports a newer version** [e4f5fbe]
  launchUpdateCheck's fresh-cache path called prog.Send() synchronously on the
  main goroutine before prog.Run() — in Bubble Tea v2 Send blocks until the event
  loop drains, but the loop hadn't started, so it deadlocked. Wrap the fresh-cache
  Send in a goroutine so main() reaches prog.Run().
- **fix(update): auto-escalate with sudo for root-owned install dirs** [d4ce1bb]
  `catcode --update` failed mid-download with "permission denied" when the binary
  lives in a root-owned dir like /usr/local/bin. runUpdate() now probes writability
  before touching the network and re-execs under sudo (passing through the
  original args) so the privileged child does the real download + verify + replace.
- **fix(goal): remove fail-fast so deploy runs all plan waves** [d49e25f]
  Two bugs stopped /goal deploys after the first wave: deploy_goal fail-fast
  aborted the whole plan on any failed step (now tracks failed_steps, skips
  dependents transitively, still runs independent later waves); and a
  contact_supervisor 5-min timeout during goal deploy (no active leader turn)
  blocked then returned "do NOT proceed" — now short-circuits when goal.phase ==
  Running with "proceed with best judgment".
- **Allow keyless local provider login** [7f89a22]
- **feat(tui): harden UX and session workflows** [8aa7bb7]
- **web: IDE panels + resilient search** [f8f1d48]
- **build/ci: provision Node for Next/SDK builds, fix local SDK link + CI formatting** [a28e62e, 4f91f74, ea8ff9d, d0c97f5, 279062f]

## 2026-07-12

- **feat: plugin commands/reload, local presets, sandbox, and learning telemetry** [a85e76a]
- **chore: ignore Windows TUI build artifact** [84a0318]

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
