use crate::{
    config::IndexerConfig,
    models::Release,
    parser::{parse_release_at, parse_release_source},
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use reqwest::Client;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

fn local_name(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn magnet_in(value: &str) -> Option<String> {
    let start = value.find("magnet:?")?;
    let candidate = &value[start..];
    let end = candidate
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '<' | '>' | '"' | '\'' | ')')
        })
        .unwrap_or(candidate.len());
    let magnet = &candidate[..end];
    magnet.starts_with("magnet:?").then(|| magnet.to_owned())
}

fn attribute(start: &BytesStart<'_>, name: &str) -> Option<String> {
    start.attributes().flatten().find_map(|attribute| {
        (local_name(attribute.key.as_ref()) == name)
            .then(|| String::from_utf8_lossy(&attribute.value).into_owned())
    })
}

/// Reads a Torznab `<...:attr name="seeders" value="5"/>` element into the
/// per-item facts. Unknown or non-numeric values leave the current value
/// untouched (they stay `None`).
fn apply_torznab_attr(
    start: &BytesStart<'_>,
    size_bytes: &mut Option<f64>,
    seeders: &mut Option<i64>,
    peers: &mut Option<i64>,
) {
    let name = attribute(start, "name")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let value = attribute(start, "value").unwrap_or_default();
    match name.as_str() {
        "seeders" => *seeders = value.trim().parse::<i64>().ok().filter(|value| *value >= 0),
        "peers" | "leechers" => *peers = value.trim().parse::<i64>().ok().filter(|value| *value >= 0),
        "size" => *size_bytes = value.trim().parse::<f64>().ok().filter(|value| *value > 0.0),
        _ => {}
    }
}

fn parse_feed_body(body: &str, source: &str) -> Result<Vec<Release>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut current = String::new();
    let mut in_item = false;
    let mut title = String::new();
    let mut magnet = String::new();
    let mut torrent_url = String::new();
    let mut description = String::new();
    let mut size_bytes: Option<f64> = None;
    // Torznab attributes (`<torznab:attr name="seeders" value="5"/>`) or plain
    // `<seeders>` elements. Unknown stays `None` so policy rules that need a
    // peer count simply do not fire instead of rejecting a release.
    let mut seeders: Option<i64> = None;
    let mut peers: Option<i64> = None;
    let mut discovered_at = Utc::now();
    let mut out = Vec::new();
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(error) if !out.is_empty() => {
                // Some RSS endpoints keep the connection open after sending a
                // useful prefix. Preserve the complete items already parsed
                // instead of discarding them because the XML tail is missing.
                tracing::debug!(%error, items = out.len(), "accepting partial RSS body");
                break;
            }
            Err(error) => return Err(error.into()),
        };
        match event {
            Event::Start(start) => {
                let name = local_name(start.name().as_ref());
                if name == "item" || name == "entry" {
                    in_item = true;
                    title.clear();
                    magnet.clear();
                    torrent_url.clear();
                    description.clear();
                    size_bytes = None;
                    seeders = None;
                    peers = None;
                    discovered_at = Utc::now();
                }
                if in_item {
                    if name == "enclosure" {
                        if let Some(value) = attribute(&start, "length") {
                            size_bytes = value.trim().parse::<f64>().ok().filter(|v| *v > 0.0);
                        }
                    }
                    if name == "attr" {
                        apply_torznab_attr(&start, &mut size_bytes, &mut seeders, &mut peers);
                    }
                    if let Some(value) =
                        attribute(&start, "href").or_else(|| attribute(&start, "url"))
                    {
                        if let Some(found) = magnet_in(&value) {
                            magnet = found;
                        } else if crate::parser::is_torrent_url(&value) {
                            torrent_url = value;
                        }
                    }
                }
                current = name;
            }
            Event::Empty(start) if in_item => {
                let name = local_name(start.name().as_ref());
                if name == "enclosure" {
                    if let Some(value) = attribute(&start, "length") {
                        size_bytes = value.trim().parse::<f64>().ok().filter(|v| *v > 0.0);
                    }
                }
                if name == "attr" {
                    apply_torznab_attr(&start, &mut size_bytes, &mut seeders, &mut peers);
                }
                if name == "link" || name == "enclosure" || name == "content" {
                    if let Some(value) =
                        attribute(&start, "href").or_else(|| attribute(&start, "url"))
                    {
                        if let Some(found) = magnet_in(&value) {
                            magnet = found;
                        } else if crate::parser::is_torrent_url(&value) {
                            torrent_url = value;
                        }
                    }
                }
            }
            Event::Text(text) if in_item => {
                let value = text.unescape()?.into_owned();
                if current == "title" {
                    title = value.clone();
                }
                if current == "description" {
                    description.push_str(&value);
                    description.push(' ');
                }
                if current == "size" {
                    size_bytes = value.trim().parse::<f64>().ok().filter(|v| *v > 0.0);
                }
                if current == "seeders" {
                    seeders = value.trim().parse::<i64>().ok().filter(|v| *v >= 0);
                }
                if current == "peers" || current == "leechers" {
                    peers = value.trim().parse::<i64>().ok().filter(|v| *v >= 0);
                }
                if matches!(
                    current.as_str(),
                    "pubdate" | "published" | "updated" | "date"
                ) {
                    discovered_at = DateTime::parse_from_rfc2822(&value)
                        .map(|value| value.with_timezone(&Utc))
                        .or_else(|_| {
                            DateTime::parse_from_rfc3339(&value)
                                .map(|value| value.with_timezone(&Utc))
                        })
                        .unwrap_or(discovered_at);
                }
                if let Some(found) = magnet_in(&value) {
                    magnet = found;
                }
                if current == "link" && crate::parser::is_torrent_url(&value) {
                    torrent_url = value;
                }
            }
            Event::CData(text) if in_item => {
                let value = String::from_utf8_lossy(&text).into_owned();
                if current == "title" {
                    title = value.clone();
                }
                if current == "description" {
                    description.push_str(&value);
                    description.push(' ');
                }
                if current == "size" {
                    size_bytes = value.trim().parse::<f64>().ok().filter(|v| *v > 0.0);
                }
                if current == "seeders" {
                    seeders = value.trim().parse::<i64>().ok().filter(|v| *v >= 0);
                }
                if current == "peers" || current == "leechers" {
                    peers = value.trim().parse::<i64>().ok().filter(|v| *v >= 0);
                }
                if matches!(
                    current.as_str(),
                    "pubdate" | "published" | "updated" | "date"
                ) {
                    discovered_at = DateTime::parse_from_rfc2822(&value)
                        .map(|value| value.with_timezone(&Utc))
                        .or_else(|_| {
                            DateTime::parse_from_rfc3339(&value)
                                .map(|value| value.with_timezone(&Utc))
                        })
                        .unwrap_or(discovered_at);
                }
                if let Some(found) = magnet_in(&value) {
                    magnet = found;
                }
                if current == "link" && crate::parser::is_torrent_url(&value) {
                    torrent_url = value;
                }
            }
            Event::End(end) => {
                let name = local_name(end.name().as_ref());
                if in_item && (name == "item" || name == "entry") {
                    // size sanity (legacy `_sanity_check`): drop samples/NFO.
                    let size_mb = size_bytes
                        .map(|bytes| bytes / 1_048_576.0)
                        .or_else(|| {
                            let mb = crate::utils::parse_size_mb(&description);
                            (mb > 0.0).then_some(mb)
                        });
                    if !size_mb.is_some_and(|mb| mb < 50.0) {
                        if let Some(mut release) = parse_release_source(
                            &title,
                            &magnet,
                            (!torrent_url.is_empty()).then_some(torrent_url.as_str()),
                            source,
                            discovered_at,
                        ) {
                            release.size_bytes = size_bytes
                                .map(|bytes| bytes.round() as i64)
                                .filter(|bytes| *bytes > 0)
                                .unwrap_or(0);
                            release.seeders = seeders.unwrap_or(-1);
                            release.peers = peers.unwrap_or(-1);
                            out.push(release);
                        }
                    }
                    in_item = false;
                }
                current.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn is_cloudflare_challenge(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("just a moment")
        || lower.contains("cf-challenge")
        || lower.contains("enable javascript and cookies")
        || lower.contains("attention required! | cloudflare")
}

/// Cloudflare cookies and User-Agent learned from FlareSolverr, keyed by
/// domain. legacy copies them into its `requests` session so that detail pages
/// can be fetched directly instead of paying a FlareSolverr round-trip each
/// time; the same trick is applied here to the shared HTTP client.
#[derive(Clone)]
struct CfSession {
    user_agent: String,
    cookie: String,
}

static CF_SESSIONS: LazyLock<Mutex<HashMap<String, CfSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Aggregate concurrency per host. A single feed keeps its own small limit, but
/// many feeds run at once (one per configured source) and detail scraping can
/// fan out to dozens of requests; without a shared cap a host starts answering
/// `429 Too Many Requests`. 8 concurrent requests per host is well within what
/// torrentgalaxy/ext.to tolerate in testing.
const HOST_MAX_CONCURRENCY: usize = 8;

static HOST_LIMITS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn domain_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
}

fn host_semaphore(url: &str) -> Option<Arc<tokio::sync::Semaphore>> {
    let domain = domain_of(url)?;
    let mut limits = HOST_LIMITS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Some(
        limits
            .entry(domain)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(HOST_MAX_CONCURRENCY)))
            .clone(),
    )
}

