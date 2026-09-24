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
        let schedule_feed = |set: &mut tokio::task::JoinSet<Result<Vec<Release>>>,
                             iter: &mut std::vec::IntoIter<String>| {
            if let Some(url) = iter.next() {
                let client = self.client.clone();
                let flaresolverr = flaresolverr.clone();
                let feed = feed_label(&url);
                set.spawn(async move {
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
                    match &result {
                        Ok(items) => {
                            crate::logging::source_ok("feed", &feed, items.len());
                            tracing::debug!(feed = %feed, items = items.len(), "rss feed analyzed")
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
                            crate::logging::source_fail("feed", &feed, &friendly);
                            tracing::warn!("⚠️ RSS feed unavailable — {feed}: {friendly}")
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
                Err(error) => tracing::warn!(%error, "rss feed task failed"),
            }
            schedule_feed(&mut feed_set, &mut feed_iter);
        }
        // One readable summary instead of one line per feed.
        let feeds_releases = all.len();
        let mut by_source: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for release in &all {
            let raw = release.source.split(" - ").next().unwrap_or("feed");
            // Generic RSS feeds carry the whole URL as source: show just the host.
            let label = url::Url::parse(raw)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_else(|| raw.to_string());
            *by_source.entry(label).or_default() += 1;
        }
        let breakdown = by_source
            .iter()
            .map(|(label, count)| format!("{label}: {count}"))
            .collect::<Vec<_>>()
            .join(" | ");
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
                set.spawn(async move {
                    let started = std::time::Instant::now();
                    let items = match tokio::time::timeout(
                        AUTOMATIC_SEARCH_TIMEOUT,
                        search_one(&client, &cfg, &query, &ids, None),
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
                            && Config::series_release_allowed(
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
                            && Config::movie_release_allowed(movie, &release.quality)
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
                    // Per-target detail: utile solo in diagnosi. Il ciclo riporta
                    // già il totale in "Step 2/2 completato".
                    tracing::debug!(query = %query, compatible, "🔎 release compatibili");
                } else {
                    tracing::debug!(query = %query, found = items.len(), "🔎 search: no matching releases");
                }
                all.extend(items);
            }
            schedule(&mut set, &mut iter);
        }
        tracing::info!(
            "🔎 Step 2/2 completato: {targets_done} target analizzati · {targets_with_hits} con release compatibili · {step2_usable} release compatibili"
        );
        let engine_failures = websearch::take_engine_failures();
        if !engine_failures.is_empty() {
            let detail = engine_failures
                .iter()
                .map(|(engine, count)| format!("{engine} ({count})"))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!("⚠️ Unreachable web engines this cycle: {detail}");
        }
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::with_capacity(all.len());
        for release in std::mem::take(&mut all) {
            if let Some(reason) = cfg.release_denied_reason(&release) {
                tracing::debug!(
                    title = %release.title,
                    source = %release.source,
                    reason,
                    "🚫 FILTER rejected"
                );
                continue;
            }
            if let Some(reason) = cfg.source_filter_denied_reason(&release) {
                tracing::debug!(
                    title = %release.title,
                    source = %release.source,
                    reason = %reason,
                    "🚫 FILTER rejected"
                );
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
        print_source_report();
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
        search_one(&self.client, cfg, query, &[], Some(MANUAL_SEARCH_TIMEOUT)).await
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
        search_one(&self.client, cfg, query, &owned, None).await
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

async fn search_one(
    client: &Client,
    cfg: &Config,
    query: &str,
    external_ids: &[(String, String)],
    web_timeout: Option<Duration>,
) -> Vec<Release> {
    let mut all = Vec::new();
    let indexers = cfg
        .indexers
        .iter()
        .filter(|indexer| indexer.enabled)
        .cloned()
        .collect::<Vec<_>>();
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
                    tracing::debug!(indexer = %name, results = items.len(), query, "indexer search completed");
                    results.extend(items);
                }
                Ok((name, query, Err(error))) => {
                    let error = crate::utils::redact_url_secrets(&error.to_string());
                    crate::logging::source_fail("indexer", &name, &error);
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
    all.retain(|release| {
        cfg.release_allowed(release)
            && magnet_hash(&release.magnet).is_some_and(|hash| seen.insert(hash))
    });
    all
}

#[allow(dead_code)]
fn _indexer_name(indexer: &IndexerConfig) -> &str {
    &indexer.name
}

/// One readable line per source (feed, indexer, web engine) telling the user
/// whether it worked and how many releases it produced. Aggregated per cycle:
/// attempts are repeated for every query, so a raw per-attempt log would flood.
fn print_source_report() {
    let stats = crate::logging::take_source_stats();
    if stats.is_empty() {
        return;
    }
    // Report dettagliato per sorgente: utile in diagnosi, ma sono decine di
    // righe per ciclo. Il riepilogo ("Sources: ..." e i warning sugli engine
    // irraggiungibili) resta a INFO/WARN.
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
