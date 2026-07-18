<#
.SYNOPSIS
    Catalyst Code installer for Windows - TUI + optional web service.

.DESCRIPTION
    DEFAULT: download the prebuilt standalone catcode.exe (Rust core embedded)
    from GitHub Releases and put it on your user PATH - no compiler, no admin.
    With -WithWeb, also download catcode-core.exe + the prebuilt web bundle and
    install the web frontend as a Windows Service (NSSM) or a logon Scheduled
    Task (web install is inlined in this script - same as install.sh --with-web).

    No download needed - pipe it straight from the web:
        irm https://raw.githubusercontent.com/catalystctl/catcode/master/install.ps1 | iex

    Run with no parameters in an interactive terminal to get a menu
    (install, install with web, add web, update, reinstall, uninstall, status).

    With arguments (e.g. -WithWeb), use the scriptblock form:
        & ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/master/install.ps1))) -WithWeb

    Or from a repo clone:
        pwsh -ExecutionPolicy Bypass -File .\install.ps1
        pwsh -ExecutionPolicy Bypass -File .\install.ps1 -WithWeb

.PARAMETER Version
    Pin a release (e.g. "0.2.0" or "v0.2.0"). Default: latest.

.PARAMETER BaseUrl
    Download base URL override (default: GitHub Releases for the resolved tag).

.PARAMETER InstallDir
    Where catcode.exe + catcode-core.exe are installed. Default:
    %LOCALAPPDATA%\Programs\catcode (per-user, no admin).

.PARAMETER WithWeb
    Also install the web frontend service (downloads catcode-core.exe + the
    prebuilt web bundle; sets up an NSSM service or a Scheduled Task).

.PARAMETER Port
    Web service port. Default 49283.

.PARAMETER BindHost
    Web bind host. Default 0.0.0.0 (use 127.0.0.1 + a reverse proxy for public use).

.PARAMETER WebDir
    Where to extract the web bundle. Default %LOCALAPPDATA%\catalyst-code\web.

.PARAMETER Update
    Re-download the latest release and reinstall (also restarts the web service
    if it was previously installed).

.PARAMETER Uninstall
    Stop + remove catcode, catcode-core, the web service/task, and install state.

.PARAMETER AddWeb
    Add the web service to an existing install (installs catcode-core.exe + the
    prebuilt web bundle + service/task). Pins to the installed version unless
    -Version is given.

.PARAMETER Reinstall
    Reinstall the currently-installed version (re-downloads the same release).

.PARAMETER Status
    Show the current install state (version, paths, web on/off) and exit.

.PARAMETER DryRun
    Print the plan, execute nothing.

.PARAMETER NoColor
    Disable colored output.

.EXAMPLE
    .\install.ps1                  # interactive menu + optional settings prompts
    .\install.ps1 -WithWeb -Port 8080 -BindHost 127.0.0.1
    .\install.ps1 -Version 0.2.0
    .\install.ps1 -Update
    .\install.ps1 -AddWeb
    .\install.ps1 -Reinstall
    .\install.ps1 -Uninstall
    .\install.ps1 -Status
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$BaseUrl = '',
    [string]$InstallDir = '',
    [switch]$WithWeb,
    [int]$Port = 49283,
    [string]$BindHost = '0.0.0.0',
    [string]$WebDir = '',
    [switch]$Update,
    [switch]$Uninstall,
    [switch]$AddWeb,
    [switch]$Reinstall,
    [switch]$Status,
    [switch]$DryRun,
    [switch]$NoColor,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$ProgressPreference    = 'SilentlyContinue'   # speed up Invoke-WebRequest on large .exe

# -- constants + env-derived defaults (resolved in the body so a missing --
# LOCALAPPDATA never crashes param binding; on Windows it is always set for
# user sessions, but SYSTEM/service accounts may lack it).
$Repo                = 'catalystctl/catcode'
$Arch                = 'x86_64'
function Resolve-LocalAppData {
    if ($env:LOCALAPPDATA) { return $env:LOCALAPPDATA }
    if ($env:USERPROFILE) { return Join-Path $env:USERPROFILE 'AppData\Local' }
    return $env:HOME   # non-Windows / fallback
}
$DataDir   = Join-Path (Resolve-LocalAppData) 'catalyst-code'
$StateFile = Join-Path $DataDir 'installer.state'
if (-not $InstallDir) { $InstallDir = Join-Path (Resolve-LocalAppData) 'Programs\catcode' }
if (-not $WebDir)     { $WebDir     = Join-Path $DataDir 'web' }

# mirror the -WithWeb switch into a script-scoped flag (so -Update can set it
# from the recorded install state).
$script:WithWeb = [bool]$WithWeb

