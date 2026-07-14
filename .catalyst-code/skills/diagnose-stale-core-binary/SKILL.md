---
name: diagnose-stale-core-binary
description: Diagnose a core "bad command: unknown variant <X>" error reported after updating/installing — it's almost always a stale RUNNING core binary (deleted inode), not a code bug. Use whenever the user pastes a serde unknown-variant error from the core, especially "after updating or installing".
---

# Diagnose stale-core-binary "unknown variant" error

## When to use
- User pastes `bad command: unknown variant '<X>', expected one of [...]` from the core.
- Especially: "this happens after updating / installing" on Linux or Windows.
- The error lists a known-good but OLDER set of variants (it has `list_plugins` but is missing variants a recent commit added).

## Core insight
This harness spawns the core as a **subprocess** (TUI: `coreBinaryPath()` in `tui/main.go`; web: `CoreProcess` in `web/src/server/live-session.ts`). Rebuilding/reinstalling the core binary does **NOT** restart already-running cores. An idle/hung core keeps running the OLD binary (a *deleted inode*) for hours. A current TUI/web client sends a command the old core's serde `Command` enum doesn't know → "unknown variant". **It is not a code bug** — the source already has the variant; only the running process is stale.

## Steps

### 1. Confirm the source has `<X>` (rule out a real code gap)
```bash
grep -n "rename = \"<X>\"" core/src/protocol.rs   # inside pub enum Command (≈lines 35–362)
grep -n "Command::<VariantId> =>" core/src/main.rs # dispatched?
```
If both present → not a code bug; continue. (Find the variant id by reading the enum around the rename.)

### 2. Fingerprint which commit the running binary predates
Diff the error's "expected one of" list against `core/src/protocol.rs`. The MISSING variants identify the last commit the running binary included. Find when they were added:
```bash
git log --oneline -S '<X>' -- core/src/protocol.rs
```
(e.g. `list_plugin_commands`/`plugin_command`/`reload_plugins`/`list_agents` → commit `a85e76a`.)

### 3. Find stale (deleted-inode) running cores
```bash
for p in $(ls /proc | grep -E '^[0-9]+$'); do
  exe=$(readlink /proc/$p/exe 2>/dev/null) || continue
  case "$(basename "$exe")" in core|catcode-core)
    echo "pid=$p ppid=$(ps -o ppid= -p $p|tr -d ' ') exe=$exe start=$(ps -o lstart= -p $p)";; esac
done
```
A `(deleted)` suffix on the exe = stale core = the error source.

### 4. Verify the on-disk binary is current (empirical — do NOT trust `strings`)
`strings <bin> | grep '^<X>$'` is UNRELIABLE: serde rename `&str`s are packed without NULs, so anchored `strings` misses them. Use **substring** count (0 = truly absent/stale, ≥1 = present/current) OR the empirical test:
```bash
mkdir -p /tmp/cc
{ printf '{"type":"init","model":null,"approval":"never","reasoning_effort":null}\n'
  printf '{"type":"<X>"}\n'; sleep 1; } \
| timeout 6 <core-binary> --workspace /tmp/cc --approval never \
    --session /tmp/cc/s.jsonl --debug-log /tmp/cc/dbg.jsonl 2>/tmp/cc/err \
| grep -aE '<X's response event>|bad command|unknown variant'
```
Accept (returns the expected event) = on-disk binary is current → only the *running* process is stale.

## Fix
- **If the on-disk binary is current (the usual case):** kill the stale cores. The harness auto-restarts from the current on-disk binary (TUI: `coreEOFMsg` restart path; web: `live-session.ts` `ensureCore` respawns on next view). No rebuild, no code change.
- **If the on-disk binary itself is stale:** `cargo build --release` in `core/`, then reinstall to the installed layout (`/usr/local/bin/catcode-core` beside `/usr/local/bin/catcode`; TUI resolves via `coreBinaryPath` search order — env `CATCODE_CORE` overrides). Then restart the harnesses.

## Gotchas
- `strings … | grep '^<X>$'` false-negative on serde renames (packed, no NUL) — use substring `grep -c` or the empirical test.
- Stale cores can survive for **hours** (idle/hung), so "I updated and restarted the app but it still errors" usually means a *different* long-lived core (e.g. a web live-session or a second TUI window) — enumerate ALL cores, not just the current window's.
- The shared `debug.jsonl` (`--debug-log`) receives logs from EVERY core instance; grepping it for "bad command" yields false positives from tool-call args that literally contain those strings.
- This incident (2026-07-13): `list_plugin_commands` (+`plugin_command`/`reload_plugins`/`list_agents`, commit `a85e76a`); on-disk binaries already current; stale cores self-cleared on restart.

## Example
User pasted `unknown variant 'list_plugin_commands', expected one of [init, set_key, …, list_plugins, refresh_memory, …]` (list omits the four `a85e76a` variants). Source had it (protocol.rs:208 + main.rs:2782). On-disk `/usr/local/bin/catcode-core` and `core/target/release/core` were sha-identical (`95614c5a…`) and accepted the command empirically. Stale deleted-inode cores (a Jul-6 `catcode-core` + 4 web live-sessions) were the source; they self-exited within minutes. No fix needed.
