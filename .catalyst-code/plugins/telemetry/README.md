# telemetry plugin

Aggregates per-turn model metrics — tokens (in/out/cached), latency (TTFT),
throughput (TPS), and elapsed time — into a per-workspace telemetry summary so a
developer can track trends over time: token cost, prefix-cache effectiveness,
and model responsiveness.

Works **out of the box**: it reads the metrics the core surfaces in the
`session_stop` hook context, so it does **not** require enabling the JSONL
debug log (`CATALYST_CODE_DEBUG_LOG`).

## How it works

The core fires the `session_stop` lifecycle hook after every assistant turn
(it is turn-scoped, despite the "session" name). With `pass_args: true`, the
hook receives the cumulative session totals and the just-completed turn's
metrics. This plugin:

1. Appends one record per turn to `turns.jsonl` (append-only).
2. Incrementally updates `summary.json` (read → apply → atomic write) — no
   full rescans, so it stays cheap as a session grows.
3. Regenerates `summary.md`, a human-readable report.

## Output location

`~/.config/catalyst-code/telemetry/<workspace-hash>/`

The `<workspace-hash>` is the FNV-1a 64-bit hash of the canonical workspace path
(16-char hex), matching the core's memory-store hashing so the two trees
correlate. Each workspace gets its own summary.

## Files

| file          | purpose                                                  |
|---------------|----------------------------------------------------------|
| `turns.jsonl` | one JSON record per turn (tokens, ttft, tps, model, ts)   |
| `summary.json`| incremental aggregates (totals, averages, per-model, …)  |
| `summary.md`  | the same, rendered for humans                             |

## What it captures

- **Token trends** — cumulative in/out/cached and per-turn breakdown.
- **Prefix-cache hit rate** — `cached_tokens / tokens_in`.
- **Latency (TTFT)** — avg / min / max over turns that report it.
- **Throughput (TPS)** — avg over turns that report it.
- **Per-model breakdown** — turns + tokens split by model id.

## Not captured (deliberately deferred — see docs/SELF_LEARNING.md §12)

These signals are marked deferred in the design doc and require either the
JSONL debug log or additional core instrumentation:

- **Skill utilization** (counts of `read_file` on `SKILL.md`).
- **Test pass rate** (parsing `bash` test output).
- **`/undo` correction rate** (undo rewrites history; no per-turn trace).

They can be added later by a richer `session_stop` plugin that scans the
conversation session file incrementally, without touching core.

## Configuration

None. The hook runs whenever the plugin is enabled (it is staged globally by
default). To disable it, use the plugin management command or delete the
directory under `~/.catalyst-code/plugins/telemetry/`.
