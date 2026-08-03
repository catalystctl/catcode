# Codex — ChatGPT OAuth

This first-party bundle connects the harness to the ChatGPT Codex backend with
the same device authorization flow used by the open-source Codex CLI. It is
staged automatically under `~/.catalyst-code/plugins/codex/`.

Use `/login` and choose **ChatGPT (Codex)**, or run:

```text
/login codex
```

The harness opens `https://chatgpt.com/codex/device`, displays the user code,
and polls the official device-token endpoint automatically. There is no
`/oauth-code` step for this provider. After authorization, the token exchange
is persisted in `~/.config/catalyst-code/oauth/codex.json` and refreshes are
performed automatically.

If the official Codex CLI has already completed a file-backed ChatGPT login,
the provider imports `$CODEX_HOME/auth.json` (or `~/.codex/auth.json`) without
starting another device flow. Keyring-only Codex CLI credentials are not
read by this plugin; running `/login codex` will start a fresh device flow in
that case. Logging out removes the harness copy and does not sign the Codex
CLI out of its own credential store.

## Official endpoints

- Device user code: `POST https://auth.openai.com/api/accounts/deviceauth/usercode`
- Device polling: `POST https://auth.openai.com/api/accounts/deviceauth/token`
- Verification page: `https://auth.openai.com/codex/device`
- Authorization-code exchange and refresh: `https://auth.openai.com/oauth/token`
- Device redirect URI: `https://auth.openai.com/deviceauth/callback`
- Codex API: `https://chatgpt.com/backend-api/codex/responses`

`CODEX_AUTH_BASE_URL` may be declared for a compatible test or staging
endpoint. It is intentionally a non-secret configuration variable.
