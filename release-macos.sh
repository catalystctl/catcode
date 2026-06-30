#!/usr/bin/env bash
# Build a single, self-contained macOS executable per arch (arm64 + x86_64).
# The Rust core is cross-compiled with cargo-zigbuild (zig as the macOS linker);
# the Go TUI is then built with the core embedded via go:embed (-tags embed_core)
# so the result is ONE file per arch. Run it from any CWD and it extracts its
# bundled core to ~/Library/Caches/umans-harness and launches the harness in that
# CWD — no separate umans-core, no install step.
#
# Run on Linux. Needs: cargo + the aarch64-apple-darwin / x86_64-apple-darwin
# rustup targets, Go 1.21+, zig (0.13+) on PATH, and cargo-zigbuild:
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

mkdir -p dist tui/embed
# Always remove a stale injected core on exit so it never leaks into the tree.
trap 'rm -f "$EMBED_FILE"' EXIT

echo "==> building umans-harness ${VERSION} for macOS (standalone, core embedded)"

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
}

build_arch aarch64-apple-darwin arm64 arm64
build_arch x86_64-apple-darwin  amd64 x86_64

echo "[3/3] checksums..."
( cd dist
  for f in umans-harness-${VERSION}-macos-arm64 umans-harness-${VERSION}-macos-x86_64; do
	sha256sum "$f" > "$f.sha256"
  done )

# P1-23: for distribution, codesign + notarize each binary with an Apple
# Developer ID in CI (not done here — no signing identity available):
#   codesign --force --options runtime --sign "Developer ID Application: ..." "$f"
#   ditto -c -k --keepParent "$f" "${f}.zip"
#   xcrun notarytool submit "${f}.zip" --apple-id ... --team-id ... --password ... --wait
#   xcrun stapler staple "$f"
# Without this, Gatekeeper blocks the unsigned binary for most users.

echo "==> dist/umans-harness-${VERSION}-macos-arm64   (Apple Silicon)"
echo "==> dist/umans-harness-${VERSION}-macos-x86_64  (Intel)"
echo
echo "Download + run on a Mac:"
echo "  chmod +x umans-harness-${VERSION}-macos-arm64"
echo "  ./umans-harness-${VERSION}-macos-arm64      # launches in the current directory"
echo "First run: /key sk-...  then /model, then type a prompt."
