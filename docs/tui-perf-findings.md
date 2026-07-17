# TUI performance findings

Audit pass: **render path** (View / render.go / blocks / markdown / tool+diff panels / modals / input wrap / viewport SetContent).
Scope: `tui/` only. **No code fixes in this pass.**
Status values: `open` | `fixed` | `wontfix` | `deferred`.

Source map: `docs/tui-perf-surface.md`.

---

## R-001 — Double-build chrome every busy frame (render-to-measure)

| Field | Value |
|-------|-------|
| **severity** | P0 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `relayoutHeights`, `*Height` helpers, `View` |
| **status** | fixed |

**Fix (Batch A):** Per-View viewChromeCache so relayoutHeights measure reuses chrome strings for paint (animated input once/frame).

**Evidence:** `View` (≈1431–1467) always calls `relayoutHeights()` then paints `renderHeader`, `renderActivityShelf`, `renderGoalProgressPanel`, `renderMentionFlyout`, `renderInputBox`, `renderFooter`. Height helpers measure by full render:

- `headerHeight` → `lipgloss.Height(renderHeader())` (L58)
- `footerHeight` → `inputBoxHeight()` + `lipgloss.Height(renderFooter())` (L1090–1096)
- `inputBoxHeight` → `lipgloss.Height(renderInputBox())` (L1090–1091)
- `activityShelfHeight` → full `renderActivityShelf()` (L1353–1358)
- `goalProgressPanelHeight` → full `renderGoalProgressPanel` (`goal_ux.go` L256–261)
- `mentionFlyoutHeight` → full `renderMentionFlyout` (`mention.go` L616–621)
- `positionBarHeight` → `renderPositionBar()` (L213–217)

While `busy` + spinner (~10 FPS), animated input is built **twice per frame**. Comment on `relayoutHeights`/`View` claims “CHEAP / height math only” (L60–64, L1441–1442) — false for these helpers.

**Proposed fix sketch:** Cache last painted strings or structural heights (input lines, shelf row count, goal collapsed/expanded row budget). Measure without lipgloss full paint; reuse cached chrome strings within the same `View` call.

**Repro hint:** `CATCODE_ANIMATED_BORDER=1`, start a turn, sample CPU / allocs on `renderInputBoxAnimated` while spinner ticks.

---

## R-002 — `layout()` invalidates transcript on height-only change

| Field | Value |
|-------|-------|
| **severity** | P0 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `layout` |
| **status** | fixed |

**Fix (Batch A):** layout() invalidates wrap cache only when viewport.Width() changes; height-only skips invalidateAll.

**Evidence:** L102–109:

```go
if s.viewport.Width() != prevW || s.viewport.Height() != prevH {
    s.invalidateAll()
}
s.refresh()
```

Block wrap width is `s.viewport.Width()` (`blocks.go` L170). Height-only changes (todo shelf, scout panel, goal panel, activity expand, approval banner) do **not** invalidate wrap correctness, yet clear `cache` + every `renderStr` and force full re-wrap.

**Proposed fix sketch:** Invalidate only when `Width()` changes (or explicit content/width triggers). Height-only → `relayoutHeights()` + optional `refresh` only if live blocks need it — not `invalidateAll`.

**Repro hint:** Long transcript + `todo_write` / spawn start/finish; watch `invalidateAll` / `SetContent` rate while viewport width fixed.

---

## R-003 — Scout `tool_result` double transcript rebuild

| Field | Value |
|-------|-------|
| **severity** | P0 |
| **category** | render |
| **file:symbol** | `tui/handlers.go`: `tool_result` |
| **status** | fixed |

**Fix (Batch A):** tool_result does invalidate + optional relayoutHeights + single refresh (no refresh-then-layout).

**Evidence:** ≈L407–410: `invalidateAll(); refresh();` then if scout `layout()` which always ends in another `refresh()` (and often another `invalidateAll` via R-002 when panel height changes).

**Proposed fix sketch:** Single path: update block → narrow invalidate → `relayoutHeights()` → one `refresh()`. Never refresh then `layout()`.

**Repro hint:** Finish a `spawn`/`subagent` with a large cached transcript; count `renderBlocks` / `SetContent` calls per result.

---

## R-004 — `layout()` always `refresh()` even when W/H unchanged

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `layout`; `tui/handlers.go` activity-scroll / toggle sites |
| **status** | open |

**Evidence:** `layout` always calls `refresh()` (L109). Activity focus mode calls `layout()` on every ↑/↓/page scroll and toggle (`handlers.go` ≈1713–1729, 1802). Scroll does not change transcript content; when shelf height is stable, W/H often unchanged → still full `renderBlocks()` + `SetContent`.

**Proposed fix sketch:** Split API: `relayoutHeightsOnly()`, `refreshIfNeeded()`. Activity scroll → mutate `activityScroll` only (View already rebuilds shelf). Demote height-only panel events to `relayoutHeights` without transcript refresh when cache valid.

**Repro hint:** Expand activity shelf with many todos; hold ↓; profile `viewport.SetContent`.

---

## R-005 — 36× `handlers.go` `layout()` for panel/chrome mutations

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/handlers.go` (36 call sites) |
| **status** | open |

**Evidence:** `rg -c 's\.layout\(\)' tui/handlers.go` → **36**. Sites include todo panel, scout start/finish, banners, queue, goal toggle, provider_changed, done/aborted, activity toggles — mostly height/chrome, not terminal width resize. Combined with R-002/R-004 → layout storm.

**Proposed fix sketch:** Classify each site: true resize/width → `layout()`; height/chrome → `relayoutHeights()`; content-only → `refresh()` / narrow invalidate. Keep `WindowSizeMsg` on full `layout()`.

**Repro hint:** Static review of the 36 sites + busy turn with todos+scouts; compare `invalidateAll` frequency before/after demotion.

---

## R-006 — `tool_result` / toggles call `invalidateAll` for one block

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/handlers.go`: `tool_result`, `toggle_tool_output`, `toggle_reasoning`; `tui/blocks.go`: `invalidateAll` |
| **status** | open |

