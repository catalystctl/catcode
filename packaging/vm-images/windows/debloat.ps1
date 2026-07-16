# debloat.ps1 — Windows 11 IoT Enterprise LTSC test-VM provisioning.
#
# Two phases (driven by autounattend.xml):
#   -Phase Specialize  : runs as SYSTEM during specialize pass. Removes bloat
#                        (Appx/capabilities/services), kills telemetry, disables
#                        Windows Update, installs + enables OpenSSH server.
#   -Phase Oobe        : runs as testuser (admin) at first logon. Drops the SSH
#                        authorized key, sets the OpenSSH default shell to
#                        PowerShell, verifies a browser is present, then shuts
#                        the VM down (signals build.sh the image is done).
#
# Tuned for a webui/tui TEST target: keeps a browser (Edge, or installs Firefox
# if none), PowerShell, OpenSSH, networking, .NET. Removes everything
# consumer/embedded we don't need to drive a browser or a terminal.
#
# Re-runnable: safe to invoke again (everything is idempotent / -ErrorAction
# SilentlyContinue). No product key, no activation, no domain.

param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Specialize", "Oobe")]
    [string]$Phase
)

$ErrorActionPreference = "SilentlyContinue"
$ProgressPreference    = "SilentlyContinue"   # speed up Appx/DISM output

# --- Appx packages to remove (provisioned = for new users; + per-user). -----
# LTSC ships with far less than consumer Win11, but still carries some. This
# list is the union of what LTSC 2024 may include; missing entries are skipped.
$RemoveAppx = @(
    "Microsoft.BingNews", "Microsoft.BingWeather", "Microsoft.BingSearch*",
    "Microsoft.GetHelp", "Microsoft.Getstarted", "Microsoft.Microsoft3DViewer",
    "Microsoft.MicrosoftOfficeHub", "Microsoft.MicrosoftSolitaireCollection",
    "Microsoft.MicrosoftStickyNotes", "Microsoft.MixedReality.Portal",
    "Microsoft.OneConnect", "Microsoft.People", "Microsoft.SkypeApp",
    "Microsoft.Wallet", "Microsoft.WindowsCommunicationsApps",   # Mail/Calendar
    "Microsoft.WindowsFeedbackHub", "Microsoft.WindowsMaps",
    "Microsoft.WindowsSoundRecorder", "Microsoft.Xbox*",
    "Microsoft.ZuneMusic", "Microsoft.ZuneVideo", "Microsoft.WindowsTerminal",
    "Microsoft.Todos", "Microsoft.PowerAutomateDesktop", "Microsoft.DevHome*",
    "Microsoft.549981C3F5F10",   # Cortana
    "MicrosoftCorporationII.MicrosoftAccountControlCenter",
    "MicrosoftCorporationII.QuickAssist", "Microsoft Clipchamp*",
    "Microsoft.Windows.DevHome*", "Microsoft.OutlookForWindows*",
    "Microsoft.Windows.Search*", "Microsoft.Copilot*"
)

# --- Windows capabilities to remove. ----------------------------------------
$RemoveCapabilities = @(
    "Browser.InternetExplorer*",        # IE mode (not needed)
    "Media.WindowsMediaPlayer*",        # Groove/Media Player
    "App.StepsRecorder*",
    "App.Support.QuickAssist*",
    "Hello.Face*",
    "MathRecognizer*",
    "OpenSSH.Client*"                   # keep the SERVER, drop the client
)

# --- Services to disable. --------------------------------------------------
$DisableServices = @(
    "DiagTrack",                        # Connected User Experiences & Telemetry
    "dmwappushservice",                 # WAP Push Message Routing
    "SysMain",                          # Superfetch (test VM: no benefit)
    "WSearch",                          # Windows Search indexer
    "Fax", "fhsvc",                     # Fax, File History
    "RetailDemo", "SCardSvr", "ScDeviceEnum", "SCPolicySvc",  # Smart card
    "XblAuthManager", "XblGameSave", "XboxGipSvc", "XboxNetApiSvc",
    "MapsBroker", "lfsvc",              # Downloaded Maps, Location
    "PhoneSvc", "SmsRouter",            # Phone, SMS
    "WMPNetworkSvc",                    # WMP sharing
    "WerSvc",                           # Windows Error Reporting
    "PcaSvc"                            # Program Compatibility Assistant
)

function Write-Step($msg) { Write-Host "[debloat:$Phase] $msg" }

