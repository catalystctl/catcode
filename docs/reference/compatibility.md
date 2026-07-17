# Compatibility

## Supported Platforms

| Platform | TUI | Web | Hard sandbox | Installer |
|----------|:---:|:---:|:------------:|:---------:|
| Linux (x86_64) | ✅ | ✅ | `firejail` | `install.sh` (systemd) |
| Linux (aarch64) | ✅ | ✅ | `firejail` | `install.sh` (systemd) |
| macOS (x86_64) | ✅ | ✅ | `seatbelt` | `install.sh` (launchd) |
| macOS (arm64) | ✅ | ✅ | `seatbelt` | `install.sh` (launchd) |
| Windows (x86_64) | ✅ | ✅ | — | `install.ps1` (NSSM / Scheduled Task) |

## Provider Compatibility

The harness speaks two wire protocols:

| Protocol | Provider kind | Config key |
|----------|--------------|------------|
| **OpenAI-compatible** (chat completions) | `openai` | Standard OpenAI API, Umans, OpenCode Go, OpenRouter, Gemini, xAI, Groq, Together, any OpenAI-proxy |
| **Anthropic-compatible** (messages) | `anthropic` | Anthropic API (Claude) |

Provider presets (built-in):

| Preset | Kind | Env var | Notes |
|--------|------|---------|-------|
| Umans | openai | `UMANS_API_KEY` | First-party; OAuth via plugin |
| OpenCode Go | openai | `OPENCODE_API_KEY` | First-party |
| OpenRouter | openai | `OPENROUTER_API_KEY` | Model aggregator |

Custom providers can be added via `config.json` with any `base_url` and protocol `kind`.

## Build Requirements

| Component | Toolchain | Minimum version |
|-----------|-----------|-----------------|
| `core/` | Rust (stable) | 1.82 |
| `tui/` | Go | 1.25.0 |
| `web/` | Node.js or Bun | Node ≥ 20, Bun ≥ 1.x |
| `sdk/` | TypeScript / Node.js | Node ≥ 20 |

## Browser Compatibility (Web Frontend)

The web frontend runs in any modern browser with ES2022 support:
- Chrome 90+
- Firefox 90+
- Safari 16+
- Edge 90+

## Protocol

- **Transport:** newline-delimited JSON over stdio
- **Encoding:** UTF-8
- **Message format:** Each line is a complete JSON object
- **Commands:** Tagged JSON (`"type": "send"`, `"type": "init"`, …) on stdin
- **Events:** Tagged JSON (`"type": "delta"`, `"type": "tool_call"`, …) on stdout
- **Compatibility tag:** `"_session_version": 1` in session file headers

> See the [Wire Protocol Reference](../architecture/protocol.md) for the full specification.
