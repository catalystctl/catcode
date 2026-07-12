---
name: remove-superseded-slash-key
description: Confirm `/key` was superseded by `/login`/`/logout` and keep removal complete (no slash command, docs point at /login, set_key protocol stays).
---

# `/key` superseded by `/login` / `/logout`

## Verdict

`/key` is **no longer needed**. `/login` covers API-key paste, key override, and OAuth; `/logout` clears credentials. Keep the wire `set_key` command for SDK/web/reauth — only the **slash command** was removed.

## When this comes up

User asks: "is `/key` still needed now that `/login`/`/logout` exist? remove if not."

## Checklist (verify, then fix leftovers)

1. **TUI slash handlers** — no `case "/key"` in `tui/handlers.go` / `tui/modal.go`; command palette + settings hub must not list `/key`.
2. **Modals** — no `openAPIKeyModal` / `editTargetAPIKey`; key paste lives inside `/login` provider flow.
3. **User-facing copy** — auth errors / first-run / smoke tests say `run /login`, not `run /key sk-...`.
4. **Tests** — drop `/key` modal and `/key sk-...` command tests; keep login key-entry tests.
5. **Docs/skills** — CHANGELOG documents removal; skills must not tell users to run `/key`.
6. **Do NOT remove** — protocol `set_key`, SDK `setRuntimeApiKey`, web reauth that sends `set_key`.

## Done when

`rg` for `case "/key"|label: "/key"|`/key sk-|run /key` is empty outside CHANGELOG historical notes.
