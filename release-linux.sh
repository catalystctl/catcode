#!/usr/bin/env bash
# Build self-contained Linux artifacts for the Catalyst Code:
#
#   1. A standalone executable (TUI with the Rust core embedded via go:embed,
#      -tags embed_core) — one file, run from any CWD; it extracts its bundled
#      core to ~/.cache/catalyst-code on first run. No install.
#   2. A catcode AppImage wrapping that standalone executable — double-clickable
#      on desktop Linux and runnable as ./catcode-<ver>-x86_64.AppImage from any
#      terminal; self-contained (squashfs payload, no deps).
#
# Output (dist/):
#   catcode-<ver>-linux-<arch>          standalone executable
#   catcode-<ver>-linux-<arch>.sha256
#   catcode-<ver>-<arch>.AppImage               AppImage installer
#   catcode-<ver>-<arch>.AppImage.sha256
#
# Run on Linux. Needs: cargo (stable), Go 1.21+. appimagetool is fetched on
# demand to ~/.cache/appimagetool/ if not on PATH (needs network once).
#   ./release-linux.sh [version]     # version defaults to the git commit (short SHA)
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(git rev-parse --short HEAD 2>/dev/null || grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
EMBED_FILE="tui/embed/catcode-core"

require() { command -v "$1" >/dev/null 2>&1 || { echo "error: '$1' not found — $2" >&2; exit 1; }; }
require cargo "install rustup/rust"
require go    "https://go.dev/dl/"

# Map the host arch to the labels used in artifact names. AppImage uses
# x86_64/aarch64; the standalone follows the macOS convention (x86_64/arm64).
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
	x86_64|amd64)  APPIMG_ARCH="x86_64";  STANDALONE_ARCH="x86_64" ;;
	aarch64|arm64) APPIMG_ARCH="aarch64"; STANDALONE_ARCH="arm64"  ;;
	*) echo "error: unsupported host arch '$HOST_ARCH' (expected x86_64 or aarch64)" >&2; exit 1 ;;
esac

mkdir -p dist tui/embed
# Never leak an injected core into the tree on exit.
trap 'rm -f "$EMBED_FILE"' EXIT

echo "==> building catalyst-code ${VERSION} for Linux (${HOST_ARCH})"

echo "[1/6] core -> native release (cargo, --locked)..."
cargo build --release --locked --manifest-path core/Cargo.toml
CORE_BIN="core/target/release/core"
[[ -f "$CORE_BIN" ]] || { echo "error: expected core binary at $CORE_BIN" >&2; exit 1; }

echo "[2/6] tui -> standalone (go build -tags embed_core, core embedded)..."
cp "$CORE_BIN" "$EMBED_FILE"
STANDALONE="dist/catcode-${VERSION}-linux-${STANDALONE_ARCH}"
( cd tui && CGO_ENABLED=0 go build -trimpath -tags embed_core \
		-ldflags "-s -w -X main.coreVersion=${VERSION}" -o "../${STANDALONE}" . )
chmod +x "${STANDALONE}"
rm -f "$EMBED_FILE"
echo "    -> ${STANDALONE}  ($(du -h "${STANDALONE}" | cut -f1))"

# Separate core binary (no embed) for the web service's CATCODE_CORE. The
# installer downloads this alongside the standalone when --with-web is used, so
# the web service spawns a real core without depending on the TUI's first-run
# extraction. Built from the SAME cargo artifact as the embedded core — no
# second compile.
CORE_BIN="core/target/release/core"
CORE_ART="dist/catcode-core-${VERSION}-linux-${STANDALONE_ARCH}"
cp "$CORE_BIN" "$CORE_ART"; chmod +x "$CORE_ART"
echo "    -> ${CORE_ART}  ($(du -h "$CORE_ART" | cut -f1))"

echo "[3/6] generating AppImage icon..."
ICON="dist/.appimg-${VERSION}/catcode.png"
mkdir -p "$(dirname "$ICON")"
python3 packaging/linux/make-icon.py "$ICON" 256

echo "[4/6] assembling AppDir..."
APPDIR="dist/.appimg-${VERSION}/catcode.AppDir"
rm -rf "$APPDIR"; mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
                         "$APPDIR/usr/share/applications"
