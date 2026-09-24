#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="${REXTTO_DATA_DIR:-$ROOT/data}"
LOG_FILE="${1:-$DATA_DIR/rextto.log}"

if [[ ! -r "$LOG_FILE" ]]; then
    echo "Log non trovato o non leggibile: $LOG_FILE" >&2
    echo "Imposta REXTTO_DATA_DIR oppure passa il percorso del log come argomento." >&2
    exit 1
fi

if command -v less >/dev/null 2>&1; then
    exec less +F -- "$LOG_FILE"
fi

echo "less non è installato: uso tail -F (CTRL-C per interrompere)." >&2
exec tail -n 100 -F -- "$LOG_FILE"
