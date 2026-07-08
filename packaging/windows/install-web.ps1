<#
.SYNOPSIS
    Install the Catalyst Code web frontend as a background service on Windows.

.DESCRIPTION
    DEFAULT: download a prebuilt web bundle (catcode-web-<ver>.tar.gz) and a
    prebuilt catcode-core.exe, then run the web continuously as either:

      * a Windows Service via NSSM (preferred — starts at boot, auto-restarts
        on crash, runs with no user logged in), or
      * a Scheduled Task at logon with a restart-loop wrapper (zero extra
        dependencies — runs only while a user is logged in).

    No compiler (cargo/go/next build) is needed. The web bundle is the Next.js
    standalone output; it only needs Node OR Bun present to RUN (not to build).

    catcode-core.exe is auto-installed (from catcode-<ver>-windows.zip) if it
    is not already on PATH / in the default install dir.

    -BuildFromSource falls back to the old repo-checkout + `next build` path.

.PARAMETER Port
    Port the web server binds. Default 49283.

.PARAMETER BindHost
    Bind host. Default 0.0.0.0 (all interfaces). For public exposure, put a
    reverse proxy (caddy/nginx/IIS) with TLS in front and bind 127.0.0.1.

.PARAMETER Version
    Release version to download (e.g. "0.2.0" or "v0.2.0"). Default: latest.

.PARAMETER BaseUrl
    Download base URL override (default: GitHub Releases for the resolved tag).

.PARAMETER WebDir
    Where to extract the web bundle. Default:
    %LOCALAPPDATA%\catalyst-code\web.

.PARAMETER CatcodeCore
    Path to catcode-core.exe. Auto-detected; downloaded if missing.

.PARAMETER RepoDir
    (Source path only) path to the catalyst-code repo checkout.

.PARAMETER BuildFromSource
    Build the web from a repo checkout instead of downloading the prebuilt bundle.

.PARAMETER Uninstall
    Stop and remove the web service/task instead of installing.

.EXAMPLE
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1 -Port 8080 -BindHost 127.0.0.1
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1 -Version 0.2.0
    pwsh -ExecutionPolicy Bypass -File .\install-web.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [int]$Port = 49283,
    [string]$BindHost = '0.0.0.0',
    [string]$Version = '',
    [string]$BaseUrl = '',
    [string]$WebDir = '',
    [string]$RepoDir = '',
    [string]$CatcodeCore = '',
    [switch]$BuildFromSource,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$Repo = 'catalystctl/catcode'
$SvcName = 'CatalystCodeWeb'
$TaskName = 'CatalystCodeWeb'
$DataDir = Join-Path $env:LOCALAPPDATA 'catalyst-code'
if (-not $WebDir)  { $WebDir  = Join-Path $DataDir 'web' }
if (-not $RepoDir) { $RepoDir = $PSScriptRoot }
$WrapperPath = Join-Path $DataDir 'run-web.cmd'
$LogPath = Join-Path $DataDir 'catalyst-code-web.log'
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\catcode'

# runtime: 'bun' | 'node'  (set in Detect-Runtime)
$RT = $null
$RTExe = $null

function Write-Info($t) { Write-Host "  $t" -ForegroundColor Cyan }
function Write-Ok($t)   { Write-Host "  $t" -ForegroundColor Green }
function Write-Warn2($t){ Write-Host "  $t" -ForegroundColor Yellow }
function Die($t) { Write-Host "`n  error: $t" -ForegroundColor Red; exit 1 }

# --- detect bun or node (to RUN the web) -----------------------------------
function Detect-Runtime {
    $bun = Get-Command bun -ErrorAction SilentlyContinue
    if ($bun) { $script:RT = 'bun'; $script:RTExe = $bun.Source; return }
    $node = Get-Command node -ErrorAction SilentlyContinue
    if ($node) { $script:RT = 'node'; $script:RTExe = $node.Source; return }
    Die 'neither bun nor node found — install one to RUN the web frontend (https://bun.sh or https://nodejs.org)'
}

# --- resolve release version + base URL ------------------------------------
function Resolve-Release {
    if ($Version) {
        $script:Tag = $Version
        if (-not $script:Tag.StartsWith('v')) { $script:Tag = "v$($script:Tag)" }
        $script:Ver = $script:Tag.Substring(1)
    } else {
        $api = "https://api.github.com/repos/$Repo/releases/latest"
        try {
            $rel = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'catcode-installer' } -ErrorAction Stop
            $script:Tag = $rel.tag_name
            $script:Ver = $script:Tag.Substring(1)
        } catch {
            Die "could not resolve the latest release from $api. The repo may be private or rate-limited. Pass -Version <v> or -BaseUrl <url>."
        }
    }
    if ($BaseUrl) {
        $script:Base = $BaseUrl.TrimEnd('/')
    } else {
        $script:Base = "https://github.com/$Repo/releases/download/$($script:Tag)"
    }
}

