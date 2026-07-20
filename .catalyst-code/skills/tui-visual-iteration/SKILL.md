---
name: tui-visual-iteration
description: Visually iterate on the Go TUI by rendering realistic frames and screenshotting them — so layout/color/aesthetic changes are judged by EYES, not just ANSI codes
version: 1
---

## When to use

You are redesigning or restyling the Go TUI (`tui/`) and need to SEE the result — a "redesign" judged only by reading Go source or raw ANSI escapes is guesswork. Use this to render real frames of any state (conversation, welcome, approval, busy, error) and screenshot them. Pair with the derived-tone + card/rail/status-dot conventions in the `web-design-system-catalyst-obsidian` memory.

The existing `add-tui-tool-renderer` skill covers adding a per-tool *block* renderer; THIS skill is for iterating on the overall look (chrome, layout, spacing, color) — a different job.

## The pattern (throwaway, never committed)

1. **Render harness** — create `tui/hack_visual.go` (package `main`, so it's compiled but you `rm` it when done). An `init()` checks `os.Args[1]=="__viz"` and renders a scene to stdout, then `os.Exit(0)`. Build realistic sessions by reusing test-style constructors:
   - `s := initialSession()`; set `s.ready=true`, `s.coreLifecycle=coreReady`, `s.width/s.height`, `s.authed=true`, `s.models=[...]`, `s.modelIdx=0`, `s.cwd`.
   - Populate the transcript with `s.logUser("…")`, `b := s.push(blkAssistant); b.text.WriteString("…markdown incl ```go fence```")`, `s.cur=nil`, tool blocks via `t := s.push(blkTool); t.name=…; t.ok=true; t.hasOk=true; t.dur=…`.
   - Call `s.layout()` then print `s.View().Content`. Support a `plain` arg that prints `stripANSI(out)` for geometry inspection.
   - Run: `cd tui && go build -o /tmp/catviz . && /tmp/catviz __viz conv 84 26 plain`.
2. **ANSI→HTML converter** (`/tmp/ansi2html.py`) — parse SGR: `1`=bold, `3`=italic, `38;2;r;g;b`=fg, `48;2;r;g;b`=bg, `38;5;n`=xterm-256 fg, `0`=reset. Wrap spans; page bg `#1a1a1a`, JetBrains Mono, `white-space:pre`.
3. **Screenshot** with the user's cached Playwright headless chromium:
   `chromium.launch({executablePath: ~/.cache/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell})` (module path `/tmp/node_modules/playwright-core`; `npm i playwright-core` if absent). `page.goto('file:///tmp/tui_<scene>.html')`, then `el.screenshot()` on the `#tui` element.
4. **Audit color precisely** — grep the ANSI output for `(38|48);2;r;g;b` triples to confirm each surface hits the exact Catalyst token (accent `207;138;89`, success `59;222;119`, card `36;36;36`=#242424, railDim `65;65;65`). This catches clashes (e.g. amber inline-code vs the warn banner) that plain-text views hide.

## Gotchas

- **Delete `tui/hack_visual.go` before finishing** (and the /tmp artifacts) — it's a throwaway. `grep -rn __viz tui/*.go` should return nothing.
- **Tests pin structure, not just colors.** `visual_golden_test.go` snapshots plain-text layout via `canonicalVisual` (collapses `─{3,}` runs to a single `─` and strips ANSI). Any structural change (new rails, `● you ──` ledger rules, unboxed composer) requires updating the `want` strings. Mouse drag-select tests shift +N columns when you add an N-cell left rail. `render_smoke_test.go` asserts role text (e.g. the assistant model id, "you", "activity").
- **Unicode box-drawing chars (`─│╮▮●❯`) corrupt the `edit` tool's search/replace and Python heredocs.** Read the exact lines with `read_file`/`sed -n` first and copy them verbatim, or use a Python script with clean ASCII anchors (find a unique ASCII line, splice around it). Never retype these glyphs into an edit search.
- **Keep the busy-comet geometry invariant** (tui-animation-infrastructure memory): the animated composer and the static composer must produce the SAME line count and per-cell width — only colors animate. If you unbox the composer, rewrite the comet to sweep the hairline, not the old box perimeter.
- **Run tests serially**: `go test -p 1 ./...`. Parallel runs flake on env-var races, and in this workspace up to ~6 concurrent agent sessions edit the same tree — a parallel `go test ./...` can report failures that a clean serial run disproves. Re-read any file a concurrent session may have touched before editing it.
