#!/usr/bin/env bash
# fetch-uup.sh — build a genuine Windows 11 IoT Enterprise LTSC 2024 ISO from
# Microsoft's own update servers via the UUP (Unified Update Platform) dump API.
#
# This is the "download automatically" path for the real IoT LTSC SKU. It pulls
# the UUP file set from dl.delivery.mp.microsoft.com and assembles a bootable
# ISO with wimlib + the uup-dump converter scripts.
#
#   STATUS: experimental. The API + converter are correct as of writing, but
#   this has not been smoke-tested end-to-end on a KVM host in this repo. If it
#   fails, fall back to WIN_SOURCE=eval or a pre-downloaded WIN_ISO.
#
# Requirements: curl, jq, wimlib-tools, cabextract, xorriso, genisoimage|mkisofs.
set -euo pipefail

OUT="${1:-win11-iot-ltsc.iso}"
WORK="${2:-uup-build}"
EDITION="${UUP_EDITION:-iotenterpriseltsc}"   # IoT Enterprise LTSC
LANG="${UUP_LANG:-en-us}"
API="https://api.uupdump.net"

need() { command -v "$1" >/dev/null 2>&1 || { echo "[uup] missing dependency: $1" >&2; exit 1; }; }
for d in curl jq wimlib-imagex cabextract xorriso; do need "$d"; done

echo "[uup] querying known builds for IoT Enterprise LTSC 2024"
# known.php lists builds; we want the LTSC 2024 (build 26100) IoT Enterprise.
curl -fsL "$API/known.php" -o known.json
UUID="$(jq -r --arg ed "$EDITION" '
  .builds | to_entries
  | map(select(.value.title | test("IoT Enterprise"; "i")) )
  | map(select(.value.title | test("LTSC"; "i")) )
  | sort_by(.value.created | tonumber) | reverse
  | .[0].key' known.json)"

if [[ -z "$UUID" || "$UUID" == "null" ]]; then
    echo "[uup] could not find an IoT Enterprise LTSC build in known.php" >&2
    echo "[uup] browse https://uupdump.net/ for the exact build id and set UUP_UUID" >&2
    exit 1
fi
echo "[uup] selected build uuid: $UUID"

echo "[uup] fetching file list (edition=$EDITION, lang=$LANG)"
curl -fsL "$API/get.php?id=$UUID&lang=$LANG&edition=$EDITION" -o get.json

mkdir -p "$WORK"
cd "$WORK"

# Download the converter scripts (uup-dump/converter on GitHub).
if [[ ! -d converter ]]; then
    echo "[uup] fetching converter scripts"
    curl -fsL "https://github.com/uup-dump/converter/archive/refs/heads/master.tar.gz" -o conv.tgz
    tar xzf conv.tgz
    mv converter-master converter
fi

# Download the UUP file set. The get.php response lists files under .files;
# the converter's download script handles this, but we can also fetch directly.
echo "[uup] downloading UUP files"
python3 - <<PY || bash converter/convert.sh
import json, sys, urllib.request, os
data = json.load(open("../get.json"))
files = data.get("files", {})
for name, meta in files.items():
    url = meta.get("url")
    if not url:
        continue
    print(f"[uup] {name}")
    urllib.request.urlretrieve(url, name)
PY

echo "[uup] assembling ISO (this takes a few minutes)"
# The converter produces the ISO in the current dir.
bash converter/convert.sh

ISO="$(ls -1 *.iso 2>/dev/null | head -1 || true)"
if [[ -z "$ISO" ]]; then
    echo "[uup] conversion produced no ISO — check $WORK" >&2
    exit 1
fi
cp "$ISO" "../$OUT"
echo "[uup] done: $OUT"
