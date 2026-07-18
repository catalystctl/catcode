#!/usr/bin/env bash
# Build a READY-TO-RUN, cross-platform web frontend bundle for the Catalyst Code.
#
#   dist/catcode-web-<ver>.tar.gz   (+ .sha256)
#
# The tarball contains Next.js's "standalone" output: a server.js + the
# minimal node_modules it needs + the static/ and public/ assets. It runs
# under Node >= 22.13.0 (or Bun) with NO `next build` on the host:
#
#     tar xzf catcode-web-<ver>.tar.gz
#     PORT=49283 HOSTNAME=0.0.0.0 CATCODE_CORE=/usr/local/bin/catcode-core \
#       node start.js
#
# One tarball for Linux/macOS/Windows. The terminal uses zigpty, which ships
# all platform PTY prebuilds (~450KB) inside a single package — no per-OS web
# builds, no node-gyp, no Windows release host. Auth uses node:sqlite (no
# better-sqlite3). sharp / Next SWC are pruned (build-only / unused).
# Requires Bun (https://bun.sh) or Node+npm to BUILD (on the release host),
# and Node or Bun to RUN (on the install host). `install.sh --with-web` /
# `install.ps1 -WithWeb` download this tarball instead of building.
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
command -v node >/dev/null 2>&1 || {
  echo "error: need Node.js to build the Next.js web bundle" >&2
  exit 1
}
echo "==> building catalyst-code web ${VERSION} (runtime: ${RT})"

install_deps() {
  local dir="$1"
  if [[ "$RT" == bun ]]; then
    ( cd "$dir" && "$RT_BIN" install --frozen-lockfile )
  elif [[ -f "$dir/package-lock.json" ]]; then
    ( cd "$dir" && "$RT_BIN" ci )
  else
    ( cd "$dir" && "$RT_BIN" install )
  fi
}

# --- 1. SDK -----------------------------------------------------------------
echo "[1/5] sdk: install deps (${RT})..."
install_deps sdk
echo "[2/5] sdk: build (tsc -> sdk/dist/)..."
( cd sdk && $RT run build )

# --- 2. web (next build, standalone output) ---------------------------------
echo "[3/5] web: install deps (${RT})..."
install_deps web
echo "[4/5] web: next build (output: standalone)..."
( cd web && NEXT_TELEMETRY_DISABLED=1 node node_modules/next/dist/bin/next build )

# --- 2b. bundle the custom Next server (WS terminal at /api/terminal) --------
# Next app-router route handlers cannot upgrade to WebSocket, so the custom
# server (web/src/server/server.ts) wraps next() and attaches a ws.WebSocketServer
# at /api/terminal on the SAME port. Bundle its JavaScript with esbuild
# (Node-compatible — works whether the release host built with bun or npm) and
# ship it IN PLACE of Next's standalone server.js, so the release serves HTTP
# plus the terminal WebSocket on a single port. (Contract §7.4.)
# Keep zigpty external and copy its package (all OS prebuilds) into the artifact.
echo "[4b/5] web: bundle custom server (esbuild) -> .server-bundle.js..."
( cd web && ./node_modules/.bin/esbuild src/server/server.ts \
    --bundle --platform=node --format=esm \
    --outfile=.server-bundle.js \
    --external:next --external:ws --external:zigpty --external:kysely \
    --external:better-auth --external:@better-auth/passkey \
    --external:@catalyst-code/coding-agent )

# --- 3. assemble the standalone bundle -------------------------------------
# Next standalone puts server.js + minimal node_modules at .next/standalone/,
# but the static assets (CSS/JS chunks, fonts) live at .next/static/ and the
# public/ dir at the repo root. The standalone server expects them relocated
# INTO .next/standalone/.next/static and .next/standalone/public.
echo "[5/5] assembling ${OUT}..."
STAGE="dist/.web-stage-${VERSION}"
rm -rf "$STAGE"; mkdir -p "$STAGE"

# With outputFileTracingRoot set to the monorepo root, Next nests the app under
# standalone/web/. Older/single-package builds put it directly at standalone/.
# In both cases flatten the actual app root so server.js, .next, node_modules,
# and package.json share the directory that `node start.js` runs from.
if [[ -f web/.next/standalone/web/server.js ]]; then
  cp -a "web/.next/standalone/web/." "$STAGE/"
