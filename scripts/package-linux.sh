#!/usr/bin/env bash
# Build the standalone Rextto Linux package.
#
# The archive contains everything needed to run the daemon on Linux x86_64/
# aarch64 without a compiler:
#
#   rexttod            the daemon (with an $ORIGIN/lib rpath)
#   ui/                the compiled web interface
#   lib/               the bundled libtorrent shared object
#   run.sh             launcher that sets LD_LIBRARY_PATH and REXTTO_UI_DIR
#   README.md          quick start and prerequisites
#
# Usage:
#   scripts/package-linux.sh [--binary PATH] [--ui PATH] [--output FILE]
#                            [--arch ARCH] [--no-libtorrent] [--label TEXT]
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="$ROOT/target/release/rexttod"
UI="$ROOT/ui/target/site"
ARCH="$(uname -m)"
case "$ARCH" in
    amd64) ARCH="x86_64" ;;
    arm64) ARCH="aarch64" ;;
esac
OUTPUT="$ROOT/rextto-linux-$ARCH.tar.gz"
BUNDLE_LIBTORRENT=1
LABEL="$(cat "$ROOT/build_number" 2>/dev/null || echo "dev")"

usage() {
    cat <<'USAGE'
Build the standalone Rextto Linux package.

Usage:
  scripts/package-linux.sh [--binary PATH] [--ui PATH] [--output FILE]
                           [--arch ARCH] [--no-libtorrent] [--label TEXT]

Options:
  --binary PATH     daemon executable (default: target/release/rexttod)
  --ui PATH         compiled web UI (default: ui/target/site)
  --output FILE     output archive (default: rextto-linux-<arch>.tar.gz)
  --arch ARCH       target architecture (default: host)
  --label TEXT      build label written in the archive README
  --no-libtorrent   do not bundle the libtorrent shared library
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary) BINARY="$2"; shift 2 ;;
        --ui) UI="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        --label) LABEL="$2"; shift 2 ;;
        --no-libtorrent) BUNDLE_LIBTORRENT=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[[ -x "$BINARY" ]] || { echo "error: executable not found: $BINARY" >&2; exit 1; }
[[ -f "$UI/pkg/ui.js" ]] || { echo "error: web UI not found in $UI (expected $UI/pkg/ui.js)" >&2; exit 1; }

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/rextto-package.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

cp "$BINARY" "$STAGE/rexttod"
chmod 0755 "$STAGE/rexttod"
cp -a "$UI" "$STAGE/ui"

if [[ "$BUNDLE_LIBTORRENT" == "1" ]]; then
    libtorrent="$(ldd "$BINARY" 2>/dev/null | awk '/libtorrent-rasterbar/ {print $3; exit}')"
    if [[ -z "$libtorrent" || ! -f "$libtorrent" ]]; then
        echo "error: cannot find the libtorrent shared library to bundle" >&2
        echo "       install libtorrent-rasterbar or pass --no-libtorrent" >&2
        exit 1
    fi
    mkdir -p "$STAGE/lib"
    # The binary records the soname `libtorrent-rasterbar.so.2.0`, so copy the
    # resolved library under exactly that name.
    cp -L "$libtorrent" "$STAGE/lib/$(basename "$(ldd "$BINARY" | awk '/libtorrent-rasterbar/ {print $1; exit}')")"
    echo "[package] bundled libtorrent from $libtorrent"
fi

cat >"$STAGE/run.sh" <<'RUN'
#!/usr/bin/env bash
# Launcher for the standalone Rextto package.
set -Eeuo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export REXTTO_UI_DIR="${REXTTO_UI_DIR:-$here/ui}"
exec "$here/rexttod" "$@"
RUN
chmod 0755 "$STAGE/run.sh"

cat >"$STAGE/README.md" <<README
# Rextto standalone package

This archive contains everything needed to run Rextto on Linux **$ARCH**.

## Contents

- \`rexttod\` — the daemon (linked against the bundled \`lib/\`)
- \`ui/\` — the compiled web interface
- \`lib/\` — the bundled libtorrent shared library
- \`run.sh\` — launcher (sets \`LD_LIBRARY_PATH\` and \`REXTTO_UI_DIR\`)

## Quick start

\`\`\`bash
# Check the version
./run.sh --version

# Run against a local data directory, without real downloads
REXTTO_DATA_DIR="\$PWD/data" REXTTO_DRY_RUN=1 REXTTO_ACTIVE=0 \\
  ./run.sh --config "\$PWD/data/rextto.json"

# Or install it as a service (recommended)
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | bash
\`\`\`

The web UI is served at \`http://<host>:5000\` by default.

## Prerequisites

The package needs a 64-bit Linux system with glibc, \`libstdc++\`, OpenSSL 3
(\`libssl.so.3\`/\`libcrypto.so.3\`), \`zlib\` and \`libzstd\`. These are part of
every current distribution. \`ffprobe\` (package \`ffmpeg\`) is optional and only
enables real media inspection.

## Updating

The installed \`rexttod\` can update itself:

\`\`\`bash
./run.sh --update
\`\`\`

Build metadata: $LABEL
README

entries=(rexttod ui run.sh README.md)
[[ -d "$STAGE/lib" ]] && entries+=(lib)
tar -C "$STAGE" -czf "$OUTPUT" "${entries[@]}"
sha256sum "$OUTPUT" > "$OUTPUT.sha256"
echo "[package] wrote $OUTPUT"
echo "[package] wrote $OUTPUT.sha256"
