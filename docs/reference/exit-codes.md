# Exit Codes

## Core Binary (`catcode-core`)

| Code | Meaning |
|------|---------|
| `0` | Normal exit (session end, graceful shutdown) |
| `1` | General error (CLI arg parse failure, config load failure) |

The core is a stdio JSONRPC server; its exit codes are rarely seen directly because the TUI/web manage its lifecycle and report failures as events.

## TUI Binary (`catcode`)

| Code | Meaning |
|------|---------|
| `0` | Normal exit (quit key, `/exit`, `/quit`) |
| `1` | Runtime error (core crash, Bubble Tea fatal) |
| `2` | Bad CLI argument |

## Installer Scripts

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General failure |
| `2` | Unsupported platform |
| `3` | Missing dependency |
| `4` | Permission denied |

## Release Scripts

| Code | Meaning |
|------|---------|
| `0` | All platforms built successfully |
| `1` | One or more platforms failed |

> **Note:** The core does not define distinct exit codes for different failure modes. Most
> runtime errors (network issues, API auth failures, plugin crashes) are communicated via the
> JSONL protocol as error events, not exit codes.
