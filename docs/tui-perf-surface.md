# TUI performance surface map

Read-only scout artifact for the TUI performance audit/fix goal.
Scope: Go Bubble Tea v2 TUI under `tui/`. **No code changes in this pass.**

Workspace: `catalystctl/catcode` · branch context at scout time: `master`.

---

## 1. Event / render loop

### Entry points

| Hook | File:symbol | Role |
|------|-------------|------|
| `Init` | `tui/main.go` `(*session).Init` | `tea.Batch(startCore(), tick(), spinner.Tick)` |
| `Update` | `tui/main.go` `(*session).Update` | Central switch: window size, timers, core events, keys, paste |
| `View` | `tui/render.go` `(*session).View` → `tea.View` | Always calls `relayoutHeights()` when ready, then chrome + viewport |
| Program | `tui/main.go` `main` | `tea.NewProgram(initialSession())` then `prog.Run()` |

Core I/O path:

```
core stdout JSONL
  → reader goroutine (startCore) → coreEvents chan (buf 256)
  → waitForEvent Cmd → coreEventMsg
  → handlers.go handleCoreEvent / switch on type
  → scheduleStreamRefresh / refresh / layout / invalidateAll
  → viewport.SetContent(renderBlocks())
  → View → cursed (incremental) renderer
```

Stdin path: `sendCore` → non-blocking `stdinCh` (buf 256) → writer goroutine → core stdin. Drop+log on full buffer (never block Update).

### Who triggers redraws

Any `Update` return with a model change causes View. Hot message types:

| Msg | Cadence / source | Side effect |
|-----|------------------|-------------|
| `spinner.TickMsg` | ~10 FPS while `busy \|\| !ready` (`spinner.Dot`, `time.Second/10`) | Spinner frame + View (animated border piggybacks) |
| `tickMsg` | `tea.Tick(time.Second)` via `tick()` — **1s**, comment wrongly says 500ms | `hasLiveContent()` → `refresh()`; toast expiry; restart spinner if needed |
| `streamRefreshMsg` | Coalesced 33ms / 66ms / 100ms by `len(blocks)` | `refresh()` after delta/thinking batches |
| `tea.WindowSizeMsg` | Resize | `layout()` (heights + invalidate + refresh) |
| `coreEventMsg` | Core JSONL | handlers → layout/refresh/invalidate |
| Key / Paste / Mouse | User | input/modal; some paths `refresh`/`invalidateAll` |
| `updateAvailableMsg`, `sudoTimeoutMsg`, … | Async | occasional `layout()` |

**Idle invariant:** when `!busy && ready`, spinner stops (`spinnerActive=false`). Idle pty output must stay ~0 bytes so mouse copy works. Do not add always-on timers.

### Timers / Batch inventory

- `Init`: `tea.Batch(startCore, tick, spinner.Tick)`
- `tick()` → `tickMsg` every **1s** (`main.go` ~602–603)
- `scheduleStreamRefresh` (`blocks.go` ~223–237): `tea.Batch(waitForEvent, tea.Tick(frameDelay → streamRefreshMsg))` with `frameDelay` 33/66/100ms
- `sudo.go`: `tea.Tick(sudoAutoClose, …)` for sudo auto-decline
- `startCore`: `tea.Tick(coreStartupTimeout, readyTimeoutMsg)`

Animation policy (memory `tui-animation-infrastructure`): animate **color only**; wall-clock phase; reuse spinner cadence; `CATCODE_ANIMATED_BORDER` + `!prefersReducedMotion()` gate for comet border (`renderInputBox`).

---

## 2. Hottest View / render call graph

Every ready frame (`View`):

```
View()
  relayoutHeights()                         // height math — but helpers RENDER to measure
    headerHeight()           → renderHeader()
    positionBarHeight()      → renderPositionBar()   (cheap: empty→0 else 1)
    footerHeight()
      inputBoxHeight()       → renderInputBox()      // may be renderInputBoxAnimated
      renderFooter()         → lipgloss.Height
    mentionFlyoutHeight()    → renderMentionFlyout()
    activityShelfHeight()    → renderActivityShelf()
    oauthBannerHeight()      // structural 0/1
    goalProgressPanelHeight()→ renderGoalProgressPanel()
    → viewport.SetWidth/SetHeight, input.SetWidth
  parts:
    renderHeader()                              // AGAIN
    renderCoreFailureBanner? / renderUpdateBanner? / renderOauthBanner?
    viewport.View()                             // already-built SetContent string
    renderPositionBar?
    renderActivityShelf?                        // AGAIN if non-empty
    renderGoalProgressPanel?                    // AGAIN
    renderMentionFlyout?                        // AGAIN
    renderInputBox()                            // AGAIN (animated when busy+env)
    renderFooter()                              // AGAIN
    [modal] renderModalOverlay
    renderAskOverlay / renderSudoOverlay
  constrainViewContent(full, W, H)              // MaxWidth+MaxHeight over entire screen
  tea.NewView(content) + AltScreen/MouseMode
```

