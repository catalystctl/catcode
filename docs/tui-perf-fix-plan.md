# TUI performance fix plan

Prioritized backlog from `docs/tui-perf-findings.md` + `docs/tui-perf-surface.md`.
**No code fixes in this planning step.** Implementers execute one batch per step.

Scope: `tui/` (+ `docs/tui-perf-findings.md` status updates). No core/web changes.

Source inventory (open, excluding existing wontfix): **5 P0 · 23 P1 · 17 P2**.
Already wontfix in findings: **U-015**, **D-012**, **D-013**.

---

## Goals

1. Eliminate busy-frame jank (double-render chrome, layout/invalidate storms, scout progress SetContent storms).
2. Cap session RSS leaks (`args`/`diff` uncapped retention).
3. Cut measurable waste on common paths (string copies, parse-once, idle 1 Hz View, redundant refreshes).
4. Polish secondary paths without risking Bubble Tea v2 / idle-zero / visual goldens.
5. Re-audit until **zero new P0/P1** (or only documented wontfix/deferred).

### Success for the re-audit loop

| Gate | Criteria |
|------|----------|
| Findings ledger | Every issue in `docs/tui-perf-findings.md` has severity, `file:symbol`, and status `fixed` / `wontfix` / `deferred` (+ reason) |
| Final scout | Zero **new** P0/P1 after last fix batch (only documented wontfix/deferred remain) |
| Tests | `go test ./tui/...` green |
| BT v2 | `View() tea.View`; no sync `prog.Send` before `Run`; animation = color-only on spinner cadence; view height <= terminal-1 |
| Idle | Idle pty byte count ~0 over >=2s (`!busy && ready`, spinner off) |
| Diff | Limited to `tui/` (+ findings/plan docs); no unrelated core/web |

---

## Batch A — P0 (jank / CPU spikes / layout storms / leaks)

**Worker step size:** one step. Land all five before Batch B.
**Theme:** stop double work and storms; stop uncapped retention.

### A1. R-001 — Double-build chrome every busy frame (P0)

| | |
|--|--|
| **Files** | `tui/render.go` (`relayoutHeights`, `*Height` helpers, `View`); `tui/goal_ux.go` (`goalProgressPanelHeight`); `tui/mention.go` (`mentionFlyoutHeight`) |
| **Approach** | Replace render-to-measure with structural / cached heights. Prefer row-budget math (input wrap lines, shelf rows, goal collapsed/expanded rows, oauth 0/1, position bar 0/1). Optionally cache last painted chrome strings and reuse within the same `View` call so measure != second full paint. Keep `relayoutHeights` in `View` (already fixed overflow path — do not remove). |
| **Tests** | `go test ./tui/ -run 'TestVisualGoldens|TestRenderSmoke|TestInputBox|TestMentionRendersInFullView|TestGoalModalProfileAndLayout|TestAskRequestRendersFlyout'` |
| **Risks** | Wrong height -> viewport overflow / cursed scroll drift (BT v2 needs <= term-1 slack). Animated border must still animate on spinner frames only. Golden snapshots sensitive to chrome line counts. |

### A2. R-002 — layout() invalidates transcript on height-only change (P0)

| | |
|--|--|
| **Files** | `tui/render.go` (`layout`) |
| **Approach** | Invalidate wrap cache **only when viewport.Width() changes** (or explicit content-width triggers). Height-only -> `relayoutHeights()` + `refresh()` only if content dirty; never `invalidateAll` for H-only. Keep full invalidate on true resize width / `WindowSizeMsg`. |
| **Tests** | `go test ./tui/ -run 'TestVisualGoldens|TestRenderSmoke|TestRebuildBlocksFromHistory'` + manual: long transcript + todo/scout panel toggle -> no full re-wrap |
| **Risks** | Stale wrap if some path changes effective wrap width without Width() change (nested padding). Prefer explicit invalidate helper for those rare sites. |

### A3. R-003 — Scout tool_result double transcript rebuild (P0)

