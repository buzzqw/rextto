#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNIT_NAME="rextto.service"
UNIT_SOURCE="$ROOT/systemd/$UNIT_NAME"
UNIT_TARGET="/etc/systemd/system/$UNIT_NAME"
BIN="$ROOT/target/release/rexttod"
UI_BUNDLE="$ROOT/ui/target/site/pkg/ui.js"

LEGACY_SERVICE="${REXTTO_LEGACY_SERVICE:-}"
if [ -n "$LEGACY_SERVICE" ] && systemctl is-active --quiet "$LEGACY_SERVICE"; then
    echo "The legacy service '$LEGACY_SERVICE' is active and uses the same ports. Stop it first: sudo systemctl stop $LEGACY_SERVICE" >&2
    exit 1
fi

if [[ ! -x "$BIN" ]] || find "$ROOT/src" "$ROOT/native" "$ROOT/Cargo.toml" "$ROOT/build.rs" -newer "$BIN" -print -quit | grep -q .; then
    cargo build --release --manifest-path "$ROOT/Cargo.toml"
fi

if [[ ! -f "$UI_BUNDLE" ]] || find "$ROOT/ui/app/src" "$ROOT/ui/frontend/src" "$ROOT/ui/style" "$ROOT/ui/public" "$ROOT/ui/Cargo.toml" -newer "$UI_BUNDLE" -print -quit | grep -q .; then
    # The daemon serves the static UI bundle; the SSR server is not needed.
    cargo leptos --manifest-path "$ROOT/ui/Cargo.toml" build --release --frontend-only
fi

# The repository unit uses __ROOT__ placeholders: materialise the real path.
GENERATED_UNIT="$(mktemp)"
sed \
    -e "s#__ROOT__#$ROOT#g" \
    -e "s#__BIN__#$BIN#g" \
    -e "s#__DATA__#$ROOT/data#g" \
    -e "s#__UI__#$ROOT/ui/target/site#g" \
    -e "s#__USER__#$(id -un)#g" \
    -e "s#__GROUP__#$(id -gn)#g" \
    "$UNIT_SOURCE" > "$GENERATED_UNIT"
sudo install -m 0644 "$GENERATED_UNIT" "$UNIT_TARGET"
rm -f "$GENERATED_UNIT"
sudo systemctl daemon-reload
sudo systemctl enable "$UNIT_NAME"
sudo systemctl restart "$UNIT_NAME"
sudo systemctl --no-pager --full status "$UNIT_NAME"
