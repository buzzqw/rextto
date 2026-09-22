#!/usr/bin/env bash
# Collaudo isolato (vedi docs/ACCEPTANCE.md). Non tocca i dati, le porte o i
# servizi di produzione: usa una data-dir temporanea e porte dedicate in
# dry-run. Eseguibile senza root.
#
# Uso:
#   scripts/acceptance.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PORT="${ACCEPTANCE_PORT:-5180}"
ENGINE_PORT="${ACCEPTANCE_ENGINE_PORT:-8980}"
DATA="$(mktemp -d /tmp/rextto-acceptance.XXXXXX)"
BIN="$ROOT/target/release/rexttod"
LOG="$DATA/acceptance.log"
pid=""

cleanup() {
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    rm -rf "$DATA"
}
trap cleanup EXIT

echo "== 1/5 test di regressione =="
cargo test --all-targets --quiet

echo "== 2/5 build release =="
if [ ! -x "$BIN" ]; then
    cargo build --release
fi

echo "== 3/5 avvio isolato in dry-run (porta $PORT) =="
REXTTO_DATA_DIR="$DATA" \
REXTTO_LISTEN="127.0.0.1:$PORT" \
REXTTO_ENGINE_LISTEN="127.0.0.1:$ENGINE_PORT" \
REXTTO_DRY_RUN=1 \
REXTTO_ACTIVE=0 \
REXTTO_LIBTORRENT=0 \
RUST_LOG=rextto=info \
    "$BIN" >"$LOG" 2>&1 &
pid=$!

for _ in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

echo "== 4/5 verifica API di base =="
for endpoint in /api/health /api/status /api/config /api/sources/health /api/services; do
    code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT$endpoint" || true)"
    printf '  %-24s %s\n' "$endpoint" "$code"
done

echo "== 5/5 esito =="
curl -s "http://127.0.0.1:$PORT/api/health" | head -c 400 || true
echo
grep -iE "error" "$LOG" | tail -5 || true
echo "Collaudo isolato completato. Nessuna modifica a dati/servizi reali."
