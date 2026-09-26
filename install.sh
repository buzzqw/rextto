#!/usr/bin/env bash
# Rextto installer/updater.
#
# Supported entry point:
#   curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | bash
#
# The installer prefers a prebuilt GitHub Release. If no release asset exists
# yet, it falls back to building the current GitHub source tree. Runtime data is
# deliberately kept outside the installation directory, so updates never touch
# databases, downloads, logs or configuration.
set -Eeuo pipefail

readonly REPO="${REXTTO_REPO:-buzzqw/rextto}"
readonly INSTALL_DIR="${REXTTO_INSTALL_DIR:-/opt/rextto}"
readonly DATA_DIR="${REXTTO_DATA_DIR:-/var/lib/rextto}"
readonly PORT="${REXTTO_PORT:-5000}"
readonly ENGINE_PORT="${REXTTO_ENGINE_PORT:-8889}"
readonly RELEASE="${REXTTO_VERSION:-${REXTTO_CHANNEL:-continuous}}"
readonly SOURCE_REF="${REXTTO_SOURCE_REF:-main}"
readonly LIBTORRENT_VERSION="${REXTTO_LIBTORRENT_VERSION:-2.0.11}"
readonly LIBTORRENT_PREFIX="${REXTTO_LIBTORRENT_PREFIX:-$INSTALL_DIR/libtorrent}"
readonly SERVICE_NAME="rextto.service"

TMP_DIR=""
SERVICE_USER=""
SERVICE_GROUP=""
PAYLOAD_BINARY=""
PAYLOAD_UI=""
PAYLOAD_LIB=""
PAYLOAD_ROOT=""
PAYLOAD_VERSION=""

if [[ ${EUID} -eq 0 ]]; then
    SUDO=()
else
    command -v sudo >/dev/null 2>&1 || {
        echo "Rextto installation needs root privileges (install sudo or run as root)." >&2
        exit 1
    }
    SUDO=(sudo)
fi

log() { printf '[rextto] %s\n' "$*"; }
die() { echo "[rextto] ERROR: $*" >&2; exit 1; }
root_cmd() { "${SUDO[@]}" "$@"; }

cleanup() {
    if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
        rm -rf "$TMP_DIR"
    fi
}
trap cleanup EXIT

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64) printf 'x86_64\n' ;;
        aarch64|arm64) printf 'aarch64\n' ;;
        *) die "unsupported CPU architecture: $(uname -m)" ;;
    esac
}

install_system_packages() {
    [[ "${REXTTO_SKIP_PACKAGES:-0}" == "1" ]] && return
    local id="" version_id=""
    # shellcheck disable=SC1091
    source /etc/os-release
    id="${ID:-}"
    version_id="${VERSION_ID:-}"
    log "preparing ${id:-Linux} ${version_id} dependencies"

    case "$id" in
        debian|ubuntu|linuxmint|pop)
            root_cmd apt-get update
            root_cmd env DEBIAN_FRONTEND=noninteractive apt-get install -y \
                ca-certificates curl tar gzip xz-utils pkg-config \
                build-essential binutils mold cmake libssl-dev \
                libboost-dev libboost-system-dev mediainfo
            ;;
        fedora)
            root_cmd dnf install -y \
                ca-certificates curl tar gzip xz pkgconf-pkg-config \
                gcc-c++ binutils mold make cmake openssl-devel boost-devel mediainfo
            ;;
        rhel|rocky|almalinux|centos)
            root_cmd dnf install -y epel-release || true
            root_cmd dnf install -y \
                ca-certificates curl tar gzip xz pkgconf-pkg-config \
                gcc-c++ binutils mold make cmake openssl-devel boost-devel mediainfo
            ;;
        opensuse*|sles)
            root_cmd zypper --non-interactive refresh
            root_cmd zypper --non-interactive install --no-recommends \
                ca-certificates curl tar gzip xz pkg-config \
                gcc-c++ binutils mold make cmake libopenssl-devel boost-devel mediainfo
            ;;
        arch|manjaro|endeavouros)
            root_cmd pacman -Sy --noconfirm --needed \
                ca-certificates curl tar gzip xz pkgconf \
                base-devel binutils mold cmake openssl boost mediainfo
            ;;
        *)
            die "unsupported distribution '$id'. Supported: Debian/Ubuntu, Fedora/RHEL/Rocky/Alma, openSUSE and Arch Linux."
            ;;
    esac
}

