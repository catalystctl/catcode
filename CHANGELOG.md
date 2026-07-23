# Changelog

All notable changes to **Catalyst Code** (formerly Umans Harness), day by day from first commit.

## 2026-07-23

- Added web frontend + notification audit reports; dropped empty tui embed placeholder; clarified git-commit-all changelog step. [88f0351]
- Refreshed web visual theme (accent-driven glow/elevation shadows, tighter radii, mono labels) and reworked the work-state bar. [11912f6]
- Required Node 22.13+ runtime guard in web scripts + e2e/regression runners + TESTING.md. [ee42e45]
- IDE shell: terminal reattach (reattach-only "missing"), persisted-layout sanitization/repair, editor serialized-save queue, cancellable workspace switch. [789e4fb]
- Cross-session notifications + live session status feed (bridge-synthesized session_status, header bell + tab badge, desktop-notification opt-in). [f616fbf]
- Simplified README to a concise overview pointing into docs/. [e3066c8]
- Required Node 22.13+ (dropped Bun) in installers; fixed PowerShell "if"-in-expression-position runtime crash. [09e8b1f]
- Skip auto-reflect when the model already delivered its answer (avoids burying it under an empty second finish). [6284a88]
- Removed unused sandbox dead code, fixed exec_stream_with builder, stop microVM on session exit. [eed7a0b]
- Replaced Firejail/Seatbelt/unshare with Microsandbox microVM backend. [c8add8c]

## 2026-07-22

- Added tests for toggling reasoning expand/collapse (ctrl+t) in TUI. [1dffb79]
- Applied rustfmt drift on grep enrichment. [2baa3f4]
- Documented models_override config support + env_passthrough; added diagnose-provider-fallback-models skill. [c4e0d29]
- Fixed ask select option cycling and added block render cache in TUI. [18e41ab]
- Added custom provider modal and sibling-core self-update in TUI. [0e806e3]
- Added custom provider modal with model discovery + per-model overrides in web. [a78a6fc]
- Added OAuth env_passthrough for plugin OAuth scripts. [bb13a89]
- Added custom provider add, model overrides, and model discovery in core. [2de1cb0]
- Added add_custom_provider + discover_provider_models protocol fixtures. [ea0ed70]
- Added audit-tool-usage-from-sessions skill. [eef7dee]
- Added -v/-F/-w/-A/-B flags, glob negation, and multi-file paths to grep tool. [be0174c]

## 2026-07-21

- Encoded standing rule that git-commit-all updates CHANGELOG.md on every commit/push. [debe3c8]
- Added Homebrew tap with cask + formula and release automation. [1cf1193]
- Made all web-service envs adjustable and added CORS expose/origin/trusted-origins. [9631a7a]
- Tracked check scripts and isolated protocol-harness config in CI. [7b75f03]
- Added working-wave busy indicator above the composer in TUI. [258c4dd]

## 2026-07-20

- Dropped trailing newline in install-web.sh. [f5be59b]
- Added codebase-quality-opinion and tui-visual-iteration skills. [88b8fe4]
- Added release smoke, protocol/architecture, and SDK checks in CI. [9bcc3d6]
- Visual redesign with derived surface tones and composer card in TUI. [4e0841c]
- Handled protocol v2 lifecycle events in web. [8d47fe8]
- Adopted protocol v2 handshake and event catalog in SDK. [11b2f1a]
- Documented v2 protocol, runtime coordinator, and hardening plan. [473f3bd]
- Added v2 wire schema and event/command fixtures. [dc611ce]
- Split monolithic core modules into focused subdirectories. [797b2da]

## 2026-07-18

- Added application-managed mouse selection and protocol v2 handshake in TUI. [129c711]

## 2026-07-17

- Charm TUI polish, preview SPA proxy, and parallel releases. [4132156]
- Ship one cross-platform web bundle via zigpty. [1d6a331]
- Linked README table of contents to the full docs index. [c386114]
- Adopted more Charm bubbles and polished goal UX in TUI. [f20649c]
- Added control center and richer goal/reducer state in web. [7f96160]
- Exposed goal iteration and verdict events in SDK. [43b95e8]
- Deepened goal orchestration and learning retrieval in core. [2d7bf44]
- Hardened installers and release web workflow. [49266c1]
- Expanded agent skills and authoring guidance. [c00dc7b]
- Published user guides and TUI performance notes. [b962e5b]

