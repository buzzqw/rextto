use crate::{
    archive::Archive,
    comics::{ComicsDb, GetComicsClient},
    config::Config,
    database::Database,
    engine::Engine,
    libtorrent::LibtorrentClient,
    models::{CycleStats, Release},
    notifier::Notifier,
    parser::parse_release,
    tmdb::TmdbClient,
    utils::magnet_hash,
};
use anyhow::Result;
use chrono::Utc;
use std::sync::{Arc, Mutex};

/// Visual boundary that makes each cycle easy to locate in the log.
const CYCLE_DIVIDER: &str = "══════════════════════════════════════════════════════════════";

pub async fn run_cycle(
    cfg: &Config,
    engine: &Engine,
    db: &Arc<Mutex<Database>>,
    archive: &Arc<Mutex<Archive>>,
    comics: &Arc<ComicsDb>,
    notifier: &Notifier,
    torrents: &LibtorrentClient,
) -> Result<CycleStats> {
    run_cycle_domain(cfg, engine, db, archive, comics, notifier, torrents, None).await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_cycle_domain(
    cfg: &Config,
    engine: &Engine,
    db: &Arc<Mutex<Database>>,
    archive: &Arc<Mutex<Archive>>,
    comics: &Arc<ComicsDb>,
    notifier: &Notifier,
    torrents: &LibtorrentClient,
    domain: Option<&str>,
) -> Result<CycleStats> {
    let mut stats = CycleStats {
        last_started_at: Some(Utc::now()),
        ..Default::default()
    };
    tracing::info!("{CYCLE_DIVIDER}");
    tracing::info!(
        "🔄 CYCLE STARTED (mode: {})",
        domain.unwrap_or("full")
    );
    let comics_interval = comics
        .setting("comics_check_interval", "604800")?
        .parse::<i64>()
        .unwrap_or(604800)
        .max(0);
    let last_comics_check = comics
        .setting("last_comics_check_ts", "0")?
        .parse::<i64>()
        .unwrap_or(0);
    let now = Utc::now().timestamp();
    // An explicitly requested comics cycle must start immediately instead of
    // waiting for the automatic interval; otherwise the Comics button appears
    // to do nothing when the next scheduled check is not due yet.
    let comics_requested = domain == Some("comics");
    if domain != Some("series")
        && domain != Some("movies")
        && (comics_requested
            || comics_interval == 0
            || now.saturating_sub(last_comics_check) >= comics_interval)
    {
        match crate::comics::run_cycle(
            comics,
            &GetComicsClient::new(),
            notifier,
            &cfg.data_dir.join("comics"),
            torrents,
            cfg,
        )
        .await
        {
            Ok(downloaded) => tracing::info!(downloaded, "comics cycle completed"),
            Err(error) => {
                stats.error("comics");
                tracing::warn!(%error, "comics cycle failed");
            }
        }
        comics.set_setting("last_comics_check_ts", &now.to_string())?;
    }
    if domain == Some("comics") {
        db.lock().unwrap().save_cycle(&stats)?;
        return Ok(stats);
    }
    // Tracked torrents that are no longer in the session would remain "active"
    // without reconciliation and block re-downloads forever.
    {
        let live = torrents
            .list()
            .into_iter()
            .map(|torrent| torrent.hash.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        match db.lock().unwrap().reconcile_missing_torrents(&live) {
            Ok(count) if count > 0 => {
                tracing::info!(count, "torrents reconciled: marked missing from session")
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "torrent reconciliation failed"),
        }
    }
    let mut releases = engine.scrape_all(cfg).await?;
    if domain == Some("series") {
        releases.retain(|release| release.kind == "series");
    }
    if domain == Some("movies") {
        releases.retain(|release| release.kind == "movie");
    }
    // Do not keep or reconsider releases whose infohash was permanently
    // rejected (for example a season pack whose real files belong to another
    // season). This also removes stale conflicting rows from the archive.
    let blocked_hashes = {
        let database = db.lock().unwrap();
        releases
            .iter()
            .filter_map(|release| crate::utils::magnet_hash(&release.magnet))
            .filter(|hash| database.is_blocklisted(hash).unwrap_or(false))
            .collect::<std::collections::HashSet<_>>()
    };
    for hash in &blocked_hashes {
        archive.lock().unwrap().remove_hash(hash)?;
    }
    if !blocked_hashes.is_empty() {
        tracing::info!(count = blocked_hashes.len(), "blocked releases removed from this cycle");
        releases.retain(|release| {
            crate::utils::magnet_hash(&release.magnet)
                .is_none_or(|hash| !blocked_hashes.contains(&hash))
        });
    }
    archive.lock().unwrap().save_batch(&releases, cfg)?;
    // "Seen from feed": record every collected release, including unmonitored
    // titles, so it remains available for archive browsing.
    if let Err(error) = db.lock().unwrap().record_seen_batch(&releases, cfg) {
        tracing::debug!(%error, "feed seen recording failed");
    }
    let mut archive_queries = Vec::new();
    for series in cfg
        .series
        .iter()
        .filter(|series| series.enabled && domain != Some("movies"))
    {
        archive_queries.push(series.name.clone());
        archive_queries.extend(
            series
                .aliases
                .iter()
                .filter(|alias| !alias.trim().is_empty())
                .cloned(),
        );
    }
    if domain != Some("series") {
        archive_queries.extend(
            cfg.movies
                .iter()
                .filter(|movie| movie.enabled)
                .map(|movie| movie.name.clone()),
        );
    }
    let mut archive_hashes = std::collections::HashSet::new();
    for query in archive_queries {
        for (title, magnet, source) in archive.lock().unwrap().search(&query)? {
            let Some(hash) = magnet_hash(&magnet) else {
                continue;
            };
            if db
                .lock()
                .unwrap()
                .is_blocklisted(&hash)
                .unwrap_or(false)
            {
                continue;
            }
            if !archive_hashes.insert(hash) {
                continue;
            }
            if let Some(release) = parse_release(&title, &magnet, &format!("archive:{source}")) {
                if let Some(reason) = cfg.all_release_denied_reason(&release) {
                    if cfg.release_is_monitored(&release) {
                        crate::rules::log_rejection(&release, &reason);
                    }
                } else {
                    releases.push(release);
                }
            }
        }
    }
    let mut ready_pending = std::collections::HashSet::new();
    for (_series, title, magnet, _season, _episode) in db.lock().unwrap().ready_pending()? {
        if let Some(release) = parse_release(&title, &magnet, "timeframe") {
            if let Some(reason) = cfg.all_release_denied_reason(&release) {
                if cfg.release_is_monitored(&release) {
                    crate::rules::log_rejection(&release, &reason);
                }
                continue;
            }
            if let Some(hash) = magnet_hash(&release.magnet) {
                if db
                    .lock()
                    .unwrap()
                    .is_blocklisted(&hash)
                    .unwrap_or(false)
                {
                    continue;
                }
                ready_pending.insert(hash);
            }
            releases.push(release);
        }
    }
    // Movies held back by a delay profile.
    for (_name, title, magnet, _year) in db.lock().unwrap().ready_pending_movies()? {
        if let Some(release) = parse_release(&title, &magnet, "delay") {
            if let Some(reason) = cfg.all_release_denied_reason(&release) {
                if cfg.release_is_monitored(&release) {
                    crate::rules::log_rejection(&release, &reason);
                }
                continue;
            }
            if let Some(hash) = magnet_hash(&release.magnet) {
                if db
                    .lock()
                    .unwrap()
                    .is_blocklisted(&hash)
                    .unwrap_or(false)
                {
                    continue;
                }
                ready_pending.insert(hash);
            }
            releases.push(release);
        }
    }
    if domain != Some("movies") {
        refresh_series_metadata(cfg, db).await;
    }
    let gap_filling = cfg
        .settings
        .get("gap_filling")
        .map(|value| matches!(value.as_str(), "yes" | "true" | "1"))
        .unwrap_or(true);
    let archive_gaps = if domain == Some("movies") || !gap_filling {
        Vec::new()
    } else {
        db.lock()
            .unwrap()
            .archive_gaps()?
            // Respect the current configuration: only existing, enabled series
            // and monitored seasons (neither disabled nor outside `seasons`).
            // Without this filter gap fill would search deleted series and seasons
            // that the user explicitly disabled.
            .into_iter()
            .filter(|(series, season, _)| cfg.find_series_match(series, Some(*season)).is_some())
            .collect::<Vec<_>>()
    };
    let mut gap_summary = std::collections::HashMap::<(String, i64), Vec<i64>>::new();
    for (series, season, episode) in &archive_gaps {
        gap_summary
            .entry((series.clone(), *season))
            .or_default()
            .push(*episode);
    }
    for ((series, season), mut episodes) in gap_summary {
        episodes.sort_unstable();
        // Only at debug: the full list of open gaps is noisy. The log reports a
        // gap when it is actually filled (see DOWNLOAD STARTED below).
        tracing::debug!(
            "→ {} S{:02} gap: {}",
            series,
            season,
            episodes_label(&episodes)
        );
    }
    let gap_limit = cfg
        .settings
        .get("gap_fill_max_per_series")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let deep_interval_hours = cfg
        .settings
        .get("gap_deep_interval_hours")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(6)
        .max(1);
    let deep_max_per_cycle = cfg
        .settings
        .get("gap_deep_max_per_cycle")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5);
    let now_ts = Utc::now().timestamp();
    let last_deep = cfg
        .settings
        .get("last_deep_gap_fill_ts")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let is_deep = now_ts.saturating_sub(last_deep) >= deep_interval_hours.saturating_mul(3600);
    if is_deep {
        let _ = crate::config::Config::save_setting(
            &cfg.data_dir,
            "last_deep_gap_fill_ts",
            &now_ts.to_string(),
        );
    }
    tracing::info!(
        "🔎 Gap fill: {} missing episode(s) to check · {}",
        archive_gaps.len(),
        if is_deep {
            format!("deep pass, up to {deep_max_per_cycle} online searches")
        } else {
            "archive pass".to_string()
        }
    );

    // Phase 1 (every cycle, free): local archive search only.
    let mut live_candidates: Vec<(String, i64, i64, String)> = Vec::new();
    let mut live_per_series = std::collections::HashMap::<String, usize>::new();
    let mut archive_hits = 0usize;
    let gap_targets = archive_gaps
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    for (series, season, episode) in archive_gaps {
        let query = format!("{series} S{season:02}E{episode:02}");
        let mut found = false;
        if let Ok(items) = archive.lock().unwrap().search(&query) {
            for (title, magnet, source) in items {
                let Some(hash) = magnet_hash(&magnet) else {
                    continue;
                };
                if db
                    .lock()
                    .unwrap()
                    .is_blocklisted(&hash)
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Some(release) = parse_release(&title, &magnet, &format!("archive:{source}"))
                {
                    if let Some(reason) = cfg.all_release_denied_reason(&release) {
                        if cfg.release_is_monitored(&release) {
                            crate::rules::log_rejection(&release, &reason);
                        }
                    } else {
                        tracing::debug!(series = %series, season, episode, title = %release.title, source = %release.source, "archive release found");
                        releases.push(release);
                        found = true;
                    }
                }
            }
        }
        if found {
            archive_hits += 1;
            continue;
        }
        // Phase 2 candidate: deep live search only, like legacy.
        if !is_deep || live_candidates.len() >= deep_max_per_cycle {
            continue;
        }
        if gap_limit > 0 && live_per_series.get(&series).copied().unwrap_or(0) >= gap_limit {
            continue;
        }
        if db
            .lock()
            .unwrap()
            .gap_recently_searched(&series, season, episode, 23)?
        {
            continue;
        }
        // legacy queries only the series name (+ language), never SxxExx.
        let language = cfg
            .find_series_match(&series, Some(season))
            .map(|entry| entry.language.clone())
            .unwrap_or_default();
        let live_query = if language.trim().is_empty()
            || matches!(language.as_str(), "any" | "*" | "none" | "custom")
        {
            series.clone()
        } else {
            format!("{series} {language}")
        };
        *live_per_series.entry(series.clone()).or_default() += 1;
        live_candidates.push((series, season, episode, live_query));
    }
    tracing::info!(
        "🔎 Gap fill: {} found in the archive, {} to look up online",
        archive_hits,
        live_candidates.len()
    );

    if !live_candidates.is_empty() {
        let cfg_arc = Arc::new(cfg.clone());
        let mut gap_set = tokio::task::JoinSet::new();
        let mut gap_iter = live_candidates.into_iter();
        let schedule_gap =
            |set: &mut tokio::task::JoinSet<(String, i64, i64, Vec<Release>)>,
             iter: &mut std::vec::IntoIter<(String, i64, i64, String)>| {
                if let Some((series, season, episode, query)) = iter.next() {
                     let engine = engine.clone();
                     let cfg = cfg_arc.clone();
                     set.spawn(async move {
                         tracing::debug!(series = %series, season, episode, query = %query, "gap-fill live search started");
                         let started = std::time::Instant::now();
                         let found = engine.search_query(&cfg, &query).await;
                         tracing::debug!(series = %series, season, episode, query = %query, elapsed_ms = started.elapsed().as_millis(), results = found.len(), "gap-fill live search completed");
                         (series, season, episode, found)
                     });
                }
            };
        for _ in 0..3 {
            schedule_gap(&mut gap_set, &mut gap_iter);
        }
        while let Some(joined) = gap_set.join_next().await {
            if let Ok((series, season, episode, found)) = joined {
                releases.extend(found);
                let _ = db
                    .lock()
                    .unwrap()
                    .mark_gap_searched(&series, season, episode);
            }
            schedule_gap(&mut gap_set, &mut gap_iter);
        }
    }
    if domain == Some("series") {
        releases.retain(|release| release.kind == "series");
    }
    if domain == Some("movies") {
        releases.retain(|release| release.kind == "movie");
    }
    stats.scraped = releases.len();
    let mut best = Vec::<Release>::new();
    // Smallest archived episode per series, for the adaptive size floor.
    let mut series_min_cache = std::collections::HashMap::<String, Option<i64>>::new();
    for mut release in releases {
        stats.candidates += 1;
        if release.kind == "series" {
            let Some(series) = release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_match(name, release.season))
            else {
                // Not monitored: never log these, they are pure noise.
                continue;
            };
            if !cfg.series_release_allowed(series, &release.quality, &release.title) {
                log_candidate_rejected(
                    &release,
                    "excluded by the series quality/language/exclude rules",
                );
                continue;
            }
            release.series = Some(series.name.clone());
            let series_min = *series_min_cache.entry(series.name.clone()).or_insert_with(|| {
                db.lock()
                    .unwrap()
                    .series_archived_min_size(&series.name)
                    .unwrap_or(None)
            });
            if let Some(reason) = crate::rules::sane_size_denied_reason(&release, series_min) {
                crate::rules::log_rejection(&release, &reason);
                continue;
            }
        } else {
            let Some(movie) = cfg.find_movie_match(&release.title, release.year) else {
                // Not monitored: never log these, they are pure noise.
                continue;
            };
            if !cfg.movie_release_allowed(movie, &release.quality) {
                log_candidate_rejected(
                    &release,
                    "excluded by the movie quality/language/subtitle rules",
                );
                continue;
            }
            release.title = movie.name.clone();
            release.year = movie.year.parse::<i64>().ok().or(release.year);
            let movie_min = db
                .lock()
                .unwrap()
                .movie_archived_size(&movie.name, release.year)
                .unwrap_or(None);
            if let Some(reason) = crate::rules::sane_size_denied_reason(&release, movie_min) {
                crate::rules::log_rejection(&release, &reason);
                continue;
            }
        }
        let score = cfg.release_score(&release);
        if release.kind == "series" {
            let series = release.series.as_deref().unwrap_or_default();
            let season = release.season.unwrap_or_default();
            let range = release
                .episode_range
                .iter()
                .copied()
                .filter(|episode| *episode > 0)
                .collect::<std::collections::HashSet<_>>();
            let complete = release.episode_range.contains(&0);
            if best.iter().any(|old| {
                old.kind == "series"
                    && old.series.as_deref() == Some(series)
                    && old.season == Some(season)
                    && {
                        let old_range = old
                            .episode_range
                            .iter()
                            .copied()
                            .filter(|episode| *episode > 0)
                            .collect::<std::collections::HashSet<_>>();
                        let old_complete = old.episode_range.contains(&0);
                        ((complete && old_complete)
                            || (!complete
                                && old_complete
                                && incumbent_wins(&release, score, old, cfg))
                            || (!complete && !old_complete && old_range.is_superset(&range)))
                            && incumbent_wins(&release, score, old, cfg)
                    }
            }) {
                // Rejected because the selection already contains an equal or
                // better release: routine deduplication, not an INFO event.
                tracing::debug!(
                    target = %release_target(&release),
                    score,
                    reason = "superseded by an equal or better release already selected",
                    "🚫 release rejected (superseded)"
                );
                continue;
            }
            best.retain(|old| {
                if old.kind != "series"
                    || old.series.as_deref() != Some(series)
                    || old.season != Some(season)
                {
                    return true;
                }
                let old_range = old
                    .episode_range
                    .iter()
                    .copied()
                    .filter(|episode| *episode > 0)
                    .collect::<std::collections::HashSet<_>>();
                let old_complete = old.episode_range.contains(&0);
                !((complete || (range.len() > 1 && range.is_superset(&old_range)))
                    && !incumbent_wins(&release, score, old, cfg)
                    && (!old_complete || complete))
            });
            if let Some(index) = best.iter().position(|old| {
                old.kind == "series"
                    && old.series.as_deref() == Some(series)
                    && old.season == Some(season)
                    && old.episode_range == release.episode_range
            }) {
                if !incumbent_wins(&release, score, &best[index], cfg) {
                    best[index] = release;
                }
            } else {
                best.push(release);
            }
        } else if let Some(index) = best.iter().position(|old| {
            old.kind == "movie" && old.title == release.title && old.year == release.year
        }) {
            if !incumbent_wins(&release, score, &best[index], cfg) {
                best[index] = release;
            }
        } else {
            best.push(release);
        }
    }
    tracing::info!("🎯 CANDIDATES — {} release(s) survived the filters", best.len());
    for release in &best {
        tracing::debug!(
            target = %release_target(release),
            kind = %release.kind,
            episodes = %episodes_label(&release.episode_range),
            gap_episodes = %episodes_label(&gap_episodes_for_release(release, &gap_targets)),
            source = %release.source,
            score = cfg.release_score(release),
            "candidate ready for evaluation"
        );
    }
    // Advanced setting: refuse to start downloads when the download disk is
    // almost full. `min_free_space_gb = 0` disables the guard.
    let min_free_bytes = cfg
        .settings
        .get("min_free_space_gb")
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|gib| *gib > 0.0)
        .map(|gib| (gib * 1024.0 * 1024.0 * 1024.0) as u64);
    if let Some(minimum) = min_free_bytes {
        if let Some(free) = crate::libtorrent::free_space_bytes(&cfg.libtorrent_dir) {
            if free < minimum {
                tracing::warn!(
                    free = %crate::logging::human_bytes(free),
                    minimum = %crate::logging::human_bytes(minimum),
                    "cycle: free space below min_free_space_gb, downloads skipped"
                );
                stats.error("min_free_space");
                return Ok(stats);
            }
        }
    }
    let empty_archive_index = crate::models::ArchiveQualityIndex::default();
    let mut archive_index_cache = std::collections::HashMap::new();
    // Downloads currently in the libtorrent session: do not propose an existing
    // hash or episode again, even when the database has not recorded it yet.
    let live_downloads = {
        let mut live = crate::models::LiveDownloads::default();
        for torrent in torrents.list() {
            live.hashes.insert(torrent.hash.to_ascii_lowercase());
            if let Some(key) = crate::parser::parse_episode_key(&torrent.name) {
                live.episodes.insert(key);
            }
        }
        live
    };
    let mut upgrades = 0usize;
    let mut new_items = 0usize;
    let mut started_details = Vec::new();
    for mut release in best {
        // RSS feeds that expose only a `.torrent` link (for example TorrentLeech)
        // have no magnet: download the file, derive its infohash, and retain the
        // file for adding it (private trackers require it for announcing).
        let mut torrent_file: Option<std::path::PathBuf> = None;
        if release.magnet.trim().is_empty() {
            if let Some(url) = release.torrent_url.clone() {
                match resolve_torrent_url(engine, cfg, &url).await {
                    Ok((magnet, path)) => {
                        release.magnet = magnet;
                        // The feed was persisted before selection. Replace its
                        // short-lived Jackett URL with the stable magnet so a
                        // later archive search can use it without re-downloading
                        // an already expired link.
                        if let Err(error) = archive
                            .lock()
                            .unwrap()
                            .canonicalize_torrent_url(&url, &release.magnet)
                        {
                            tracing::warn!(%error, "could not canonicalize archived torrent URL");
                        }
                        torrent_file = Some(path);
                    }
                    Err(error) => {
                        stats.error("torrent_link");
                        tracing::warn!(
                            target = %release_target(&release),
                            %error,
                            "torrent link resolution failed"
                        );
                        continue;
                    }
                }
            }
        }
        reconcile_pack_identity_from_magnet(&mut release);
        let is_ready_pending =
            magnet_hash(&release.magnet).is_some_and(|hash| ready_pending.contains(&hash));
        // A candidate that fills a known archive gap, or that scores above the
        // bypass threshold, is never held by a delay profile.
        let is_gap = match (release.series.as_deref(), release.season, release.episode) {
            (Some(series), Some(season), Some(episode)) => {
                gap_targets.contains(&(series.to_string(), season, episode))
            }
            _ => false,
        };
        let bypass_delay = is_gap
            || (cfg.delay_bypass_score() > 0
                && cfg.release_score(&release) >= cfg.delay_bypass_score());
        if release.kind == "series" && !is_ready_pending {
            if let Some(series) = release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_match(name, release.season))
            {
                // Per-series `timeframe` (hours) wins over the global setting.
                let delay_minutes = if series.timeframe > 0 {
                    series.timeframe.saturating_mul(60)
                } else {
                    cfg.delay_minutes("series")
                };
                if delay_minutes > 0 && !bypass_delay {
                    tracing::info!(
                        target = %release_target(&release),
                        delay_minutes,
                        "queued for delay"
                    );
                    db.lock().unwrap().queue_pending_scored(
                        &release,
                        delay_minutes,
                        cfg.release_score(&release),
                    )?;
                    continue;
                }
            }
        }
        if release.kind == "movie" && !is_ready_pending {
            let delay_minutes = cfg.delay_minutes("movie");
            if delay_minutes > 0 && !bypass_delay {
                tracing::info!(
                    target = %release_target(&release),
                    delay_minutes,
                    "movie queued for delay"
                );
                db.lock().unwrap().queue_pending_movie_scored(
                    &release,
                    delay_minutes,
                    cfg.release_score(&release),
                )?;
                continue;
            }
        }
        // Archive index (per series, computed once per cycle): decisions also
        // consider real files on disk, not only database rows.
        let archive_index = if release.kind == "series" {
            let key = release.series.clone().unwrap_or_default();
            let season = release.season;
            archive_index_cache
                .entry(key)
                .or_insert_with(|| {
                    release
                        .series
                        .as_deref()
                        .and_then(|name| cfg.find_series_match(name, season))
                        .map(|series| {
                            let archive = cfg.resolve_archive_path(series);
                            crate::cleaner::index_archive(
                                &series.name,
                                archive.as_deref().unwrap_or(std::path::Path::new("")),
                                &cfg.settings,
                            )
                        })
                        .unwrap_or_default()
                })
                .clone()
        } else {
            empty_archive_index.clone()
        };
        // Per-title "allow upgrades" toggle (default on). The gap flag feeds the
        // always-on "no orphan older episode" best practice.
        let forbid_upgrade = if release.kind == "series" {
            release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_match(name, release.season))
                .is_some_and(|series| series.disable_upgrades)
        } else {
            cfg.find_movie_match(&release.title, release.year)
                .is_some_and(|movie| movie.disable_upgrades)
        };
        let approval_context = crate::models::ApprovalContext {
            archive: archive_index,
            live: live_downloads.clone(),
            forbid_upgrade,
            gap_episode: is_gap,
        };
        let (approved, approval_reason, score) = {
            let db = db.lock().unwrap();
            let score = cfg.release_score(&release);
            let min_diff = cfg.upgrade_min_score_diff;
            let result = if release.kind == "series" {
                db.check_series_scored(&release, score, min_diff, &approval_context)?
            } else {
                db.check_movie_scored_with(&release, score, min_diff, forbid_upgrade)?
            };
            (result.0, result.1, score)
        };
        let gap_episodes = gap_episodes_for_release(&release, &gap_targets);
        let from_archive = release.source.starts_with("archive:");
        let decision_reason = if from_archive {
            "gap_filled"
        } else if !gap_episodes.is_empty() {
            "gap_fill"
        } else {
            approval_reason.as_str()
        };
        if approved {
            let download_dir = crate::postprocess::download_dir_for(&release, cfg);
            let added = match &torrent_file {
                Some(path) => torrents.add_file_with_path(path, cfg, download_dir.as_deref()),
                None => torrents.add_with_path(&release.magnet, cfg, download_dir.as_deref()),
            };
            match added {
                Ok(true) => {
                    let started_detail = if gap_episodes.is_empty() {
                        format!(
                            "{} [{}]: {}",
                            release_target(&release),
                            release.source,
                            release.title
                        )
                    } else {
                        format!(
                            "{} · episodi {} [{}]: {}",
                            release_target(&release),
                            episodes_label(&gap_episodes),
                            release.source,
                            release.title
                        )
                    };
                    started_details.push(started_detail);
                    if gap_episodes.is_empty() {
                        tracing::info!(
                            "📥 Download started [{}]: {} · {} · score {}",
                            release.source,
                            release_target(&release),
                            release.kind,
                            score
                        );
                    } else {
                        tracing::info!(
                            "✅ Gap filled: {} · episodes {} · downloading [{}]: {}",
                            release_target(&release),
                            episodes_label(&gap_episodes),
                            release.source,
                            release.title
                        );
                    }
                    db.lock()
                        .unwrap()
                        .register_torrent_scored(&release, score)?;
                    if let Some(hash) = magnet_hash(&release.magnet) {
                        let _ = db
                            .lock()
                            .unwrap()
                            .set_torrent_reason(&hash, decision_reason);
                    }
                    if is_ready_pending {
                        if let (Some(series), Some(season), Some(episode)) =
                            (release.series.as_deref(), release.season, release.episode)
                        {
                            db.lock().unwrap().remove_pending(series, season, episode)?;
                        } else if release.kind == "movie" {
                            db.lock()
                                .unwrap()
                                .remove_pending_movie(&release.title, release.year)?;
                        }
                    }
                    stats.downloads_started += 1;
                    if approval_reason == "upgrade" {
                        upgrades += 1;
                    } else {
                        new_items += 1;
                    }
                    if from_archive || !gap_episodes.is_empty() {
                        stats.gaps_filled += 1;
                    }
                    if let Err(error) = notifier
                        .notify_event(
                            "download_started",
                            serde_json::json!({
                                "title": &release.title,
                                "kind": &release.kind,
                                "series": &release.series,
                                "season": release.season,
                                "episode": release.episode,
                                "source": &release.source,
                                "magnet_hash": magnet_hash(&release.magnet),
                                "quality_score": score,
                                "reason": decision_reason,
                                "approval_reason": &approval_reason,
                                "gap_episodes": &gap_episodes,
                                "resolution": &release.quality.resolution,
                                "video_codec": &release.quality.codec,
                                "audio": &release.quality.audio,
                                "hdr": &release.quality.hdr,
                                "language": &release.quality.language,
                            }),
                        )
                        .await
                    {
                        tracing::warn!(%error, "download notification failed");
                    }
                }
                Ok(false) => {
                    db.lock().unwrap().rollback_release(&release)?;
                    stats.error("torrent_rejected");
                }
                Err(error) => {
                    db.lock().unwrap().rollback_release(&release)?;
                    return Err(error);
                }
            }
        } else {
            // "Already present / already active" rejections (duplicate or episode
            // active in the session) are routine, so keep them at debug level to
            // keep the cycle log readable.
            if matches!(approval_reason.as_str(), "duplicate" | "active_episode") {
                tracing::debug!(
                    target = %release_target(&release),
                    score,
                    reason = %decision_reason,
                    approval_reason = %approval_reason,
                    "download skipped (already present or active)"
                );
            } else {
                tracing::info!(
                    target = %release_target(&release),
                    kind = %release.kind,
                    source = %release.source,
                    score,
                    reason = %decision_reason,
                    approval_reason = %approval_reason,
                    gap_episodes = %episodes_label(&gap_episodes),
                    "⏭️ download skipped"
                );
            }
        }
    }
    db.lock().unwrap().save_cycle(&stats)?;
    let started = stats.last_started_at.unwrap_or_else(Utc::now);
    let elapsed = (Utc::now() - started).num_seconds().max(0);
    tracing::info!(
        "📊 CYCLE REPORT — duration {} — scraped: {} | candidates: {} | downloads started: {} (upgrade: {} · new: {}) | gaps filled: {} | errors: {}",
        human_duration(elapsed),
        stats.scraped,
        stats.candidates,
        stats.downloads_started,
        upgrades,
        new_items,
        stats.gaps_filled,
        stats.errors
    );
    if started_details.is_empty() {
        tracing::info!("📦 CYCLE DOWNLOADS — no downloads started");
    } else {
        tracing::info!(
            "📦 CYCLE DOWNLOADS — {}",
            started_details.join(" · ")
        );
    }
    if stats.downloads_started == 0 {
        tracing::info!("💤 No downloads in this cycle");
    }
    tracing::info!("{CYCLE_DIVIDER}");
    Ok(stats)
}