**Evidence:** `invalidateAll` (L125–137) resets entire `cache` and clears every block’s `renderStr`/`renderStart`/`renderEnd`. `tool_result` ≈407; expand-output ≈1784; thinking toggle ≈1766. Only one block’s body changed.

**Proposed fix sketch:** Truncate cache at `match`’s `cacheIdx` / `renderStart`, or re-render from that index forward; keep prefix `cache` intact. Focus decoration (R-012) should not clear body cache.

**Repro hint:** 200+ finalized tool cards; one new `tool_result`; measure time in `renderBlockFull`.

---

## R-007 — `renderBlocks` full-string copies for line offsets

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/blocks.go`: `renderBlocks`, `renderedLineOffset` |
| **status** | open |

**Evidence:** Extending cache: `start := renderedLineOffset(s.cache.String())` (L176) — allocates full cache string then `strings.Count`. Live path: `b.WriteString(s.cache.String())` (L184) + `renderedLineOffset(b.String())` (L187, L204). Golden-path allocs scale with transcript size every refresh.

**Proposed fix sketch:** Maintain `cacheLines int` (and per-block start/end) incrementally on `WriteString`; avoid `String()` except final viewport payload (or write cache directly into a reusable buffer).

**Repro hint:** Long session + stream coalesce refresh; alloc profile on `renderBlocks`.

---

## R-008 — Streaming markdown full reparse each batch

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/blocks.go`: `renderBlock`/`renderBlockFull`; `tui/markdown.go`: `renderMarkdown` |
| **status** | open |

**Evidence:** Stream throttle `streamBatch=64` (L456–432) still calls `renderBlockFull` → `renderMarkdown(b.text.String(), w)` on **entire** text (L461–477). `renderMarkdown` splits all lines and restyles (L24–82). Cost ≈ O(n²/64) over a long reply. Ponytail at L144–146 acknowledges missing incremental wrap cache.

**Proposed fix sketch:** Incremental line/segment cache keyed by width; append-only parse from last stable fence/paragraph boundary; keep throttle as frequency cap.

**Repro hint:** Stream a multi-KB markdown reply with code fences; CPU in `renderMarkdown` vs bytes streamed.

---

## R-009 — `constrainViewContent` styles entire screen every View

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `constrainViewContent`, `View` |
| **status** | open |

**Evidence:** Every ready frame L1483–1504: `lipgloss.NewStyle().MaxWidth(...).MaxHeight(...).Render(content)` over the joined full UI string.

**Proposed fix sketch:** Structural clamp (truncate parts while building), or cheap line-split truncate without lipgloss style object; reserve Max* for debug/assert builds.

**Repro hint:** Busy spinner frames; time spent in `constrainViewContent` / lipgloss Render.

---

## R-010 — Animated input rebuilds 32-style ramp every paint

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `renderInputBoxAnimated` |
| **status** | open |

**Evidence:** L760–771 builds `ramp := make([]lipgloss.Style, inflightLevels)` (`inflightLevels=32`) with `lipgloss.NewStyle()` per level every call. Combined with R-001 → up to 64 style constructions / frame while busy+env.

**Proposed fix sketch:** Theme-cache ramp keyed by `(dim, accent)` hex; only recompute phase/`styleAt` per frame.

**Repro hint:** `CATCODE_ANIMATED_BORDER=1` + busy turn; allocs in `renderInputBoxAnimated`.

---

## R-011 — `hasLiveContent` scans all blocks on 1s tick

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/blocks.go`: `hasLiveContent`; `tui/main.go`: `tickMsg` |
| **status** | open |

**Evidence:** When `cur == nil`, L773–777 walks all `s.blocks` looking for in-flight tools. Called from `tickMsg` every **1s** (`main.go` L602–603, L679–684).

**Proposed fix sketch:** Maintain `inFlightCount` / check `subProgress` + tail tools only.

**Repro hint:** 400-block session, idle-but-one-inflight edge; or microbench `hasLiveContent`.

---

## R-012 — Transcript focus nav invalidates entire cache

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/transcript_nav.go`: `moveTranscriptFocus`, find-match path |
| **status** | open |

**Evidence:** L51–52 and L93–94: `invalidateAll(); refresh()` only to move `▸` decoration (`decorateFocusedBlock` in `blocks.go` L437–443).

**Proposed fix sketch:** Re-render only previous + new focused block heads, or overlay focus marker without dropping body cache.

**Repro hint:** Alt+Up/Down through a long transcript; watch cache rebuilds.

---

## R-013 — Modal/ask/sudo build full base then discard it

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `View`; `tui/modal.go`: `renderModalOverlay`; `tui/ask.go`: `renderAskOverlay` |
| **status** | open |

**Evidence:** `View` joins full chrome+viewport (L1444–1468) then `renderModalOverlay` / `renderAskOverlay`. Overlay helpers `lipgloss.Place(...)` the box and **return only the overlay** — `base` unused (`modal.go` L3089–3108; `ask.go` L264–277).

