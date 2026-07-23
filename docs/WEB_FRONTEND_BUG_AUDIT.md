# CatCode Web Frontend Bug Audit

## Executive summary

Revision `07967df62c11725a4ee3085e56f23e942aa24a29` contains seven confirmed user/developer-facing reliability defects under the tested environments:

| Severity | Count |
| --- | ---: |
| P0 | 2 |
| P1 | 2 |
| P2 | 3 |
| P3 | 0 |

The highest-risk subsystem is the editor/project lifecycle. Two independently reproduced paths silently lose unsaved edits: switching projects discards every dirty Monaco model without a prompt, and a delayed save response clears the dirty flag for text typed after the submitted snapshot.

The terminal is the next-highest risk. Transport loss leaves a silent dead terminal that still appears alive, same-ID sessions can be terminated in the wrong workspace, and normal termination leaves an unacknowledged control WebSocket open.

All seven findings were reproduced twice. `WEB-002` was also reproduced twice against a clean production build. The docking matrix completed twice for all five movable panels and all four dock locations without a confirmed docking-state failure or duplicate chat composer. Resize interruption cleanup passed in both instrumented passes.

This was not a stable-worktree audit. Additional user changes landed while testing. The initial worktree already had local modifications; later, more files changed and the current working tree stopped typechecking/building because `header.tsx` removed imports that its JSX still references. A detached clean worktree was therefore used to verify the commit’s checks, production build, production startup, mobile smoke test, and production reproduction of `WEB-002`. Findings below point to code that exists in the audited commit and are not dependent on the later header changes.

This report does not claim the frontend is bug-free. Two final exhaustive application-wide passes could not be completed after the shared working tree changed underneath the audit. The repeated focused final passes produced no additional reproducible findings beyond the seven documented here.

### Remediation status — 2026-07-23

All seven confirmed findings have been fixed in the working tree:

- `WEB-001`: project switches now stop for an explicit dirty-buffer discard confirmation, including switches launched from the command palette.
- `WEB-002`: editor saves capture immutable submitted snapshots and execute serially; later edits remain dirty and reverse-order disk writes are prevented.
- `WEB-003`, `WEB-004`, and `WEB-006`: terminal reconnect/status handling is visible, reattachment cannot silently create a replacement PTY, termination is workspace-scoped, and control sockets receive an acknowledgement and close.
- `WEB-005`: persisted layout state is schema-checked, unknown values are repaired, and dimensions are clamped to preserve a usable editor at restore time and after viewport changes.
- `WEB-007`: build/start now require and validate real Node.js 22.13+ before Next starts; installers and user-facing runtime documentation no longer claim Bun is a compatible server runtime.

Post-fix verification included 106 passing tests, a passing TypeScript check, a successful Node 22 production build, two browser reproductions each for the save race and dirty project-switch guard, two terminal lifecycle/cross-workspace/acknowledgement reproductions, and two persisted-layout recovery measurements. The Bun-only build path now fails immediately with the intended actionable Node requirement instead of failing during Next page-data collection.

## Audit environment

- Date: 2026-07-23
- Host timezone: America/New_York
- OS: Linux
- Browser: Headless Chrome 150.0.7871.24 through Puppeteer 25.3.0
- Viewports exercised directly: 1366×768 plus the repository mobile audit at 375×667, 390×844, 412×915, 768×1024, and 1280×800
- Runtime:
  - Node 22.21.1
  - Bun 1.3.14
- Application modes:
  - Next development custom server
  - Clean-commit production build and custom server
- Authentication: Existing local single-account credentials from the gitignored `web/.env.local`
- Workspaces used for cross-project tests:
  - `/home/karutoil/glm-5.2-ai-harnesss/web`
  - `/home/karutoil/glm-5.2-ai-harnesss`
- Audit scripts: `web/scripts/frontend-regression.mjs` and
  `web/scripts/terminal-regression.mjs`; generated evidence:
  `web/.frontend-audit/runtime/`

### Worktree qualification

At audit start, the following relevant files were already modified: `web/src/app/globals.css`, `approval.tsx`, `attach.tsx`, `chat.tsx`, `composer.tsx`, `flyout.tsx`, `markdown.tsx`, `message.tsx`, `thinking.tsx`, `toasts.tsx`, `tool-call.tsx`, and `web/tailwind.config.ts`, plus non-web files.

