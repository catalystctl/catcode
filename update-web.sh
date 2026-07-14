#!/usr/bin/env bash
# Update, validate, build, and restart the in-repo Catalyst Code web service.
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SERVICE="${CATALYST_WEB_SERVICE:-catalyst-code-web.service}"
BUN="${BUN:-/home/karutoil/.bun/bin/bun}"
NODE="${NODE:-/home/karutoil/.nvm/versions/node/v22.22.2/bin/node}"
PUBLIC_ORIGIN="${CATCODE_WEB_ORIGIN:-https://cc.karutoil.site}"
PULL=1
RUN_TESTS=1

usage() {
  cat <<'EOF'
Usage: ./update-web.sh [--no-pull] [--skip-tests]

  --no-pull     Deploy the current checkout without fetching/pulling Git.
  --skip-tests  Skip type checks and web tests (build checks still run).

Local changes are never overwritten. If tracked files have local changes, the
script skips the pull automatically and deploys the current checkout.
EOF
}

while (($#)); do
  case "$1" in
    --no-pull) PULL=0 ;;
    --skip-tests) RUN_TESTS=0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

for command in git cargo curl; do
  command -v "$command" >/dev/null || { echo "Missing required command: $command" >&2; exit 1; }
done
[[ -x "$BUN" ]] || { echo "Bun not found: $BUN" >&2; exit 1; }
[[ -x "$NODE" ]] || { echo "Node not found: $NODE" >&2; exit 1; }

if ((EUID == 0)); then
  SYSTEMCTL=(systemctl)
else
  SYSTEMCTL=(sudo systemctl)
fi

cd "$REPO"

if ((PULL)); then
  if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "==> Tracked local changes detected; skipping pull and deploying the current checkout"
  else
    echo "==> Updating the checkout"
    git pull --ff-only
  fi
fi

echo "==> Installing dependencies and building the SDK"
(cd sdk && "$BUN" install --frozen-lockfile && "$BUN" run build)
(cd web && "$BUN" install --frozen-lockfile)

if ((RUN_TESTS)); then
  echo "==> Validating web sources"
  (cd web && "$BUN" run typecheck && "$BUN" test)
fi

echo "==> Building the release core"
(cd core && cargo build --release --locked)

# Preserve the currently served tree while Next creates a fresh .next folder.
# A failed build can therefore roll back without leaving the service broken.
BACKUP=""
SERVICE_STOPPED=0
rollback() {
  local status=$?
  if ((status != 0)); then
    echo "Update failed; restoring the previous web build" >&2
    if [[ -n "$BACKUP" && -d "$BACKUP" ]]; then
      rm -rf "$REPO/web/.next"
      mv "$BACKUP" "$REPO/web/.next"
    fi
    if ((SERVICE_STOPPED)); then
      "${SYSTEMCTL[@]}" restart "$SERVICE" || true
    fi
  fi
  exit "$status"
}
trap rollback EXIT

if [[ -d web/.next ]]; then
  # Keep this outside outputFileTracingRoot (the repository); otherwise Next
  # may discover and attempt to trace files from the backup during its build.
  BACKUP="${TMPDIR:-/tmp}/catalyst-code-web-next-backup.$$"
  mv web/.next "$BACKUP"
fi

echo "==> Building the standalone web bundle with Node"
(cd web && CATCODE_WEB_ORIGIN="$PUBLIC_ORIGIN" "$NODE" node_modules/next/dist/bin/next build)
[[ -f web/.next/standalone/web/server.js || -f web/.next/standalone/server.js ]] || {
  echo "Next build completed without a standalone server bundle" >&2
  exit 1
}

echo "==> Restarting $SERVICE"
"${SYSTEMCTL[@]}" stop "$SERVICE"
SERVICE_STOPPED=1
"${SYSTEMCTL[@]}" start "$SERVICE"

for attempt in {1..20}; do
  if "${SYSTEMCTL[@]}" is-active --quiet "$SERVICE" && curl --silent --show-error --fail --output /dev/null http://127.0.0.1:49283/; then
    SERVICE_STOPPED=0
    if [[ -n "$BACKUP" && -d "$BACKUP" ]]; then
      rm -rf "$BACKUP"
    fi
    trap - EXIT
    echo "==> Update complete: http://127.0.0.1:49283"
    exit 0
  fi
  sleep 1
done

"${SYSTEMCTL[@]}" status "$SERVICE" --no-pager -l || true
echo "Service did not become healthy within 20 seconds" >&2
exit 1
