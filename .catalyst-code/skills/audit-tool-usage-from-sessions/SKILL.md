---
name: audit-tool-usage-from-sessions
description: Audit all agent sessions to diagnose why the agent prefers bash over a native tool (e.g. grep via bash vs the grep tool), and decide what to fix.
when-to-use |
  - User asks "why does my agent still use X via bash instead of our X tool?"
  - User asks to "audit all sessions for tool usage" or "figure out why the agent chooses bash over <tool>"
  - You need data to decide whether to enrich an existing native tool or add a new one
steps |
  1. Locate the session logs: `~/.config/catalyst-code/sessions/*/*.jsonl` (JSONL; each line is a message). Also `~/.config/catalyst-code/debug.jsonl`. Exclude the live session you're running in.
  2. Confirm the native tool's REAL capabilities by reading both its schema (core/src/tooling/schema.rs `definitions()`) AND its implementation (core/src/tools.rs — `grep()`/`execute()` dispatch). Schema ≠ impl sometimes; the impl may wrap a CLI (grep wraps `rg`).
  3. Write a Python audit script (write_file, not inline bash) that:
     - walks every session .jsonl, parses assistant `tool_calls[].function.{name,arguments}` (arguments is a JSON string) AND content-block tool_use shapes
     - counts native tool calls (grep/glob/read_file/list_dir) for proportion
     - for bash calls invoking the search binary (grep/rg/find/awk/sed), classify WHY with tags: OUTSIDE_WS (~/, /abs, .. — legitimate, can't fix w/o breaking confinement), PIPE (| head / | grep / | wc / | sort / | xargs), FLAG_V/FLAG_F/FLAG_W/FLAG_A/FLAG_B, GLOB_NEG_MULTI, FIND, AWK_SED, SYSTEM (ps/proc/env), and a PLAIN_INWS bucket = plain in-workspace grep using only flags the native tool ALREADY supports (pure habit).
     - a cmd can hit multiple tags; keep examples per tag.
  4. Read the tagged counts: tags present in native tool = habit/awareness (fix the schema description, not code); tags ABSENT from native tool = real gaps (fix the code). OUTSIDE_WS is a deliberate non-fix.
  5. Implement the high-value, low-risk gaps in BOTH code paths (native grep has an rg path + a pure-Rust fallback — change both), add unit tests covering both paths, run `cargo test --bin core <tool>` + full `cargo test --bin core` + `cargo clippy` to confirm zero new warnings.
  6. Enrich the schema description to surface what the tool ALREADY does (e.g. "output includes line numbers; covers -n/-l/-c/-i/-C; paginates via head_limit") — this is often the biggest lever since the #1 driver is usually `| head` habit, not missing capability.
  7. Persist findings as a decision memory; do NOT weaken workspace confinement.
gotchas |
  - Session assistant messages use `tool_calls[].function.arguments` as a JSON STRING — json.loads it. Also handle `content: [...]` block arrays for older shapes.
  - `| head` is the dominant "bash grep" driver and native grep ALREADY paginates via head_limit — it's an awareness problem, not a capability gap. Don't over-engineer.
  - OUTSIDE_WS (system/proc/ps/sibling repos) is correct confinement, not a bug — never "fix" it by letting native tools escape.
  - Run the existing native-tool tests FIRST as a baseline before changing anything; rg must be on PATH for the rg path (it usually is).
example |
  Diagnosed grep-via-bash: audit of 661 sessions showed invert(-v)=329, fixed_string(-F)=14, word(-w)=43, after(-A)=20 were real gaps; added them to native grep in both rg + pure-Rust paths (17/17 tests pass). `| head`=2116 was awareness — fixed via schema description, not code. OUTSIDE_WS=1610 left as deliberate confinement.
