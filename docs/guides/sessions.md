# Session Management

Catalyst Code uses **append-only JSONL session files** to persist conversations.
Sessions survive restart, crash, and system reboot — a resuming process picks up
where the last one left off with no manual recovery.

---

## On-Disk Location

Session files live under `~/.config/catalyst-code/sessions/`. Each workspace
(project directory) gets its own subdirectory named by an FNV-1a hash of the
canonical working directory path:

```
~/.config/catalyst-code/sessions/
  <hex-project-hash>/
    <session-id>.jsonl
    <session-id>.jsonl
    ...
```

This means distinct projects never share session listings, and the same project
always resolves to the same directory regardless of how it was opened.

### Legacy flat file

A legacy file at `~/.config/catalyst-code/sessions/<hex>.jsonl` (without a
subdirectory) is automatically migrated into the per-project directory on first
launch.

### TUI CLI flag

The `--session <FILE>` flag (`CATALYST_CODE_SESSION` env var) overrides the
session file path. This is set automatically by the TUI on startup.

---

## File Format (JSONL)

Each session file is **newline-delimited JSON** with a version header on the
first line:

```jsonl
{"_session_version": 1}
{"role":"system","content":"You are a coding agent…"}
{"role":"user","content":"Add a login form"}
{"role":"assistant","content":"…"}
```

### Schema version header

- The first non-empty line is always `{"_session_version": <N>}`.
- The loader validates this header. If the file's version is **newer** than the
  running binary's `SESSION_VERSION`, loading is refused with a clear error
  message — this prevents silently dropping messages from a future-format file.
- A file with no header is still loaded (treated as a pre-versioning file) for
  backward compatibility.

### Sidecar files

Each session file may have associated sidecar files stored beside it:

| Sidecar | Extension | Content |
|---------|-----------|---------|
| Session metadata | `.meta.json` | Title, pinned state |
| Cumulative stats | `.stats` | Token totals, cached tokens, turn count (JSON) |
| Escalated approvals | `.escalations` | Tool kinds the user approved "always" for this session |
| Checkpoint index | `.checkpoints.jsonl` | Hybrid filesystem checkpoints (see below) |
| Process lock | `.lock` | PID of the owning process (prevents concurrent writes) |

---

## Auto-Resume on Restart

On launch, the TUI scans the current project's session directory for the **most
recently modified** `.jsonl` file and passes it to the core via `--session`.
The core calls `session::load()`, which:

1. Reads the version header and validates it.
2. Replays every subsequent JSON line as a `Message` into the in-memory
   conversation.
3. Returns an empty conversation for a missing file — not an error (first run).

This means restarting the app (or recovering from a crash) immediately shows
the full prior conversation with zero user action.

---

## Slash Commands

All session commands can be entered at the TUI prompt.

### `/new`

Start a fresh session. The current session file is left intact on disk so
sessions accumulate per project. A new timestamped `.jsonl` file is created
and becomes the active session.

### `/sessions`

Open the session picker. Shows every `.jsonl` file in the project's session
directory, with:

- **Title** — the first user message text (truncated to 80 chars), derived from
  the session content (not from the filename).
- **Message count**
- **Last modified time** (pinned entries sorted first, then by recency)
- **Pinned status**

### `/reset`

- Cancels any in-flight turn.
- Clears the in-memory conversation.
- **Truncates the session file** to header-only (via `session::rewrite`).
- Resets cumulative stats and token estimates.

Useful for a clean slate without restarting.

### `/clear`

- Cancels any in-flight turn.
- Clears **only the in-memory** conversation.
- **Keeps the session file intact** — a restart will still resume the full
  history.
- Resets stats and token estimates.

Useful when the visible context is noisy but you want to keep the session
around.

### `/undo`

- Cancels any in-flight turn.
- Restores the filesystem from the latest auto-checkpoint (undoing file changes
  made during the last turn).
- Pops the last user message and all subsequent assistant/tool messages from
  the conversation.
- Rewrites the session file with the trimmed history.
- Replays the remaining conversation to the TUI.

`/undo` is safe to call repeatedly to walk back multiple turns.

### `/compact`

Force a context compaction immediately, regardless of the auto-compact
threshold. Optionally accepts custom instructions:

```
/compact Focus on code samples and API surface
```

If called with no arguments, uses the configured `compact_instructions`.

### `/stats`

Display cumulative usage for the current session:

- Total tokens in (prompt), out (completion), cached
- Turn count
- Cache hit ratio
- Current estimated context size

Stats survive restart via the `.stats` sidecar.

---

## Session Management (Picker & Commands)

### Load a different session

From the picker (`/sessions`), select a session to load. The core:

1. Loads the selected file via `session::load()`.
2. Cancels the in-flight turn.
3. Replaces the in-memory conversation with the loaded messages.
4. Restores the loaded session's cumulative stats.
5. Points the active `session_file` at the selected path.
6. Replays the transcript to the TUI.

