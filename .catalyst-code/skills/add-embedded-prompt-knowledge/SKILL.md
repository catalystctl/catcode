---
name: add-embedded-prompt-knowledge
description: Make the compiled core "know" about a capability by embedding a concise pointer in the always-injected system prompt (not an opt-in skill, which isn't shipped). Respects the lean-standing-prompt guard.
version: 1
---

## When to use

The user says "ensure the core knows how to do X" / "make the agent aware of Y"
/ "if I ask it to <task>, it should know how." This is about the COMPILED
BINARY's standing knowledge — what every agent session starts with, in any
workspace, even a fresh install with no source and no skills on disk.

## Background — what the binary actually knows

Two layers of agent knowledge, only ONE survives compilation into a fresh
workspace:

| Layer | Embedded in binary? | Ships to fresh installs? |
|-------|---------------------|--------------------------|
| **Embedded consts** (`SYSTEM_PROMPT_BASE`, `PLUGIN_DOCS`, `PROVIDER_GUIDE`) spliced in `build_system_prompt()` (core/src/main.rs) | ✅ yes — always in the prompt | ✅ yes |
| **Opt-in skills** (`.catalyst-code/skills/<name>/SKILL.md`) | ❌ no — loaded from disk on `/skill:<name>` | ❌ no — NOT copied by install.sh/install.ps1 |

So if a capability must be recognized in ANY workspace (including a user's
fresh install with no source tree), it MUST be an embedded const. Skills are
the right place for the FULL manual / decision tree; the embedded const is the
pointer that (a) recognizes the task and (b) routes to the skill when present.

## Steps

1. **Confirm it's an embedded-knowledge need**, not a code feature. If the user
   wants the agent to *recognize* a task and *know the paths* (e.g. "add a
   provider via config or plugin"), embed a pointer. If they want new behavior,
   that's a code change (see `add-core-tool` / `add-config-knob` instead).

2. **Add a const** in `core/src/main.rs` near the other prompt consts
   (`SYSTEM_PROMPT_BASE`, `SUBAGENT_ORCHESTRATOR_STUB`, `PROVIDER_GUIDE`). Use a
   raw string `r#"..."#`. Keep it a LEAN POINTER: task recognition + the
   decision rule + the minimal actionable detail + a pointer to the skill for
   the full manual. Do NOT inline the full manual — that's what the
   `standing_prompt_stays_lean_and_defers_plugin_manual` test guards against.

3. **Inject it** in `build_system_prompt()`. Inject BEFORE the `if with_skill`
   block (unconditionally) if both the main agent AND subagents should see it
   (matches `PLUGIN_DOCS`/`PROVIDER_GUIDE`); inside `if with_skill` for
   main-only (matches `SUBAGENT_ORCHESTRATOR_STUB` + skill manifest).

4. **Update the size-guard test**
   `system_prompt_slim_tests::standing_prompt_stays_lean_and_defers_plugin_manual`
   (core/src/main.rs, near the end). It sums the always-injected consts and
   asserts they stay under a ceiling. Add `+ YOUR_CONST.len()` to the `fixed`
   sum. If the new total exceeds the current ceiling, raise the ceiling
   modestly (keep ~8–15% headroom so small future edits don't break it). The
   ceiling exists to catch accidental bloat — raising it for a deliberate,
   user-requested capability is fine.

5. **Verify**: `cargo test --bins standing_prompt_stays_lean_and_defers_plugin_manual`
   and `cargo check --bins`. Also confirm the test's negative assertions still
   hold (it asserts the prompt does NOT contain full manuals like
   `"Declaring an OAuth provider"` / `"### Hook contract"` — don't use those
   headings in your const).

6. **Point to a skill** for the full detail. If no skill exists for the
   capability's full manual, create one (`.catalyst-code/skills/<name>/SKILL.md`)
   so the embedded pointer has somewhere to defer to.

## Pitfalls

- **Don't bloat the standing prompt.** It's injected EVERY turn and lives in
  the prefix cache. A lean pointer (~1k chars) is fine; a full manual is not.
  The test name says it all: *stays lean and defers the manual*.
- **Skills are NOT shipped.** `install.sh`/`install.ps1` do not copy
  `.catalyst-code/skills/`. So "apply the X skill" is a dead pointer in a fresh
  install — the embedded const must be self-sufficient enough to act on the
  common case, with the skill as an enhancement when present.
- **Measure, don't estimate.** Const char counts are easy to misjudge (raw
  strings, arrows, code spans). After editing, measure with a quick python
  regex extract of `const NAME: &str = r#"(.*?)"#;` and `len()` before setting
  the ceiling.

## Worked example

`PROVIDER_GUIDE` (core/src/main.rs): the user said "ensure the core knows how
to add providers via config and via plugins." Added a ~1245-char const with:
task recognition ("add/connect provider X"), two no-recompile paths (config for
API-key auth, plugin `oauth` block for login flows), the decision rule, the
minimal config JSON inline (self-sufficient for the common case), and pointers
to `add-key-provider` / `plugin-authoring` skills for full schemas. Injected
unconditionally after `PLUGIN_DOCS`; ceiling raised 3500 → 4500.