**Proposed fix sketch:** Early-out in `View` when modal/ask/sudo active: skip chrome join / skip unused base; Place against blank or dimmed backdrop intentionally.

**Repro hint:** Open command palette during busy stream; compare View allocs vs idle modal.

---

## R-014 — Footer metrics JSON unmarshal on every footer paint

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `renderFooterPerformance`, `renderMetrics` |
| **status** | open |

**Evidence:** `renderFooterPerformance` L344 `json.Unmarshal(s.lastMetrics, …)` every call. Footer painted via height measure + View (R-001). `FooterMetrics` defaults on. Separate `renderMetrics` also unmarshals (L381+).

**Proposed fix sketch:** Parse metrics once in the metrics handler into typed fields; footer only formats cached floats/strings.

**Repro hint:** Busy frames with footer metrics on; allocs in `encoding/json` from `renderFooter*`.

---

## R-015 — Per-frame `lipgloss.NewStyle` in chrome helpers

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `renderPositionBar`, `renderFooter`, `renderOauthBanner`, `constrainViewContent`, activity shelf box |
| **status** | open |

**Evidence:** e.g. `renderPositionBar` L202–208 builds a new styled banner every call (also from `positionBarHeight`). Footer L303 `NewStyle().MaxWidth`. Shelf L1345–1350. Theme globals in `styles.go` already exist for many colors — these paths bypass them.

**Proposed fix sketch:** Prebuild/update theme styles on theme change; reuse width-parameterized styles carefully (or structural width without new Style).

**Repro hint:** Scroll up so position bar shows; busy spinner; allocs in lipgloss style construction.

---

## R-016 — `renderKeyHints` string ReplaceAll on every block render

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/blocks.go`: `renderKeyHints`, `renderBlock` |
| **status** | open |

**Evidence:** L446–453 scans/replaces `ctrl+o` / `ctrl+t` on every `renderBlock` output (streaming batches + cache fills after invalidate).

**Proposed fix sketch:** Pass resolved key labels into panel renderers once; or replace only when keybind ≠ default.

**Repro hint:** Custom keybinds + long stream; time in `strings.ReplaceAll` under `renderBlock`.

---

## R-017 — Tool render re-parses args JSON per field

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/tool_blocks.go`: `argField`, `(*block).arg` |
| **status** | open |

**Evidence:** Each `b.arg(key)` → `json.Unmarshal` of full args (L34–43). Renderers call `arg` multiple times per block. Amplified by R-006 full-cache rebuilds and in-flight live re-renders on tick/refresh.

**Proposed fix sketch:** Parse args once into `map` on tool_call / cache on block; renderers read fields from map.

**Repro hint:** Many tool cards + `invalidateAll`; CPU in `json.Unmarshal` from `argField`.

---

## R-018 — `dimStyle.Italic(true)` allocates styles in panel hot paths

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/blocks.go`: `renderOutputPanel`; `tui/diff.go`: `renderDiffPanel`; `tui/tool_blocks.go` hints |
| **status** | open |

**Evidence:** Repeated `dimStyle.Italic(true).Render(...)` (e.g. `blocks.go` L648–654, `diff.go` L85–91) creates derived styles per call instead of a package-level `dimItalicStyle` (compare `thinkStyle` already italic in `styles.go`).

**Proposed fix sketch:** Add `dimItalicStyle` (and err italic) to theme init; use in panels.

**Repro hint:** Expand large tool/diff panels; allocs around Italic style derive.

---

## R-019 — `colorsDisabled` strips ANSI over full screen every View

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/render.go`: `View` L1484–1485; `noColorANSIRe` |
| **status** | open |

**Evidence:** When `colorsDisabled()`, every frame runs regex `ReplaceAllString` on the entire composed UI.

**Proposed fix sketch:** Prefer rendering with no-color styles from the start (theme branch already exists in `themes.go`); avoid post-hoc strip.

**Repro hint:** `NO_COLOR=1` / colors-disabled path; busy frames; time in regexp.

---

## R-020 — Misleading tick cadence comment (docs drift)

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | render |
| **file:symbol** | `tui/main.go`: `tick`, `tickMsg` handler comment |
| **status** | open |

**Evidence:** `tick()` uses `tea.Tick(time.Second, …)` (L602–603) but comment at L694 says “every 500ms”. Not a CPU bug by itself; risks wrong “fix” of cadence during perf work.

**Proposed fix sketch:** Fix comment to 1s; do not change cadence without idle-pty measurement.

**Repro hint:** Read L602 vs L694.

---

---

Audit pass: **update path** (`Update` / handlers / keybinds / timers / mouse / modal routing / protocol ingest).
Scope: `tui/` only. **No code fixes in this pass.**
Deduped against R-001…R-020 (layout/invalidate/render costs already filed stay render-category; this pass adds Update-loop triggers and event-pump issues).

---

