---
name: codebase-quality-opinion
description: Answer "Is this a good codebase?" with an honest, evidence-grounded verdict — gather real metrics + smell signals, reframe scary aggregates, then give strengths AND weaknesses without cheerleading
version: 1
---

# Codebase Quality Opinion

Use when the user asks for a **judgment** about a codebase — "Is this a good
codebase?", "What do you think of this repo?", "Rate this code", "Is the code
quality good?" — i.e. they want your *opinion*, not an orientation, a bug hunt,
or a launch gate.

## When to use vs. the neighbors

| Prompt shape | Skill |
|---|---|
| "What is this codebase?" / "Explain this repo" | `codebase-overview` (orientation, no audit) |
| **"Is this a good codebase?" / "What do you think of the quality?"** | **THIS skill** (lightweight judgment) |
| "Review the whole codebase for bugs" / "audit everything" | `parallel-codebase-review` (fan-out reviewers) |
| "Is this ready to go public / open-source?" | `production-readiness-review` (secrets + build gate + go/no-go) |

This skill is **single-agent and lightweight** — you gather evidence yourself
and reason to a verdict. Do NOT fan out subagents for an opinion question unless
the user escalates to a real audit.

## Core principle

Opinions must be *grounded in evidence you just gathered*, not README marketing
or stale memory. And a good codebase review is **not cheerleading** — it names a
real weakness. Lead with a direct verdict, back strengths with numbers, and
give at least one honest, specific caveat.

## Workflow

1. **Orient cheaply.** `list_dir` top level + each major component dir. Read the
   README enough to know the *stated* purpose (you'll judge reality against it,
   not repeat the pitch).

2. **Gather structural metrics.** One bash pass:
   - file count + total LOC per language (`find ... -name '*.rs' | wc -l` etc.)
   - **largest files** (`find ... -exec wc -l {} + | sort -rn | head`) — file-size
     concentration is the #1 maintainability smell.
   - test files + test LOC vs source LOC (test-to-code ratio signals discipline).
   - doc file count.

3. **Gather smell signals — but REFRAME scary aggregates.** This is the key step.
   Raw counts mislead without context:
   - `TODO|FIXME|HACK|XXX` count (density ~0 is a good sign; the *absence* of
     litter, especially with explanatory comments elsewhere, says the author cares).
   - `.unwrap()` / `.expect()` / `panic!` / `todo!()` counts — **then segment by
     test vs production paths.** A scary "815 unwraps" is fine if 783 (96%) are in
     `#[cfg(test)]` modules. Reframe the aggregate: how many are in prod paths?
     (Count unwraps in files containing `#[cfg(test)]` vs total.)
   - Sample one real source file (the `main`/entry head) to hear the author's
     voice — explanatory comments on lint suppressions, deliberate deferrals.

4. **Check the seams, not just the code.**
   - Manifests = dependency hygiene (rustls vs OpenSSL, feature gating,
     `default = []`, release LTO/codegen profile, edition/recency).
   - `git log --oneline -20` = engineering hygiene (scoped, conventionally
     prefixed, methodical commits vs. "wip"/"stuff").
   - Docs = are they substantive (architecture/guides/reference) or stubs?

5. **Verdict shape.** Direct answer first ("Yes — for what it is, genuinely
   good, with one real weakness"), then:
   - **Strengths (verified)** — each with a concrete number/evidence, not adjectives.
   - **The honest weakness** — at least one specific, actionable caveat
     (e.g. "main.rs is 11,101 lines — a god-file"). If you can't find one, look
     harder; an all-positive review is usually under-examined.
   - **Net** — one or two sentences weighing the two, with the single thing you'd
     push on if asked to improve it.

## Gotchas

- **Scary aggregate metrics are usually false alarms until segmented.** An
  `.unwrap()` count, a `// TODO` count, a high LOC file — none mean anything
  until you know where they live (test vs prod, single god-file vs many small).
  Always refine before citing a number as a weakness.
- **README ≠ reality.** Projects pitch themselves; verify the safety model,
  test ratio, and dep choices actually exist in code, then judge against the
  pitch.
- **Don't be a cheerleader.** The most useful review names the weakness
  specifically enough that the author can act on it. "It's great!" is low-value.
- **Don't over-invest.** An opinion needs enough evidence to be credible, not a
  full audit. If the user wants depth, they'll escalate — then switch to
  `parallel-codebase-review`.
- **Snapshot metrics drift.** LOC and file sizes change; if you persist any,
  mark them date-stamped and re-verify on reuse rather than asserting stale
  numbers as current fact.
