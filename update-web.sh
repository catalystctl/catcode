#!/usr/bin/env bash
# Update, validate, build, and restart the in-repo Catalyst Code web service.
# Deploys the hub terminal workspace (primary UI at / and alias /hub).
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SERVICE="${CATALYST_WEB_SERVICE:-catalyst-code-web.service}"
# Env override → PATH → common bun install location (not a machine-specific absolute).
BUN="${BUN:-$(command -v bun 2>/dev/null || true)}"
if [[ -z "$BUN" && -x "${HOME}/.bun/bin/bun" ]]; then
  BUN="${HOME}/.bun/bin/bun"
fi
NODE="${NODE:-$(command -v node 2>/dev/null || true)}"
PUBLIC_ORIGIN="${CATCODE_WEB_ORIGIN:-https://cc.karutoil.site}"
LOCAL_BASE="${AUDIT_BASE:-http://127.0.0.1:49283}"
PULL=1
RUN_TESTS=1
HUB_E2E=0

usage() {
  cat <<'EOF'
Usage: ./update-web.sh [--no-pull] [--skip-tests] [--hub-e2e]

  --no-pull     Deploy the current checkout without fetching/pulling Git.
  --skip-tests  Skip type checks and web tests (build checks still run).
  --hub-e2e     After the service is healthy, run the authenticated /hub
                browser regression (web/scripts/hub-regression.mjs) against
                the deployed service. Needs AUDIT_EMAIL/AUDIT_PASSWORD in the
                environment or web/.env.local, and puppeteer's browser.

Local changes are never overwritten. If tracked files have local changes, the
script skips the pull automatically and deploys the current checkout.
EOF
}

while (($#)); do
  case "$1" in
    --no-pull) PULL=0 ;;
    --skip-tests) RUN_TESTS=0 ;;
    --hub-e2e) HUB_E2E=1 ;;
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
  # Drop generated App Router types from a previous deploy. After route removals
  # (e.g. hub-only cleanup) stale .next/types still import deleted route modules
  # and fail `tsc --noEmit` even when sources are clean. Next regenerates them
  # on the build step below; the live .next tree is backed up just before that.
  rm -rf web/.next/types
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
# systemd 203/EXEC if the launcher lost +x (git checkout / copy without mode bits).
if [[ -f scripts/run-web.sh ]]; then
  chmod +x scripts/run-web.sh
fi
"${SYSTEMCTL[@]}" stop "$SERVICE"
SERVICE_STOPPED=1
"${SYSTEMCTL[@]}" start "$SERVICE"

health_ok() {
  "${SYSTEMCTL[@]}" is-active --quiet "$SERVICE" || return 1
  # Hub is auth-gated: unauthenticated it redirects (307/308) to /login.
  # Either the shell (200) or the redirect proves the route built. Check both
  # / (primary) and /hub (alias) so a broken root entry fails health.
  local root_code hub_code
  root_code="$(curl --silent --output /dev/null --write-out '%{http_code}' "$LOCAL_BASE/" || true)"
  hub_code="$(curl --silent --output /dev/null --write-out '%{http_code}' "$LOCAL_BASE/hub" || true)"
  [[ "$root_code" == 200 || "$root_code" == 307 || "$root_code" == 308 ]] || return 1
  [[ "$hub_code" == 200 || "$hub_code" == 307 || "$hub_code" == 308 ]]
}

for attempt in {1..20}; do
  if health_ok; then
    SERVICE_STOPPED=0
    if [[ -n "$BACKUP" && -d "$BACKUP" ]]; then
      rm -rf "$BACKUP"
    fi
    trap - EXIT
    if ((HUB_E2E)); then
      echo "==> Running hub browser regression against $LOCAL_BASE"
      if [[ -z "${AUDIT_EMAIL:-}" && ! -f web/.env.local ]]; then
        echo "--hub-e2e: no AUDIT_EMAIL/AUDIT_PASSWORD env and no web/.env.local; skipping" >&2
      else
        (cd web && AUDIT_BASE="$LOCAL_BASE" "$NODE" scripts/hub-regression.mjs)
      fi
    fi
    echo "==> Update complete: $LOCAL_BASE (hub: $LOCAL_BASE/hub)"
    exit 0
  fi
  sleep 1
done

"${SYSTEMCTL[@]}" status "$SERVICE" --no-pager -l || true
echo "Service did not become healthy within 20 seconds" >&2
exit 1
