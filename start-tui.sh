#!/usr/bin/env bash
# Avvia la TUI di Rextto (client del demone in esecuzione).
#
# Variabili:
#   REXTTO_URL        base HTTP del demone (default http://127.0.0.1:5000)
#   REXTTO_API_TOKEN  token API se configurato sul demone
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/tui/target/release/rextto-tui"

if [[ ! -x "$BIN" ]] || find "$ROOT/tui/src" "$ROOT/tui/Cargo.toml" -newer "$BIN" -print -quit | grep -q .; then
    cargo build --release --manifest-path "$ROOT/tui/Cargo.toml"
fi

exec "$BIN"