configure_libtorrent_environment() {
    local lib_dir="$LIBTORRENT_PREFIX/lib"
    local lib64_dir="$LIBTORRENT_PREFIX/lib64"
    export PKG_CONFIG_PATH="$LIBTORRENT_PREFIX/lib/pkgconfig:$LIBTORRENT_PREFIX/lib64/pkgconfig:${PKG_CONFIG_PATH:-}"
    export CMAKE_PREFIX_PATH="$LIBTORRENT_PREFIX${CMAKE_PREFIX_PATH:+:$CMAKE_PREFIX_PATH}"
    export CXXFLAGS="-I$LIBTORRENT_PREFIX/include ${CXXFLAGS:-}"
    export LIBRARY_PATH="$lib_dir:$lib64_dir:${LIBRARY_PATH:-}"
    export LD_LIBRARY_PATH="$lib_dir:$lib64_dir:${LD_LIBRARY_PATH:-}"
    export RUSTFLAGS="${RUSTFLAGS:-} -L native:$lib_dir -L native:$lib64_dir"
}

build_libtorrent() {
    [[ "${REXTTO_SKIP_LIBTORRENT_BUILD:-0}" == "1" ]] && {
        log "skipping libtorrent source build because REXTTO_SKIP_LIBTORRENT_BUILD=1"
        return
    }

    local marker="$LIBTORRENT_PREFIX/VERSION"
    if [[ -s "$marker" ]] && [[ "$(cat "$marker")" == "$LIBTORRENT_VERSION" ]] \
        && { [[ -f "$LIBTORRENT_PREFIX/lib/libtorrent-rasterbar.so" ]] \
            || [[ -f "$LIBTORRENT_PREFIX/lib64/libtorrent-rasterbar.so" ]]; }; then
        log "libtorrent $LIBTORRENT_VERSION is already installed"
        configure_libtorrent_environment
        return
    fi

    log "building libtorrent $LIBTORRENT_VERSION"
    local archive="$TMP_DIR/libtorrent.tar.gz"
    local source_root="$TMP_DIR/libtorrent-source"
    curl -fsSL --retry 3 \
        "https://github.com/arvidn/libtorrent/releases/download/v$LIBTORRENT_VERSION/libtorrent-rasterbar-$LIBTORRENT_VERSION.tar.gz" \
        -o "$archive"
    mkdir -p "$source_root"
    tar -xzf "$archive" -C "$source_root"
    local source_dir
    source_dir="$(find "$source_root" -mindepth 1 -maxdepth 1 -type d -print -quit)"
    [[ -n "$source_dir" ]] || die "could not unpack the libtorrent source archive"
    local build_dir="$source_dir/build"
    cmake -S "$source_dir" -B "$build_dir" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$LIBTORRENT_PREFIX" \
        -DCMAKE_INSTALL_LIBDIR=lib \
        -Dbuild_tests=OFF \
        -Dbuild_examples=OFF \
        -Dbuild_tools=OFF \
        -Dpython-bindings=OFF
    cmake --build "$build_dir" --parallel "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"
    root_cmd cmake --install "$build_dir"
    root_cmd install -d -m 0755 "$LIBTORRENT_PREFIX"
    printf '%s\n' "$LIBTORRENT_VERSION" | root_cmd tee "$marker" >/dev/null
    configure_libtorrent_environment
}

select_service_account() {
    SERVICE_USER="${REXTTO_USER:-rextto}"
    [[ "$SERVICE_USER" != root ]] || die "REXTTO_USER cannot be root"

    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
        log "creating service account $SERVICE_USER"
        local nologin="/usr/sbin/nologin"
        [[ -x "$nologin" ]] || nologin="/usr/bin/nologin"
        root_cmd useradd --system --home-dir "$DATA_DIR" --shell "$nologin" "$SERVICE_USER"
    fi
    SERVICE_GROUP="$(id -gn "$SERVICE_USER")"
}

