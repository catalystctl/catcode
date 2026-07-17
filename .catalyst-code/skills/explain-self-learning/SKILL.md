---
name: explain-self-learning
description: Answer "What's good about self-learning/memory? Where do we see benefits?" from architecture memories — no re-derivation
version: 1
triggers:
  - self learning
  - memory system
  - what's so good
  - where will we see benefits
---

## When to use

Use when the user asks what is valuable about Catalyst's self-learning / memory system, where benefits show up, or similar product/architecture pitch questions (recurring across sessions).

## Steps

1. **Pull architecture memories first** (do not invent from scratch):
   - `memory` action=get `self-learning-system-architecture`
   - `memory` action=get `learning-layer-turn-loop-wiring`
   - `memory` action=get `codebase-intelligence-phase1-2`
   - Optionally `global-memory-scope` if cross-project scope comes up
2. Answer from those facts. Keep it user-facing (benefits + when you feel them), not a module dump.
3. Be honest about known gaps (e.g. `failure_atlas` / `rejected_approaches` read-wired but thinly populated in production).

## Answer shape

- One-line thesis: durable facts + project structure + past outcomes inject into the live turn loop — later sessions start smarter without re-teaching.
- **What's good** (short bullets): scoped memory + hygiene, auto-reflect, per-turn context pack / index / coupling, stable project identity, local/offline.
- **Where you feel it**: returning to a repo; cross-project globals; recurring task shapes; large/unfamiliar codebases; subagents with role packs; preference stickiness.
- One honest caveat on population gaps if relevant.

## Avoid

- Do not re-walk `core/src` unless memories look stale vs code.
- Do not pitch "the model remembers chat" — that is not the win.
- Do not invent embedding/hosted-sync claims; retrieval is lexical/hybrid and local.
