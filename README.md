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
- **Self-contained and self-updating** — one release archive (daemon, web UI,
  bundled libtorrent) that `rexttod --update` installs atomically, keeping
  databases and configuration intact.
- **Multiple sources** — generic RSS feeds, HTML listings (with FlareSolverr
  fallback for Cloudflare), Torznab indexers (Jackett/Prowlarr) and web engines.
- **Quality scoring** — resolution, source, codec, audio, HDR/Dolby Vision,
  groups and configurable weights, with a built-in simulator. One score is used
  consistently for acquisition, searches, upgrades, post-processing, archive
  records and rescoring, including size and movie-subtitle bonuses.
- **Automatic upgrades** — replaces an archived file when a better release
  appears (resolution jump, HDTV→WEB-DL, HDR, repack) beyond a configurable
  score threshold.
- **Series & movies** — TMDB metadata, posters, per-season monitoring, missing
  episode search, calendar, manual search.
- **Torrents** — embedded libtorrent: queue, limits, tags, peers, trackers,
  files, storage moves, seed policy, fastresume, VPN killswitch and restart
  recovery. **Stalled torrents are really paused and excluded from active slots**,
  then resumed/reannounced automatically. Add options include pause, sequential, skip-check, queue-top,
  first/last piece and metadata-only; per-file priorities, web seeds, tracker
  editing, super seeding and `.torrent`/magnet export are in the torrent
  details.
- **Comics** — GetComics monitoring and weekly packs. Every comic added (weekly
  pack, monitored title or *Download Now*) gets the **`Comic`** tag, so the
  *NAS paths per category (tag)* rule routes it to the configured folder.
- **Integrations** — Trakt, Simkl, Jellyfin, Plex, Telegram/e-mail/webhook
  notifications.
- **Web UI** — responsive single-page app, dark/light theme, fully **Italian and
  English** (runtime translation layer with YAML import/export), with log viewer,
  health, charts and maintenance tools.
- **Seen from feed** — every release seen in the sources, grouped by title,
  browsable even for titles you do not monitor.
- **Automatic sanity rules** — hardcoded subtitles and absurd file sizes
  (per-resolution floors derived from a real archive) are rejected; no numbers
  to configure. Rejections are routine and logged at `DEBUG`, so the production
  `INFO` log stays clean.
- **Acquisition tuning** — delay before grabbing with a pending queue and a
  per-title "allow upgrades" switch.
- **Real media inspection** — `ffprobe` results (HDR, codec, audio, languages)
  are stored per file and **used in upgrade comparisons**, so decisions read the
  real archived file, not just its name. New files are probed on completion and
  a scheduled incremental backfill covers the rest; additive only, it never
  downgrades a file.
- **Resilience** — escalating provider backoff with a visible reset list and
  scheduled housekeeping/VACUUM.
- **Automation without clutter** — external event hooks live under
  *Integrations* and watched folders under *Configuration*. Watched files are
  checked for stability and import failures retry with backoff instead of being
  abandoned after a fixed attempt limit.
- **Monotonic library** — Rextto never pulls an older episode outside a
  recognised gap while it already owns later ones (a genuine quality upgrade
  still passes); gap-fill and manual actions always win.
- **Backups** — manual or scheduled (local, FTP, cloud folder, Telegram).
  They include databases and configuration, not media files or torrent state.
- **Light on resources** — one daemon uses about **150 MB RSS** and
  **2–3 % of a single CPU core** while idle on a real library with the
  libtorrent queue active (measured: 9 threads, 155 MB RSS, ~2.7 % CPU over a
  15 s sample, ~43 s CPU used in 17 min uptime). RAM **rises while downloading
  because libtorrent grows** (automatic disk cache and piece buffers) and is
  released once the transfer finishes; the peak stays bounded by the libtorrent
  cache settings, so there is nothing to tune for normal use.

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
version, or let the installed daemon update itself (see *Updating Rextto*).
Existing databases, configuration, downloads, archive paths and logs are kept in
`/var/lib/rextto`; the program and web UI live in `/opt/rextto`.

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

