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
    if crate::messages::is_english() {
        tracing::info!(
            "🔄 Cycle started (mode: {})",
            domain.unwrap_or("full")
        );
    } else {
        tracing::info!(
            "🔄 Ciclo avviato (modalità: {})",
            domain.unwrap_or("completa")
        );
    }
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
    if domain != Some("series")
        && domain != Some("movies")
        && (comics_interval == 0 || now.saturating_sub(last_comics_check) >= comics_interval)
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
    // Torrent tracciati ma non più presenti nella sessione: senza riconciliazione
    // restano "in corso" e bloccano per sempre il ri-scaricamento.
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
    archive.lock().unwrap().save_batch(&releases)?;
    // "Visti nei feed": memorizza ogni release raccolta (anche quelle non
    // monitorate) per la consultazione nell'archivio.
    if let Err(error) = db.lock().unwrap().record_seen_batch(&releases) {
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
            if !archive_hashes.insert(hash) {
                continue;
            }
            if let Some(release) = parse_release(&title, &magnet, &format!("archive:{source}")) {
                if cfg.release_allowed(&release) {
                    releases.push(release);
                }
            }
        }
    }
    let mut ready_pending = std::collections::HashSet::new();
    for (_series, title, magnet, _season, _episode) in db.lock().unwrap().ready_pending()? {
        if let Some(release) = parse_release(&title, &magnet, "timeframe") {
            if !cfg.release_allowed(&release) {
                continue;
            }
            if let Some(hash) = magnet_hash(&release.magnet) {
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
        db.lock().unwrap().archive_gaps()?
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
        tracing::info!(series = %series, season, gaps = ?episodes, "gap-fill target identified");
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
        is_deep,
        gaps = archive_gaps.len(),
        deep_max_per_cycle,
        "cycle: gap-fill starting"
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
                if let Some(release) = parse_release(&title, &magnet, &format!("archive:{source}"))
                {
                    if cfg.release_allowed(&release) {
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
        archive_hits,
        live = live_candidates.len(),
        "cycle: gap-fill archive pass done"
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
                        let found = engine.search_query(&cfg, &query).await;
                        tracing::debug!(series = %series, season, episode, query = %query, results = found.len(), "gap-fill live search completed");
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
    for mut release in releases {
        stats.candidates += 1;
        if release.kind == "series" {
            let Some(series) = release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_match(name, release.season))
            else {
                continue;
            };
            if !Config::series_release_allowed(series, &release.quality, &release.title) {
                continue;
            }
            release.series = Some(series.name.clone());
        } else {
            let Some(movie) = cfg.find_movie_match(&release.title, release.year) else {
                continue;
            };
            if !Config::movie_release_allowed(movie, &release.quality) {
                continue;
            }
            release.title = movie.name.clone();
            release.year = movie.year.parse::<i64>().ok().or(release.year);
        }
        let score = release.quality.score_with_settings(&cfg.settings);
        if release.kind == "series" {
            let series = release.series.as_deref().unwrap_or_default();
            let season = release.season.unwrap_or_default();
            let range = release
                .episode_range
                .iter()
                .copied()
                .filter(|episode| *episode > 0)
                .collect::<std::collections::HashSet<_>>();
            let complete = release.episode_range.iter().any(|episode| *episode == 0);
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
                        let old_complete = old.episode_range.iter().any(|episode| *episode == 0);
                        ((complete && old_complete)
                            || (!complete
                                && old_complete
                                && old.quality.score_with_settings(&cfg.settings) >= score)
                            || (!complete && !old_complete && old_range.is_superset(&range)))
                            && old.quality.score_with_settings(&cfg.settings) >= score
                    }
            }) {
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
                let old_complete = old.episode_range.iter().any(|episode| *episode == 0);
                !((complete || (range.len() > 1 && range.is_superset(&old_range)))
                    && score >= old.quality.score_with_settings(&cfg.settings)
                    && (!old_complete || complete))
            });
            if let Some(index) = best.iter().position(|old| {
                old.kind == "series"
                    && old.series.as_deref() == Some(series)
                    && old.season == Some(season)
                    && old.episode_range == release.episode_range
            }) {
                if best[index].quality.score_with_settings(&cfg.settings) < score {
                    best[index] = release;
                }
            } else {
                best.push(release);
            }
        } else if let Some(index) = best.iter().position(|old| {
            old.kind == "movie" && old.title == release.title && old.year == release.year
        }) {
            if best[index].quality.score_with_settings(&cfg.settings) < score {
                best[index] = release;
            }
        } else {
            best.push(release);
        }
    }
    tracing::info!(
        candidates = best.len(),
        "cycle: candidate releases selected"
    );
    for release in &best {
        tracing::debug!(
            kind = %release.kind,
            series = ?release.series,
            season = ?release.season,
            episodes = ?release.episode_range,
            gap_episodes = ?gap_episodes_for_release(&release, &gap_targets),
            title = %release.title,
            source = %release.source,
            score = release.quality.score_with_settings(&cfg.settings),
            "candidate release ready for evaluation"
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
                    free_gb = free as f64 / 1_073_741_824.0,
                    min_gb = minimum as f64 / 1_073_741_824.0,
                    "cycle: spazio libero sotto min_free_space_gb, download saltati"
                );
                stats.error("min_free_space");
                return Ok(stats);
            }
        }
    }
    for release in best {
        let is_ready_pending =
            magnet_hash(&release.magnet).is_some_and(|hash| ready_pending.contains(&hash));
        if release.kind == "series" {
            if let Some(series) = release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_match(name, release.season))
            {
                if series.timeframe > 0 && !is_ready_pending {
                    tracing::info!(
                        series = %series.name,
                        season = ?release.season,
                        episode = ?release.episode,
                        title = %release.title,
                        timeframe_hours = series.timeframe,
                        "release queued for timeframe"
                    );
                    db.lock()
                        .unwrap()
                        .queue_pending(&release, series.timeframe)?;
                    continue;
                }
            }
        }
        let (approved, approval_reason, score) = {
            let db = db.lock().unwrap();
            let score = release.quality.score_with_settings(&cfg.settings);
            let min_diff = cfg.upgrade_min_score_diff;
            let result = if release.kind == "series" {
                db.check_series_scored(&release, score, min_diff)?
            } else {
                db.check_movie_scored(&release, score, min_diff)?
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
            match torrents.add_with_path(&release.magnet, cfg, download_dir.as_deref()) {
                Ok(true) => {
                    tracing::info!(
                        kind = %release.kind,
                        series = ?release.series,
                        season = ?release.season,
                        episode = ?release.episode,
                        title = %release.title,
                        source = %release.source,
                        score,
                        reason = %decision_reason,
                        approval_reason = %approval_reason,
                        gap_episodes = ?gap_episodes,
                        hash = ?magnet_hash(&release.magnet),
                        "download started"
                    );
                    db.lock().unwrap().register_torrent(&release)?;
                    if is_ready_pending {
                        if let (Some(series), Some(season), Some(episode)) =
                            (release.series.as_deref(), release.season, release.episode)
                        {
                            db.lock().unwrap().remove_pending(series, season, episode)?;
                        }
                    }
                    stats.downloads_started += 1;
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
            tracing::debug!(
                kind = %release.kind,
                series = ?release.series,
                season = ?release.season,
                episode = ?release.episode,
                title = %release.title,
                source = %release.source,
                score,
                reason = %decision_reason,
                approval_reason = %approval_reason,
                gap_episodes = ?gap_episodes,
                "candidate skipped"
            );
        }
    }
    db.lock().unwrap().save_cycle(&stats)?;
    let started = stats.last_started_at.unwrap_or_else(Utc::now);
    let elapsed = (Utc::now() - started).num_seconds().max(0);
    if crate::messages::is_english() {
        tracing::info!(
            "📊 CYCLE REPORT — duration {} — scraped: {} | candidates: {} | downloads started: {} | gaps filled: {} | errors: {}",
            human_duration(elapsed),
            stats.scraped,
            stats.candidates,
            stats.downloads_started,
            stats.gaps_filled,
            stats.errors
        );
    } else {
        tracing::info!(
            "📊 CYCLE REPORT — durata {} — scraping: {} | candidati: {} | download avviati: {} | gap riempiti: {} | errori: {}",
            human_duration(elapsed),
            stats.scraped,
            stats.candidates,
            stats.downloads_started,
            stats.gaps_filled,
            stats.errors
        );
    }
    if stats.downloads_started == 0 {
        if crate::messages::is_english() {
            tracing::info!("💤 No downloads in this cycle");
        } else {
            tracing::info!("💤 Nessun download in questo ciclo");
        }
    }
    Ok(stats)
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
    for series in cfg.series.iter().filter(|series| series.enabled) {
        let stale = db
            .lock()
            .unwrap()
            .series_metadata_stale(&series.name, 24)
            .unwrap_or(true);
        if !stale {
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
}