During the audit, additional local changes appeared, including `header.tsx`, `intercom.tsx`, `sidebar.tsx`, and `work-state.tsx`. The later `header.tsx` state references `CheckIcon` and `DotIcon` after removing their imports. The final current-worktree typecheck/lint/build failures caused by those edits are recorded as environment drift, not as findings against the audited commit.

## Commit or revision audited

- Git commit: `07967df62c11725a4ee3085e56f23e942aa24a29`
- Commit date: 2026-07-23 10:19:24 -0400
- Subject: `docs(changelog): reference sandbox migration commit [c8add8c]`
- Primary runtime discovery occurred on that commit plus the local modifications present at audit start.
- Clean production verification used a detached temporary worktree at exactly the commit above.

## Commands executed

| Command | Result |
| --- | --- |
| `npm run typecheck` | Could not start because Node/npm were not on the initial shell `PATH`. |
| `bun run typecheck` | Passed on the initial worktree. Later failed after concurrent local edits with three undefined names in `header.tsx`. |
| `bun run lint` | Initially completed with four warnings. Later failed on the same three undefined header icons. `next lint` also emits its Next 16 deprecation notice. |
| `bun test` | 104 passed, 0 failed. |
| `bun run build` with Bun’s `node` shim | Failed during page-data collection: `node:sqlite` could not be resolved for `/api/preview`, `/api/git`, and `/api/version`. Confirmed as `WEB-007`. |
| `bun run build` with Node 22.21.1 first on `PATH` | Passed on the initially audited worktree. |
| Clean detached commit: typecheck/lint/tests/build | Typecheck passed; lint completed with four warnings; 104 tests passed; production build passed. |
| `bun run dev` | Started successfully on the documented port 3000 and served the authenticated IDE. |
| `bun run start` after clean detached build | Started successfully on port 3001. Next emitted a warning that `next start` is not intended with `output: standalone`, but the custom server accepted requests. |
| `bun run audit:mobile` in development | Passed authenticated routes: zero horizontal-overflow findings and all expected mobile bottom navs present. |
| `bun run audit:mobile` against clean production | Passed with the same result. |
| `scripts/frontend-regression.mjs` | Exercised save races, dirty project switching, corrupt persistence, interrupted resize, all panel/dock combinations, console/network logging, and repeat runs. |
| `scripts/terminal-regression.mjs` | Exercised terminal destruction, missing reconnect/status, same-ID cross-workspace termination, terminate acknowledgement, and WebSocket lifecycle twice. |

The first mobile login attempt used port 3100 and received Better Auth’s expected invalid-origin rejection because the repository trusts the documented development origins. It was rerun on port 3000 and passed; the port-3100 result is not a finding.

## Feature map

### Main UI modes

- `IdeShell` owns one `useAgent()` instance and one workspace-scoped `useIde()` instance.
- `uiMode` selects the desktop/mobile IDE shell or chat-only full-bleed mode.
- Desktop IDE includes the fixed activity bar, Explorer sidebar, editor/main area, right and bottom docks, status bar, drop overlay, project switcher, settings, and command palette.
- Mobile uses a single visible view selected by bottom navigation: Files, Editor, Chat, Git, Terminal, or Preview.
- Focus mode hides desktop IDE chrome and docks but leaves dock components mounted with `display: none`.

### Movable panels and docks

- Movable panels: Chat, Terminal, Source Control, Preview, Screen.
- Dock positions: left, right, bottom, main.
- `panelLocations` defines each panel’s assigned dock.
- `panelVisibility` defines whether a panel participates.
- `activeDockPanels` selects the visible panel for each dock.
- Explorer/editor are fixed surfaces selected by setting the active left/main dock pointer to `null`.
- Native drag state is held in `IdeShell.dragging`, mirrored by `body.catalyst-panel-dragging`, and consumed by an always-mounted `DockDropOverlay`.

### Persistent and in-memory state

Workspace-scoped localStorage key:

```text
catcode:ide-layout:<encodeURIComponent(workspace)>
```

Legacy migration key:

```text
catcode:ide-layout
```