else
  cp -a "web/.next/standalone/." "$STAGE/"
fi
mkdir -p "$STAGE/.next/static"
cp -a "web/.next/static/." "$STAGE/.next/static/"
# public/ is optional (empty in some setups); copy if present.
if [[ -d web/public ]]; then
  mkdir -p "$STAGE/public"
  cp -a "web/public/." "$STAGE/public/"
fi

# Replace Next's standalone server.js with our CUSTOM server: same next() HTTP
# handling PLUS the /api/terminal WebSocket. Pure-JS, single file.
cp -f "web/.server-bundle.js" "$STAGE/server.js"
rm -f "web/.server-bundle.js"
# The custom server is built outside Next's route graph, so packages marked
# external above are not reliably included by standalone tracing. Copy each
# external package and its complete runtime dependency closure. This also
# dereferences the workspace SDK symlink; an absolute monorepo symlink would be
# broken once the tarball is installed elsewhere.
COPIED_RUNTIME_PACKAGES=()
copy_runtime_package() {
  local pkg="$1" src dest dep copied
  for copied in "${COPIED_RUNTIME_PACKAGES[@]}"; do
    [[ "$copied" == "$pkg" ]] && return
  done
  COPIED_RUNTIME_PACKAGES+=("$pkg")
  src="web/node_modules/$pkg"
  [[ -f "$src/package.json" ]] || {
    echo "error: runtime package $pkg is missing from web/node_modules" >&2
    exit 1
  }
  dest="$STAGE/node_modules/$pkg"
  rm -rf "$dest"
  mkdir -p "$dest"
  cp -aL "$src/." "$dest/"

  while IFS= read -r dep; do
    if [[ -n "$dep" && -f "web/node_modules/$dep/package.json" ]]; then
      copy_runtime_package "$dep"
    fi
  done < <(node -e '
    const fs = require("node:fs");
    const pkg = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const deps = { ...pkg.dependencies, ...pkg.optionalDependencies };
    process.stdout.write(Object.keys(deps).join("\n") + "\n");
  ' "$src/package.json")
}

for pkg in \
  next ws zigpty kysely \
  better-auth @better-auth/passkey @better-auth/kysely-adapter @catalyst-code/coding-agent; do
  copy_runtime_package "$pkg"
done

# SWC compiles/transforms application code during `next build`. This bundle
# contains the completed standalone build, so the production server does not
# invoke the compiler. Keeping the 100+ MB native addon only duplicates a
# build-time tool in every desktop install.
rm -rf "$STAGE/node_modules/@next"/swc-*

# sharp is an optional Next.js dependency for `next/image` optimization. We
# serve plain <img> / API URLs and do not use the image optimizer, so the
# platform .node + libvips (~15–20 MB) is dead weight in every install.
# Auth already uses node:sqlite (no better-sqlite3). The only intentional
# native runtime binaries are zigpty's multi-platform prebuilds.
rm -rf "$STAGE/node_modules/sharp" "$STAGE/node_modules/@img"

# Source maps are useful while developing but are not loaded by the production
# server. Next and its compiled dependencies account for most of these files.
find "$STAGE" -type f -name '*.map' -delete

# Next can omit the root package.json when outputFileTracingRoot points above
# web/. Keep an explicit ESM runtime manifest for `node start.js`.
node -e '
  const fs = require("node:fs");
  const stage = process.argv[1];
  const versionOf = (name) => JSON.parse(
    fs.readFileSync(`web/node_modules/${name}/package.json`, "utf8")
  ).version;
  fs.writeFileSync(`${stage}/package.json`, JSON.stringify({
    name: "catcode-web-runtime",
    version: "0.0.0",
    private: true,
    type: "module",
    dependencies: {
      zigpty: versionOf("zigpty")
    }
  }, null, 2) + "\n");
' "$STAGE"

# Sanity: the entrypoint must exist.
[[ -f "$STAGE/server.js" ]] || { echo "error: $STAGE/server.js missing — standalone build failed?" >&2; exit 1; }

# A tiny runner that reads HOSTNAME (default 0.0.0.0) + PORT (default 49283)
# and execs the standalone server, so the service unit can stay simple. Named
# start.js so it is obvious it is the process entrypoint.
cat >"$STAGE/start.js" <<'EOF'
// Entry point for the prebuilt Catalyst Code web bundle.
// Env: PORT (default 49283), HOSTNAME (default 0.0.0.0), NODE_ENV (default production).
process.env.PORT = process.env.PORT || "49283";
process.env.HOSTNAME = process.env.HOSTNAME || "0.0.0.0";
// The custom server (server.js) reads NODE_ENV to pick dev vs prod serving;
// default to production so `node start.js` serves the prebuilt .next.
process.env.NODE_ENV = process.env.NODE_ENV || "production";
import("./server.js");
EOF

# Embed the release git commit so the web UI can show version / update status
# without needing a .git checkout on the install host.
COMMIT_FULL="$(git rev-parse HEAD 2>/dev/null || true)"
COMMIT_SHORT="${VERSION}"
if [[ -n "$COMMIT_FULL" && "$COMMIT_FULL" == "${VERSION}"* ]]; then
  : # VERSION already matches this commit's short SHA
elif [[ -n "$COMMIT_FULL" ]]; then
  COMMIT_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo "$VERSION")"
