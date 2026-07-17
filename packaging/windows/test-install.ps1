<#
.SYNOPSIS
    Smoke-test install.ps1 on Linux or Windows (requires pwsh).

.DESCRIPTION
    Catches the class of bugs that close the user's PowerShell window under
    `irm | iex` / scriptblock hosts, plus basic parse + dry-run sanity.

    Run from repo root:
      pwsh -NoProfile -File ./packaging/windows/test-install.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$Installer = Join-Path $Root 'install.ps1'
$Shim = Join-Path $Root 'packaging/windows/install-web.ps1'
$failed = 0

function Pass([string]$msg) { Write-Host "  PASS  $msg" -ForegroundColor Green }
function Fail([string]$msg) {
    Write-Host "  FAIL  $msg" -ForegroundColor Red
    $script:failed++
}

Write-Host ''
Write-Host '  Catalyst Code — install.ps1 smoke tests' -ForegroundColor Cyan
Write-Host ''

# ── 1. ParseFile (syntax) ─────────────────────────────────────
foreach ($path in @($Installer, $Shim)) {
    $tokens = $null; $errs = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errs)
    if ($errs -and $errs.Count -gt 0) {
        Fail "parse $(Split-Path -Leaf $path): $($errs[0])"
    } else {
        Pass "parse $(Split-Path -Leaf $path)"
    }
}

# ── 2. Die must throw, not exit (source guard) ────────────────
$src = Get-Content -LiteralPath $Installer -Raw
if ($src -match 'function Die\([^)]*\)\s*\{[^}]*\bexit\b') {
    Fail 'Die() still contains exit — will close host under irm|iex'
} elseif ($src -notmatch 'function Die\([^)]*\)\s*\{[^}]*\bthrow\b') {
    Fail 'Die() must throw so irm|iex hosts keep the window open'
} else {
    Pass 'Die() uses throw (not exit)'
}

if ($src -match "'0'\s*\{[^}]*\bexit\s+0\b") {
    Fail "menu '0' still uses exit 0 — closes host under irm|iex"
} else {
    Pass "menu '0' does not exit the host"
}

$shimSrc = Get-Content -LiteralPath $Shim -Raw
if ($shimSrc -match '(?m)^\s*exit\s+\$LASTEXITCODE\b') {
    Fail 'install-web.ps1 still exits with LASTEXITCODE — closes host after child'
} else {
    Pass 'install-web.ps1 does not exit the host after child'
}

# ── 3. Die throws inside a scriptblock without killing the host ─
$hostSurvived = $false
try {
    $dieProbe = @'
$ErrorActionPreference = 'Stop'
function Die($t) { Write-Host "`n  error: $t" -ForegroundColor Red; throw "install failed: $t" }
Die 'probe'
'@
    try {
        & ([scriptblock]::Create($dieProbe))
        Fail 'Die probe did not throw'
    } catch {
        if ("$_" -match 'install failed: probe') {
            $hostSurvived = $true
            Pass 'scriptblock host survives Die throw'
        } else {
            Fail "unexpected Die probe error: $_"
        }
    }
} catch {
    Fail "host did not survive Die probe: $_"
}
if (-not $hostSurvived) { Fail 'Die probe host-survival check failed' }

# ── 4. -Help / -DryRun in a child process ─────────────────────
$pwsh = (Get-Command pwsh -ErrorAction Stop).Source

$help = & $pwsh -NoProfile -File $Installer -Help 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Fail "-Help exited $LASTEXITCODE"
} elseif ($help -notmatch 'Catalyst Code') {
    Fail '-Help output missing expected banner'
} else {
    Pass '-Help runs cleanly'
}

$dry = & $pwsh -NoProfile -File $Installer -DryRun -Version '0.0.0-test' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Fail "-DryRun exited $LASTEXITCODE`n$dry"
} elseif ($dry -notmatch 'dry-run') {
    Fail "-DryRun missing dry-run marker`n$dry"
} else {
    Pass '-DryRun runs without downloading'
}

# ── 5. Failed download must fail loudly ───────────────────────
$dlTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("catcode-dl-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dlTmp | Out-Null
try {
    $dl = & $pwsh -NoProfile -Command "
        \$env:TEMP = '$dlTmp'; \$env:TMP = '$dlTmp'; \$env:LOCALAPPDATA = '$dlTmp'; \$env:USERPROFILE = '$dlTmp'
        & '$Installer' -WithWeb -Version 'nosuch' -BaseUrl 'http://127.0.0.1:9'
    " 2>&1 | Out-String
    $dlCode = $LASTEXITCODE
} finally {
    Remove-Item -LiteralPath $dlTmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($dlCode -eq 0) {
    Fail 'bad BaseUrl install unexpectedly succeeded'
} elseif ($dl -notmatch 'error:|download failed|install failed') {
    Fail "bad BaseUrl did not surface a clear error (exit $dlCode)`n$dl"
} else {
    Pass "bad download fails loudly (exit $dlCode) without silent close"
}

# ── 6. Install-WebBundle prefers Windows-specific web bundle ─
$src = Get-Content -LiteralPath $Installer -Raw
if ($src -notmatch 'Windows web bundle') {
    Fail 'Install-WebBundle does not prefer a Windows-specific web bundle'
} else {
    Pass 'Install-WebBundle prefers Windows-specific web bundle'
}
if ($src -notmatch 'Installing native modules for this platform') {
    Fail 'Install-WebBundle is missing native-module fallback rebuild step'
} else {
    Pass 'Install-WebBundle will rebuild native modules as fallback'
}

# ── 7. Host survives scriptblock-wrapped Die ──────────────────
$wrappedOk = $false
$tmpHome = Join-Path ([System.IO.Path]::GetTempPath()) ("catcode-install-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmpHome | Out-Null
$prevLA = $env:LOCALAPPDATA
$prevUP = $env:USERPROFILE
try {
    $env:LOCALAPPDATA = $tmpHome
    $env:USERPROFILE = $tmpHome
    $text = Get-Content -LiteralPath $Installer -Raw
    try {
        & ([scriptblock]::Create($text)) -Update
        Fail 'scriptblock -Update with empty state should have thrown'
    } catch {
        $wrappedOk = $true
        Pass 'scriptblock host survives installer Die (Update, no state)'
    }
} catch {
    Fail "scriptblock survival test crashed the harness: $_"
} finally {
    $env:LOCALAPPDATA = $prevLA
    $env:USERPROFILE = $prevUP
    Remove-Item -LiteralPath $tmpHome -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
if ($failed -gt 0) {
    Write-Host "  $failed test(s) failed" -ForegroundColor Red
    exit 1
}
Write-Host '  All install.ps1 smoke tests passed.' -ForegroundColor Green
Write-Host ''