**Double-build cost:** while busy (~10 FPS spinner), `renderInputBox` / panels used for height are built once in `relayoutHeights` and again for paint. Animated path rebuilds a 32-level lipgloss ramp every call (`renderInputBoxAnimated`).

Transcript rebuild (not every View — only on `refresh`/`layout`):

```
refresh()
  viewport.SetContent(renderBlocks())
    cache walk: for cacheIdx… finalized blocks
      renderedLineOffset(cache.String())   // full string copy + Count \n
      renderBlock → renderBlockFull → renderMarkdown / renderToolBlock
    cache.String() copy into Builder
    live cur + in-flight tools re-rendered
```

Streaming: `renderBlock` throttles live assistant/thinking to `streamBatch=64` bytes between full re-renders; each actual render still parses **full** markdown (`markdown.go` `renderMarkdown`).

Tool cards: `blocks.go` `renderToolBlock` → `tool_blocks.go` per-name dispatch; bodies use `renderOutputPanel` / diff panels (truncate unless expanded).

---

## 3. Largest state holders

| State | Location | Bound / notes |
|-------|----------|---------------|
| `blocks []*block` | `session` | `maxBlocks=400`; trim copies into fresh slice (+ `blkTrimmed` marker) |
| Per-block `text` (`strings.Builder`), `args`, `output`, `diff`, `renderStr` | `block` | `output` capped `maxStoredOutput=256KiB` |
| `cache strings.Builder` + `cacheIdx` | finalized transcript render | ~3× transcript size while warm; `invalidateAll` resets |
| `viewport` content string | bubbles viewport | Set by `refresh` |
| `history []string` | `extras.go` | `historyMax=100` |
| `subProgress []*subProgressEntry` | live scouts | copy-trim on remove; drives activity shelf |
| `todos` | latest `todo_write` | pinned panel (capped display `maxPinnedTodos=5`) |
| `composerDrafts` | input ownership stack | small |
| `modal` / ask / sudo / approval / intercom | overlays | infrequent but heavy render when open |
| `coreEvents` / `stdinCh` | chans buf **256** | backpressure: stdin drop; stdout blocks (except final EOF line) |

`invalidateAll` (`blocks.go`): resets cache + clears every block’s `renderStr` / line ranges — O(n blocks).

---

## 4. Known gotchas to honor

### Bubble Tea v2

- `View() tea.View` (not `string`); content via `tea.NewView`; `.Content` in tests.
- Keys: `tea.KeyPressMsg` + `msg.String()`; paste: `tea.PasteMsg`.
- Mouse: `tea.MouseMsg` interface; wheel is `tea.MouseWheelMsg`.
- Alt-screen / mouse: fields on `tea.View`, not program options.
- Cursed renderer: view height must be **≤ terminal−1** (slack line in `relayoutHeights`). Exact fill causes scroll/cursor drift.
- Lipgloss v2: `Width`/`Height` are methods.

### Send-before-Run

- `update.go` `launchUpdateCheck`: **must** `go prog.Send(...)` when cache hits before `Run()` — sync Send deadlocks (Bubble Tea v2).
- Signal path already uses goroutine + `prog.Send(sigtermMsg{})`.

### Core subprocess backpressure

- Event channel blocking send preserves events; reader drops only last line on error+EOF path.
- `sendCore` never blocks UI; overflow → drop + error toast.
- Always re-arm `waitForEvent` after handling (handlers comments ~816+) or the pump dies.

### Animation / idle

- Spinner only while `busy || !ready`; idle = zero re-render.
- Prefer spinner cadence over new `tea.Tick` for visuals.
- `tickMsg` comment says 500ms but code is **1s** — fix comment when touching, don’t silently change cadence without measuring copy/CPU.

### Already fixed (do not redo)

- Idle spinner stop (copy storm).
- `relayoutHeights` in `View` (overflow from missed mutation sites).
- `maxBlocks` + copy-trim (RSS prefix retention).
- Stream coalesce + `streamBatch` throttle.
- `hasLiveContent`-gated tick refresh.
- stdin writer channel (UI freeze).

---

## 5. Suggested audit order (by risk)

### P0 — every busy frame / layout storm

1. **Render-to-measure double work** — `render.go` `relayoutHeights` + `*Height` helpers vs `View` paint. Cache structural heights or last-render heights; stop calling full `renderInputBox`/`renderActivityShelf`/`renderGoalProgressPanel`/`renderMentionFlyout`/`renderHeader` twice per frame.
2. **`layout()` overuse** — `handlers.go` has **36** `s.layout()` call sites (todos, scouts, banners, goal, queue, approvals, …). `layout` = `relayoutHeights` + **invalidateAll when viewport W/H changes** + `refresh`. Panel grow/shrink changes viewport height → full transcript re-wrap. Demote to `relayoutHeights()` + `refresh()` when wrap **width** unchanged; keep full `layout()` for true resize / width change.
3. **`tool_result` → `invalidateAll()`** — `handlers.go` ~407: one tool finish clears entire finalized cache. Narrow to invalidate that block / from `cacheIdx` of match, or re-cache only the finished tool.

