---
name: add-core-background-task
description: Add a periodic background task to the Rust core (heartbeat, poll, presence publisher) that runs independent of turns
version: 1
---

## When to use

You need something in `catalyst-code`'s Rust core that runs CONTINUOUSLY, on a
timer, independent of turns — not triggered by a tool call or a user message.
Examples already in the codebase:

- **`umans_conc` poll** (`main.rs`) — every 5s hits `/v1/usage`, emits
  `umans_conc { used, limit, provider }` so the footer shows live concurrency.
- **Presence heartbeat** (`main.rs` + `presence` module) — every 8s publishes
  this session's `WorkState` to a per-pid file so peers can detect concurrent
  activity, and refreshes a cached peer snapshot.

If the thing must happen AT a specific point in the turn loop (before/after a
model request, on each tool result), that's a **plugin hook** or a `run_turn`
injection — NOT a background task. Background tasks are for ambient polling/
broadcasting that has no per-turn trigger.

## Where things live

- **Spawn site** — `core/src/main.rs::async fn main()`, AFTER the
  `Arc<State>` is constructed and BEFORE `let stdin = …; while let … lines.next_line()`.
  The existing `umans_conc` poll block is the canonical template — copy its shape.
- **The task** — a `tokio::spawn(async move { loop { …; tokio::time::sleep(interval).await; } })`.
  Capture `state.clone()` (an `Arc<State>`) + any read-once values (pid, a
  workspace clone) into the move. Inside, snapshot State fields via their
  `tokio::sync::Mutex`/`RwLock` (`.lock().await` / `.read().await`), hold the
  guard only as long as needed (`drop(guard)`), then do the I/O.
- **State field for output** — if a consumer needs the polled value on the hot
  path (e.g. a per-tool-result check), cache it in a NEW `Mutex<T>` field on
  `State` that the task refreshes each tick; consumers read the cache (cheap,
  no I/O) instead of re-polling. Add the field to the `State` struct + initialize
  it in the `Arc::new(State { … })` literal. (See presence: `peers: Mutex<Vec<_>>`
  refreshed by the heartbeat, read by `maybe_concurrency_note` in `run_turn`.)

## Steps

1. **Decide cadence + whether a hot-path cache is needed.** If a turn-loop
   consumer needs the value, add a `State` field for the cached snapshot
   (struct decl + init literal) and have the task refresh it each tick. If the
   task only EMITS an event (no turn-loop consumer), no State field is needed.
2. **Capture read-once inputs before the move** — `std::process::id()`, a
   `workspace.clone()` read from `state.cfg` (cfg is moved into the `RwLock` at
   State construction), `home_dir()`, a `started_at` timestamp. These are
   constant for the process lifetime, so reading them once avoids per-tick
   lock churn.
3. **Do the first write/emit IMMEDIATELY (before the loop)** so a consumer
   checking right after startup sees the value — don't wait one full interval.
4. **Spawn the task** mirroring the `umans_conc` block: clone `state`, move it
   in, `loop { …work…; tokio::time::sleep(interval).await; }`. Keep work cheap;
   a background tick should never block the turn loop (it can't — it's a
   separate task — but a slow tick delays the next tick).
5. **Crash-safety for any FILE the task writes** — use atomic temp+fsync+rename
   (same pattern as `session`/`memory` persistence). If the task writes a
   per-process file, also provide a `clear_<thing>()` called on clean shutdown
   (the stdin-EOF path at the tail of `main`, after awaiting the running turn)
   so the file disappears instantly; tolerate `kill -9` via a stale-reaper on
   read (mtime threshold) since the shutdown path doesn't run on SIGKILL.

## Gotchas

- **`tokio::time::sleep`, not `std::thread::sleep`.** The task runs on the
  async runtime; `std::thread::sleep` blocks a worker thread.
- **Don't hold a `MutexGuard` across `.await`.** Snapshot the value (`.clone()`)
  and `drop(guard)` before any `.await`/I/O, or you'll serialize the task behind
  the turn loop (and vice versa). The `umans_conc` block does this correctly.
- **`name` vs `&name`** — the dispatch variables in `run_turn` are owned
  `String`s; a helper taking `&str` needs `&name` (deref coercion), not `name`.
- **The debug binary `core/target/debug/core` can vanish between
  `cargo build` and a smoke run** — a concurrent `cargo test`/`cargo clippy`
  (e.g. this session's own harness) reorganizes incremental artifacts and the
  main bin can be absent while `deps/` remains. Re-run `cargo build` immediately
  before any smoke test that execs the binary.
- **Config root is `~/.config/catalyst-code/`** (post-rename; older notes may
  say `umans-harness`). Per-workspace scoping reuses `crate::memory::project_hash`
  (already `pub`, fnv1a 16-hex of canonicalized cwd) — no need to promote or
  duplicate it.

## Smoke-testing

A background task that writes a file is best verified by a real launch: start
the binary with a FIFO stdin, send `{"type":"init"}`, sleep one+ interval, then
check the file exists with the expected content; then close stdin (EOF, NOT
`kill` — `kill` skips the shutdown-cleanup path) and confirm the file is gone.
Use `find ~/.config/catalyst-code/<subsys>/ -name "<pid>.<ext>"` to locate the
per-process file (don't assume the workspace-hash subdir — list and match by pid).
