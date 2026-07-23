---
name: cross-os-sandbox-recommendation
description: Recommend a free, lightweight, all-OS sandboxing approach for the catalyst-code harness, grounded in its existing Sandbox enum + build_bash_command router.
---

# Cross-OS Sandboxing Recommendation

## When to use
The user asks (often verbatim): "What do you recommend for sandboxing that would work on ALL OSes, that is free and lightweight to use with our harness?" This recurs frequently — do NOT re-grep from scratch; the architecture is stable and captured below.

## Grounding (verify, don't assume)
1. Read `core/src/config.rs` around the `Sandbox` enum (variants: `None`, `Firejail`, `Seatbelt`) + its `parse`/`as_str`.
2. Read `build_bash_command` in `core/src/tools.rs` (~line 1809) — the router that wraps the bash tool invocation through the chosen sandbox, returns `(Option<temp profile path>, tokio::process::Command)`, caches profiles by `(workspace, no_network)`.
3. Confirm the two gaps: (a) Windows `#[cfg(windows)]` block returns a PLAIN shell (no sandbox); (b) Linux uses external setuid `firejail` (install required) instead of in-kernel Landlock.

## The answer (stable)
**There is no single third-party sandbox binary that is free + lightweight + runs on all three OSes.** The harness's per-OS native-primitive abstraction is the right design — extend it, don't replace it with one binary.

| OS | Primitive | State |
|----|-----------|-------|
| macOS | Seatbelt / `sandbox-exec` + generated `.sb` allowlist | ✅ done, built-in, zero-install |
| Linux | **Landlock** (fs, kernel ≥5.13) + `unshare -n` (net) | ⚠️ add `Sandbox::Landlock`; keep `Firejail` as opt-in |
| Windows | **Job Object + restricted token + low integrity + per-dir ACLs**; AppContainer for net | ❌ add `Sandbox::Windows` (the real gap) |

- Linux Landlock: `landlock` crate + `Command::pre_exec` → `landlock_restrict_self()` before exec. Maps 1:1 onto the existing firejail allowlist. One-way (tighten only) — fine for per-command child. Older kernels fall back to firejail/None.
- Windows: std/tokio `Command` has NO `pre_exec` on Windows → fs/token confinement needs `CreateProcessW(CREATE_SUSPENDED)` → assign job/token → `ResumeThread` via the `windows` crate (~few hundred lines). Weaker stopgap reusing `Command`: post-spawn Job Object (process/resource containment, tiny race, no fs confinement). Network can't be blocked in pure userspace without a WFP driver → AppContainer (omit `internetClient` capability).

## Reject (and why)
- Docker/Podman — Linux VM on Mac/Win; not lightweight, install friction, poor per-command TUI fit.
- gVisor / Firecracker / bubblewrap — Linux-only.
- Windows Sandbox (`.wsb`) — Pro/Enterprise only, full VM, not per-command.
- E2B / Modal — cloud, not local-free.

## Target end state
`Sandbox::{None, Firejail, Seatbelt, Landlock, Windows}` — free, lightweight, zero-install on every OS, all behind the existing enum + `build_bash_command` router.

## Offer, don't auto-implement
This is a recommendation question. End by offering to implement `Sandbox::Landlock` first (smallest, biggest Linux win, reuses existing profile logic + `unshare -n`) or sketch the Windows Job-Object path. Do not start editing unprompted.

## Reference
Full detail + line citations: `memory action=get id:harness-sandbox-architecture`.