## U-001 — Always-on `tickMsg` forces `View` every 1s while idle

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/main.go`: `tick`, `tickMsg` case in `Update` |
| **status** | open |

**Evidence:** `tick()` re-arms forever (`tea.Tick(time.Second)` L602–603). Every `tickMsg` returns `tea.Batch(tick(), …)` (L690–699), so Bubble Tea runs `View` once per second even when `!busy && ready`, toast is nil, and `hasLiveContent()` is false (refresh gated L683–684). Idle spinner stop prevents ~10 FPS storms, but the 1 Hz timer still drives full `relayoutHeights` + chrome (see R-001) every second. Cursed renderer may emit ~0 pty bytes on identical frames, yet CPU/allocs still run.

**Proposed fix sketch:** Skip View work when tick only re-arms and no toast expiry / no live refresh / no spinner restart. Or demote idle tick to a toast-only timer started when a toast is set. Keep 1s cadence when `hasLiveContent` or `busy`.

**Repro hint:** Idle session; sample CPU in `View`/`relayoutHeights` over 10s with no input (expect spikes every 1s today).

---

## U-002 — `subagent_progress` refreshes transcript on every phase tick

| Field | Value |
|-------|-------|
| **severity** | P0 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `subagent_progress` |
| **status** | fixed |

**Fix (Batch A):** subagent_progress: relayoutHeights on add/remove only; phase ticks no longer call refresh()/SetContent.

**Evidence:** After mutating shelf fields, L767 always `s.refresh()` (full `renderBlocks` + `SetContent`). New/done entries also `s.layout()` (L732, L739). Unlike deltas (`scheduleStreamRefresh` 33–100ms coalesce), scout progress (`tool` / `tool_end` / `streaming`) is unbounded → SetContent storm during parallel scouts, independent of stream coalesce.

**Proposed fix sketch:** Coalesce like streams (`scheduleStreamRefresh` or shelf-only dirty → `relayoutHeights` without transcript rebuild when only `curTool`/counts change). `layout()` only when entry added/removed (height change).

**Repro hint:** Run `/parallel` or a goal with multiple workers; count `viewport.SetContent` / `refresh` per progress event.

---

## U-003 — High-frequency core events force full `View` with no dirty gate

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `metrics`, `umans_conc`, `skills`, `vision_config`, `plugin_commands`; `tui/main.go`: `coreEventMsg` |
| **status** | open |

**Evidence:** Every `coreEventMsg` runs `handleCoreEvent` then returns `waitForEvent` (L1233). Bubble Tea paints `View` after every Update. `metrics` (L612–637) and `umans_conc` (L639–647) only mutate footer fields — no `refresh`, but still trigger full chrome rebuild. Comments note umans concurrency is polled independently and shown while idle (`render.go` ~452). Mid-turn metrics arrive periodically on top of spinner + stream refresh.

**Proposed fix sketch:** Dirty flags (`footerDirty`, `chromeDirty`) and skip expensive chrome when only footer numbers change; or coalesce metrics/conc into the next spinner/`tickMsg` paint. Avoid View work for pure list updates (`skills`) until palette opens.

**Repro hint:** Idle Umans session; watch Update/View rate vs `umans_conc` events in debug log.

---

## U-004 — Synchronous heavy work on UI goroutine stalls the event pump

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `handleCoreEvent`; `tui/blocks.go`: `refresh`, `rebuildBlocksFromHistory`; `tui/main.go`: core stdout reader |
| **status** | open |

**Evidence:** `tool_result` / `history` / `bash_execution` / `done` paths call `invalidateAll` + `refresh`/`layout` inline before re-arming `waitForEvent`. `rebuildBlocksFromHistory` (blocks.go L974+) walks entire session on UI thread. Reader uses blocking send into `coreEvents` buf 256 (main.go L476–490). A large tool_result render or `/load` can stall Update long enough to fill the channel → reader blocks → core stdout backpressure.

**Proposed fix sketch:** Keep backpressure, but narrow invalidate (R-006), coalesce refresh, and/or defer non-critical rebuilds. For history load, build off-thread and Send a ready msg (careful with model ownership).

**Repro hint:** `/load` a large session or finish a tool with huge output while deltas continue; measure Update hold time and `coreEvents` fill level.

---

## U-005 — `coreEvent.get` re-unmarshals `Raw` per field

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/protocol.go`: `(*coreEvent).get`, `rawKey` |
| **status** | open |

**Evidence:** `get` (L166–180) `json.Unmarshal`s the entire `Raw` map on every call, then unmarshals the value. `tool_result` calls `get("output")`, `get("id")`, `get("diff")`, `get("ok")` (handlers L386–402) → 4 full parses of a potentially large line. `subagent_progress` similarly multi-gets. Hot path tax on UI thread (amplifies U-004).

**Proposed fix sketch:** Parse once into `map[string]json.RawMessage` (or typed struct) per event; `get` reads the map. Special-case large fields via `rawKey` without stringifying.

**Repro hint:** Alloc profile on `encoding/json.Unmarshal` during a turn with large tool results.

---

## U-006 — Busy-turn triple cadence: spinner + stream refresh + tick refresh

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/main.go`: `spinner.TickMsg`, `tickMsg`, `streamRefreshMsg` |
| **status** | open |

**Evidence:** While `busy`: spinner ~10 FPS (L701–709) forces View; `streamRefreshMsg` refreshes viewport at 33/66/100ms (blocks.go L223–237); `tickMsg` also `refresh()` when `hasLiveContent()` (L683–684) for in-flight ◷ timers. Three independent redraw drivers. Stream coalesce is good; the 1s refresh duplicates work already covered by stream ticks when `cur != nil`, and spinner View already rebuilds chrome (R-001).

**Proposed fix sketch:** Drive in-flight badge elapsed from spinner/stream paints (pass `now` into render) and stop `tickMsg`→`refresh` while `streamRefreshPending` or spinner active. Keep tick only for toast expiry + spinner restart.

**Repro hint:** Long stream; count `refresh`/`SetContent` vs spinner ticks per second.

---

## U-007 — `tool_call` double refresh (`logTool` then `layout`)

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/blocks.go`: `logTool`; `tui/handlers.go`: `tool_call` |
| **status** | open |

