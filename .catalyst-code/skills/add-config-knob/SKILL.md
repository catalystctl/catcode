---
name: add-config-knob
description: Add a new configurable setting to the Rust core (struct field, JSON, env, CLI, TUI surfacing)
version: 1
---

## When to use

You are adding a new runtime-tunable setting to the catalyst-code core — e.g. a
timeout, a limit, a toggle, a path. The core already has ~30 such knobs
(`bash_timeout_secs`, `idle_timeout_secs`, `fetch_max_bytes`, `sandbox`,
`max_session_tokens`, …) and every one was added by repeating this same
five-layer wiring. Follow it so the knob is configurable the same ways and
surfaces consistently.

## Where things live

All in `core/src/config.rs` unless noted:
- **Struct + default** — the `Config` struct field + `Default` value.
- **JSON layer** — `apply_json(c, v)` reads the field from a config file.
- **Env layer** — `load()` reads `CATALYST_CODE_*` env vars (user-owned).
- **CLI layer** — `load()` parses the `--flag` + the `HELP` constant documents it.
- **Runtime surfacing** — if the TUI must see/change it at runtime: a `set_config`
  key in `main.rs`, a `config_changed` event, and a field on the `ready` event
  (TUI: `tui/settings.go` settings store + modal, web: reducer/types).

Precedence (high→low): CLI > env > `settings.local.json` > `settings.json` >
`~/.config/catalyst-code/settings.json` > `~/.config/catalyst-code/config.json`
> `~/.config/catalyst-code/catalyst-code.d/*.json`. Arrays concat+dedupe;
objects deep-merge; `null` deletes.

## Steps

1. **Field + default** — add the field to `Config` and give it a default in
   `impl Default for Config`. Pick a sane default; document WHY in a comment.
2. **JSON** — in `apply_json`, read it: `v.get("my_knob").and_then(|x| x.as_u64())`
   (or `.as_bool()` / `.as_str()` / `.as_array()`).
3. **Env** — in `load()`, read `CATALYST_CODE_MY_KNOB` and apply (use `.parse()`
   with `unwrap_or(default)` so a bad value doesn't panic).
4. **CLI** — add a `"--my-knob" => { ... take_val ... }` arm in `load()` and a
   line in the `HELP` constant (`[env: CATALYST_CODE_MY_KNOB]`).
5. **Surface (if runtime-visible)** — (a) add a `set_config` match arm in
   `main.rs` (coerce string/number), (b) emit `config_changed` with the new
   value, (c) include it in the `ready` event so the TUI/web read it on connect.
   In the TUI, add it to `settingsStore` + the settings modal; in web, to
   `src/lib/types.ts` + the reducer if it affects the UI.
6. **Verify** — `cd core && cargo fmt --all && cargo clippy --all-targets &&
  cargo test --locked`. Add a `#[test]` in `config.rs`'s `tests` module
  (env-var save/restore pattern is already used there — copy it).

## Example (adding `fetch_max_bytes` already exists — shape shown)

```rust
// 1) struct + default
pub fetch_max_bytes: usize,            // in Config
fetch_max_bytes: 262_144,             // in Default

// 2) JSON (apply_json)
if let Some(b) = v.get("fetch_max_bytes").and_then(|x| x.as_u64()) {
    c.fetch_max_bytes = b as usize;
}

// 3) env (load)
if let Ok(v) = std::env::var("CATALYST_CODE_FETCH_MAX_BYTES") {
    if let Ok(n) = v.parse::<usize>() { c.fetch_max_bytes = n; }
}

// 4) CLI (load) + HELP
"--fetch-timeout" => { if let Some(v) = take_val(&mut i) { c.fetch_timeout_secs = v.parse().unwrap_or(c.fetch_timeout_secs); } }
//   --fetch-timeout <SECS>  Wall-clock timeout for the `fetch` tool [env: CATALYST_CODE_FETCH_TIMEOUT]
```

## Gotchas

- **Never** read a security-sensitive toggle (e.g. `trust_project_plugins`)
  from a project-local JSON file — an untrusted repo could ship `settings.json`
  to self-enable it. Keep such knobs env/CLI-only. (See trust-project-plugins-security memory.)
- Env vars that affect the sandbox/network are dead unless actually read in
  `load()` — the Dockerfile's `ENV CATALYST_CODE_SANDBOX=firejail` etc. were
  once documented but unwired (a past bug). Always add the `load()` read.
- Compile `bash_deny_regex` once at startup into `bash_deny_regex_compiled`
  (don't recompile per call) — mirror that pattern for any pre-compiled config.
