---
name: git-commit-all
description: Stage and commit all working-tree changes (tracked + untracked) with a descriptive message derived from the diff, then show the commit hash
---

# git-commit-all

When the user says "commit all," "commit everything," "please commit all in git status," or similar, stage ALL working-tree changes (modified, new, deleted) and create a single commit with a descriptive message.

## When to use

- After completing a multi-file change (feature, refactor, bug fix)
- When the user explicitly asks to commit everything in one go
- When the working tree has a coherent set of changes that belong together

Do NOT use for:
- Staging only specific files (use `git add <paths>` + `git commit` manually)
- Amend/squash workflows
- When there are merge conflicts or the index is in a conflicted state

## Steps

1. **Show what will be committed**: `git status --short` and `git diff --stat` so the user can confirm.
2. **Stage everything**: `git add --all` (stages modified, new, and deleted files).
3. **Build a commit message** from the diff summary:
   - Read the file list from `git diff --cached --stat`
   - Group by directory/module (e.g. "core: …", "tui: …", "web: …")
   - Include the primary change type (e.g. "refactor", "fix", "add feature X")
   - If a single logical change spans files, use one sentence; if multiple, use bullet points
   - Keep the subject line ≤72 chars; body wraps at 72
4. **Commit**: `git commit -m "<message>"`
5. **Show the commit**: `git log -1 --oneline`

## Message convention

Follow conventional commits when the change has a clear type:

```
type(scope): short description

- bullet for notable per-file changes
- bullet for rationale if non-obvious
```

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `style`, `perf`

Scopes: `core`, `tui`, `web`, `sdk`, or module name within core (e.g. `core/provider`, `core/message`).

## Example

```
refactor(core): replace opaque Value conversation with typed Message enum

- core/src/message.rs: new Message/Content/ToolCall types (720 lines)
- core/src/provider.rs: stream_turn/sanitizers → &[Message], native Anthropic builder
- core/src/main.rs: State.conversation → Vec<Message>, typed compaction
- core/src/session.rs: backward-compatible Message serialization in JSONL
- core/src/subagent.rs: typed sub-conversation building
- core/src/logging.rs: estimate_message_tokens → &Message
```

## Edge cases

- **Nothing to commit**: exit early with "nothing to commit, working tree clean"
- **Unmerged paths** (conflicts): abort — `git add --all` would stage conflict markers; tell the user to resolve conflicts first
- **Large binary files**: check `git diff --cached --stat` for suspicious files >1MB and warn before committing
