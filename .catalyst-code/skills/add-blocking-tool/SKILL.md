---
name: add-blocking-tool
description: Add a tool that BLOCKS until the user/frontend answers (approval, intercom, ask). Extends add-core-tool with the Pending+Notify+event/command pair.
version: 1
---

## When to use

You are adding a tool whose result depends on a human (or the TUI/web frontend)
answering BEFORE the model can continue — the tool call must *block* mid-loop,
surface a prompt to the user, and resume with their answer. This is distinct
from `add-core-tool` (sync/async fire-and-forget tools) and `core-event-to-web`
(one-directional events): a blocking tool needs a round-trip (event out →
command back) and a `Notify` to wake the awaiting future.

Three canonical instances already exist — copy the closest one:
- **`approval_request` / `approve`** — `request_approval()` in `main.rs`. Returns
  a granted/denied bool (the simplest shape; no structured payload back).
- **`intercom_message` / `intercom_reply`** — `intercom.rs::resolve_ask`. A
  subagent→orchestrator blocking ask returning free text.
- **`ask_request` / `ask_reply`** — `request_ask()` in `main.rs`. Returns a
  structured answers object (the most general shape).

## The pattern (5 pieces, all required)

1. **Pending struct + State map** (`main.rs`):
   ```
   pub struct PendingFoo {
       request_id: String,
       notify: Arc<Notify>,
       result: Mutex<Option<Value>>,   // None=awaiting, Some=answer
   }
   // + State::pending_foos: Mutex<HashMap<String, Arc<PendingFoo>>>
   // + static FOO_SEQ: AtomicU64 (unique ids so parallel subagents can't collide)
   ```
   Initialize the map field in the `State { … }` literal (search `pending: Mutex::new`).

2. **Event out + command back** (`protocol.rs`):
   - Emit `Event::new("foo_request").with("request_id", …).with(<payload>)` from
     the request function. The dynamic `Event` needs no enum variant.
   - Add a `Command` variant `#[serde(rename = "foo_reply")] FooReply { request_id, <answer> }`.

3. **`request_foo()` async fn** (`main.rs`) — mirrors `request_approval`:
   - Build the PendingFoo, insert into the map, emit the request event.
   - `tokio::select!` on `notify.notified()` (→ take the answer) vs
     `cancel.cancelled()` (→ remove from map, return Aborted).
   - Return an enum: `Answered(answer)` / `Skipped` / `Aborted`.

4. **Command handler** (the `match cmd` in `main.rs`): look up the PendingFoo by
   `request_id`, set its result, `notify.notify_one()`. Mirror `Command::Approve`
   / `Command::AskReply`.

5. **Tool-loop dispatch** (the `else if name == …` chain in `run_turn`): call
   `request_foo()`, map the result → `Outcome`. **On `Aborted`, emit `aborted` +
   `done` and `return` from `run_turn`** (the assistant message with the tool_call
   is already in history; the orphaned tool_call is cleaned by the always-run
   sanitizer next turn — same as the approval gate). Do NOT add a fake tool_result
   on abort. Validation errors (bad args) should return `Skipped`/an err Outcome
   WITHOUT blocking.

## Frontend wiring (both TUI and web)

- **Wire field names must match core exactly.** Core emits `request_id`; the
  command echoes `request_id` back. Mismatches silently hang forever (see the
  `web-intercom-reply-fieldname-bug` memory: web read `request_id` but core
  emitted `id` → the ask hung).
- **TUI:** a priority key handler in `handleKey` (right after the modal check)
  owns all keys while the prompt is up; render as a banner or `renderModalOverlay`
  in `View()`. See `tui/ask.go` for a multi-field flyout. Use `coreEvent.rawKey`
  (protocol.go) to pull structured fields (arrays/objects) — `.get()` only returns
  strings.
- **Web:** add the event to the `CoreEvent` union + a reducer `case` that sets
  `pendingFoo` (clear it in `reset`/`history`/workspace-switch, mirroring
  `pendingApproval`); add the reply to `CoreCommand` + a `useAgent` callback that
  optimistically clears `pendingFoo` then `send()`s the command; render a
  component in `chat.tsx` next to the approval/intercom banners.
- **State hygiene:** clear `pendingFoo` on `done`/`aborted`/`reset`/`history`
  everywhere `pendingApproval` is cleared (reducer.ts has ~4 sites).

## Gotchas

- A blocking tool that is ONLY in `execute()` (no sentinel + no `request_foo` in
  the tool loop) returns its err string as the tool_result and never blocks — the
  model thinks it ran. Wire BOTH the sentinel and the async dispatch.
- The `return` inside the `else if name == …` expression is valid Rust (`return`
  has type `!`), but double-check the brace balance — inserting an arm mid-chain
  is the most common source of mismatched braces.
- Keep blocking tools orchestrator-only by NOT adding them to
  `subagent::all_tool_names()`; subagents that need human input use
  `contact_supervisor` (which IS a blocking tool, subagent-scoped).
