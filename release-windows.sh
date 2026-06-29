#!/usr/bin/env bash
# Cross-compile ucli (the TUI) + umans-core for Windows x86_64 and package them
# into a per-user MSI that puts `ucli` on the user PATH. The MSI installs to
# %LOCALAPPDATA%\Programs\ucli with no admin prompt and shows up in Add/Remove
# Programs; new processes pick `ucli` up from any CWD.
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

echo "==> building ucli ${VERSION} for Windows (${TARGET}) as an MSI"

echo "[1/4] core -> umans-core.exe (cargo)..."
cargo build --release --target "$TARGET" --manifest-path core/Cargo.toml

echo "[2/4] tui -> ucli.exe (go, GOOS=windows)..."
( cd tui && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -o ucli.exe . )

echo "[3/4] staging + building MSI (wixl)..."
rm -rf "$STAGE"; mkdir -p "$STAGE"
cp "core/target/${TARGET}/release/core.exe" "$STAGE/umans-core.exe"
cp tui/ucli.exe                       "$STAGE/ucli.exe"
cp packaging/windows/ucli.wxs         "$STAGE/ucli.wxs"
( cd "$STAGE" && wixl -D Version="$VERSION" ucli.wxs -o "ucli-${VERSION}-windows.msi" )
mv "$STAGE/ucli-${VERSION}-windows.msi" "$MSI"
rm -rf "$STAGE"   # drop intermediate stage; the MSI is self-contained

echo "[4/4] checksum..."
( cd dist && sha256sum "$(basename "$MSI")" > "$(basename "$MSI").sha256" )

echo "==> ${MSI}  ($(du -h "$MSI" | cut -f1))"
echo "==> ${MSI}.sha256"
echo "Install on Windows:  msiexec /i $(basename "$MSI")    (or double-click; no admin needed)"
echo "Silent:              msiexec /i $(basename "$MSI") /quiet"
