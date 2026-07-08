---
name: wire-tui-blocking-flyout
description: Surface a core wire event as a blocking flyout/modal in the Go TUI (event case + key dispatch + render overlay). Use when the core emits a blocking-prompt event (ask/approval/intercom-style) that the TUI must render and capture input for.
---

# Wire a core wire event → Go TUI blocking flyout

Use when the Rust core emits a wire event that should pop a **blocking flyout /
modal** in the Go TUI and capture user input until resolved (the core blocks on
a `Notify` until the TUI sends a reply command). Existing instances:
`ask_request` (ask tool), `approval_request` (approval gate), `intercom_message`
(subagent need_decision). The shape is identical for all three.

This is the **TUI counterpart** of `core-event-to-web` (which covers the Next.js
web side) and is NOT the same as `add-tui-tool-renderer` (which renders a tool
*block* in the transcript — non-blocking, no input capture).

## When to use

- A core event needs a modal/flyout the user interacts with (select, type, submit).
- The core blocks until the TUI sends a reply command (`ask_reply`, `approve`,
  `intercom_reply`).
- NOT for: passive transcript rendering (→ `add-tui-tool-renderer`), web-only
  surfacing (→ `core-event-to-web`), or adding the core-side blocking machinery
  (→ `add-blocking-tool`).

## The three integration points (ALL required — missing any = dead code)

The TUI keeps per-prompt state on the `session` struct (e.g.
`pendingAsk *askPrompt`, `pendingApproval *approvalPrompt`). A prompt type is
dead code unless ALL THREE of these wire it in:

### 1. Event handler — `tui/handlers.go` `handleCoreEvent` switch
Add a `case "<event_type>":` that parses the event payload into the prompt
struct and assigns it to the session field. Place it near the other blocking
prompts (`approval_request` / `intercom_message`).

```go
case "ask_request":
    // rawKey is REQUIRED for structured fields (arrays/objects): ev.get
    // unmarshals into a string, which FAILS for an array → returns "".
    qraw, ok := ev.rawKey("questions")
    if !ok {
        qraw = json.RawMessage("[]")
    }
    if a := parseAskRequest(ev.get("request_id"), qraw); a != nil {
        s.pendingAsk = a
        s.input.Blur()          // modal owns keys; don't leave the chat input blinking
        s.logInfo("❓ agent asks …") // transcript marker (approval/intercom both log)
        s.layout()
    }
```

### 2. Key dispatch — `tui/handlers.go` `handleKey`
Add an intercept right AFTER the modal intercept, BEFORE scroll/global keys. A
blocking flyout owns all keys (option cycling, text entry, submit, skip):

```go
if s.modal.kind != modalNone {
    return s.handleModalKey(msg)
}
// ↓ insert here — same precedence as a modal
if s.pendingAsk != nil {
    return s.handleAskKey(msg)
}
```

Without this the prompt state is set but receives zero keystrokes — the user
can't answer and the core blocks forever.

### 3. Render — `tui/render.go` `View()`
Apply the overlay at the END of `View()`, after the modal overlay. The overlay
helper is a no-op (returns `base` unchanged) when nothing is pending, so it can
be called unconditionally:

```go
view := strings.Join(parts, "\n")
if s.modal.kind != modalNone {
    view = s.renderModalOverlay(view)   // was: return s.renderModalOverlay(view)
}
return s.renderAskOverlay(view)         // no-op when s.pendingAsk == nil
```

The overlay uses `lipgloss.Place(w, h, Center, Center, box)` — it centers the
box in a w×h field of spaces, blanking the background (same as `renderModalOverlay`).

## The rawKey-vs-get gotcha (the #1 silent failure)

`coreEvent.get(key)` unmarshals the value into a **string** — which FAILS for a
JSON array/object and returns `""`. So `ev.get("questions")` on an
`ask_request` (whose `questions` is an array) yields `""`, the parser gets empty
input, and the flyout never opens. **Always use `ev.rawKey(key)`** (returns
`json.RawMessage`) for any structured field. This is the exact bug that left the
TUI's ask feature dead for its entire existence.

## Validation errors: transient inline, not transcript spam

