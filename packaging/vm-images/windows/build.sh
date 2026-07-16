#!/usr/bin/env bash
# build.sh — build the Windows 11 IoT Enterprise LTSC base qcow2 for the
# catalyst test_env tool.
#
# Flow: ensure deps → fetch virtio-win ISO → obtain the Windows ISO → build a
# "provision" ISO (autounattend.xml + debloat.ps1 via $OEM$ + SSH pubkey) →
# boot QEMU/KVM with swtpm → unattend installs + debloats + shuts down → the
# resulting qcow2 is the base image clones boot from.
#
# Image source (set WIN_SOURCE):
#   iso   : use a pre-downloaded ISO at $WIN_ISO            (most reliable)
#   eval  : auto-download the Enterprise LTSC 2024 eval ISO (reliable-ish)
#   uup   : build the genuine IoT Enterprise LTSC ISO via UUP (experimental)
# Default: iso if $WIN_ISO exists, else eval.
#
# Output: $OUT (default: ~/.catalyst-code/vm-images/windows-11-iot-ltsc.qcow2)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${OUT:-$HOME/.catalyst-code/vm-images/windows-11-iot-ltsc.qcow2}"
DISK_GB="${DISK_GB:-64}"
RAM_MB="${RAM_MB:-8192}"
CPUS="${CPUS:-4}"
WIN_SOURCE="${WIN_SOURCE:-}"
WIN_ISO="${WIN_ISO:-}"
WIN_EVAL_URL="${WIN_EVAL_URL:-}"   # optional direct eval-ISO URL

SSH_KEY_DIR="${SSH_KEY_DIR:-$HOME/.catalyst-code/test-env}"
SSH_KEY="${SSH_KEY_DIR}/id_ed25519"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

log() { echo "[build] $*"; }
die() { echo "[build] ERROR: $*" >&2; exit 1; }

# ── 1. dependencies ────────────────────────────────────────────────────────
need() { command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1 (install it first)"; }
for d in qemu-system-x86_64 qemu-img swtpm xorriso; do need "$d"; done

# ── 2. virtio-win ISO ───────────────────────────────────────────────────────
VIRTIO_ISO="$WORK/virtio-win.iso"
bash "$HERE/fetch-virtio-win.sh" "$VIRTIO_ISO"

# ── 3. Windows ISO ─────────────────────────────────────────────────────────
WIN_ISO_OUT="$WORK/win.iso"
if [[ -z "$WIN_SOURCE" ]]; then
    if [[ -n "$WIN_ISO" && -f "$WIN_ISO" ]]; then WIN_SOURCE="iso"; fi
    if [[ -z "$WIN_SOURCE" ]]; then WIN_SOURCE="eval"; fi
fi
log "image source: $WIN_SOURCE"

case "$WIN_SOURCE" in
    iso)
        [[ -n "$WIN_ISO" && -f "$WIN_ISO" ]] || die "WIN_SOURCE=iso but WIN_ISO is not set/missing"
        ln -sf "$WIN_ISO" "$WIN_ISO_OUT"
        log "using provided ISO: $WIN_ISO"
        ;;
    eval)
        if [[ -z "$WIN_EVAL_URL" ]]; then
            cat >&2 <<'EOF'
[build] WIN_SOURCE=eval requires WIN_EVAL_URL.
  Download "Windows 11 Enterprise LTSC 2024" from the Microsoft Evaluation
  Center (https://www.microsoft.com/evalcenter/evaluate-windows-11-enterprise)
  and pass the direct .iso URL via WIN_EVAL_URL, or set WIN_ISO to the file.
  (The eval ISO is Enterprise LTSC, not IoT — functionally equivalent for
  ephemeral test VMs. For the genuine IoT SKU use WIN_SOURCE=uup.)
EOF
            exit 1
        fi
        log "downloading eval ISO"
        curl -fL --retry 3 -o "$WIN_ISO_OUT" "$WIN_EVAL_URL"
        ;;
    uup)
        log "building IoT Enterprise LTSC ISO via UUP (experimental)"
        bash "$HERE/fetch-uup.sh" "$WIN_ISO_OUT" "$WORK/uup-build"
        ;;
    *) die "unknown WIN_SOURCE=$WIN_SOURCE (use iso|eval|uup)" ;;
esac
[[ -f "$WIN_ISO_OUT" ]] || die "Windows ISO not present at $WIN_ISO_OUT"