Persisted fields include active panel, dock assignments and visibility, dock sizes, collapsed/visible flags, expanded directories, live terminal metadata, active terminal ID, and UI mode.

In-memory-only fields include open editor/diff/patch tabs, active editor tab, Git status, preview target/history, file-tree cache/search/menu/rename state, Monaco view instances, pending terminal commands, modal state, drag state, focus mode, and the current mobile view.

Other browser keys discovered include theme/model/thinking preferences and the per-workspace chat drawer key:

```text
catalyst:chat-drawer:<encodeURIComponent(workspace)>
```

### Connection ownership

- Chat/core connection: one `EventSource` in the `useAgent()` owned by `IdeShell`. `ChatInner` receives that API and does not construct another connection.
- Terminal: one WebSocket for the currently rendered terminal, plus one-shot WebSockets for explicit termination.
- Preview/Screen: iframe-based surfaces; Preview also registers a window `message` listener for element inspection.

### Terminal lifecycle

- Client IDs are `term_<Date.now()>_<sequence>`.
- Live metadata persists per workspace.
- The server keys PTYs by owner, workspace, and session ID, retains live PTYs after client detach, keeps 2 MiB scrollback, and replays it on reattachment.
- Only the active terminal has a mounted Ghostty renderer/WebSocket; switching tabs detaches the previous renderer while leaving its PTY alive.
- Server exit sends `exit`; explicit destruction sends `terminated` to attached clients.
- No client reconnection or detached/failed state exists, which causes `WEB-003`.

### Editor lifecycle and file refresh

- Monaco models use a workspace-bearing custom URI.
- Model disposers are registered by tab ID and disposed on tab close or wholesale workspace change.
- The visible editor is recreated on tab changes while the model survives.
- Dirty state compares model content against `savedRef`.
- Save uses `PUT /api/file`.
- Agent file-change events increment `fileChangeSeq`; clean models reload while dirty models retain content and show a disk-changed banner.
- File-tree rename/delete blocks operations affecting dirty tabs.
- Workspace switching does not perform the equivalent check.

### File tree, Git, Preview, and project flows

- File tree lazily requests one directory level, caches children by relative directory, tracks stale responses by workspace, and aborts debounced search on query/workspace changes.
- File CRUD uses `/api/file`; tree listing uses `/api/tree`; mention/palette search uses `/api/files`.
- Git panel refreshes on mount, focus, and a 10-second interval. Actions return a fresh status. Diff/patch tabs are routed to Monaco viewers.
- Preview supports Markdown, HTML, images, PDF, loopback proxy URLs, external iframes, navigation history, reload, and element-to-chat inspection.
- Screen builds a noVNC iframe from a pasted WebSocket URL.
- Project switching is initiated through `useAgent.switchWorkspace`; the resulting workspace value changes the `useIde` storage key and resets project-local in-memory IDE state.

### Loading, errors, cleanup, and shared-state writers

- Editor exposes loading, top error, saving, dirty, and disk-changed indicators.
- Tree exposes loading/search/empty/error states.
- Git exposes loading/busy/error/result state.
- Markdown preview exposes loading/error; iframe previews rely on the embedded surface plus a blank-page hint.
- Terminal exposes only alive/exit code, not transport state.
- Resize cleanup handles pointerup, pointercancel, lost capture, and unmount, restoring cursor, selection, body class, and global listeners.
- Drag cleanup handles tab dragend, document dragend, successful drop, and shell unmount.

Multiple systems update the same state in these important places:

- `openTabs`/models: explorer actions, Git diff actions, command palette, editor dirty events, rename/delete actions, and workspace restoration.
- terminal metadata: activity bar, terminal tabs, Git run-command actions, WebSocket exit messages, and persistence restoration.
- Git status: tree decoration fetch, Git panel mount/poll/focus refresh, and Git action responses.
- panel layout: activity bar, command palette, native drag/drop, resize handles, focus mode, chat-only mode, mobile navigation, and localStorage restoration.
- agent workspace/session state: SSE snapshots/events plus command POST responses.

## Test matrix

Legend: Pass = tested with no confirmed defect; Finding = linked confirmed defect; Partial = meaningful coverage but not the full requested matrix; Blocked = required external environment or later worktree drift prevented completion.