/// Minimum spacing between request starts to the same host. Concurrency alone
/// is not enough: torrentgalaxy answers `429` when a burst of parallel requests
/// arrives, and tests showed 50 detail pages succeed with a ~250 ms/4-way pace
/// while an unpaced burst fails. 250 ms caps the host at 4 req/s.
const HOST_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

static HOST_LAST_REQUEST: LazyLock<Mutex<HashMap<String, tokio::time::Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Waits until the per-host start interval has elapsed, then claims the slot.
async fn throttle_host(url: &str) {
    let Some(domain) = domain_of(url) else {
        return;
    };
    loop {
        let wait = {
            let mut last = HOST_LAST_REQUEST
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = tokio::time::Instant::now();
            match last.get(&domain).copied() {
                Some(previous) if previous + HOST_MIN_INTERVAL > now => {
                    Some(previous + HOST_MIN_INTERVAL - now)
                }
                _ => {
                    last.insert(domain.clone(), now);
                    None
                }
            }
        };
        match wait {
            Some(delay) => tokio::time::sleep(delay).await,
            None => return,
        }
    }
}

/// Pauses a host after a `429` so the next requests do not extend the penalty.
fn penalize_host(url: &str, penalty: std::time::Duration) {
    let Some(domain) = domain_of(url) else {
        return;
    };
    let mut last = HOST_LAST_REQUEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    last.insert(domain, tokio::time::Instant::now() + penalty);
}

fn session_for(url: &str) -> Option<CfSession> {
    let domain = domain_of(url)?;
    CF_SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&domain)
        .cloned()
}