# -- helpers --------------------------------------------------
function W-Info($t) { if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Cyan } }
function W-Ok($t)   { if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Green } }
function W-Warn($t){ if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Yellow } }
# Prefer throw over exit: under `irm | iex` or `& ([scriptblock]::Create(...))`,
# exit kills the user's entire PowerShell window so they never see the error.
# throw surfaces a red error and leaves the shell open. `pwsh -File` still
# exits non-zero on an uncaught throw (CI/scripted use stays correct).
function Die($t)   { Write-Host "`n  error: $t" -ForegroundColor Red; throw "install failed: $t" }

# Native exes (schtasks/sc/nssm) write expected failures to stderr. With
# $ErrorActionPreference=Stop, PowerShell turns that into a terminating
# NativeCommandError even when redirected - so "task not found" on first
# install aborts the script. Run them under Continue and use $LASTEXITCODE.
function Invoke-Native {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(ValueFromRemainingArguments)][string[]]$ArgumentList
    )
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $FilePath @ArgumentList *> $null
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
    }
}

function Show-Help {
    $usage = @"
  Catalyst Code - installer for Windows

  Usage:
    pwsh -ExecutionPolicy Bypass -File .\install.ps1 [options]
    irm https://raw.githubusercontent.com/catalystctl/catcode/master/install.ps1 | iex
    & ([scriptblock]::Create((irm .../install.ps1))) -WithWeb

  Options:
    -Version <v>        pin a release (e.g. "0.2.0" or "v0.2.0")   default: latest
    -BaseUrl <url>      download from a mirror instead of GitHub Releases
    -InstallDir <path>  binary install dir        (default: %LOCALAPPDATA%\Programs\catcode)
    -WithWeb            also install the web frontend service
    -Port <n>           web service port          (default: 49283)
    -BindHost <h>       web bind host             (default: 0.0.0.0)
    -WebDir <path>      web bundle install dir    (default: %LOCALAPPDATA%\catalyst-code\web)
    -AddWeb             add the web service to an existing install
    -Update             re-download latest + reinstall (+ restart the web service)
    -Reinstall          reinstall the currently-installed version
    -Uninstall          stop + remove binaries, service, and state
    -Status             show the current install state
    -DryRun             print the plan, execute nothing
    -NoColor            disable colored output
    -Help               show this help
"@
    Write-Host $usage
}

# -- release resolution + asset download (mirrors install.sh) -
# Strip a leading "v" from a tag for the version string used in asset names.
# Unlike Substring(1), this leaves commit-SHA tags (e.g. "1c08256") intact.
function Get-VerFromTag {
    param([string]$Tag)
    if ($Tag.StartsWith('v') -or $Tag.StartsWith('V')) { return $Tag.Substring(1) }
    return $Tag
}

function Resolve-Release {
    if ($Version) {
        # Accept "0.2.0" (-> v0.2.0 semver tag), "v0.2.0" (as-is), or a commit
        # SHA like "1c08256" (as-is - SHA tags have no leading v). Only prepend v
        # for bare semver (digits.digits), never for hex SHAs.
        $script:Tag = $Version
        if ($script:Tag -match '^[0-9]+\.[0-9]+' -and -not $script:Tag.StartsWith('v')) {
            $script:Tag = "v$($script:Tag)"
        }
        $script:Ver = Get-VerFromTag $script:Tag
    } else {
        $api = "https://api.github.com/repos/$Repo/releases/latest"
        try {
            $rel = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'catcode-installer' } -ErrorAction Stop
            $script:Tag = $rel.tag_name
            $script:Ver = Get-VerFromTag $script:Tag
        } catch {
            Die "could not resolve the latest release from $api.`n  The repo may be private or rate-limited. Pass -Version <v> (e.g. -Version 0.2.0) or -BaseUrl <url> to a public mirror."
        }
    }
    if ($BaseUrl) {
        $script:Base = $BaseUrl.TrimEnd('/')
    } else {
        $script:Base = "https://github.com/$Repo/releases/download/$($script:Tag)"
    }
}

# download <Base>/<Name> + <Name>.sha256, verify the checksum. Returns the file path.
function Get-Asset {
    param([string]$Name)
    $url  = "$($script:Base)/$Name"
    $tmp = $env:TEMP
    if (-not $tmp) { $tmp = $env:TMP }
    if (-not $tmp) { $tmp = $env:TMPDIR }
    if (-not $tmp) { $tmp = [System.IO.Path]::GetTempPath() }
    if (-not $tmp) { Die 'no temp directory (TEMP/TMP unset)' }
    $dest = Join-Path $tmp $Name
    W-Info "Downloading $Name ..."
    try {
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    } catch {
        Die "download failed: $url`n  $($_.Exception.Message)"
    }
    try {
        Invoke-WebRequest -Uri "$url.sha256" -OutFile "$dest.sha256" -UseBasicParsing
    } catch {
        Die "checksum download failed: $url.sha256"
    }
    $expected = (Get-Content "$dest.sha256" -Raw).Trim().Split(' ')[0].ToLower()
    $actual   = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { Die "checksum mismatch for $Name (expected $expected, got $actual)" }
    W-Ok "Verified $Name"
    return $dest
}