cp "$STANDALONE" "$APPDIR/catcode"; chmod +x "$APPDIR/catcode"
cp packaging/linux/AppRun "$APPDIR/AppRun"; chmod +x "$APPDIR/AppRun"
cp packaging/linux/catcode.desktop "$APPDIR/catcode.desktop"
cp "$ICON" "$APPDIR/.DirIcon"
cp "$ICON" "$APPDIR/catcode.png"
cp "$ICON" "$APPDIR/usr/share/icons/hicolor/256x256/apps/catcode.png"
cp packaging/linux/catcode.desktop "$APPDIR/usr/share/applications/catcode.desktop"

echo "[5/6] building AppImage (appimagetool)..."
APPIMG="dist/catcode-${VERSION}-${APPIMG_ARCH}.AppImage"
resolve_appimagetool() {
	# 1) explicit override
	if [[ -n "${APPIMAGETOOL:-}" && -x "$APPIMAGETOOL" ]]; then echo "$APPIMAGETOOL"; return; fi
	# 2) on PATH
	if command -v appimagetool >/dev/null 2>&1; then command -v appimagetool; return; fi
	# 3) cached fetch
	local cache="$HOME/.cache/appimagetool/appimagetool-${APPIMG_ARCH}.AppImage"
	if [[ -x "$cache" ]]; then echo "$cache"; return; fi
	# 4) download once (needs network). Progress + warnings go to STDERR so the
	# captured stdout is ONLY the resolved path — otherwise $TOOL gets polluted
	# with the "downloading..." line and the exec fails ("No such file").
	echo "    downloading appimagetool (${APPIMG_ARCH})..." >&2
	mkdir -p "$(dirname "$cache")"
	local url="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${APPIMG_ARCH}.AppImage"
	if ! curl -fsSL -o "$cache" "$url"; then
		echo "warning: failed to download appimagetool from $url — skipping AppImage" >&2
		return 1
	fi
	chmod +x "$cache"
	echo "$cache"
}
TOOL="$(resolve_appimagetool || true)"
# appimagetool is itself an AppImage; in headless/CI containers without FUSE
# (libfuse.so.2) it refuses to mount, so run it with APPIMAGE_EXTRACT_AND_RUN=1,
# which extracts to a temp dir and execs the payload — no FUSE required.
# The AppImage is an OPTIONAL no-install convenience — the installer only needs
# the standalone + core (already built). If appimagetool is unavailable or its
# build fails, skip the AppImage rather than failing the whole release.
_imglog="$(mktemp)"
if [[ -n "$TOOL" && -x "$TOOL" ]] && APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" "$APPDIR" "$APPIMG" >"$_imglog" 2>&1; then
	chmod +x "$APPIMG"
	echo "    -> ${APPIMG}  ($(du -h "$APPIMG" | cut -f1))"
else
	echo "    warning: appimagetool unavailable or AppImage build failed — skipping" >&2
	[[ -s "$_imglog" ]] && { echo "    --- appimagetool output: ---" >&2; sed 's/^/      /' "$_imglog" >&2; }
	APPIMG=""
fi
rm -f "$_imglog"
rm -rf "$(dirname "$APPDIR")"

echo "[6/6] checksums..."
( cd dist
  sha256sum "$(basename "$STANDALONE")" > "$(basename "$STANDALONE")".sha256
  sha256sum "$(basename "$CORE_ART")"    > "$(basename "$CORE_ART")".sha256
  [[ -n "$APPIMG" ]] && sha256sum "$(basename "$APPIMG")" > "$(basename "$APPIMG")".sha256 || true )

echo "==> ${STANDALONE}        (standalone; run from any dir)"
echo "==> ${STANDALONE}.sha256"
echo "==> ${CORE_ART}   (core binary; for the web service CATCODE_CORE)"
echo "==> ${CORE_ART}.sha256"
if [[ -n "$APPIMG" ]]; then
	echo "==> ${APPIMG}            (AppImage; run from a terminal)"
	echo "==> ${APPIMG}.sha256"
	echo
	echo "Run the AppImage:"
	echo "  chmod +x ${APPIMG##*/}  &&  ./${APPIMG##*/}      # launches the TUI in this CWD"
fi
echo "Run the standalone:"
echo "  chmod +x ${STANDALONE##*/}  &&  ./${STANDALONE##*/}"
echo "First run inside either:  /login   then /model, then type a prompt."
echo "Note: the agent's bash tool needs bash on PATH (present by default)."
echo "      Sandboxing (--sandbox firejail / --no-network) is Linux-only."
