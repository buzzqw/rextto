use crate::{
    config::{Config, IndexerConfig},
    models::Release,
    parser::parse_release,
    rss::{fetch_feed, fetch_torznab_flaresolverr},
    utils::magnet_hash,
    websearch,
};
use anyhow::Result;
use reqwest::Client;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct Engine {
    client: Client,
    /// Optional archive DB used to persist the escalating provider backoff.
    /// `None` in unit tests and read-only tools that never touch a database.
    db: Option<Arc<std::sync::Mutex<crate::database::Database>>>,
}

/// A title search fans out again to every configured indexer and web engine.
/// Keep this deliberately below the Tokio worker count so HTTP handlers retain
/// workers while a scheduled cycle is running.
const QUERY_CONCURRENCY: usize = 2;
/// Feed HTML/RSS can decode and parse large response bodies. Keep enough Tokio
/// workers available for Axum while a cycle scans remote sources.
const FEED_CONCURRENCY: usize = 4;
/// Includes direct retries and the optional FlareSolverr fallback. A malformed
/// or stalled feed must not indefinitely retain a slot in the feed fan-out.
const FEED_FETCH_BUDGET: Duration = Duration::from_secs(75);
/// A single unresponsive indexer or HTML engine must not keep a cycle task
/// alive forever. This covers the whole fan-out, not just one HTTP request.
const AUTOMATIC_SEARCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Le ricerche avviate dall'interfaccia non devono attendere gli indexer/web
/// lenti. I cicli automatici mantengono invece il timeout HTTP più generoso.
const MANUAL_SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