# -- PATH management -----------------------------------------
function Add-ToPath {
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $path) { $path = '' }
    $parts = @($path.Split(';') | Where-Object { $_ -ne '' })
    if ($parts -notcontains $InstallDir) {
        $newPath = (($parts + $InstallDir) -join ';')
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        W-Ok "Added $InstallDir to your user PATH."
    } else {
        W-Ok "$InstallDir is already on your user PATH."
    }
    # refresh the current session so `catcode` works immediately
    if ($env:Path -notlike "*$InstallDir*") { $env:Path = "$env:Path;$InstallDir" }
}

# -- TUI install (download standalone catcode.exe) ------------
function Install-Tui {
    if (-not (Test-Path -LiteralPath $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    $tuiAsset = "catcode-$($script:Ver)-windows-$Arch.exe"
    $src = Get-Asset $tuiAsset
    Copy-Item -LiteralPath $src -Destination (Join-Path $InstallDir 'catcode.exe') -Force
    W-Ok "Installed catcode.exe -> $InstallDir\catcode.exe"
    Add-ToPath
}

# -- separate core binary for the web service's CATCODE_CORE --
function Install-CoreForWeb {
    $coreAsset = "catcode-core-$($script:Ver)-windows-$Arch.exe"
    $src = Get-Asset $coreAsset
    Copy-Item -LiteralPath $src -Destination (Join-Path $InstallDir 'catcode-core.exe') -Force
    W-Ok "Installed catcode-core.exe -> $InstallDir\catcode-core.exe"
}


# --- web service (inlined - formerly packaging/windows/install-web.ps1) ---
$SvcName     = 'CatalystCodeWeb'
$TaskName    = 'CatalystCodeWeb'
$WrapperPath = Join-Path $DataDir 'run-web.cmd'
$LogPath     = Join-Path $DataDir 'catalyst-code-web.log'
$script:RT = $null
$script:RTExe = $null

function Detect-Runtime {
    # Prefer Node for the prebuilt Next standalone server; both Node and Bun run
    # the same bundled JavaScript (no native rebuilds required).
    $node = Get-Command node -ErrorAction SilentlyContinue
    if ($node) {
        $nodeVer = (& node --version) -replace '^v',''
        if ([version]$nodeVer -lt [version]'22.13.0') {
            Die "Node.js >= 22.13.0 is required (found v${nodeVer}); the web frontend uses node:sqlite"
        }
        $script:RT = 'node'; $script:RTExe = $node.Source; return
    }
    $bun = Get-Command bun -ErrorAction SilentlyContinue
    if ($bun) { $script:RT = 'bun'; $script:RTExe = $bun.Source; return }
    Die 'neither node nor bun found - install one to RUN the web frontend (https://nodejs.org or https://bun.sh)'
}

# --- resolve release version + base URL ------------------------------------
# Strip a leading "v" from a tag for the version string used in asset names.
# Unlike Substring(1), this leaves commit-SHA tags (e.g. "1c08256") intact.
function Assert-WebBundle {
    param([string]$Dir)
    $startJs = Join-Path $Dir 'start.js'
    if (-not (Test-Path -LiteralPath $startJs)) { Die "web bundle missing start.js (extraction failed?)" }
    if (-not (Test-Path -LiteralPath (Join-Path $Dir 'server.js'))) { Die 'web bundle missing server.js' }
    if (-not (Test-Path -LiteralPath (Join-Path $Dir 'package.json'))) {
        Die 'web bundle missing package.json (incomplete release artifact)'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Dir '.next\BUILD_ID'))) {
        Die 'web bundle missing .next/BUILD_ID (incomplete release artifact)'
    }
    if ((Test-Path -LiteralPath (Join-Path $Dir 'web\server.js')) -or
        (Test-Path -LiteralPath (Join-Path $Dir 'web\node_modules'))) {
        Die @"
web bundle has nested web/ layout - this release artifact was packed incorrectly.
  Use a newer catcode-web-*.tar.gz built by current release-web.sh.
"@
    }
    foreach ($req in @('next', 'ws', 'zigpty', 'better-auth')) {
        $pkg = Join-Path $Dir "node_modules\$req\package.json"
        if (-not (Test-Path -LiteralPath $pkg)) {
            Die "web bundle missing node_modules/$req - incomplete release artifact (custom server cannot start)."
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Dir 'version.json'))) {
        Die 'web bundle missing version.json (git commit not embedded)'
    }
    W-Ok "Web bundle looks runnable ($Dir)"
}