download_release() {
    local arch="$1" base archive checksum
    archive="$TMP_DIR/rextto-linux-$arch.tar.gz"
    checksum="$archive.sha256"
    if [[ "$RELEASE" == latest || "$RELEASE" == stable ]]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$RELEASE"
    fi

    log "looking for prebuilt release ($RELEASE, $arch)"
    if ! curl -fsSL --retry 3 "$base/rextto-linux-$arch.tar.gz" -o "$archive"; then
        return 1
    fi
    if curl -fsSL --retry 3 "$base/rextto-linux-$arch.tar.gz.sha256" -o "$checksum"; then
        # Compare the hashes directly: the published .sha256 may name the file
        # differently (e.g. an absolute path from the packaging step), and
        # `sha256sum -c` would then fail only because the filename is missing.
        local expected actual
        expected="$(awk '{print $1; exit}' "$checksum" | tr 'A-F' 'a-f')"
        actual="$(sha256sum "$archive" | awk '{print $1}')"
        [[ -n "$expected" && "$expected" == "$actual" ]] \
            || die "release checksum verification failed"
        log "release checksum verified"
    else
        log "release has no checksum asset; continuing with HTTPS transport verification"
    fi
    mkdir -p "$TMP_DIR/release"
    tar -xzf "$archive" -C "$TMP_DIR/release"
    local binary
    binary="$(find "$TMP_DIR/release" -type f -name rexttod -print -quit)"
    [[ -n "$binary" ]] || die "release archive does not contain rexttod"
    local ui_bundle
    ui_bundle="$(find "$TMP_DIR/release" -type f -path '*/ui/pkg/ui.js' -print -quit)"
    [[ -n "$ui_bundle" ]] || die "release archive does not contain the web UI"
    local site="${ui_bundle%/pkg/ui.js}"
    [[ -f "$site/pkg/ui.js" ]] || die "release archive does not contain the web UI"
    local lib_dir
    lib_dir="$(find "$TMP_DIR/release" -type d -name lib -print -quit)"
    PAYLOAD_BINARY="$binary"
    PAYLOAD_UI="$site"
    PAYLOAD_LIB="$lib_dir"
    PAYLOAD_ROOT="$(dirname "$binary")"
    PAYLOAD_VERSION="$RELEASE"
    return 0
}

ensure_rust() {
    if ! command -v cargo >/dev/null 2>&1 || ! command -v rustup >/dev/null 2>&1; then
        log "Rust is not installed as a rustup toolchain; installing the minimal stable toolchain"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi
    command -v rustup >/dev/null 2>&1 || die "cargo exists but rustup is unavailable; install Rust or use a GitHub Release"
    rustup target add wasm32-unknown-unknown
    if ! command -v cargo-leptos >/dev/null 2>&1; then
        log "installing cargo-leptos for the fallback source build"
        cargo install cargo-leptos --locked
    fi
}

build_from_source() {
    log "no compatible release asset found; building Rextto from GitHub source"
    local source_archive="$TMP_DIR/source.tar.gz"
    curl -fsSL --retry 3 \
        "https://github.com/$REPO/archive/refs/heads/$SOURCE_REF.tar.gz" \
        -o "$source_archive"
    mkdir -p "$TMP_DIR/source"
    tar -xzf "$source_archive" -C "$TMP_DIR/source"
    local source_dir
    source_dir="$(find "$TMP_DIR/source" -mindepth 1 -maxdepth 1 -type d -print -quit)"
    [[ -n "$source_dir" ]] || die "could not unpack the GitHub source archive"
    ensure_rust
    (cd "$source_dir" && cargo build --release --locked)
    (cd "$source_dir" && cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only)
    PAYLOAD_BINARY="$source_dir/target/release/rexttod"
    PAYLOAD_UI="$source_dir/ui/target/site"
    PAYLOAD_LIB=""
    PAYLOAD_ROOT="$source_dir"
    PAYLOAD_VERSION="source-$SOURCE_REF"
}