**Evidence:** `logTool` always `refresh()` (blocks.go L316). For `todo_write` / `spawn` / `subagent`, handlers then `layout()` (L379–382) which `refresh()`es again (and may `invalidateAll` via R-002 when viewport height changes). Same pattern as scout `tool_result` double path (R-003).

**Proposed fix sketch:** `logTool` should not refresh when caller will `layout`/`refresh`; or return without paint and let one terminal refresh own the frame.

**Repro hint:** `todo_write` mid-turn; count `SetContent` per event (expect 2 today).

---

## U-008 — `goal_state` always `layout()` on every event

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `applyGoalState` |
| **status** | open |

**Evidence:** L137 `s.layout()` after every `goal_state` apply, including prompt status/summary churn during deploy. Cross-cuts R-002/R-005: height often unchanged → still full `refresh` (+ invalidate when H changes).

**Proposed fix sketch:** Compare prior vs new panel height budget / phase; `relayoutHeights` only on height change; transcript `refresh` only when persisted step cards added.

**Repro hint:** Long goal deploy; count `layout`/`invalidateAll` vs `goal_state` event rate.

---

## U-009 — Approval diff scroll / activity scroll call `layout()` per key

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `handleApprovalDiffScroll`, activity-expanded branch in `handleKey` |
| **status** | open |

**Evidence:** Approval expanded diff: every scroll key → `s.layout()` (L2076). Activity focus mode: ↑/↓/page/toggle → `layout()` (L1713–1729, L1802). Same class as R-004 (activity) — filing under update because key routing is the trigger. Scroll does not need transcript rebuild.

**Proposed fix sketch:** Mutate `diffScroll` / `activityScroll` only; let `View` rebuild chrome. No `layout`/`refresh` unless viewport W/H must change.

**Repro hint:** Expand approval diff or activity shelf; hold ↓; profile `SetContent`.

---

## U-010 — `history` / session load rebuilds entire transcript on UI thread

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `history`; `tui/blocks.go`: `rebuildBlocksFromHistory` |
| **status** | open |

**Evidence:** `rebuildBlocksFromHistory` (L974–1042) synchronously pushes every message/tool on the Update goroutine, then `invalidateAll` + `refresh`/`layout` (handlers ~539+). Large sessions freeze input/event pump (U-004).

**Proposed fix sketch:** Progressive/chunked rebuild with coalesced refresh; or build blocks in a cmd goroutine and swap in one msg (no concurrent mutation).

**Repro hint:** `/load` max-size session; measure time inside `rebuildBlocksFromHistory` on UI thread.

---

## U-011 — `finalizeInFlight` on `done`/`aborted` → `invalidateAll` then `layout`

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | update |
| **file:symbol** | `tui/blocks.go`: `finalizeInFlight`; `tui/handlers.go`: `done`, `aborted` |
| **status** | open |

**Evidence:** `finalizeInFlight` sets `invalidateAll` if any in-flight tool changed (blocks.go L800–801). Callers then `layout()` (handlers L459–497) → another refresh (+ possible second invalidate via R-002). End-of-turn always pays full cache rebuild even when only durations/notes on a few tool cards changed.

**Proposed fix sketch:** Narrow invalidate to touched tool blocks; single `relayoutHeights`+`refresh` at turn end.

**Repro hint:** Turn with many finalized tools + one in-flight; abort; count cache rebuilds.

---

## U-012 — Paste image detection runs synchronously on UI thread

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | update |
| **file:symbol** | `tui/main.go`: `tea.PasteMsg`; `tui/images.go`: `handlePasteContent` |
| **status** | open |

**Evidence:** `Update` calls `handlePasteContent` inline (main.go L851). Path sniffing / base64 decode / magic sniff (images.go L340+) runs on the UI goroutine. Large accidental base64 pastes stall Update (clipboard image keybind correctly uses async `readClipboardImageCmd`).

**Proposed fix sketch:** Size-gate: if paste > N KB and looks base64/binary, offload decode to a Cmd; keep fast path for short text/paths.

**Repro hint:** Paste a multi-MB base64 blob; observe UI freeze before reject/attach.

---

## U-013 — `handleKey` linear `kb()` scan per keypress

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `handleKey`; `tui/keybinds.go`: `kb` |
| **status** | open |

**Evidence:** Each keypress walks dozens of `s.kb` / `s.kbAny` checks (quit, modal, ask, sudo, newline, paste, activity, scroll×6, transcript, toggles, palette, …). Each `kb` does map lookup + `msg.String()` + normalize (keybinds.go L152–167). Fine at human typing rates; noisy when combined with other per-key `layout()` (U-009).

**Proposed fix sketch:** Optional: resolve `msg.String()` once; dispatch via action map for global binds. Low priority vs refresh storms.

**Repro hint:** Microbench `handleKey` with no-op key through full fallthrough.

---

## U-014 — `sudoTimeoutCmd` defined but never armed (dead timer path)

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | update |
| **file:symbol** | `tui/sudo.go`: `sudoTimeoutCmd`; `tui/main.go`: `sudoTimeoutMsg` |
| **status** | open |

**Evidence:** `Update` handles `sudoTimeoutMsg` (main.go L759–766) and `sudoTimeoutCmd` exists (sudo.go L22–25), but nothing calls `sudoTimeoutCmd` (handlers comment: intentionally no user-facing timeout). Countdown UI still depends on `tickMsg` while flyout open. Not a CPU leak; docs/dead-code drift like R-020.

**Proposed fix sketch:** Remove dead cmd/msg handler or arm the timer to match comments; don’t add a second always-on timer.

**Repro hint:** `rg sudoTimeoutCmd` — definition only.

---

