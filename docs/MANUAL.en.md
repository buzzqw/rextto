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
  mark-failed, move storage.
- **Download history** lists finished downloads (native completions and
  migrated ones). Columns: name (with a **NAS** badge when the file has been
  archived), type/season/episode, **NAS tag** (folder rule), score, status
  (*Completed*), the **library/NAS path** and the completion time.

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
- **Comics** — GetComics explore + quick add, monitored list, link extraction
  from a post, weekly-pack settings and history with resend/delete/force.

## 7. Configuration

Tabs: **Daemon, Sources, libtorrent, Scores, Rename, Advanced, Notifications,
Paths, Translations**. Unsaved changes are highlighted with a “Save all” bar.

- **Sources** — RSS feed list, indexers (Jackett/Prowlarr) with a *Verify*
  button, FlareSolverr URL + test, web engines, content filters, blacklist.
- **libtorrent** — connections/performance, protocols/trackers, security/proxy,
  RAM disk and ports, speed limits and scheduler; apply/optimise/update check.
  Under *Security, proxy and network* the **VPN killswitch interface** binds
  listening and outgoing traffic to a chosen interface (e.g. `tun0`, `wg0`);
  the list is read from the server, and the change applies after a restart.
- **Scores** — weights per category, custom groups and a live simulator.
- **Rename** — rename enable, template editor with tokens and live preview,
  TMDB/TVDB keys, language, upgrade thresholds, API token. Verification recovers
  the source token from the original release title stored in the database and
  drops empty placeholder blocks (`[]`), so a lost `[WEB-DL]` is restored instead
  of staying `unknown`.
- **Advanced** — free-space guard, trash retention, archive retention, feed
  pages, rename-verify interval, move episodes, debug flags.
- **Paths** — library root, trash, download/temp/RAM-disk dirs, per-tag rules.

## 8. Integrations

- **Trakt / Simkl** — credentials, PIN/OAuth flows, watchlist import, calendar,
  scrobble/mark-watched.
- **Jellyfin / Plex** — server URL + token and a library refresh button.

## 9. Maintenance

- Backup now, clean trash, rescore, scan archives and restart the service.
- **Trash cleanup** from the **Maintenance** toolbar is forced: it removes the
  selected trash content immediately. Cleanup started from the **Trash** panel
  respects the configured retention period.
- **Video duplicates** — *Preview duplicates* and *Clean duplicates* find video
  files clearly inferior (strictly lower resolution) left next to the best
  version in the same folder, e.g. an old 480p next to the new 1080p, and move
  them to the trash. At the **same resolution** the version matching the
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
- **A file is not renamed** — `mediainfo` should be installed (technical tags);
  check *Rename* settings and the TMDB key.
- **FTP backup fails** — use *Test FTP*: it reports the failing step (connection,
  login, remote path, upload, delete) and logs it.
- **Legacy import** — this is a CLI-only operation. Stop the old instance first
  and run `import-legacy.sh`; it copies databases without modifying the source.
- **Logs** — see `data/rextto.log` (rotated at 5 MB) or the in-app log viewer.
