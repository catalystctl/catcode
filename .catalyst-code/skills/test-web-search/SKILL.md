---
name: test-web-search
description: Test the web_search tool end-to-end — run a known query, verify hits render, and diagnose failures by failure signature (searx truncation, DDG captcha, Mojeek 403, missing API keys).
---

# Test the web_search tool

Use when the user says "test the websearch tool" / "verify web_search works" /
"why did web_search fail?" — a 25× recurring request. `web_search` is the agent's
built-in search (core/src/search_tool.rs) with a layered provider model:
**Exa + Tavily APIs first** (when keys set, round-robin + quota-tracked), then a
**scrape fallback chain** (SearXNG ranked from searx.space → DDG Lite → DDG HTML
→ Mojeek). See memory `exa-tavily-search-api-contracts` and
`duckduckgo-captcha-blocks-web-search`.

## When to use
- After changes to `core/src/search_tool.rs` (providers, parsing, byte caps),
  `config.rs` (`search_keys`), `protocol.rs` (`SetSearchKey`), or the `/search-key`
  TUI/web command.
- When the user reports a web_search failure ("all backends failed", captcha, 403).
- The user asks to "test web_search" / "verify search works".

## Steps
1. **Build the core** — a stale running binary won't have your fix (see memory
   `stale-core-binary-unknown-variant-error`). `cd core && cargo build --bin core`.
   The user's running harness must be restarted to pick up changes.
2. **Run a known query.** Two options:
   - Ask the user to run `web_search` in the harness with `{"query":"Rust programming language homepage","count":5}` and paste the result, OR
   - Verify the diagnosis statically if no live harness: check the failure string.
3. **Diagnose by failure signature** (the error's `attempts:` list tells you which
   backend each failure came from):

   | Failure signature | Meaning | Fix |
   |---|---|---|
   | `searx.space JSON parse failed: EOF … column 262144` | instances.json truncated at 256KB cap (HEAD floors at `64*1024`, so the 256KB default `fetch_max_bytes` wins) | 8MB cap fix (`cfg.fetch_max_bytes.max(8*1024*1024)`) exists in `load_searx_instances`. **Check `git diff src/search_tool.rs` first**: if the fix is UNCOMMITTED WIP, the running binary correctly reflects HEAD — you must build the *working tree* (not just rebuild HEAD) + restart. If committed, it's a stale binary → rebuild + restart. |
   | `DuckDuckGo Lite served a captcha/anomaly page` | DDG rate-limited/blocked | External block — can't fix in code. Rely on SearXNG (primary) or set API keys. |
   | `Mojeek returned HTTP 403` | Mojeek blocking | External — fallback only. |
   | `SearXNG (host) returned HTTP 403` / connection refused | that instance is down | Multi-instance retry handles it; check `rank_searx_instances` returned enough. |
   | Only scrape backends attempted (no Exa/Tavily) | No API keys set | Set via `/search-key exa` / `/search-key tavily` (persisted to config.json `search_keys`). |
   | `Exa: rate-limited (HTTP 429)` / `Tavily: quota` | API provider in cooldown | Cooldown auto-applied; load-balances to the other provider, then scrape. Check monthly usage in the output header (e.g. `43/1000 this month`). |

4. **Verify "pass":** output header like `Search: … (Exa API, 5 hit(s) · 43/1000 this month)`
   with numbered results, OR `(SearXNG (host), N hit(s))` if scraping. Each hit has
   title + url + snippet.

## What "pass" looks like
- With API keys set: results come from Exa/Tavily, header shows `· N/1000 this month`.
- Without keys: SearXNG (primary) succeeds; DDG/Mojeek are fallbacks.
- No "all backends failed" error.
- If SearXNG is the only path and searx.space is reachable, it should now parse
  (instances.json is ~720KB, well under the 8MB cap).

## Key gotchas
- **Rebuild + restart the core** after any search_tool fix — the running harness
  has the OLD binary (the `262144` error means a stale core predating the 8MB fix).
  But FIRST run `git diff src/search_tool.rs` + `git status`: if the 8MB fix is
  UNCOMMITTED working-tree WIP (common — the Exa/Tavily provider layer is often
  in-flight alongside it), the running binary isn't "stale", it just lacks unbuilt
  WIP. `cargo build --bin core` builds the working tree; `cargo check --bin core`
  confirms it compiles. Either way, the harness process must be RESTARTED to pick
  up the new binary (no hot-swap).
- **No API keys = scrape-only** = inherently flaky (DDG captchas, Mojeek 403s).
  For bulletproof search, set keys via `/search-key`. The API providers are tried
  first and are NOT subject to captchas.
- **`fetch` is the manual fallback** — if web_search is fully down, tell the user
  to `fetch` a specific URL directly.
- Tests: `cd core && cargo test search_tool::` (45 tests: parsing, load balancing,
  usage tracking, cooldowns, persisted-key override). `cargo check` for compile.
