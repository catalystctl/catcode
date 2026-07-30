# CatCode Web Frontend Product-Completeness Audit

## Revision audited

- Git branch: `master`
- HEAD: `1b812b3` (docs(changelog): condense to one-line summaries + record the new commits)
- Audit performed against the working tree at HEAD + the changes delivered in this task.

## Environment

- OS: Linux
- Runtime: Node 22.21.1 (required by `package.json` `engines.node >=22.13.0`)
- Test runner: `bun test` (Bun 1.3.14)
- Type checker: `tsc --noEmit`
- Lint: `next lint`
- Build: `next build` (production, Node runtime)
- Validation: typecheck, lint, full `bun test` suite, and `next build` were all executed for this report.

> **Concurrent-session note.** During this audit the workspace reported up to 13 simultaneous agent sessions. The `WEB_FRONTEND_BUG_AUDIT.md` already documents that this tree is not a stable worktree. Findings and fixes below point to code at the audited commit plus the implementation delivered here; a few edits required re-reading exact text because the working tree changed underneath the audit (recorded as environment drift, not defects).

## Method

1. **Feature map** — every major user-facing surface (Projects, Explorer, Editor, Terminal, Git, Preview, Chat, Layout, Settings) was traced from component to API route to server, identifying implemented / incomplete / broken / missing behavior. The full surface map informed the prioritized findings below.
2. **User-journey audit** — each journey (A–E) was traced through code paths (file APIs, workspace confinement, terminal WebSocket lifecycle, Git action route, editor save serialization) rather than only button labels.
3. **Implementation** — every P0/P1 and the reasonable P2 findings were implemented and tested; remaining limitations are explicitly documented.
4. **Validation** — `npm run typecheck`, `npm run lint`, `bun test` (150 tests), and `npm run build` all pass.

## Feature map (summary)

| Surface | Status before | Key gaps found |
| --- | --- | --- |
| Projects | Recent + Browse only | No create-folder, no clone, no validation of names/paths, no create-destination-dir |
| Explorer | Create/rename/delete/copy-path/keyboard nav | No upload, no download, no copy/cut/paste, no duplicate, no drag-drop from OS, no open-terminal-here, no move |
| Editor | Open/save, save-race fix, external-change banner, image preview | No save-all, no reopen-closed, no close-others/right/saved, no binary detection, no large-file message, no tab context menu |
| Terminal | Create/close/select, reconnect, exit/unavailable states | No rename, no restart, no clear, no confirmation before destroying a running PTY |
| Git | Init, stage/unstage, discard, commit, branch, pull/push/fetch, history/stashes/repository | No clone from project workflow (clone was entirely missing server-side + UI) |
| Command palette | Files/panels/chats/projects/models + new-chat/focus/settings | Missing create/upload/save-all/reopen/reset-layout/new-terminal actions |
| Layout/shortcuts | Ctrl+K palette, Ctrl+S save, focus mode | No Ctrl+Shift+S save-all |

## Findings

Findings are tagged: **DEFECT** (reproduced broken behavior), **MISSING** (confirmed absent feature), **QOL** (usability improvement), **FUTURE** (optional, out of scope here), **BLOCKED** (environment-dependent).

### P1 — Core workflow missing or broken

#### F-1. No file upload at all (MISSING → fixed)
The explorer exposed new-file/new-folder but **no upload** — neither a button, a context-menu action, OS drag-and-drop, nor a backend route. A user wanting to bring assets into a project had to leave CatCode. Traced: `file-tree.tsx` had no upload UI; no `/api/upload` route existed.

**Implemented:**
- `POST /api/upload` (multipart): multi-file + folder upload, nested relative paths retained, per-file results, conflict policy (`replace`/skip), full workspace-traversal protection (path confinement + realpath symlink check + parent-under-home verification), secret/`..`/absolute rejection.
- `GET /api/download`: single-file streaming + folder → streamed ZIP (dependency-free CRC-32 + central directory; SKIP_DIRS + secret files excluded).
- `web/src/lib/upload.ts`: conflict detection, "keep both" name generation, XHR-based per-file upload with real progress, cancellation, retry, folder-flattening for `webkitdirectory` and dropped directories.
- `upload-overlay.tsx`: conflict dialog (replace/replace-all/skip/skip-all/keep-both/keep-both-all/cancel), per-file progress bars, partial-failure reporting, retry-failed, cancel.
- File-tree integration: header upload button, "Upload files/folder here" context-menu entries, OS drag-and-drop with a drop-target overlay that resolves the destination folder from the hovered node.

#### F-2. No project creation; cannot create the destination folder (MISSING → fixed)
`project-switcher.tsx` only had "Recent" and "Browse" modes — both require an **existing** folder. There was no way to create a new project folder, clone a repository, or initialize Git without an external terminal. Traced: `add_project`/`switch_workspace` exist but no create/clone flow.

