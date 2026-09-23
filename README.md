# Rextto

**Rextto** is a self-hosted daemon for the automatic acquisition and archiving of
TV series, movies and comics.

A single Rust process bundles everything: the scraping engine, the SQLite
archive, the web UI/API and an embedded **libtorrent** session. No external
services required.

It watches what you configure, searches RSS/HTML sources, Torznab indexers
(Jackett/Prowlarr) and public search engines, scores every release by quality,
downloads the best one and renames/archives it into your library (NAS or local
disk).

> 🇮🇹 Italiano: [`README.it.md`](README.it.md)
> 📖 Full manual: [`docs/MANUAL.en.md`](docs/MANUAL.en.md) ·
> [`docs/MANUAL.it.md`](docs/MANUAL.it.md)

[![Donate](https://img.shields.io/badge/❤️_Support_Rextto-PayPal-00457C.svg)](https://www.paypal.com/cgi-bin/webscr?cmd=_donations&business=azanzani@gmail.com&item_name=Support+Rextto+Project)

---

## What Rextto is

- **One daemon, no external orchestrator** — scraping, downloading, renaming,
  archiving and the UI live in the same process.
- **Multiple sources** — generic RSS feeds, HTML listings (with FlareSolverr
  fallback for Cloudflare), Torznab indexers (Jackett/Prowlarr) and web engines.
- **Quality scoring** — resolution, source, codec, audio, HDR/Dolby Vision,
  groups and configurable weights, with a built-in simulator.
- **Automatic upgrades** — replaces an archived file when a better release
  appears (resolution jump, HDTV→WEB-DL, HDR, repack) beyond a configurable
  score threshold.
- **Series & movies** — TMDB metadata, posters, per-season monitoring, missing
  episode search, calendar, manual search.
- **Torrents** — embedded libtorrent: queue, limits, tags, peers, trackers,
  files, storage moves, seed policy, fastresume, VPN killswitch.
- **Comics** — GetComics monitoring and weekly packs.
- **Integrations** — Trakt, Simkl, Jellyfin, Plex, Telegram/e-mail/webhook
  notifications.
- **Web UI** — responsive single-page app, dark/light theme, **Italian and
  English**, with log viewer, health, charts and maintenance tools.
- **Seen from feed** — every release seen in the sources, grouped by title,
  browsable even for titles you do not monitor.
- **Backups** — manual or scheduled (local, FTP, cloud folder, Telegram).

## Installation

### Requirements

- Linux, Rust stable (`rustc`/`cargo`).
- `libtorrent-rasterbar` development headers and a C++17 compiler (the bridge is
  built by `build.rs`), plus OpenSSL headers.
- Optional: `mediainfo` (technical tags for renaming), `mold` (faster linking),
  FlareSolverr (Cloudflare-protected sources).
- For the UI: `cargo-leptos` and the `wasm32-unknown-unknown` target.

```bash
sudo apt-get install -y build-essential libtorrent-rasterbar-dev libssl-dev mediainfo
```

### 1. Build the daemon

```bash
cargo build --release
# binary: target/release/rexttod
```

### 2. Build the web UI

The UI is a static bundle served by the daemon.

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

### 3. First try in dry-run

No real downloads start and nothing outside its own directory is touched.

```bash
./run-safe.sh          # serves http://127.0.0.1:5000 with REXTTO_ACTIVE=0
```

### 4. Install as a service

The script builds if needed, installs/refreshes the systemd unit and restarts it.

```bash
./start-rextto-service.sh
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

### 5. Check it

```bash
curl --fail http://127.0.0.1:5000/api/status
curl --fail http://127.0.0.1:5000/api/health
```

## How to use it

Everything is managed from the web UI, on the configured address (default
`http://<host>:5000`).

### First run

1. Open the UI. If no data directory exists yet, complete the **initial setup**.
2. Keep the daemon in **dry-run** until your sources are configured: in dry-run
   no downloads start.
3. When ready, enable **active mode** in *Configuration → Daemon*.

### Configure the sources

In *Configuration → Sources* add:

- RSS feeds / HTML listings (URL and pages to follow);
- Torznab indexers (Jackett/Prowlarr) with the **Verify** button;
- web search engines and, if needed, the FlareSolverr URL;
- content filters and blocklist.

The same section holds scoring, renaming, paths and libtorrent settings.

### Add series and movies

- From **Explore** (TMDB search) in one click, or
- from **Series / Movies → Add** manually.

For each title pick minimum quality, language, seasons/years, aliases, exclusions
and the **NAS path**. Comics are managed from **Comics**.

### Cycles and downloads

Rextto works in cycles: search, evaluate, download, rename, archive.

- From the **Dashboard** you can start a full cycle, a single domain (Series,
  Movies, Comics) or an immediate backup.
- Cycles also run automatically at the configured interval.
- In **Downloads → Torrent session** you see the torrents in the client (with a
  **NAS** badge when already archived); under the name you find the **reason** for
  the download and its **source** (indexer/RSS/web). **Clean completed** removes
  torrents that reached their seed limit.
- **Download history** lists torrents that **left the session**, with the outcome
  (NAS path or the reason they were rejected).

### UI sections

| Section | Purpose |
|---|---|
| **Dashboard** | Manual search, cycle buttons, stats, network, upcoming releases |
| **Downloads** | Torrent session, add magnet/.torrent, history |
| **Series / Movies** | Library, details, missing episodes, manual search |
| **Missing** | Missing episodes and gap filling |
| **Calendar** | Upcoming releases from monitored series |
| **Explore** | TMDB discovery and release search |
| **Archive** | Past releases; *Seen from feed* for movies/series |
| **Comics** | GetComics and weekly packs |
| **Configuration** | Sources, libtorrent, scoring, renaming, paths, notifications |
| **Integrations** | Trakt, Simkl, Jellyfin, Plex |
| **Maintenance** | Backups, duplicates, scoring, legacy import, restart |
| **Health, Logs, Charts** | Diagnostics and monitoring |

A detailed walkthrough of every screen is in the
[manual](docs/MANUAL.en.md).

### Terminal UI

For servers without a browser, Rextto ships a terminal client with two modes.

```bash
./start-tui.sh                      # interactive full-screen view
./start-tui.sh status               # text command, pipe/script friendly
```

**Text commands** (work in any shell, no full-screen required):

| Command | Output |
|---|---|
| `rextto-tui status` | daemon summary (active, torrents, last cycle, feeds) |
| `rextto-tui torrents` | torrent session table |
| `rextto-tui events` | recent torrent events |
| `rextto-tui logs [n]` | last log lines (default 80) |
| `rextto-tui health` | health report (JSON) |
| `rextto-tui cycle [series\|movies\|comics]` | run a cycle |
| `rextto-tui pause <hash>` / `resume <hash>` | control a torrent |
| `rextto-tui rename-all` | background library rename |

With no arguments on a real terminal it opens the interactive view
(**Status, Torrents, Activity, Logs, Health**; `c` cycle, `p` pause/resume,
`1-5` tabs, `q` quit). Point it elsewhere with `REXTTO_URL` /
`REXTTO_API_TOKEN`.

### Data and logs

- Default data directory: `data/` (override with `REXTTO_DATA_DIR`).
- Logs in `data/rextto.log`, with 5 MB rotation (active file + 3 backups),
  streamed live in the UI.

### Environment variables

| Variable | Purpose |
|---|---|
| `REXTTO_DATA_DIR` | Data directory (databases, logs, downloads) |
| `REXTTO_LISTEN` | Web UI/API address (default `0.0.0.0:5000`) |
| `REXTTO_ENGINE_LISTEN` | Internal engine channel (default `127.0.0.1:8889`) |
| `REXTTO_ACTIVE` | `1` enables the acquisition cycles |
| `REXTTO_DRY_RUN` | `1` disables real downloads |
| `REXTTO_API_TOKEN` | Optional bearer token for the API/UI |
| `RUST_LOG` | Tracing filter (default `rextto=info`) |
| `REXTTO_URL` | TUI client: daemon base URL (default `http://127.0.0.1:5000`) |

## Development

```bash
./scripts/build-fast.sh          # fast daemon build (target/fast/rexttod)
./scripts/dev-reload.sh --daemon # build + copy + restart the service
./scripts/dev-reload.sh          # daemon + UI + restart
cargo check                      # fastest feedback
cargo test --all-targets         # tests
```

## Migrating from a legacy instance

An optional importer reads a stopped legacy data directory (series, archive,
comics databases) into Rextto's own `rextto_*.db` files. It never writes to the
source directory.

```bash
./import-legacy.sh /path/to/legacy /home/user/rextto/data
```

## ❤️ Support the project

Rextto is free, open-source software built entirely in spare time.
If it saves you hours of configuration, RAM, or disk wear — consider buying the
author a coffee.

Every donation directly funds new features, bug fixes, and keeping the project
alive.

<div align="center">

[![Donate with PayPal](https://img.shields.io/badge/Donate-PayPal-00457C?style=for-the-badge&logo=paypal)](https://www.paypal.com/cgi-bin/webscr?cmd=_donations&business=azanzani@gmail.com&item_name=Support+Rextto+Project)

*Thank you. Seriously.*

</div>

## ⚖️ Legal & fair use

Rextto is a **download automation tool**. It does not host, index, or distribute
any copyrighted content.

- Rextto connects to **indexers you configure** (Jackett, Prowlarr, public RSS
  feeds). It has no built-in index.
- What you download is **entirely your responsibility**. Use Rextto only for
  content you have the right to access — public domain, Creative Commons, or
  media you own.
- The torrent integration (libtorrent) is a neutral technology. Rextto does not
  encourage or facilitate piracy.
- This project is released under the **EUPL 1.2** open-source license.

> *"With great automation comes great responsibility."*

## License

Licensed under the **European Union Public Licence v. 1.2** — see [`LICENSE`](LICENSE).