### Updating Rextto

There are two supported update paths. Both install the same release payload and
both leave **data and configuration untouched** — everything in
`/var/lib/rextto` (databases, logs, downloads, archive paths) survives an
update.

| Method | Command | Notes |
|---|---|---|
| Installer | re-run the `install.sh` command above | also refreshes dependencies and the systemd unit |
| Daemon | `sudo rexttod --update` | updates only the payload: `rexttod`, `ui/`, `lib/`, `run.sh` |

The installed program lives in `/opt/rextto`:

```
rexttod     the daemon (rpath $ORIGIN/lib)
ui/         the compiled web interface
lib/        the bundled libtorrent
run.sh      launcher (sets LD_LIBRARY_PATH and REXTTO_UI_DIR)
VERSION     the release marker shown by --version
```

`rexttod --update` downloads `rextto-linux-<arch>.tar.gz`, verifies the
published `.sha256` when the release provides one, and stages the new payload
before touching the current installation. If the download, the checksum or the
extraction fails, the running installation is left as it was; if a swap fails,
the previous files are restored. The service is restarted automatically when
the command runs as root, otherwise the exact `systemctl` command is printed.

```bash
rexttod --version                       # version, build number and libtorrent
sudo rexttod --update                   # latest continuous build
sudo rexttod --update --channel stable  # latest tagged release
sudo rexttod --update --release v0.2.0  # a specific tag
```

The systemd unit is intentionally **not** overwritten by `--update`: local
customisations (user, ports, paths) are preserved. Use the installer to
regenerate the unit. The `VERSION` marker written next to the executable is the
release name (`continuous`, a tag, or `source-main`); the numeric build number
is compiled in and identifies the exact build.

### Standalone Linux package

Every push to `main` (and every release tag) publishes a self-contained
`rextto-linux-x86_64.tar.gz` (with a `.sha256` next to it) containing:

```
rexttod     the daemon, linked with rpath $ORIGIN/lib
ui/         the compiled web interface
lib/        the bundled libtorrent shared library
run.sh      launcher (sets LD_LIBRARY_PATH and REXTTO_UI_DIR)
README.md   quick start and prerequisites
```

Extract it and run it directly, without a compiler:

```bash
mkdir rextto && tar -xzf rextto-linux-x86_64.tar.gz -C rextto
cd rextto
./run.sh --version
REXTTO_DATA_DIR="$PWD/data" REXTTO_DRY_RUN=1 REXTTO_ACTIVE=0 ./run.sh
```

Because the archive bundles libtorrent and the daemon resolves its sibling
`ui/` automatically, no system libtorrent is required. The archive is built by
[`scripts/package-linux.sh`](scripts/package-linux.sh) and is what
`rexttod --update` installs. It needs a modern 64-bit Linux (glibc, libstdc++,
OpenSSL 3, zlib, libzstd); `ffprobe` is optional. Prebuilt assets are published
for **x86_64** only: on `aarch64` the installer falls back to building from
source, and `rexttod --update` reports that no asset is available. For a managed service install
use the installer above.

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
enabled (routine filter and sanity rejections live here too).

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

### Command line

The daemon is normally started by the systemd service. When you run `rexttod`
directly it also accepts these options:

| Option | What it does |
|---|---|
| `-h`, `--help` | print the usage summary |
| `-V`, `--version` | print the installed version, build number and bundled libtorrent |
| `--config <file>` | use a specific configuration file (default `rextto.json`) |
| `--dry-run` | start without real downloads |
| `--update` | download and install the latest payload (see *Updating Rextto*) |

`--update` options:

