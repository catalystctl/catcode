<#
.SYNOPSIS
    Install the Catalyst Code web frontend as a background service on Windows.

.DESCRIPTION
    Builds the Next.js web frontend (and its TypeScript SDK) and installs it
    to run continuously as either:

      * a Windows Service via NSSM (preferred — starts at boot, auto-restarts
        on crash, runs with no user logged in), or
      * a Scheduled Task at logon with a restart-loop wrapper (zero extra
        dependencies — runs only while a user is logged in).

    catcode-core.exe must already be installed (via the MSI or
    packaging/windows/install.ps1) so the web server can spawn it. This
    script is the Windows counterpart of `install.sh --with-web` (Linux
    systemd / macOS launchd).

.PARAMETER Port
    Port the web server binds. Default 49283.

.PARAMETER BindHost
    Bind host. Default 0.0.0.0 (all interfaces). For public exposure, put a
    reverse proxy (caddy/nginx/IIS) with TLS in front and bind 127.0.0.1.

.PARAMETER RepoDir
    Path to the catalyst-code repo checkout. Auto-detected if this script runs
    from inside the checkout (it walks up from its own location).

.PARAMETER CatcodeCore
    Path to catcode-core.exe. Auto-detected in order: this flag, the
    CATCODE_CORE env var, catcode-core.exe on PATH, then
    %LOCALAPPDATA%\Programs\catcode\catcode-core.exe.

.PARAMETER Uninstall
    Stop and remove the web service/task instead of installing it.

.EXAMPLE
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1 -Port 8080 -BindHost 127.0.0.1
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1 -Uninstall

