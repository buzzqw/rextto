# Rextto Acceptance Gate

Rextto is not production-ready until every mandatory item below is implemented,
tested, and marked complete. `dry-run` remains mandatory until then.

External torrent clients are intentionally excluded. Rextto replaces them with
the embedded Libtorrent Rasterbar session only.

## Core And Persistence

| Area | Required behavior | Status |
|---|---|---|
| Configuration | Native `rextto_config.db`, settings, movies, limits, translations and import | In progress |
| Operational DB | Series, episodes, movies, pending, metadata, gaps, history, blocklist, torrent metadata | In progress |
| Archive DB | Long-term torrent archive, deduplication and search | In progress |
| Comics DB | Monitored comics, history, weekly packs and settings | In progress |
| Import | Offline copy, idempotent import, audit snapshots and config migration | In progress |
| Logging | Daily rotating structured logs and persisted cycle reports | In progress |

## Acquisition And Decisions

| Area | Required behavior | Status |
|---|---|---|
| Parser | Series, films, packs, dates, languages, blacklist and content filters | In progress |
| Scoring | Resolution, source, codec, HDR/DV, audio, group, repack/proper and custom scores | In progress |
| Series matching | Canonical name, aliases, seasons, language, quality, timeframe and upgrades | In progress |
| Film matching | Year, keyword boundaries, exclusions, language and subtitles | In progress |
| RSS | ExtTo, Corsaro and generic RSS | In progress |
| Indexers | Jackett and Prowlarr Torznab | In progress |
| Web search | Configured BitSearch/TPB/BT4G/1337x engines, FlareSolverr fallback and BTIH deduplication | In progress |
| Archive search | Gap-fill search from the local archive with cooldown persistence | In progress |
| Cycle | Best-in-cycle, timeframe, gap-fill cooldowns and statistics | In progress |

## Libtorrent And Files

| Area | Required behavior | Status |
|---|---|---|
| Session | Ports, bandwidth, queue, DHT, LSD, UPnP and NAT-PMP | In progress |
| Torrent control | Add, live list, pause, resume, remove, details, recheck and per-torrent limits | In progress |
| Queue policy | Metadata promotion, stalled checks, ETA priority and dynamic queue | In progress |
| Resume state | Fastresume, restored torrents and safe shutdown | In progress |
| Seed policy | Global and per-torrent ratio/time, infinite seed and cleanup | In progress |
| Completion alerts | Metadata, finished, storage moved and resume alerts | In progress |
| Post-processing | NAS move, season packs, rename, cleaner and notifications | In progress |
| File safety | Cross-device moves, trash policy and duplicate/upgrade protection | In progress |

## Services And Integrations

| Area | Required behavior | Status |
|---|---|---|
| TMDB | Search, metadata, episode titles, retry and cache | In progress |
| Rename | Base, standard, full and custom formats with MediaInfo tags | In progress |
| Notifications | Telegram, email, webhook and throttling | In progress |
| Comics | Monitoring DB/API, GetComics cycle, safe HTTP/Mega/torrent downloads and notifications | In progress |
| Trakt | Device flow, watchlist, calendar and scrobbling | In progress |
| Simkl | PIN flow, lists, calendar and watched markers | In progress |
| Health | Disk, process, permissions, service and consumption metrics | In progress |
| Backup | Local archive, retention, FTP and Telegram report | Implemented, observation pending |

## API And Web UI

| Area | Required behavior | Status |
|---|---|---|
| API | Native, documented endpoints replacing engine and web APIs | In progress |
| Security | Local-first binding, explicit authentication policy and safe filesystem boundaries | In progress |
| Dashboard | Fast responsive overview, cycles, health and resource usage | Implemented, browser validation pending |
| Torrent manager | Live states, details, peers, limits and safe actions | In progress |
| Library config | Series, films, comics, aliases, quality and folders | Implemented, browser validation pending |
| Maintenance | Archive, rescore, backup, logs, language and database tools | Implemented, browser validation pending |
| i18n | Italian and English translations migrated from configuration DB | Implemented, browser validation pending |
| UX | Mobile-first responsive UI, accessible controls, no legacy Flask/JS dependency | Implemented, browser validation pending |

## Final Transition Test

The final test is permitted only when all mandatory rows are complete.

1. Stop legacy and take an offline SQLite backup.
2. Import into a clean Rextto data directory.
3. Validate database counts, configuration and stored paths.
4. Run parser and scoring regression fixtures from real releases.
5. Run an isolated Libtorrent test with temporary paths and controlled magnets.
6. Verify rename, move, season-pack cleanup, resume and seed policy.
7. Verify every API/UI path and notifications with test credentials.
8. Run Rextto alone on ports 5000 and 8889 for an observation period.
9. Only after successful observation, enable real download mode.