| Area | Coverage | Result |
| --- | --- | --- |
| Contract and implementation map | Contract, package, all IDE components, chat/agent/IDE state, registry, server/workspace, all API routes, auth/setup/login/project UI, tests, and mobile audit inspected | Pass |
| Baseline checks | Typecheck, lint, tests, build under Bun and Node | `WEB-007`; otherwise clean-commit pass with warnings |
| Development runtime | Authenticated custom server, console/network/WS instrumentation | Pass |
| Production runtime | Clean build, startup, authenticated mobile audit, production save-race reproduction | Pass |
| Panel movement | 5 panels × 4 destinations × 2 runs; body class, persisted assignment, overlay, duplicate composer invariants | Pass |
| Drag interruptions | Synthetic cancellation/dragend cleanup and repeated rapid moves | Pass; true browser-window loss across real iframe/GPU surfaces only partial |
| Resize | All currently rendered handles; pointercancel; clamps; persisted invalid sizes; cleanup | `WEB-005`; interruption cleanup passed |
| Persistence | Scoped keys, corrupt JSON handling, missing/unknown/huge fields, repeated reload | `WEB-005`; schema/device migration only partial |
| Terminal lifecycle | Create, destroy, detach-like loss, termination, same-ID multi-workspace, WS inventory | `WEB-003`, `WEB-004`, `WEB-006` |
| Terminal input/PTY apps | PTY shell prompt observed; complete special-key/alternate-screen/10k-scrollback matrix | Partial |
| Editor | Open, dirty state, delayed save, dirty project switch, reload/disk verification | `WEB-001`, `WEB-002` |
| External file refresh/conflict | Code-path inspection and agent-event behavior | Partial |
| File explorer | Lazy load, refresh, stale-workspace guard, create fixture, search/open, keyboard and mutation implementation review | Pass for tested paths; large/deep/symlink matrix partial |
| Git | Normal repository status requests, tree decorations, polling/action implementation review | Partial; destructive/conflict/history combinations not exhaustively run |
| Preview | Implementation, unit tests, proxy rewrite tests, panel docking | Partial; live local dev server and arbitrary external iframe matrix not completed |
| Screen | Implementation and docking | Blocked: no VNC environment |
| Chat | Single connection ownership, composer instance during moves, dock-to-dock remount behavior | Partial; no paid live streamed agent/tool/approval run |
| Project switching | Dirty editor round-trip across two real projects | `WEB-001` |
| Command palette/focus/keyboard | Implementation review; palette-driven file opening in early harness iterations | Partial after later header edits made current app unmountable |
| Mobile | Authenticated dev and production smoke at five sizes, nav and horizontal overflow | Pass for scripted coverage; virtual keyboard/touch/rotation partial |
| Error states | Delayed save, destroyed terminal, invalid storage, unauthenticated/invalid-origin observations | Partial |
| Resource reliability | WebSocket creation/closure inventory during terminal/panel stress | `WEB-006`; multi-hour heap/detached-node profiling not completed |

## Confirmed findings

## `WEB-001`: Switching projects silently discards dirty editor buffers

**Severity:** P0  
**Confidence:** High  
**Area:** Projects  
**Frequency:** Always  
**Environment:** Chrome 150, 1366×768, development mode  
**User impact:** Unsaved file content is silently destroyed when a user switches projects. Returning to the original project restores the disk version with no recovery path.

### Preconditions

A file is open and dirty, and a second valid project is available.

### Reproduction steps

1. Open `catcode-frontend-audit-project-switch-dirty.txt`.
2. Replace its content without saving and verify the tab shows `●`.
3. Open Switch project and select the other workspace.
4. Switch back to the original workspace.
5. Reopen the fixture.

### Expected behavior

The application blocks the switch with Save all / Discard / Cancel, or retains the dirty buffer under its original workspace.

### Actual behavior

The switch completes with no dialog. Returning shows the old disk content and a clean tab. This reproduced twice with distinct unsaved strings.

### Evidence

- `web/.frontend-audit/runtime/editor-findings.json`
- `web/.frontend-audit/runtime/project-switch-dirty-1.png`
- `web/.frontend-audit/runtime/project-switch-dirty-2.png`

### Root cause

