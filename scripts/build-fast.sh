#!/usr/bin/env bash
# Build di iterazione veloce.
#
# Usa il profilo `fast` (opt-level=2) invece di `release` (opt-level=3):
# tipicamente pochi secondi per una modifica a src/. Il binario finisce in
# target/fast/rexttod.
#
# Se `mold` o `lld` sono installati li usa per il link (link molto più rapido).
# Installazione consigliata (una tantum, da root):
#   sudo apt-get install -y mold
# Nota: il servizio systemd continua a usare target/release/rexttod, quindi per
# il deploy definitivo serve comunque `cargo build --release`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

link_flags=""
if command -v mold >/dev/null 2>&1; then
    link_flags="-C link-arg=-fuse-ld=mold"
elif command -v ld.lld >/dev/null 2>&1; then
    link_flags="-C link-arg=-fuse-ld=lld"
fi
if [ -n "$link_flags" ]; then
    export RUSTFLAGS="${RUSTFLAGS:-} $link_flags"
    echo "linker: $link_flags"
else
    echo "suggerimento: installa 'mold' per un link più rapido (sudo apt-get install -y mold)"
fi

exec cargo build --profile fast "$@"
