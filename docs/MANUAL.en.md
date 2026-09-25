# Rextto — User Manual

This manual covers the everyday use of Rextto through its web interface. The
UI is available in **English** and **Italian** (header language switch); this
document describes the English labels, the Italian ones are in
[`MANUAL.it.md`](MANUAL.it.md).

- [1. First start](#1-first-start)
- [2. Dashboard](#2-dashboard)
- [3. Downloads](#3-downloads)
- [4. Series](#4-series)
- [5. Movies](#5-movies)
- [6. Explore, Archive, Comics](#6-explore-archive-comics)
- [7. Configuration](#7-configuration)
- [8. Integrations](#8-integrations)
- [9. Maintenance](#9-maintenance)
- [10. Health, Logs, Charts](#10-health-logs-charts)
- [11. Notifications](#11-notifications)
- [12. Troubleshooting](#12-troubleshooting)

---

## 1. First start

Rextto runs as a single service. Open the web UI at `http://<host>:5000`.

- If no data directory exists yet, complete the initial setup wizard.
- **Active vs dry-run**: dry-run never starts real downloads; enable *active
  mode* in *Configuration → Daemon* only when you are ready.
- Add series/movies from **Explore** (TMDB) or **Series / Movies → Add**.

### Command line and updates

The daemon is normally run by the systemd service. When invoked directly it
understands:

- `rexttod --version` — print the installed version, the release marker and the
  bundled libtorrent;
- `rexttod --help` — usage summary;
- `rexttod --update` — download and install the latest daemon and web UI;
- `rexttod --config <file>` and `rexttod --dry-run` — as used by the service and
  by local tests.

`--update` downloads the official `rextto-linux-<arch>.tar.gz`, verifies its
checksum when published, and swaps in the executable and UI atomically. It never
touches `REXTTO_DATA_DIR`. Options: `--channel stable`, `--release <tag>`,
`--install-dir <dir>`, `--archive <file>` (offline), `--no-restart`, `--force`.
It restarts `rextto.service` when run as root. The same payload can be used
manually as a standalone package (see the README, *Standalone Linux package*).

## 2. Dashboard

- **Manual global search** — searches archive + indexers + web engines.
- **Cycle buttons** — run a full cycle or a single domain (Series, Movies,
  Comics) or a backup now.
- **Stat cards** — configured series/movies, downloaded files, free space,
  archive magnets, session torrents, seen-from-feed groups.
- **Network and active downloads** — CPU/RAM plus a live network sparkline.
- **Consumption and disks**, **upcoming releases**, **last downloads**,
  **recent activity** and **latest finds from sources**.

## 3. Downloads

- **Add** a magnet/.torrent URL or upload a `.torrent` file; optionally set a
  save path, “Download now” and “Do not rename”.
- **Session cards**: download/upload rate, torrent and peer counts.
- **Tag filter** and **temporary speed limits** (DL/UL for N minutes).
- **Table columns**: Name, Status, Progress, ↓, ↑, ETA, Peers, Ratio — click a
  header to sort. **Bulk actions**: pause, resume, recheck, remove.
- **Row actions**: pause/resume, recheck, details, remove.
- **Details** tabs: General, Tracker, Content, Peers, Limits, Storage; includes
  copy magnet, per-torrent limits/seed-days, reannounce, pin, restart,
  mark-failed, move storage. **General** also offers **Export .torrent**,
  **Super seeding** and web-seed add/remove; **Tracker** lets you edit the whole
  list (`tier|url` per line); **Content** sets the **per-file priority**
  (Skip/Normal/High/Maximum).
- **Download history** lists finished downloads (native completions and
  migrated ones). Columns: name (with a **NAS** badge when the file has been
  archived), type/season/episode, **NAS tag** (folder rule), score, status
  (*Completed*), the **library/NAS path** and the completion time.

### Stalled torrents

When a torrent stops increasing its completed-byte count for the configured
period, it enters **stalled** state. Rextto pauses it in libtorrent too, so it no
longer occupies an active slot. It remains in the session and is resumed and
reannounced on the next retry. The values are under *Configuration → libtorrent*:

- **Consider stalled after** — 60 minutes by default;
- **Stalled retry** — 60 minutes by default;
- **Stalled removal** — 20160 minutes (14 days) by default, `0` = never.

Peers without byte progress do not reset the timer. Look for `DOWNLOAD STALLED`,
`stalled torrent resumed and reannounced`, and, only after the final limit,
`DOWNLOAD FAILED — stalled` in the logs.

## 4. Series

Add a series via TMDB search or manually (title, quality, languages, seasons,
aliases, exclusions, NAS path, subtitles, timeframe).

- The list shows Name, Seasons, Quality, Language, Episodes (downloaded/total),
  Completeness, Last download; filter it with the search box.
- Bulk actions: set language, delete selected.
- **Series detail**: poster/plot/cast, badges (year, network, status,
  completeness), and, if configured, a **“disabled seasons”** badge.
- Actions: search missing, scan archive, refresh from TMDB, rename preview /
  execute, edit.
- **Episodes**: per-season accordion; per episode you can search, copy magnet,
  ignore/reactivate, force, re-download or delete; the manual search marks
  results already present in the feed.

## 5. Movies

- Tabs **Monitored / Downloaded**; sortable columns (name, year, quality,
  language).
- The editor supports quality, base language, subtitles, exclusions and up to
  **three required languages** with a per-row “required” flag.
- **Movie detail**: poster, plot, cast, edit, re-download, **Search now** and a
  “best matches” table from the sources.

## 6. Explore, Archive, Comics

- **Explore** — TMDB trending/today/popular/top-rated/now playing/upcoming, TMDB
  search, generic release search, add-to-library.
- **Archive** — full-text search of past releases with pagination, batch queue,
  copy magnet, delete; **Series/Movies from feed** tabs.
- **Seen from feed** — every release seen in the sources, grouped by title
  (movies/series) with count, best resolution and score; expand a group to
  queue a single release. Populated by each cycle, even for unmonitored titles.
- **Comics** — in **Add comic**, enter a title and click **Find**; choose the
  exact GetComics result and confirm. Rextto saves the selected post, tag,
  cover and metadata, then uses that post for downloading instead of replacing
  it with a similarly named issue. The monitored list, post link extraction,
  weekly-pack settings and resend/delete/force history are also available.

## 7. Configuration

Tabs: **Daemon, Sources, libtorrent, Scores, Rename, Advanced, Acquisition,
Notifications, Paths, Translations**. Unsaved changes are highlighted with a
“Save all” bar.

- **Sources** — RSS feed list, indexers (Jackett/Prowlarr) with a *Verify*
  button, FlareSolverr URL + test, web engines, content filters, blacklist.
- **libtorrent** — connections/performance, protocols/trackers, security/proxy,
  RAM disk and ports, speed limits and scheduler; apply/optimise/update check.
  The RAM disk section lists available `tmpfs`/`ramfs` mounts and lets you
  choose one. If no path is configured, its button creates and configures
  `/dev/shm/rextto`. The contents of `/dev/shm` are temporary and are lost
  when the machine reboots. Choosing a path automatically calculates the
  maximum torrent size, free-space margin and minimum free space; the values
  remain editable. The **Test ports** button also checks local port binding;
  it does not replace router/firewall port-forwarding verification.
  Under *Security, proxy and network* the **VPN killswitch interface** binds
  listening and outgoing traffic to a chosen interface (e.g. `tun0`, `wg0`);
  the list is read from the server, and the change applies after a restart.
- **Scores** — weights per category, custom groups and a live simulator. One
  effective score is used for acquisition, searches, upgrades, post-processing,
  archive records and rescoring; it also includes the size bonus and, for movies,
  the preferred-subtitle bonus. Use **Maintenance → Rescore** after changing
  weights.
- **Rename** — rename enable, template editor with tokens and live preview,
  TMDB/TVDB keys, language, upgrade thresholds, API token. Verification recovers
  the source token from the original release title stored in the database and
  drops empty placeholder blocks (`[]`), so a lost `[WEB-DL]` is restored instead
  of staying `unknown`.
- **Advanced** — free-space guard, trash retention, archive retention, feed
  pages, rename-verify interval, move episodes, debug flags.
- **Acquisition** — download delay for series/movies (with a high-score
  bypass), housekeeping interval, **Watched folders** (Rextto scans the chosen
  directories, waits for two stable observations, and adds copied `.torrent`/
  `.magnet` files; import failures retry with backoff until they succeed, then
  files are removed or renamed `.imported`), **Sources in backoff**
  with level, deadline, last error and per-source reset, and the automatic
  **MediaInfo backfill** (configurable files-per-run and interval), plus a
  Maintenance button for an immediate scan.
  **Housekeeping is not Archive cleanup**: it keeps the database tidy, but it
  does not delete library files, completed downloads, or releases listed in
   the Archive. *Search-cycle statistics kept* concerns only the counters for
   each cycle—releases scanned, candidates, started downloads, filled gaps, and
   errors: with `200`, cycle 201 removes the oldest cycle's counters, not the
   downloaded release. *Feed-seen entries* removes only historical feed rows
   older than the selected number of days (`0` = no cleanup); *Download history*
   in the **Downloads** section removes only rows for torrents already removed
   (`0` = keep), without affecting the Archive. Each run also
  automatically removes torrents in error older than 7 days, gap logs older
  than 30 days, upgrade backups older than 30 days, and expired source
  backoffs. The databases are compacted with `VACUUM` afterwards. Archive
  retention is separate and is configured under **Configuration → Advanced**.
- **Paths** — library root, trash, download/temp/RAM-disk dirs, per-tag rules.
   A selected RAM-disk path remains configured, but a directory created under
   `/dev/shm` must be recreated after a reboot.
 - **Translations** — advanced panel to export saved Italian or English
   translations as YAML, edit them, and import them again. The format is a
   `key: value` map, for example `"Testa porte": "Test ports"`.
   Import updates or adds keys present in the file and does not delete missing
   keys. It does not change the active language; use the selector at the top.

Release sanity checks are **automatic** and not configurable: hardcoded
subtitles (`HC`) and absurd sizes (a per-resolution floor derived from a real
archive) are refused, with an `INFO` log line reporting the title, resolution,
size and reason for monitored titles; unrelated releases are discarded without
logging. The older-episode protection is fixed too: Rextto never
re-downloads an earlier episode outside a recognised gap while it already owns
later ones (gap-fill and manual actions always pass). To freeze a title, use
**Allow upgrades** in the series/movie editor.

The real file data (`ffprobe`: HDR, codec, audio, languages) is stored per
episode/movie and **used in upgrade comparisons**, so the archived file is read
as it really is, not just from its name. It is additive (never downgrades a
file). New files are probed on completion; the rest are covered by the scheduled
incremental **MediaInfo backfill** (or the Maintenance button for an immediate
scan). If `ffprobe` is missing, the backfill pauses by itself.

## 8. Integrations

- **Trakt / Simkl** — credentials, PIN/OAuth flows, watchlist import, calendar,
  scrobble/mark-watched.
- **Jellyfin / Plex** — server URL + token and a library refresh button.
- **Event hooks** — run an external program on Rextto events
  (`download_started`, `torrent_completed`, `season_pack_completed`,
  `torrent_error`, …). Fields accept placeholders such as `{title}`, `{hash}`,
  `{path}`, `{series}`, `{episode}`; the same values are exported as `REXTTO_*`
  environment variables. Programs run without a shell and use a 60-second
  default timeout (maximum 24 hours; `0` also means the default).

## 9. Maintenance

- Backup now, clean trash, rescore, scan archives, **Refresh MediaInfo**
  (probes archived files without data via `ffprobe` and stores it) and restart
  the service.
- **Trash cleanup** from the **Maintenance** toolbar is forced: it removes the
  selected trash content immediately. Cleanup started from the **Trash** panel
  respects the configured retention period.
- **Video duplicates** — *Preview duplicates* and *Clean duplicates* find video
  files clearly inferior (strictly lower resolution) left next to the best
  version in the same folder, e.g. an old 480p next to the new 1080p, and move
  them to the trash. The check uses the filename and technical data declared in
  the name; it does not compare file contents or calculate hashes. At the
  **same resolution** the version matching the
  preferred language (*Configuration → Rename → default language*) is kept and a
  duplicate that explicitly declares a different language is moved to trash;
  files with no language tag are left untouched. Files of torrents still in the
  session are protected, and emptied sub-folders are removed. The rename
  verification also performs this cleanup automatically, and upgrade cleanup now
  also honours the "hard" upgrade reasons (resolution/source/HDR/repack).
- **Restore source** — *Restore source* remounts the `[Source]` token (WEB-DL,
  HDTV, BluRay…) in archived names that lost it, recovering it from the original
  release title in the database. It never invents a source: unknown stays
  untouched. Preview first, then execute; no re-download is involved.
 - **Database**: prune by cycles/error age, **seen-from-feed retention** (days; 0
   keeps everything), keyword prune with a list of the matching rows, and
   **VACUUM / ANALYZE** across all databases.
   Seen-from-feed cleanup removes only historical feed rows, not files or
   downloads; it also applies the standard cleanup of the last 50 cycles and
   torrent errors older than 7 days.
- **Backups**: retention, schedule (manual, every N hours or a fixed daily
  HH:MM), FTP host/user/path + **Test FTP** (checks connection, path and a probe
  upload), cloud/sync folder copy, Telegram delivery, list of available backups.
  A snapshot contains the databases and configuration; media files and torrent
  session state are not included.

## 10. Health, Logs, Charts

- **Health** — process/system status, runtime metrics (3 columns), Rextto
  service state, **indexer reachability**, path permissions, source health,
  recent errors, disks.
- **Logs** — live SSE stream with text filter, line count and follow/pause.
  Lines are English and explicitly formatted as `date time  LEVEL [component]
  message · key: value`, with highlighted keywords (NAS, download, sources,
  filters, errors). Every cycle prints a **SOURCE REPORT** with each source's
  outcome, the filter decisions, and the download events (start, metadata,
  NAS move, completion). Torrent messages always include the readable name or
  title; the hash is only a technical correlation field for errors.
- **Charts** — CPU/RAM/download/upload/disk/ram-disk sparklines and daily
  consumption.
- **Activity** — recent torrent events and downloads.

## 11. Notifications

Configure Telegram, e-mail (SMTP) or a webhook (with HMAC secret) and send a
test. Completion notifications include size, download time and average speed.

## 12. Troubleshooting

- **A source is unreachable** — check *Configuration → Sources → Verify* and the
  sources health panel; Cloudflare-protected sites need a working FlareSolverr.
- **Nothing downloads** — confirm *active mode*, that the series/movie is
  enabled, and check the quality/language filters and the free-space guard.
- **A torrent is stalled** — check the three values under *Configuration →
  libtorrent*. It is intentionally paused and excluded from active slots; wait
  for the retry or use **Resume/Restart** manually.
- **A watched-folder file is not imported** — keep the `.torrent` or `.magnet`
  extension. Rextto waits for size and timestamp stability, then retries import
  errors automatically; check the watcher log.
- **A file is not renamed** — `mediainfo` should be installed (technical tags);
  check *Rename* settings and the TMDB key.
- **FTP backup fails** — use *Test FTP*: it reports the failing step (connection,
  login, remote path, upload, delete) and logs it.
- **Logs** — see `data/rextto.log` (rotated at 5 MB) or the in-app log viewer.