- `web/src/components/ide/shell.tsx:456-464`, `IdeShell`: forwards the requested project directly to `agent.switchWorkspace`.
- `web/src/lib/use-ide.ts:205-220`, workspace-key effect: calls `disposeAllEditorModels()` and resets state.
- `web/src/lib/editor-model-registry.ts:16-20`: disposes every Monaco model.

No project-switch guard checks `openTabs.some(tab => tab.dirty)`, and open tabs are not persisted.

### Recommended fix

Route every workspace-switch entry point through a single IDE-aware guard. When dirty tabs exist, present Save all / Discard and switch / Cancel. Save all must await every PUT and remain on the current workspace if any save fails.

### Regression test

Edit a fixture, initiate a switch, assert the workspace does not change before a decision, cancel and verify the buffer remains, then confirm discard and verify only then that the workspace changes.

### Related findings

Related to `WEB-002`, the other path that removes dirty protection from unsaved content.

## `WEB-002`: A delayed save marks newer unsaved edits as saved

**Severity:** P0  
**Confidence:** High  
**Area:** Editor  
**Frequency:** Intermittent  
**Environment:** Chrome 150, 1366×768, development and production modes  
**User impact:** Text typed while a save is in flight is not stored, but the dirty marker clears. Closing, reloading, or switching projects then loses the text without warning.

### Preconditions

A Monaco text file is open and a save response is delayed.

### Reproduction steps

1. Type `saved-request-1`.
2. Press Ctrl+S and hold the outgoing `PUT /api/file`.
3. Type `-typed-after-request-1`.
4. Release the request and wait for HTTP 200.
5. Read the file via `GET /api/file` and inspect the tab.

### Expected behavior

The disk contains the submitted snapshot and the later change remains dirty.

### Actual behavior

The disk contains only `saved-request-1`; the tab is clean even though Monaco contains the longer string. Reproduced twice in development and twice again against a clean production build.

### Evidence

- `web/.frontend-audit/runtime/editor-findings.json`
- `web/.frontend-audit/runtime/save-race-1.png`
- `web/.frontend-audit/runtime/save-race-2.png`

### Root cause

`web/src/components/ide/editor.tsx:96-114`, `Editor.save`, serializes one `contentRef.current` value into the request, then reads the mutable ref again after the response and assigns that newer value to `savedRef` and `lastSavedByPath` before calling `markDirty(false)`.

### Recommended fix

Capture `const submitted = contentRef.current` before starting fetch. Send and record only `submitted`. On success, compare the current live model value with `submitted` and leave dirty true if they differ. Version overlapping saves so an older response cannot clear a newer state.

### Regression test

Delay a save, type more text, resolve the response, and assert the dirty marker remains. Add a reverse-response-order test for two concurrent saves.

### Related findings

Related to `WEB-001`.

## `WEB-003`: Disconnected or destroyed terminals remain silently usable-looking

**Severity:** P1  
**Confidence:** High  
**Area:** Terminal  
**Frequency:** Always  
**Environment:** Chrome 150, 1366×768, development mode  
**User impact:** A terminal can become permanently dead while still looking alive. Input is silently dropped, and users receive no reconnect, failure, exit, or detached status.

### Preconditions

An attached terminal PTY exists.

### Reproduction steps

1. Open a terminal.
2. Destroy the matching server PTY from a second authenticated terminal WebSocket.
3. Wait for the attached socket to receive `terminated` and close.
4. Inspect the tab, stored session, and renderer.

### Expected behavior

The terminal changes to a disconnected/exited state and reconnects or offers recovery.

### Actual behavior

The renderer remains, stored metadata remains `alive: true` with `exitCode: null`, no status text appears, and no replacement terminal WebSocket is created. Reproduced twice.

### Evidence

- `web/.frontend-audit/runtime/terminal-audit.json`
- `web/.frontend-audit/runtime/terminal-dead-1.png`
- `web/.frontend-audit/runtime/terminal-dead-2.png`

### Root cause

`web/src/components/ide/terminal.tsx:136-150` handles `data` and `exit` only, ignores `terminated`, leaves `onerror` empty, and has no `onclose`. `setTerminalExit` is therefore never called for transport loss or explicit destruction.

### Recommended fix