| | |
|--|--|
| **Files** | `tui/handlers.go` (`tool_result`) |
| **Approach** | Single path: update block -> narrow invalidate (see B1/R-006) -> `relayoutHeights()` if panel height changed -> **one** `refresh()`. Never `refresh()` then `layout()`. |
| **Tests** | `go test ./tui/ -run 'TestRenderSmoke|TestNearestToolUsesRenderedLineRanges'` |
| **Risks** | Missing refresh leaves stale tool card; over-narrow invalidate leaves old body. |

### A4. U-002 — subagent_progress refreshes transcript every phase tick (P0)

| | |
|--|--|
| **Files** | `tui/handlers.go` (`subagent_progress`); optionally reuse `scheduleStreamRefresh` in `tui/blocks.go` |
| **Approach** | Coalesce progress paints (33-100ms or shelf-dirty flag). When only `curTool`/counts change -> dirty shelf + `relayoutHeights`, **no** `renderBlocks`/`SetContent`. Call `layout`/`relayoutHeights` only on entry add/remove (height change). |
| **Tests** | `go test ./tui/ -run 'TestRenderSmoke|TestVisualGoldens'` + parallel-scout repro: count SetContent/progress event |
| **Risks** | Stale shelf text if dirty flag missed; do not add a new always-on timer — reuse stream coalesce or spinner-driven paint. |

### A5. D-001 — Uncapped args / diff retention (P0)

| | |
|--|--|
| **Files** | `tui/blocks.go` (`logTool`, `logApproveDiff`, `capOutput` -> shared `capStored`); `tui/handlers.go` (`tool_result`, `approval_request`) |
| **Approach** | Apply same 256 KiB (or shared) cap to `args` and `diff` at ingest; truncated marker for collapsed cards. Cap both `pendingApproval` copy and lasting approve block. Do not weaken `maxStoredOutput`. |
| **Tests** | `go test ./tui/ -run 'TestRenderDiffPanel|TestRebuildBlocksFromHistory|TestRenderSmoke'` + unit for cap on large args/diff |
| **Risks** | Expanded tool/approval UI may show truncated body — acceptable if marker clear; approval decision must still show enough command/path context. |

### Batch A validation (end of step)

```bash
go test ./tui/...
```

Manual/metrics: idle pty ~0; busy stream + many tools — invalidateAll/SetContent rate down; no uncapped multi-MB args after large write approval.

Update findings: mark R-001, R-002, R-003, U-002, D-001 -> `fixed`.

---

## Batch B — P1 (measurable waste on common paths)

Split into **B1 / B2 / B3** so one worker finishes one sub-batch per step.

### Batch B1 — Demote layout / narrow invalidate / kill double refresh

| ID | Files | Safe approach | Tests | Risks |
|----|-------|---------------|-------|-------|
| **R-004** | `render.go` `layout`; `handlers.go` activity-scroll | Split relayoutHeightsOnly vs refresh; activity scroll mutates scroll only | `go test ./tui/ -run 'TestRenderSmoke|TestVisualGoldens'` | Missed height change clips shelf |
| **R-005** | `handlers.go` (36x `layout()`) | Classify each site: width/resize -> `layout()`; height/chrome -> `relayoutHeights()`; content -> `refresh`/narrow invalidate. Keep `WindowSizeMsg` on full `layout()` | Static review of 36 sites + `go test ./tui/...` | One missed site -> overflow; one over-demoted -> stale wrap |
| **R-006** | `handlers.go` `tool_result`/`toggle_*`; `blocks.go` `invalidateAll` | Truncate cache at block `cacheIdx`/`renderStart`; re-render forward only | `TestNearestToolUsesRenderedLineRanges`, `TestRenderSmoke` | Focus decoration / line ranges wrong |
| **U-007** | `blocks.go` `logTool`; `handlers.go` `tool_call` | `logTool` skip refresh when caller will layout/refresh | todo_write mid-turn SetContent count | Missing paint |
| **U-008** | `handlers.go` `applyGoalState` | Compare height budget/phase; relayout only on height change; refresh only when step cards added | `TestGoalModalProfileAndLayout` | Stale goal panel |
| **U-009** | `handlers.go` approval/activity scroll | Mutate scroll only; View rebuilds chrome | hold-down repro + smoke | Diff panel scroll desync |
| **U-011** | `blocks.go` `finalizeInFlight`; `handlers.go` `done`/`aborted` | Narrow invalidate touched tools; single relayout+refresh at turn end | smoke + history | In-flight badge stuck |

