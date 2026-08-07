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
    printf '\nBuilds the release core and development TUI, then replaces the current\n'
    printf 'catcode installation when it is available on PATH. --run starts the TUI\n'
    printf 'with that exact core, even when CATCODE_CORE points at an installed binary.\n'
    exit 0
    ;;
  *)
    printf 'error: unknown option %s\n' "$1" >&2
    printf 'usage: %s [--run [TUI_ARGS...]]\n' "$(basename "$0")" >&2
    exit 2
    ;;
esac

echo "[1/3] building core (cargo, native-browser, -j$(nproc))..."
cargo build --release -j"$(nproc)" --features native-browser --manifest-path core/Cargo.toml

echo "[2/3] building tui (go)..."
( cd tui && go build -o tui . )

LOCAL_CORE="$ROOT_DIR/core/target/release/core"
INSTALLED_TUI="$(type -P catcode || true)"

# Replace the binaries used by the current installation, rather than leaving
# the freshly-built artifacts stranded in the repository. The TUI and core
# must be updated together: a source-built TUI can speak protocol changes that
# an older installed core may not understand.
install_binary() {
  local source="$1" destination="$2"
  local destination_dir
  destination_dir="$(dirname "$destination")"
  if [[ $EUID -eq 0 || -w "$destination_dir" ]]; then
    install -m 0755 "$source" "$destination"
  elif command -v sudo >/dev/null 2>&1; then
    sudo install -m 0755 "$source" "$destination"
  else
    echo "error: cannot replace $destination (directory is not writable and sudo is unavailable)" >&2
    return 1
  fi
}

if [[ -n "$INSTALLED_TUI" ]]; then
  echo "[3/3] replacing installed catcode"
  INSTALLED_DIR="$(dirname "$INSTALLED_TUI")"
  case "$INSTALLED_TUI" in
    *.exe|*.EXE) INSTALLED_CORE="$INSTALLED_DIR/catcode-core.exe" ;;
    *)           INSTALLED_CORE="$INSTALLED_DIR/catcode-core"     ;;
  esac
  echo "installing tui -> $INSTALLED_TUI"
  install_binary "$ROOT_DIR/tui/tui" "$INSTALLED_TUI"
  echo "installing core -> $INSTALLED_CORE"
  install_binary "$LOCAL_CORE" "$INSTALLED_CORE"
else
  echo "[3/3] installed catcode not found on PATH; skipping installation"
fi

echo "done: core -> core/target/release/core, tui -> tui/tui"

if [[ -n "${CATCODE_CORE:-}" && "$CATCODE_CORE" != "$LOCAL_CORE" ]]; then
  echo "warning: CATCODE_CORE=$CATCODE_CORE overrides this source build"
  echo "         run locally with: CATCODE_CORE=$LOCAL_CORE $ROOT_DIR/tui/tui"
fi

if [[ "${1:-}" == "--run" ]]; then
  shift
  echo "starting local TUI (core=$LOCAL_CORE)"
  exec env CATCODE_CORE="$LOCAL_CORE" "$ROOT_DIR/tui/tui" "$@"
fi
