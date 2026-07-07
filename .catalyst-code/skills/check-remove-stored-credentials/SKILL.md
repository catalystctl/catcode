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

The harness stores provider credentials in a few distinct places. Check ALL of
the relevant ones — a provider can have BOTH an OAuth token AND a literal/env API key:

| Provider | OAuth token file | Notes |
|---|---|---|
| Codex (ChatGPT sub) | `~/.config/catalyst-code/oauth/openai.json` | harness's OWN store. Shape: `{auth_mode, tokens{id_token, access_token, refresh_token, account_id}}` |
| Gemini | `~/.gemini/oauth_creds.json` | SHARED with gemini-cli (both write the same file) |
| Claude/Anthropic | `~/.claude/.credentials.json` | official Claude CLI's store — harness READS it, does not own it |
| Codex CLI (official) | `~/.codex/auth.json` | NOT read by the harness (intentional) — only matters if the user installed the CLI separately |

**API keys / provider config** (literal pasted keys + `api_key_env` names + which
providers exist + activeProvider): `~/.config/catalyst-code/config.json` (the
managed config). Written by `save_providers_config` (atomic temp+rename). Runtime
keys from `/key` also land here.

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
   test -f ~/.gemini/oauth_creds.json && echo "gemini: present"
   test -f ~/.claude/.credentials.json && echo "claude: present"
   test -f ~/.codex/auth.json && echo "codex-cli: present (not read by harness)"
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
   # Gemini
   rm -f ~/.gemini/oauth_creds.json
   # Claude (official CLI store — only remove if user wants to re-login there)
   rm -f ~/.claude/.credentials.json
   # Models cache (clear after ANY credential change so stale models don't mask it)
   rm -f ~/.config/catalyst-code/models-cache.json
   ```
   To remove a literal/env API key from config.json, edit `~/.config/catalyst-code/config.json`
   and delete the `api_key` (or the whole `providers[]` entry). Env-var keys
   (`api_key_env`) only NAME the env var; the secret lives in the shell env.

3. **Restart the core** after removing credentials — the running core caches
   the resolved provider in memory, so deletion only takes effect on the next
   `init`/restart (or a fresh TUI/web launch). Then re-run `/login`.

## Gotchas

- `~/.codex/auth.json` (official CLI) is INTENTIONALLY not read by the harness —
  removing it does NOT log the harness out. The harness's Codex store is
  `~/.config/catalyst-code/oauth/openai.json`.
- `~/.gemini/oauth_creds.json` is SHARED with gemini-cli; removing it logs BOTH out.
- Claude creds live in the official CLI's store; the harness borrows them, so
  there is no harness-owned Anthropic OAuth file to clear.
- Always clear `models-cache.json` when credentials change — the 8h TTL can make
  a fix look ineffective.
- Prefer deleting the credential FILE over editing config.json by hand when
  possible; the harness re-creates the OAuth file on next `/login`.
