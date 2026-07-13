---
name: extend-memory-tool
description: Add a new action (or param) to the `memory` tool — schema enum, dispatch arm, backing fn, MemoryEntry fields, tests. The critical gotcha is the json! action enum.
---

# extend-memory-tool

Add a new action to the harness `memory` tool (e.g. `deprecate`, `migrate`), or a new
param to an existing action (e.g. `save`'s `replaces`). The `memory` tool is the agent's
self-learning surface, so changes touch the data model, the tool dispatch, and the JSON
schema the model sees.

## When to use
- Adding a memory action (beyond save/append/list/get/forget/consolidate/stats).
- Adding a param to an existing memory action.
- Adding a `MemoryEntry` field (frontmatter) consumed by catalog/relevant-tail.

## Steps

1. **Data model (if new field)** — `core/src/memory.rs`:
   - Add the field to `struct MemoryEntry` (+ a doc comment).
   - Parse it in `parse_memory_file` (frontmatter match arm) + set it in the `Some(MemoryEntry { ... })`.
   - Write it in the save path if it should persist (`save_scoped_with_importance` re-derives `pin` from type; for metadata-preserving rewrites use a dedicated writer like `write_memory_file` / frontmatter surgery).
   - **Every `MemoryEntry { ... }` struct literal must be updated** or it won't compile — they live in `memory.rs` tests, `memory_hygiene.rs` tests, and `memory_recall.rs` tests. Grep `MemoryEntry {` across `core/src/*.rs` to find them all.

2. **Backing fn** — `core/src/memory.rs`: implement the logic as a `pub fn` (e.g. `mark_memory_deprecated`, `migrate_memories`). Use `WRITE_LOCK` + `Store::new(Store::default_root())` for file access. Return `Result<T, String>` for tool-friendly errors.

3. **Dispatch arm** — `core/src/tools.rs`, the `memory` tool's `match action` (search `"save" => {`). Add your `"youraction" => { ... }` arm. For a param on an existing action, read it with `args.get("name").and_then(|v| v.as_str()).unwrap_or("")`.

4. **⚠ CRITICAL — the JSON schema (the gotcha).** The `memory` tool's parameters are a `json!` macro (~`tools.rs:656`). The `action` property has a hardcoded `"enum": [...]`. **Adding the match arm is NOT enough** — if the action isn't in this enum, the model literally cannot call it (rejected at schema-validation before reaching your dispatch). You must:
   - Add the action string to the `"action"` enum.
   - Add any new params as `"properties"` entries.
   - Update the `"action"` description string to mention the new action.
   - If a new field changes pinning/type semantics, fix the `type`/`importance` property descriptions too (they drift).

5. **Surfacing** — if the new field should affect recall: `build_catalog` and `build_relevant_tail` (memory.rs) both filter/render. Exclude deprecated from both (filter `!m.deprecated`); mark in `list` and banner in `get` (tools.rs).

6. **Test** — `core/src/memory.rs` or `memory_hygiene.rs`:
   - Pure functions (catalog/tail/sort/stopword logic): no store needed; construct `MemoryEntry` literals directly (remember the new fields).
   - Store-backed fns (`migrate`, `consolidate`, `deprecate`): use `with_temp_store(label, |ws| { ... })` (memory_hygiene tests) which sets `ROOT_OVERRIDE` + serializes via `memory_test_serial()` so the public fns see your tmp store and don't race parallel tests. Call `crate::memory::<your_fn>(ws)`.
7. **Verify**: `cd core && cargo test memory 2>&1 | tail` then `cargo build`. The full suite is `cargo test` (~476 tests).

## The gotcha (why this skill exists)
Adding `deprecate`/`migrate` Rust match arms + backing fns compiled and tested green — but the
actions were **uninvokable** until the `json!` action enum was updated. Schema and dispatch are
two separate surfaces; update both or the model silently can never call your new action.

## Example (the deprecate/migrate/replaces additions)
- Field: `MemoryEntry { deprecated: bool, superseded_by: Option<String> }` + parse + `write_memory_file` writer that preserves pinned.
- Fns: `mark_memory_deprecated`, `mark_memory_deprecated_any`, `migrate_memories` (+ `MigrateReport`).
- Dispatch: `save` arm gained `replaces` (deprecates old after successful save); new `deprecate` + `migrate` arms.
- Schema: enum grew to `[..., "deprecate", "migrate"]`; added `replaces` + `superseded_by` properties; fixed `type` description (convention/decision no longer auto-pinned).
- Recall: `build_catalog` + `build_relevant_tail` both `filter(|m| !m.deprecated)`; `list` marks `[DEPRECATED]`; `get` shows a ⚠ banner.
