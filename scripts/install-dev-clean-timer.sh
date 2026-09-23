#!/usr/bin/env bash
# Installa (come unità systemd utente) il timer che ripulisce periodicamente gli
# artefatti di build sviluppo, così target/debug non cresce all'infinito.
#
# Non serve root. Con Linger abilitato (loginctl enable-linger <utente>) il
# timer parte anche quando l'utente non è connesso.
#
# Uso: scripts/install-dev-clean-timer.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$DEST"

for unit in rextto-dev-clean.service rextto-dev-clean.timer; do
    sed "s#__ROOT__#$ROOT#g" "$ROOT/systemd/$unit" > "$DEST/$unit"
done

systemctl --user daemon-reload
systemctl --user enable --now rextto-dev-clean.timer
echo
systemctl --user --no-pager list-timers rextto-dev-clean.timer
