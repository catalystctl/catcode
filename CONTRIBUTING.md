# Contributing to Catalyst Code

Thanks for your interest in contributing! Catalyst Code is a coding-agent
harness made of four cooperating components around a single stdio JSONL
protocol.

## Architecture at a glance

| Component | Language | Role |
|-----------|----------|------|
| `core/` | Rust (tokio) | The engine: conversation, model streaming, tools, sessions, plugins, subagents |
| `tui/` | Go (Bubble Tea) | Terminal UI; spawns `core` and speaks JSONL |
| `sdk/` | TypeScript | Thin pi-compatible wrapper so pi-web can swap in the harness |
| `web/` | Next.js 15 + React 19 | Web equivalent of the TUI |

## Getting started

Build the core and TUI from the repo root:

```sh
./build.sh            # cargo build --release (core) + go build (tui)
```

Or individually:

```sh
cargo build --release --manifest-path core/Cargo.toml   # → core/target/release/core
cd tui && go build -o catcode .                          # → tui/catcode
```

The web frontend has its own setup (`cd web && bun install && bun run dev`).

## Before opening a PR

**Core (Rust):**

```sh
cd core
cargo fmt --all -- --check
cargo clippy --all-targets
cargo test --locked
```

**TUI (Go):**

```sh
cd tui
gofmt -l .            # must be empty
go vet ./...
go test ./...
go build ./...
```

CI runs all of the above, so run them locally first to save a round-trip.

## Code style

- **Rust:** `cargo fmt` (rustfmt default). Avoid `unwrap()`/`expect()` on data
  that comes from the model, files, or the network — prefer `?`/`unwrap_or`/
  explicit error returns so adversarial input can't crash the core.
- **Go:** `gofmt -s`. The TUI is single-threaded `Update` + channel-only
  goroutines; keep shared state behind the `session` model and communicate via
  channels/`tea.Cmd`, not shared mutable globals.

## Security notes for contributors

- File tools confine paths to the workspace (`..`/absolute/symlink escapes are
  rejected). Don't add a path-handling tool that bypasses `workspace::resolve`.
- The `bash` tool runs under an optional sandbox (`--sandbox firejail`,
  `--no-network`). Treat the denylist as a tripwire, not a sandbox.
- Secrets: never log API keys or OAuth tokens. The `set_key` path logs only the
  provider name, never the key. Config files holding keys are written `0600`.
- Plugins from a repo's `.catalyst-code/plugins/` load only with an explicit
  `--trust-project-plugins` opt-in — never read that flag from a config file a
  repo could ship.

## Commit messages

Use a short, imperative subject (`add fetch SSRF hardening`, not `added`).
Reference the issue/PR number in the body when relevant.

## Reporting security issues

Please do **not** open a public issue for security vulnerabilities. See
`SECURITY.md` if present, or contact the maintainers privately.

## License

By contributing, you agree your contributions are licensed under the MIT
License (see `LICENSE`).
