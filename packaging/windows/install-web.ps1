<#
.SYNOPSIS
    Compatibility shim — web install now lives in the root install.ps1.

.DESCRIPTION
    Historically this script installed only the Windows web frontend. That logic
    is now inlined into the repo-root install.ps1 (mirroring install.sh --with-web).

    This file forwards to install.ps1 so old one-liners and bookmarks keep working:

      irm .../packaging/windows/install-web.ps1 | iex
      & ([scriptblock]::Create((irm .../install-web.ps1))) -Port 8080

    Prefer the unified installer going forward:

      & ([scriptblock]::Create((irm .../install.ps1))) -WithWeb
      & ([scriptblock]::Create((irm .../install.ps1))) -Update
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

Write-Host ''
Write-Host '  note: packaging/windows/install-web.ps1 is a compatibility shim.' -ForegroundColor Yellow
Write-Host '        Web install is inlined in install.ps1 — forwarding...' -ForegroundColor Yellow
Write-Host ''

function Resolve-RootInstaller {
    if ($PSScriptRoot) {
        $candidate = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\install.ps1'))
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    $url = 'https://raw.githubusercontent.com/catalystctl/catcode/master/install.ps1'
    $dest = Join-Path $env:TEMP 'catcode-install.ps1'
    Write-Host "  Downloading install.ps1 ..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    return $dest
}

$installer = Resolve-RootInstaller
$exeName = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' }
if ($env:OS -eq 'Windows_NT') { $exeName += '.exe' }
$psExe = Join-Path $PSHOME $exeName

$fwd = @('-NoProfile', '-File', $installer)
if ($Uninstall) {
    $fwd += '-Uninstall'
} elseif ($BuildFromSource) {
    Write-Host '  error: -BuildFromSource is no longer supported via this shim.' -ForegroundColor Red
    Write-Host '  Use install.ps1 -WithWeb for the prebuilt bundle, or build from a repo checkout.' -ForegroundColor Red
    exit 1
} else {
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\catcode'
    $hasTui = Test-Path -LiteralPath (Join-Path $installDir 'catcode.exe')
    if ($hasTui) { $fwd += '-AddWeb' } else { $fwd += '-WithWeb' }
    if ($Port -ne 49283) { $fwd += @('-Port', "$Port") }
    if ($BindHost -ne '0.0.0.0') { $fwd += @('-BindHost', $BindHost) }
    if ($Version) { $fwd += @('-Version', $Version) }
    if ($BaseUrl) { $fwd += @('-BaseUrl', $BaseUrl) }
    if ($WebDir) { $fwd += @('-WebDir', $WebDir) }
}

& $psExe @fwd
exit $LASTEXITCODE
