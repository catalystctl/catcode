#!/usr/bin/env bash
# Install Catalyst Code with the browser frontend in one command:
#   curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
#
# All additional arguments are forwarded to install.sh, for example:
#   ... | bash -s -- --port 8080 --host 127.0.0.1
set -euo pipefail

INSTALLER_URL="${CATCODE_INSTALLER_URL:-https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh}"
SCRIPT_DIR=""

if [[ -n "${BASH_SOURCE[0]:-}" && "${BASH_SOURCE[0]}" != /dev/* ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/install.sh" ]]; then
  exec bash "$SCRIPT_DIR/install.sh" --with-web "$@"
fi

if ! command -v curl >/dev/null 2>&1; then
  printf 'error: curl is required to download the Catalyst Code installer\n' >&2
  exit 1
fi

tmp="$(mktemp -t catalyst-web-installer.XXXXXX)"
trap 'rm -f "$tmp"' EXIT
curl -fsSL "$INSTALLER_URL" -o "$tmp"
bash "$tmp" --with-web "$@"