impl Engine {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("rextto/0.1")
                .connect_timeout(std::time::Duration::from_secs(10))
                // Prowlarr can legitimately take up to one minute when it
                // fans a query out to several indexers.
                .timeout(std::time::Duration::from_secs(75))
                .build()
                .expect("http client"),
            db: None,
        }
    }

    /// Engine bound to the archive database so provider backoff survives
    /// restarts and is shared with the rest of the daemon.
    pub fn with_db(db: Arc<std::sync::Mutex<crate::database::Database>>) -> Self {
        Self {
            db: Some(db),
            ..Self::new()
        }
    }

    /// Scarica un file `.torrent` da un feed RSS che non espone un magnet. Non
    /// logga l'URL completo: può contenere una passkey.
    pub async fn fetch_torrent(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            let host = url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_else(|| "feed".to_string());
            anyhow::bail!("HTTP {} fetching torrent from {host}", response.status());
        }
        Ok(response.bytes().await?.to_vec())
    }

    pub async fn scrape_all(&self, cfg: &Config) -> Result<Vec<Release>> {
        let mut all = Vec::new();
        let flaresolverr = cfg.flaresolverr_url.clone();
        let max_pages = cfg.feed_max_pages();
        let max_age_days = cfg.max_release_age_days;
        let old_ratio = cfg.stop_on_old_page_ratio();
        // Each cycle reports only its own sources: drop anything accumulated
        // since the previous drain (e.g. manual searches from the UI).
        let _ = crate::logging::take_source_stats();
        tracing::info!(
            "🔎 Step 1/2: scanning {} sources (HTML/RSS feeds)",
            cfg.feed_urls.len()
        );
        let mut feed_set = tokio::task::JoinSet::new();
        let mut feed_iter = cfg.feed_urls.clone().into_iter();
        let provider_db = self.db.clone();
        let schedule_feed = |set: &mut tokio::task::JoinSet<Result<Vec<Release>>>,
                             iter: &mut std::vec::IntoIter<String>| {
            if let Some(url) = iter.next() {
                let client = self.client.clone();
                let flaresolverr = flaresolverr.clone();
                let db = provider_db.clone();
                let feed = feed_label(&url);
                let source = feed_source_name(&url);
                set.spawn(async move {
                    // Skip sources inside their backoff window instead of
                    // hammering them once more every cycle.
                    let blocked = db
                        .as_ref()
                        .and_then(|db| db.lock().ok().map(|db| db.provider_blocked("feed", &source)))
                        .and_then(Result::ok)
                        .unwrap_or(false);
                    if blocked {
                        tracing::debug!(feed = %feed, "feed skipped (backoff)");
                        return Ok(Vec::new());
                    }
                    let result = match tokio::time::timeout(
                        FEED_FETCH_BUDGET,
                        fetch_feed(
                            &client,
                            &url,
                            flaresolverr.as_deref(),
                            max_pages,
                            max_age_days,
                            old_ratio,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(anyhow::anyhow!(
                            "feed exceeded the {}s total time budget",
                            FEED_FETCH_BUDGET.as_secs()
                        )),
                    };
                    let failure = match &result {
                        Ok(items) => {
                            crate::logging::source_ok("feed", &source, items.len());
                            tracing::debug!(feed = %feed, items = items.len(), "RSS feed analyzed");
                            if let Some(db) = db.as_ref() {
                                if let Ok(db) = db.lock() {
                                    let _ = db.provider_success("feed", &source);
                                }
                            }
                            None
                        }
                        Err(error) => {
                            let message = crate::utils::redact_url_secrets(&error.to_string());
                            // Cloudflare a volte chiude lo stream a metà: reqwest lo
                            // riporta come errore di decodifica. Meglio dirlo in chiaro.
                            let friendly = if message.contains("decoding response body") {
                                "connection interrupted before the feed was complete".to_string()
                            } else {
                                message.clone()
                            };
                            crate::logging::source_fail("feed", &source, &friendly);
                            tracing::warn!("⚠️ RSS feed unavailable — {feed}: {friendly}");
                            Some(friendly)
                        }
                    };
                    if let (Some(db), Some(error)) = (db.as_ref(), failure) {
                        if let Ok(db) = db.lock() {
                            let _ = db.provider_failure("feed", &source, &error);
                        }
                    }
                    result
                });
            }
        };
        for _ in 0..FEED_CONCURRENCY {
            schedule_feed(&mut feed_set, &mut feed_iter);
        }
        while let Some(joined) = feed_set.join_next().await {
            match joined {
                Ok(Ok(items)) => all.extend(items),
                Ok(Err(_)) => {}
                Err(error) => tracing::warn!(%error, "RSS feed task failed"),
            }
            schedule_feed(&mut feed_set, &mut feed_iter);
        }
        // One readable summary instead of one line per feed.
        let feeds_releases = all.len();
        // Keep the feed outcomes, including failures, for both the readable
        // summary and the detailed debug report printed after indexer searches.
        let feed_stats = crate::logging::take_source_stats();
        let breakdown = source_breakdown(&feed_stats);
        let breakdown = if breakdown.is_empty() {
            "none".to_string()
        } else {
            breakdown
        };
        tracing::info!(
            "🌐 Sources: {feeds_releases} releases from {} feeds — {breakdown}",
            cfg.feed_urls.len()
        );
        // Persist the detail-page cache right after the feed phase: the indexer
        // searches below can take minutes, and a restart would otherwise throw
        // away every magnet resolved in this cycle.
        crate::cache::save();
        let mut targets: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for series in cfg.series.iter().filter(|series| series.enabled) {
            // Pass the real external ids to Torznab: `tvdbid` with the TVDB id
            // and `tmdbid` with the TMDB id (previously the TMDB id was sent
            // mislabelled as tvdbid).
            let mut ids: Vec<(String, String)> = Vec::new();
            if !series.tvdb_id.trim().is_empty() {
                ids.push(("tvdbid".to_string(), series.tvdb_id.trim().to_string()));
            }
            if !series.tmdb_id.trim().is_empty() {
                ids.push(("tmdbid".to_string(), series.tmdb_id.trim().to_string()));
            }
            targets.push((series.name.clone(), ids));
        }
        for movie in cfg.movies.iter().filter(|movie| movie.enabled) {
            targets.push((format!("{} {}", movie.name, movie.year), Vec::new()));
        }
        let targets_total = targets.len();
        tracing::info!(
            "🔎 Step 2/2: searching {} series/movies (Torznab indexers + web engines)",
            targets_total
        );
        let cfg = Arc::new(cfg.clone());
        let mut set = tokio::task::JoinSet::<(String, Vec<Release>)>::new();
        let mut iter = targets.into_iter();
        let schedule = |set: &mut tokio::task::JoinSet<(String, Vec<Release>)>,
                        iter: &mut std::vec::IntoIter<(String, Vec<(String, String)>)>| {
            if let Some((query, ids)) = iter.next() {
                let client = self.client.clone();
                let cfg = cfg.clone();
                let provider_db = provider_db.clone();
                set.spawn(async move {
                    let started = std::time::Instant::now();
                    let items = match tokio::time::timeout(
                        AUTOMATIC_SEARCH_TIMEOUT,
                        search_one_with_db(
                            &client,
                            &cfg,
                            &query,
                            &ids,
                            None,
                            provider_db,
                        ),
                    )
                    .await
                    {
                        Ok(items) => items,
                        Err(_) => {
                            tracing::warn!(
                                query = %query,
                                timeout_secs = AUTOMATIC_SEARCH_TIMEOUT.as_secs(),
                                "scheduled title search timed out"
                            );
                            Vec::new()
                        }
                    };
                    tracing::debug!(
                        query = %query,
                        elapsed_ms = started.elapsed().as_millis(),
                        results = items.len(),
                        "scheduled title search completed"
                    );
                    (query, items)
                });
            }
        };
        for _ in 0..QUERY_CONCURRENCY {
            schedule(&mut set, &mut iter);
        }
        // Conteggio "compatibili": una release è utile solo se supera i filtri
        // globali e quelli della serie/film cercato (lingua, qualità, exclude,
        // sottotitoli). Così il log non annuncia release che poi non verranno
        // mai scaricate.
        let usable = |query: &str, items: &[Release]| -> usize {
            if let Some(series) = cfg.series.iter().find(|series| series.name == query) {
                items
                    .iter()
                    .filter(|release| {
                        cfg.release_allowed(release)
                            && cfg.series_release_allowed(
                                series,
                                &release.quality,
                                &release.title,
                            )
                    })
                    .count()
            } else if let Some(movie) = cfg
                .movies
                .iter()
                .find(|movie| format!("{} {}", movie.name, movie.year) == query)
            {
                items
                    .iter()
                    .filter(|release| {
                        cfg.release_allowed(release)
                            && cfg.movie_release_allowed(movie, &release.quality)
                    })
                    .count()
            } else {
                items
                    .iter()
                    .filter(|release| cfg.release_allowed(release))
                    .count()
            }
        };
        let mut targets_done = 0usize;
        let mut targets_with_hits = 0usize;
        let mut step2_usable = 0usize;
        while let Some(joined) = set.join_next().await {
            if let Ok((query, items)) = joined {
                targets_done += 1;
                let compatible = usable(&query, &items);
                if compatible > 0 {
                    targets_with_hits += 1;
                    step2_usable += compatible;
                    // Per-target detail is diagnostic only. The cycle already
                    // reports the total in "Step 2/2 complete".
                    tracing::debug!(query = %query, compatible, "🔎 compatible releases");
                } else {
                    tracing::debug!(query = %query, found = items.len(), "🔎 search: no matching releases");
                }
                all.extend(items);
            }
            schedule(&mut set, &mut iter);
        }
        tracing::info!(
            "🔎 Step 2/2 complete: {targets_done} targets analyzed · {targets_with_hits} with compatible releases · {step2_usable} compatible releases"
        );
        let engine_failures = websearch::take_engine_failures();
        if !engine_failures.is_empty() {
            let detail = engine_failures
                .iter()
                .map(|(engine, count)| format!("{engine} ({count})"))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!("⚠️ Web engines unreachable in this cycle: {detail}");
        }
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::with_capacity(all.len());
        for release in std::mem::take(&mut all) {
            if let Some(reason) = cfg.all_release_denied_reason(&release) {
                crate::rules::log_rejection(&release, &reason);
                continue;
            }
            // Un feed RSS con solo link `.torrent` (es. TorrentLeech) non ha un
            // magnet: non va scartato qui, la risoluzione dell'infohash avviene
            // nel ciclo. Si deduplica per URL in quel caso.
            let dedup_key = match magnet_hash(&release.magnet) {
                Some(hash) => hash,
                None => match release.torrent_url.as_deref() {
                    Some(url) => format!("url:{url}"),
                    None => {
                        tracing::debug!(
                            title = %release.title,
                            source = %release.source,
                            reason = "missing magnet hash",
                            "filter skipped"
                        );
                        continue;
                    }
                },
            };
            if !seen.insert(dedup_key) {
                tracing::debug!(
                    title = %release.title,
                    source = %release.source,
                    reason = "duplicate infohash",
                    "filter skipped duplicate"
                );
                continue;
            }
            kept.push(release);
        }
        all = kept;
        tracing::info!("✅ Scraping: {} unique releases after filters", all.len());
        let mut source_stats = feed_stats;
        source_stats.extend(crate::logging::take_source_stats());
        print_source_report(source_stats);
        crate::cache::save();
        Ok(all)
    }

    pub async fn search_query(&self, cfg: &Config, query: &str) -> Vec<Release> {
        match tokio::time::timeout(
            AUTOMATIC_SEARCH_TIMEOUT,
            self.search_query_ids(cfg, query, &[]),
        )
        .await
        {
            Ok(items) => items,
            Err(_) => {
                tracing::warn!(
                    query,
                    timeout_secs = AUTOMATIC_SEARCH_TIMEOUT.as_secs(),
                    "gap-fill title search timed out"
                );
                Vec::new()
            }
        }
    }

    /// Ricerca interattiva: gli indexer Torznab restano disponibili per tutta
    /// la loro ricerca, mentre i motori web hanno un budget complessivo breve.
    pub async fn search_query_manual(&self, cfg: &Config, query: &str) -> Vec<Release> {
        search_one_with_db(
            &self.client,
            cfg,
            query,
            &[],
            Some(MANUAL_SEARCH_TIMEOUT),
            self.db.clone(),
        )
        .await
    }

    pub async fn search_query_ids(
        &self,
        cfg: &Config,
        query: &str,
        external_ids: &[(&str, &str)],
    ) -> Vec<Release> {
        let owned: Vec<(String, String)> = external_ids
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        search_one_with_db(&self.client, cfg, query, &owned, None, self.db.clone()).await
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

async fn search_one_with_db(
    client: &Client,
    cfg: &Config,
    query: &str,
    external_ids: &[(String, String)],
    web_timeout: Option<Duration>,
    provider_db: Option<Arc<std::sync::Mutex<crate::database::Database>>>,
) -> Vec<Release> {
    let mut all = Vec::new();
    // Load the disabled set once, then exclude those providers from the fan-out.
    let blocked = provider_db
        .as_ref()
        .and_then(|db| db.lock().ok().map(|db| db.blocked_providers()))
        .and_then(Result::ok)
        .unwrap_or_default();
    let indexers = cfg
        .indexers
        .iter()
        .filter(|indexer| {
            indexer.enabled
                && !blocked.contains(&("indexer".to_string(), indexer.name.clone()))
        })
        .cloned()
        .collect::<Vec<_>>();
    let db = provider_db;
    let web_engines = cfg.websearch_engines.clone();
    let flaresolverr = cfg.flaresolverr_url.clone();
    let indexer_client = client.clone();
    let indexer_query = query.to_owned();
    let indexer_ids = external_ids.to_vec();
    let indexer_search = async move {
        let mut set = tokio::task::JoinSet::new();
        for indexer in indexers {
            let client = indexer_client.clone();
            let query = indexer_query.clone();
            let ids = indexer_ids.clone();
            let flaresolverr = flaresolverr.clone();
            set.spawn(async move {
                tracing::debug!(indexer = %indexer.name, query, "indexer search started");
                let borrowed = ids
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect::<Vec<_>>();
                let result = fetch_torznab_flaresolverr(
                    &client,
                    &indexer,
                    &query,
                    &borrowed,
                    flaresolverr.as_deref(),
                )
                .await;
                (indexer.name, query, result)
            });
        }
        let mut results = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((name, query, Ok(items))) => {
                    crate::logging::source_ok("indexer", &name, items.len());
                    if let Some(db) = db.as_ref() {
                        if let Ok(db) = db.lock() {
                            let _ = db.provider_success("indexer", &name);
                        }
                    }
                    tracing::debug!(indexer = %name, results = items.len(), query, "indexer search completed");
                    results.extend(items);
                }
                Ok((name, query, Err(error))) => {
                    let error = crate::utils::redact_url_secrets(&error.to_string());
                    crate::logging::source_fail("indexer", &name, &error);
                    if let Some(db) = db.as_ref() {
                        if let Ok(db) = db.lock() {
                            let _ = db.provider_failure("indexer", &name, &error);
                        }
                    }
                    tracing::warn!(indexer = %name, query, error = %error, "indexer search failed");
                }
                Err(error) => tracing::warn!(%error, "indexer search task failed"),
            }
        }
        results
    };
    let web_client = client.clone();
    let web_query = query.to_owned();
    let web_flaresolverr = cfg.flaresolverr_url.clone();
    let web_search = async move {
        if web_engines.is_empty() {
            return Vec::new();
        }
        match websearch::search_with_timeout(
            &web_client,
            &web_engines,
            &web_query,
            web_flaresolverr.as_deref(),
            web_timeout,
        )
        .await
        {
            Ok(items) => {
                tracing::debug!(query = %web_query, results = items.len(), "web search completed");
                items
                    .into_iter()
                    .filter_map(|(title, magnet, source)| parse_release(&title, &magnet, &source))
                    .collect()
            }
            Err(error) => {
                let error = crate::utils::redact_url_secrets(&error.to_string());
                tracing::warn!(query = %web_query, error = %error, "web search failed");
                Vec::new()
            }
        }
    };
    let (indexer_results, web_results) = tokio::join!(indexer_search, web_search);
    all.extend(indexer_results);
    all.extend(web_results);
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(all.len());
    for release in all {
        if let Some(reason) = cfg.all_release_denied_reason(&release) {
            crate::rules::log_rejection(&release, &reason);
            continue;
        }
        if magnet_hash(&release.magnet).is_some_and(|hash| seen.insert(hash)) {
            kept.push(release);
        }
    }
    kept
}