## U-015 — Mouse wheel scroll is cheap (no issue); keyboard scroll OK

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | update |
| **file:symbol** | `tui/handlers.go`: `handleMouseWheel`, `handleScrollKey` |
| **status** | wontfix |

**Evidence:** `handleMouseWheel` (L1365–1380) and `handleScrollKey` (L2084–2119) only adjust viewport offset / `follow` — no `layout`/`refresh`. Good. Contrast with activity/approval scroll (U-009). Filing as wontfix documentation so fix agents don’t “optimize” this path.

**Proposed fix sketch:** None.

**Repro hint:** Wheel/pgup with mouse-wheel on; confirm no `SetContent`.

---

## Cross-refs already filed (render pass — do not duplicate)

| Update trigger | Existing ID |
|----------------|-------------|
| `layout()` invalidate on H change / 36 handler sites | R-002, R-005 |
| `tool_result` invalidateAll + layout | R-003, R-006 |
| Activity scroll layout | R-004 |
| `hasLiveContent` full scan | R-011 |
| Transcript focus invalidateAll | R-012 |
| Tick comment 500ms vs 1s | R-020 |
| Stream coalesce / idle spinner stop | already fixed (surface map) |

---

## Summary

### Render pass (prior)
| Severity | Count |
|----------|-------|
| P0 | 3 (R-001…R-003) |
| P1 | 7 (R-004…R-010) |
| P2 | 10 (R-011…R-020) |
| Subtotal | **20** |

### Update pass (this)
| Severity | Count |
|----------|-------|
| P0 | 1 (U-002) |
| P1 | 10 (U-001, U-003…U-011) |
| P2 | 3 open (U-012…U-014) + 1 wontfix (U-015) |
| **New findings this pass** | **15** (14 open + 1 wontfix) |

### Combined open (excluding wontfix)
| Severity | Count |
|----------|-------|
| P0 | 4 |
| P1 | 17 |
| P2 | 13 |
| **Total open** | **34** |

All new update findings **open** except U-015 **wontfix**. Audit-only; no code fixes.

### Suggested fix order (update-informed)
1. U-002 coalesce `subagent_progress` (P0)
2. R-001 + U-001/U-003/U-006 (stop idle/busy redundant View drivers)
3. R-002/R-005 + U-007/U-008/U-009 demote `layout`
4. U-004 parse-once + U-005/R-003/R-006 narrow invalidate
5. U-010 history off UI thread / chunked
6. P2 paste/keybind/docs

### Out of scope this pass
No code changes. Diff limited to `docs/tui-perf-findings.md`.

---

Audit pass: **data structures + I/O** (transcript retention, protocol parse, core stdout, mentions, images, session lock, settings persistence).
Scope: `tui/` only. **No code fixes in this pass.**
Deduped against R-* / U-* (protocol re-parse cost stays cross-referenced to U-005; history UI-thread rebuild to U-010; paste sync decode to U-012; cache.String copies to R-007; metrics footer re-unmarshal to R-014; tool args JSON re-parse to R-017).

---

## D-001 — Uncapped `block.args` / `block.diff` retention

| Field | Value |
|-------|-------|
| **severity** | P0 |
| **category** | data |
| **file:symbol** | `tui/blocks.go`: `logTool`, `logApproveDiff`, `capOutput`; `tui/handlers.go`: `tool_result`, `approval_request` |
| **status** | fixed |

**Fix (Batch A):** capStored (256 KiB) applied to args/diff at logTool, logApproveDiff, tool_result, approval_request, history rebuild.

**Evidence:** `maxStoredOutput` / `capOutput` (blocks.go L346–359) bound **only** `b.output` to 256 KiB. `b.args` is stored verbatim (`logTool` L313; history rebuild L1015). `match.diff` is stored verbatim (`tool_result` L401). `approval_request` copies the same uncapped args/diff into **both** `pendingApproval` and a lasting `blkApprove` (`handlers.go` L652–658 + `logApproveDiff` L367–369). Write/edit tool args and unified diffs routinely exceed output size; up to `maxBlocks=400` cards can pin multi-MB strings each for the session.

**Proposed fix sketch:** Apply the same `capOutput` (or shared `capStored`) to `args` and `diff` at ingest; keep a short preview for collapsed cards; optionally drop raw args after a compact summary is rendered. Clear `pendingApproval` fields on resolve (already cleared) — still leave capped block copy.

**Repro hint:** Agent `write_file` of a large source file under approval; inspect `len(b.args)` / `len(b.diff)` on the tool/approve blocks after dismiss.

---

## D-002 — Assistant/thinking `strings.Builder` text unbounded per turn

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | data |
| **file:symbol** | `tui/blocks.go`: `block.text`; `tui/handlers.go`: `delta` / `thinking` |
| **status** | open |

**Evidence:** Streaming deltas append into `b.text` with no byte cap (unlike tool output). A long reply retains the full Builder contents for the life of the block (up to trim at 400 blocks) **and** a separate `renderStr` / markdown-derived cache after finalize. Peak session RSS scales with sum of reply sizes, not viewport.

**Proposed fix sketch:** Soft-cap stored text (e.g. same 256 KiB or a higher stream cap) with a truncated marker; or drop raw text once a finalized width-keyed render cache exists and only keep text when expanded/copy-focused.

**Repro hint:** Stream a multi-MB markdown answer; measure `b.text.Len()` after `done` vs rendered viewport size.

---

## D-003 — `history` load peak: full messages JSON + rebuilt blocks

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | data |
| **file:symbol** | `tui/handlers.go`: `history`; `tui/blocks.go`: `rebuildBlocksFromHistory` |
| **status** | open |

