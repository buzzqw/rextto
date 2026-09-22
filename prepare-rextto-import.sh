#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE="${1:-}"
DATA_DIR="${2:-$ROOT/data}"

LEGACY_SERVICE="${REXTTO_LEGACY_SERVICE:-}"
if [ -n "$LEGACY_SERVICE" ] && systemctl is-active --quiet "$LEGACY_SERVICE"; then
    echo "Il servizio legacy '$LEGACY_SERVICE' e' ancora attivo. Fermalo prima: sudo systemctl stop $LEGACY_SERVICE" >&2
    exit 1
fi

for db in extto_series.db extto_archive.db extto_config.db comics.db; do
    if [[ ! -f "$SOURCE/$db" ]]; then
        echo "Database mancante: $SOURCE/$db" >&2
        exit 1
    fi
done

echo "Sorgente verificata: $SOURCE"
echo "Importazione diretta in: $DATA_DIR"
exec "$ROOT/import-legacy.sh" "$SOURCE" "$DATA_DIR"