#[allow(dead_code)]
fn _indexer_name(indexer: &IndexerConfig) -> &str {
    &indexer.name
}

/// One readable line per source (feed, indexer, web engine) telling the user
/// whether it worked and how many releases it produced. Aggregated per cycle:
/// attempts are repeated for every query, so a raw per-attempt log would flood.
fn print_source_report(stats: Vec<(String, String, crate::logging::SourceStat)>) {
    if stats.is_empty() {
        return;
    }
    // Detailed per-source reporting is useful for diagnostics, but can produce
    // dozens of lines per cycle. The summary ("Sources: ..." and unreachable
    // engine warnings) remains at INFO/WARN level.
    tracing::debug!("📡 SOURCE REPORT — outcomes of every source in this cycle");
    for (kind, name, stat) in stats {
        if stat.fail == 0 {
            tracing::debug!(
                "   ✅ [{kind}] {name}: {} run(s), {} releases",
                stat.ok,
                stat.results
            );
        } else if stat.ok == 0 {
            tracing::debug!(
                "   ❌ [{kind}] {name}: {} failure(s) — {}",
                stat.fail,
                stat.last_error.as_deref().unwrap_or("unknown error")
            );
        } else {
            tracing::debug!(
                "   ⚠️ [{kind}] {name}: {} ok / {} failed, {} releases — {}",
                stat.ok,
                stat.fail,
                stat.results,
                stat.last_error.as_deref().unwrap_or("unknown error")
            );
        }
    }
}