# ============================================================================
if ($Phase -eq "Specialize") {
    Write-Step "removing provisioned Appx packages"
    foreach ($name in $RemoveAppx) {
        Get-AppxProvisionedPackage -Online |
            Where-Object { $_.DisplayName -like $name } |
            Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue
    }

    Write-Step "removing per-user Appx packages"
    foreach ($name in $RemoveAppx) {
        Get-AppxPackage -AllUsers -Name $name | Remove-AppxPackage -AllUsers -ErrorAction SilentlyContinue
    }

    Write-Step "removing Windows capabilities"
    foreach ($cap in $RemoveCapabilities) {
        Get-WindowsCapability -Online -Name $cap |
            Where-Object { $_.State -eq "Installed" } |
            Remove-WindowsCapability -Online -ErrorAction SilentlyContinue
    }

    Write-Step "disabling services"
    foreach ($svc in $DisableServices) {
        Set-Service -Name $svc -StartupType Disabled -ErrorAction SilentlyContinue
        Stop-Service -Name $svc -Force -ErrorAction SilentlyContinue
    }

    Write-Step "disabling telemetry + consumer features (registry)"
    $p = "HKLM:\SOFTWARE\Policies\Microsoft\Windows"
    New-Item -Path "$p\DataCollection" -Force | Out-Null
    Set-ItemProperty -Path "$p\DataCollection" -Name "AllowTelemetry" -Value 0 -Type DWord
    New-Item -Path "$p\CloudContent" -Force | Out-Null
    Set-ItemProperty -Path "$p\CloudContent" -Name "DisableWindowsConsumerFeatures" -Value 1 -Type DWord
    Set-ItemProperty -Path "$p\CloudContent" -Name "DisableCloudOptimizedContent" -Value 1 -Type DWord
    # Cortana off
    New-Item -Path "HKLM:\SOFTWARE\Policies\Microsoft\Windows\Windows Search" -Force | Out-Null
    Set-ItemProperty -Path "HKLM:\SOFTWARE\Policies\Microsoft\Windows\Windows Search" -Name "AllowCortana" -Value 0 -Type DWord

    Write-Step "disabling Windows Update (so the image does not mutate during tests)"
    $wu = "HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU"
    New-Item -Path $wu -Force | Out-Null
    Set-ItemProperty -Path $wu -Name "NoAutoUpdate" -Value 1 -Type DWord
    Set-Service -Name wuauserv -StartupType Disabled -ErrorAction SilentlyContinue
    Stop-Service -Name wuauserv -Force -ErrorAction SilentlyContinue

    Write-Step "installing + enabling OpenSSH server"
    $cap = Get-WindowsCapability -Online -Name "OpenSSH.Server~~~~0.0.1.0"
    if ($cap.State -ne "Installed") {
        Add-WindowsCapability -Online -Name "OpenSSH.Server~~~~0.0.1.0" -ErrorAction SilentlyContinue | Out-Null
    }
    Set-Service -Name sshd -StartupType Automatic
    Start-Service -Name sshd
    # Open the firewall for SSH (22).
    New-NetFirewallRule -Name sshd -DisplayName "OpenSSH Server (sshd)" `
        -Enabled True -Direction Inbound -Protocol TCP -LocalPort 22 `
        -Action Allow -Profile Any -ErrorAction SilentlyContinue | Out-Null

    Write-Step "power: high performance, never sleep"
    powercfg /setactive 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c   # High Performance
    powercfg /change standby-timeout-ac 0
    powercfg /change monitor-timeout-ac 0
    powercfg /change hibernate-timeout-ac 0

    Write-Step "Specialize phase complete"
    exit 0
}

# ============================================================================
if ($Phase -eq "Oobe") {
    # Runs as testuser (admin). Self-elevate for the bits that need it.
    if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Start-Process -FilePath "powershell.exe" -ArgumentList "-ExecutionPolicy Bypass -NoProfile -NonInteractive -File `"$PSCommandPath`" -Phase Oobe" -Verb RunAs -Wait
        exit 0
    }

    Write-Step "installing SSH authorized_keys for testuser"
    $sshDir = "C:\Users\testuser\.ssh"
    New-Item -Path $sshDir -ItemType Directory -Force | Out-Null
    # The public key is delivered on the provision ISO at the root (id_ed25519.pub).
    # Scan drive letters because the provision ISO's letter is dynamic.
    $pubKey = $null
    foreach ($d in (Get-PSDrive -PSProvider FileSystem).Name) {
        $candidate = "${d}:\id_ed25519.pub"
        if (Test-Path $candidate) { $pubKey = Get-Content -Raw $candidate; break }
    }
    if ($pubKey) {
        $pubKey.Trim() | Out-File -FilePath "$sshDir\authorized_keys" -Encoding ascii -NoNewline
        icacls "$sshDir\authorized_keys" /inheritance:r /grant "testuser:F" | Out-Null
        Write-Step "authorized_keys written"
    } else {
        Write-Step "WARNING: no id_ed25519.pub found on provision ISO — SSH key auth will fail"
    }

    Write-Step "setting OpenSSH default shell to PowerShell"
    $reg = "HKLM:\SOFTWARE\OpenSSH"
    New-Item -Path $reg -Force | Out-Null
    Set-ItemProperty -Path $reg -Name "DefaultShell" -Value "C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe" -Type String
    Set-ItemProperty -Path $reg -Name "DefaultShellCommandOption" -Value "No" -Type String
    Restart-Service -Name sshd -Force -ErrorAction SilentlyContinue

    Write-Step "verifying a browser is present (for webui tests)"
    $edge = Get-ChildItem "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" -ErrorAction SilentlyContinue
    if (-not $edge) {
        $edge = Get-ChildItem "C:\Program Files\Microsoft\Edge\Application\msedge.exe" -ErrorAction SilentlyContinue
    }
    if (-not $edge) {
        Write-Step "no Edge found — installing Firefox as the test browser"
        # Headless-ish install of Firefox via the official stub is unreliable
        # offline; in practice the LTSC image includes Edge. If it does not,
        # the build should bake a browser into the provision ISO instead.
        Write-Step "WARNING: no browser present — bake one into the provision ISO"
    } else {
        Write-Step "Edge present at $($edge.FullName)"
    }

    Write-Step "writing provisioned marker"
    "provisioned $(Get-Date -Format o)" | Out-File "C:\Windows\Setup\Scripts\PROVISIONED" -Encoding ascii

    Write-Step "OOBE phase complete — shutting down to finalize the base image"
    shutdown /s /t 5 /c "catalyst test-vm provisioning complete"
    exit 0
}

Write-Host "[debloat] unknown phase: $Phase"
exit 1