/// Human-readable target of a release: `Series S01E02`, `Series S01`, or the
/// title for movies. Keeps log lines free of Rust's `Some(...)` debug noise.
fn release_target(release: &Release) -> String {
    if release.kind == "series" {
        if let Some(series) = release.series.as_deref() {
            let season = release.season.map(|season| format!("S{season:02}"));
            let episode = match release.episode {
                Some(0) => Some("pack".to_string()),
                Some(episode) => Some(format!("E{episode:02}")),
                None => None,
            };
            let suffix = [season, episode]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("");
            return if suffix.is_empty() {
                series.to_string()
            } else {
                format!("{series} {suffix}")
            };
        }
    }
    release.title.clone()
}

/// Some indexers publish a corrected/translated title while the torrent's
/// display name still carries the actual season and episode range. Prefer that
/// identity for season packs: the files are named from the torrent metadata,
/// and rejecting a valid S05 pack as S06 would make it download again forever.
fn reconcile_pack_identity_from_magnet(release: &mut Release) {
    if release.kind != "series" || !release.is_pack {
        return;
    }
    let Ok(url) = url::Url::parse(&release.magnet) else {
        return;
    };
    let Some(display_name) = url
        .query_pairs()
        .find(|(key, _)| key == "dn")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.trim().is_empty() && value != &release.title)
    else {
        return;
    };
    let Some(corrected) = crate::parser::reconcile_pack_identity(release, &display_name) else {
        return;
    };
    tracing::warn!(
        title = %release.title,
        torrent_name = %display_name,
        declared_season = ?release.season,
        torrent_season = ?corrected.season,
        "correcting season-pack identity from torrent display name"
    );
    *release = corrected;
}

