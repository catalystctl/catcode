# Example plugins

Install any of these with:

```text
/plugin-install docs/examples/plugins/<name> workspace
```

or globally:

```text
/plugin-install docs/examples/plugins/<name> global
```

Then `/plugin-reload` after edits. Make scripts executable (`chmod +x`).

| Plugin | What it shows |
|--------|----------------|
| `path-guard` | `pre_write` deny for `.env` / keys |
| `hello-command` | `/hello` slash command + `notify` / `status` |
| `sqlite-memory` | `memory_provider` backed by SQLite |
| `sandbox-deny-env` | `bash` tool `override` that blocks secret-touching commands |
| `grok-oauth` | Plugin-declared OAuth provider |

Also see the bundled `telemetry` and `vision-handoff` plugins under `.catalyst-code/plugins/`.
