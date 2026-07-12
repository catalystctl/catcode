---
name: review-and-fix-git-status
description: Review uncommitted working-tree changes for a feature, find bugs/issues, fix them, and loop verification until every CI gate is green. Use when asked to "review git status and fix issues" or "find and fix bugs in the current changes."
---

# Review & fix uncommitted changes (loop until clean)

Use when the user asks to review the current `git status` / uncommitted changes and fix any bugs or issues — especially "loop till you're sure you caught all bugs/issues."

## When to use
- "Review our git status and fix any issues then loop till you are sure you caught all bugs/issues."
- "Find and fix bugs in my uncommitted changes."
- Any "verify my working tree is ready to commit" request focused on the diff (not a full-repo audit — that's `production-readiness-review`).

## Steps

### 0. Scope + concurrency awareness
- `git status` + `git diff --stat` (uncommitted) and `git log --oneline -5` (what was just committed).
- If other agent sessions are active in the workspace (the harness warns "N other agent session(s) active"), treat compile errors NOT in files the diff touches as a concurrent session's in-flight work — isolate and ignore them (see the `concurrent-user-edits-isolate-errors` global gotcha). Verify fixes against `git diff --stat` of what YOU touched.

### 1. Verify the CI gates FIRST (cheap, catches the most common breakage)
The most frequent issue in uncommitted feature work is **formatting/lint drift** — the author wrote correct logic but CI's format/lint gates reject it. These are CI-blocking even when the code compiles. Run them against the EXACT CI config (mirror `.github/workflows/ci.yml`), not just "it builds":
- Rust core: `cd core && cargo fmt --all -- --check` (capture exit via a temp file — `cmd | tail` makes `$?` the tail's exit, not cargo fmt's), `cargo clippy --all-targets`, `cargo check --tests`, `cargo test --locked`.
- Go TUI: `cd tui && go build ./... && go vet ./... && go test ./...`.
- Web: `cd web && npx tsc --noEmit`.
- Run changed components in parallel. Only block on errors in files YOU touched (isolate concurrent edits).

**Gotcha — fmt exit code via pipe:** `cargo fmt --check 2>&1 | tail -5; echo $?` reports `tail`'s exit (0), masking fmt's real exit 1. Use `cargo fmt --all -- --check > /tmp/f.out 2>&1; echo "EXIT:$?"` or check `PIPESTATUS[0]`.

### 2. Read the diffs for logic bugs
Read each changed file's diff. Focus on:
- **New control flow** — gates, early-returns, ordering of checks (e.g. does a cache-restore run before or after hooks? does a deny check short-circuit correctly?).
- **Mirrored logic** — when one code path duplicates another (e.g. subagent loop mirroring main loop), verify it routes through the SAME canonical helper, not a hand-rolled list that drifts and omits cases (e.g. a `wipe` list omitting `bash` → stale cache).
- **Shared mutable state** — caches, env vars, session state shared across processes/threads: is invalidation complete? are temp-file names unique? (see `cross-process-file-write-hazards` global gotcha).
- **Off-by-one / boundary conditions** in caps, budgets, keep-windows.
- **Stale references** to removed constants/functions after a refactor.

### 3. Run the test suites
`cargo test --locked` (core), `go test ./...` (tui). Investigate every failure — but first check if it's a known pre-existing flake (e.g. `workspace_activity_lists_peers` flakes in the full suite but passes alone) or a concurrent session's breakage, not yours.

### 4. Fix issues found
- Apply the smallest correct change. For format drift, `cargo fmt --all` (writes in place).
- For a duplicated-list-drift bug, replace the hand-rolled list with the canonical helper.
- Re-verify the specific component after each fix (fmt → re-run `--check`; logic fix → re-run `cargo check`/tests).

### 5. Loop: re-verify everything until green
Re-run the full gate set after the LAST edit (a late `cargo fmt` to fix one comment can re-introduce nothing, but a logic edit can shift formatting). Confirm:
- `cargo fmt --check` exit 0, `cargo check --tests` clean, `cargo test --locked` all pass, `cargo clippy --all-targets` (note: CI uses no `-D warnings`, so clippy WARNINGS don't fail CI — but fix the cheap ones anyway; leave only warnings in committed code outside the diff).
- TUI build/vet/test, web tsc.

### 6. Distinguish in-scope from out-of-scope warnings
- **In scope:** warnings in files the `git diff` touches (the uncommitted feature). Fix them.
- **Out of scope:** warnings in committed code NOT in `git diff` (e.g. `oauth.rs` doc warnings surfacing from a newer rustc) — these are pre-existing and likely owned by a concurrent session. Note them; don't fix (avoids stepping on in-flight work).

## Anti-patterns
- Trusting `go build ./...` / `cargo check` alone — these validate compilation but NOT fmt/lint CI gates. A green build with a red `cargo fmt --check` still fails CI.
- "Fixing" a compile error in a file you didn't touch — it's almost certainly a concurrent session's in-flight work. Isolate to your touched files.
- Sweeping `git add --all` after fixes in a multi-session workspace — stages neighbors' unfinished work. (Use hunk-staging per the `git-commit-all` skill if committing.)
