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
> 🔬 Feature research (Sonarr/Radarr/qBittorrent/BiglyBT/autobrr):
> [`docs/FEATURE-RESEARCH.md`](docs/FEATURE-RESEARCH.md)

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
  files, storage moves, seed policy, fastresume, VPN killswitch and restart
  recovery.
- **Comics** — GetComics monitoring and weekly packs.
- **Integrations** — Trakt, Simkl, Jellyfin, Plex, Telegram/e-mail/webhook
  notifications.
- **Web UI** — responsive single-page app, dark/light theme, **Italian and
  English**, with log viewer, health, charts and maintenance tools.
- **Seen from feed** — every release seen in the sources, grouped by title,
  browsable even for titles you do not monitor.
- **Automatic sanity rules** — hardcoded subtitles and absurd file sizes
  (per-resolution floors derived from a real archive) are rejected with a clear
  `INFO` log line; no numbers to configure.
- **Acquisition tuning** — delay before grabbing with a pending queue,
  per-title "allow upgrades" switch, ground-truth media inspection via
  `ffprobe`, escalating provider backoff and scheduled housekeeping/VACUUM.
- **Automation without clutter** — external event hooks live under
  *Integrations* and watched folders under *Configuration*.
- **Monotonic library** — Rextto never pulls an older episode outside a
  recognised gap while it already owns later ones (a genuine upgrade below the
  profile cutoff still passes); gap-fill and manual actions always win.
- **Backups** — manual or scheduled (local, FTP, cloud folder, Telegram).
  They include databases and configuration, not media files or torrent state.

## Installation

### Install on a Linux server

The official installer supports Debian, Ubuntu, Fedora, openSUSE and Arch Linux.
It installs the compiler dependencies, builds and installs **libtorrent** from
source, then downloads the continuous build of the latest `main` commit. Stable
release assets can be selected explicitly; if no asset exists yet it automatically
builds the current GitHub source instead.
It also creates the service account, systemd service, runtime directories and
the empty databases on the first start. It never imports legacy data.

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | bash
```

Run the same command again to check for updates and restart Rextto with the new
version. Existing databases, configuration, downloads, archive paths and logs
are kept in `/var/lib/rextto`; the program and web UI live in `/opt/rextto`.

To install the latest stable tagged release instead of the continuous build:

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | \
  REXTTO_CHANNEL=stable bash
```

Useful overrides (optional):

```bash
curl -fsSL https://raw.githubusercontent.com/buzzqw/rextto/main/install.sh | \
  REXTTO_DATA_DIR=/srv/rextto REXTTO_PORT=5000 bash
```

The service is `rextto.service`:

```bash
sudo systemctl status rextto.service
sudo journalctl -u rextto.service -f
```

### Build from a checkout (development)

For contributors, install the build dependencies and build the daemon:

```bash
cargo build --release
# binary: target/release/rexttod
```

The UI is a static bundle served by the daemon:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

To run locally in dry-run mode, without real downloads:

```bash
REXTTO_DATA_DIR="$PWD/data" REXTTO_ACTIVE=0 REXTTO_DRY_RUN=1 \
  cargo run --release -- --dry-run
```

For a real service installation, use the installer described above.

### Check it

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

### Reading the logs

The log follows the operation instead of using an info hash as the only readable
identity:

1. **CYCLE STARTED** identifies the mode and domain (`full`, Series, Movies or
   Comics).
2. **Step 1/2** and **Step 2/2** show how many sources and titles are being
   analysed; unreachable sources include their name and reason.
3. **Gap fill** separates archive hits from episodes that require an
   online search.
4. **Download started / Gap filled** shows title, episodes, source and
   score. A skipped candidate includes the decision reason.
5. **CYCLE REPORT** and **CYCLE DOWNLOADS** summarise duration, releases,
   downloads, upgrades, gaps and errors.
6. Torrent events explain metadata, NAS/RAM-disk moves, completion, renaming,
   seeding, recovery and removal.

Each line uses `date time LEVEL [component] message · key: value`. Torrent lines
always include a readable name or title; the hash is only a technical correlation
field for errors. `INFO` describes the normal path, `WARN`/`ERROR` explain the
failed operation and affected resource, and `DEBUG` adds diagnostic detail when
enabled.

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
| **Integrations** | Trakt, Simkl, Jellyfin, Plex, event hooks |
| **Maintenance** | Backups, duplicates, scoring, restart |
| **Health, Logs, Charts** | Diagnostics and monitoring |

A detailed walkthrough of every screen is in the
[manual](docs/MANUAL.en.md).

### Terminal UI

For servers without a browser, Rextto ships a dependency-free Python TUI
(standard library `curses` + `urllib`, nothing to install):

```bash
python3 scripts/rextto_tui.py
# or point it at another instance:
REXTTO_URL=http://192.168.1.10:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py
```

Tabs: **Status · Torrents · Logs · Health** (auto-refresh).

| Keys | Action |
|---|---|
| `1-4` / `Tab` | switch tab |
| `?` | show keyboard help |
| `↑↓` | select a torrent in the Torrents tab |
| `PgUp/PgDn`, `Home/End` | page through torrents and logs |
| `Enter` | open the selected torrent details (`Enter`/`Esc` to go back) |
| `r` | refresh · `c` run a cycle (full/series/movies/comics) · `q` quit |
| `s` / `e` | manual search / recent torrent events |
| `a` | **add a magnet link or `.torrent` URL** |
| `t` | **add a `.torrent` file** (path) |
| `p` / `b` | pause/resume · restart the selected torrent |
| `d` | remove it (asks whether to delete the files) |
| `X` | remove completed torrents that reached their seed limits |
| `k` / `R` | recheck / reannounce |
| `n` / `i` / `u` | toggle no-rename · pin · unpin |
| `L` | set global download/upload limits (KiB/s) |
| Details: `1-4` | general / trackers / files / peers |
| `x` | on the Health tab, clean the trash (asks for confirmation) |
| `/` / `f` | on the Logs tab, filter / toggle follow mode |

### Data and logs

- Default data directory: `data/` (override with `REXTTO_DATA_DIR`).
- Logs in `data/rextto.log`, with 5 MB rotation (active file + 3 backups),
  streamed live in the UI. The viewer supports filtering and follow/pause.

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
cargo build --profile fast      # fast daemon build (target/fast/rexttod)
cargo build --release            # production daemon build
cargo check                      # fastest feedback
cargo test --all-targets         # tests
cargo clean                      # remove build artefacts when needed
```

Development builds stay small and never accumulate forever:

- The `dev`/`test` profiles use line tables only and no debug info for
  dependencies (`Cargo.toml`), which keeps `target/debug` around 1 GB instead of
  ~10 GB.
- Developer-only helper scripts are intentionally local and ignored by Git; they
  are not required by the installer or by a production installation.

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