fi
DIRTY=false
if [[ -n "$(git status --porcelain 2>/dev/null || true)" ]]; then DIRTY=true; fi
cat >"$STAGE/version.json" <<EOF
{
  "commit": "${COMMIT_SHORT}",
  "commitFull": "${COMMIT_FULL:-$COMMIT_SHORT}",
  "dirty": ${DIRTY},
  "builtAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "source": "release"
}
EOF
# Keep a copy under .next as well (API also probes there for source installs).
mkdir -p "$STAGE/.next"
cp -f "$STAGE/version.json" "$STAGE/.next/version.json"

# Fail closed if the monorepo flatten left a nested app root — installers run
# `node start.js` from the archive root and resolve packages from ./node_modules.
if [[ -f "$STAGE/web/server.js" || -d "$STAGE/web/node_modules" ]]; then
  echo "error: staged bundle still has nested web/ — flatten failed (standalone layout changed?)" >&2
  exit 1
fi
[[ -f "$STAGE/package.json" ]] || { echo "error: staged bundle missing package.json" >&2; exit 1; }
[[ -f "$STAGE/.next/BUILD_ID" ]] || { echo "error: staged bundle missing .next/BUILD_ID" >&2; exit 1; }
[[ -f "$STAGE/version.json" ]] || { echo "error: staged bundle missing version.json" >&2; exit 1; }
for req in next ws zigpty kysely better-auth @better-auth/passkey @better-auth/kysely-adapter @catalyst-code/coding-agent; do
  [[ -f "$STAGE/node_modules/$req/package.json" ]] || {
    echo "error: staged bundle missing node_modules/$req (custom server cannot start)" >&2
    exit 1
  }
done
# Fail closed: only zigpty's multi-platform prebuilds may ship .node files
# (no sharp/swc/better-sqlite3 / per-OS optional packages).
pty_node_found=0
while IFS= read -r -d '' node_bin; do
  case "$node_bin" in
    "$STAGE"/node_modules/zigpty/prebuilds/*.node)
      pty_node_found=1
      ;;
    *)
      echo "error: unexpected native addon in web bundle: ${node_bin#"$STAGE"/}" >&2
      echo "  Only zigpty/prebuilds/*.node are allowed (no sharp/swc/better-sqlite3)." >&2
      exit 1
      ;;
  esac
done < <(find "$STAGE" -type f -name '*.node' -print0)
if [[ "$pty_node_found" -eq 0 ]]; then
  echo "error: staged bundle missing zigpty prebuilds (terminal will not work)" >&2
  exit 1
fi
# Resolve the custom server's critical imports the same way install hosts will.
(
  cd "$STAGE"
  node --input-type=module -e 'await Promise.all(["next","ws","zigpty","better-auth","@catalyst-code/coding-agent"].map((m)=>import(m))); console.log("runtime imports OK")'
) || { echo "error: staged bundle failed module resolution smoke test" >&2; exit 1; }

echo "==> version.json commit=${COMMIT_SHORT} dirty=${DIRTY}"

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
echo "Serves HTTP + a persistent, real-PTY /api/terminal WebSocket on one port."
