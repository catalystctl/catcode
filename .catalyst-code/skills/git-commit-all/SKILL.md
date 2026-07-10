---
name: git-commit-all
description: Stage and commit all working-tree changes (tracked + untracked) with a descriptive message derived from the diff, show the commit hash, and push to the remote when asked
---

# git-commit-all

When the user says "commit all," "commit everything," "please commit all in git status," or similar, stage ALL working-tree changes (modified, new, deleted) and create a single commit with a descriptive message. When they ALSO say "push" ("commit git status and push", "commit and push"), follow the commit with a push to the remote (step 6 below).

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
2. **Verify it compiles** (don't land broken code): run the project's type checker / build for each *changed* component before committing — e.g. `cd core && cargo check` (main binary; avoid `cargo check --tests` if a pre-existing test-binary breakage unrelated to your change is known), `cd tui && go build ./... && go vet ./...`, `cd web && npx tsc --noEmit`. Run the changed ones in parallel; only block the commit on errors in files YOU touched (isolate concurrent-user edits — see the concurrent-user-edits-isolate-errors gotcha). Skip components you didn't touch.

   **Prefer the EXACT CI gates, not just "it compiles"** — especially when the diff touches formatting or adds/changes a CI gate. A plain `cargo check` / `go build` won't catch fmt drift that CI fails on. The authoritative gates (mirror `.github/workflows/ci.yml`):
   - core: `cargo fmt --all -- --check` · `cargo clippy --all-targets` (treat warnings as errors under `-D warnings` if CI sets it) · `cargo test --locked`
   - tui: `gofmt -l .` (must be EMPTY) · `go vet ./...` · `go build ./...` · `go test -race ./...` (the `-race` matters — catches data-race fixes a plain `go test` won't)
   - When a commit ADDS a new CI gate (e.g. a gofmt step), run that gate yourself before committing — the gate's first run should already pass on your commit.
3. **Stage everything**: `git add --all` (stages modified, new, and deleted files).
4. **Build a commit message** from the diff:
   - For a SMALL change, `git diff --cached --stat` (file list + churn) is enough.
   - For a LARGE or multi-feature diff (hundreds of lines, many files), read the FULL `git diff` (not just `--stat`) — `--stat` won't reveal the distinct features interleaved across shared files like `main.rs`; the actual hunks let you enumerate each feature accurately in the body.
   - Group by directory/module (e.g. "core: …", "tui: …", "web: …")
   - Include the primary change type (e.g. "refactor", "fix", "add feature X")
   - If a single logical change spans files, use one sentence; if multiple, use bullet points
   - Keep the subject line ≤72 chars; body wraps at 72
5. **Commit**: `git commit -m "<message>"`
6. **Show the commit**: `git log -1 --oneline`
7. **Push (only if the user asked to push)**:
   - Current branch: `git rev-parse --abbrev-ref HEAD` (if it prints `HEAD`, you're in detached HEAD — abort and tell the user to checkout a branch first).
   - If the branch has an upstream (`git rev-parse --abbrev-ref @{u}` succeeds): `git push`.
   - If it has NO upstream: `git push -u origin <branch>` (sets upstream on first push).
   - On non-fast-forward rejection: `git pull --rebase origin <branch>` then `git push`.
   - Report the range pushed (`<old>..<new>  <branch> -> <branch>`) and, if the repo has workflows triggered on push, list them (`gh run list -L 3 -R <owner/repo>`) so the user sees what the push kicked off.
8. **Watch CI to green (when the user said "get it live" / was fixing CI)**: a push that says "get those changes live" or that fixes a red CI isn't done at `git push` — the point is a GREEN run. After pushing, capture the newest run id from the `gh run list` output above, then watch it to completion:
   - `gh run watch <run-id> --repo <owner/repo> --exit-status` (blocks until the run finishes; exits non-zero if any job failed).
   - On success: confirm green with a one-line summary (`gh run view <run-id> --repo <owner/repo>`, list the ✓ jobs).
   - On failure: `gh run view --job <failed-job-id> --repo <owner/repo> --log-failed` to read the failing step's log, diagnose, fix, commit, push again, and re-watch. Loop until green (cap a few rounds — if a job is structurally broken, surface it to the user instead of flailing).
   - Benign annotations (cache-restore misses like "Dependencies file is not found … go.sum", Node.js deprecation notices, runner-warnings) are NOT failures — don't chase them; only act on a job marked `X`.
   - Skip this step if the push was to a branch with no CI, or the user only said "commit and push" without a live/CI connotation.

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
- **Multiple unrelated changes**: if the working tree contains two or more logically distinct changes (e.g. a feature plus an untracked skill, or a refactor plus an unrelated fix), commit them SEPARATELY — one commit per logical change — rather than muddling them into one. Stage each group with explicit `git add <paths>` (NOT `git add --all`), commit, then stage+commit the next. The single `git add --all` + one commit is only for changes that belong together.
- **Other agent sessions active (concurrent workspace)**: before staging, run `workspace_activity` — if other sessions are running, their in-flight edits are in YOUR working tree too (e.g. a `README.md` another session is fixing, a `discord-post.txt` another is writing). `git add --all` would sweep those into YOUR commit, committing another agent's unfinished/half-baked work. Instead stage ONLY your files with explicit `git add <paths>` (verify against `git diff --stat` of what you actually touched), then `git status --short` to confirm nothing of theirs is staged before committing. This is the staging-side counterpart of isolating THEIR compile errors (step 2): isolate THEIR file changes too.

  **When a SHARED file is entangled** (my hunks + a concurrent session's hunks in the SAME file — common for `core/src/main.rs` in this repo): `git add <file>` stages the WHOLE file, sweeping in their work — which breaks the build (e.g. a concurrent session added `mod models_dev;` to main.rs but `models_dev.rs` is still untracked/unstaged → dangling mod = compile error) AND commits another session's unfinished feature. Use HUNK-LEVEL staging via `git apply --cached`:
  1. `git add` the files EXCLUSIVELY mine (no concurrent edits).
  2. For each entangled file: `git diff <file> | awk` to keep only MY hunks (selected by the `@@ -OLDSTART,LEN` line), write to a patch, `git apply --cached --recount <patch>`. The old-start is relative to HEAD, and since the concurrent hunks are in different regions, each hunk's context matches HEAD and applies cleanly to the index.
  3. Verify `git diff --cached | grep` for concurrent markers (the other session's function/module names) is EMPTY before committing — then `git commit` (staged only; never `--all`). The concurrent sessions' unstaged changes stay in the working tree untouched.
  4. To fold a follow-up fix (e.g. a rustfmt pass) into the just-pushed commit: apply the fmt edits, re-hunk-stage, `git commit --amend --no-edit`, `git push --force-with-lease`. Force-push-with-lease is safe for a seconds-old commit here — concurrent sessions edit the LOCAL working tree (they haven't pulled), so no one else has the SHA you're rewriting.

  GOTCHA: `cargo fmt --all -- --check` (and `gofmt -l .`) run LOCALLY checks the WORKING tree (mine + concurrent unstaged), so it can report clean while the COMMITTED code — what CI checks, WITHOUT the concurrent unstaged changes — still fails fmt on a concurrent session's already-committed-but-unformatted lines. CI rustfmt/gofmt failures at lines you didn't touch = a concurrent session's committed code, NOT yours; don't reformat those lines (the working tree has their newer edits there → conflict risk). Report them, don't fix them.
- **Detached HEAD**: `git rev-parse --abbrev-ref HEAD` prints `HEAD` — can't push a branch; tell the user to checkout a branch first.
- **No remote / not authed**: `git push` fails — tell the user to add a remote or authenticate (`gh auth login` / SSH key).
- **Protected branch / PR-required**: a direct push to a protected branch is rejected — tell the user the repo requires a PR.
