<#
.SYNOPSIS
    Catalyst Code installer for Windows — TUI + optional web service.

.DESCRIPTION
    DEFAULT: download the prebuilt standalone catcode.exe (Rust core embedded)
    from GitHub Releases and put it on your user PATH — no compiler, no admin.
    With -WithWeb, also download catcode-core.exe + the prebuilt web bundle and
    install the web frontend as a Windows Service (NSSM) or a logon Scheduled
    Task (delegates to packaging/windows/install-web.ps1).

    No download needed — pipe it straight from the web:
        irm https://raw.githubusercontent.com/catalystctl/catcode/master/install.ps1 | iex

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

.PARAMETER WebInstallerUrl
    URL to packaging/windows/install-web.ps1 (used only with -WithWeb when this
    script is NOT run from a repo clone). Default: raw.githubusercontent.com master.

.PARAMETER Update
    Re-download the latest release and reinstall (also restarts the web service
    if it was previously installed).

.PARAMETER Uninstall
    Stop + remove catcode, catcode-core, the web service/task, and install state.

.PARAMETER DryRun
    Print the plan, execute nothing.

.PARAMETER NoColor
    Disable colored output.

.EXAMPLE
    .\install.ps1
    .\install.ps1 -WithWeb -Port 8080 -BindHost 127.0.0.1
    .\install.ps1 -Version 0.2.0
    .\install.ps1 -Update
    .\install.ps1 -Uninstall
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
    [string]$WebInstallerUrl = '',
    [switch]$Update,
    [switch]$Uninstall,
    [switch]$DryRun,
    [switch]$NoColor,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$ProgressPreference    = 'SilentlyContinue'   # speed up Invoke-WebRequest on large .exe

# ── constants + env-derived defaults (resolved in the body so a missing ──
# LOCALAPPDATA never crashes param binding; on Windows it is always set for
# user sessions, but SYSTEM/service accounts may lack it).
$Repo                = 'catalystctl/catcode'
$Arch                = 'x86_64'
$DefaultWebInstaller = "https://raw.githubusercontent.com/$Repo/master/packaging/windows/install-web.ps1"
function Resolve-LocalAppData {
    if ($env:LOCALAPPDATA) { return $env:LOCALAPPDATA }
    if ($env:USERPROFILE) { return Join-Path $env:USERPROFILE 'AppData\Local' }
    return $env:HOME   # non-Windows / fallback
}
$DataDir   = Join-Path (Resolve-LocalAppData) 'catalyst-code'
$StateFile = Join-Path $DataDir 'installer.state'
if (-not $InstallDir) { $InstallDir = Join-Path (Resolve-LocalAppData) 'Programs\catcode' }
if (-not $WebDir)     { $WebDir     = Join-Path $DataDir 'web' }

# resolve the current PowerShell executable (used to run install-web.ps1 in a
# child process so its exits never kill this installer's flow).
$exeName = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' }
if ($env:OS -eq 'Windows_NT') { $exeName += '.exe' }
$PsExe = Join-Path $PSHOME $exeName

# mirror the -WithWeb switch into a script-scoped flag (so -Update can set it
# from the recorded install state).
$script:WithWeb = [bool]$WithWeb

# ── helpers ──────────────────────────────────────────────────
function W-Info($t) { if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Cyan } }
function W-Ok($t)   { if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Green } }
function W-Warn($t){ if ($NoColor) { Write-Host "  $t" } else { Write-Host "  $t" -ForegroundColor Yellow } }
function Die($t)   { Write-Host "`n  error: $t" -ForegroundColor Red; exit 1 }

function Show-Help {
    $usage = @"
  Catalyst Code — installer for Windows

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
    -WebInstallerUrl <url>  URL to install-web.ps1 (default: raw.githubusercontent.com master)
    -Update             re-download latest + reinstall (+ restart the web service)
    -Uninstall          stop + remove binaries, service, and state
    -DryRun             print the plan, execute nothing
    -NoColor            disable colored output
    -Help               show this help
"@
    Write-Host $usage
}

# ── release resolution + asset download (mirrors install.sh) ─
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
    $dest = Join-Path $env:TEMP $Name
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

# ── PATH management ─────────────────────────────────────────
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

# ── TUI install (download standalone catcode.exe) ────────────
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

# ── separate core binary for the web service's CATCODE_CORE ──
function Install-CoreForWeb {
    $coreAsset = "catcode-core-$($script:Ver)-windows-$Arch.exe"
    $src = Get-Asset $coreAsset
    Copy-Item -LiteralPath $src -Destination (Join-Path $InstallDir 'catcode-core.exe') -Force
    W-Ok "Installed catcode-core.exe -> $InstallDir\catcode-core.exe"
}

# ── locate (or download) packaging/windows/install-web.ps1 ───
function Resolve-WebInstaller {
    # 1) local — run from a repo clone (install.ps1 sits at the repo root)
    if ($PSScriptRoot) {
        $local = Join-Path $PSScriptRoot 'packaging\windows\install-web.ps1'
        if (Test-Path -LiteralPath $local) { return $local }
    }
    # 2) download from -WebInstallerUrl (default: raw master)
    $url  = if ($WebInstallerUrl) { $WebInstallerUrl } else { $DefaultWebInstaller }
    $dest = Join-Path $env:TEMP 'catcode-install-web.ps1'
    W-Info "Downloading install-web.ps1 ..."
    try {
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    } catch {
        Die "could not download install-web.ps1 from $url.`n  If the repo is private, clone it and run install.ps1 from the repo root, or pass -WebInstallerUrl <mirror>."
    }
    return $dest
}

