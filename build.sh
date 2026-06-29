#!/usr/bin/env bash
# ponytail: minimal build, no flags beyond what each toolchain defaults to
set -euo pipefail
cd "$(dirname "$0")"

echo "[1/2] building core (cargo)..."
cargo build --release --manifest-path core/Cargo.toml

echo "[2/2] building tui (go)..."
( cd tui && go build -o tui . )

echo "done: core -> core/target/release/core, tui -> tui/tui"
