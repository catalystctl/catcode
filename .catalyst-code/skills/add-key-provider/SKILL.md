---
name: add-key-provider
description: Add a new key-auth model provider to the harness (OpenAI- or Anthropic-compatible endpoint). Config-only when model discovery matches a known shape; code change in provider.rs when it doesn't.
version: 1
---

## When to use

The user says something like "add provider X" / "I want to use X's models" /
"connect to <endpoint>". This skill decides whether the provider can be added
**via config file only** (no recompile) or needs a **code change** in
`core/src/provider.rs`, and does the work.

## Background — how providers work today

A provider = one endpoint + one API key + a **wire protocol**. The harness keeps
its internal conversation in OpenAI chat-completions shape always; the provider's
`kind` decides the translation at the HTTP boundary only.

Two moving parts, each is either config-driven or code-driven:

| Part | Config-driven? | Where |
|------|----------------|-------|
| Endpoint URL, key, headers, wire kind | ✅ yes | `ProviderConfig` (`config.rs`) |
| **Model discovery** (list models + their caps) | ⚠️ partly | `provider.rs` — paths are hardcoded constants |

The gap that decides config-vs-code: **model discovery**. The harness knows
exactly THREE hardcoded response shapes (see Decision tree). If the target
endpoint matches one, config-only works. If not, you add a code branch
(the OpenCode-Go pattern).

## Step 0 — Gather facts about provider X

Read the provider's API docs. You need:

1. **Base URL** — e.g. `https://api.example.com/v1`. Must include the version
   segment because the harness appends paths directly (`{base}/chat/completions`).
2. **Wire protocol** — does it expose `/chat/completions` (OpenAI-compatible) or
   `/v1/messages` (Anthropic)? Pick `kind: "openai"` or `kind: "anthropic"`.
3. **Auth** — API key (header name? `Authorization: Bearer` is default; Anthropic
   uses `x-api-key` unless OAuth). Env var name? OAuth (→ see the
   `match-official-cli-oauth` skill instead)?
4. **Model discovery endpoint** — does it serve `/models/info`, `/models`, or
   `/v1/models`? **Grab a sample JSON response.** This is the critical input.
5. **Per-model caps** — does the discovery response include context_window /
   max_output_tokens / reasoning / vision per model? If yes → great (config-only
   with a known shape). If the endpoint returns bare ids only → you'll need a
   curated overrides table (code, or the proposed `models_override` config field).

## Step 1 — Decision tree (config-only vs code change)

Look at the **sample model-discovery JSON** from step 0.4:

- **Shape A — Umans `/models/info`** (rich): a JSON **object keyed by model id**,
  each value has `display_name` + `capabilities.{context_window,
  recommended_max_tokens, supports_vision, reasoning:{supported,levels}}`.
  → **config-only.** Set `kind:"openai"`; discovery auto-finds `/models/info`.

- **Shape B — OpenAI `/models`** (bare ids): `{ "data":[ {"id":"gpt-4o",...} ] }`.
  Caps are NOT in the response → the harness applies its curated `openai_model_caps`
  table for known families (gpt-5/o-series/gpt-4.1/gpt-4o/gemini) and flat
  defaults for unknown ids.
  → **config-only IF** your models are known families OR you accept flat default
  caps (200k ctx / 65k out). Else needs `models_override` (proposed) or a code
  branch.

- **Shape C — Anthropic `/v1/models`**: `{ "data":[ {"id":..,"display_name":..} ] }`.
  → **config-only** with `kind:"anthropic"`. Caps come from a static Claude table.

- **Shape D — none of the above** (custom fields, nested differently, or the
  endpoint 404s on all three paths).
  → **code change** required in `provider.rs`. Follow the OpenCode-Go pattern
  (§"Code-change path" below). Do NOT try to force it through config today —
  the model-info endpoint path and its parser are hardcoded.

- **Special wire tweaks** — if the provider needs `reasoning_effort` /
  `reasoning_content` replay (Umans/Zhipu GLM behavior) or a non-Bearer auth
  header, check `is_umans()` / `is_codex_endpoint()` etc. in `provider.rs`.
  These are host-gated; a new host needs a new matcher OR the proposed
  `wire` config block.

## Step 2 — Config-only path (Shapes A/B/C)

Add a `providers` entry to `~/.config/catalyst-code/settings.json` (user-owned,
0600). Minimal shape (mirrors `parse_provider` in `config.rs`):

```jsonc
{
  "providers": [
    {
      "name": "my-provider",
      "kind": "openai",                 // "openai" | "anthropic"
      "base_url": "https://api.example.com/v1",
      "api_key_env": "MY_PROVIDER_API_KEY",
      "headers": [["X-Organization", "12345"]]
    }
  ],
  "activeProvider": "my-provider"
}
```

Fields (`parse_provider`, `config.rs:1278`):
- `name` (required) — slug, unique.
- `kind` — `"openai"` (default) or `"anthropic"`. Decides `/chat/completions`
  vs `/messages` + the auth header (`Bearer` vs `x-api-key`) + discovery path.
- `base_url` (required) — include `/v1`. Paths are appended directly.
- `api_key` — literal key (stored in the 0600 file). OR `api_key_env` — env var
  **name** (preferred; secret stays in env). One or the other; `api_key` wins.
- `headers` — extra HTTP headers, `[[key,val],…]`.

Then: use `/login` to paste the API key, add the provider via config with a
literal `api_key`, or set `api_key_env` and export the named env var (read at
request time). A fresh install with no configured provider stays signed out.

