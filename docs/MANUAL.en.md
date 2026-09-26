# Rextto — User Manual

This manual covers the everyday use of Rextto through its web interface. The
UI is available in **English** and **Italian** (header language switch); this
document describes the English labels, the Italian ones are in
[`MANUAL.it.md`](MANUAL.it.md).

## How to use this guide

Rextto is a single daemon: it searches sources, evaluates releases, manages
libtorrent, renames files and archives them. Normal operation does not require
separate orchestrator processes.

The recommended path for a new installation is:

1. configure paths and sources;
2. keep the daemon in **dry-run**;
3. add one test title;
4. run a manual search or cycle;
5. inspect Health and Logs;
6. enable active mode only after checking the results.

This manual distinguishes between:

- **manual search**: inspect results and queue one choice;
- **automatic cycle**: search monitored titles and decide what to download;
- **archive**: files already imported or stored in the library;
- **torrent session**: downloads still managed by libtorrent.

### First-run checklist

Before enabling real downloads, verify that:

- `http://<host>:5000` is reachable;
- *Health* reports no path-permission or free-space problem;
- the temporary directory is writable;
- the library/NAS directory is mounted and writable by the service user;
- at least one source succeeds in *Verify*;
- a manual search returns plausible releases;
- dry-run produced no unexpected errors.

If you use a NAS, first create a test file in the destination as the same user
that runs `rextto.service`. A path visible to your shell user may not be visible
to the systemd service user.

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

### Recommended first cycle

1. In *Configuration → Paths*, check the download, temporary, library and trash
   directories.
2. In *Configuration → Sources*, add one working source and click **Verify**.
   Add the others only after the first one works.
3. Keep real downloads disabled and add one series with one season or one test
   movie.
4. From the Dashboard, run a cycle for the relevant domain.
5. Open *Health* and *Logs*: you should see the cycle, queried sources and the
   reason why a release was accepted or rejected.
6. Run a manual search and inspect one result with **Why not this one?**.
7. When paths, filters and results are correct, enable *Active mode*.

Do not change quality, paths and sources at the same time when troubleshooting:
one change at a time makes the cause reproducible.

### Paths and responsibilities

Rextto uses separate paths for separate roles:

| Path | Use | May it be temporary? |
|---|---|---|
| Download | data for active torrents | no, while a torrent is active |
| Temporary/incomplete | metadata and incomplete data | yes, but it must be writable |
| Library/NAS | final archived files | no |
| Trash | files removed during upgrades/cleanup | yes, according to retention |
| Watched folder | `.torrent`/`.magnet` files to import | yes, but not while copying |

Do not use the temporary directory as the final library and do not delete files
from an active torrent manually: use the torrent-session actions.

### Command line and updates

The daemon is normally run by the systemd service. When invoked directly it
understands:

- `rexttod --version` — installed version, build number, release marker and
  bundled libtorrent;
- `rexttod --help` — usage summary;
- `rexttod --update` — download and install the latest payload;
- `rexttod --config <file>` and `rexttod --dry-run` — used by the service and for
  local tests.

**Updating.** The installer and `rexttod --update` install the same release
payload. `--update` downloads `rextto-linux-<arch>.tar.gz`, verifies the
published `.sha256` when present, stages the files, then swaps the executable,
the web UI, the bundled `lib/` and `run.sh` with atomic renames. Data and
configuration in `REXTTO_DATA_DIR` (default `/var/lib/rextto`) are never touched:
a failed download, checksum or extraction leaves the running installation
untouched, and a failed swap is rolled back. The `VERSION` marker next to the
executable is updated and shown by `--version`.