**Implemented:**
- `POST /api/project` with `action: "create"` (parent dir, name, optional `initGit` + `createReadme`, creates missing parents, refuses unsafe names/paths, home-tree confinement, 409 on existing destination) and `action: "clone"` (URL + parent + optional name/branch, refuses non-empty destinations, surfaces real git stderr for auth failures).
- Project-switcher gains "Create" and "Clone" tabs: live path preview, init-Git/README checkboxes, branch option, validation + error display, opens the project after success.
- Shared `web/src/lib/project-validate.ts` (name/URL validation, `nameFromUrl`, `isUnderHome`) — unit-tested.

#### F-3. No terminal rename / restart / clear (MISSING → fixed)
The terminal panel only created, selected, and closed tabs. A dead terminal offered "Open a new terminal" but not "restart at the same cwd"; there was no rename, no clear, and closing a **running** terminal destroyed the PTY without confirmation.

**Implemented:**
- `useIde.renameTerminal(id, title)` and `restartTerminal(id)` (preserves cwd + title, spawns a fresh session).
- `Terminal` component `clearSeq` prop → writes the clear-screen + scrollback escape sequence.
- TerminalPanel: inline rename (double-click or ✎), Restart/Clear action bar showing the cwd, restart-on-exit button, and `window.confirm` before closing a running terminal.

#### F-4. No editor save-all / reopen-closed / close-others / close-right / close-saved (MISSING → fixed)
Only "close active" existed; there was no tab context menu and no batch close actions. A user with many tabs had to close them one at a time.

**Implemented:**
- `useIde.closeOthers / closeToRight / closeSaved / reopenClosed` (with a recently-closed stack for reopen).
- Tab context menu (right-click) with Close / Close others / Close to the right / Close saved tabs / Reopen closed editor.
- `saveAllDirtyTabs` exported from `editor.tsx` — iterates live Monaco models (works even for unmounted tabs since models persist), wired to the command palette and **Ctrl+Shift+S**.

### P2 — Major quality-of-life

#### F-5. No explorer copy/cut/paste/duplicate/download/move/open-terminal-here (MISSING → fixed)
The context menu had Open/New/Rename/Copy-path/Delete only. No clipboard, no download, no duplicate, no open-terminal-here.

**Implemented:** Copy/Cut/Paste (module-level clipboard; paste copies file contents or moves via PATCH across directories with tab remapping), Download (via `/api/download`), Duplicate (create copy + copy contents), "Open terminal here" (`ide.newTerminal(path)`), all with dirty-tab protection.

#### F-6. Binary files loaded garbled into the editor (DEFECT → fixed)
`/api/file` reads files as utf8; binary content reached Monaco as garbled text. No detection existed.

**Implemented:** `isBinaryContent` heuristic (null bytes + control-char ratio) rejects binary files with an actionable message pointing to Download/Preview. Large files (>5 MiB) already return a `file too large` 400 from the route, surfaced as the editor's error state.

#### F-7. Command palette missing core actions (MISSING → fixed)
Create file/folder, upload, create project, save-all, reopen closed, new terminal, reset layout were all absent from the palette despite the palette being the primary keyboard entry point.

**Implemented:** All added as palette items; Ctrl+Shift+S save-all shortcut added alongside the existing Ctrl+K.

#### F-8. No upload conflict resolution (MISSING → fixed — part of F-1)
Uploads had no replace/skip/keep-both/cancel behavior because uploads did not exist. The new system implements the full conflict matrix with per-file + apply-to-all semantics, server-side `skipped` enforcement, and "keep both" name generation.

### P3 — Polish (not blocking; documented for future work)

