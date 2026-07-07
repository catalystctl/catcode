---
name: core-event-to-web
description: Add a new core wire event and surface it in the Next.js web frontend (types → reducer → component). Use when the Rust core should emit a new event kind and the web should react to it.
---

# Core wire event → web component

Use when you need the Rust core to emit a NEW event kind (not a new *tool* — see
`add-core-tool`) and the Next.js web frontend to consume it. Typical triggers: a
lifecycle/observability signal, a new payload the UI should render, or fixing a
core↔web field-name mismatch.

## When to use

- Adding a new event the core emits on stdout (the stdio JSONL protocol).
- Surfacing new state in the web UI driven by that event.
- NOT for: adding a tool the model can call (→ `add-core-tool`), adding a config
  knob (→ `add-config-knob`), or TUI-only rendering (→ `add-tui-tool-renderer`).

## The pipeline (4 layers, all required for end-to-end)

The bridge (`web/src/server/core-bridge.ts` + `live-session.ts`) is DELIBERATELY
generic: it runs every core event through the one shared `reduce()` and fans it
out over SSE. So you only touch 4 files — no bridge code.

### 1. Core: emit the event (`core/src/<module>.rs`)
Events are DYNAMIC — there is NO event enum. Just construct + emit:
```rust
use crate::protocol::{emit, Event};
emit(&Event::new("my_event")
    .with("run_id", json!(id))
    .with("payload", json!(v)));
```
- Pick the emission site at the right lifecycle point (start / per-turn / done).
- For keyed collections (per-run, per-session), tag every event with a stable id.
- Cap large string payloads (truncate to ~8k–16k) before emitting if the value
  can be huge (file contents, bash output) — the event is extra traffic on top of
  the main stream.

### 2. Web types (`web/src/lib/types.ts`)
Add a variant to the `CoreEvent` union matching the exact field NAMES the core
emits (the #1 bug source is a name mismatch — e.g. core emits `id`, web reads
`request_id` → silently undefined → reply/command fails). If new UI state is
needed, add the type + a field on `AgentState`.

### 3. Web reducer (`web/src/lib/reducer.ts`)
- Add the field to `initialState` (e.g. `myState: {}`).
- Add a `case "my_event":` in `reduce()`. For keyed collections, use an
  `upsert(state, key, fn)` helper that get-or-creates a stub so an event arriving
  before its "start" event never crashes.
- The reducer is SHARED by the server snapshot AND the browser, so a case here
  flows to reconnecting clients automatically.

### 4. Web component (`web/src/components/<name>.tsx`)
Consume the new `AgentState` field. Wire it into `chat.tsx` (modal/panel switch
or inline). Reuse `web/src/lib/format.ts` helpers (`relativeTime`, `formatMs`,
`formatTokens`, `truncate`, `prettyArgs`, `toolIcon`) + `icons.tsx`.

## Verify (the two gates that matter)

- `cargo check` (core main binary) — must pass.
- `tsc --noEmit` (run `cd web && npx tsc --noEmit`) — must pass.
- `cargo clippy --all-targets` for lint (may show a STALE cached test target
  that looks green — don't trust it; fresh `cargo check --tests` is authoritative).
- Caveat: the core TEST binary (`cargo test` / `cargo check --tests`) currently
  fails on a PRE-EXISTING `oauth.rs` breakage (json macro / jsonwebtoken) from
  uncommitted Cargo.toml changes — unrelated to event work. Confirm your files are
  clean with `cargo check --tests 2>&1 | grep error | grep -v oauth.rs`.

## Gotchas

- **Field-name mismatches core↔web** are silent and deadly. Always read the
  actual `Event::new("…").with("field", …)` emit site and copy the exact key.
- **The Go TUI ignores unknown event types** (it matches a switch on known
  types), so adding core events is forward-compatible / non-breaking for the TUI.
  But it won't RENDER them unless you also extend `tui/handlers.go`.
- **No streaming of suppressed events**: subagent turns run `stream_turn(quiet:
  true)`, which gates `delta`/`tool_call_*`/`thinking` emits. To surface a
  subagent's live content you must emit your OWN tagged events at the loop level
  (the data is already in `run_agent_inner`'s locals) — you cannot just flip
  `quiet:false` (that would emit unattributed deltas interleaving with the
  parent stream). See the `subagent-observability-feature` memory for the worked
  example.
- **Reducer is the single source of truth**: don't special-case events in the
  bridge — add the case to `reduce()` and both snapshot + live clients get it.