# --- download + verify sha256 ----------------------------------------------
function Get-Asset {
    param([string]$Name)
    $url = "$($script:Base)/$Name"
    $dest = Join-Path $env:TEMP $Name
    Write-Info "Downloading $Name ..."
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    # checksum
    $shaUrl = "$url.sha256"
    $shaDest = "$dest.sha256"
    Invoke-WebRequest -Uri $shaUrl -OutFile $shaDest -UseBasicParsing
    $expected = (Get-Content $shaDest -Raw).Trim().Split(' ')[0]
    $actual = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { Die "checksum mismatch for $Name (expected $expected, got $actual)" }
    Write-Ok "Verified $Name"
    return $dest
}

# --- ensure catcode-core.exe is present (download zip if missing) ----------
function Ensure-CoreExe {
    # 1) explicit -CatcodeCore
    if ($CatcodeCore -and (Test-Path -LiteralPath $CatcodeCore)) { return $CatcodeCore }
    # 2) env
    if ($env:CATCODE_CORE -and (Test-Path -LiteralPath $env:CATCODE_CORE)) { return $env:CATCODE_CORE }
    # 3) on PATH
    $g = Get-Command catcode-core.exe -ErrorAction SilentlyContinue
    if ($g) { return $g.Source }
    # 4) default install dir
    $p = Join-Path $InstallDir 'catcode-core.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    # 5) download catcode-<ver>-windows.zip (catcode.exe + catcode-core.exe + install.ps1)
    Write-Info 'catcode-core.exe not found — downloading the Windows bundle ...'
    $zip = Get-Asset "catcode-$($script:Ver)-windows.zip"
    if (-not (Test-Path -LiteralPath $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null }
    Expand-Archive -LiteralPath $zip -DestinationPath $InstallDir -Force
    $core = Join-Path $InstallDir 'catcode-core.exe'
    if (-not (Test-Path -LiteralPath $core)) { Die "downloaded zip did not contain catcode-core.exe" }
    # add to user PATH (idempotent) so `catcode` works too
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $path) { $path = '' }
    $parts = @($path.Split(';') | Where-Object { $_ -ne '' })
    if ($parts -notcontains $InstallDir) {
        $newPath = (($parts + $InstallDir) -join ';')
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Ok "Added $InstallDir to user PATH."
    }
    Write-Ok "Installed catcode-core.exe -> $core"
    return $core
}

# --- extract the prebuilt web bundle ---------------------------------------
function Install-WebBundle {
    $tgz = Get-Asset "catcode-web-$($script:Ver).tar.gz"
    if (-not (Test-Path -LiteralPath $WebDir)) { New-Item -ItemType Directory -Path $WebDir -Force | Out-Null }
    Get-ChildItem -LiteralPath $WebDir -Force | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    # Windows 10+ ships tar (bsdtar); it handles .tar.gz natively.
    $tar = Get-Command tar -ErrorAction SilentlyContinue
    if (-not $tar) { Die 'tar not found (Windows 10 1803+ ships it). Extract the .tar.gz manually or install tar.' }
    Write-Info "Extracting web bundle -> $WebDir ..."
    & tar -xzf $tgz -C $WebDir
    if ($LASTEXITCODE -ne 0) { Die "tar extraction failed (exit $LASTEXITCODE)" }
    $startJs = Join-Path $WebDir 'start.js'
    if (-not (Test-Path -LiteralPath $startJs)) { Die "web bundle missing start.js (extraction failed?)" }
    Write-Ok "Web bundle extracted to $WebDir"
}

