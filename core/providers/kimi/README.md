# Kimi (Moonshot) — Official OAuth Provider

First-party provider for **Kimi Code** (Moonshot AI) subscription access. Logs
in with a Kimi account via OAuth 2.0 **device-code** flow; no API key required.

This plugin is staged into every install at first run
(`~/.catalyst-code/plugins/kimi/`) — it ships with the harness, so it is
officially supported and requires no separate user install.

## Usage

```
/login kimi          # start device-code login (opens a verification URL)
/oauth-code kimi     # finish after authorizing in the browser
/logout kimi         # remove credentials + provider config
```

After login the provider appears in `/models` as `kimi-for-coding`
(256K context, reasoning + vision). Models auto-discover from the live
`https://api.kimi.com/coding/v1/models` endpoint.

## How it works

- **Flow**: OAuth 2.0 device code (RFC 8628) against `auth.kimi.com`.
  `login` returns the verification URL + a `pending` blob holding the
  `device_code`; the harness shows the prompt and waits for `/oauth-code`;
  `complete` polls the token endpoint until approved and writes the token.
- **Token storage**: `~/.config/catalyst-code/oauth/kimi.json` (atomic, `0600`).
  A `flock` sidecar serializes refresh across concurrent harness processes so a
  rotated refresh token is never clobbered.
- **Per-request headers**: the `X-Msh-*` device-identity headers
  (`User-Agent: KimiCLI/<ver>`, `X-Msh-Platform`, `X-Msh-Device-Id`, …) are
  returned by the `token` action and merged onto every turn by the harness.
- **Thinking**: Kimi uses a dual mechanism — top-level `reasoning_effort`
  **and** `thinking: {type: enabled|disabled}`. The harness injects both
  (gated by `provider::is_kimi`) when an effort > none is selected.

## Constants

| Constant | Value |
|----------|-------|
| Client ID | `17e5f671-d194-4dfb-9706-5516cb48c098` |
| Scope | `kimi_for_coding` |
| Device auth | `https://auth.kimi.com/api/oauth/device_authorization` |
| Token | `https://auth.kimi.com/api/oauth/token` |
| API base | `https://api.kimi.com/coding/v1` |
| Refresh threshold | 300s before expiry |
| Compat version | `1.12.0` (env `KIMI_CODE_CLI_VERSION`) |

Sources: the official `kimi` CLI (MoonshotAI/kimi-code) and the
`opencode-kimicode-auth` reference implementation.
