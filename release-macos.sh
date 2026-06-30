#!/usr/bin/env bash
# Build self-contained macOS artifacts for the Umans AI harness, per arch
# (arm64 + x86_64):
#
#   1. A standalone executable — the TUI with the Rust core embedded via go:embed
#      (-tags embed_core). One file per arch; run it from any CWD and it extracts
#      its bundled core to ~/Library/Caches/umans-harness on first run — no
#      separate umans-core, no install.
#
#   2. A ucli .dmg installer wrapping that standalone executable. The .dmg
#      contains `ucli` (the standalone) + a double-clickable "Install
#      ucli.command" that copies it onto your PATH, + a README. Mounts on macOS
#      and installs with one click.
#
# Run on Linux. Needs: cargo + the aarch64-apple-darwin / x86_64-apple-darwin
# rustup targets, Go 1.21+, zig (0.13+) on PATH, cargo-zigbuild, and (to build
# the .dmg) xorriso. Building the .dmg ON a Mac instead uses hdiutil (real UDIF).
#
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   cargo install cargo-zigbuild      # and put zig on PATH
#
#   ./release-macos.sh [version]     # version defaults to core/Cargo.toml
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
EMBED_FILE="tui/embed/umans-core"

require() { command -v "$1" >/dev/null 2>&1 || { echo "error: '$1' not found — $2" >&2; exit 1; }; }
require cargo         "install rustup/rust"
require go            "https://go.dev/dl/"
require zig           "install zig 0.13+ from https://ziglang.org/download/ and put it on PATH"
require cargo-zigbuild "cargo install cargo-zigbuild"
# The .dmg needs hdiutil (macOS) or xorriso (Linux). Fail fast with a clear
# message if neither is present so the user isn't told after a long build.
if ! command -v hdiutil >/dev/null 2>&1 && ! command -v xorriso >/dev/null 2>&1; then
	echo "error: need hdiutil (on macOS) or xorriso (on Linux) to build the .dmg" >&2
	echo "       install xorriso (e.g. apt install xorriso / brew install xorriso)" >&2
	exit 1
fi

mkdir -p dist tui/embed
# Always remove a stale injected core on exit so it never leaks into the tree.
trap 'rm -f "$EMBED_FILE"' EXIT

echo "==> building umans-harness ${VERSION} for macOS (standalone + .dmg, core embedded)"

# make_dmg <standalone_path> <tag>  -> dist/ucli-<ver>-macos-<tag>.dmg
make_dmg() {
	local standalone="$1" tag="$2"
	local volname="ucli-${VERSION}-macos-${tag}"
	local out="dist/ucli-${VERSION}-macos-${tag}.dmg"
	local stage; stage="$(mktemp -d)"
	# Layout: ucli (the standalone, renamed) + one-click installer + README.
	cp "$standalone" "$stage/ucli"
	cp packaging/macos/install.command "$stage/Install ucli.command"
	sed "s/<VERSION>/${VERSION}/g; s/<ARCH>/${tag}/g" \
		packaging/macos/README.txt > "$stage/README.txt"
	chmod +x "$stage/ucli" "$stage/Install ucli.command"
	if command -v hdiutil >/dev/null 2>&1; then
		# Real UDIF compressed DMG (build host is macOS).
		hdiutil create -srcfolder "$stage" -volname "$volname" \
			-fs HFS+ -format UDZO -ov "$out" >/dev/null
	elif command -v xorriso >/dev/null 2>&1; then
		# HFS+ hybrid image (build host is Linux; mounts on macOS via
		# DiskImageMounter). The ISO9660 volid-compliance warning, if any,
		# is cosmetic — the HFS+ volume name is what macOS displays.
		xorriso -as mkisofs -V "$volname" -hfsplus -rock -no-pad \
			-o "$out" "$stage" >/dev/null 2>&1 || \
			xorriso -as mkisofs -V "$volname" -hfsplus -rock -no-pad -o "$out" "$stage"
	else
		echo "error: need hdiutil (macOS) or xorriso (Linux) to build the .dmg" >&2
		echo "       (the standalone was still built; only the .dgm was skipped)" >&2
		rm -rf "$stage"; return 1
	fi
	rm -rf "$stage"
	echo "    -> ${out}  ($(du -h "$out" | cut -f1))"
}

build_arch() {
	local rust_target="$1" goarch="$2" tag="$3"

	echo "[1/3] core -> ${rust_target} (cargo zigbuild --release)..."
	cargo zigbuild --release --target "$rust_target" --manifest-path core/Cargo.toml
	# cargo-zigbuild writes to the standard target/<triple>/release path; the
	# [[bin]] name in core/Cargo.toml is "core" (no suffix on macOS).
	local corebin="core/target/${rust_target}/release/core"
	[[ -f "$corebin" ]] || { echo "error: expected core binary at $corebin" >&2; exit 1; }
	cp "$corebin" "$EMBED_FILE"

	echo "[2/3] tui -> darwin/${goarch} (go build -tags embed_core, core embedded)..."
	local out="dist/umans-harness-${VERSION}-macos-${tag}"
	( cd tui && CGO_ENABLED=0 GOOS=darwin GOARCH="$goarch" \
		go build -trimpath -tags embed_core \
			-ldflags "-s -w -X main.coreVersion=${VERSION}" \
			-o "../${out}" . )
	chmod +x "${out}"
	rm -f "$EMBED_FILE"
	echo "    -> ${out}  ($(du -h "${out}" | cut -f1))"

	echo "[3/3] dmg -> dist/ucli-${VERSION}-macos-${tag}.dmg..."
	make_dmg "${out}" "${tag}" || echo "    (warning: .dmg for ${tag} skipped — see message above)"
}

build_arch aarch64-apple-darwin arm64 arm64
build_arch x86_64-apple-darwin  amd64 x86_64

echo "[done] checksums..."
( cd dist
  for f in umans-harness-${VERSION}-macos-arm64 umans-harness-${VERSION}-macos-x86_64 \
           ucli-${VERSION}-macos-arm64.dmg    ucli-${VERSION}-macos-x86_64.dmg; do
	[[ -f "$f" ]] && sha256sum "$f" > "$f.sha256"
  done )

# P1-23: for distribution, codesign + notarize each artifact with an Apple
# Developer ID in CI (not done here — no signing identity available):
#   codesign --force --options runtime --sign "Developer ID Application: ..." "$f"
#   ditto -c -k --keepParent "$f" "${f}.zip"
#   xcrun notarytool submit "${f}.zip" --apple-id ... --team-id ... --password ... --wait
#   xcrun stapler staple "$f"
# Without this, Gatekeeper blocks the unsigned artifacts for most users.

echo "==> dist/umans-harness-${VERSION}-macos-arm64   (standalone, Apple Silicon)"
echo "==> dist/umans-harness-${VERSION}-macos-x86_64  (standalone, Intel)"
echo "==> dist/ucli-${VERSION}-macos-arm64.dmg         (installer, Apple Silicon)"
echo "==> dist/ucli-${VERSION}-macos-x86_64.dmg        (installer, Intel)"
echo
echo "Download + run the standalone from any directory:"
echo "  chmod +x umans-harness-${VERSION}-macos-arm64"
echo "  ./umans-harness-${VERSION}-macos-arm64      # launches in the current directory"
echo "Or install via the .dmg:  open ucli-${VERSION}-macos-arm64.dmg  -> double-click 'Install ucli.command'"
echo "Then run from any terminal:  ucli"
echo "First run: /key sk-...  then /model, then type a prompt."