If loading fails (e.g. a future-version file), the TUI shows the error and the
current session is unchanged.

### Rename a session

Sessions are identified in the picker by their **title**, which is derived from
the first user message automatically. You can override it via the rename
action in the picker, which sets `title` in the `.meta.json` sidecar.

### Delete a session

From the picker, a session can be deleted. The core:

- Refuses to delete a session that is locked by another process (`.lock` file
  present).
- Removes the `.jsonl` file, `.meta.json`, `.stats`, and `.meta.lock` sidecars.
- Active sessions cannot be deleted — switch to another session first.

### Pin a session

Pinned sessions appear at the top of the picker, sorted above unpinned ones
(and then by recency). Pin state is stored in `.meta.json` (`pinned: true`).

---

## Checkpoint System

Checkpoints provide a **hybrid filesystem undo** for `/undo` and crash recovery.

### How checkpoints work

When a checkpoint is created:

- **Git workspace:** runs `git stash create` and stores the stash reference
  under `refs/catcode/checkpoints/<id>`.
- **Non-git workspace:** copies changed (or explicitly listed) files to
  `.catalyst-code/checkpoints/<id>/`.

An index JSONL file records metadata:

```jsonl
{"id":"cp-1743200000","label":"auto-before-destructive","created_at":1743200000,
 "kind":"git","head_sha":"abc123","stash_sha":"def456",
 "paths":["src/main.rs"],"dir":null,"auto":true}
```

### When checkpoints are created

1. **Auto-before-destructive** — immediately before the first destructive tool
   call in a turn wave (once per turn). This is what makes `/undo` restore
   file changes.
2. **Manual** — via the `create_checkpoint` command (TUI or API) with an
   optional label and path list.

### Checkpoint commands

- `/checkpoint` — create a manual checkpoint with the label "manual".
- `/checkpoints` — list all checkpoints (returns a `checkpoints` event).
- `/checkpoint restore <id>` — restore files from a specific checkpoint.
  Conversation is unchanged; only files are rewritten.

### Checkpoint index location

The index lives at `<session-file>.checkpoints.jsonl`, or at
`.catalyst-code/checkpoints/index.jsonl` when no session file is configured.

---

## Auto-Compact

When the conversation approaches the model's context window, the core can
automatically **summarize and prune** the oldest messages to stay under the
limit.

### Configuration keys (in `~/.config/catalyst-code/config.json`)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `auto_compact` | bool | `true` | Enable automatic compaction on context pressure |
| `compact_instructions` | string | `null` | Optional guidance for the summarization prompt |
| `context_compact_at` | number | 0.75 | Fraction of `context_window` that triggers compaction |

### CLI equivalents

```
--auto-compact true
--compact-instructions "Focus on code and API usage"
```

### Behavior

- When the estimated context exceeds `context_window × context_compact_at`, a
  summarization is triggered at the start of the next turn.
- The summarizer reduces older messages while keeping recent context and system
  messages intact.
- Manually forcing `/compact` always works, regardless of the `auto_compact`
  setting.
- The summarization uses an OpenAI-compatible endpoint with a
  `compact_instructions` prompt, then replaces the summarized portion with a
  compact message.

---

## Crash Safety

Catalyst Code is designed so that a crash (power loss, process kill, kernel
panic) loses **at most one in-flight turn**.

### Append semantics

Messages are appended to the JSONL file one at a time as they are generated.
Each append is flushed to the kernel (but not fsynced) so multi-message turns
are not serialized behind disk syncs.

### Turn-end fsync

At the **end of each turn** (or on abort paths that have already appended
results), `session::sync()` is called:

1. `f.sync_all()` on the session file — ensures appended data reaches the disk.
2. `fsync_dir(parent)` — ensures the directory entry is durable (important
   after a rename on POSIX).

### Rewrite atomicity

Operations that rewrite the entire file (`reset`, `undo`, `compact`) use a
**temp-file + fsync + atomic rename** pattern:

1. Write to a unique temp file in the same directory.
2. `fsync` the temp file.
3. Rename the temp over the target.
4. `fsync` the parent directory.

This means a crash during a rewrite never truncates the existing conversation
— the old file stays intact until the rename completes.

### Process lock

Each running instance places a `.lock` sidecar with its PID. This prevents:

- Two processes writing to the same session file concurrently.
- Deleting a session that is currently active in another process.

---

## Session Token Budget

The `--max-session-tokens <N>` flag (`CATALYST_CODE_MAX_SESSION_TOKENS`) sets a
hard token budget for the session. When the cumulative prompt tokens exceed
this limit, the session is automatically compacted regardless of the context
window. `0` (the default) means unlimited.

---

## Related

- [Architecture overview](../index.md)
- [Configuration reference](../configuration/index.md)
