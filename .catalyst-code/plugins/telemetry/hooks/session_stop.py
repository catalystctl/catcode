#!/usr/bin/env python3
"""telemetry session_stop hook.

Fires after every assistant turn (the `session_stop` lifecycle hook). Aggregates
the per-turn metrics the core surfaces in the hook context into a per-workspace
telemetry summary, so a developer can track token trends, prefix-cache hit
rate, latency (TTFT), and throughput (TPS) over time — without enabling the
JSONL debug log.

Inputs (stdin JSON; `args` is present because pass_args:true in the manifest):
  {
    "hook": "session_stop",
    "tool": "",
    "workspace": "/abs/path/to/workspace",
    "session_id": "<file>.jsonl",
    "timestamp": 1719000000,
    "args": {
      "session": {"turns": N, "tokens_in": N, "tokens_out": N,
                  "cached_tokens": N, "model": "..."},
      "turn":   {"tokens_in": N, "tokens_out": N, "cached_tokens": N,
                 "ttft_ms": N|null, "elapsed_ms": N, "tps": F|null,
                 "model": "..."}          # null/absent before the 1st turn
    }
  }

Output (stdout, exactly one JSON line):
  {"allow": true, "reason": "telemetry: ..."}
`session_stop` is a lifecycle hook: `allow` is ignored, but a single JSON
response is required and a non-zero exit means the hook is silently skipped —
so this script always exits 0 and emits a clear `reason`.

Writes, under ~/.config/catalyst-code/telemetry/<workspace-hash>/:
  turns.jsonl   — one record per turn (append-only)
  summary.json  — incremental aggregates (read -> update -> atomic write)
  summary.md    — human-readable rendering, regenerated each call

The workspace hash is FNV-1a 64-bit of the canonical workspace path (16-char
hex), matching the core's memory-store hashing so the two trees correlate.

Robustness: every field is optional; any error is caught and reported as a
skip reason so a bad payload or full disk can never wedge a turn.
"""
import datetime
import json
import os
import sys