Track connecting/connected/detached/exited/failed/reconnecting separately. Handle `terminated`, `error`, and `close`; disable silent input writes; and retry attachment with bounded backoff using the same workspace/session ID.

### Regression test

Force-close a terminal socket without an exit envelope. Assert a disconnected state, exactly one reconnect loop, reattachment to the existing PTY when available, and an actionable failed state when it is gone.

### Related findings

Related to `WEB-004` and `WEB-006`.

## `WEB-004`: Terminal termination can target the same ID in the wrong workspace

**Severity:** P1  
**Confidence:** High  
**Area:** Terminal  
**Frequency:** Rare  
**Environment:** Chrome 150, 1366×768, development mode  
**User impact:** Closing a terminal can kill a different project’s PTY when the user has the same persisted/client-generated ID in both projects.

### Preconditions

Two workspaces have live terminal sessions with the same ID for the same user.

### Reproduction steps

1. Open `audit_collision_*` in workspace A.
2. Open the same ID in workspace B.
3. Send `{type:"terminate", sessionId}`.
4. Observe the attached clients.

### Expected behavior

The terminal in the initiating/current workspace closes.

### Actual behavior

Workspace A—the first matching map entry—receives `terminated` and closes, while B remains open. Reproduced twice.

### Evidence

`web/.frontend-audit/runtime/terminal-audit.json`

### Root cause

- `terminalKey` at `web/src/server/server.ts:104-106` correctly includes workspace.
- `findOwnedTerminal` at lines 130-135 ignores workspace.
- The terminate branch at lines 242-247 uses that ambiguous lookup.
- `terminateTerminalSession` at `web/src/components/ide/terminal.tsx:31-37` does not send workspace.

### Recommended fix

Send workspace with terminate, authorize it, and look up the exact composite key. Reject ambiguous or missing workspace rather than scanning by ID.

### Regression test

Open same-ID terminals in A and B, terminate B, verify B closes and A still executes a marker command.

### Related findings

Related to `WEB-003` and `WEB-006`.

## `WEB-005`: Persisted dimensions are restored without validation and can hide the IDE

**Severity:** P2  
**Confidence:** High  
**Area:** Layout  
**Frequency:** Rare  
**Environment:** Chrome 150, 1366×768, development mode  
**User impact:** Corrupt or incompatible stored layout values can reduce the main editor to zero width. Recovery is possible only if the user knows to collapse multiple docks from the activity bar.

### Preconditions

The workspace layout key contains invalid or very large persisted values.

### Reproduction steps

1. Store `sidebarWidth=100000`, `copilotWidth=100000`, `bottomPanelHeight=-9000`, and an invalid `activePanel`.
2. Reload.
3. Measure the regions.

### Expected behavior

The state is validated, clamped, repaired, and rewritten.

### Actual behavior

The sidebar and right dock each measure 100000 px while the main editor measures 0 px. The invalid state remains stored. Reproduced twice.

### Evidence

`web/.frontend-audit/runtime/audit-runtime.json`

### Root cause

`loadPersisted` and the workspace restore effect in `web/src/lib/use-ide.ts:99-126,205-220` copy recognized fields without runtime validation. Setter clamps are bypassed. `IdeShell` applies the raw widths directly.

### Recommended fix

Use a versioned runtime schema, validate enums/maps, require finite numbers, clamp against component and viewport bounds, reserve a minimum main width, repair active pointers, and persist the repaired payload.

### Regression test

Load a table of corrupt/partial/old/huge states at several viewports and assert usable geometry, valid assignments, no crash, and repaired storage.

### Related findings

None.

## `WEB-006`: Normal terminal termination leaves its control WebSocket open

**Severity:** P2  
**Confidence:** High  
**Area:** Performance  
**Frequency:** Always  
**Environment:** Chrome 150, 1366×768, development mode  
**User impact:** Repeated terminal closure leaks one WebSocket per live terminal until page unload, producing unbounded resource growth in long sessions.

### Preconditions

The requested terminal exists.

### Reproduction steps

1. Open a second socket using the client termination flow.
2. Send terminate for the existing ID.
3. Wait for a response/close.
4. Inspect readyState.

### Expected behavior

The server acknowledges and closes the one-shot socket.