A blocking flyout's submit-failure (e.g. an empty required field) must set an
`errMsg` field on the prompt struct and render it **inside the flyout box**
(cleared on the next non-submit keypress), NOT call `s.logError(...)`. The
latter appends a permanent "✗ …" line to the transcript on EVERY Enter — a
user mashing Enter on an empty required field spams the log (observed 6× in the
wild). Mirror the `intercomNudge` pulse pattern, not the transcript log.

## Key handling: action names + hardcoded fallbacks

Two gotchas that left the ask flyout's navigation dead even after wiring:

1. **`s.kb(msg, action)` silently returns false for unregistered action names.**
   The keybind registry (`keybindDefs` in `tui/keybinds.go`) is the single
   source of truth for action names. `s.kb(msg, "next_field")` returns false
   because the registered action is `"field_next"` (tab). The ask code used
   invented names (`next_field`/`down`/`prev_field`/`up`) — none matched, so
   navigation never fired. Always grep `keybindDefs` for the exact Action string
   before writing `s.kb`/`s.kbAny` calls. Common ones: `field_next`/`field_prev`
   (tab/shift+tab), `nav_down`/`nav_up` (↓/↑), `nav_down_alt`/`nav_up_alt`
   (j/k), `send` (enter), `close` (esc), `cycle_left`/`cycle_right` (←/→/h/l).

2. **Blocking flyouts need hardcoded `msg.String()` arrow fallbacks**, mirroring
   the scroll handler (`msg.String() == "up" || s.kbAny(msg, "nav_up",
   "nav_up_alt")`). Relying solely on the keybind map means a user who
   disabled/rebound a nav key in `/keybinds` can't navigate the flyout at all.
   Arrows must ALWAYS work for a blocking prompt:
   ```go
   if s.kb(msg, "field_next") || msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt") {
   ```

## Verify

- `cd tui && go build ./...` — must pass.
- `cd tui && go vet ./...` — must pass.
- `cd tui && go test ./...` — must pass.
- `gofmt -l <changed>.go` — empty output = clean (CI runs `go vet`/`go test`/`go
  build` for the TUI, not `gofmt`, but keep changed files formatted).

## Test pattern

Exercise the REAL event path via `handleCoreEvent` with a constructed
`*coreEvent`, not just direct field assignment — that's what guards the event
wiring (the part that was missing):

```go
func askRequestEvent(t *testing.T, requestID, questions string) *coreEvent {
    raw, _ := json.Marshal(map[string]any{
        "request_id": requestID,
        "questions":  json.RawMessage(questions),
    })
    return &coreEvent{Type: "ask_request", Raw: raw}
}

func TestAskRequestSetsFlyout(t *testing.T) {
    s := initialSession()
    s.ready = true
    s.width, s.height = 80, 24
    s.layout()
    s.handleCoreEvent(askRequestEvent(t, "ask-1", `[{"id":"x","prompt":"X?","type":"select","options":["A","B"],"required":true}]`))
    if s.pendingAsk == nil { t.Fatal("ask_request must set pendingAsk") }
}
```

Also test: render produces the prompt text (`stripANSI(s.renderAskOverlay(base))`
contains the question), Enter submits + clears, Esc skips + clears.

**Test isolation gotcha:** `initialSession()` calls `loadSettings()`, which
reads the user's REAL `~/.config/catalyst-code/settings.json` — so any test that
depends on keybinds (e.g. asserting `k` navigates via `nav_up_alt`) is
environment-dependent and will fail on a machine where the user disabled that
binding. Reset to defaults in keybind-sensitive tests:
```go
s := initialSession()
s.keybinds = defaultKeybinds() // isolate from user settings
```

## Diagnostic methodology (don't chase exotic races)

When "tool/event not surfacing in a frontend" is reported, **grep BOTH
frontends for the event case + dispatch + render wiring FIRST**, before
theorizing about restart races, field-name mismatches, or ordering bugs. The
common cause is a missing integration point (dead code), not a subtle race. The
`ask-tool-restart-wedge` memory was a red herring for a report that was simply
the TUI never handling `ask_request` at all.
