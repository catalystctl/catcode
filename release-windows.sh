#!/usr/bin/env bash
# Cross-compile catcode (the TUI) + catcode-core for Windows x86_64 and package them
# TWO ways:
#
#   1. A per-user MSI that puts `catcode` on the user PATH. The MSI installs
#      catcode.exe + catcode-core.exe into %LOCALAPPDATA%\Programs\catcode with no admin
#      prompt and shows up in Add/Remove Programs; new processes pick `catcode` up
#      from any CWD.
#
#   2. A self-contained standalone catcode-<ver>-windows-x86_64.exe — the Rust core
#      is embedded into the TUI via go:embed (-tags embed_core), so it is ONE
#      file with no install and no separate catcode-core. Run it from any CWD; it
#      extracts its bundled core to %LOCALAPPDATA%\catalyst-code on first run.
#
# Run on Linux: needs the x86_64-pc-windows-gnu Rust target, Go, and msitools
# (wixl). The same packaging/windows/catcode.wxs also compiles with the WiX
# Toolset (candle+light) on a Windows build host.
#
#   ./release-windows.sh [version]      # version defaults to the git commit (short SHA)
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(git rev-parse --short HEAD 2>/dev/null || grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
TARGET="x86_64-pc-windows-gnu"
STAGE="dist/.msi-stage"
MSI="dist/catcode-${VERSION}-windows.msi"
STANDALONE="dist/catcode-${VERSION}-windows-x86_64.exe"
EMBED_FILE="tui/embed/catcode-core"

mkdir -p dist tui/embed
trap 'rm -rf "$STAGE" "$EMBED_FILE"' EXIT

echo "==> building catcode ${VERSION} for Windows (${TARGET}): MSI + standalone .exe"

echo "[1/5] core -> catcode-core.exe (cargo, --locked)..."
cargo build --release --locked --target "$TARGET" --manifest-path core/Cargo.toml
CORE_EXE="core/target/${TARGET}/release/core.exe"
[[ -f "$CORE_EXE" ]] || { echo "error: expected core binary at $CORE_EXE" >&2; exit 1; }

echo "[2/5] tui -> catcode.exe (go, GOOS=windows, reproducible) — MSI two-file layout..."
( cd tui && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath \
		-ldflags "-s -w -X main.coreVersion=${VERSION}" -o catcode.exe . )

echo "[3/5] tui -> standalone catcode.exe (go build -tags embed_core, core embedded)..."
cp "$CORE_EXE" "$EMBED_FILE"
( cd tui && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath -tags embed_core \
		-ldflags "-s -w -X main.coreVersion=${VERSION}" -o "../${STANDALONE}" . )
rm -f "$EMBED_FILE"
echo "    -> ${STANDALONE}  ($(du -h "${STANDALONE}" | cut -f1))"

# Separate core binary (no embed) for the web service's CATCODE_CORE —
# downloadable directly, mirroring the Linux/macOS release assets.
CORE_ART="dist/catcode-core-${VERSION}-windows-x86_64.exe"
cp "$CORE_EXE" "$CORE_ART"
echo "    -> ${CORE_ART}  ($(du -h "$CORE_ART" | cut -f1))"

echo "[4/5] staging + (optional) MSI (wixl)..."
rm -rf "$STAGE"; mkdir -p "$STAGE"
cp "$CORE_EXE"            "$STAGE/catcode-core.exe"
cp tui/catcode.exe           "$STAGE/catcode.exe"
cp packaging/windows/catcode.wxs "$STAGE/catcode.wxs"
# The MSI is an OPTIONAL no-install convenience installer — the installer only
# downloads the standalone .exe + catcode-core.exe (already built in steps 2-3).
# Skip the MSI if wixl (msitools) is unavailable, instead of failing the release.
if command -v wixl >/dev/null 2>&1; then
	( cd "$STAGE" && wixl -D Version="$VERSION" catcode.wxs -o "catcode-${VERSION}-windows.msi" ) \
		&& mv "$STAGE/catcode-${VERSION}-windows.msi" "$MSI" \
		|| { echo "warning: wixl failed — skipping MSI" >&2; MSI=""; }
else
	echo "warning: wixl not found — skipping MSI (apt install msitools to produce it)" >&2
	MSI=""
fi
rm -f tui/catcode.exe

echo "[5/5] zip fallback + checksums..."
# The documented packaging/windows/install.ps1 no-build fallback needs a zip
# of the two exes beside it; previously only the .msi was emitted.
cp packaging/windows/install.ps1 "$STAGE/" 2>/dev/null || true
ZIP="dist/catcode-${VERSION}-windows.zip"
( cd "$STAGE" && zip -j "../$(basename "$ZIP")" catcode.exe catcode-core.exe install.ps1 >/dev/null 2>&1 \
		|| echo "warning: zip unavailable; skipping fallback archive" )
rm -rf "$STAGE"
( cd dist
  [[ -n "$MSI" ]] && sha256sum "$(basename "$MSI")" > "$(basename "$MSI")".sha256 || true
  sha256sum "$(basename "$STANDALONE")" > "$(basename "$STANDALONE")".sha256
  sha256sum "$(basename "$CORE_ART")"    > "$(basename "$CORE_ART")".sha256
  sha256sum "$(basename "$ZIP")"        > "$(basename "$ZIP")".sha256 2>/dev/null || true )

if [[ -n "$MSI" ]]; then
	echo "==> ${MSI}            (installer; ($(du -h "$MSI" | cut -f1)))"
	echo "==> ${MSI}.sha256"
fi
echo "==> ${STANDALONE}      (standalone; ($(du -h "$STANDALONE" | cut -f1)))"
echo "==> ${STANDALONE}.sha256"
echo "==> ${CORE_ART}  (core binary; web service CATCODE_CORE)"
echo "==> ${CORE_ART}.sha256"
echo "==> ${ZIP}             (no-build fallback)"
# P1-23: Authenticode-sign the MSI/exe in CI (osslsigncode/signtool + a
# code-signing cert) to avoid SmartScreen warnings. Not done here (no cert).
echo
echo "Install the MSI:   msiexec /i $(basename "$MSI")            (or double-click; no admin needed)"
echo "Silent:            msiexec /i $(basename "$MSI") /quiet"
echo "Run the standalone: .\\$(basename "$STANDALONE")            (no install; any CWD)"
echo "First run inside catcode:  /key sk-...   then pick a model with /model"
echo "Tip: the agent's bash tool needs bash on PATH (Git Bash or WSL)."
