---
name: add-tui-tool-renderer
description: Add a per-tool block renderer to the Go TUI (tui/tool_blocks.go dispatcher + render helper) so a new tool surfaces its key info instead of a raw JSON blob
version: 1
---

## When to use

You added (or are adding) a new built-in tool to the Rust core (see the
`add-core-tool` skill) and it will appear often enough in the chat that the
default generic rendering — `renderGenericToolBlock` (name + raw args blob +
collapsed output) — is too noisy or hides the one thing that matters. Every
tool the human sees often (`bash`, `read_file`, `edit`, `grep`, `todo_write`,
`git_*`, `fetch`, `memory`, …) has a dedicated renderer in `tui/tool_blocks.go`
added by repeating this exact shape. Skip it for rarely-seen / internal tools
(the generic fallback is fine and always works).

## Where things live

- **Dispatcher** — `tui/blocks.go` `(s *session) renderToolBlock(b *block, w int)`:
  a `switch b.name { case "<name>": return render<Name>Block(b, w) … }`.
  `b.sub` (sub-agent *internal* calls) are collapsed BEFORE the switch to a dim
  one-liner via `renderSubToolLine`, so you only render the top-level call.
  `default` → `renderGenericToolBlock`.
- **Renderers** — `tui/tool_blocks.go` (~960 lines). Signature is always
  `func render<Name>Block(b *block, w int) string`. Returns the whole block
  (head + body) as one string.
- **Shared helpers** (same file / blocks.go) — reuse these, do not hand-roll boxes:
  - `renderToolHead(b, w, segs)` → builds the header line (icon · name · keyarg … dur status). Returns `(head string, failed bool)`.
  - `renderOutputPanel(output, expanded, w, unit, err)` → collapses to the first 3 lines unless expanded; appends a dim `… +N lines (ctrl+o expand)` hint. `unit` = "lines"/"matches"/"entries". `err` tints the rule red.
  - `renderNumberedOutput(output, startLine, expanded, w, err)` → same but with a right-aligned line-number gutter (for `read_file`).
  - `renderRowsPanel` / `renderDiffPanel(b.diff, w)` (for edit/patch/write_file).
  - `keyArgFor(name, args)` / `toolKeyArg(b)` → the single most relevant arg (path/command/pattern/url) for the head + the collapsed one-liner.
- **Block fields** — `b.args` is a RAW JSON STRING (not a parsed map): use `b.arg(key)`, `argField(args,key)`, `b.argObjArr(key)` (array of objects, e.g. todos), `b.argStrArr(key)` (string array, e.g. git_add paths). `b.dur` / `b.started` / `b.inFlight()` (in-flight = awaiting result); `b.expanded` (ctrl+o, per-block). The ✓/✗/◷ status + failure tint come from the `tool_result.ok` captured in `handlers.go`.

## House style

A single left `│` rule, NO boxed cards. Surface the one thing that matters
(the bash command, the file path, the grep pattern, the todo list). Failures
tint the body's rule red so they're scannable while scrolling.

## Steps

1. **Add the dispatch arm** in `tui/blocks.go` `renderToolBlock`:
   `case "<name>": return render<Name>Block(b, w)` (before `default`).
2. **Write the renderer** in `tui/tool_blocks.go`:
   ```go
   func render<Name>Block(b *block, w int) string {
       head, failed := renderToolHead(b, w, joinSegs(truncate(b.arg("path"), w-16)))
       var out strings.Builder
       out.WriteString(head)
       if !b.inFlight() {
           out.WriteString("\n" + renderOutputPanel(b.output, b.expanded, w, "lines", failed))
       }
       return out.String()
   }
   ```
   Swap `renderOutputPanel` for `renderNumberedOutput` / `renderRowsPanel` /
   `renderDiffPanel` as the tool's output dictates. For a diff-producing tool,
   render `renderDiffPanel(b.diff, w)` when `b.diff != ""`.
3. **Pick the key arg** for the head via `b.arg(...)` (the path / command /
   pattern / url) — this is what shows in the collapsed view and the approval
   banner. Add a `keyArgFor` case if you want it in the sub-agent one-liner too.
4. **Verify** — `cd tui && go vet ./... && go test ./... && go build .`.
   Add a render smoke test next to the existing `render_smoke_test.go` /
   `tool_blocks` patterns if the rendering is non-trivial.

## Example (a `count_lines` tool → numbered output)

```go
// blocks.go, in renderToolBlock switch:
case "count_lines":
    return renderCountLinesBlock(b, w)

// tool_blocks.go:
func renderCountLinesBlock(b *block, w int) string {
    head, failed := renderToolHead(b, w, joinSegs(b.arg("path")))
    var out strings.Builder
    out.WriteString(head)
    if !b.inFlight() {
        out.WriteString("\n" + renderOutputPanel(b.output, b.expanded, w, "lines", failed))
    }
    return out.String()
}
```

## Gotchas

- A core tool with NO renderer still renders — it falls through to
  `renderGenericToolBlock` (functional but a raw args blob). Add a renderer for
  anything the human will see often; skip it for rare/internal tools.
- `b.args` is a raw JSON **string**, not a map. Never index it directly — use
  `b.arg(key)` / `argObjArr` / `argStrArr`, which degrade gracefully if args is
  malformed (a bare string in tests/edge cases).
- Honor `b.expanded` (ctrl+o is **per-block**) — collapse the body by default;
  the panel helpers already do this if you pass `b.expanded` through.
- Keep the house style (single `│` rule, no boxes). If you reach for a card,
  you're reinventing `renderOutputPanel`/`renderRowsPanel` — use them instead.
- The matching core-side workflow is the `add-core-tool` skill (schema +
  classify + dispatch + guards). This skill is the TUI half only.
