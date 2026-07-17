# TUI perf fix — Batch A (P0)

Implements all five Batch A items from `docs/tui-perf-fix-plan.md`.

## Changes

| ID | What | Files |
|----|------|-------|
| **R-001** | Per-`View` chrome string cache (`viewChromeCache`) so `relayoutHeights` measure-by-render does not double-build header/footer/input/shelf/goal/mention/position bar (animated input once per busy frame) | `tui/main.go`, `tui/render.go`, `tui/goal_ux.go`, `tui/mention.go` |
| **R-002** | `layout()` invalidates wrap cache **only** when `viewport.Width()` changes; height-only panel toggles no longer `invalidateAll` | `tui/render.go` |
| **R-003** | `tool_result`: one path — `invalidateAll` → `relayoutHeights` (scout only) → single `refresh()`; removed refresh-then-`layout()` | `tui/handlers.go` |
| **U-002** | `subagent_progress`: `relayoutHeights` on entry add/remove; phase ticks no longer `refresh()` / `SetContent` (shelf paints via normal View) | `tui/handlers.go` |
| **D-001** | Shared `capStored` (256 KiB) for `args`/`diff` at ingest (`logTool`, `logApproveDiff`, `tool_result`, `approval_request`, history rebuild); `capOutput` aliases it | `tui/blocks.go`, `tui/handlers.go`, `tui/cap_stored_test.go` |

## Findings

Marked **fixed** in `docs/tui-perf-findings.md`: R-001, R-002, R-003, U-002, D-001.

## Validation

```bash
go test ./tui/...
```

(Plus targeted: `TestCapStoredArgsAndDiff`, visual goldens / render smoke as in the plan.)

## Risks / follow-ups

- Narrow `invalidateAll` on `tool_result` remains Batch B1 (R-006).
- Chrome cache is View-scoped only; handler-path height helpers still render once (no double within View).
- Expanded tool/approval UI may show truncated args/diff with `…[truncated]` marker — intentional.
