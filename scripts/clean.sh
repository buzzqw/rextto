#!/usr/bin/env bash
# Mantiene snella la cartella del progetto.
#
# Di default è una simulazione (non rimuove nulla): elenca lo spazio
# recuperabile. In fase di costruzione conviene tenere i profili debug, che
# rendono veloci `cargo test` e `cargo check`.
#
# Uso:
#   scripts/clean.sh              # report (nessuna cancellazione)
#   scripts/clean.sh --debug      # rimuove i profili debug e i report di test
#   scripts/clean.sh --all        # cargo clean completo (debug + release)
#   scripts/clean.sh --auto       # come --debug ma salta se una build è in corso
#                                 # (usato dal timer systemd settimanale)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mode="report"
for arg in "$@"; do
    case "$arg" in
        --debug) mode="debug" ;;
        --all) mode="all" ;;
        --auto) mode="auto" ;;
        -h | --help)
            sed -n '2,13p' "$0"
            exit 0
            ;;
        *)
            echo "opzione sconosciuta: $arg" >&2
            exit 2
            ;;
    esac
done

# Modalità automatica (timer): non tocca nulla mentre una build è in corso, per
# non cancellare artefatti che cargo/rustc stanno usando.
if [ "$mode" = "auto" ]; then
    if pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1 \
        || pgrep -x cargo-leptos >/dev/null 2>&1; then
        echo "$(date '+%F %T') build in corso: pulizia rimandata"
        exit 0
    fi
    echo "$(date '+%F %T') pulizia automatica degli artefatti di sviluppo"
    mode="debug"
fi

du_now() {
    [ -e "$1" ] && du -sh "$1" 2>/dev/null | cut -f1 || true
}

targets=(
    "target/debug"
    "target/fast"
    "ui/target/debug"
    "ui/target/front"
    "ui/target/wasm32-unknown-unknown"
    "tui/target/debug"
    "ui/end2end/playwright-report"
    "ui/end2end/test-results"
    "scripts/__pycache__"
)

if [ "$mode" = "report" ]; then
    echo "Spazio attualmente occupato (nulla viene rimosso):"
    for path in "${targets[@]}"; do
        [ -e "$path" ] && printf '  %-40s %s\n' "$path" "$(du_now "$path")"
    done
    echo
    echo "Usa --debug per liberare i profili debug non necessari alla produzione,"
    echo "oppure --all per un 'cargo clean' completo."
    exit 0
fi

if [ "$mode" = "all" ]; then
    cargo clean
    cargo clean --manifest-path ui/Cargo.toml
    rm -rf ui/end2end/playwright-report ui/end2end/test-results
    echo "Pulizia completa eseguita."
    exit 0
fi

# --debug: rimuove solo artefatti rigenerabili e non usati in produzione.
# Restano intatti target/release (binario del servizio), ui/target/site (bundle
# servito) e tui/target/release (client installato).
rm -rf target/debug target/fast
rm -rf ui/target/debug ui/target/front ui/target/wasm32-unknown-unknown
rm -rf tui/target/debug
rm -rf ui/end2end/playwright-report ui/end2end/test-results
rm -rf scripts/__pycache__
echo "Profili debug e report di test rimossi. La prossima build di test sarà più lenta."