**Evidence:** `history` unmarshals `messages` into `[]map[string]json.RawMessage` (handlers L527–532) then `rebuildBlocksFromHistory` copies every user/assistant/tool string into new blocks (L974–1040). Peak memory ≈ JSON graph + block strings simultaneously on the UI goroutine. Cross-cuts U-010 (stall); this finding is the **retention/peak** aspect.

**Proposed fix sketch:** Stream/chunk rebuild; release `msgs` early (`msgs = nil` after each push batch); or rebuild off-thread and swap. Prefer not holding RawMessage maps after content extraction.

**Repro hint:** `/load` a large session; heap profile during `rebuildBlocksFromHistory`.

---

## D-004 — Core stdout line: double full JSON parse before handlers

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | io |
| **file:symbol** | `tui/main.go`: stdout reader goroutine; `tui/protocol.go`: `(*coreEvent).get` |
| **status** | open |

**Evidence:** Reader (main.go L468–474) `json.Unmarshal`s each line into `Raw`, then calls `ev.get("type")` which **re-Unmarshals the entire Raw map** (protocol.go L166–180). Large `tool_result` lines pay 2× decode in the reader alone; handlers then call `get` repeatedly (U-005) for more full parses. Channel buffer 256 (L438) + blocking send is correct for backpressure (keep).

**Proposed fix sketch:** Parse once into `map[string]json.RawMessage` (or typed envelope with `type` + residual) at read time; store the map on `coreEvent`; `get`/`rawKey` index it. Avoid stringifying large fields.

**Repro hint:** Alloc profile on `encoding/json.Unmarshal` during a turn with multi-100KB tool results; count Unmarshals per line.

---

## D-005 — `skills` event retains full SKILL.md `Content` bodies

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | data |
| **file:symbol** | `tui/handlers.go`: `skills`; `tui/protocol.go`: `skillInfo` |
| **status** | open |

**Evidence:** Comment at handlers L1221–1223 says the TUI only needs name/description for the palette, but unmarshals into `[]skillInfo` including `Content string` (protocol.go L153–158) and stores `s.skillsList = skills` (L1231). Skill bodies can be large and multiply with installed skills for the whole process lifetime.

**Proposed fix sketch:** Unmarshal into a slim struct (`Name`, `Description`, `Location` only), or `json.Unmarshal` with a type that has `Content json.RawMessage \`json:"-"\`` / omit the field.

**Repro hint:** Install many large skills; compare `skills` event payload size vs retained `skillsList` heap.

---

## D-006 — Mention cache TTL forces repeated git/walk I/O while typing `@`

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | io |
| **file:symbol** | `tui/mention.go`: `mentionCacheTTL`, `recursiveSearchWithRefresh`, `gitMentionList`, `fillMentionCache` |
| **status** | open |

**Evidence:** `mentionCacheTTL = 2 * time.Second` (L297). While the flyout is open, keystrokes call `evalMention` → `recursiveSearchWithRefresh`; every >2s stale window starts another background `git ls-files … -z` (L448–450) or `WalkDir` (up to 40k visits). `gitMentionList` does `strings.Split(string(out), "\x00")` (L469) materializing the **entire** git stdout before the 10k item cap stops appending. Walk itself is off UI thread (good); I/O + alloc churn is still high on large repos.

**Proposed fix sketch:** Raise TTL (30–120s) or invalidate on cwd/fsnotify; stream git output without full `string(out)`; stop reading git stdout once `mentionCacheCap` reached.

**Repro hint:** Large git repo; hold `@` query open and type for >2s; watch `git ls-files` process spawns / CPU.

---

## D-007 — Send path: sync file read + base64 data-URL materialization on UI thread

| Field | Value |
|-------|-------|
| **severity** | P1 |
| **category** | io |
| **file:symbol** | `tui/images.go`: `materializeOwnedImage`, `withImages`; caps `maxAttachImageBytes` / `maxTotalAttachBytes` |
| **status** | open |

**Evidence:** Caps exist (20 MiB/image, 40 MiB total — L26–27) but `materializeOwnedImage` (L506–527) `os.ReadFile`s + `base64.StdEncoding.EncodeToString` into a data URL on the caller’s goroutine during send (`withImages`). That is Update-thread CPU + multi-10MB string allocs before `json.Marshal` to core. Clipboard keybind is async (`readClipboardImageCmd`); this path is not. Cross-cuts U-012 (paste decode); this is the **send** side.

**Proposed fix sketch:** Prefer sending filesystem paths the core can read; if data URL required, encode in a `tea.Cmd` and Send a ready payload. Or stream base64 without holding both raw + encoded.

**Repro hint:** Paste a ~15 MiB PNG then Enter; measure stall before stdin write.

---

## D-008 — `evalMention` linear scan of up to 10k paths per keystroke

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | data |
| **file:symbol** | `tui/mention.go`: `evalMention`, `recursiveSearchWithRefresh` |
| **status** | open |

**Evidence:** Every input mutation with an active `@` token (handlers fallthrough → `evalMention`) builds `[]rune(input)`, then filters `mentionCache.list` with `strings.ToLower(it.insert)` per entry until 12 hits (L355–363). Cache miss path is async; **hit** path is O(n) on UI thread each key. No prefix index.

**Proposed fix sketch:** Keep cache entries pre-lowercased; optional simple prefix bucket / sort + binary search; debounce filter if list is huge.

**Repro hint:** Warm cache at 10k entries; type inside `@query`; microbench `recursiveSearchWithRefresh`.