.NOTES
    The web frontend is a Next.js app and must be built from source, so this
    script runs from a checkout of the catalyst-code repo (like install.sh).
    Requires Bun (https://bun.sh) or Node.js + npm. NSSM (https://nssm.cc)
    is optional but recommended for a true boot-time service.
#>
[CmdletBinding()]
param(
    [int]$Port = 49283,
    [string]$BindHost = '0.0.0.0',
    [string]$RepoDir = '',
    [string]$CatcodeCore = '',
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$SvcName = 'CatalystCodeWeb'
$TaskName = 'CatalystCodeWeb'
$DataDir = Join-Path $env:LOCALAPPDATA 'catalyst-code'
$WrapperPath = Join-Path $DataDir 'run-web.cmd'
$LogPath = Join-Path $DataDir 'catalyst-code-web.log'

# runtime: 'bun' or 'npm' (set in Detect-Runtime)
$RT = $null       # 'bun' | 'npm'
$RTExe = $null     # absolute path to the runtime exe

function Write-Info($t) { Write-Host "  $t" -ForegroundColor Cyan }
function Write-Ok($t)   { Write-Host "  $t" -ForegroundColor Green }
function Write-Warn2($t){ Write-Host "  $t" -ForegroundColor Yellow }
function Die($t) { Write-Host "`n  error: $t" -ForegroundColor Red; exit 1 }

# --- resolve repo root ------------------------------------------------------
function Find-RepoRoot {
    $d = $PSScriptRoot
    while ($d -and $d -ne (Split-Path $d -Parent)) {
        if ((Test-Path (Join-Path $d 'core' 'Cargo.toml')) -and (Test-Path (Join-Path $d 'build.sh'))) {
            return $d
        }
        $d = Split-Path $d -Parent
    }
    return $null
}

# --- detect bun or npm ------------------------------------------------------
function Detect-Runtime {
    $bun = Get-Command bun -ErrorAction SilentlyContinue
    if ($bun) { $script:RT = 'bun'; $script:RTExe = $bun.Source; return }
    $npm = Get-Command npm -ErrorAction SilentlyContinue
    if ($npm) {
        $node = Get-Command node -ErrorAction SilentlyContinue
        if (-not $node) { Die 'npm found but node is not on PATH — install Node.js' }
        $script:RT = 'npm'; $script:RTExe = $npm.Source; return
    }
    Die 'neither bun nor npm found — install one (https://bun.sh or https://nodejs.org)'
}

# --- resolve catcode-core.exe ----------------------------------------------
function Find-CoreExe {
    if ($CatcodeCore -and (Test-Path -LiteralPath $CatcodeCore)) { return $CatcodeCore }
    if ($env:CATCODE_CORE -and (Test-Path -LiteralPath $env:CATCODE_CORE)) { return $env:CATCODE_CORE }
    $g = Get-Command catcode-core.exe -ErrorAction SilentlyContinue
    if ($g) { return $g.Source }
    $p = Join-Path $env:LOCALAPPDATA 'Programs\catcode\catcode-core.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    return $null
}

# --- build SDK + web --------------------------------------------------------
function Build-Web {
    Write-Host ''
    Write-Host 'Building web frontend (SDK + Next.js)' -ForegroundColor Cyan
    Detect-Runtime
    Write-Ok "Runtime: $RT ($RTExe)"

    Push-Location (Join-Path $RepoDir 'sdk')
    try {
        Write-Info 'Installing SDK deps...'
        & $RTExe install *> $null
        if ($LASTEXITCODE -ne 0) { Die 'SDK dependency install failed' }
        Write-Info 'Building SDK (tsc)...'
        & $RTExe run build *> $null
        if ($LASTEXITCODE -ne 0) { Die 'SDK build failed (sdk/dist/)' }
    } finally { Pop-Location }

    Push-Location (Join-Path $RepoDir 'web')
    try {
        Write-Info 'Installing web deps...'
        & $RTExe install *> $null
        if ($LASTEXITCODE -ne 0) { Die 'web dependency install failed' }
        Write-Info 'Building web (next build)...'
        $env:NEXT_TELEMETRY_DISABLED = '1'
        & $RTExe run build
        if ($LASTEXITCODE -ne 0) { Die 'web build failed (next build)' }
    } finally { Pop-Location }
    Write-Ok 'Web build complete'
}

# --- NSSM service path -------------------------------------------------------
function Install-NssmService {
    $nssm = Get-Command nssm -ErrorAction SilentlyContinue
    if (-not $nssm) { return $false }
    $nssm = $nssm.Source

    # Remove a prior service of the same name (idempotent reinstall).
    & $nssm stop $SvcName *> $null 2>&1
    & $nssm remove $SvcName confirm *> $null 2>&1

    Write-Info "Installing Windows Service '$SvcName' (NSSM)..."
    & $nssm install $SvcName $RTExe 'run' 'start' '--' '--hostname' $BindHost *> $null
    if ($LASTEXITCODE -ne 0) { Die "nssm install failed (exit $LASTEXITCODE)" }
    & $nssm set $SvcName AppDirectory (Join-Path $RepoDir 'web') *> $null
    & $nssm set $SvcName AppEnvironmentExtra "NODE_ENV=production" "PORT=$Port" "CATCODE_CORE=$CoreExe" *> $null
    & $nssm set $SvcName AppStdout $LogPath *> $null
    & $nssm set $SvcName AppStderr $LogPath *> $null
    & $nssm set $SvcName AppRotateFiles 1 *> $null
    & $nssm set $SvcName AppRotateBytes 10485760 *> $null
    & $nssm set $SvcName AppRestartDelay 3000 *> $null
    & $nssm set $SvcName Start 'SERVICE_AUTO_START' *> $null
    & $nssm start $SvcName *> $null
    if ($LASTEXITCODE -ne 0) { Die "nssm start failed — check: nssm get $SvcName AppStdout ; Get-Content $LogPath" }
    Write-Ok "Service '$SvcName' installed and started (NSSM, auto-start at boot)"
    return $true
}

# --- scheduled-task fallback (zero-dependency) -------------------------------
function Write-Wrapper {
    $webDir = Join-Path $RepoDir 'web'
    # A restart-loop wrapper: re-launches `next start` 3s after it exits, so a
    # crash is recovered without NSSM. env vars are set here because scheduled
    # tasks do not reliably inherit the user environment.
    $cmd = @(
        '@echo off',
        "set NODE_ENV=production",
        "set PORT=$Port",
        "set CATCODE_CORE=$CoreExe",
        ':loop',
        "cd /d `"$webDir`"",
        "`"$RTExe`" run start -- --hostname $BindHost",
        'echo [%date% %time%] web exited, restarting in 3s...',
        'timeout /t 3 /nobreak >nul',
        'goto loop'
    ) -join "`r`n"
    if (-not (Test-Path $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
    [System.IO.File]::WriteAllText($WrapperPath, $cmd)
}

function Install-Task {
    Write-Info 'NSSM not found — installing as a Scheduled Task at logon...'
    Write-Warn2 'Note: a Scheduled Task runs only while a user is logged in.'
    Write-Warn2 '      For a true boot-time service, install NSSM (https://nssm.cc) and re-run.'
    Write-Wrapper
    # Remove a prior task of the same name.
    schtasks /query /tn $TaskName *> $null 2>&1
    if ($LASTEXITCODE -eq 0) { schtasks /delete /tn $TaskName /f *> $null }
    schtasks /create /tn $TaskName /tr "`"$WrapperPath`"" /sc onlogon /rl limited /f *> $null
    if ($LASTEXITCODE -ne 0) { Die "schtasks /create failed (exit $LASTEXITCODE)" }
    schtasks /run /tn $TaskName *> $null
    Write-Ok "Scheduled task '$TaskName' created and started (at logon, restart-loop wrapper)"
}

# --- uninstall --------------------------------------------------------------
function Do-Uninstall {
    Write-Host ''
    Write-Host 'Removing web service' -ForegroundColor Cyan
    $nssm = Get-Command nssm -ErrorAction SilentlyContinue
    $removed = $false
    if ($nssm) {
        $nssm = $nssm.Source
        & $nssm stop $SvcName *> $null 2>&1
        & $nssm remove $SvcName confirm *> $null 2>&1
        if ($LASTEXITCODE -eq 0) { Write-Ok "Removed Windows Service '$SvcName'"; $removed = $true }
    } else {
        # No NSSM: try the service via sc.exe in case it exists from a prior NSSM install.
        sc.exe stop $SvcName *> $null 2>&1
        sc.exe delete $SvcName *> $null 2>&1
    }
    schtasks /query /tn $TaskName *> $null 2>&1
    if ($LASTEXITCODE -eq 0) {
        schtasks /end /tn $TaskName *> $null 2>&1
        schtasks /delete /tn $TaskName /f *> $null
        Write-Ok "Removed Scheduled Task '$TaskName'"; $removed = $true
    }
    if (Test-Path $WrapperPath) { Remove-Item $WrapperPath -Force; Write-Ok "Removed wrapper $WrapperPath" }
    if (-not $removed) { Write-Warn2 'No service or task found to remove (already clean?)' }
    Write-Host ''
    Write-Host '  Web service removed. catcode/catcode-core were left installed.' -ForegroundColor Green
    exit 0
}

# --- main -------------------------------------------------------------------
if (-not $RepoDir) {
    $RepoDir = Find-RepoRoot
}
if (-not $RepoDir -or -not (Test-Path (Join-Path $RepoDir 'web'))) {
    Die 'could not locate the repo (no web/ found). Run from inside the checkout or pass -RepoDir <path>.'
}
$RepoDir = (Resolve-Path $RepoDir).Path
Write-Host ''
Write-Host '  Catalyst Code — web frontend service installer (Windows)' -ForegroundColor Cyan
Write-Host "  repo:   $RepoDir"
Write-Host "  port:   $Port   host: $BindHost"

if ($Uninstall) { Do-Uninstall }

Detect-Runtime
$CoreExe = Find-CoreExe
if (-not $CoreExe) {
    Die @'
catcode-core.exe not found. Install it first via the MSI or:
    pwsh -ExecutionPolicy Bypass -File packaging\windows\install.ps1
Or pass -CatcodeCore <path> (or set CATCODE_CORE).
'@
}
Write-Ok "Core: $CoreExe"

Build-Web

if (-not (Install-NssmService)) { Install-Task }

$access = if ($BindHost -eq '0.0.0.0') { 'all interfaces' } else { $BindHost }
Write-Host ''
Write-Host "  Done. Web frontend running at http://localhost:$Port ($access)" -ForegroundColor Green
Write-Host "  logs:   $LogPath"
Write-Host "  update: re-run this script (it rebuilds + restarts)"
Write-Host "  stop:   nssm stop $SvcName  (or:  schtasks /end /tn $TaskName)"
Write-Host '  auth:   ensure a key/login exists (~/.config/catalyst-code/settings.json) or set UMANS_API_KEY.'
if ($BindHost -ne '127.0.0.1') {
    Write-Warn2 "Bound to $BindHost — put a TLS reverse proxy in front for public use."
}
