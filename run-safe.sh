#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

export REXTTO_DATA_DIR="${REXTTO_DATA_DIR:-$ROOT/data}"
export REXTTO_LISTEN="${REXTTO_LISTEN:-127.0.0.1:5000}"
export REXTTO_ENGINE_LISTEN="${REXTTO_ENGINE_LISTEN:-127.0.0.1:8889}"
export REXTTO_IMPORT_SOURCE="${REXTTO_IMPORT_SOURCE:-$ROOT/import-source}"
export REXTTO_ACTIVE=0
export RUST_LOG="${RUST_LOG:-rextto=info}"

exec cargo run --release -- --dry-run