fn source_breakdown(stats: &[(String, String, crate::logging::SourceStat)]) -> String {
    stats
        .iter()
        .filter(|(kind, _, _)| kind == "feed")
        .map(|(_, name, stat)| {
            if stat.fail > 0 && stat.ok == 0 {
                format!("{name}: error")
            } else if stat.fail > 0 {
                format!("{name}: {} ({} error)", stat.results, stat.fail)
            } else {
                format!("{name}: {}", stat.results)
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn feed_source_name(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.contains("ext.to") || lower.contains("extto") {
        "ExtTo".to_string()
    } else if lower.contains("torrentgalaxy") || lower.contains("tgx") {
        "TGx".to_string()
    } else if lower.contains("torrentleech") {
        "TorrentLeech".to_string()
    } else if lower.contains("knaben") {
        "Knaben".to_string()
    } else if lower.contains("eztv") {
        "EZTV".to_string()
    } else if let Ok(parsed) = url::Url::parse(url) {
        parsed.host_str().unwrap_or("feed").to_string()
    } else {
        feed_label(url)
    }
}

#[cfg(test)]
mod tests {
    use super::{feed_source_name, source_breakdown};
    use crate::logging::SourceStat;

    #[test]
    fn names_known_feed_hosts() {
        assert_eq!(feed_source_name("https://rss.knaben.org/ita/feed"), "Knaben");
        assert_eq!(feed_source_name("https://torrentgalaxy.to/rss"), "TGx");
        assert_eq!(
            feed_source_name("https://rss.torrentleech.org/feed"),
            "TorrentLeech"
        );
    }

    #[test]
    fn source_breakdown_includes_failed_feeds() {
        let mut failed = SourceStat::default();
        failed.fail = 1;
        let mut successful = SourceStat::default();
        successful.ok = 1;
        successful.results = 650;
        let stats = vec![
            ("feed".into(), "Knaben".into(), failed),
            ("feed".into(), "TGx".into(), successful),
        ];

        assert_eq!(source_breakdown(&stats), "Knaben: error | TGx: 650");
    }
}

fn feed_label(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.split('?').next().unwrap_or(url).to_string();
    };
    let host = parsed.host_str().unwrap_or("feed");
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() {
        host.to_string()
    } else {
        format!("{host}{path}")
    }
}
