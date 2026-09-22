# Rextto

**Rextto** is a self-hosted media acquisition and archive daemon. A single Rust
process bundles the scraping engine, the SQLite storage, the web API/UI and an
embedded libtorrent session — no external services required to run it.

It watches the series and movies you configure, searches RSS/HTML sources,
Torznab indexers and public search engines, scores every release by quality,
downloads the best one and renames/archives it into your library.

> 🇮🇹 Italiano: vedi [`README.it.md`](README.it.md).
> Full user manual: [`docs/MANUAL.en.md`](docs/MANUAL.en.md) ·
> [`docs/MANUAL.it.md`](docs/MANUAL.it.md).

---

## Features

- **Sources** — generic RSS feeds, HTML listings (with FlareSolverr fallback for
  Cloudflare), Torznab indexers (Jackett/Prowlarr) and web search engines.
- **Quality scoring** — resolution, source, codec, audio, HDR/Dolby Vision,
  groups and configurable weights, with a built-in simulator.
- **Automatic upgrades** — replaces an archived file when a better release
  appears, using legacy-style rules (resolution jump, HDTV→WEB-DL, HDR, repack)
  plus a configurable score threshold.
- **Series & movies** — TMDB metadata, posters, per-season monitoring, missing
  episode search, calendar, manual search.
- **Torrents** — embedded libtorrent: queue, per-torrent limits, tags, peers,
  trackers, files, storage moves, seed policy, fastresume.
- **Comics** — GetComics monitoring and weekly packs.
- **Integrations** — Trakt, Simkl, Jellyfin, Plex, Telegram/e-mail/webhook
  notifications.
- **Privacy** — VPN killswitch: binds libtorrent listening and outgoing traffic
  to a chosen interface (`tun0`/`wg0`), selectable from the UI.
- **Web UI** — responsive single-page app (dark/light theme) with **Italian and
  English** localisation; log viewer, health, charts, maintenance tools.
- **Seen from feed** — every release seen in the sources, grouped by title and
  browsable under *Archive → Seen from feed*, even when not monitored.
- **Maintenance & backups** — duplicate video cleanup, lost source-token restore,
  database pruning, plus manual/scheduled backups (local, FTP, cloud folder,
  Telegram).
- **Feed** — rolling magnet RSS feed at `/feed.xml`, for external consumers.

## Requirements

- Linux, Rust stable (`rustc`/`cargo`).
- `libtorrent-rasterbar` development headers and a C++17 compiler (the libtorrent
  bridge is built by `build.rs`), plus OpenSSL headers.
- Optional: `mediainfo` (technical tags for renaming), `mold` (faster linking),
  FlareSolverr (Cloudflare-protected sources), `cargo-leptos` + the
  `wasm32-unknown-unknown` target (to build the UI).

```bash
sudo apt-get install -y build-essential libtorrent-rasterbar-dev libssl-dev mediainfo
```

## Build

```bash
# Daemon (optimised, used by the systemd unit)
cargo build --release

# Web UI (static bundle served by the daemon)
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cd ui && cargo leptos build --frontend-only
```

For the development loop there is a fast profile (no LTO) and helper scripts:

```bash
./scripts/build-fast.sh          # target/fast/rexttod
./scripts/dev-reload.sh --daemon # build + copy + restart the service
./scripts/dev-reload.sh          # daemon + UI + restart
cargo check                      # fastest feedback
```

## Configuration

Rextto reads `rextto.json` (if present) and stores runtime settings in
`rextto_config.db`. The data directory defaults to `data/` and can be overridden
with environment variables:

| Variable | Purpose |
|---|---|
| `REXTTO_DATA_DIR` | Data directory (databases, logs, downloads) |
| `REXTTO_LISTEN` | Web UI/API address (default `0.0.0.0:5000`) |
| `REXTTO_ENGINE_LISTEN` | Internal engine channel (default `127.0.0.1:8889`) |
| `REXTTO_ACTIVE` | `1` enables the acquisition cycles |
| `REXTTO_DRY_RUN` | `1` disables real downloads |
| `REXTTO_API_TOKEN` | Optional bearer token for the API/UI |
| `RUST_LOG` | Tracing filter (default `rextto=info`) |

Most options are editable from **Configuration** in the web UI (sources,
indexers, scoring, libtorrent, renaming, notifications, paths, translations).

## Running

Dry-run (writes only to Rextto's own directory):

```bash
./run-safe.sh
```

As a service:

```bash
./start-rextto-service.sh        # install/refresh the systemd unit and restart
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

Quick checks once it is running:

```bash
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/health
```

## Web UI and API

The UI is served from `ui/target/site/pkg` on the configured address. Selected
endpoints:

- `GET /api/status`, `GET /api/health`
- `GET /api/config`, `POST /api/config/settings`, `POST /api/config/library`
- `GET /api/series`, `GET /api/movies`, `GET /api/gaps`, `GET /api/calendar`
- `GET /api/torrents`, `POST /api/send-magnet`, torrent actions under
  `/api/torrents/{hash}/...`
- `GET /api/archive`, `POST /api/archive/batch-download`
- `GET /api/series/seen/grouped`, `GET /api/movies/seen/grouped` (seen from feed)
- `GET /api/network/interfaces`
- `POST /api/maintenance/clean-duplicates`, `POST /api/maintenance/restore-source`
- `GET /api/comics`, `POST /api/comics/explore`
- `POST /api/search`, `GET /api/sources/health`
- `GET /api/logs/stream` (SSE), `GET /feed.xml` (magnet RSS)

## Logs

Logs are written to `data/rextto.log` with **size-based rotation at 5 MB**,
keeping the active file plus three backups (`rextto.log.1` … `rextto.log.3`).
The log viewer in the UI streams them live.

## Testing

```bash
cargo test --all-targets
```

## Migrating from a legacy instance

An optional importer can read a stopped legacy data directory (series, archive,
comics databases) into Rextto's own `rextto_*.db` files:

```bash
./import-legacy.sh /path/to/legacy /home/user/rextto/data
```

It never writes to the source directory.

## License

Licensed under the **European Union Public Licence v. 1.2** — see
[`LICENSE`](LICENSE).