fn remember_session(url: &str, user_agent: &str, cookies: &serde_json::Value) {
    let Some(domain) = domain_of(url) else {
        return;
    };
    let cookie = cookies
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|cookie| {
                    let name = cookie.get("name")?.as_str()?;
                    let value = cookie.get("value")?.as_str()?;
                    Some(format!("{name}={value}"))
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    if user_agent.is_empty() && cookie.is_empty() {
        return;
    }
    let mut sessions = CF_SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = sessions.entry(domain).or_insert_with(|| CfSession {
        user_agent: String::new(),
        cookie: String::new(),
    });
    if !user_agent.is_empty() {
        entry.user_agent = user_agent.to_owned();
    }
    if !cookie.is_empty() {
        entry.cookie = cookie;
    }
}

async fn fetch_with_flaresolverr(client: &Client, flaresolverr: &str, url: &str) -> Result<String> {
    let _permit = crate::utils::FLARESOLVERR_LIMITER
        .acquire()
        .await
        .map_err(|_| anyhow::anyhow!("FlareSolverr request limiter closed"))?;
    let endpoint = format!("{}/v1", flaresolverr.trim_end_matches('/'));
    let response = client
        .post(&endpoint)
        .json(&serde_json::json!({"cmd": "request.get", "url": url, "maxTimeout": 20000}))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?;
    let value: serde_json::Value = response.json().await?;
    let solution = value.get("solution");
    remember_session(
        url,
        solution
            .and_then(|solution| solution.get("userAgent"))
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        solution
            .and_then(|solution| solution.get("cookies"))
            .unwrap_or(&serde_json::Value::Null),
    );
    let body = solution
        .and_then(|solution| solution.get("response"))
        .and_then(|response| response.as_str())
        .unwrap_or_default()
        .to_owned();
    if body.is_empty() {
        anyhow::bail!("flaresolverr returned an empty response");
    }
    Ok(body)
}

/// HTTP statuses that mean "Cloudflare is in front of the origin", where a
/// real browser (FlareSolverr) can succeed. Random statuses such as 404 or 429
/// must NOT trigger FlareSolverr: rate limiting in particular is made worse by
/// hammering it with a headless browser request.
fn cloudflare_blocked(status: u16) -> bool {
    matches!(status, 403 | 503 | 520..=530)
}

async fn flaresolverr_or(
    client: &Client,
    flaresolverr: &str,
    url: &str,
    reason: &str,
) -> Result<String> {
    tracing::info!(
        feed_url = %url,
        flaresolverr = %flaresolverr,
        reason = %reason,
        "trying FlareSolverr"
    );
    match fetch_with_flaresolverr(client, flaresolverr, url).await {
        Ok(body) => Ok(body),
        Err(error) => {
            tracing::warn!(
                feed_url = %url,
                flaresolverr = %flaresolverr,
                %error,
                "FlareSolverr RSS fallback failed"
            );
            Err(error)
        }
    }
}

/// Tentativi per scaricare il corpo di un feed: Cloudflare a volte chiude lo
/// stream a metà ("error decoding response body" è proprio questo). I feed sono
/// testi piccoli, quindi in caso di errore transitorio si può riscaricare.
const FEED_FETCH_ATTEMPTS: u32 = 3;
/// Timeout del singolo tentativo: il vecchio limite di 10s poteva troncare i
/// feed grandi su linea lenta.
const FEED_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Knaben keeps streaming a very large RSS response instead of closing it
/// promptly. The browser can render the first entries while that happens; do
/// the same here and stop after one normal listing page.
const KNABEN_STREAM_ITEM_LIMIT: usize = 50;
const KNABEN_STREAM_BODY_LIMIT: usize = 512 * 1024;

/// Esito di un singolo tentativo diretto (senza FlareSolverr), per decidere se
/// ritentare, passare a FlareSolverr o arrendersi.
enum FetchAttempt {
    Body(String),
    Cloudflare(String),
    Transient(String),
    Fatal(String),
}

async fn fetch_body_direct(
    client: &Client,
    url: &str,
    timeout: std::time::Duration,
) -> FetchAttempt {
    let mut request = client.get(url).timeout(timeout);
    if let Some(session) = session_for(url) {
        if !session.user_agent.is_empty() {
            request = request.header(reqwest::header::USER_AGENT, session.user_agent);
        }
        if !session.cookie.is_empty() {
            request = request.header(reqwest::header::COOKIE, session.cookie);
        }
    }
    // Cap aggregate concurrency per host; il permit viene rilasciato prima di un
    // eventuale retry o chiamata FlareSolverr (che parla con un altro host).
    let host_permit = match host_semaphore(url) {
        Some(semaphore) => match semaphore.acquire_owned().await {
            Ok(permit) => Some(permit),
            Err(_) => return FetchAttempt::Fatal("host request limiter closed".into()),
        },
        None => None,
    };
    throttle_host(url).await;
    let direct = request.send().await;
    drop(host_permit);
    let mut response = match direct {
        Ok(response) => response,
        Err(error) => return FetchAttempt::Transient(error.to_string()),
    };
    let status = response.status();
    if status.is_success() {
        let body = if domain_of(url).is_some_and(|host| host.ends_with("knaben.org")) {
            read_knaben_stream(&mut response).await
        } else {
            response.text().await.map_err(|error| error.to_string())
        };
        return match body {
            Ok(body) if is_cloudflare_challenge(&body) => {
                FetchAttempt::Cloudflare("cloudflare challenge body".into())
            }
            Ok(body) => FetchAttempt::Body(body),
            // Stream chiuso a metà: errore transitorio, si ritenta.
            Err(error) => FetchAttempt::Transient(error),
        };
    }
    if cloudflare_blocked(status.as_u16()) {
        return FetchAttempt::Cloudflare(format!("direct status {status}"));
    }
    if status.as_u16() == 429 {
        penalize_host(url, std::time::Duration::from_secs(2));
        tracing::warn!(feed_url = %url, "host rate limited the request (429)");
        return FetchAttempt::Transient(format!("HTTP {status}"));
    }
    if status.is_server_error() {
        return FetchAttempt::Transient(format!("HTTP {status}"));
    }
    FetchAttempt::Fatal(format!("HTTP {status}"))
}

/// Read the useful prefix of Knaben's streaming RSS response. Its endpoint can
/// keep producing items for a long time without sending the closing `</rss>`;
/// waiting for `Response::text()` would make an otherwise usable feed hit the
/// global cycle budget.
async fn read_knaben_stream(response: &mut reqwest::Response) -> std::result::Result<String, String> {
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| error.to_string())?;
        let Some(chunk) = chunk else {
            break;
        };
        body.extend_from_slice(&chunk);

        let item_count = body
            .windows(b"</item>".len())
            .filter(|window| window == b"</item>")
            .count();
        if item_count >= KNABEN_STREAM_ITEM_LIMIT || body.len() >= KNABEN_STREAM_BODY_LIMIT {
            break;
        }
    }

    if body.is_empty() {
        return Err("empty Knaben RSS response".into());
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Scarica il corpo di un feed con qualche tentativo sui soli errori
/// transitori. Cloudflare è l'ultima spiaggia, dopo i retry diretti, così non
/// viene martellato ad ogni micro-errore.
async fn fetch_body(client: &Client, url: &str, flaresolverr: Option<&str>) -> Result<String> {
    let mut last_transient = String::new();
    for attempt in 1..=FEED_FETCH_ATTEMPTS {
        match fetch_body_direct(client, url, FEED_FETCH_TIMEOUT).await {
            FetchAttempt::Body(body) => return Ok(body),
            FetchAttempt::Cloudflare(reason) => {
                // Non è un problema di rete transitorio: lascia fare a FlareSolverr.
                let Some(flaresolverr) = flaresolverr else {
                    anyhow::bail!("{reason}");
                };
                return flaresolverr_or(client, flaresolverr, url, &reason).await;
            }
            FetchAttempt::Fatal(error) => anyhow::bail!("{error}"),
            FetchAttempt::Transient(error) => {
                last_transient = error;
                if attempt < FEED_FETCH_ATTEMPTS {
                    tracing::debug!(
                        attempt,
                        attempts = FEED_FETCH_ATTEMPTS,
                        error = %last_transient,
                        "feed fetch attempt failed; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
                }
            }
        }
    }
    // Stream ostinato: ultima spiaggia FlareSolverr, se configurato.
    if let Some(flaresolverr) = flaresolverr {
        return flaresolverr_or(client, flaresolverr, url, "direct attempts exhausted").await;
    }
    anyhow::bail!("{last_transient}")
}

pub async fn fetch_feed(
    client: &Client,
    url: &str,
    flaresolverr: Option<&str>,
    max_pages: usize,
    max_age_days: i64,
    old_ratio: f64,
) -> Result<Vec<Release>> {
    // ext.to / Corsaro / TorrentGalaxy are HTML listings, not RSS: parsing them
    // as XML only produced a misleading "feed is not valid RSS" line and wasted
    // work. Only generic feeds go through the RSS parser.
    let lower = url.to_ascii_lowercase();
    let kind = if lower.contains("ext.to") || lower.contains("extto") {
        "ExtTo"
    } else if lower.contains("corsaro") {
        "Corsaro"
    } else if lower.contains("torrentgalaxy") {
        "TorrentGalaxy"
    } else {
        let body = fetch_body(client, url, flaresolverr).await?;
        return parse_feed_body(&body, url);
    };
    let body = fetch_body(client, url, flaresolverr).await?;
    let label = source_label(kind, url);
    // TorrentGalaxy uploader listings link to `/post-detail/...` pages instead
    // of carrying magnets, so they need the dedicated parser (legacy `_tgx_user`).
    if kind == "TorrentGalaxy" {
        return fetch_tgx_listing(client, &body, url, &label, flaresolverr).await;
    }
    // Age filter + early-stop (legacy `max_age_days` / `stop_on_old_page_threshold`).
    let cutoff = (max_age_days > 0)
        .then(|| Utc::now() - chrono::Duration::days(max_age_days));
    let mut all = Vec::new();
    let mut old_total = 0_usize;
    let mut page_total = 0_usize;
    // `feed_max_pages`: number of listing pages to walk (legacy MAX_PAGES).
    for page in 0..max_pages.max(1) {
        let page_url = if page == 0 {
            url.to_string()
        } else {
            format!(
                "{}{}page={}",
                url,
                if url.contains('?') { '&' } else { '?' },
                page
            )
        };
        let page_body = if page == 0 {
            body.clone()
        } else {
            match fetch_body(client, &page_url, flaresolverr).await {
                Ok(body) => body,
                Err(_) => break,
            }
        };
        let (items, old, total) = fetch_traditional_listing(
            client,
            &page_url,
            &page_body,
            kind,
            &label,
            flaresolverr,
            cutoff,
        )
        .await?;
        if items.is_empty() && total == 0 {
            break;
        }
        all.extend(items);
        old_total += old;
        page_total += total;
        if cutoff.is_some() && page_total > 0 {
            let ratio = old_total as f64 / page_total as f64;
            if ratio >= old_ratio {
                tracing::info!(
                    feed_url = %url,
                    page,
                    old = old_total,
                    total = page_total,
                    "listing early-stop: page is mostly older than the age limit"
                );
                break;
            }
        }
    }
    Ok(all)
}

async fn fetch_traditional_listing(
    client: &Client,
    url: &str,
    body: &str,
    kind: &str,
    label: &str,
    flaresolverr: Option<&str>,
    cutoff: Option<DateTime<Utc>>,
) -> Result<(Vec<Release>, usize, usize)> {
    let selector = if kind == "ExtTo" {
        scraper::Selector::parse("a.torrent-title-link, a[href^='magnet:']")
    } else {
        scraper::Selector::parse("a[href*='/torrent/'], a[href*='/torrents/'], a[href^='magnet:']")
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let base = url::Url::parse(url)?;
    let links = {
        let document = scraper::Html::parse_document(body);
        document
            .select(&selector)
            // legacy walks 50 items per page; ext.to listings carry exactly 50
            // direct magnets, so a lower cap silently drops releases.
            .take(50)
            .map(|link| {
                (
                    link.text().collect::<String>().trim().to_owned(),
                    link.value().attr("href").unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let page_total = links.len();
    let mut output = Vec::new();
    let mut pending: Vec<(String, url::Url)> = Vec::new();
    for (title, href) in links {
        if href.starts_with("magnet:") {
            // ext.to download buttons have no text: recover the title from the
            // magnet `dn=` parameter, exactly like legacy's listing fallback.
            let title = if title.is_empty() {
                title_from_magnet(&href).unwrap_or_default()
            } else {
                title
            };
            push_release(&mut output, &title, &href, label);
            continue;
        }
        if title.is_empty() {
            continue;
        }
        if pending.len() >= 12 {
            continue;
        }
        if let Ok(detail) = base.join(&href) {
            pending.push((title, detail));
        }
    }
    let old = fetch_detail_magnets(client, pending, label, flaresolverr, &mut output, cutoff).await;
    Ok((output, old, page_total))
}

/// Parses TorrentGalaxy `div.tgxtablerow` rows into `(title, detail_url)`
/// pairs. Kept synchronous so the non-`Send` scraper document never crosses
/// an await point.
fn tgx_detail_links(body: &str, base: &url::Url) -> Vec<(String, url::Url)> {
    let Ok(row_selector) = scraper::Selector::parse("div.tgxtablerow") else {
        return Vec::new();
    };
    let Ok(link_selector) =
        scraper::Selector::parse("a.txlight[href*='post-detail'], a[href*='post-detail']")
    else {
        return Vec::new();
    };
    let document = scraper::Html::parse_document(body);
    let mut pending = Vec::new();
    for row in document.select(&row_selector).take(50) {
        let Some(link) = row.select(&link_selector).next() else {
            continue;
        };
        let title = link.text().collect::<String>().trim().to_owned();
        let href = link.value().attr("href").unwrap_or_default();
        if title.is_empty() || href.is_empty() {
            continue;
        }
        if let Ok(detail) = base.join(href) {
            pending.push((title, detail));
        }
    }
    pending
}

/// TorrentGalaxy uploader pages (`/get-posts/user:NAME/`) list rows in
/// `div.tgxtablerow` linking to `/post-detail/<id>/<slug>/`; the magnet lives
/// on the detail page. Ports legacy's `_tgx_user` parser.
async fn fetch_tgx_listing(
    client: &Client,
    body: &str,
    url: &str,
    label: &str,
    flaresolverr: Option<&str>,
) -> Result<Vec<Release>> {
    let base = url::Url::parse(url)?;
    // Keep the (non-`Send`) scraper document inside its own scope so it is
    // dropped before the awaits below.
    let pending = tgx_detail_links(body, &base);
    if pending.is_empty() {
        tracing::warn!(feed_url = %url, "torrentgalaxy listing returned no post-detail rows");
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    // legacy's `_tgx_user` does not apply the age filter.
    fetch_detail_magnets(client, pending, label, flaresolverr, &mut output, None).await;
    tracing::debug!(feed_url = %url, label = %label, items = output.len(), "torrentgalaxy listing parsed");
    Ok(output)
}

/// Resolves detail pages to magnets, reusing the persistent cache by title so
/// already-seen releases cost no HTTP request. Ports legacy's `SmartCache` use.
/// Returns how many fetched details were older than `cutoff` (age filter).
async fn fetch_detail_magnets(
    client: &Client,
    pending: Vec<(String, url::Url)>,
    label: &str,
    flaresolverr: Option<&str>,
    output: &mut Vec<Release>,
    cutoff: Option<DateTime<Utc>>,
) -> usize {
    let mut old = 0_usize;
    let mut misses: Vec<(String, url::Url)> = Vec::new();
    for (title, detail) in pending {
        match crate::cache::get(&title) {
            // Una civetta in cache non vale: la si risolve di nuovo dal dettaglio.
            Some(magnet) if !is_placeholder_magnet(&magnet) => {
                push_release(output, &title, &magnet, label)
            }
            _ => misses.push((title, detail)),
        }
    }
    if misses.is_empty() {
        return old;
    }
    let mut set = tokio::task::JoinSet::new();
    let mut iter = misses.into_iter();
    let schedule = |set: &mut tokio::task::JoinSet<(String, Option<String>)>,
                    iter: &mut std::vec::IntoIter<(String, url::Url)>| {
        if let Some((title, detail)) = iter.next() {
            let client = client.clone();
            let flaresolverr = flaresolverr.map(str::to_owned);
            set.spawn(async move {
                let body = fetch_body(&client, detail.as_str(), flaresolverr.as_deref())
                    .await
                    .ok();
                (title, body)
            });
        }
    };
    for _ in 0..6 {
        schedule(&mut set, &mut iter);
    }
    while let Some(joined) = set.join_next().await {
        if let Ok((title, Some(body))) = joined {
            if let Some(magnet) = extract_magnet(&body) {
                let discovered_at = crate::utils::parse_date_any(&body).unwrap_or_else(Utc::now);
                if cutoff.is_some_and(|limit| discovered_at < limit) {
                    old += 1;
                } else {
                    crate::cache::set(title.clone(), magnet.clone());
                    push_release_at(output, &title, &magnet, label, discovered_at);
                }
            }
        }
        schedule(&mut set, &mut iter);
    }
    old
}

fn push_release(output: &mut Vec<Release>, title: &str, magnet: &str, label: &str) {
    push_release_at(output, title, magnet, label, Utc::now());
}

fn push_release_at(
    output: &mut Vec<Release>,
    title: &str,
    magnet: &str,
    label: &str,
    discovered_at: DateTime<Utc>,
) {
    // La civetta anti-bot di ext.to non è un magnet reale: non archiviarla.
    if is_placeholder_magnet(magnet) {
        return;
    }
    let tagged = with_source_tag(title, label);
    if let Some(release) = parse_release_at(&tagged, magnet, label, discovered_at) {
        output.push(release);
    }
}

/// legacy `t_display`: append the source tag to the title so the uploader
/// survives into the archive and the generated magnet feed.
fn with_source_tag(title: &str, label: &str) -> String {
    if label.is_empty() || title.contains(&format!("[{label}]")) {
        title.to_string()
    } else {
        format!("{title} [{label}]")
    }
}

/// Recovers a readable title from the magnet `dn=` parameter. ext.to listing
/// download anchors carry no text, so this is the only source for the title.
fn title_from_magnet(magnet: &str) -> Option<String> {
    let parsed = url::Url::parse(magnet).ok()?;
    parsed
        .query_pairs()
        .find(|(key, _)| key == "dn")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.trim().is_empty())
}

/// Per-uploader source label, matching legacy (`ExtTo - user`, `TGx - user`).
fn source_label(kind: &str, url: &str) -> String {
    let extract = |pattern: &str| {
        crate::utils::cached_regex(pattern)
            .ok()
            .and_then(|regex| regex.captures(url))
            .and_then(|captures| captures.get(1))
            .map(|value| decode_component(value.as_str()))
    };
    match kind {
        "ExtTo" => extract(r"filter=u=([^&]+)")
            .map(|user| format!("ExtTo - {user}"))
            .unwrap_or_else(|| "ExtTo".to_string()),
        "Corsaro" => extract(r"/user/([^/?]+)")
            .map(|user| format!("Corsaro - {user}"))
            .unwrap_or_else(|| "Corsaro".to_string()),
        "TorrentGalaxy" => extract(r"/get-posts/user:([^/?]+)")
            .map(|user| format!("TGx - {user}"))
            .unwrap_or_else(|| "TGx".to_string()),
        other => other.to_string(),
    }
}

fn decode_component(value: &str) -> String {
    url::form_urlencoded::parse(value.as_bytes())
        .map(|(key, _)| key.into_owned())
        .collect::<String>()
}

/// ext.to (e siti simili) usano un magnet civetta anti-bot al posto
/// dell'infohash reale: `btih:areMouseMovesMostlyStraightLined`. Va ignorato,
/// altrimenti il fallback di [`extract_magnet`] lo scambia per un infohash
/// base32 e lo archivia come magnet "corrotto".
fn is_placeholder_magnet(magnet: &str) -> bool {
    magnet
        .to_ascii_lowercase()
        .contains("btih:aremousemovesmostlystraightlined")
}

fn extract_magnet(body: &str) -> Option<String> {
    let direct = crate::utils::cached_regex(r#"magnet:\?xt=urn:bt(?:ih|mh):[0-9A-Za-z]{32,68}[^\s\"'<>]*"#)
        .ok()?
        .find(body)
        .map(|value| value.as_str().to_owned());
    direct
        .or_else(|| {
            crate::utils::cached_regex(r#"(?i)\b([a-f0-9]{40}|[a-z2-7]{32})\b"#)
                .ok()?
                .captures(body)
                .and_then(|capture| capture.get(1))
                .map(|hash| format!("magnet:?xt=urn:btih:{}", hash.as_str()))
        })
        .filter(|magnet| !is_placeholder_magnet(magnet))
}

fn torznab_endpoint(indexer: &IndexerConfig) -> String {
    let base = indexer.url.trim().trim_end_matches('/');
    if base.contains("/api") || base.contains("torznab") {
        return base.to_string();
    }
    // legacy detects the kind from the URL/port (Prowlarr default 9696, Jackett
    // 9117), not from the configured name: a renamed indexer must still work.
    let url = base.to_ascii_lowercase();
    let name = indexer.name.to_ascii_lowercase();
    let is_prowlarr = url.contains("prowlarr") || url.contains(":9696") || name.contains("prowlarr");
    if is_prowlarr {
        format!("{base}/api/v1/search")
    } else {
        format!("{base}/api/v2.0/indexers/all/results/torznab/api")
    }
}

fn parse_prowlarr_json(body: &str, source: &str) -> Result<Vec<Release>> {
    let items: Vec<serde_json::Value> = serde_json::from_str(body)?;
    let mut out = Vec::new();
    for item in items {
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let magnet = item
            .get("magnetUrl")
            .and_then(serde_json::Value::as_str)
            .or_else(|| item.get("downloadUrl").and_then(serde_json::Value::as_str))
            .unwrap_or_default();
        if title.is_empty() || !magnet.starts_with("magnet:") {
            continue;
        }
        let label = item
            .get("indexer")
            .and_then(serde_json::Value::as_str)
            .map(|indexer| format!("prowlarr:{indexer}"))
            .unwrap_or_else(|| source.to_string());
        if let Some(mut release) = crate::parser::parse_release(title, magnet, &label) {
            release.size_bytes = item
                .get("size")
                .and_then(serde_json::Value::as_i64)
                .filter(|size| *size > 0)
                .unwrap_or(0);
            release.seeders = item
                .get("seeders")
                .and_then(serde_json::Value::as_i64)
                .filter(|value| *value >= 0)
                .unwrap_or(-1);
            release.peers = item
                .get("leechers")
                .and_then(serde_json::Value::as_i64)
                .or_else(|| item.get("peers").and_then(serde_json::Value::as_i64))
                .filter(|value| *value >= 0)
                .unwrap_or(-1);
            out.push(release);
        }
    }
    Ok(out)
}

pub async fn fetch_torznab(
    client: &Client,
    indexer: &IndexerConfig,
    query: &str,
) -> Result<Vec<Release>> {
    fetch_torznab_flaresolverr(client, indexer, query, &[], None).await
}

pub async fn fetch_torznab_with(
    client: &Client,
    indexer: &IndexerConfig,
    query: &str,
    external_ids: &[(&str, &str)],
) -> Result<Vec<Release>> {
    fetch_torznab_flaresolverr(client, indexer, query, external_ids, None).await
}

/// Torznab search with optional FlareSolverr fallback when the indexer blocks
/// the request (Cloudflare / 403), like legacy does for ext.to.
pub async fn fetch_torznab_flaresolverr(
    client: &Client,
    indexer: &IndexerConfig,
    query: &str,
    external_ids: &[(&str, &str)],
    flaresolverr: Option<&str>,
) -> Result<Vec<Release>> {
    let endpoint = torznab_endpoint(indexer);
    let is_prowlarr_json = endpoint.ends_with("/api/v1/search");
    let mut url = url::Url::parse(&endpoint)?;
    {
        let mut pairs = url.query_pairs_mut();
        if is_prowlarr_json {
            pairs.append_pair("query", query);
            pairs.append_pair("type", "search");
        } else {
            pairs.append_pair("t", "search");
            pairs.append_pair("q", query);
            pairs.append_pair("extended", "1");
            for (key, value) in external_ids {
                if !value.trim().is_empty() {
                    pairs.append_pair(key, value);
                }
            }
        }
        pairs.append_pair("apikey", &indexer.api_key);
    }
    let full_url = url.to_string();
    let response = client.get(&full_url).send().await?;
    let (content_type, body) = match response.error_for_status() {
        Ok(ok) => {
            let content_type = ok
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            (content_type, ok.text().await?)
        }
        Err(error) => {
            let Some(flaresolverr) = flaresolverr.filter(|value| !value.trim().is_empty()) else {
                return Err(error.into());
            };
            let error = crate::utils::redact_url_secrets(&error.to_string());
            tracing::info!(indexer = %indexer.name, error = %error, "torznab blocked, retrying via FlareSolverr");
            let body = fetch_with_flaresolverr(client, flaresolverr, &full_url).await?;
            let content_type = if body.trim_start().starts_with('[') {
                "application/json".to_string()
            } else {
                "application/xml".to_string()
            };
            (content_type, body)
        }
    };
    // Torznab/Prowlarr replies can be large. XML/JSON decoding also invokes the
    // release parser for every item, all of which is synchronous CPU work. Do
    // not let that monopolize Tokio's workers (and consequently Axum's accept
    // and request tasks) during the scheduled fan-out.
    let source = indexer.name.clone();
    tokio::task::spawn_blocking(move || {
        if content_type.contains("json") || body.trim_start().starts_with('[') {
            parse_prowlarr_json(&body, &source)
        } else {
            parse_feed_body(&body, &source)
        }
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_torznab_endpoints_for_jackett_and_prowlarr() {
        let jackett = IndexerConfig {
            name: "jackett".into(),
            url: "http://host:9117".into(),
            api_key: "k".into(),
            enabled: true,
        };
        assert_eq!(
            torznab_endpoint(&jackett),
            "http://host:9117/api/v2.0/indexers/all/results/torznab/api"
        );
        let prowlarr = IndexerConfig {
            name: "prowlarr".into(),
            url: "http://host:9696".into(),
            api_key: "k".into(),
            enabled: true,
        };
        assert_eq!(
            torznab_endpoint(&prowlarr),
            "http://host:9696/api/v1/search"
        );
        let explicit = IndexerConfig {
            name: "custom".into(),
            url: "http://host:9696/12/api".into(),
            api_key: "k".into(),
            enabled: true,
        };
        assert_eq!(torznab_endpoint(&explicit), "http://host:9696/12/api");
        // The kind is detected from the port even when the name is arbitrary.
        let renamed = IndexerConfig {
            name: "my-indexer".into(),
            url: "http://host:9696".into(),
            api_key: "k".into(),
            enabled: true,
        };
        assert_eq!(torznab_endpoint(&renamed), "http://host:9696/api/v1/search");
        let jackett_renamed = IndexerConfig {
            name: "feed".into(),
            url: "http://host:9117".into(),
            api_key: "k".into(),
            enabled: true,
        };
        assert_eq!(
            torznab_endpoint(&jackett_renamed),
            "http://host:9117/api/v2.0/indexers/all/results/torznab/api"
        );
    }

    #[test]
    fn parses_prowlarr_json_results() {
        let body = r#"[{"title":"Example S01E01 1080p","magnetUrl":"magnet:?xt=urn:btih:0123456789012345678901234567890123456789","indexer":"Knaben"}]"#;
        let releases = parse_prowlarr_json(body, "prowlarr").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].source, "prowlarr:Knaben");
    }

    #[test]
    fn extracts_magnet_from_namespaced_rss_attributes_and_description() {
        let xml = r#"<rss><channel><item><title>Example.S01E01.1080p</title><description><![CDATA[<a href="magnet:?xt=urn:btih:0123456789012345678901234567890123456789">download</a>]]></description></item></channel></rss>"#;
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        assert!(xml.contains("magnet:?"));
        assert_eq!(
            magnet_in("prefix magnet:?xt=urn:btih:0123456789012345678901234567890123456789 suffix")
                .unwrap(),
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789"
        );
        let _ = reader.read_event().unwrap();
    }

    #[test]
    fn parses_cdata_titles_before_a_truncated_item() {
        let xml = r#"<rss><channel>
            <item><title><![CDATA[Example.S01E01.1080p.ITA]]></title><link><![CDATA[magnet:?xt=urn:btih:0123456789012345678901234567890123456789]]></link></item>
            <item><title><![CDATA[Truncated.S01E02]]></title><description><![CDATA[unfinished
        </channel></rss>"#;
        let releases = parse_feed_body(xml, "test").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].title, "Example.S01E01.1080p.ITA");
    }

    #[test]
    fn preserves_rss_publication_date_for_release_age_filters() {
        let xml = r#"<rss><channel><item><title>Example.S01E01.1080p.ITA</title><pubDate>Tue, 01 Sep 2026 12:00:00 +0000</pubDate><link>magnet:?xt=urn:btih:0123456789012345678901234567890123456789</link></item></channel></rss>"#;
        let releases = parse_feed_body(xml, "test").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(
            releases[0].discovered_at.to_rfc3339(),
            "2026-09-01T12:00:00+00:00"
        );
    }

    #[test]
    fn malformed_rss_is_reported_as_error() {
        // Malformed XML must surface an error (the caller then falls back to
        // the listing parser or skips the feed).
        let xml = "<rss><channel><item><title>Broken</title></link></head>";
        assert!(parse_feed_body(xml, "test").is_err());
    }

    #[test]
    fn feed_size_sanity_drops_tiny_items() {
        let xml = r#"<rss><channel>
          <item><title>Sample.S01E01.1080p</title><link>magnet:?xt=urn:btih:0123456789012345678901234567890123456789</link><enclosure length="1048576" type="application/x-bittorrent"/></item>
          <item><title>Real.S01E02.1080p.ITA</title><link>magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd</link><enclosure length="2147483648" type="application/x-bittorrent"/></item>
          <item><title>Tiny.From.Description.2024</title><link>magnet:?xt=urn:btih:1111111111111111111111111111111111111111</link><description>Size: 20 MB</description></item>
        </channel></rss>"#;
        let releases = parse_feed_body(xml, "test").unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].title, "Real.S01E02.1080p.ITA");
    }

    #[test]
    fn builds_per_uploader_source_labels_like_extto() {
        assert_eq!(
            source_label("ExtTo", "https://extto.org/browse/?filter=u=CorsaroNero"),
            "ExtTo - CorsaroNero"
        );
        assert_eq!(
            source_label(
                "TorrentGalaxy",
                "https://torrentgalaxy.one/get-posts/user:MIRCrewRS/"
            ),
            "TGx - MIRCrewRS"
        );
        assert_eq!(
            source_label(
                "ExtTo",
                "https://extto.org/browse/?filter=c=Movies&filter=tg=Italian"
            ),
            "ExtTo"
        );
    }

    #[test]
    fn recovers_title_from_magnet_display_name() {
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=The%20Black%20Hole%201979%20ITA&tr=udp%3A%2F%2Ft.example";
        assert_eq!(
            title_from_magnet(magnet).as_deref(),
            Some("The Black Hole 1979 ITA")
        );
        assert!(title_from_magnet("magnet:?xt=urn:btih:0123456789012345678901234567890123456789").is_none());
    }

    #[test]
    fn appends_source_tag_once() {
        assert_eq!(with_source_tag("Movie 2024", "ExtTo - X"), "Movie 2024 [ExtTo - X]");
        assert_eq!(
            with_source_tag("Movie 2024 [ExtTo - X]", "ExtTo - X"),
            "Movie 2024 [ExtTo - X]"
        );
        assert_eq!(with_source_tag("Movie 2024", ""), "Movie 2024");
    }

    #[test]
    fn parses_torrentgalaxy_rows_to_detail_links() {
        let body = r#"
            <div class="tgxtablerow txlight">
              <div class="tgxtablecell">
                <a class="txlight" title="Clash of the Thundermans (2026)" href="/post-detail/969210/clash/"><span src="torrent"><b>Clash of the Thundermans (2026) 2160p H265 iTA EnG MIRCrew</b></span></a>
              </div>
            </div>"#;
        let base = url::Url::parse("https://torrentgalaxy.one/get-posts/user:MIRCrewRS/").unwrap();
        let links = tgx_detail_links(body, &base);
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].0,
            "Clash of the Thundermans (2026) 2160p H265 iTA EnG MIRCrew"
        );
        assert_eq!(
            links[0].1.as_str(),
            "https://torrentgalaxy.one/post-detail/969210/clash/"
        );
    }

    #[test]
    fn only_cloudflare_statuses_trigger_flaresolverr() {
        for status in [403, 503, 520, 521, 524, 530] {
            assert!(cloudflare_blocked(status), "{status} should be CF-like");
        }
        for status in [200, 301, 404, 429, 500, 502] {
            assert!(!cloudflare_blocked(status), "{status} must not be CF-like");
        }
    }

    #[test]
    fn host_limiter_is_shared_per_domain() {
        let a = host_semaphore("https://torrentgalaxy.one/get-posts/user:X/").unwrap();
        let b = host_semaphore("https://torrentgalaxy.one/post-detail/1/").unwrap();
        let c = host_semaphore("https://other.example/x").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[test]
    fn remembers_cloudflare_session_from_flaresolverr_solution() {
        let cookies = serde_json::json!([
            {"name": "cf_clearance", "value": "abc", "domain": ".ext-to-test.example"},
            {"name": "PHPSESSID", "value": "xyz", "domain": ".ext-to-test.example"}
        ]);
        remember_session("https://ext-to-test.example/browse/", "BrowserUA/1.0", &cookies);
        let session = session_for("https://ext-to-test.example/post-detail/1/").expect("session");
        assert_eq!(session.user_agent, "BrowserUA/1.0");
        assert!(session.cookie.contains("cf_clearance=abc"));
        assert!(session.cookie.contains("PHPSESSID=xyz"));
        assert!(session_for("https://never-visited.example/").is_none());
    }

    #[tokio::test]
    async fn detail_cache_avoids_network_fetch() {
        crate::cache::set(
            "Cached Movie 2024".into(),
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
        );
        let pending = vec![(
            "Cached Movie 2024".to_string(),
            // Unreachable URL: a cache hit must never spawn a request.
            url::Url::parse("http://127.0.0.1:1/never").unwrap(),
        )];
        let client = Client::new();
        let mut output = Vec::new();
        let old = fetch_detail_magnets(&client, pending, "ExtTo - X", None, &mut output, None).await;
        assert_eq!(old, 0);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].title, "Cached Movie 2024 [ExtTo - X]");
        assert_eq!(output[0].source, "ExtTo - X");
    }

    #[tokio::test]
    async fn listing_uses_magnet_dn_when_anchor_has_no_text() {
        // ext.to serves `<a href="magnet:...&dn=...">` with no inner text.
        let body = r#"<html><body><a class="dwn-btn torrent-dwn" rel="nofollow" href="magnet:?xt=urn:btih:0123456789012345678901234567890123456789&amp;dn=The%20Black%20Hole%201979%20ITA%201080p&amp;tr=udp%3A%2F%2Ft.example"> </a></body></html>"#;
        let client = Client::new();
        let (releases, old, total) = fetch_traditional_listing(
            &client,
            "https://extto.org/browse/?filter=u=CorsaroNero",
            body,
            "ExtTo",
            "ExtTo - CorsaroNero",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(old, 0);
        assert_eq!(total, 1);
        assert_eq!(
            releases[0].title,
            "The Black Hole 1979 ITA 1080p [ExtTo - CorsaroNero]"
        );
        assert_eq!(releases[0].source, "ExtTo - CorsaroNero");
    }
}