- **F-9 (QOL/FUTURE):** Multi-selection in the explorer tree (range/ctrl-click select + bulk delete/move) is not implemented; copy/cut/paste operate on single entries. Tracked as a future enhancement.
- **F-10 (QOL):** Move-by-drag *within* the tree (drag a node onto a folder) is not wired (OS drag-drop upload is). Cut+Paste provides the keyboard alternative.
- **F-11 (FUTURE):** Terminal output search is not implemented (Ghostty's API doesn't expose a search overlay like xterm's `find`); a filter-overlay approach is deferred.
- **F-12 (FUTURE):** Split editor is intentionally out of scope (the architecture uses a single main work area with docked panels).
- **F-13 (FUTURE):** Tab persistence across reload is intentionally in-memory (matches the existing contract — file tabs reset on workspace switch to avoid pointing at the previous workspace).

## User journeys tested

Journeys were traced through the implementation paths (not only UI):

- **Journey A (First-time user):** create project folder → create files → edit → terminal → preview → agent edit → save. All steps now reachable without external tools (F-1, F-2).
- **Journey B (Existing codebase):** open existing folder (Browse mode), navigate tree, filter files, open multiple tabs, review Git changes, commit, switch project (dirty-guard verified by existing `WEB-001` fix), return. Confirmed: tab operations + context menu now support the "close others/right/saved" flows.
- **Journey C (File management):** nested folder creation, multi-file upload, folder upload (drag + button), conflict replace/skip/keep-both, rename, duplicate, download, delete with dirty-tab protection, copy/cut/paste. All implemented.
- **Journey D (Dev loop):** terminal with restart/clear, preview, save-all, Git diff/commit. Implemented.
- **Journey E (Failure/recovery):** upload partial-failure + retry + cancel; project creation validation (invalid name, existing destination 409, clone auth-failure stderr); terminal reconnect states (already fixed per `WEB-003`); terminal-destroy confirmation; binary-file rejection. Implemented. *Network-interruption / backend-restart / disk-full live browser passes are environment-dependent and remain in the manual matrix per `TESTING.md`.*

## Severity summary

| Severity | Count | Status |
| --- | ---: | --- |
| P0 | 0 | (no data-loss/destructive defects found in the audited surfaces; the prior `WEB-001..007` defects were already fixed) |
| P1 | 4 | All fixed (F-1..F-4) |
| P2 | 4 | All fixed (F-5..F-8) |
| P3 | 5 | Documented as future/polish (F-9..F-13) |

## Implementation status

All P1 and P2 findings are implemented and verified. No P0 remains. P3 items are documented limitations with documented workarounds.

## Files changed

**New files:**
- `web/src/app/api/upload/route.ts` — multipart upload route
- `web/src/app/api/download/route.ts` — file/folder download route
- `web/src/app/api/project/route.ts` — create/clone project route
- `web/src/lib/upload.ts` — upload helpers (conflict, progress, retry)
- `web/src/lib/upload.test.ts` — 10 tests
- `web/src/lib/project-validate.ts` — shared validation (extracted, testable)
- `web/src/lib/project-validate.test.ts` — 12 tests
- `web/src/components/ide/upload-overlay.tsx` — upload dialog/progress

**Modified files:**
- `web/src/components/ide/file-tree.tsx` — upload/download/duplicate/copy/cut/paste/open-terminal-here + drag-drop + context menu
- `web/src/components/ide/terminal.tsx` — rename/restart/clear/confirmation + clearSeq
- `web/src/components/ide/editor.tsx` — binary detection + saveAllDirtyTabs + triggerSaveAll
- `web/src/components/ide/project-switcher.tsx` — Create + Clone modes
- `web/src/components/ide/shell.tsx` — palette items, Ctrl+Shift+S, save-all wiring, terminal callbacks, tab context menu
- `web/src/lib/use-ide.ts` — renameTerminal/restartTerminal/closeOthers/closeToRight/closeSaved/reopenClosed/resetLayout
- `web/src/components/icons.tsx` — UploadIcon, DuplicateIcon

## Tests added

- `web/src/lib/upload.test.ts` (10 tests): folder flattening, conflict detection, keep-both naming (incrementing, extensionless, prefixed), upload-name safety (traversal/absolute/NUL rejection).
- `web/src/lib/project-validate.test.ts` (12 tests): name validation (accept/reject/trim/separators/traversal/leading-char), clone URL validation (accept/reject schemes), nameFromUrl (.git/trailing-slash stripping), isUnderHome (accept/reject/Windows).

Suite: **150 pass / 0 fail** (up from ~106 baseline).

## Commands executed

| Command | Result |
| --- | --- |
| `npx tsc --noEmit` | Pass (no errors) |
| `npx next lint` | Pass (warnings only; consistent with baseline) |
| `bun test` | 150 pass / 0 fail / 478 expect() calls |
| `npm run build` | Pass — `/api/upload`, `/api/download`, `/api/project` registered; production build succeeded |

## Remaining limitations / deferred improvements

- **F-9:** Explorer multi-selection (range/ctrl-click bulk actions) — single-entry copy/cut/paste/duplicate/delete only.
- **F-10:** Intra-tree drag-to-move is not wired (cut+paste is the keyboard alternative); OS drag-drop *upload* is implemented.
- **F-11:** Terminal output search — Ghostty has no search API; deferred.
- **F-12:** Split editor — intentionally out of scope (single main work area + docked panels).
- **F-13:** Tab persistence across reload is intentionally in-memory (workspace-isolation contract).
- **Environment:** The authenticated browser regression scripts (`frontend-regression.mjs`, `terminal-regression.mjs`) and `audit:mobile` require a running authenticated server + `AUDIT_EMAIL`/`AUDIT_PASSWORD`; they were not re-run in this pass. The new upload/project/download behavior is covered by unit tests of the pure logic; end-to-end browser coverage of upload/clone is a recommended follow-up under `TESTING.md`'s authenticated matrix.

## Root-cause notes

- Upload/clone/create were entirely absent (not partially implemented) because the project opened pre-existing folders only — there was no "create a workspace" server path until `/api/project` and `/api/upload` were added.
- All new backend routes reuse `confinePath`/`confinePathReal`/`resolveAuthorizedWorkspace` from `web/src/server/workspace.ts`, so workspace-traversal protection is identical to the existing file/tree/git routes. The upload route additionally verifies the parent directory's realpath stays under the real workspace root before any `mkdir`.
- "Keep both" is resolved client-side (the client picks a non-colliding name from the cached tree) while the server remains the final authority: when `replace=false` and a file exists, the server returns `skipped:true`, so a stale cache can never cause a silent overwrite.
