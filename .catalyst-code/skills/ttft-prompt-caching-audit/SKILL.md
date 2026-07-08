---
name: ttft-prompt-caching-audit
description: Audit the request path for Time-to-First-Token health — prefix-cache stability, provider caching opt-in, and the already-captured cache-hit metric
version: 1
---

## When to use

Use this when the user asks about TTFT / latency / "first token" speed, or when turns
feel slow to start responding. TTFT in an LLM agent harness is dominated by two things:
**prefix-cache hit rate** (how much of the prompt the provider can reuse from the prior
turn) and **request size**. This audit walks the code paths that decide both.

The audit is grounded in THIS harness's provider path (`core/src/`): `provider.rs`
(`stream_turn_*`), `message.rs::build_anthropic_request`, `main.rs` `run_turn`,
`logging.rs::grounded_estimate`, `config.rs` compaction knobs.

## Steps

1. **Confirm streaming is on.** Each `stream_turn_*` sets `stream: true`. If a path
   doesn't, that's the first bug — non-streaming means waiting for the full response
   before any token.

2. **Check system-prompt stability (the cacheable prefix).** Read
   `build_system_prompt` (main.rs ~87). It must be assembled from STABLE sources
   (constants, git context, memory, plugin docs, skill manifest) and NOT mutate per
   turn. Anything injected that changes every turn (a timestamp, a per-request id)
   at the *head* busts the entire prefix cache.

3. **Check the rolling work-state / context-summary placement.** Find where the
   transient summary is pushed (`work_state_message`, main.rs ~3844). It MUST be a
   TAIL message (last in the array) and popped before persisting (main.rs ~3867), so
   updating it never invalidates the cached prefix. If it's spliced into the system
   prompt or mid-stream, it busts the cache every turn.

4. **Check the sanitizer's no-op behavior.** `sanitize_orphaned_tool_calls` +
   `sanitize_tool_call_arguments` run unconditionally before every request, but must
   only rewrite+persist when they actually changed something (main.rs ~3790). On a
   clean turn it's an O(n) scan that returns 0 — that's fine. When it fires (rare:
   aborted turn, malformed args) it rewrites history → one busted turn (unavoidable).

5. **Check compaction/digest tuning (request size).** `context_compact_at` (default
   0.90) and `context_digest_at` (default 0.40, config.rs ~730). The soft digest at
   40% is GOOD for TTFT — it collapses stale large tool results into one-line digests
   well before compaction, shrinking every subsequent request. If `context_digest_at`
   is 0 or very high, request size (and thus TTFT) creeps up over a long session.

6. **THE KEY STEP — check per-PROVIDER caching opt-in.** This is where most TTFT
   wins hide. Grep the tree for `cache_control`:
   - **OpenAI-compatible path** (Umans/GLM/Qwen, `stream_turn_openai`): caching is
     IMPLICIT — no opt-in needed. Already captured via `cached_tokens` from
     `prompt_tokens_details.cached_tokens`. Nothing to add; the lever is measurement
     (step 7).
   - **Anthropic/Claude path** (`stream_turn_anthropic` → `message.rs::build_anthropic_request`):
     Anthropic does NOT cache by default. You MUST set `cache_control` either as a
     top-level field (automatic) or as explicit breakpoints on content blocks. If
     `cache_control` appears nowhere in the Anthropic builder, every Claude turn
     reprocesses the full prompt and `cache_read_input_tokens` comes back 0 — the
     single biggest TTFT lever.
   - **Anthropic subtlety that defeats the naive fix:** automatic caching puts the
     breakpoint on the LAST block. Our last block is the transient work-state tail,
     which changes every turn → zero hits (Anthropic's docs flag this exact pattern
     as the "common mistake"). So use EXPLICIT breakpoints: on the last system block
     (always hits within a session; the system prompt is large enough to clear the
     1024–4096-token min threshold) and a rolling one on the last PERSISTED message
     (the 20-block lookback then gives per-turn hits, since each turn adds <20
     blocks). Keep breakpoints OFF the changing work-state tail. Verify against the
     live Anthropic prompt-caching docs (fetch https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)
     since the API and breakpoint rules evolve.

7. **Measure with what's already captured.** Both provider paths already report
   cache tokens: OpenAI `cached_tokens` (parsed in `stream_turn_openai`) and
   Anthropic `cache_read_input_tokens` (parsed in `stream_turn_anthropic`). They're
   logged in `turn_done` (main.rs ~4452) alongside `ttft_ms`. The diagnostic, zero
   code: if `cached_tokens ≈ tokens_in`, caching is healthy; if it's ~0, something
   is busting the prefix — re-walk steps 2–6 to find it. Surfacing the cache-hit ratio
   (`cached_tokens / tokens_in`) next to `ttft_ms` in the UI is the cheapest way to
   make cache health visible instead of buried in logs.

## Answer shape

- Lead with what's already correct (so the user knows the foundation is sound).
- Name the ONE concrete gap with a file reference and why it matters.
- Give the prioritized fix (usually: add explicit `cache_control` breakpoints to the
  Anthropic path; surface the cache-hit metric) and offer to implement.
- Note the provider split: OpenAI/Umans is implicit-cached (fine); Anthropic needs
  explicit opt-in (the gap).

## Avoid

- Don't claim a provider "auto-caches" without verifying against its current docs —
  OpenAI does implicit prefix caching; Anthropic requires explicit `cache_control`.
  These rules change; fetch the docs to be precise before advising.
- Don't put a cache breakpoint on content that changes every turn (timestamps,
  per-request context, the transient work-state tail). The lookback only finds
  entries WRITTEN at breakpoints; a changing breakpoint yields zero hits.
- Don't conflate request-size levers (compaction/digest) with cache-hit levers —
  both affect TTFT but are independent. Tune both.
