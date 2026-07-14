---
name: git-status-diffstat
description: Report the current +/- (insertions/deletions) summary of uncommitted working-tree changes. Use when asked "what is the current +/- for our git status" or any quick "show me the diff stats" request — NOT a full review+fix (that's review-and-fix-git-status).
---

# Report git status +/- summary

Use when the user wants a quick readout of the current uncommitted changes' size — the "+/−" or diffstat — without a review or fix loop.

## When to use
- "What is the current +/- for our git status?"
- "Show me the diff stats."
- "How much have I changed?"
- Any request for a terse, numbers-only change summary.

If they also want bugs found / fixed, use `review-and-fix-git-status` instead.

## Steps

### 1. One command gets everything
```bash
git status --short && echo "---DIFFSTAT---" && git diff --stat && echo "---STAGED---" && git diff --cached --stat
```
This yields: modified/untracked file list, the unstaged diffstat (with per-file +/−), and any staged diffstat separately. Note `git status --short` shows `??` for untracked files; `git diff --stat` does NOT count untracked files (they're new, not yet diffed).

### 2. Format the reply
- **Tracked modifications (unstaged): N files → +X / −Y** (net **Z**) — take the final `N files changed, X insertions(+), Y deletions(-)` line from the unstaged diffstat.
- If anything is staged, add a **Staged:** line with its stats.
- **Untracked (new, not in diffstat):** list the `??` entries grouped logically (new routes, new components, new skills, etc.). These are NOT in the +/− totals because git can't diff a file that has no tracked baseline.
- Optionally call out the **biggest churn** files by insertion count — helps the user see where the work concentrated at a glance.

## Anti-patterns
- Adding up untracked file line counts into the +/− total — git doesn't, and it misrepresents the "diff" (untracked = whole-file new, not insertions against a baseline).
- Running `git diff --stat` alone and missing untracked files — `??` entries only appear via `git status --short`.
- Treating this as a review request — this skill is report-only. No fmt/clint/test runs, no fixes. Hand off to `review-and-fix-git-status` if the user then asks to find/fix bugs.