# ── 4. SSH keypair (for test_env exec over SSH) ────────────────────────────
mkdir -p "$SSH_KEY_DIR" && chmod 700 "$SSH_KEY_DIR"
if [[ ! -f "$SSH_KEY" ]]; then
    log "generating SSH keypair at $SSH_KEY"
    ssh-keygen -t ed25519 -N "" -f "$SSH_KEY" -C "catalyst-test-env" >/dev/null
fi
chmod 600 "$SSH_KEY"

# ── 5. provision ISO (autounattend.xml + $OEM$/debloat.ps1 + pubkey) ────────
log "building provision ISO"
PROV="$WORK/provision"
mkdir -p "$PROV/\$OEM\$/\$\$/Setup/Scripts"
cp "$HERE/autounattend.xml" "$PROV/autounattend.xml"
cp "$HERE/debloat.ps1"      "$PROV/\$OEM\$/\$\$/Setup/Scripts/debloat.ps1"
cp "$SSH_KEY.pub"           "$PROV/id_ed25519.pub"

# Inject a product key if provided (pins the IoT LTSC edition on UUP ISOs).
if [[ -f "$HERE/product-key.txt" ]]; then
    KEY="$(tr -d '[:space:]' < "$HERE/product-key.txt")"
    log "injecting product key into autounattend"
    python3 - "$PROV/autounattend.xml" "$KEY" <<'PY'
import re, sys
path, key = sys.argv[1], sys.argv[2]
s = open(path).read()
s = s.replace("<!-- PRODUCTKEY_PLACEHOLDER: build.sh injects <ProductKey> here if\n             product-key.txt is present (genuine IoT LTSC edition pinning).\n             Absent = keyless/eval install (image index selects edition). -->",
              f"<ProductKey><Key>{key}</Key><WillShowUI>OnError</WillShowUI></ProductKey>")
open(path, "w").write(s)
PY
fi

PROV_ISO="$WORK/provision.iso"
xorriso -as mkisofs -iso-level 4 -J -R -V PROVISION -o "$PROV_ISO" "$PROV" 2>/dev/null

# ── 6. blank disk + swtpm ──────────────────────────────────────────────────
DISK="$WORK/disk.qcow2"
log "creating blank disk ($DISK_GB GB)"
qemu-img create -f qcow2 "$DISK" "${DISK_GB}G" >/dev/null

SWTPM_DIR="$WORK/swtpm-state"
SWTPM_SOCK="$WORK/swtpm.sock"
mkdir -p "$SWTPM_DIR"
log "starting swtpm"
swtpm socket --tpmstate "dir=$SWTPM_DIR" \
    --ctrl "type=unixio,path=$SWTPM_SOCK" --tpm2 --daemon
trap 'rm -rf "$WORK"; pkill -f "$SWTPM_SOCK" 2>/dev/null || true' EXIT

# ── 7. boot QEMU and wait for the unattend to shut down ─────────────────────
log "booting QEMU/KVM — install + debloat runs unattended (~30-45 min)"
log "  (VNC on :0 if you want to watch: vncviewer localhost:5900)"
ACCEL="kvm"
if [[ "$(uname -s)" == "Darwin" ]]; then ACCEL="hvf"; fi

qemu-system-x86_64 \
    -enable-kvm -accel "$ACCEL" \
    -machine q35 -cpu host -smp "$CPUS" -m "$RAM_MB" \
    -drive file="$DISK",format=qcow2,if=virtio \
    -drive file="$WIN_ISO_OUT",media=cdrom \
    -drive file="$VIRTIO_ISO",media=cdrom \
    -drive file="$PROV_ISO",media=cdrom \
    -netdev user,id=n0 -device virtio-net,netdev=n0 \
    -chardev socket,id=chrtpm,path="$SWTPM_SOCK" \
    -tpmdev emulator,id=tpm0,chardev=chrtpm -device tpm-crb \
    -display none -vnc :0 \
    -boot d -no-reboot

# QEMU exits when the guest shuts down (the unattend's final step).
log "guest shut down — finalizing base image"

# ── 8. publish the base qcow2 ─────────────────────────────────────────────
mkdir -p "$(dirname "$OUT")"
mv "$DISK" "$OUT"
log "done: $OUT"
log "set CATALYST_TESTENV_WINDOWS_BASE=$OUT so the test_env tool finds it"