# Run install-web.ps1 in a CHILD PROCESS so its exits never terminate this
# installer's flow. Returns the child exit code.
function Invoke-WebInstaller([switch]$DoUninstall) {
    $webInstaller = Resolve-WebInstaller
    if ($DoUninstall) {
        W-Info 'Removing web service (delegating to install-web.ps1) ...'
        & $PsExe -NoProfile -File $webInstaller -Uninstall
        return $LASTEXITCODE
    }
    $coreExe = Join-Path $InstallDir 'catcode-core.exe'
    W-Info 'Installing web service (delegating to install-web.ps1) ...'
    & $PsExe -NoProfile -File $webInstaller -Port $Port -BindHost $BindHost `
        -Version $script:Ver -BaseUrl $script:Base -WebDir $WebDir -CatcodeCore $coreExe
    return $LASTEXITCODE
}

# ── install state ────────────────────────────────────────────
function Save-State([bool]$WebInstalled) {
    $st = [ordered]@{
        version      = $script:Ver
        with_web     = if ($WebInstalled) { 'yes' } else { 'no' }
        install_dir  = $InstallDir
        web_dir      = $WebDir
        port         = $Port
        host         = $BindHost
        installed_at = (Get-Date -Format 'yyyy-MM-ddTHH:mm:ssZ')
    }
    if (-not (Test-Path -LiteralPath $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
    $st | ConvertTo-Json | Set-Content -LiteralPath $StateFile -Encoding UTF8
    W-Ok "Recorded install state -> $StateFile"
}

function Load-State {
    if (-not (Test-Path -LiteralPath $StateFile)) { return $null }
    try { return (Get-Content -LiteralPath $StateFile -Raw | ConvertFrom-Json) } catch { return $null }
}

# ── summaries ────────────────────────────────────────────────
function Summary-Install {
    $webLine = if ($script:WithWeb) { "http://${BindHost}:$Port  (service: NSSM or Scheduled Task)" } else { '(not installed — re-run with -WithWeb)' }
    Write-Host ''
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
    Write-Host '  ✓  Installed  Catalyst Code  v' -NoNewline -ForegroundColor Green
    Write-Host "$($script:Ver)" -ForegroundColor Green
    Write-Host "    binary:  $InstallDir\catcode.exe" -ForegroundColor Green
    Write-Host "    web:     $webLine" -ForegroundColor Green
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
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
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
    Write-Host '  ✓  Updated  Catalyst Code  v' -NoNewline -ForegroundColor Green
    Write-Host "$($script:Ver)" -ForegroundColor Green
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
}

function Summary-Uninstall {
    Write-Host ''
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
    Write-Host '  ✓  Removed  Catalyst Code' -ForegroundColor Green
    Write-Host '  ────────────────────────────────────────────' -ForegroundColor Green
    Write-Host '  Open a NEW PowerShell window for a clean PATH.' -ForegroundColor DarkGray
}

# ── actions ───────────────────────────────────────────────────
function Do-Install {
    Write-Host ''
    Write-Host '  Catalyst Code — installer (Windows)' -ForegroundColor Cyan
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
        $rc = Invoke-WebInstaller
        if ($rc -ne 0) { Die "web service install failed (install-web.ps1 exited $rc)." }
        Save-State $true
    } else {
        W-Info 'Skipping web service (pass -WithWeb to install it)'
        Save-State $false
    }
    Summary-Install
}

function Do-Update {
    Write-Host ''
    Write-Host '  Catalyst Code — update' -ForegroundColor Cyan
    $st = Load-State
    if (-not $st) { Die "no previous install found at $StateFile — run install.ps1 first." }
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
        $rc = Invoke-WebInstaller
        if ($rc -ne 0) { W-Warn "web service update returned $rc (it self-restarts on re-install)" }
        Save-State $true
    } else {
        Save-State $false
    }
    Summary-Update
}

function Do-Uninstall {
    Write-Host ''
    Write-Host '  Catalyst Code — uninstall' -ForegroundColor Cyan
    $st = Load-State
    if ($st) { W-Info "Found previous install (v$($st.version), web: $($st.with_web))" }
    else     { W-Warn "no state file at $StateFile — attempting default paths" }

    if ($DryRun) {
        W-Info '[dry-run] would remove the web service + catcode.exe + catcode-core.exe + state'
        return
    }

    # web service first (if it was installed)
    $hadWeb = ($st -and $st.with_web -eq 'yes')
    if ($hadWeb) {
        $rc = Invoke-WebInstaller -DoUninstall
        if ($rc -ne 0) { W-Warn "web uninstall returned $rc (continuing)" }
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

# ── main ─────────────────────────────────────────────────────
if ($Help) { Show-Help; return }
if ($Update -and $Uninstall) { Die 'cannot combine -Update and -Uninstall.' }
if ($Update)   { Do-Update;   return }
if ($Uninstall) { Do-Uninstall; return }
Do-Install