**B1 exit:** `go test ./tui/...`; findings R-004..R-006, U-007..U-009, U-011 -> `fixed`.

### Batch B2 — Alloc / parse / stream cost on hot path

| ID | Files | Safe approach | Tests | Risks |
|----|-------|---------------|-------|-------|
| **R-007** | `blocks.go` `renderBlocks`, `renderedLineOffset` | Maintain `cacheLines` (+ per-block start/end) incrementally; avoid `String()` except final payload | `TestNearestToolUsesRenderedLineRanges`, smoke | Off-by-one focus/scroll targets |
| **R-010** | `render.go` `renderInputBoxAnimated` | Theme-cache 32-style ramp keyed by (dim, accent); recompute phase only | `TestInputBox*`, `TestVisualGoldens`, `CATCODE_ANIMATED_BORDER=1` smoke | Theme switch stale ramp |
| **R-009** | `render.go` `constrainViewContent`, `View` | Prefer structural clamp while building; keep cheap assert in tests; avoid full-screen MaxWidth/MaxHeight every frame | `TestVisualGoldens`, `TestRenderSmoke`, modal overflow tests | Removing safety clamp -> BT scroll/cursor drift — **measure first**; if unsure -> defer |
| **U-005** + **D-004** | `protocol.go` `get`; `main.go` reader | Parse once to `map[string]json.RawMessage` at read; `get` indexes map; avoid re-Unmarshal | broad `go test ./tui/...` | Event field type regressions |
| **U-001** | `main.go` `tick`/`tickMsg` | Idle: skip View work when tick only re-arms (no toast/live/spinner). Or toast-only timer. Keep 1s when live/busy. **Do not** change cadence without idle-pty measure | idle pty ~0 over 10s; smoke | Breaking toast expiry / spinner restart |
| **U-003** | `handlers.go` metrics/conc/skills | Dirty flags (`footerDirty`/`chromeDirty`) or coalesce into next spinner/tick; skills list update without View until palette | `TestFooterMetricsCommandAndRender` | Stale footer numbers |
| **U-006** | `main.go` spinner/tick/stream | Drive in-flight elapsed from spinner/stream `now`; stop tick->refresh while stream pending or spinner active | busy stream SetContent rate | Timers freeze on in-flight cards |

**B2 exit:** `go test ./tui/...`; mark IDs fixed; confirm idle still ~0 pty.

### Batch B3 — Heavier / careful P1 (may partially defer)

| ID | Files | Safe approach | Tests | Risks |
|----|-------|---------------|-------|-------|
| **R-008** | `blocks.go` `renderBlock*`; `markdown.go` | Incremental line/segment cache by width; append from last stable fence/paragraph; keep `streamBatch` | long-stream CPU repro + smoke | Fence/table glitches — **candidate defer** if no metrics |
| **U-004** | handlers/blocks refresh paths | Narrow invalidate + coalesce first (A/B1); defer off-thread rebuild unless still stalls | large tool_result while deltas | Model ownership races |
| **U-010** + **D-003** | `handlers.go` `history`; `blocks.go` `rebuildBlocksFromHistory` | Chunked rebuild + early `msgs=nil`; or Cmd goroutine swap-in one msg | `TestRebuildBlocksFromHistory*` | Concurrent mutation / flicker — **candidate defer** without stall metrics |
| **D-002** | `blocks.go` text Builder; delta/thinking | Soft-cap (e.g. 256 KiB+) with marker **or** drop raw after finalized width-keyed cache | smoke + history | Truncating visible reply — **needs product decision / defer** |
| **D-005** | `handlers.go` `skills`; `protocol.go` `skillInfo` | Slim unmarshal (Name/Description/Location only) | palette/slash tests if any | Missing content if something reads Content |
| **D-006** | `mention.go` | Raise TTL 30-120s; stream git stdout; stop at `mentionCacheCap` | `TestMention*`; large-repo `@` | Stale file list after git add |
| **D-007** | `images.go` `materializeOwnedImage` | Prefer paths; else encode in `tea.Cmd` | image attach tests if present | Protocol expects data URL — verify before change |

