#!/usr/bin/env bash
# fetch-virtio-win.sh — download the Fedora virtio-win ISO (freely
# redistributable) used to give the Win11 installer virtio storage/net drivers.
#
# The stable URL points at the latest release on Fedora People. We verify the
# download with the published SHA256 checksum when available.
set -euo pipefail

VIRTIO_VERSION="${VIRTIO_VERSION:-latest}"
OUT="${1:-virtio-win.iso}"
BASE="https://fedorapeople.org/groups/virt/virtio-win"

echo "[virtio-win] fetching $VIRTIO_VERSION → $OUT"
if [[ "$VIRTIO_VERSION" == "latest" ]]; then
    # The "latest" symlink on Fedora People resolves to the newest stable.
    URL="$BASE/virtio-win.iso"
else
    URL="$BASE/virtio-win-$VIRTIO_VERSION.iso"
fi

if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 -o "$OUT" "$URL"
else
    wget -O "$OUT" "$URL"
fi

# Best-effort checksum verification (the .iso.sha256 may not exist for "latest").
SUM_URL="${URL}.sha256"
if curl -fsL "$SUM_URL" -o "$OUT.sha256" 2>/dev/null; then
    expected="$(awk '{print $1}' "$OUT.sha256")"
    actual="$(sha256sum "$OUT" | awk '{print $1}')"
    if [[ "$expected" == "$actual" ]]; then
        echo "[virtio-win] checksum OK ($actual)"
    else
        echo "[virtio-win] WARNING: checksum mismatch (expected $expected, got $actual)" >&2
    fi
else
    echo "[virtio-win] no checksum file at $SUM_URL — skipping verification" >&2
fi

echo "[virtio-win] done: $OUT"
