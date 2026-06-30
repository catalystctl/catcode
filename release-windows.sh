#!/usr/bin/env bash
# Cross-compile ucli (the TUI) + umans-core for Windows x86_64 and package them
# TWO ways:
#
#   1. A per-user MSI that puts `ucli` on the user PATH. The MSI installs
#      ucli.exe + umans-core.exe into %LOCALAPPDATA%\Programs\ucli with no admin
#      prompt and shows up in Add/Remove Programs; new processes pick `ucli` up
#      from any CWD.
#
#   2. A self-contained standalone ucli-<ver>-windows-x86_64.exe — the Rust core
#      is embedded into the TUI via go:embed (-tags embed_core), so it is ONE
#      file with no install and no separate umans-core. Run it from any CWD; it
#      extracts its bundled core to %LOCALAPPDATA%\umans-harness on first run.
#
# Run on Linux: needs the x86_64-pc-windows-gnu Rust target, Go, and msitools
# (wixl). The same packaging/windows/ucli.wxs also compiles with the WiX
# Toolset (candle+light) on a Windows build host.
#
#   ./release-windows.sh [version]      # version defaults to core/Cargo.toml
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
TARGET="x86_64-pc-windows-gnu"
STAGE="dist/.msi-stage"
MSI="dist/ucli-${VERSION}-windows.msi"
STANDALONE="dist/ucli-${VERSION}-windows-x86_64.exe"
EMBED_FILE="tui/embed/umans-core"

mkdir -p dist tui/embed
trap 'rm -rf "$STAGE" "$EMBED_FILE"' EXIT

echo "==> building ucli ${VERSION} for Windows (${TARGET}): MSI + standalone .exe"

echo "[1/5] core -> umans-core.exe (cargo, --locked)..."
cargo build --release --locked --target "$TARGET" --manifest-path core/Cargo.toml
CORE_EXE="core/target/${TARGET}/release/core.exe"
[[ -f "$CORE_EXE" ]] || { echo "error: expected core binary at $CORE_EXE" >&2; exit 1; }

echo "[2/5] tui -> ucli.exe (go, GOOS=windows, reproducible) — MSI two-file layout..."
( cd tui && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath \
		-ldflags "-s -w -X main.coreVersion=${VERSION}" -o ucli.exe . )

echo "[3/5] tui -> standalone ucli.exe (go build -tags embed_core, core embedded)..."
cp "$CORE_EXE" "$EMBED_FILE"
( cd tui && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath -tags embed_core \
		-ldflags "-s -w -X main.coreVersion=${VERSION}" -o "../${STANDALONE}" . )
rm -f "$EMBED_FILE"
echo "    -> ${STANDALONE}  ($(du -h "${STANDALONE}" | cut -f1))"

echo "[4/5] staging + building MSI (wixl)..."
rm -rf "$STAGE"; mkdir -p "$STAGE"
cp "$CORE_EXE"            "$STAGE/umans-core.exe"
cp tui/ucli.exe           "$STAGE/ucli.exe"
cp packaging/windows/ucli.wxs "$STAGE/ucli.wxs"
( cd "$STAGE" && wixl -D Version="$VERSION" ucli.wxs -o "ucli-${VERSION}-windows.msi" )
mv "$STAGE/ucli-${VERSION}-windows.msi" "$MSI"
rm -f tui/ucli.exe

echo "[5/5] zip fallback + checksums..."
# The documented packaging/windows/install.ps1 no-build fallback needs a zip
# of the two exes beside it; previously only the .msi was emitted.
cp packaging/windows/install.ps1 "$STAGE/" 2>/dev/null || true
ZIP="dist/ucli-${VERSION}-windows.zip"
( cd "$STAGE" && zip -j "../$(basename "$ZIP")" ucli.exe umans-core.exe install.ps1 >/dev/null 2>&1 \
		|| echo "warning: zip unavailable; skipping fallback archive" )
rm -rf "$STAGE"
( cd dist
  sha256sum "$(basename "$MSI")"        > "$(basename "$MSI")".sha256
  sha256sum "$(basename "$STANDALONE")" > "$(basename "$STANDALONE")".sha256
  sha256sum "$(basename "$ZIP")"        > "$(basename "$ZIP")".sha256 2>/dev/null || true )

echo "==> ${MSI}            (installer; ($(du -h "$MSI" | cut -f1)))"
echo "==> ${MSI}.sha256"
echo "==> ${STANDALONE}      (standalone; ($(du -h "$STANDALONE" | cut -f1)))"
echo "==> ${STANDALONE}.sha256"
echo "==> ${ZIP}             (no-build fallback)"
# P1-23: Authenticode-sign the MSI/exe in CI (osslsigncode/signtool + a
# code-signing cert) to avoid SmartScreen warnings. Not done here (no cert).
echo
echo "Install the MSI:   msiexec /i $(basename "$MSI")            (or double-click; no admin needed)"
echo "Silent:            msiexec /i $(basename "$MSI") /quiet"
echo "Run the standalone: .\\$(basename "$STANDALONE")            (no install; any CWD)"
echo "First run inside ucli:  /key sk-...   then pick a model with /model"
echo "Tip: the agent's bash tool needs bash on PATH (Git Bash or WSL)."