### Actual behavior

The PTY is destroyed but the requester receives no message and remains `OPEN` after 700 ms. Reproduced twice.

### Evidence

`web/.frontend-audit/runtime/terminal-audit.json`

### Root cause

For an existing session, `web/src/server/server.ts:242-246` calls `destroyTerminal(existing)` but neither sends to nor closes the requester, which is not in the target’s client set. `terminateTerminalSession` closes only from `onmessage` or `onerror`.

### Recommended fix

Send a `terminated` acknowledgement to the requester and close it. Add a client timeout that closes the control socket even if the server fails to acknowledge.

### Regression test

Open/close 50 terminals and assert no `/api/terminal` sockets remain after acknowledgements.

### Related findings

Related to `WEB-003` and `WEB-004`.

## `WEB-007`: The documented Bun-only build path cannot import node:sqlite

**Severity:** P2  
**Confidence:** High  
**Area:** Other  
**Frequency:** Always  
**Environment:** Bun 1.3.14 with no external Node on PATH, production build  
**User impact:** A developer or installer following the documented primary Bun workflow cannot create the production frontend.

### Preconditions

Bun is installed, but a separate Node executable is not on PATH.

### Reproduction steps

1. Run `bun run build`.
2. Wait for Next page-data collection.

### Expected behavior

The documented Bun workflow builds.

### Actual behavior

Multiple API route imports fail with `Cannot find package 'node:sqlite'`. Placing Node 22.21.1 first on PATH makes the same initial worktree build successfully.

### Evidence

Baseline console output:

```text
error: Cannot find package 'node:sqlite' from '.../.next/server/app/api/preview/route.js'
error: Cannot find package 'node:sqlite' from '.../.next/server/app/api/git/route.js'
error: Cannot find package 'node:sqlite' from '.../.next/server/app/api/version/route.js'
```

### Root cause

`web/package.json` invokes `node`, the README calls Bun the primary runtime, and `web/src/lib/auth.ts` imports Node’s `node:sqlite`. In a Bun-only installation, Bun’s node shim executes the script but cannot resolve that module.

### Recommended fix

Either require and verify real Node 22.13+ for build/start, or use a Bun-compatible database adapter. Make the README, engines, package scripts, installer, and CI matrix agree.

### Regression test

Run the documented production build in separate Node-only and Bun-only CI jobs. If Bun is no longer supported, fail before Next starts with an actionable message.

### Related findings

None.

## Suspected issues requiring additional environment access

These are plausible from implementation review but were not counted because they were not reproduced twice under a valid runtime:

1. Global Ctrl/Cmd+K is intercepted unconditionally in `IdeShell`, including while Monaco, Ghostty, rename fields, and the chat composer own focus. This likely breaks Monaco’s chord prefix and terminal Ctrl+K behavior. Runtime confirmation was blocked when concurrent header edits made the shared app fail to mount.
2. Git refresh responses have no request generation/order guard. Mount, focus, interval, tree-decoration fetch, and action responses can race and an older response can overwrite newer status.
3. The Screen panel always constructs an `http://.../vnc.html` iframe, even for a `wss://` target. HTTPS deployments may block the mixed-content iframe; no HTTPS VNC environment was available.
4. External disk modifications made outside the agent event stream do not appear to have a filesystem watcher path into open Monaco models. A terminal/editor conflict matrix needs a stable runtime and controlled file watcher access.
5. The legacy unscoped layout migration fallback can be loaded into multiple workspace keys. Because terminal metadata is included, this may be one practical source of the duplicate terminal IDs required by `WEB-004`.

## Areas tested with no findings

- Single `useAgent()` ownership in the IDE shell and no duplicate chat composer during the completed docking matrix.
- Five movable panels assigned to left, right, bottom, and main in two repeated passes.
- Drag completion removed `catalyst-panel-dragging`.
- The inactive overlay did not retain pointer events in the corrected invariant.
- Pointer-cancel resize cleanup removed `catalyst-resizing` and restored cursor/text selection.
- File-tree requests reject stale responses from a previous workspace by comparing the request workspace.
- File-tree search uses AbortController and a debounce.
- File-tree rename and delete explicitly refuse to affect dirty tabs.
- Editor models use workspace-qualified Monaco URIs.
- Clean-tab agent file changes refresh the model; dirty tabs retain their text and show a warning in the inspected path.
- Terminal server keys normal PTYs by owner, workspace, and session ID and reuses that exact key for open/reattach.
- Terminal input and resize envelopes are bounded server-side.
- Preview Markdown request ordering uses a generation token/cancellation check.
- Clean-commit dev and production mobile smoke tests found no document-level horizontal overflow at the scripted sizes.
- Authenticated dev and production routes loaded without captured page errors in the completed mobile smoke.
- The clean commit passed all 104 existing Bun tests.

