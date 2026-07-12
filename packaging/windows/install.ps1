# catcode installer for Windows.
#
# Copies catcode.exe + catcode-core.exe into %LOCALAPPDATA%\Programs\catcode and adds
# that directory to the user PATH so `catcode` works from any CWD in PowerShell.
#
# Run from inside the unzipped bundle (the two .exe files sit next to this
# script):
#     pwsh -ExecutionPolicy Bypass -File .\install.ps1
# or right-click the file -> "Run with PowerShell" (unblock it first if
# Windows flagged it as downloaded:  Unblock-File .\install.ps1).
#
# No administrator rights are required: this installs per-user. Open a NEW
# PowerShell window after install so the refreshed PATH is visible.

[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\catcode')
)

$ErrorActionPreference = 'Stop'
$BundleDir = $PSScriptRoot

# --- locate the bundled binaries (this script ships beside them) ----------
$tui  = Join-Path $BundleDir 'catcode.exe'
$core = Join-Path $BundleDir 'catcode-core.exe'
foreach ($f in @($tui, $core)) {
    if (-not (Test-Path -LiteralPath $f)) {
        throw "Missing bundled binary: $f`nRun install.ps1 from inside the unzipped catcode folder."
    }
}

Write-Host "Installing catcode to $InstallDir" -ForegroundColor Cyan
if (-not (Test-Path -LiteralPath $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}
Copy-Item -LiteralPath $tui  -Destination $InstallDir -Force
Copy-Item -LiteralPath $core -Destination $InstallDir -Force

# --- add to user PATH (idempotent, case-insensitive) -----------------------
$path  = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $path) { $path = '' }
$parts = @($path.Split(';') | Where-Object { $_ -ne '' })
if ($parts -notcontains $InstallDir) {
    if ($parts.Count -gt 0) {
        $newPath = (($parts + $InstallDir) -join ';')
    } else {
        $newPath = $InstallDir
    }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $InstallDir to your user PATH." -ForegroundColor Green
} else {
    Write-Host "$InstallDir is already on your user PATH." -ForegroundColor DarkGray
}

# refresh the current session so `catcode` works here immediately too
$env:Path = "$env:Path;$InstallDir"

Write-Host ""
Write-Host "Done. Open a NEW PowerShell window (so PATH reloads) and run:" -ForegroundColor Green
Write-Host "    catcode" -ForegroundColor Yellow
Write-Host ""
Write-Host "First run inside catcode:  /login   then pick a model with /model"
Write-Host "Tip: the agent's bash tool needs bash on PATH (Git Bash or WSL)."
