#!/usr/bin/env bash
# Build a READY-TO-RUN web frontend bundle for the Catalyst Code.
#
#   dist/catcode-web-<ver>.tar.gz   (+ .sha256)
#
# The tarball contains Next.js's "standalone" output: a server.js + the
# minimal node_modules it needs + the static/ and public/ assets. It runs
# under any Node >= 18 (or Bun) with NO `next build` on the host:
#
#     tar xzf catcode-web-<ver>.tar.gz
#     PORT=49283 HOSTNAME=0.0.0.0 CATCODE_CORE=/usr/local/bin/catcode-core \
#       node server.js
#
# ONE tarball serves every platform (Linux/macOS/Windows) — it is pure JS.
# Requires Bun (https://bun.sh) or Node+npm to BUILD (on the release host),
# and Node or Bun to RUN (on the install host). `install.sh --with-web` /
# `install-web.ps1` download this tarball instead of building.
#
#   ./release-web.sh [version]     # version defaults to the git commit (short SHA)
set -euo pipefail
cd "$(dirname "$0")"

VERSION="${1:-$(git rev-parse --short HEAD 2>/dev/null || grep -m1 '^version' core/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
OUT="dist/catcode-web-${VERSION}.tar.gz"

# --- runtime (bun preferred, npm fallback) ----------------------------------
RT="" RT_BIN=""
if command -v bun >/dev/null 2>&1; then RT="bun"; RT_BIN="$(command -v bun)"
elif command -v npm >/dev/null 2>&1; then RT="npm"; RT_BIN="$(command -v npm)"
else echo "error: need bun or npm to build the web bundle" >&2; exit 1; fi
echo "==> building catalyst-code web ${VERSION} (runtime: ${RT})"

# --- 1. SDK -----------------------------------------------------------------
echo "[1/5] sdk: install deps (${RT})..."
( cd sdk && $RT install )
echo "[2/5] sdk: build (tsc -> sdk/dist/)..."
( cd sdk && $RT run build )

# --- 2. web (next build, standalone output) ---------------------------------
echo "[3/5] web: install deps (${RT})..."
( cd web && $RT install )
echo "[4/5] web: next build (output: standalone)..."
( cd web && NEXT_TELEMETRY_DISABLED=1 $RT run build )

# --- 3. assemble the standalone bundle -------------------------------------
# Next standalone puts server.js + minimal node_modules at .next/standalone/,
# but the static assets (CSS/JS chunks, fonts) live at .next/static/ and the
# public/ dir at the repo root. The standalone server expects them relocated
# INTO .next/standalone/.next/static and .next/standalone/public.
echo "[5/5] assembling ${OUT}..."
STAGE="dist/.web-stage-${VERSION}"
rm -rf "$STAGE"; mkdir -p "$STAGE"

cp -a "web/.next/standalone/." "$STAGE/"
mkdir -p "$STAGE/.next/static"
cp -a "web/.next/static/." "$STAGE/.next/static/"
# public/ is optional (empty in some setups); copy if present.
if [[ -d web/public ]]; then
  mkdir -p "$STAGE/public"
  cp -a "web/public/." "$STAGE/public/"
fi

# Sanity: the entrypoint must exist.
[[ -f "$STAGE/server.js" ]] || { echo "error: $STAGE/server.js missing — standalone build failed?" >&2; exit 1; }

# A tiny runner that reads HOSTNAME (default 0.0.0.0) + PORT (default 49283)
# and execs the standalone server, so the service unit can stay simple. Named
# start.js so it is obvious it is the process entrypoint.
cat >"$STAGE/start.js" <<'EOF'
// Entry point for the prebuilt Catalyst Code web bundle.
// Env: PORT (default 49283), HOSTNAME (default 0.0.0.0).
process.env.PORT = process.env.PORT || "49283";
process.env.HOSTNAME = process.env.HOSTNAME || "0.0.0.0";
// next standalone server reads HOSTNAME/PORT from env.
import("./server.js");
EOF

# Tar from INSIDE the stage so the archive root is clean (server.js at top).
( cd "$STAGE" && tar czf "../$(basename "$OUT")" . )
rm -rf "$STAGE"

# --- 4. checksum ------------------------------------------------------------
( cd dist && sha256sum "$(basename "$OUT")" > "$(basename "$OUT")".sha256 )

echo "==> ${OUT}  ($(du -h "$OUT" | cut -f1))"
echo "==> ${OUT}.sha256"
echo
echo "Run it:"
echo "  tar xzf $(basename "$OUT")"
echo "  PORT=49283 HOSTNAME=0.0.0.0 CATCODE_CORE=/usr/local/bin/catcode-core node start.js"
echo "Cross-platform (pure JS) — runs on Linux, macOS, Windows (under Node)."
