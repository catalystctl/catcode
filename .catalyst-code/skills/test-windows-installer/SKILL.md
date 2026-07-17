---
name: test-windows-installer
description: Smoke-test install.ps1 / install-web.ps1 with pwsh on Linux or Windows — catch Die/exit host-close bugs and download failures before users hit them.
---

# Test Windows installer (pwsh)

## When to use

- Changing `install.ps1`, `packaging/windows/install-web.ps1`, or download/service registration paths
- User reports PowerShell window closing mid-install (`irm | iex` / scriptblock hosts)
- Need a Linux-side check before pushing Windows packaging fixes

## Steps

1. Ensure `pwsh` is on PATH (`command -v pwsh`).
2. From repo root run:

```bash
pwsh -NoProfile -File ./packaging/windows/test-install.ps1
```

3. Expect all PASS lines. The harness covers:
   - `ParseFile` syntax for `install.ps1` + shim
   - Source guards: `Die` must `throw` (not `exit`); menu Bye must not `exit 0`; shim must not `exit $LASTEXITCODE`
   - Scriptblock host survives a `Die` throw
   - `-Help` / `-DryRun` child runs
   - Bad `-BaseUrl` fails loudly (non-zero) with a visible error
   - Scriptblock-wrapped `-Update` with empty state throws without killing the harness

## Gotchas

- Under `irm | iex` / `& ([scriptblock]::Create(...))`, `exit` closes the **entire** PowerShell window — prefer `throw` for fatal installer errors.
- `$ErrorActionPreference='Stop'` turns native stderr (e.g. missing `schtasks`) into terminating errors — use `Invoke-Native` (Continue + `$LASTEXITCODE`).
- Linux `pwsh` often has empty `$env:TEMP` — `Get-Asset` must fall back to `TMP` / `TMPDIR` / `GetTempPath()`.
- Isolate smoke-test env (`LOCALAPPDATA`/`TEMP`) so Linux runs do not write under `~/Programs`.
- Do not use `script` PTY to drive `Read-Host` menus (DSR hang); force `$canMenu=$true` + pipe stdin instead.

## Example

After editing `Install-Task` or `Die`:

```bash
pwsh -NoProfile -File ./packaging/windows/test-install.ps1
# All install.ps1 smoke tests passed.
```