# --- NSSM service -----------------------------------------------------------
function Install-NssmService {
    $nssm = Get-Command nssm -ErrorAction SilentlyContinue
    if (-not $nssm) { return $false }
    $nssm = $nssm.Source
    & $nssm stop $SvcName *> $null 2>&1
    & $nssm remove $SvcName confirm *> $null 2>&1
    Write-Info "Installing Windows Service '$SvcName' (NSSM)..."
    & $nssm install $SvcName $RTExe (Join-Path $WebDir 'start.js') *> $null
    if ($LASTEXITCODE -ne 0) { Die "nssm install failed (exit $LASTEXITCODE)" }
    & $nssm set $SvcName AppDirectory $WebDir *> $null
    & $nssm set $SvcName AppEnvironmentExtra "NODE_ENV=production" "PORT=$Port" "HOSTNAME=$BindHost" "CATCODE_CORE=$CoreExe" *> $null
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

# --- scheduled-task fallback (zero-dependency) ------------------------------
function Write-Wrapper {
    # restart-loop wrapper: re-launches `node start.js` 3s after it exits.
    $cmd = @(
        '@echo off',
        "set NODE_ENV=production",
        "set PORT=$Port",
        "set HOSTNAME=$BindHost",
        "set CATCODE_CORE=$CoreExe",
        ':loop',
        "cd /d `"$WebDir`"",
        "`"$RTExe`" `"$WebDir\start.js`"",
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
    schtasks /query /tn $TaskName *> $null 2>&1
    if ($LASTEXITCODE -eq 0) { schtasks /delete /tn $TaskName /f *> $null }
    schtasks /create /tn $TaskName /tr "`"$WrapperPath`"" /sc onlogon /rl limited /f *> $null
    if ($LASTEXITCODE -ne 0) { Die "schtasks /create failed (exit $LASTEXITCODE)" }
    schtasks /run /tn $TaskName *> $null
    Write-Ok "Scheduled task '$TaskName' created and started (at logon, restart-loop wrapper)"
}

# --- build-from-source fallback (old path) ---------------------------------
function Build-Web-FromSource {
    Write-Host ''
    Write-Host 'Building web frontend from source (SDK + Next.js)' -ForegroundColor Cyan
    $rt = $null; $rtExe = $null
    $bun = Get-Command bun -ErrorAction SilentlyContinue
    if ($bun) { $rt = 'bun'; $rtExe = $bun.Source }
    else {
        $npm = Get-Command npm -ErrorAction SilentlyContinue
        if ($npm) { $node = Get-Command node -ErrorAction SilentlyContinue; if (-not $node) { Die 'npm found but node is not on PATH' }; $rt = 'npm'; $rtExe = $npm.Source }
        else { Die 'neither bun nor npm found (https://bun.sh or https://nodejs.org)' }
    }
    Write-Ok "Runtime: $rt ($rtExe)"
    Push-Location (Join-Path $RepoDir 'sdk')
    try { & $rtExe install *> $null; if ($LASTEXITCODE -ne 0) { Die 'SDK dep install failed' }; & $rtExe run build *> $null; if ($LASTEXITCODE -ne 0) { Die 'SDK build failed' } }
    finally { Pop-Location }
    Push-Location (Join-Path $RepoDir 'web')
    try { & $rtExe install *> $null; if ($LASTEXITCODE -ne 0) { Die 'web dep install failed' }; $env:NEXT_TELEMETRY_DISABLED = '1'; & $rtExe run build; if ($LASTEXITCODE -ne 0) { Die 'web build failed' } }
    finally { Pop-Location }
    # source path runs `next start` from the repo web dir
    $script:WebDir = Join-Path $RepoDir 'web'
    $script:RT = $rt; $script:RTExe = $rtExe
    Write-Ok 'Web build complete'
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
    if (Test-Path -LiteralPath $WebDir) { Remove-Item -LiteralPath $WebDir -Recurse -Force -ErrorAction SilentlyContinue; Write-Ok "Removed web bundle $WebDir" }
    if (-not $removed) { Write-Warn2 'No service or task found to remove (already clean?)' }
    Write-Host ''
    Write-Host '  Web service removed. catcode/catcode-core were left installed.' -ForegroundColor Green
    exit 0
}

# --- main -------------------------------------------------------------------
Write-Host ''
Write-Host '  Catalyst Code — web frontend service installer (Windows)' -ForegroundColor Cyan

if ($Uninstall) { Do-Uninstall }

if ($BuildFromSource) {
    if (-not (Test-Path (Join-Path $RepoDir 'web'))) { Die "source path needs a repo checkout with web/ (run from inside it or pass -RepoDir). RepoDir=$RepoDir" }
    Write-Host "  mode: build-from-source (repo: $RepoDir)" -ForegroundColor DarkGray
    Detect-Runtime
    $CoreExe = $CatcodeCore
    if (-not $CoreExe -or -not (Test-Path -LiteralPath $CoreExe)) {
        $g = Get-Command catcode-core.exe -ErrorAction SilentlyContinue
        if ($g) { $CoreExe = $g.Source } else { Die 'catcode-core.exe not found. Install it first (MSI or install.ps1) or pass -CatcodeCore.' }
    }
    Build-Web-FromSource
} else {
    Write-Host "  mode: download (prebuilt, no compile)" -ForegroundColor DarkGray
    Resolve-Release
    Write-Host "  version: $($script:Ver)   base: $($script:Base)"
    Detect-Runtime
    $CoreExe = Ensure-CoreExe
    Write-Ok "Core: $CoreExe"
    Install-WebBundle
}

Write-Host ''
Write-Host "  web dir: $WebDir" -ForegroundColor DarkGray
Write-Host "  port:    $Port   host: $BindHost" -ForegroundColor DarkGray

if (-not (Install-NssmService)) { Install-Task }

$access = if ($BindHost -eq '0.0.0.0') { 'all interfaces' } else { $BindHost }
Write-Host ''
Write-Host "  Done. Web frontend running at http://localhost:$Port ($access)" -ForegroundColor Green
Write-Host "  logs:   $LogPath"
Write-Host "  update: re-run this script (it re-downloads + restarts)"
Write-Host "  stop:   nssm stop $SvcName  (or:  schtasks /end /tn $TaskName)"
Write-Host '  auth:   ensure a key/login exists (~/.config/catalyst-code/settings.json) or set UMANS_API_KEY.'
if ($BindHost -ne '127.0.0.1') {
    Write-Warn2 "Bound to $BindHost — put a TLS reverse proxy in front for public use."
}
