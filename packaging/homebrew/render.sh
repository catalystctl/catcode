#!/usr/bin/env bash
# Render the Homebrew cask + formula for a published release, substituting the
# version + per-arch sha256 into the templates under packaging/homebrew/.
#
#   ./render.sh <version> <arm64-bin-sha> <x86_64-bin-sha> <arm64-dmg-sha> <x86_64-dmg-sha> <out-dir>
#
# The cask gets the .dmg sha256 (it installs from the .dmg); the formula gets
# the standalone-binary sha256 (it installs the raw binary). Writes:
#   <out-dir>/Casks/catcode.rb
#   <out-dir>/Formula/catcode.rb
#
# Used by .github/workflows/homebrew-tap.yml. Also runnable locally for a
# dry-run, e.g.:
#   ./packaging/homebrew/render.sh 0.2.0 aaaa bbbb cccc dddd /tmp/tap-out
set -euo pipefail

if [[ $# -ne 6 ]]; then
	echo "usage: $0 <version> <arm64-bin-sha> <x86_64-bin-sha> <arm64-dmg-sha> <x86_64-dmg-sha> <out-dir>" >&2
	exit 2
fi

VERSION="$1"; ARM_BIN_SHA="$2"; INTEL_BIN_SHA="$3"; ARM_DMG_SHA="$4"; INTEL_DMG_SHA="$5"; OUT="$6"

# Resolve the repo root (this script lives at packaging/homebrew/render.sh).
cd "$(dirname "$0")/../.."

for s in "$ARM_BIN_SHA" "$INTEL_BIN_SHA" "$ARM_DMG_SHA" "$INTEL_DMG_SHA"; do
	if [[ ! "$s" =~ ^[0-9a-f]{64}$ ]]; then
		echo "error: not a 64-hex sha256: $s" >&2; exit 1
	fi
done

mkdir -p "$OUT/Casks" "$OUT/Formula"

sed -e "s|@@VERSION@@|${VERSION}|g" \
    -e "s|@@SHA256_ARM@@|${ARM_DMG_SHA}|g" \
    -e "s|@@SHA256_INTEL@@|${INTEL_DMG_SHA}|g" \
    packaging/homebrew/Casks/catcode.rb > "$OUT/Casks/catcode.rb"

sed -e "s|@@VERSION@@|${VERSION}|g" \
    -e "s|@@SHA256_ARM@@|${ARM_BIN_SHA}|g" \
    -e "s|@@SHA256_INTEL@@|${INTEL_BIN_SHA}|g" \
    packaging/homebrew/Formula/catcode.rb > "$OUT/Formula/catcode.rb"

echo "rendered:"
( cd "$OUT" && find . -type f | sort )