**B3 exit:** `go test ./tui/...`; any skipped items -> `deferred` with reason in findings.

---

## Batch C — P2 (polish)

**Worker step size:** one step (or two if timeboxed). Skip anything that risks goldens without benefit.

| ID | Files | Approach | Tests | Risks |
|----|-------|----------|-------|-------|
| **R-011** | `blocks.go` `hasLiveContent`; `main.go` tick | Maintain `inFlightCount` / check `subProgress`+tail | smoke | Missed live -> no timer refresh |
| **R-012** | `transcript_nav.go` | Re-render prev+new focus heads only; no full invalidate | smoke + nav | Lost focus marker |
| **R-013** | `render.go` `View`; `modal.go`; `ask.go` | Early-out when overlay active; skip unused base join | `TestAsk*`, keybinds modal, elevated-auth tests | Dimmed backdrop change |
| **R-014** + **D-011** | `render.go` footer; `handlers.go` metrics | Parse metrics once to typed fields; drop Raw | `TestFooterMetricsCommandAndRender`, goldens | Footer string format drift |
| **R-015** | `render.go` chrome helpers | Prebuild theme styles | goldens | Theme switch |
| **R-016** | `blocks.go` `renderKeyHints` | Resolve labels once / skip if defaults | smoke | Wrong key hint text |
| **R-017** | `tool_blocks.go` `arg` | Parse args once into map on tool_call | tool render tests | Arg type edges |
| **R-018** | `blocks.go`/`diff.go`/`tool_blocks.go` | Package-level `dimItalicStyle` | `TestRenderDiffPanel` | Theme init order |
| **R-019** | `render.go` `View` | Prefer no-color theme branch over post-hoc regex strip | colors-disabled smoke | Missed ANSI |
| **R-020** | `main.go` tick comment | Fix comment to 1s; **do not** change cadence | n/a | Accidental cadence change |
| **U-012** | `main.go` Paste; `images.go` | Size-gate large pastes -> Cmd | paste/image tests | Slow reject path |
| **U-013** | `handlers.go`/`keybinds.go` | Resolve `msg.String()` once; optional action map | `keybinds_test.go` | Bind miss |
| **U-014** | elevated-auth module / `main.go` | Remove dead timeout cmd/msg **or** enable to match docs — no second always-on timer | elevated-auth tests | Accidental timeout UX |
| **D-008** | `mention.go` | Pre-lowercased cache; optional prefix index | `TestMention*` | Filter behavior |
| **D-009** | `extras.go` `pushHistory` | Copy-trim like blocks | history tests | None significant |
| **D-010** | `settings.go` `save` | Debounced background save; sync on quit | `settings_persist_test.go` | Lost settings on crash |

**C exit:** `go test ./tui/...`; mark fixed; leave intentional wontfix as-is.

---

## WONTFIX / defer list

### Already wontfix (keep)

| ID | Rationale |
|----|-----------|
| **U-015** | Mouse/keyboard transcript scroll already cheap (no layout/refresh). Do not "optimize". |
| **D-012** | Session lock I/O is cold-path only. |
| **D-013** | Existing caps (`maxBlocks`, `capOutput`, mention/image caps) are correct guards — extend pattern, do not remove. |

### Defer unless metrics prove need (too risky / large)

