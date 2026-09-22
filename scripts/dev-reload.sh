#!/usr/bin/env bash
# Ciclo di sviluppo rapido: NON usa il profilo `release` (minuti). Compila il
# daemon col profilo `fast`, aggiorna la UI in modalità dev e riavvia il
# servizio tramite l'endpoint (senza sudo).
#
# Uso:
#   scripts/dev-reload.sh            # daemon + UI + riavvio
#   scripts/dev-reload.sh --daemon   # solo daemon (più veloce)
#   scripts/dev-reload.sh --ui       # solo UI
#   REXTTO_NO_RESTART=1 scripts/dev-reload.sh   # compila e non riavvia
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
mode="${1:-all}"

if [ "$mode" != "--ui" ]; then
    ./scripts/build-fast.sh
    # Il servizio systemd punta a target/release/rexttod: per lo sviluppo gli
    # passiamo il binario `fast` così il riavvio è immediato. Un successivo
    # `cargo build --release` lo rimpiazza con quello ottimizzato.
    cp -f target/fast/rexttod target/release/rexttod
    echo "daemon: target/fast/rexttod -> target/release/rexttod"
fi

if [ "$mode" != "--daemon" ]; then
    # Rextto serves the static UI, so the Leptos SSR server is never used:
    # `--frontend-only` skips compiling it (~3x faster UI builds).
    (cd ui && cargo leptos build --frontend-only)
fi

if [ "${REXTTO_NO_RESTART:-0}" != "1" ]; then
    curl -s --max-time 10 -X POST http://127.0.0.1:5000/api/service/restart \
        -H 'content-type: application/json' -d '{"action":"restart"}' >/dev/null || true
    echo "riavvio del servizio richiesto"
fi