---

## D-009 — `pushHistory` trim retains backing-array prefix

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | data |
| **file:symbol** | `tui/extras.go`: `pushHistory` |
| **status** | open |

**Evidence:** `historyMax=100` (L18) but trim is `s.history = s.history[len(s.history)-historyMax:]` (L26–27) without copy — same class of leak fixed for `blocks` (memory-leak-audit / blocks.go L98–106). Dropped prompt strings stay reachable via the old array prefix until a rare capacity growth reallocates.

**Proposed fix sketch:** Copy into a fresh `[]string` of capacity `historyMax` (mirror blocks trim).

**Repro hint:** Push >100 large prompts; heap dump reachable strings from `history` backing store.

---

## D-010 — `settings.save` synchronous disk RMW on UI thread

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | io |
| **file:symbol** | `tui/settings.go`: `(*settingsStore).save`; call sites e.g. `handlers.go` toggle_reasoning L1760 |
| **status** | open |

**Evidence:** `save` (L400–493) does `ReadFile` → merge map → `MarshalIndent` → `CreateTemp` → `Write` → `Sync` → `Chmod` → `Rename` inline. Triggered from Update (think toggle, modals, provider key changes). Not every keystroke (good), but a slow disk stalls the event pump. Not on the spinner hot loop.

**Proposed fix sketch:** Debounce/coalesce saves in a background goroutine with generation token; keep sync only for quit/logout.

**Repro hint:** Toggle reasoning on a busy NFS/home disk; observe Update stall.

---

## D-011 — `lastMetrics` retains full event Raw for process life

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | data |
| **file:symbol** | `tui/handlers.go`: `metrics`; `tui/main.go`: `lastMetrics` |
| **status** | open |

**Evidence:** `s.lastMetrics = ev.Raw` (handlers ≈L613) keeps the entire metrics JSON line. Footer re-unmarshals it every paint (R-014). Size is usually small; still unnecessary retention + parse tax.

**Proposed fix sketch:** Parse once into typed floats/strings on the metrics event; drop Raw.

**Repro hint:** Same as R-014; confirm Raw length vs used fields.

---

## D-012 — Session lock path is cold (not a hot-loop issue)

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | io |
| **file:symbol** | `tui/settings.go`: `claimSession`, `sessionLockedByAnotherProcess`; `session_lock_*.go` |
| **status** | wontfix |

**Evidence:** Lock files use `O_CREATE|O_EXCL`, stale-PID checks, and `Stat` only on session claim/switch/picker — not per keystroke or per frame. Correct place for I/O. Filing as wontfix so fix agents do not “optimize” this path.

**Proposed fix sketch:** None.

**Repro hint:** `rg claimSession|sessionLocked` — no Update/tick callers.

---

## D-013 — Already-bounded structures (document only)

| Field | Value |
|-------|-------|
| **severity** | P2 |
| **category** | data |
| **file:symbol** | `maxBlocks`, `historyMax`, `capOutput`, `mentionCacheCap`, image caps, single-slot `queued` |
| **status** | wontfix |

**Evidence:** Transcript cards capped at 400 with copy-trim (blocks.go L79–106); prompt recall 100; tool **output** 256 KiB; mention list 10k; pending images 8 / 40 MiB total; follow-up queue is a single `*queuedMsg` (handlers refuse when full). These are working guards — do not remove. Residual risk is uncapped args/diff/text (D-001/D-002), not missing caps on these axes.

**Proposed fix sketch:** None (keep caps; extend pattern to args/diff/text).

**Repro hint:** Read constants; `Test*` for maxBlocks trim.

---

## Cross-refs already filed (do not duplicate as new primary IDs)

| Data/IO concern | Existing ID |
|-----------------|-------------|
| `coreEvent.get` re-Unmarshal per field | U-005 |
| History rebuild stalls Update | U-010 |
| Paste base64 decode on UI thread | U-012 |
| `cache.String()` / line-offset copies | R-007 |
| Footer metrics Unmarshal every paint | R-014 |
| Tool renderer `arg()` Unmarshal per field | R-017 |
| `renderBlocks` retention of full cache string | R-007 + maxBlocks |
| Blocks prefix slice leak | fixed (memory-leak-audit) |

---

## Summary (data/io pass)

| Severity | Count |
|----------|-------|
| P0 | 1 (D-001) |
| P1 | 6 (D-002…D-007) |
| P2 open | 4 (D-008…D-011) |
| P2 wontfix | 2 (D-012, D-013) |
| **New findings this pass** | **13** (11 open + 2 wontfix) |

### Combined open after this append (excluding all wontfix)
| Severity | Count |
|----------|-------|
| P0 | 5 (R-001…R-003, U-002, D-001) |
| P1 | 23 |
| P2 | 17 |
| **Total open** | **45** |

**New finding count this pass:** 13  
**P0s remain unlisted?** No — D-001 is the only new P0 from data/io; prior P0s (R-001…R-003, U-002) already listed. No additional unlisted P0s found in transcript retention, protocol/stdout, mentions, images, session lock, or settings hot paths.

### Suggested fix order (data/io-informed)
1. D-001 cap args/diff (and approval duplicate)
2. D-004 parse-once at stdout (feeds U-005)
3. D-002 soft-cap stream text / drop raw after cache
4. D-005 slim skills; D-006 mention TTL + stream git
5. D-007 async materialize; D-003/U-010 history peak
6. P2 history copy-trim, mention index, deferred settings save, typed metrics

### Out of scope this pass
No code changes. Diff limited to `docs/tui-perf-findings.md`.
