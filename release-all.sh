#!/usr/bin/env bash
# Build EVERY release artifact for catalyst-code across all platforms:
#
#   Windows  -> MSI installer  + standalone catcode.exe        (release-windows.sh)
#   macOS    -> .dmg installer  + standalone executable     (release-macos.sh)
#   Linux    -> AppImage        + standalone executable     (release-linux.sh)
#
# Each platform script is run independently and per-platform pass/fail is
# reported, so a host with only a partial toolchain (e.g. no zig for macOS)
# still builds whatever it can instead of aborting on the first missing tool.
# Exit status is non-zero if any platform failed.
#
#   ./release-all.sh [version]     # version defaults to the git commit (short SHA)
set -uo pipefail
cd "$(dirname "$0")" || exit

VERSION="${1:-$(git rev-parse --short HEAD 2>/dev/null || grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
echo "############ catalyst-code ${VERSION} — full release (all platforms) ############"

pass=0; fail=0; failed=""

run() {
	local name="$1" script="$2"
	echo
	echo "==================== $name ===================="
	if bash "$script" "$VERSION"; then
		echo "==================== $name: OK ===================="
		pass=$((pass + 1))
	else
		echo "==================== $name: FAILED ===================="
		fail=$((fail + 1))
		failed="${failed} ${name%% *}"
	fi
}

run "Windows  (MSI + standalone .exe)"  release-windows.sh
run "macOS    (standalone + .dmg)"      release-macos.sh
run "Linux    (standalone + AppImage)"  release-linux.sh

echo
echo "############ release summary ############"
echo "  passed : $pass"
echo "  failed : $fail${failed:+  (${failed# })}"
echo "  dist/  :"
( cd dist 2>/dev/null && for f in catcode-* catalyst-code-*; do [ -e "$f" ] && printf '    %s\n' "$f"; done | sort -u ) || echo "    (none)"

if [ "$fail" -ne 0 ]; then
	echo
	echo "One or more platforms failed — see the per-platform output above."
	echo "Common gaps: macOS needs zig + cargo-zigbuild; Linux AppImage needs"
	echo "network once to fetch appimagetool (or set APPIMAGETOOL=<path>)."
	exit 1
fi
echo
echo "All platforms built. Ship the dist/ artifacts."
