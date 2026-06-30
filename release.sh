#!/usr/bin/env bash
# ponytail: minimal release script. Builds both binaries, packs them with the
# README into a versioned tarball, and writes a checksum. No goreleaser, no
# cross-compile matrix — add targets when you actually ship for another arch.
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    VERSION=$(grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
fi
echo "==> building umans-harness ${VERSION}"

echo "[1/4] building core (cargo --release, --locked for reproducibility)..."
cargo build --release --locked --manifest-path core/Cargo.toml

echo "[2/4] building tui (go, reproducible -trimpath + version)..."
( cd tui && CGO_ENABLED=0 go build -trimpath \
    -ldflags "-s -w -X main/coreVersion=${VERSION}" -o umans-harness-tui . )

echo "[3/4] staging..."
STAGE="umans-harness-${VERSION}"
rm -rf "dist/${STAGE}"
mkdir -p "dist/${STAGE}/bin"
cp core/target/release/core          "dist/${STAGE}/bin/umans-core"
cp tui/umans-harness-tui             "dist/${STAGE}/bin/umans-tui"
cp README.md Dockerfile              "dist/${STAGE}/"
cat > "dist/${STAGE}/INSTALL.md" <<EOF
umans-harness ${VERSION}

Install:
  sudo cp bin/umans-core bin/umans-tui /usr/local/bin/

Run:
  umans-tui            # spawns umans-core from PATH or ../core/target/release
  umans-core --help    # the core speaks stdio JSON; the TUI drives it

First run: /key sk-...  then /model, then type a prompt.
Sandboxing: pass --sandbox firejail --no-network to the core, or set it in
the TUI settings modal.
EOF

echo "[4/4] packing tarball + checksum..."
( cd dist && tar czf "${STAGE}.tar.gz" "${STAGE}" )
( cd dist && sha256sum "${STAGE}.tar.gz" > "${STAGE}.tar.gz.sha256" )

echo "==> dist/${STAGE}.tar.gz  ($(du -h dist/${STAGE}.tar.gz | cut -f1))"
echo "==> dist/${STAGE}.tar.gz.sha256"
