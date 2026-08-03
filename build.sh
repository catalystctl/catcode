#!/usr/bin/env bash
# ponytail: minimal build, with an optional local TUI launch
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

case "${1:-}" in
  ""|--run)
    ;;
  --help|-h)
    printf 'usage: %s [--run [TUI_ARGS...]]\n' "$(basename "$0")"
    printf '\nBuilds the release core and development TUI. --run starts the TUI\n'
    printf 'with that exact core, even when CATCODE_CORE points at an installed binary.\n'
    exit 0
    ;;
  *)
    printf 'error: unknown option %s\n' "$1" >&2
    printf 'usage: %s [--run [TUI_ARGS...]]\n' "$(basename "$0")" >&2
    exit 2
    ;;
esac

echo "[1/2] building core (cargo)..."
cargo build --release --manifest-path core/Cargo.toml

echo "[2/2] building tui (go)..."
( cd tui && go build -o tui . )

echo "done: core -> core/target/release/core, tui -> tui/tui"

LOCAL_CORE="$ROOT_DIR/core/target/release/core"
if [[ -n "${CATCODE_CORE:-}" && "$CATCODE_CORE" != "$LOCAL_CORE" ]]; then
  echo "warning: CATCODE_CORE=$CATCODE_CORE overrides this source build"
  echo "         run locally with: CATCODE_CORE=$LOCAL_CORE $ROOT_DIR/tui/tui"
fi

if [[ "${1:-}" == "--run" ]]; then
  shift
  echo "starting local TUI (core=$LOCAL_CORE)"
  exec env CATCODE_CORE="$LOCAL_CORE" "$ROOT_DIR/tui/tui" "$@"
fi
