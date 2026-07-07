---
name: test-ask-tool
description: Exercise the ask tool (blocking model→user question flyout) across all its shapes — select, text, required/optional, allowCustom, multi-question, skip.
---

# Test the ask tool

Use when the user says "test the ask tool" / "can you test ask?" / "verify the ask
flyout". The `ask` tool blocks the agent loop until the user answers (or
skips/aborts). See memory `ask-tool-feature` for the architecture.

## When to use
- User asks to test/verify the ask tool or the question flyout.
- After changes to `core/src/tools.rs` (ask schema), `main.rs` (`request_ask` /
  `PendingAsk` / dispatch), the TUI ask render, or the web `AskPrompt` component.

## Test matrix (cover all in one `ask` call)

Send a single `ask` with these questions to exercise every shape:

1. **select, required** — `type:"select"`, `required:true`, 3-4 `options`. Verifies
   the multiple-choice flyout renders and a pick returns.
2. **select, allowCustom** — `type:"select"`, `allowCustom:true`, a few options.
   Verifies the user can type a custom answer instead of picking.
3. **text, required** — `type:"text"`, `required:true`, with a `placeholder`.
   Verifies the free-text box.
4. **text, optional** — `type:"text"`, `required:false`. Verifies the user can
   SKIP this one (answer comes back as skipped) without aborting the whole ask.
5. **select with one option** — edge case: a single-option select still renders
   and is answerable.

Keep option text short. Use distinct, stable `id` values (no duplicates — the
validator rejects them).

## Steps
1. Call `ask` with the matrix above (one call, 5 questions).
2. The flyout appears for the user. Wait for the answer (the tool blocks).
3. On return, report per-question: the id, the prompt, and the answer (or
   "(skipped)" / "(aborted)"). Confirm required questions got real answers and
   optional ones could be skipped.
4. If testing skip/abort paths: ask the user to hit skip on the optional question
   and (separately) to dismiss the whole flyout to verify the Aborted path
   emits the expected "user aborted" outcome and run_turn exits cleanly.

## What "pass" looks like
- Every question type renders correctly in the flyout (select list vs text box).
- `allowCustom` lets the user type a non-listed answer.
- Required questions can't be left empty; optional ones can be skipped.
- Answers are echoed back keyed by `id` in the tool result.
- Aborting the flyout ends the turn without leaving an orphaned tool_call (the
  sanitizer cleans it next turn — see memory `orphaned-tool-call-400-fix`).

## Scope note
This tests the END-TO-END ask path (schema → dispatch → flyout → answer echo).
For pure schema validation without a user, see `core/src/tools.rs`
`validate_ask_questions` (rejects dup ids, empty options, bad type). Do NOT call
`ask` in an unattended/automated context expecting it to self-answer — it blocks.