| Option | What it does |
|---|---|
| `--repo <owner/name>` | GitHub repository to download from (default `buzzqw/rextto`) |
| `--channel <name>` | `continuous` (default) or `stable` |
| `--release <tag>` | install a specific release tag |
| `--install-dir <dir>` | installation directory (default: the binary's directory) |
| `--archive <file>` | install from a local archive instead of downloading |
| `--force` | reinstall even if the version is unchanged (only meaningful with `--release`: the rolling `continuous` and `stable` channels always fetch the latest asset) |
| `--no-restart` | do not restart `rextto.service` after installing |

Examples:

```bash
rexttod --version                          # what is installed now
sudo rexttod --update                      # latest continuous build
sudo rexttod --update --channel stable     # latest tagged release
sudo rexttod --update --release v0.2.0     # a specific tag
rexttod --update --install-dir /srv/rextto --no-restart
rexttod --update --archive ./rextto-linux-x86_64.tar.gz   # offline
```

When testing an update without touching a real installation, combine
`--install-dir` with a throwaway directory and `--no-restart`; `--archive`
avoids the network entirely.

### Where to find the details

The README is the practical overview; the [manual](docs/MANUAL.en.md) documents
every screen. Quick index:

| Topic | README | Manual |
|---|---|---|
| Install and service | *Installation* | [1. First start](docs/MANUAL.en.md#1-first-start) |
| Update, version, package | *Updating Rextto*, *Command line* | [1. First start](docs/MANUAL.en.md#1-first-start) |
| First run and modes | *First run* | [1. First start](docs/MANUAL.en.md#1-first-start) |
| Dashboard, cycles, stats | *Cycles and downloads* | [2. Dashboard](docs/MANUAL.en.md#2-dashboard) |
| Torrents, stalled, history | *Cycles and downloads* | [3. Downloads](docs/MANUAL.en.md#3-downloads) |
| Series, episodes, gaps | *Add series and movies* | [4. Series](docs/MANUAL.en.md#4-series) |
| Movies | *Add series and movies* | [5. Movies](docs/MANUAL.en.md#5-movies) |
| Explore, Archive, Comics | *UI sections* | [6. Explore, Archive, Comics](docs/MANUAL.en.md#6-explore-archive-comics) |
| Sources, scoring, renaming | *Configure the sources* | [7. Configuration](docs/MANUAL.en.md#7-configuration) |
| Trakt, Jellyfin, hooks | *UI sections* | [8. Integrations](docs/MANUAL.en.md#8-integrations) |
| Backups, duplicates, DB | *UI sections* | [9. Maintenance](docs/MANUAL.en.md#9-maintenance) |
| Health, logs, charts | *Reading the logs* | [10. Health, Logs, Charts](docs/MANUAL.en.md#10-health-logs-charts) |
| Notifications | *UI sections* | [11. Notifications](docs/MANUAL.en.md#11-notifications) |
| Common problems | — | [12. Troubleshooting](docs/MANUAL.en.md#12-troubleshooting) |

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
| `REXTTO_UI_DIR` | Directory of the compiled web UI (packaged installs) |
| `REXTTO_INSTALL_DIR` | Installation directory used by `--update` |
| `REXTTO_REPO` | GitHub repository used by `--update` (default `buzzqw/rextto`) |
| `REXTTO_ACTIVE` | `1` enables the acquisition cycles |
| `REXTTO_DRY_RUN` | `1` disables real downloads |
| `REXTTO_API_TOKEN` | Optional bearer token for the API/UI |
| `RUST_LOG` | Tracing filter (default `rextto=info`) |
| `REXTTO_URL` | TUI client: daemon base URL (default `http://127.0.0.1:5000`) |
| `REXTTO_CHANNEL` / `REXTTO_VERSION` | Installer/updater channel (`continuous`, `stable`) or a specific release tag |
| `REXTTO_PORT` / `REXTTO_ENGINE_PORT` | Installer: UI/API port (default `5000`) and engine port (default `8889`) |
| `REXTTO_USER` | Installer: service account to create/use (default `rextto`) |
| `REXTTO_SKIP_PACKAGES` | Installer: `1` skips system package installation |
| `REXTTO_SKIP_LIBTORRENT_BUILD` | Installer: `1` skips building libtorrent from source |
| `REXTTO_SOURCE_REF` | Installer fallback: GitHub ref to build from (default `main`) |

## Development

### Build, test and run

```bash
cargo build --profile fast      # fast daemon build (target/fast/rexttod)
cargo build --release            # production daemon build
cargo check                      # fastest feedback
cargo test --all-targets         # tests
cargo clean                      # remove build artefacts when needed
```

The web UI is a separate Leptos/WASM workspace:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
```

`scripts/acceptance.sh` runs the isolated smoke test described in
[`docs/ACCEPTANCE.md`](docs/ACCEPTANCE.md): it uses a temporary data directory
and dedicated ports in dry-run, so it never touches a real installation.

### Packaging

[`scripts/package-linux.sh`](scripts/package-linux.sh) builds the standalone
archive that the installer and `rexttod --update` consume:

```bash
cargo build --release --locked
cargo leptos --manifest-path ui/Cargo.toml build --release --frontend-only
scripts/package-linux.sh --binary target/release/rexttod --ui ui/target/site
# -> rextto-linux-x86_64.tar.gz + rextto-linux-x86_64.tar.gz.sha256
```

The archive contains `rexttod`, the compiled `ui/`, the bundled libtorrent in
`lib/`, a `run.sh` launcher and a short README. The daemon is linked with an
`$ORIGIN/lib` rpath and looks for a sibling `ui/` directory, so it runs straight
from the extracted archive; `install.sh` copies the same payload into
`/opt/rextto`.

To exercise the updater locally without replacing your checkout binary, point it
at a throwaway directory and use the freshly built archive:

```bash
scripts/package-linux.sh --output /tmp/rextto-linux-x86_64.tar.gz
mkdir -p /tmp/rextto-install
target/release/rexttod --update --archive /tmp/rextto-linux-x86_64.tar.gz \
  --install-dir /tmp/rextto-install --no-restart
/tmp/rextto-install/rexttod --version
```

### How the pieces fit

| Component / resource | Role | Functions it powers |
|---|---|---|
| Rust + Tokio + Axum | daemon runtime and HTTP server | cycles, REST API, SSE log stream, static UI |
| Leptos + WASM (`ui/`) | single-page front-end | dashboard, library screens, settings, bilingual UI |
| SQLite (rusqlite, bundled) | local persistence | series/episodes, movies, archive, comics, config, cycle stats, torrent metadata |
| libtorrent (`native/libtorrent_bridge.cpp`, `src/libtorrent.rs`) | embedded BitTorrent engine | queue and limits, seeding policy, trackers, files, peers, fastresume, VPN killswitch |
| reqwest + scraper + quick-xml (`src/rss.rs`, `src/websearch.rs`) | source acquisition | RSS/HTML listings, Torznab indexers, web engines, FlareSolverr fallback |
| TMDB / TVDB (`src/tmdb.rs`, `src/tvdb.rs`) | metadata providers | posters, seasons, episode dates, discovery |
| ffprobe / MediaInfo (`src/mediainfo.rs`) | real media inspection | codec/HDR/audio/language data that feeds upgrade comparisons |
| Trakt / Simkl / Jellyfin / Plex (`src/integrations.rs`) | media-server integrations | watchlist, calendar, scrobble, library refresh |
| Telegram / SMTP / webhook (`src/notifier.rs`) | notifications | completion/error alerts, HMAC-signed webhooks |
| FTP / cloud / Telegram (`src/backup.rs`, suppaftp) | scheduled backups | database and configuration snapshots |
| zip, flate2, sha1/hmac/sha2 | utilities | archive handling, hashing, webhook signing |
| parser + rules + scoring (`src/parser.rs`, `src/rules.rs`, `src/config.rs`) | domain logic | release parsing, quality scoring, sanity checks, upgrades |
| CLI + self-update (`src/cli.rs`, `src/update.rs`) | operations | `--version`/`--help`, release download with checksum, atomic payload swap and rollback |
| installer + packaging + systemd | operations | source/release install, service unit, standalone archive |
| legacy importer (`src/importer.rs`) | migration CLI | one-off import of an older installation's databases |

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