| ID | Severity | Why defer |
|----|----------|-----------|
| **R-008** | P1 | Incremental markdown is correctness-heavy (fences/tables); keep `streamBatch` until CPU profile shows dominate after B2. |
| **R-009** | P1 | `constrainViewContent` is a BT v2 height safety net; remove/replace only after structural clamp proven + goldens green. If Batch B2 attempt fails safely -> `deferred`. |
| **U-010** / **D-003** | P1 | Off-UI-thread history swap risks model races; prefer chunked sync + nil msgs first; full async only if `/load` stall measured. |
| **D-002** | P1 | Soft-capping assistant text changes UX; needs explicit product call or only drop raw after immutable render cache exists. |
| **U-004** (async) | P1 | Prefer A/B1 narrow+coalesce; goroutine rebuild only if channel fill still measured. |
| **D-007** | P1 | Data-URL vs path is protocol-coupled; defer until core accept path confirmed. |
| **U-013** | P2 | Human key rates; noise vs refresh storms. |
| **R-019** | P2 | Niche colors-disabled path; low ROI vs golden/theme risk. |

### Do not redo (already fixed — surface map)

Idle spinner stop; `relayoutHeights` in `View`; `maxBlocks` copy-trim; stream coalesce + `streamBatch`; `hasLiveContent`-gated tick refresh; stdin writer channel.

---

## Recommended execution order

```
Batch A  (P0)     -> re-scout P0 only
Batch B1 (layout) -> re-scout layout/invalidate
Batch B2 (alloc)  -> re-scout idle/busy CPU
Batch B3 (careful P1 / defer rest)
Batch C  (P2 polish)
Final scout       -> zero new P0/P1
```

After each batch: update `docs/tui-perf-findings.md` statuses; run `go test ./tui/...`.

---

## Cross-cutting regression risks (all batches)

1. **Bubble Tea v2:** `View() tea.View` / `tea.NewView`; `KeyPressMsg`; never sync `prog.Send` before `Run` (`update.go` launchUpdateCheck stays goroutine-wrapped).
2. **`visual_golden_test.go`:** Any chrome height/wording/footer metrics change breaks `TestVisualGoldens` — update goldens only when intentional UX change; prefer structural height caches that preserve painted strings.
3. **Idle zero redraw:** No new always-on timers; spinner only while `busy || !ready`; U-001 must not reintroduce 1 Hz pty churn.
4. **Cursed renderer:** view height <= terminal-1 (slack line in `relayoutHeights`).
5. **Animation:** color-only; piggyback spinner; `CATCODE_ANIMATED_BORDER` + reduced-motion gate.
6. **Event pump:** always re-arm `waitForEvent`; keep coreEvents backpressure (blocking send).

---

## Validation commands (canonical)

```bash
# Full package
go test ./tui/...

# High-signal subsets after chrome/layout work
go test ./tui/ -run 'TestVisualGoldens|TestRenderSmoke|TestInputBox|TestMentionRendersInFullView|TestGoalModalProfileAndLayout|TestAskRequestRendersFlyout|TestFooterMetricsCommandAndRender|TestNearestToolUsesRenderedLineRanges|TestRebuildBlocksFromHistory|TestRenderDiffPanel'

# After history/data work
go test ./tui/ -run 'TestRebuildBlocksFromHistory|TestHistory'

# After settings/mention polish
go test ./tui/ -run 'TestMention|TestSettings|TestKeybinds'
```

Manual gates:

- Idle >=2s: pty bytes ~ 0.
- Busy long stream: spinner ~10 FPS; `SetContent` coalesced; no per-progress full rebuild (U-002).
- Many tools: no full-cache rebuild per `tool_result` after R-006.
- Large write approval: `args`/`diff` capped (D-001).

---

## Open questions (non-blocking for Batch A)

1. **D-002 soft-cap:** Cap assistant/thinking text at 256 KiB, higher limit, or only drop raw after render cache? Default plan: **defer** until after A/B1/B2.
2. **R-009:** Keep Max* clamp as debug/assert-only vs structural truncate — decide in B2 after measuring View cost post R-001.
3. **D-007:** Does core accept filesystem image paths from TUI, or data URLs only?

Batch A does not need these answered to start.

---

## Handoff checklist for implementers

- [ ] Read `docs/tui-perf-surface.md` + this plan before coding
- [ ] One batch per step; update findings statuses
- [ ] Diff only `tui/` (+ findings doc)
- [ ] `go test ./tui/...` green
- [ ] Final scout: zero new P0/P1 (or documented wontfix/deferred)