def _emit(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.write("\n")


def _fnv1a_hex(s):
    """FNV-1a 64-bit of a string -> 16-char zero-padded lowercase hex.
    Matches the core's memory-store workspace hashing for consistency."""
    h = 0xCBF29CE484222325
    for b in s.encode("utf-8", "surrogatepass"):
        h ^= b
        h = (h * 0x100000001B3) % (1 << 64)
    return format(h, "016x")


def _workspace_hash(workspace):
    try:
        rp = os.path.realpath(workspace or "")
    except Exception:
        rp = workspace or ""
    return _fnv1a_hex(rp or "")


def _as_int(v):
    if v is None:
        return None
    try:
        return int(v)
    except (TypeError, ValueError):
        try:
            return int(float(v))
        except (TypeError, ValueError):
            return None


def _as_float(v):
    if v is None:
        return None
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def _atomic_write(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def _load_summary(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return None


def _new_summary(workspace, ws_hash):
    return {
        "workspace": workspace,
        "workspace_hash": ws_hash,
        "first_ts": None,
        "last_ts": None,
        "total_turns": 0,
        "total_tokens_in": 0,
        "total_tokens_out": 0,
        "total_cached_tokens": 0,
        "cache_hit_rate": 0.0,
        "ttft_count": 0,
        "ttft_sum_ms": 0,
        "min_ttft_ms": None,
        "max_ttft_ms": None,
        "avg_ttft_ms": None,
        "tps_count": 0,
        "tps_sum": 0.0,
        "avg_tps": None,
        "elapsed_sum_ms": 0,
        "avg_elapsed_ms": 0.0,
        "per_model": {},
    }


def _fmt_int(n):
    try:
        return format(int(n), ",")
    except Exception:
        return str(n)


def _fmt_float(f, nd=2):
    if f is None:
        return "n/a"
    try:
        return f"{float(f):.{nd}f}"
    except Exception:
        return "n/a"


def _ts_to_str(ts):
    if ts is None:
        return "n/a"
    try:
        return datetime.datetime.fromtimestamp(
            int(ts), tz=datetime.timezone.utc
        ).strftime("%Y-%m-%d %H:%M:%SZ")
    except Exception:
        return "n/a"


def _render_md(s):
    lines = []
    lines.append("# Telemetry Summary")
    lines.append("")
    lines.append(f"Workspace: {s.get('workspace','')}  (hash {s.get('workspace_hash','')})")
    lines.append(f"Window: {_ts_to_str(s.get('first_ts'))} .. {_ts_to_str(s.get('last_ts'))}")
    lines.append("")
    lines.append(f"Turns: {s.get('total_turns', 0)}")
    tin = s.get("total_tokens_in", 0) or 0
    tout = s.get("total_tokens_out", 0) or 0
    tcache = s.get("total_cached_tokens", 0) or 0
    rate = s.get("cache_hit_rate", 0.0) or 0.0
    lines.append(
        f"Tokens in: {_fmt_int(tin)}  | out: {_fmt_int(tout)}  | "
        f"cached: {_fmt_int(tcache)}  (cache hit {rate*100:.2f}%)"
    )
    lines.append("")
    ttft_n = s.get("ttft_count", 0) or 0
    lines.append(
        f"Latency (TTFT):  avg {_fmt_float(s.get('avg_ttft_ms'))} ms  | "
        f"min {s.get('min_ttft_ms')}  | max {s.get('max_ttft_ms')}  (n={ttft_n})"
    )
    tps_n = s.get("tps_count", 0) or 0
    lines.append(f"Throughput:      avg {_fmt_float(s.get('avg_tps'))} tok/s  (n={tps_n})")
    lines.append(f"Elapsed/turn:    avg {_fmt_float(s.get('avg_elapsed_ms'))} ms")
    lines.append("")
    per_model = s.get("per_model", {}) or {}
    if per_model:
        lines.append("## Per model")
        lines.append("| model | turns | tokens_in | tokens_out | cached |")
        lines.append("|---|---|---|---|---|")
        for model in sorted(per_model.keys()):
            m = per_model[model] or {}
            lines.append(
                f"| {model} | {m.get('turns',0)} | "
                f"{_fmt_int(m.get('tokens_in',0))} | "
                f"{_fmt_int(m.get('tokens_out',0))} | "
                f"{_fmt_int(m.get('cached_tokens',0))} |"
            )
    return "\n".join(lines) + "\n"


def main():
    try:
        raw = sys.stdin.read()
        ctx = json.loads(raw) if raw.strip() else {}
    except Exception as e:
        _emit({"allow": True, "reason": f"telemetry skipped: bad stdin json ({e})"})
        return

    workspace = ctx.get("workspace") or ""
    ts = ctx.get("timestamp")
    args = ctx.get("args") or {}
    turn = args.get("turn")
    session = args.get("session") or {}

    # No completed turn yet (e.g. a session_stop with no model turn): nothing
    # per-turn to record. Still ack so the hook is happy.
    if not turn:
        _emit({"allow": True, "reason": "telemetry: no turn metrics (skipped)"})
        return

    try:
        ws_hash = _workspace_hash(workspace)
        home = os.path.expanduser("~")
        base = os.path.join(home, ".config", "catalyst-code", "telemetry", ws_hash)
        os.makedirs(base, exist_ok=True)

        turns_path = os.path.join(base, "turns.jsonl")
        summary_path = os.path.join(base, "summary.json")
        md_path = os.path.join(base, "summary.md")

        t_in = _as_int(turn.get("tokens_in")) or 0
        t_out = _as_int(turn.get("tokens_out")) or 0
        t_cache = _as_int(turn.get("cached_tokens")) or 0
        ttft = _as_int(turn.get("ttft_ms"))  # may be None
        elapsed = _as_int(turn.get("elapsed_ms")) or 0
        tps = _as_float(turn.get("tps"))  # may be None
        model = turn.get("model") or "(unknown)"

        # Append the per-turn record.
        record = {
            "ts": ts,
            "model": model,
            "tokens_in": t_in,
            "tokens_out": t_out,
            "cached_tokens": t_cache,
            "ttft_ms": ttft,
            "elapsed_ms": elapsed,
            "tps": tps,
        }
        with open(turns_path, "a", encoding="utf-8") as f:
            f.write(json.dumps(record))
            f.write("\n")

        # Incrementally update the summary (no full rescans).
        s = _load_summary(summary_path) or _new_summary(workspace, ws_hash)
        s["workspace"] = workspace
        s["workspace_hash"] = ws_hash
        if s.get("first_ts") is None:
            s["first_ts"] = ts
        s["last_ts"] = ts
        s["total_turns"] = int(s.get("total_turns", 0)) + 1
        s["total_tokens_in"] = int(s.get("total_tokens_in", 0)) + t_in
        s["total_tokens_out"] = int(s.get("total_tokens_out", 0)) + t_out
        s["total_cached_tokens"] = int(s.get("total_cached_tokens", 0)) + t_cache
        tin_total = s["total_tokens_in"]
        s["cache_hit_rate"] = (
            round(s["total_cached_tokens"] / tin_total, 4) if tin_total else 0.0
        )
        if ttft is not None:
            s["ttft_count"] = int(s.get("ttft_count", 0)) + 1
            s["ttft_sum_ms"] = int(s.get("ttft_sum_ms", 0)) + ttft
            prev_min = s.get("min_ttft_ms")
            prev_max = s.get("max_ttft_ms")
            s["min_ttft_ms"] = ttft if prev_min is None else min(prev_min, ttft)
            s["max_ttft_ms"] = ttft if prev_max is None else max(prev_max, ttft)
            s["avg_ttft_ms"] = round(s["ttft_sum_ms"] / s["ttft_count"], 2)
        if tps is not None:
            s["tps_count"] = int(s.get("tps_count", 0)) + 1
            s["tps_sum"] = float(s.get("tps_sum", 0.0)) + tps
            s["avg_tps"] = round(s["tps_sum"] / s["tps_count"], 2)
        s["elapsed_sum_ms"] = int(s.get("elapsed_sum_ms", 0)) + elapsed
        s["avg_elapsed_ms"] = round(s["elapsed_sum_ms"] / s["total_turns"], 2)

        pm = s.setdefault("per_model", {})
        m = pm.setdefault(
            model, {"turns": 0, "tokens_in": 0, "tokens_out": 0, "cached_tokens": 0}
        )
        m["turns"] = int(m.get("turns", 0)) + 1
        m["tokens_in"] = int(m.get("tokens_in", 0)) + t_in
        m["tokens_out"] = int(m.get("tokens_out", 0)) + t_out
        m["cached_tokens"] = int(m.get("cached_tokens", 0)) + t_cache

        _atomic_write(summary_path, json.dumps(s, indent=2) + "\n")
        _atomic_write(md_path, _render_md(s))

        _emit(
            {
                "allow": True,
                "reason": (
                    f"telemetry: recorded turn {session.get('turns','?')} "
                    f"(model={model})"
                ),
            }
        )
    except Exception as e:
        _emit({"allow": True, "reason": f"telemetry skipped: {e}"})


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        # Last resort: never exit non-zero. A lifecycle hook failure is
        # silently skipped by the core, but a clean reason beats a silent drop.
        _emit({"allow": True, "reason": f"telemetry skipped: {e}"})
    sys.exit(0)
