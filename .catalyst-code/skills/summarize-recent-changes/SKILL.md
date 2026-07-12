---
name: summarize-recent-changes
description: Summarize the most recent changes in the repo — latest commit(s) + any uncommitted working-tree edits — concisely.
---

# Summarize Recent Changes

Use when asked "summarize the most recent changes", "what's new", "what did you just do", or similar — i.e. report on recent *deltas* (as opposed to `codebase-overview`, which answers "what is this codebase" on an unfamiliar repo).

## Steps

0. **Enable git tools** — call `load_tools` with `tools:["git"]` once if not already enabled (git_* tools are deferred).
1. **Working tree state** — `git_status`. Determines whether the "recent changes" are committed (→ inspect commits) or uncommitted (→ inspect the diff), or both.
2. **Recent commits** — `git_log` (limit ~15–20) for the commit history line. The HEAD commit is almost always what "most recent changes" refers to when the tree is clean.
3. **Uncommitted changes** — `git_diff` (unstaged) and/or `git_diff` with `staged:true`. Only meaningful if `git_status` shows changes; skip if clean.
4. **Drill into the HEAD commit** when the tree is clean:
   - `git show --stat <sha>` → files touched (scope/scale).
   - `git log -1 --format='%H%n%an <%ae>%n%ad%n%n%B' <sha>` → the full commit message body (the author's own grouping/rationale is the best raw material for a summary — reuse it, don't paraphrase it away).
   - If the HEAD commit is a small follow-up, also glance at the 1–2 commits before it for context.
5. **Summarize concisely.** Group by component/area (e.g. Rust core vs Go TUI vs web), lead with the most significant/critical fix, preserve the severity labels the author used (CRITICAL/HIGH/...), and call out behavior changes a user would notice. Keep it tight — a few bullets per area, not a wall of text.

## Notes

- Don't edit project code for a summary request — this is read-only recon.
- If the working tree is clean, "most recent changes" = the HEAD commit; there's nothing else to report. Say so rather than fabricating broader scope.
- Prefer the commit message's own structure (it already groups findings) over re-inventing the grouping.
- If `git_diff`/`git_status` return nothing, that IS the answer ("clean tree, all changes are in the latest commit") — proceed to the commit.
