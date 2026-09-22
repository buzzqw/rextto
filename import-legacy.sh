#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE="${1:-}"
DATA_DIR="${2:-$ROOT/data}"

cd "$ROOT"
exec cargo run --release -- import --from-copy "$SOURCE" --data-dir "$DATA_DIR"
