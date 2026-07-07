---
name: codebase-overview
description: Quickly answer "What is this codebase?" by inspecting top-level structure, README, manifests, and build entry points
version: 1
---

## When to use

Use this when the user asks for a concise orientation to an unfamiliar repository:
"What is this codebase?", "Explain this repo", "Give me a quick overview", or similar.

## Steps

1. Inspect the top-level layout with `list_dir ""`.
2. Read the main README (usually `README.md`) enough to identify the project's stated purpose, components, and run/build instructions.
3. Read package/manifests for the main stacks, such as `Cargo.toml`, `go.mod`, `package.json`, `pyproject.toml`, or similar.
4. If there are multiple apps/components, map each top-level directory to its role.
5. Answer at the level the user asked for: concise, architectural, and practical. Include build/run entry points if obvious.

## Answer shape

- One-sentence identity: "This repository is ..."
- Bullet list of major components/directories and their responsibilities.
- A short "how it works" paragraph for the main runtime flow.
- Optional build/run commands if they are directly visible.

## Avoid

- Do not over-index the whole repo unless the user asks for a deep audit.
- Do not infer unsupported business context from file names alone; prefer README/manifests.
- Do not make edits to project code for an overview-only request.