## 2026-07-16

- Sanitize WindowsApps out of PATH before Windows web bundle build in CI. [eb9f1de]
- Build and prefer a Windows-specific web bundle. [eac639f]
- Rebuild native modules after extracting web bundle on Windows. [8595b48]
- Made install.ps1 ASCII-safe for Windows PowerShell 5.1. [7f34874]
- Keep PowerShell open on installer errors in Windows installer. [81b6f02]
- Tolerate missing schtasks on first web install in Windows installer. [c6af9a3]
- Added codebase intelligence and native browser tools. [dd3f156]
- Unified Windows web install into root install.ps1. [48ba28a]
- Fixed audit path and digest_to_budget test expectations in CI. [ebd8f6b]
- Gated UnixStream for Windows builds and applied rustfmt/gofmt. [1df5401]
- Polished file explorer and editor dirty tracking. [c426a7a]
- Open git diffs in Monaco DiffEditor tabs. [06cb75f]
- Offer in-app CLI and frontend self-update from About page. [3bba703]
- Refresh core and web companions with catcode --update in TUI. [be505e9]
- Added Fedora install-test image for clean-environment install.sh validation. [9d7648f]
- Polished docked chat empty state and composer layout in web. [a20d738]
- Deduplicated goal completion and show plan_ready progress in TUI. [c0df354]
- Validate goals after the final wave only. [c18be20]
- Seed worktrees from main after promote so dependent waves see prior changes. [44f78ec]
- Hardened tools for timeouts, denylist, and atomic writes. [370661a]
- Treat OpenAI SSE error frames after HTTP 200 as provider failures. [adf342c]
- Closed fetch SSRF via userinfo stripping and blocked CGNAT range. [2e42adf]
- Restore plugin providers before startup discovery. [6e36cf9]
- Documented Cursor-routed models via catcode-cursor-provider. [602c9eb]
- Added lasting goal cards, work-state, and IDE shell polish in web. [838440e]
- Persisted goal progress and render finish tool blocks in TUI. [11da923]
- Typed goal_step_complete and goal_completion_summary events in SDK. [b46d0c7]
- Skip dotenv secrets and .runtime when copying plugins. [ea28e10]
- Added goal progress events, finish UX, and cursor-bridge metadata. [924a127]

## 2026-07-15

- Added web preview proxy, screen panel, and IDE git/settings polish. [5b8e315]
- Updated README, vision-handoff plugin docs, and add-key-provider skill. [352aef8]
- Added web release bundling and one-command web installer. [719133f]
- Polished web IDE shell, session management, and core-event reducer. [aaed8ea]
- Exposed raw core event catalog via core_event passthrough in the SDK. [e2c3f74]
- Handled goal-deploy/checkpoint/worktree events and added goal concurrency profiles in TUI. [7634444]
- Added audit/checkpoint/embed/worktree modules and wrapped up goal-deploy lifecycle. [dbb9a49]

## 2026-07-14

- Fixed sudo prompting and cleaned up provider plugins. [780c660]
- Shipped a VSCode-class web IDE shell with four panels, Monaco loader, and project switcher. [557098f]
- Improved TUI visuals and expanded the web IDE for mobile and git workflows. [6ade8e2]

## 2026-07-13

- Hardened TUI UX and session workflows. [8aa7bb7]
- Provision Node for web builds. [a28e62e]
- Build Next bundles with Node. [4f91f74]
- Run Next builds with Node. [ea8ff9d]
- Fixed local SDK package link. [d0c97f5]
- Fixed CI formatting and SDK build. [279062f]
- Added web IDE panels and resilient search. [f8f1d48]
- Allowed keyless local provider login. [7f89a22]
- Removed fail-fast in goal deploy so all independent plan waves still run. [d49e25f]
- Auto-escalate catcode --update with sudo for root-owned install directories. [d4ce1bb]
- Moved subscription OAuth into plugins and added memory tooling + install scope prompts. [b6d518e]
- Fixed launch-time deadlock when the update cache reports a newer version. [e4f5fbe]