/// `1,2,3`, or `none` when empty; avoids `[]` in the log.
fn episodes_label(episodes: &[i64]) -> String {
    if episodes.is_empty() {
        "none".to_string()
    } else {
        episodes
            .iter()
            .map(|episode| episode.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// A release for a monitored title that does not meet the quality/language/
/// exclude rules is routine noise (thousands per cycle), so it is logged at
/// debug only.
fn log_candidate_rejected(release: &Release, reason: &str) {
    tracing::debug!(
        target = %release_target(release),
        kind = %release.kind,
        source = %release.source,
        reason,
        "release excluded by rules"
    );
}

fn human_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {}s", seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

/// Downloads a `.torrent`, derives its v1 infohash, and saves it in the state dir.
/// Returns the equivalent magnet and the file path used when adding it.
async fn resolve_torrent_url(
    engine: &Engine,
    cfg: &Config,
    url: &str,
) -> Result<(String, std::path::PathBuf)> {
    let bytes = engine.fetch_torrent(url).await?;
    let hash = crate::utils::torrent_info_hash(&bytes)
        .ok_or_else(|| anyhow::anyhow!("invalid torrent payload"))?;
    let dir = cfg.state_dir.join("feed_torrents");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{hash}.torrent"));
    std::fs::write(&path, &bytes)?;
    Ok((format!("magnet:?xt=urn:btih:{hash}"), path))
}

/// True when `incumbent` must not be replaced by `candidate`: at equal scores a
/// REMUX (full-resolution version) wins, especially when it is the remux of an
/// episode that was already selected or downloaded.
fn incumbent_wins(
    candidate: &Release,
    candidate_score: i64,
    incumbent: &Release,
    cfg: &Config,
) -> bool {
    // Use the same policy-aware score the candidate was ranked with, so score
    // rules and custom formats cannot make the two sides inconsistent.
    let incumbent_score = cfg.release_score(incumbent);
    incumbent_score > candidate_score
        || (incumbent_score == candidate_score
            && !(candidate.quality.is_remux() && !incumbent.quality.is_remux()))
}

fn gap_episodes_for_release(
    release: &Release,
    gap_targets: &std::collections::HashSet<(String, i64, i64)>,
) -> Vec<i64> {
    let (Some(series), Some(season)) = (release.series.as_deref(), release.season) else {
        return Vec::new();
    };
    let mut episodes = release
        .episode_range
        .iter()
        .copied()
        .filter(|episode| {
            *episode > 0 && gap_targets.contains(&(series.to_owned(), season, *episode))
        })
        .collect::<Vec<_>>();
    episodes.sort_unstable();
    episodes.dedup();
    episodes
}

async fn refresh_series_metadata(cfg: &Config, db: &Arc<Mutex<Database>>) {
    if cfg.tmdb_api_key.is_none() {
        return;
    }
    let tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    let known_statuses = db
        .lock()
        .unwrap()
        .series_statuses()
        .unwrap_or_default();
    for series in cfg.series.iter().filter(|series| series.enabled) {
        let stale = db
            .lock()
            .unwrap()
            .series_metadata_stale(&series.name, 24)
            .unwrap_or(true);
        // Fetch the TMDB status even when season metadata is recent if the status
        // has never been persisted (the first refresh).
        let needs_status = !known_statuses.contains_key(&series.name);
        if !stale && !needs_status {
            continue;
        }
        let tmdb_id = if !series.tmdb_id.trim().is_empty() {
            Some(series.tmdb_id.clone())
        } else {
            tmdb.resolve_series_id(&series.name).await.ok().flatten()
        };
        let Some(tmdb_id) = tmdb_id else {
            continue;
        };
        if stale {
            match tmdb.season_counts(&tmdb_id).await {
                Ok(counts) => {
                    let values = counts.into_iter().collect::<Vec<_>>();
                    if let Err(error) = db
                        .lock()
                        .unwrap()
                        .save_series_metadata(&series.name, &values)
                    {
                        tracing::warn!(series=%series.name, %error, "TMDB season metadata save failed");
                    }
                }
                Err(error) => {
                    tracing::debug!(series=%series.name, %error, "TMDB season metadata refresh failed")
                }
            }
        }
        // Persist TMDB status ("Ended"/"Returning Series") for the series-list badge.
        if let Ok(Some(info)) = tmdb.series_info(&series.name, Some(&tmdb_id)).await {
            let status = info
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let last_air_date = info
                .get("last_air_date")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if let Err(error) = db
                .lock()
                .unwrap()
                .save_series_status(&series.name, status, last_air_date)
            {
                tracing::warn!(series=%series.name, %error, "series status save failed");
            }
        }
    }
}
