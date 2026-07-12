---
name: check-remove-stored-credentials
description: Inspect or clear stored provider credentials (OAuth tokens + API keys) when debugging auth, switching accounts, or forcing a re-login
---

# Check / Remove Stored Credentials

Use when asked to find, inspect, or delete stored API keys / OAuth tokens for a
provider — e.g. "do we have a key stored?", "remove the stored key", "make me
re-login", "switch accounts", or when a fix should take effect but a stale token
or models cache is masking it.

## Where credentials live (verified against core/src)

Auth is **explicit only**: a fresh install with no configured provider does
not scan env vars, and the harness does not use third-party CLI stores (Claude
CLI, gcloud ADC, Codex CLI) for auth. A legacy `~/.gemini/oauth_creds.json`
from an older build is read **once** on startup to migrate it to the new store,
then never read again. Credentials exist only after `/login` (pasted API key or
OAuth), or for a provider explicitly configured with `api_key_env` (the named
env var is read at request time).

| Provider | OAuth token file | Notes |
|---|---|---|
| Codex (ChatGPT sub) | `~/.config/catalyst-code/oauth/openai.json` | harness's OWN store. Shape: `{auth_mode, tokens{id_token, access_token, refresh_token, account_id}}` |
| Gemini | `~/.config/catalyst-code/oauth/gemini.json` | harness's OWN store. A legacy `~/.gemini/oauth_creds.json` is migrated here on first launch |
| Claude/Anthropic | `~/.config/catalyst-code/oauth/anthropic.json` | harness's OWN store (does not read `~/.claude/.credentials.json`) |
| xAI / Qwen / others | `~/.config/catalyst-code/oauth/<id>.json` | harness's OWN store |
| Codex CLI (official) | `~/.codex/auth.json` | NOT read by the harness |
| gemini-cli (legacy) | `~/.gemini/oauth_creds.json` | Read once for migration to the new store, then ignored. `/logout` removes it |
| Claude CLI | `~/.claude/.credentials.json` | NOT read by the harness |

**API keys / provider config** (literal pasted keys, `api_key_env` names, which
providers exist + activeProvider): `~/.config/catalyst-code/config.json` (the
managed config).
Written by `save_providers_config` (atomic temp+rename). Runtime keys from
`/login` (API-key paste) and the `set_key` protocol also land here.

**Models cache**: `~/.config/catalyst-code/models-cache.json` (keyed
`base_url|kind`, 8-hour TTL, schema v4). A STALE cache can mask a freshly-changed
key/provider for up to 8h. When you change credentials, clear it too so
discovery re-runs.

**Escalations** (per-session "always"-approved tool kinds): `<session>.escalations`
sidecar next to the session JSONL — not a credential, but relevant to a full reset.

## Steps

1. **List what's stored** — show the user each relevant file's presence + (for
   the harness's own OAuth file) the masked token fields + account_id, and the
   `providers`/`activeProvider` block of config.json. Use `bash`:
   ```bash
   ls -la ~/.config/catalyst-code/oauth/ 2>/dev/null
   # masked peek at config.json providers (no raw secrets printed):
   jq '{activeProvider, providers: [.providers[]? | {name, kind, base_url, has_api_key:(.api_key!=null), api_key_env}]}' ~/.config/catalyst-code/config.json 2>/dev/null
   ```
   Do NOT print raw secret values — mask them. The user asked to check, not leak.

2. **To remove / force re-login** — delete the OAuth token file(s) for the
   target provider, and (if switching providers entirely) drop the matching
   entry from config.json's `providers[]` + clear the models cache:
   ```bash
   # Codex
   rm -f ~/.config/catalyst-code/oauth/openai.json
   # Gemini (also remove the legacy path so a restart doesn't re-migrate it)
   rm -f ~/.config/catalyst-code/oauth/gemini.json
   rm -f ~/.gemini/oauth_creds.json
   # Claude
   rm -f ~/.config/catalyst-code/oauth/anthropic.json
   # Models cache (clear after ANY credential change so stale models don't mask it)
   rm -f ~/.config/catalyst-code/models-cache.json
   ```
   To remove a literal API key from config.json, edit `~/.config/catalyst-code/config.json`
   and delete the `api_key` (or the whole `providers[]` entry).

3. **Restart the core** after removing credentials — the running core caches
   the resolved provider in memory, so deletion only takes effect on the next
   `init`/restart (or a fresh TUI/web launch). Then re-run `/login`.

## Gotchas

- Third-party CLI credential files are INTENTIONALLY not read — removing them
  does NOT affect the harness, and having them does NOT sign the harness in.
- A fresh download / empty `~/.config/catalyst-code` starts fully signed out;
  the user must paste an API key or complete OAuth via `/login`.