## Areas that could not be tested

- Firefox, Safari/WebKit, Windows, and macOS behavior.
- Real mouse loss outside the browser across every iframe/GPU canvas combination.
- Touch and pen pointer capture, virtual keyboard occlusion, safe-area devices, and physical rotation.
- Every requested viewport/zoom combination; the repository audit covered five representative sizes.
- Live Screen/noVNC input and reconnection.
- Full terminal key matrix, alternate-screen programs, password prompts, 10,000-line scrollback, high-latency bursts, proxy idle timeouts, and actual process/server restart.
- Complete Git topology matrix: no-commit, detached HEAD, upstream divergence, conflicts, rebase, submodules, and concurrent destructive operations.
- All Preview file types and a live Vite/Next dev server with HMR.
- Live paid agent streaming, tool calls, approvals, attachments, cancellation, model switching, and very long chat history.
- Passkeys, two-factor authentication, backup codes, and real session-expiry recovery.
- Large-file/large-tree/large-Git performance thresholds and multi-hour heap/detached-DOM profiling.
- Every malformed/partial/duplicate API and SSE response.
- Two final application-wide passes after the worktree became unstable.

## Recommended fix order

1. `WEB-001` — prevent dirty-buffer destruction on workspace changes.
2. `WEB-002` — make save completion snapshot/version aware.
3. `WEB-003` — implement terminal transport state and reattachment.
4. `WEB-004` — scope termination to workspace.
5. `WEB-006` — acknowledge and close termination sockets.
6. `WEB-005` — validate, clamp, and migrate persisted layouts.
7. `WEB-007` — align supported runtime documentation, scripts, and dependencies.

`WEB-001` and `WEB-002` should ship together with regression coverage because both undermine the browser’s dirty-state safety boundary.

## Recommended regression-test plan

1. Add a browser-level editor suite with controllable API latency:
   - edit during save;
   - overlapping saves resolving out of order;
   - failed save followed by navigation;
   - dirty project switch via every entry point;
   - external update during dirty and clean states.
2. Add terminal WebSocket integration tests against the custom server:
   - create, attach, detach, reattach, exit, terminate;
   - destroyed socket without exit;
   - server restart;
   - duplicate IDs across workspaces;
   - repeated close leak check;
   - stale messages after replacing a renderer.
3. Add pure state/property tests for `useIde` persistence:
   - versioned migrations;
   - corrupt JSON and wrong types;
   - unknown panel IDs;
   - contradictory locations/active pointers;
   - invalid dimensions and viewport shrink;
   - legacy terminal metadata.
4. Keep the 5×4 docking matrix as a Puppeteer test and run it with terminal output, preview iframe load, and chat streaming fixtures.
5. Add request-generation guards and race tests for Git/tree/palette/preview fetches.
6. Expand mobile automation to the full requested viewport/zoom set and test every nav item with touch emulation.
7. Add CI jobs for the actual supported runtime matrix. Production build and custom-server startup must be exercised from the artifact layout used by releases.

## Final confidence assessment

Confidence is high for the seven confirmed findings: each has a direct code path, controlled evidence, and two reproductions; the two editor data-loss findings also have independent disk verification, and `WEB-002` was repeated in production.

Confidence is medium for overall frontend coverage. The core layout/editor/project/terminal paths received adversarial runtime testing, but several large matrices remained partial or blocked, and the shared worktree changed during the audit. Therefore the absence of findings in untested combinations should not be interpreted as evidence of correctness.

No additional reproducible findings were discovered during the repeated focused final passes under the tested environments. Two consecutive complete application-wide passes were not achievable after the worktree changed and the current app stopped mounting.