## 2026-07-12

- Ignored Windows TUI build artifact in git. [84a0318]
- Added plugin commands/reload, local presets, sandbox, and learning telemetry. [a85e76a]
- Documented the review-uncommitted-changes workflow. [947aaae]
- Overhauled compaction reliability and staged deferred tool schemas. [6a48bd4]

## 2026-07-11

- Removed /key command and consolidated OAuth tokens into the app's own config directory. [3e54128]

## 2026-07-10

- Ran rustfmt + gofmt to unblock CI. [e801999]
- Preserved commit-SHA release tags on Windows. [e3365a3]
- Added PI-compatible !bash, plugin memory_provider hooks, install scopes, and fixed CLI precedence. [1c08256]
- Create auth config directory before opening SQLite in web auth. [77b9000]
- Added GitHub plugin installs, tool cache, models.dev API, and TUI model picker. [8f5d083]
- Documented hunk-level staging for entangled concurrent edits. [78004d4]

## 2026-07-09

- Closed critical/high security gaps and improved long-chat durability. [ed7c7c0]
- Added sudo passthrough: sudo commands surface as a blocking password flyout. [52be092]
- Added multi-provider catalog, Antigravity OAuth, web auth, and installer menu. [0e60400]
- Added /goal mode for plan-then-deploy subagent orchestration. [2480b53]
- Replaced settings field editor with dedicated per-category modals in the TUI. [6613273]
- Documented concurrent-session staging isolation in git-commit-all skill. [8480161]
- Migrated TUI to Bubble Tea v2 with declarative View and typed KeyPressMsg. [f3e5dc1]
- Let plugins declare OAuth providers and reordered auto-reflect to fire before the summary. [714cbd4]
- Fixed modified Enter handling in terminals (Shift+Enter newline, Ctrl+Enter steer). [ddabd5d]

## 2026-07-08

- Added catcode self-updater with launch-time banner and catcode --update CLI. [4a0e52a]
- Added 'watch CI to green' step to the git-commit-all skill. [1a0228e]
- Fixed a flaky core test and removed Docker-based CI jobs. [1467d8b]
- Updated git-commit-all skill to prefer exact CI gates. [5da7ce2]
- Rewrote README and added a top-level Windows installer. [b6b19bd]
- Implemented cross-process-safe atomic file writes via a new fsutil module. [cfcc07d]
- Closed production-readiness gaps including SSRF protections and lifecycle edge cases. [3383c6f]
- Added cross-session workspace presence tracking to avoid concurrent-session conflicts. [d105507]
- Brought plugins to feature parity with direct core edits for approvals, intercom, diagnostics, and lifecycle. [3c34a1b]
- Wired up the ask tool flyout in the TUI so user questions actually appear. [2c92b9c]

## 2026-07-07

- Retriggered self-hosted runner verification for docker/buildah job. [c3da32c]
- Routed CI jobs to secure self-hosted Podman runners. [65e8be5]
- Enabled cargo build --locked and CARGO_NET_RETRY in Dockerfile for reproducible, resilient builds. [3d4a791]
- Changed Dockerfile base from golang:1.24-slim to golang:1.24-bookworm. [4b5f3aa]
- Made AppImage/MSI optional in releases so they publish without wixl/appimagetool. [020091f]
- Fixed Dockerfile inline-comment parse error and rustfmt drift in CI. [2f51b11]
- Documented conditional push and workflow reporting in the git-commit-all skill. [1be7ae0]
- Installers download prebuilt binaries from GitHub Releases keyed by commit SHA instead of compiling from source. [6fafe02]


## 2026-06-29 — Initial commit

- Initial public release of the assistant coding harness (named *umans-harness* at the time). [7e95016]
- Multi-turn conversations with tool execution and model provider integration. [7e95016]
- Added subagents and intercom system; removed fixed `--max-turns` cap. [3e9b8be]
- Added macOS standalone executable, Windows MSI installer builds, vision handoff, and image attachments. [9860270]
- Fixed plugin hook dispatch and added CI workflow. [8083e55]
