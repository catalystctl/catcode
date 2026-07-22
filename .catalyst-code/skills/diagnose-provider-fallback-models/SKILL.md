---
name: diagnose-provider-fallback-models
description: Diagnose why a configured provider shows generic fallback models (Umans/Gemini/Codex/xAI) instead of its real models in the picker.
version: 1
---

## When to use

The user reports a provider's model list shows the WRONG models — generic
fallback ones (e.g. "my local/LM Studio provider only shows the Umans fallback
models, not its actual models") instead of that provider's real models.

## Background — why fallback models appear

`discover_models_openai` (`core/src/providers/discovery.rs`) tries
`/v1/models/info` (Umans-rich) first, then falls back to `/v1/models`. If BOTH
yield nothing usable, it returns `openai_fallback_models(base_url)`:

| host shape                       | fallback returned            |
|----------------------------------|------------------------------|
| Codex (`is_codex_endpoint`)      | `codex_fallback_models()`     |
| Gemini / Code Assist             | `gemini_fallback_models()`    |
| xAI (`is_xai_endpoint`)          | `xai_fallback_models()`       |
| **everything else (incl. localhost)** | `fallback_models()` = **generic Umans list** |

So "I see Umans models for my local/other provider" = discovery could not reach
or parse the endpoint. The fallback is returned **uncached** (no entry is
written on failure), so once the endpoint serves, the next fetch gets the real
models — **no cache clear is needed**. `parse_openai_models_list` reads the
standard `{data:[{id,…}]}` OpenAI shape robustly, so parsing is almost never the
cause; don't blame it.

## Diagnostic steps

1. **Probe the endpoint live:**
   `curl -sS -m 5 -w '\nHTTP %{http_code}\n' http://<host>:<port>/v1/models`
   (also `/v1/models/info`). Interpret:
   - **HTTP 000 / "Could not connect"** → server not running/not started. For
     LM Studio: the app being open is NOT enough — click "Start Server"; also
     load a model or enable JIT loading. Try `127.0.0.1` vs `localhost` (an
     IPv6 `::1` miss happens if the server binds IPv4-only).
   - **HTTP 401/403** → API auth enabled + token mismatch (config `api_key` vs
     the server's expected token). Disable auth on the server or fix the token.
   - **HTTP 2xx + `{data:[]}`** → no model loaded and JIT off (LM Studio) → load
     a model / enable JIT.
   - **HTTP 2xx + `{data:[{id,…}]}`** → endpoint is FINE; the issue is
     client-side: a stale fresh cache entry, or the harness hasn't re-run
     discovery (refresh the model list / restart). Go to step 2.
2. **Check the cache:** `~/.config/catalyst-code/models-cache.json` →
   `entries["<base_url>|<kind>"]`. A fresh (<8h) entry short-circuits discovery
   (returned without a network call). If it holds wrong models, delete the file
   (or bump `MODELS_CACHE_VERSION`) to force a refresh.
3. **Port mismatch:** if the server runs on a non-default port, `base_url` in
   the provider config must match (LM Studio default = 1234).

## Pitfalls

- The fallback list looks like real provider models — don't assume the
  provider's API is broken; for local providers it's almost always "server not
  started" or "no model loaded."
- A 2xx with empty `data` (model not loaded) silently falls back the SAME way a
  connection failure does.
- `activeProvider` pointing at an unreachable local provider makes EVERY turn
  fail, not just the model list — check that first if turns error out.

## Worked example

LM Studio provider `http://localhost:1234/v1` shows Umans models.
`curl http://localhost:1234/v1/models` → "Could not connect to server" (HTTP
000). Root cause: LM Studio server not started. Fix: open LM Studio → Developer
tab → Start Server, load the gemma model. No code or cache change needed; the
next discovery fetch pulls the real model.