function Install-WebBundle {
    # Universal cross-platform tarball (same asset Linux/macOS/Windows installers fetch).
    $tgz = Get-Asset "catcode-web-$($script:Ver).tar.gz"
    if (-not (Test-Path -LiteralPath $WebDir)) { New-Item -ItemType Directory -Path $WebDir -Force | Out-Null }
    Get-ChildItem -LiteralPath $WebDir -Force | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    # Windows 10+ ships tar (bsdtar); it handles .tar.gz natively.
    $tar = Get-Command tar -ErrorAction SilentlyContinue
    if (-not $tar) { Die 'tar not found (Windows 10 1803+ ships it). Extract the .tar.gz manually or install tar.' }
    W-Info "Extracting web bundle -> $WebDir ..."
    & tar -xzf $tgz -C $WebDir
    if ($LASTEXITCODE -ne 0) { Die "tar extraction failed (exit $LASTEXITCODE)" }
    Write-WebVersionJson -Dir $WebDir -Commit $script:Ver -Source 'release'
    Assert-WebBundle -Dir $WebDir

    W-Ok "Web bundle extracted to $WebDir"
}

function Write-WebVersionJson {
    param(
        [string]$Dir,
        [string]$Commit,
        [string]$Source = 'release'
    )
    $builtAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $payload = [ordered]@{
        commit     = $Commit
        commitFull = $Commit
        dirty      = $false
        builtAt    = $builtAt
        source     = $Source
    }
    $json = $payload | ConvertTo-Json
    $path = Join-Path $Dir 'version.json'
    Set-Content -LiteralPath $path -Value $json -Encoding UTF8
    $nextDir = Join-Path $Dir '.next'
    if (Test-Path -LiteralPath $nextDir) {
        Set-Content -LiteralPath (Join-Path $nextDir 'version.json') -Value $json -Encoding UTF8
    }
    W-Ok "Web version: $Commit ($Source)"
}

