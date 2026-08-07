#!/usr/bin/env bash
# Reject an ELF binary that requires a newer glibc than the supported baseline.
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
	echo "usage: $0 <elf-binary> [maximum-glibc-version]" >&2
	exit 2
fi

binary="$1"
maximum="${2:-2.35}"

[[ -f "$binary" ]] || { echo "error: binary not found: $binary" >&2; exit 2; }
command -v readelf >/dev/null 2>&1 || {
	echo "error: readelf is required to verify the Linux glibc baseline" >&2
	exit 2
}

required="$({ readelf --version-info "$binary" 2>/dev/null || true; } \
	| grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' \
	| sed 's/^GLIBC_//' \
	| sort -Vu \
	| tail -n 1 || true)"

if [[ -z "$required" ]]; then
	echo "error: no GLIBC symbol versions found in $binary (not a glibc-linked ELF binary?)" >&2
	exit 1
fi

newest="$(printf '%s\n%s\n' "$maximum" "$required" | sort -V | tail -n 1)"
if [[ "$newest" != "$maximum" ]]; then
	echo "error: $binary requires GLIBC_$required; supported maximum is GLIBC_$maximum" >&2
	exit 1
fi

echo "glibc baseline OK: $binary requires at most GLIBC_$required (maximum GLIBC_$maximum)"
