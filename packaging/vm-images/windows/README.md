# Windows 11 IoT Enterprise LTSC test-VM image

Builds a debloated **Windows 11 IoT Enterprise LTSC 2024** qcow2 for the
`test_env` tool. The agent spins up clones of this image to test Windows-specific
webuis/TUIs, and can VNC into the screen or grab screenshots.

## What it produces

A base qcow2 that boots straight to a logged-in, debloated desktop via
`AutoLogon` (~20-30s per clone). Inside: a browser (Edge), PowerShell, OpenSSH
server (key auth), networking. Removed: Xbox, Mail/Calendar, Maps, Weather,
Cortana, Teams, Clipchamp, telemetry, Windows Update, IE mode, and the rest of
the consumer/embedded cruft you don't need to drive a browser or terminal.

## Prerequisites (Linux host)

```
qemu-system-x86_64 qemu-img swtpm xorriso wimlib cabextract curl jq python3
```
KVM must be available (`ls -la /dev/kvm`). On macOS swap `kvm`→`hvf` (set in
build.sh). ~8 GB RAM + 64 GB disk per running VM.

## Build

```bash
# Easiest: provide an ISO you already have (VLSC / Visual Studio Subscription,
# or one you built via UUP). Reliable.
WIN_ISO=/path/to/win11-iot-ltsc.iso bash build.sh

# Auto-download the Enterprise LTSC 2024 eval ISO (functionally equivalent for
# ephemeral test VMs; time-limited but fine since clones are destroyed).
# Get the direct .iso URL from the Evaluation Center, then:
WIN_SOURCE=eval WIN_EVAL_URL='https://.../Win11_..._LTSC_Eval.iso' bash build.sh

# Genuine IoT Enterprise LTSC, built from Microsoft's update servers via UUP.
# Experimental — needs validation on a KVM host.
WIN_SOURCE=uup bash build.sh
```

Output: `~/.catalyst-code/vm-images/windows-11-iot-ltsc.qcow2`
Point the tool at it: `CATALYST_TESTENV_WINDOWS_BASE=<path>`

## How the unattend works

| pass | runs as | does |
|---|---|---|
| `windowsPE` | WinPE | wipe disk → GPT (EFI/MSR/Windows) → install LTSC image index 1 → load virtio drivers → Win11 check bypass (swtpm fallback) |
| `specialize` | SYSTEM | remove Appx/capabilities/services → kill telemetry → disable Windows Update → install + enable OpenSSH server → high-performance power |
| `oobeSystem` | testuser (admin) | create local admin → `AutoLogon` → drop SSH authorized key → set OpenSSH default shell to PowerShell → **shut down** (finalizes the base image) |

`debloat.ps1` is delivered via the `$OEM$/$$/Setup/Scripts/` folder on the
provision ISO, so it lands at `C:\Windows\Setup\Scripts\debloat.ps1` before
first logon. The SSH public key (`id_ed25519.pub`) rides on the same ISO root.

## Files

| file | purpose |
|---|---|
| `build.sh` | orchestrator: deps → ISOs → QEMU install → base qcow2 |
| `autounattend.xml` | the unattended-install answer file |
| `debloat.ps1` | bloat removal + OpenSSH + finalize (Specialize/Oobe phases) |
| `fetch-virtio-win.sh` | download the Fedora virtio-win driver ISO |
| `fetch-uup.sh` | build the genuine IoT LTSC ISO from MS update servers (experimental) |
| `product-key.txt` | (optional, gitignored) KMS client setup key to pin the IoT LTSC edition |

## Licensing

The built qcow2 contains Microsoft software — **build it locally per host; do
not commit or redistribute it** (see `.gitignore`). Eval/UUP terms permit
running in a VM for testing. The scripts, XML, and PowerShell here are our own
and redistributable.

## Defaults you may want to change

- **Browser**: stock Edge is kept. To use Chrome/Firefox instead, bake the
  installer into the provision ISO and run it in the Oobe phase.
- **Image source**: `iso` (provided) is most reliable; `uup` gives the genuine
  IoT SKU but is experimental.
- **Product key**: omit for keyless/eval; drop one in `product-key.txt` to pin
  the IoT Enterprise LTSC edition on a UUP-built ISO.