# --- NSSM service -----------------------------------------------------------
function Install-NssmService {
    $nssm = Get-Command nssm -ErrorAction SilentlyContinue
    if (-not $nssm) { return $false }
    $nssm = $nssm.Source
    [void](Invoke-Native $nssm stop $SvcName)
    [void](Invoke-Native $nssm remove $SvcName confirm)
    W-Info "Installing Windows Service '$SvcName' (NSSM)..."
    & $nssm install $SvcName $RTExe (Join-Path $WebDir 'start.js') *> $null
    if ($LASTEXITCODE -ne 0) { Die "nssm install failed (exit $LASTEXITCODE)" }
    & $nssm set $SvcName AppDirectory $WebDir *> $null
    & $nssm set $SvcName AppEnvironmentExtra "NODE_ENV=production" "PORT=$Port" "HOSTNAME=$BindHost" "CATCODE_CORE=$script:CoreExe" *> $null
    & $nssm set $SvcName AppStdout $LogPath *> $null
    & $nssm set $SvcName AppStderr $LogPath *> $null
    & $nssm set $SvcName AppRotateFiles 1 *> $null
    & $nssm set $SvcName AppRotateBytes 10485760 *> $null
    & $nssm set $SvcName AppRestartDelay 3000 *> $null
    & $nssm set $SvcName Start 'SERVICE_AUTO_START' *> $null
    & $nssm start $SvcName *> $null
    if ($LASTEXITCODE -ne 0) { Die "nssm start failed - check: nssm get $SvcName AppStdout ; Get-Content $LogPath" }
    W-Ok "Service '$SvcName' installed and started (NSSM, auto-start at boot)"
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
        "set CATCODE_CORE=$script:CoreExe",
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
    W-Info 'NSSM not found - installing as a Scheduled Task at logon...'
    W-Warn 'Note: a Scheduled Task runs only while a user is logged in.'
    W-Warn '      For a true boot-time service, install NSSM (https://nssm.cc) and re-run.'
    Write-Wrapper
    # /query fails with "file not found" when the task is absent - expected on first install.
    if ((Invoke-Native schtasks /query /tn $TaskName) -eq 0) {
        [void](Invoke-Native schtasks /delete /tn $TaskName /f)
    }
    $ec = Invoke-Native schtasks /create /tn $TaskName /tr "`"$WrapperPath`"" /sc onlogon /rl limited /f
    if ($ec -ne 0) { Die "schtasks /create failed (exit $ec)" }
    [void](Invoke-Native schtasks /run /tn $TaskName)
    W-Ok "Scheduled task '$TaskName' created and started (at logon, restart-loop wrapper)"
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
    W-Ok "Runtime: $rt ($rtExe)"
    Push-Location (Join-Path $RepoDir 'sdk')
    try { & $rtExe install *> $null; if ($LASTEXITCODE -ne 0) { Die 'SDK dep install failed' }; & $rtExe run build *> $null; if ($LASTEXITCODE -ne 0) { Die 'SDK build failed' } }
    finally { Pop-Location }
    Push-Location (Join-Path $RepoDir 'web')
    try { & $rtExe install *> $null; if ($LASTEXITCODE -ne 0) { Die 'web dep install failed' }; $env:NEXT_TELEMETRY_DISABLED = '1'; & $rtExe run build; if ($LASTEXITCODE -ne 0) { Die 'web build failed' } }
    finally { Pop-Location }
    # source path runs `next start` from the repo web dir
    $script:WebDir = Join-Path $RepoDir 'web'
    $script:RT = $rt; $script:RTExe = $rtExe
    W-Ok 'Web build complete'
}

# --- uninstall --------------------------------------------------------------

function Install-WebService {
    Detect-Runtime
    # Script-scoped so Install-NssmService / Write-Wrapper see CATCODE_CORE.
    $script:CoreExe = Join-Path $InstallDir 'catcode-core.exe'
    $CoreExe = $script:CoreExe
    if (-not (Test-Path -LiteralPath $CoreExe)) {
        Die "catcode-core.exe missing at $CoreExe - install core first"
    }
    Install-WebBundle
    if (-not (Install-NssmService)) { Install-Task }
    W-Ok "Web frontend service ready at http://localhost:$Port"
}

function Uninstall-WebService {
    W-Info 'Removing web service ...'
    $nssm = Get-Command nssm -ErrorAction SilentlyContinue
    $removed = $false
    if ($nssm) {
        $nssm = $nssm.Source
        [void](Invoke-Native $nssm stop $SvcName)
        $ec = Invoke-Native $nssm remove $SvcName confirm
        if ($ec -eq 0) { W-Ok "Removed Windows Service '$SvcName'"; $removed = $true }
    } else {
        [void](Invoke-Native sc.exe stop $SvcName)
        [void](Invoke-Native sc.exe delete $SvcName)
    }
    if ((Invoke-Native schtasks /query /tn $TaskName) -eq 0) {
        [void](Invoke-Native schtasks /end /tn $TaskName)
        [void](Invoke-Native schtasks /delete /tn $TaskName /f)
        W-Ok "Removed Scheduled Task '$TaskName'"; $removed = $true
    }
    if (Test-Path $WrapperPath) { Remove-Item $WrapperPath -Force; W-Ok "Removed wrapper $WrapperPath" }
    if (Test-Path -LiteralPath $WebDir) {
        Remove-Item -LiteralPath $WebDir -Recurse -Force -ErrorAction SilentlyContinue
        W-Ok "Removed web bundle $WebDir"
    }
    if (-not $removed) { W-Warn 'No service or task found to remove (already clean?)' }
}


# -- install state --------------------------------------------
function Save-State([bool]$WebInstalled) {
    $installedAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $webFlag = if ($WebInstalled) { 'yes' } else { 'no' }
    $st = [ordered]@{
        version      = $script:Ver
        with_web     = $webFlag
        install_dir  = $InstallDir
        web_dir      = $WebDir
        port         = $Port
        host         = $BindHost
        installed_at = $installedAt
    }
    if (-not (Test-Path -LiteralPath $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
    $st | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $DataDir 'installer.state.json') -Encoding UTF8
    $shellLines = @(
        '# Catalyst Code installer state - written by install.ps1',
        '# (shell-sourcable; consumed by catcode --update)',
        'METHOD="download"',
        ('PREFIX="' + $InstallDir + '"'),
        ('PORT="' + $Port + '"'),
        ('HOST="' + $BindHost + '"'),
        ('WEB_DIR="' + $WebDir + '"'),
        ('WEB_INSTALLED="' + $webFlag + '"'),
        'UNIT_NAME="CatalystCodeWeb"',
        ('VERSION="' + $script:Ver + '"'),
        ('INSTALLED_AT="' + $installedAt + '"')
    )
    Set-Content -LiteralPath $StateFile -Value ($shellLines -join "`r`n") -Encoding UTF8
    W-Ok "Recorded install state -> $StateFile"
}

function Load-State {
    if (-not (Test-Path -LiteralPath $StateFile)) { return $null }
    $raw = Get-Content -LiteralPath $StateFile -Raw
    try {
        $j = $raw | ConvertFrom-Json
        if ($null -ne $j.version -or $null -ne $j.with_web) { return $j }
    } catch {}
    $jsonBackup = Join-Path $DataDir 'installer.state.json'
    if (Test-Path -LiteralPath $jsonBackup) {
        try { return (Get-Content -LiteralPath $jsonBackup -Raw | ConvertFrom-Json) } catch {}
    }
    $map = @{}
    foreach ($line in ($raw -split "`r?`n")) {
        if ($line -match '^([A-Z_]+)="([^"]*)"') { $map[$Matches[1]] = $Matches[2] }
    }
    if (-not $map.ContainsKey('VERSION') -and -not $map.ContainsKey('WEB_INSTALLED')) { return $null }
    return [pscustomobject]@{
        version      = $map['VERSION']
        with_web     = if ($map['WEB_INSTALLED']) { $map['WEB_INSTALLED'] } else { 'no' }
        install_dir  = if ($map['PREFIX']) { $map['PREFIX'] } else { $InstallDir }
        web_dir      = if ($map['WEB_DIR']) { $map['WEB_DIR'] } else { $WebDir }
        port         = if ($map['PORT']) { [int]$map['PORT'] } else { $Port }
        host         = if ($map['HOST']) { $map['HOST'] } else { $BindHost }
        installed_at = $map['INSTALLED_AT']
    }
}

# -- summaries ------------------------------------------------
function Summary-Install {
    $webLine = if ($script:WithWeb) { "http://${BindHost}:$Port  (service: NSSM or Scheduled Task)" } else { '(not installed - re-run with -WithWeb)' }
    Write-Host ''
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host '  OK  Installed  Catalyst Code  v' -NoNewline -ForegroundColor Green
    Write-Host "$($script:Ver)" -ForegroundColor Green
    Write-Host "    binary:  $InstallDir\catcode.exe" -ForegroundColor Green
    Write-Host "    web:     $webLine" -ForegroundColor Green
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host ''
    Write-Host '  Open a NEW PowerShell window (so PATH reloads) and run:' -ForegroundColor Green
    Write-Host '    catcode' -ForegroundColor Yellow
    if ($script:WithWeb) {
        Write-Host "  web:   http://localhost:$Port   (logs: $env:LOCALAPPDATA\catalyst-code\catalyst-code-web.log)" -ForegroundColor Green
    }
    Write-Host '  auth:  /login  (or set UMANS_API_KEY)'
}

function Summary-Update {
    Write-Host ''
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host '  OK  Updated  Catalyst Code  v' -NoNewline -ForegroundColor Green
    Write-Host "$($script:Ver)" -ForegroundColor Green
    Write-Host '  --------------------------------------------' -ForegroundColor Green
}

function Summary-Uninstall {
    Write-Host ''
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host '  OK  Removed  Catalyst Code' -ForegroundColor Green
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host '  Open a NEW PowerShell window for a clean PATH.' -ForegroundColor DarkGray
}

# -- actions ---------------------------------------------------
function Do-Install {
    Write-Host ''
    Write-Host '  Catalyst Code - installer (Windows)' -ForegroundColor Cyan
    Write-Host '  mode: download (prebuilt, no compile)' -ForegroundColor DarkGray
    Resolve-Release
    Write-Host "  version: $($script:Ver)   base: $($script:Base)" -ForegroundColor DarkGray
    Write-Host "  install: $InstallDir" -ForegroundColor DarkGray
    if ($script:WithWeb) { Write-Host "  web:     $WebDir (port $Port, host $BindHost)" -ForegroundColor DarkGray }

    if ($DryRun) {
        W-Info '[dry-run] would download + install catcode.exe'
        if ($script:WithWeb) { W-Info '[dry-run] would also install catcode-core.exe + the web service' }
        return
    }

    Install-Tui
    if ($script:WithWeb) {
        # record the TUI install first so a web failure still leaves a usable state
        Save-State $false
        Install-CoreForWeb
        Install-WebService
        Save-State $true
    } else {
        W-Info 'Skipping web service (pass -WithWeb to install it)'
        Save-State $false
    }
    Summary-Install
}

function Do-Update {
    Write-Host ''
    Write-Host '  Catalyst Code - update' -ForegroundColor Cyan
    $st = Load-State
    if (-not $st) { Die "no previous install found at $StateFile - run install.ps1 first." }
    W-Info "Previous install: v$($st.version) (web: $($st.with_web))"

    Resolve-Release
    Write-Host "  version: $($script:Ver)   base: $($script:Base)" -ForegroundColor DarkGray

    if ($DryRun) {
        W-Info '[dry-run] would reinstall catcode.exe'
        if ($st.with_web -eq 'yes') { W-Info '[dry-run] would reinstall + restart the web service' }
        return
    }

    Install-Tui
    if ($st.with_web -eq 'yes') {
        $script:WithWeb = $true
        Install-CoreForWeb
        Install-WebService
        Save-State $true
    } else {
        Save-State $false
    }
    Summary-Update
}

function Do-Uninstall {
    Write-Host ''
    Write-Host '  Catalyst Code - uninstall' -ForegroundColor Cyan
    $st = Load-State
    if ($st) { W-Info "Found previous install (v$($st.version), web: $($st.with_web))" }
    else     { W-Warn "no state file at $StateFile - attempting default paths" }

    if ($DryRun) {
        W-Info '[dry-run] would remove the web service + catcode.exe + catcode-core.exe + state'
        return
    }

    # web service first (if it was installed)
    $hadWeb = ($st -and $st.with_web -eq 'yes')
    if ($hadWeb) {
        Uninstall-WebService
    }

    # binaries
    foreach ($b in 'catcode.exe', 'catcode-core.exe') {
        $p = Join-Path $InstallDir $b
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force; W-Ok "Removed $p" }
    }
    # state
    if (Test-Path -LiteralPath $StateFile) { Remove-Item -LiteralPath $StateFile -Force; W-Ok "Removed $StateFile" }
    Summary-Uninstall
}

function Show-Menu {
    $st = Load-State
    $status = if ($st) { "v$($st.version) (web: $($st.with_web))" } else { 'not installed' }
    Write-Host ''
    Write-Host '  Catalyst Code - installer menu' -ForegroundColor Cyan
    Write-Host "  platform: Windows    status: $status" -ForegroundColor DarkGray
    while ($true) {
        Write-Host ''
        Write-Host '  What would you like to do?' -ForegroundColor DarkGray
        Write-Host ''
        Write-Host '    1  Install              (catcode TUI + core)'
        Write-Host '    2  Install with web     (TUI + core + 24/7 web service)'
        Write-Host '    3  Add web service      (add web to an existing install)'
        Write-Host '    4  Update               (download latest + reinstall)'
        Write-Host '    5  Reinstall            (reinstall the current version)'
        Write-Host '    6  Uninstall            (remove everything)'
        Write-Host '    7  Status               (show current install state)'
        Write-Host '    0  Exit'
        Write-Host ''
        $choice = Read-Host '  Select [0-7]'
        if ([string]::IsNullOrWhiteSpace($choice)) { return 'install' }  # stdin closed -> default
        switch ($choice) {
            '1' { return 'install' }
            '2' { $script:WithWeb = $true; return 'install' }
            '3' { return 'add-web' }
            '4' { return 'update' }
            '5' { return 'reinstall' }
            '6' { return 'uninstall' }
            '7' { return 'status' }
            '0' { Write-Host '  Bye.' -ForegroundColor DarkGray; return 'exit' }
            default { Write-Host '  invalid choice - try again' -ForegroundColor Yellow }
        }
    }
}

# Interactive settings prompts (menu path only). Enter keeps each default.
# Changing Port / BindHost / WebDir here feeds straight into the NSSM/task
# service install so the web URL updates automatically.
function Prompt-Value {
    param([string]$Label, [string]$Default = '')
    $hint = if ($Default) { $Default } else { 'empty / latest' }
    $ans = Read-Host "  $Label [$hint]"
    if ([string]::IsNullOrWhiteSpace($ans)) { return $Default }
    return $ans.Trim()
}

function Prompt-InstallOptions {
    param([string]$Action)
    switch ($Action) {
        'install' { }
        'add-web' { }
        'update' { }
        'reinstall' { }
        default { return }
    }

    Write-Host ''
    $customize = Read-Host '  Customize install settings (paths, port, version)? [y/N]'
    if ($customize -notmatch '^(?i)y(es)?$') {
        W-Info "Using defaults (install=$InstallDir port=$Port host=$BindHost)"
        return
    }

    Write-Host ''
    Write-Host '  Install settings  (press Enter to keep each default)' -ForegroundColor Cyan
    Write-Host ''

    $script:InstallDir = Prompt-Value 'Binary install directory' $InstallDir
    $script:Version    = Prompt-Value 'Release version pin' $Version
    $script:BaseUrl    = Prompt-Value 'Download base URL (mirror)' $BaseUrl

    $wantWeb = [bool]$script:WithWeb -or ($Action -eq 'add-web')
    if (-not $wantWeb -and ($Action -eq 'update' -or $Action -eq 'reinstall')) {
        $st = Load-State
        if ($st -and $st.with_web -eq 'yes') { $wantWeb = $true }
    }

    if ($wantWeb) {
        $script:WebDir = Prompt-Value 'Web bundle directory' $WebDir

        while ($true) {
            $raw = Prompt-Value 'Web service port' "$Port"
            $p = 0
            if ([int]::TryParse($raw, [ref]$p) -and $p -ge 1 -and $p -le 65535) {
                $script:Port = $p
                break
            }
            Write-Host '  port must be an integer 1-65535' -ForegroundColor Yellow
        }

        $script:BindHost = Prompt-Value 'Web bind host' $BindHost
    }

    Write-Host ''
    W-Ok "Will use: install=$($script:InstallDir)  port=$($script:Port)  host=$($script:BindHost)"
    if ($script:Version) { W-Ok "version pin: $($script:Version)" }
    if ($script:BaseUrl) { W-Ok "base URL: $($script:BaseUrl)" }
    if ($wantWeb) { W-Ok "web dir: $($script:WebDir)" }
}

function Do-AddWeb {
    Write-Host ''
    Write-Host '  Catalyst Code - add web service' -ForegroundColor Cyan
    $st = Load-State
    if (-not $st) { Die "no previous install found at $StateFile - run install.ps1 first to install catcode." }
    if ($st.with_web -eq 'yes') { W-Warn 'web service is already installed - reinstalling it' }
    $script:WithWeb = $true
    # pin to the installed version unless one was explicitly given
    if (-not $Version) { $Version = $st.version }
    Resolve-Release
    Write-Host "  version: $($script:Ver)   base: $($script:Base)" -ForegroundColor DarkGray
    Write-Host "  install: $InstallDir" -ForegroundColor DarkGray

    if ($DryRun) {
        W-Info '[dry-run] would install catcode-core.exe + the web service'
        return
    }
    Install-CoreForWeb
    Install-WebService
    Save-State $true
    Summary-AddWeb
}

function Do-Reinstall {
    Write-Host ''
    Write-Host '  Catalyst Code - reinstall' -ForegroundColor Cyan
    $st = Load-State
    if (-not $st) { Die "no previous install found at $StateFile - run install.ps1 first." }
    W-Info "Reinstalling v$($st.version) (web: $($st.with_web))"
    if (-not $Version) { $Version = $st.version }
    if ($st.with_web -eq 'yes') { $script:WithWeb = $true }
    Do-Install
}

function Do-Status {
    Write-Host ''
    Write-Host '  Catalyst Code - status' -ForegroundColor Cyan
    $st = Load-State
    if (-not $st) {
        W-Warn "no previous install found at $StateFile"
        W-Info 'Catalyst Code does not appear to be installed.'
        return
    }
    W-Ok  "Version:      v$($st.version)"
    W-Info "Install dir:  $($st.install_dir)"
    W-Info "Web service:  $($st.with_web)"
    if ($st.with_web -eq 'yes') {
        W-Info "Web dir:      $($st.web_dir)"
        W-Info "Web address:  http://localhost:$($st.port)"
    }
    if ($st.installed_at) { W-Info "Installed at: $($st.installed_at)" }
    $exe = Join-Path $st.install_dir 'catcode.exe'
    if (Test-Path -LiteralPath $exe) { W-Ok "catcode.exe present at $exe" }
    else { W-Warn "catcode.exe NOT found at $exe" }
}

function Summary-AddWeb {
    Write-Host ''
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host '  OK  Web service added  Catalyst Code  v' -NoNewline -ForegroundColor Green
    Write-Host "$($script:Ver)" -ForegroundColor Green
    Write-Host "    core:  $InstallDir\catcode-core.exe" -ForegroundColor Green
    Write-Host "    web:   http://localhost:$Port" -ForegroundColor Green
    Write-Host '  --------------------------------------------' -ForegroundColor Green
    Write-Host "  logs:  $env:LOCALAPPDATA\catalyst-code\catalyst-code-web.log" -ForegroundColor DarkGray
}

# -- main -----------------------------------------------------
# Determine whether the user passed any explicit option. If nothing was
# requested and we're in an interactive terminal, show the menu; otherwise
# run the implied action directly (preserves CI / `irm | iex` automation).
$canMenu = $false
try { $canMenu = (-not [Console]::IsInputRedirected) -and [Environment]::UserInteractive } catch {}

# Show the interactive menu only when NO parameters were passed (and we're in a
# real terminal). Any explicit option - even -WithWeb or -Version - runs the
# implied action directly, preserving CI / scripted use. $PSBoundParameters
# reflects what the caller actually passed, unaffected by the default
# resolution that happens earlier in the script.
if ($PSBoundParameters.Count -eq 0 -and $canMenu) {
    $action = Show-Menu
    Prompt-InstallOptions -Action $action
} else {
    if ($Help) { Show-Help; return }
    if ($Update -and $Uninstall) { Die 'cannot combine -Update and -Uninstall.' }
    if ($Update)        { $action = 'update' }
    elseif ($Uninstall) { $action = 'uninstall' }
    elseif ($AddWeb)    { $action = 'add-web' }
    elseif ($Reinstall) { $action = 'reinstall' }
    elseif ($Status)    { $action = 'status' }
    else                { $action = 'install' }
}

switch ($action) {
    'install'   { Do-Install }
    'update'    { Do-Update }
    'uninstall' { Do-Uninstall }
    'add-web'   { Do-AddWeb }
    'reinstall' { Do-Reinstall }
    'status'    { Do-Status }
    'exit'      { return }
    default     { Die "unknown action: $action" }
}