install_payload() {
    [[ -x "$PAYLOAD_BINARY" ]] || die "built payload does not contain an executable rexttod"
    [[ -f "$PAYLOAD_UI/pkg/ui.js" ]] || die "built payload does not contain the web UI"
    log "installing Rextto in $INSTALL_DIR"
    root_cmd install -d -m 0755 "$INSTALL_DIR"
    root_cmd install -m 0755 "$PAYLOAD_BINARY" "$INSTALL_DIR/rexttod"
    root_cmd rm -rf "$INSTALL_DIR/ui"
    root_cmd cp -a "$PAYLOAD_UI" "$INSTALL_DIR/ui"
    if [[ -n "${PAYLOAD_LIB:-}" && -d "$PAYLOAD_LIB" ]]; then
        root_cmd install -d -m 0755 "$INSTALL_DIR/lib"
        root_cmd cp -a "$PAYLOAD_LIB/." "$INSTALL_DIR/lib/"
    fi
    # Launcher and release notes: prefer the ones shipped in the payload, then
    # fall back to a generated launcher so `run.sh` always exists (the README
    # documents it and manual runs use it).
    if [[ -n "${PAYLOAD_ROOT:-}" && -f "$PAYLOAD_ROOT/run.sh" ]]; then
        root_cmd install -m 0755 "$PAYLOAD_ROOT/run.sh" "$INSTALL_DIR/run.sh"
    else
        root_cmd tee "$INSTALL_DIR/run.sh" >/dev/null <<'RUN'
#!/usr/bin/env bash
# Launcher for the installed Rextto payload.
set -Eeuo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export REXTTO_UI_DIR="${REXTTO_UI_DIR:-$here/ui}"
exec "$here/rexttod" "$@"
RUN
        root_cmd chmod 0755 "$INSTALL_DIR/run.sh"
    fi
    if [[ -n "${PAYLOAD_ROOT:-}" && -f "$PAYLOAD_ROOT/README.md" ]]; then
        root_cmd install -m 0644 "$PAYLOAD_ROOT/README.md" "$INSTALL_DIR/README.md"
    fi
    printf '%s\n' "$PAYLOAD_VERSION" | root_cmd tee "$INSTALL_DIR/VERSION" >/dev/null
}

install_service() {
    log "creating systemd service"
    root_cmd install -d -m 0755 -o "$SERVICE_USER" -g "$SERVICE_GROUP" "$DATA_DIR"
    root_cmd install -d -m 0755 -o "$SERVICE_USER" -g "$SERVICE_GROUP" \
        "$DATA_DIR/downloads" "$DATA_DIR/incomplete" "$DATA_DIR/state" "$DATA_DIR/trash" "$DATA_DIR/backups"
    root_cmd chown -R "$SERVICE_USER:$SERVICE_GROUP" "$DATA_DIR"
    root_cmd tee "/etc/systemd/system/$SERVICE_NAME" >/dev/null <<UNIT
[Unit]
Description=Rextto media acquisition daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_GROUP
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/rexttod --config $DATA_DIR/rextto.json
Restart=on-failure
RestartSec=10
Environment=REXTTO_DATA_DIR=$DATA_DIR
Environment=REXTTO_UI_DIR=$INSTALL_DIR/ui
Environment=LD_LIBRARY_PATH=$INSTALL_DIR/lib:$LIBTORRENT_PREFIX/lib:$LIBTORRENT_PREFIX/lib64
Environment=REXTTO_LISTEN=0.0.0.0:$PORT
Environment=REXTTO_ENGINE_LISTEN=127.0.0.1:$ENGINE_PORT
Environment=REXTTO_ACTIVE=${REXTTO_ACTIVE:-1}
Environment=REXTTO_DRY_RUN=${REXTTO_DRY_RUN:-0}
Environment=REXTTO_LIBTORRENT=1
Environment=RUST_LOG=${RUST_LOG:-rextto=info}

[Install]
WantedBy=multi-user.target
UNIT
    root_cmd systemctl daemon-reload
    root_cmd systemctl enable "$SERVICE_NAME" >/dev/null
    root_cmd systemctl restart "$SERVICE_NAME"
    root_cmd systemctl is-active --quiet "$SERVICE_NAME" || {
        root_cmd systemctl --no-pager --full status "$SERVICE_NAME" || true
        die "Rextto service did not start"
    }
}

main() {
    command -v curl >/dev/null 2>&1 || die "curl is required to start the installer"
    command -v systemctl >/dev/null 2>&1 || die "systemd is required on the target machine"
    TMP_DIR="$(mktemp -d -t rextto-install.XXXXXX)"
    local arch
    arch="$(detect_arch)"
    install_system_packages
    select_service_account
    # A prebuilt release already bundles libtorrent in `lib/`, so only build it
    # from source when we have to compile the daemon ourselves.
    if ! download_release "$arch"; then
        build_libtorrent
        build_from_source
    fi
    # Stop only after the new payload is ready. This keeps an update available
    # even if a download/build fails halfway through.
    root_cmd systemctl stop "$SERVICE_NAME" >/dev/null 2>&1 || true
    install_payload
    install_service
    log "installation/update complete"
    log "open http://$(hostname -f 2>/dev/null || hostname):$PORT"
    log "data and databases: $DATA_DIR"
    log "logs: $DATA_DIR/rextto.log"
}

main "$@"