### P1 — streaming & refresh cost

4. **`renderBlocks` string copies** — `s.cache.String()` + `renderedLineOffset(s.cache.String())` every extension; track `cacheLines` incrementally.
5. **Streaming markdown O(n) per batch** — `renderBlockFull` → `renderMarkdown(full)` every ≥64 bytes → ~O(n²/64). Incremental wrap/line cache (ponytail in `blocks.go`).
6. **`constrainViewContent`** — `MaxWidth`/`MaxHeight` over full screen every View; prefer structural clamp or cheaper truncate.
7. **`renderInputBoxAnimated` ramp** — rebuild 32 `lipgloss.Style`s every paint; theme-cache ramp keyed by dim/accent.

### P2 — secondary / polish

8. **`hasLiveContent`** — when `cur == nil`, scans all blocks each 1s tick; maintain `inFlightCount` or check `subProgress`/tail only.
9. **`transcript_nav` focus** — `invalidateAll` on every Alt+Up/Down (`transcript_nav.go` ~51); prefer local focus decoration without full cache drop.
10. **Modal/ask overlays** — `modal.go` `renderModalOverlay`, `ask.go` `renderAskOverlay`: measure only when open; ensure they don’t force transcript invalidate.
11. **Comment drift** — tick 500ms vs 1s; keep docs/tests aligned.

### Validation hooks (for fix steps)

- Idle: pty byte count over ~2s → **0**.
- Busy long stream: CPU + `SetContent`/`refresh` rate; spinner stays ~10 FPS.
- Many tools: no full-cache rebuild per `tool_result` if narrowed.
- `go test ./tui/...`
- Diff limited to `tui/` (+ tests).

---

## File / symbol index (perf-relevant)

| File | Symbols to audit |
|------|------------------|
| `tui/main.go` | `session`, `Init`, `Update`, `tick`, `waitForEvent`, `sendCore`, `startCore` |
| `tui/render.go` | `relayoutHeights`, `layout`, `View`, `constrainViewContent`, `renderInputBox*`, `*Height` helpers, chrome renderers |
| `tui/blocks.go` | `push`, `invalidateAll`, `renderBlocks`, `scheduleStreamRefresh`, `refresh`, `renderBlock`, `renderBlockFull`, `hasLiveContent`, `maxBlocks`, `streamBatch` |
| `tui/markdown.go` | `renderMarkdown`, `renderMarkdownLine` |
| `tui/handlers.go` | delta/thinking → `scheduleStreamRefresh`; `tool_*` → `invalidateAll`/`layout`; ~36 `layout()` |
| `tui/tool_blocks.go` | per-tool body renderers |
| `tui/goal_ux.go` | `renderGoalProgressPanel`, `goalProgressPanelHeight` |
| `tui/mention.go` | `renderMentionFlyout`, `mentionFlyoutHeight` |
| `tui/transcript_nav.go` | `moveTranscriptFocus` + invalidate |
| `tui/modal.go` / `ask.go` / `sudo.go` | overlays |
| `tui/keybinds.go` | dispatch (not hot CPU; don’t add per-frame work) |
| `tui/update.go` | `launchUpdateCheck` Send-before-Run |
| `tui/extras.go` | `historyMax` |
| `tui/protocol.go` | `tickMsg`, `subProgressEntry`, event types |

---

## Handoff (next scout / fix agent)

1. **Start:** `tui/render.go` — replace render-to-measure in `relayoutHeights` / `*Height` with cached or structural heights so busy frames stop double-building animated input + panels. Then re-measure View CPU.
2. **Next:** classify each of the 36 `handlers.go` `layout()` sites: resize-width vs height-only → demote height-only to `relayoutHeights`+`refresh`.
3. **Next:** narrow `tool_result` `invalidateAll`; fix `renderBlocks` line-offset / `String()` copies; theme-cache animated ramp.
4. **Track findings** in `docs/tui-perf-findings.md` (severity, file:symbol, status fixed/wontfix/deferred).
5. **Re-audit** until zero new P0/P1 (or documented wontfix); run `go test ./tui/...`; preserve BT v2 + idle-zero-redraw invariants.
6. **Do not** re-fix already-landed spinner/idle, maxBlocks copy-trim, stream coalesce, or View-side `relayoutHeights` unless regressions appear.

Meta-prompt for implementer:

> You are fixing Go TUI perf in `tui/` (Bubble Tea v2). Read `docs/tui-perf-surface.md`. Prefer height caches over render-to-measure; demote `layout()` when wrap width unchanged; narrow `invalidateAll` on tool_result. Keep idle redraw at zero; no new always-on timers; View returns `tea.View`; never sync `prog.Send` before `Run`. Append every issue to `docs/tui-perf-findings.md`. Tests: `go test ./tui/...`. Diff only under `tui/`.