- `--channel stable` / `--release <tag>` — choose the release to install;
- `--install-dir <dir>` — install elsewhere (default: the binary's directory);
- `--archive <file>` — install from a local archive (offline);
- `--force` — reinstall even if the version is unchanged;
- `--no-restart` — do not restart `rextto.service`.

Run it as root to restart `rextto.service` automatically; otherwise the exact
`systemctl` command is printed. The systemd unit is not overwritten, so local
customisations (user, ports, paths) are preserved. The same payload is the
standalone Linux package described in the README (*Standalone Linux package*).

## 2. Dashboard

- **Manual global search** — searches archive + indexers + web engines.
- **Cycle buttons** — run a full cycle or a single domain (Series, Movies,
  Comics) or a backup now.
- **Stat cards** — configured series/movies, downloaded files, free space,
  archive magnets, session torrents, seen-from-feed groups.
- **Network and active downloads** — CPU/RAM plus a live network sparkline.
- **Consumption and disks**, **upcoming releases**, **last downloads**,
  **recent activity** and **latest finds from sources**.

### Manual search from the Dashboard

Manual search is useful for understanding what Rextto sees before changing a
configuration or starting a download:

1. enter a title or technical query;
2. wait for sources to finish, or for slow-source timeouts to appear;
3. compare title, source, quality, size and seed/peer counts;
4. use **Filter loaded results** to narrow the list locally;
5. open **Why not this one?** on interesting results;
6. click **Queue** only after checking the reason and archive comparison.

The local filter works on results already received, including title, source and
technical quality fields. Typing in it does not query indexers again or change
the original search query.

Manual search may show releases that are not eligible so they can be inspected.
Visibility does not mean that the automatic cycle would download the release.

### How to read a cycle

A normal cycle goes through search, filters, archive comparison, selection and
queueing. The number of releases found is not the number of downloads: a release
may be excluded because it is unmonitored, too old, blocked, already present or
inferior to the archived file.

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
- When you use **Move storage**, the log records the request, destination and
  command acceptance; the final outcome is written when libtorrent completes or
  rejects the move. With **Check**, the log distinguishes command start from
  completion and includes state and verified bytes.
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

### States and recommended actions

| State | Meaning | Recommended action |
|---|---|---|
| Queued | registered but not started yet | wait for the cycle/session |
| Downloading | transfer in progress | check speed and peers |
| Stalled | no real byte progress | wait for the automatic retry |
| Seeding | download complete, seeding is active | leave it or remove it according to policy |
| Error | the torrent reported an error | read the reason before removing it |
| Archived/NAS | the final file was copied to the library | check the path; do not delete an active source manually |

**Remove** acts on the torrent session and may ask whether to delete files.
**Clean completed** is more selective: it removes torrents that reached their
seeding limits. Removing a torrent from the session does not necessarily remove
the file from the library.

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
  The rename preview lists the *Old → New* names and has a **Force rename**
  button: it reprocesses files that already pass the quick check, recomputing
  the exact target name from the template and the TMDB/TVDB metadata, useful
  when a file looks correct but does not match the configured template.
- **Episodes**: per-season accordion; per episode you can search, copy magnet,
  ignore/reactivate, force, re-download or delete; the manual search marks
  results already present in the feed.

### Adding and configuring a series

For a new series, TMDB search is the safest method:

1. open **Explore**, search for the title and select the correct result;
2. check title, year, network and poster;
3. choose quality, language, subtitles and monitored seasons;
4. set a per-series archive path only if you do not want the global path;
5. save in dry-run and inspect the detail page.

The most important fields are:

| Field | Effect |
|---|---|
| Quality | restricts acceptable releases and contributes to the score |
| Language | requires the configured language when it is recognisable in the title |
| Subtitles | controls preference without accepting hardcoded subtitles |
| Seasons | decides which seasons are monitored; ranges are supported |
| Aliases | links alternate names to the same series |
| Exclusions | words in a title that must block the release |
| Allow upgrades | enables or blocks replacement of archived files |

Leave future seasons enabled if you want calendar and missing searches to keep
working. Use *Ignore* only for episodes that should no longer be searched;
reactivating them makes them candidates again in later cycles.

### Missing episodes and packs

From **Search missing** or an episode detail:

1. check that the season is monitored;
2. search for the single episode or the series domain;
3. compare individual releases and season packs;
4. if an episode exists on disk but not in the database, scan the archive;
5. use **Force** only for an intentional manual action.

Rextto normally avoids downloading older episodes when later episodes already
exist, unless this is a recognised gap or a genuine upgrade. A season pack can
fill several episodes, but archive comparison still happens episode by episode.

## 5. Movies

- Tabs **Monitored / Downloaded**; sortable columns (name, year, quality,
  language).
- The editor supports quality, base language, subtitles, exclusions and up to
  **three required languages** with a per-row “required” flag.
- **Movie detail**: poster, plot, cast, edit, re-download, **Search now** and a
  “best matches” table from the sources.

### Adding and selecting a movie

1. find the movie in **Explore** and check year and original title;
2. add it to the library;
3. set quality, language, subtitles and exclusions;
4. use **Search now** to inspect results without waiting for a cycle;
5. open **Why not this one?** before queueing a borderline release.

Movies are identified by title and year when available. Avoid creating duplicates
with different spellings: correct the monitored movie metadata instead of adding
it again.

Required languages can be optional or mandatory. A mandatory language must be
present for the release to be accepted; an optional preference affects selection
and score without automatically blocking the release.

## 6. Explore, Archive, Comics

- **Explore** — TMDB trending/today/popular/top-rated/now playing/upcoming, TMDB
  search, generic release search, add-to-library.
- **Archive** — full-text search of past releases with pagination, batch queue,
  copy magnet, delete; **Series/Movies from feed** tabs.
- **Seen from feed** — every release seen in the sources, grouped by title
  (movies/series) with count, best resolution and score; expand a group to
  queue a single release. Populated by each cycle, even for unmonitored titles.
- **Why not this one?** — search results include a read-only explanation for a
  release. It shows the decision, score and score components, passed or blocking
  rules, source filters, quality/language/subtitle checks, blocklist, active
  downloads and comparison with the archived file. It does not queue the
  release, create placeholders or modify the database. The result covers the
  candidate checks; cycle selection can also depend on gap filling, smart
  episode, delay, free space and other candidates.
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
  For Jackett, use its base URL, for example `http://host:9117`, together with
  the API key. Rextto builds the Torznab endpoint
  `/api/v2.0/indexers/all/results/torznab/api`. The health check uses `t=caps`
  with the same key, so it checks the API rather than only Jackett's home page.
  Results may identify the source as `jackett:TrackerName`.
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
    The interface translates strings at runtime and falls back to the Italian
    source when a translation is missing.

### Configuring a Torznab indexer

For Jackett:

1. create or verify at least one indexer inside Jackett;
2. copy the API key from the Jackett page;
3. in Rextto click **+ Jackett**;
4. enter the base URL, for example `http://jackett:9117`, and the API key;
5. save and click **Verify**;
6. check *Health → API status*.

Rextto automatically uses `/api/v2.0/indexers/all/results/torznab/api` for
Jackett. Do not append that path when using the base URL. If you use a custom
full Torznab endpoint, keep it configured as an explicit endpoint. Prowlarr uses
its own search endpoint and normally uses port `9696`.

An indexer can be reachable but unhealthy: Jackett may return HTTP 200 with a
Torznab error caused by a wrong API key or no enabled indexer. In that case
*Health* shows **API error** and the detail; cycle logs also show source backoff
when applicable.

### Score, filters and “Why not this one?”

The main checks are independent:

1. monitored title;
2. blocklist and duplicates;
3. global filters and source filter;
4. release sanity, including hardcoded subtitles and minimum size;
5. title quality, language, subtitles and exclusions;
6. comparison with archived files and active downloads.

The score helps choose between eligible candidates; it does not make a release
valid when a blocking filter fails. In **Why not this one?**:

- **Passed** means that check succeeded;
- **Blocking** is sufficient reason to reject the candidate;
- **Informational** depends on the complete cycle context;
- archive comparison says whether this is a first download or a possible upgrade.

The explanation is diagnostic, not a reservation: opening it does not create
torrent rows, queue a magnet or change configuration.

### Delays and source backoff

The acquisition delay can hold a release before starting so a better result can
appear. A high score may bypass the delay according to configuration. Source
backoff is different: repeated errors temporarily prevent new requests to the
problematic source.

In *Configuration → Acquisition* you can see level, deadline and last error. Use
the per-source reset after fixing the cause; do not use it to hide a wrong API
key, or backoff will start again.

### Configuring a NAS safely

For a NAS library:

1. mount the filesystem before Rextto starts;
2. grant read/write access to the service user;
3. set a conservative minimum-free-space guard;
4. scan the archive after copying existing files;
5. check the actual path in history after the first completion.

If the NAS is not mounted, do not temporarily replace the path with a local root
without understanding the effect: files may be archived in the wrong place. Fix
the mount and let the download wait instead.

Release sanity checks are **automatic** and not configurable: hardcoded
subtitles (`HC`) and absurd sizes (a per-resolution floor derived from a real
archive) are refused. Rejections are routine and are logged at `DEBUG` with the
title, resolution, size and reason, so the production `INFO` log stays clean;
enable debug logging to inspect them. The older-episode protection is fixed too:
Rextto never re-downloads an earlier episode outside a recognised gap while it
already owns later ones (gap-fill and manual actions always pass). To freeze a
title, use **Allow upgrades** in the series/movie editor.

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

### Connecting an external service

Integrations are not required to download and archive media. Enable them one at
a time and use the test button whenever one is available:

- **Trakt/Simkl**: complete the PIN/OAuth flow, verify that the correct account
  is shown, and only then enable watchlist import or scrobbling;
- **Jellyfin/Plex**: enter a URL reachable by the daemon and a token with the
  minimum required permissions, then test a library refresh;
- **Hooks**: first configure a harmless program that writes a log, verify its
  placeholders, and only then connect it to automation or notification scripts.

Hooks run without a shell: pipes, redirections and operators such as `&&` are not
interpreted. If you need them, create a real executable script and pass values
through placeholders or `REXTTO_*` variables. Do not put tokens or passwords in
arguments if the command may be recorded in system logs.

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

### Backups: what they protect and what they do not

A Rextto backup protects databases and configuration. It does not contain videos,
media archives or the complete libtorrent session state. Before restoring:

1. stop or pause automatic cycles;
2. keep a copy of the current database;
3. verify the backup date and size;
4. restore only to a compatible installation;
5. check paths and permissions before enabling downloads again.

Restoring a database does not automatically move media files. If the library was
moved, fix paths or scan the archive before starting upgrades and missing searches.

### Cleanup and irreversible operations

Use previews first whenever available. In particular:

- *Preview duplicates* shows what would be moved to trash;
- *Rename preview* shows old and new names;
- *Database cleanup* affects historical rows, not the library;
- trash cleanup can permanently delete files;
- removing a torrent may ask whether to delete its data.

Do not confuse **Trash**, **Archive**, **Download history** and the **torrent
session**: they are different sets, and cleaning one does not automatically clean
the others.

## 10. Health, Logs, Charts

- **Health** — process/system status, runtime metrics (3 columns), Rextto
  service state, **indexer reachability**, path permissions, source health,
  recent errors, disks. For Jackett the check uses the Torznab `caps` endpoint,
  so an API-key or configured-indexer problem is distinguished from simple host
  reachability.
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

### Health: interpreting source status

For each indexer the table distinguishes:

- **OK**: valid HTTP response and no Torznab application error;
- **API error**: the service responds, but the API key or indexer configuration is
  rejected;
- **Unreachable**: no response arrived before the timeout.

This distinction matters: restarting Rextto does not fix a wrong API key, while a
network problem may require checking DNS, containers, ports or the firewall.

## 11. Notifications

Configure Telegram, e-mail (SMTP) or a webhook (with HMAC secret) and send a
test. Completion notifications include size, download time and average speed.

### Configuration procedure

1. save credentials in the **Notifications** tab;
2. enable only the events you want to receive;
3. send the UI test;
4. check both the provider response and the Rextto log;
5. test a completion event only with an unimportant download.

For a webhook, verify the HMAC secret on the receiving side and do not confuse
an HTTP test with delivery of a real event. For SMTP, check host, port, TLS,
user and sender: a reachable server may still reject the sender or require a
different authentication method.

## Quick reference

### Which action to use

| Goal | Action |
|---|---|
| Understand what exists online | Manual search |
| Fill missing episodes | Search missing / Series cycle |
| Choose one specific release | **Why not this one?** → Queue |
| Make existing files known | Scan archive |
| Change names without re-downloading | Rename preview |
| Fix an external service | Health → source → Verify |
| Remove inferior files | Preview duplicates → Cleanup |
| Save configuration and databases | Backup |

### Glossary

- **Release**: a result found by a feed, indexer or web engine.
- **Cycle**: one automatic search and selection pass.
- **Gap**: a missing episode recognised in the archive.
- **Placeholder**: a temporary row representing an incomplete download.
- **Upgrade**: replacing an archived file with a better release.
- **Backoff**: a progressive pause for requests to a failing source.
- **Seed**: sharing a torrent after completion.
- **NAS**: network destination used for the library or configured paths.

## 12. Troubleshooting

- **A source is unreachable** — check *Configuration → Sources → Verify* and the
  sources health panel. For Jackett verify the base URL, API key and that at
  least one indexer is enabled in Jackett. Torznab errors are detected even when
  Jackett returns HTTP 200. Cloudflare-protected sites need a working
  FlareSolverr.
- **Jackett is reachable but shows an API error** — copy the API key again, check
  that at least one indexer is enabled in Jackett, and verify that the URL in
  Rextto is the correct base URL. Do not append
  `/api/v2.0/indexers/all/results/torznab/api` twice.
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
- **A release is visible but not selected** — open **Why not this one?** and
  check monitored title, global filters, sanity, quality/language and archive
  comparison first. If those pass, read the informational cycle-selection step:
  other candidates, delay, free space and gap filling can still change the result.
- **A file exists on the NAS but Rextto considers it missing** — check the episode
  filename, the series path and permissions, then use *Scan archive*. The scan
  recognises video names with season/episode information, not arbitrary files that
  cannot identify their content.
- **A download is complete but not in the library** — inspect the log for move,
  permission and free-space errors; do not delete the source until the archived
  path is visible in history.
- **FTP backup fails** — use *Test FTP*: it reports the failing step (connection,
  login, remote path, upload, delete) and logs it.
- **Logs** — see `data/rextto.log` (rotated at 5 MB) or the in-app log viewer.