**What you CANNOT do via config today** (these are the gaps — see the review's
proposed schema): override the model-discovery *endpoint path*, declare a custom
*response schema/field paths*, supply per-model *capability overrides*, or point
at a *usage/billing endpoint*. If you need any of those, use the code-change
path (or wait for the proposed config extension).

## Step 3 — Code-change path (Shape D, or custom wire/discovery)

When discovery doesn't match a known shape, add a code branch in
`core/src/provider.rs`. The reference implementation is **OpenCode Go**
(provider.rs:860–1196). Copy its four-part shape:

1. **A host matcher** — `pub fn is_my_provider(base_url: &str) -> bool` that
   parses the host exactly (never a bare `contains` — see `is_umans`'s host-parse
   comment for the look-alike-domain gotcha). Match on host + a path segment.
2. **A capabilities table** — `fn my_provider_caps(id) -> Option<(ctx,max,vision)>`
   sourced from the provider's docs or Models.dev (`https://models.dev/models.json`).
   This replaces what the endpoint doesn't return. Keep it in sync with §3.
3. **A known-models + display-name table** — `fn my_provider_known_models()` for
   the offline fallback (display names the `/models` list omits).
4. **A wire-protocol router** (only if the provider serves models over BOTH
   OpenAI and Anthropic protocols under one key, like OpenCode Go) —
   `fn my_provider_model_protocol(id) -> Option<bool>` (true=OpenAI, false=Anthropic).
   Model this as **two ProviderConfigs** (one per `kind`) sharing base_url+key,
   via a `preset_provider_configs` expansion (config.rs:496).

Then wire discovery: in `discover_models_openai` / `discover_models_anthropic`,
add `if is_my_provider(&provider.base_url) { return my_provider_discover_models(...).await; }`
BEFORE the generic `/models/info` → `/models` fallthrough. Parse the endpoint's
real response shape with a new `fn parse_my_provider_models(&Value)`.

Add unit tests mirroring the opencode-go tests (provider.rs:3601+):
`is_my_provider_matches_path`, `my_provider_curated_lists_partition_by_protocol`.

**Auth special-case:** if the provider uses a non-Bearer header or needs
client-version params (Codex sends `?client_version=0.0.0`), add it in the
discovery + `stream_turn` request builders. `ResolvedProvider.oauth` flag exists
for the Anthropic `anthropic-beta: oauth` header variant.

## Step 4 — Preset (first-party keepers only)

In-tree `/login` presets are limited to **Umans**, **OpenCode Go**, and
**OpenRouter**. Do **not** add other vendors to `PROVIDER_PRESETS` — ship them as
plugins (`plugin.json` provider / `oauth` block, or a user config entry).

If you are extending one of the three keepers (e.g. OpenCode Go dual-protocol
expansion), edit the existing `ProviderPreset` / `preset_provider_configs` branch
in `config.rs`. For a new vendor, follow the plugin-authoring skill instead.

## Worked examples

### Umans (the default) — config-only, Shape A
- `kind:"openai"`, `base_url:"https://api.code.umans.ai/v1"`, key `UMANS_API_KEY`.
- Discovery: endpoint serves `/models/info` (Shape A, rich caps) → fully
  automatic. The GLM-specific wire tweaks (`reasoning_effort`, `reasoning_content`
  replay) fire because `is_umans()` matches host `umans.ai`. **No code needed.**
- It's also `PROVIDER_PRESETS[0]` (a preset) so `/login` lists it.

### OpenCode Go — the code-change reference, Shape D
- One key at `https://opencode.ai/zen/go/v1` serves models over BOTH wire
  protocols (GLM/Kimi/DeepSeek/MiMo via OpenAI; MiniMax/Qwen via Anthropic).
- Its `/v1/models` returns bare ids with **no caps and no protocol field** → none
  of shapes A/B/C apply → full code branch: `is_opencode_go`, `opencode_go_caps`
  (Models.dev-sourced), `opencode_go_known_models`, `opencode_go_model_protocol`,
  + `preset_provider_configs` expands to two configs (opencode-go +
  opencode-go-anthropic). ~400 lines of provider.rs.
- This is the ceiling of what "add a provider" costs today when discovery is
  non-standard. The proposed `models_override` + `models.schema:"custom"` config
  fields (see review) would shrink most of this to JSON.

## Pitfalls

- **Don't `contains()` a hostname** — `is_umans`/`is_opencode_go` parse the host
  so `api.umans.ai.evil.com` isn't mistaken for Umans (would enable GLM-only wire
  fields on the wrong endpoint → 400s). Copy the host-parse pattern.
- **Cache is keyed `base_url|kind`** (`provider_cache_key`) and has an 8h TTL +
  schema version (`MODELS_CACHE_VERSION`, currently 5). If you change the parser,
  bump the version so stale caches refresh instead of masking the fix. Delete
  `~/.config/catalyst-code/models-cache.json` to force-refresh during testing.
- **`api_key` vs `api_key_env`**: storing the env-var *name* (not the secret) is
  the convention for presets; a literal key is fine in the 0600 user file but
  NEVER in a project-local `settings.json` (untrusted repo footgun).
- **Orphaned tool-call 400s**: orthogonal to providers, but a new provider that
  returns malformed tool-call JSON can trigger the always-run sanitizer. If a
  custom provider botches function-call JSON, verify with a minimal one-tool repro
  before trusting it in an agentic loop (see `opencode-go-provider-design` memory
  re: minimax-m3).
