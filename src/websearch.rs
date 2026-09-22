use crate::utils::{magnet_hash, sanitize_magnet};
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

const TRACKERS: [&str; 3] = [
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.torrent.eu.org:451/announce",
];

/// Per-cycle counters of web-engine failures, drained by the engine to print a
/// single summary line instead of one warning per query.
static ENGINE_FAILURES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Drains the accumulated web-engine failures (`engine -> count`).
pub fn take_engine_failures() -> Vec<(String, usize)> {
    let mut failures = ENGINE_FAILURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut items = failures.drain().collect::<Vec<_>>();
    items.sort_by(|a, b| b.1.cmp(&a.1));
    items
}

pub async fn search(
    client: &Client,
    engines: &[String],
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let mut set = tokio::task::JoinSet::new();
    let limiter = Arc::new(Semaphore::new(4));
    for engine in engines.iter().map(|value| value.to_ascii_lowercase()) {
        let client = client.clone();
        let query = query.to_string();
        let flaresolverr = flaresolverr_url.map(str::to_string);
        let limiter = limiter.clone();
        if !matches!(
            engine.as_str(),
            "bitsearch"
                | "tpb"
                | "thepiratebay"
                | "knaben"
                | "nyaa"
                | "eztv"
                | "btdig"
                | "torrentscsv"
                | "limetorrents"
                | "torrentz2"
                | "bt4g"
                | "1337x"
                | "1337"
        ) {
            continue;
        }
        set.spawn(async move {
            let _permit = limiter.acquire_owned().await.ok();
            let flaresolverr = flaresolverr.as_deref();
            let found = match engine.as_str() {
                "bitsearch" => search_bitsearch(&client, &query).await,
                "tpb" | "thepiratebay" => search_tpb(&client, &query).await,
                "knaben" => search_knaben(&client, &query).await,
                "nyaa" => search_nyaa(&client, &query).await,
                "eztv" => search_eztv(&client, &query).await,
                "btdig" => search_btdig(&client, &query, flaresolverr).await,
                "torrentscsv" => search_torrentscsv(&client, &query).await,
                "limetorrents" => search_limetorrents(&client, &query, flaresolverr).await,
                "torrentz2" => search_torrentz2(&client, &query, flaresolverr).await,
                "bt4g" => search_bt4g(&client, &query, flaresolverr).await,
                "1337x" | "1337" => search_1337x(&client, &query, flaresolverr).await,
                _ => return (Vec::new(), None),
            };
            match found {
                Ok(found) => {
                    tracing::debug!(engine = %engine, query = %query, results = found.len(), "web engine search completed");
                    (found, None)
                }
                Err(error) => {
                    let error = crate::utils::redact_url_secrets(&error.to_string());
                    tracing::debug!(engine = %engine, query = %query, error = %error, "web engine search failed");
                    (Vec::new(), Some(engine))
                }
            }
        });
    }
    let mut results = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok((found, failure)) = joined {
            results.extend(found);
            if let Some(engine) = failure {
                // Aggregate per cycle instead of logging once per query.
                *ENGINE_FAILURES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .entry(engine)
                    .or_insert(0) += 1;
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    results.retain(|(_, magnet, _)| magnet_hash(magnet).is_some_and(|hash| seen.insert(hash)));
    Ok(results)
}

async fn search_bitsearch(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let response = client
        .get("https://bitsearch.to/api/v1/search")
        .query(&[("q", query), ("fuv", "yes"), ("limit", "20")])
        .send()
        .await?
        .error_for_status()?;
    let data: Value = response.json().await.context("decode BitSearch response")?;
    let mut results = Vec::new();
    for item in data
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let hash = item
            .get("infohash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if title.is_empty() || hash.len() != 40 {
            continue;
        }
        if let Some(magnet) = build_magnet(hash, title) {
            results.push((title.to_owned(), magnet, "BitSearch".into()));
        }
    }
    Ok(results)
}

async fn search_tpb(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let response = client
        .get("https://apibay.org/q.php")
        .query(&[("q", query), ("cat", "0")])
        .send()
        .await?
        .error_for_status()?;
    let data: Value = response
        .json()
        .await
        .context("decode Pirate Bay response")?;
    let mut results = Vec::new();
    for item in data.as_array().into_iter().flatten() {
        let title = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let hash = item
            .get("info_hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if title.is_empty() || hash.len() != 40 || hash.chars().all(|value| value == '0') {
            continue;
        }
        if let Some(magnet) = build_magnet(hash, title) {
            results.push((title.to_owned(), magnet, "ThePirateBay".into()));
        }
    }
    Ok(results)
}

async fn search_knaben(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let response = client.post("https://api.knaben.org/v1").json(&serde_json::json!({"query": query, "from": 0, "size": 20, "hideNsfw": true, "orderBy": "seeders", "orderDirection": "desc"})).send().await?.error_for_status()?;
    let data: Value = response.json().await.context("decode Knaben response")?;
    Ok(data
        .get("hits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let title = item.get("title").and_then(Value::as_str)?.trim();
            let magnet = item.get("magnetUrl").and_then(Value::as_str)?.trim();
            (!title.is_empty() && sanitize_magnet(magnet, Some(title)).is_some())
                .then(|| (title.to_owned(), magnet.to_owned(), "Knaben".into()))
        })
        .collect())
}

async fn search_nyaa(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let url = format!(
        "https://nyaa.si/?page=rss&q={}",
        url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
    );
    let releases = crate::rss::fetch_feed(client, &url, None, 3, 0, 0.8).await?;
    Ok(releases
        .into_iter()
        .map(|release| (release.title, release.magnet, "Nyaa".into()))
        .collect())
}

async fn search_eztv(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let expected = crate::utils::cached_regex(r"(?i)^(.+?)\s+s(\d{1,2})e(\d{1,3})\b")
        .unwrap()
        .captures(query)
        .and_then(|capture| {
            Some((
                capture.get(1)?.as_str().trim().to_owned(),
                capture.get(2)?.as_str().parse::<i64>().ok()?,
                capture.get(3)?.as_str().parse::<i64>().ok()?,
            ))
        });
    let Some((expected_series, expected_season, expected_episode)) = expected else {
        return Ok(Vec::new());
    };
    let response = client
        .get("https://eztvx.to/api/get-torrents")
        .query(&[("limit", "100"), ("page", "1"), ("keywords", query)])
        .send()
        .await?
        .error_for_status()?;
    let data: Value = response.json().await.context("decode EZTV response")?;
    let mut output = Vec::new();
    for item in data
        .get("torrents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| item.get("filename").and_then(Value::as_str))
            .unwrap_or_default()
            .trim();
        let hash = item
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let magnet = item
            .get("magnet_url")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                (hash.len() == 40)
                    .then(|| build_magnet(hash, title))
                    .flatten()
            });
        if !title.is_empty() && magnet.is_some() {
            let accepted =
                crate::parser::parse_release(title, magnet.as_deref().unwrap_or_default(), "EZTV")
                    .is_some_and(|release| {
                        release.kind == "series"
                            && release.season == Some(expected_season)
                            && release.episode_range.contains(&expected_episode)
                            && release.series.as_deref().is_some_and(|series| {
                                crate::parser::series_names_match(&expected_series, series)
                            })
                    });
            if accepted {
                output.push((title.to_owned(), magnet.unwrap(), "EZTV".into()));
            }
        }
        if output.len() >= 20 {
            break;
        }
    }
    Ok(output)
}

async fn search_btdig(
    client: &Client,
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let url = format!(
        "https://btdig.com/search?q={}&order=0&p=0",
        url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
    );
    let body = fetch_html(client, &url, flaresolverr_url).await?;
    let selector = scraper::Selector::parse("a[href^='magnet:']")
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let document = scraper::Html::parse_document(&body);
    let mut output = Vec::new();
    for link in document.select(&selector) {
        let magnet = link.value().attr("href").unwrap_or_default();
        let title = link.text().collect::<String>().trim().to_owned();
        // Alcuni link BTDigg hanno come testo un magnet troncato (`magnet:?xt=…`):
        // non è un titolo, salta la voce.
        if title.to_ascii_lowercase().starts_with("magnet:") {
            continue;
        }
        if !title.is_empty() && sanitize_magnet(magnet, Some(&title)).is_some() {
            output.push((title, magnet.to_owned(), "BTDigg".into()));
        }
        if output.len() >= 20 {
            break;
        }
    }
    Ok(output)
}

async fn fetch_html(client: &Client, url: &str, flaresolverr_url: Option<&str>) -> Result<String> {
    if let Ok(response) = client
        .get(url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
    {
        if response.status().is_success() {
            let body = response.text().await?;
            if body.len() > 1000 && !body.to_ascii_lowercase().contains("cloudflare") {
                return Ok(body);
            }
        }
    }
    let Some(endpoint) = flaresolverr_url.filter(|value| !value.trim().is_empty()) else {
        anyhow::bail!("HTML engine unavailable and FlareSolverr is not configured");
    };
    let _permit = crate::utils::FLARESOLVERR_LIMITER
        .acquire()
        .await
        .map_err(|_| anyhow::anyhow!("FlareSolverr request limiter closed"))?;
    let endpoint = format!("{}/v1", endpoint.trim_end_matches('/'));
    let response = client
        .post(&endpoint)
        .json(&serde_json::json!({"cmd":"request.get","url":url,"maxTimeout":20000}))
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await?
        .error_for_status()?;
    let data: Value = response
        .json()
        .await
        .context("decode FlareSolverr response")?;
    if data
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status != "ok")
    {
        anyhow::bail!("FlareSolverr returned status {:?}", data.get("status"));
    }
    data.get("solution")
        .and_then(|solution| solution.get("response"))
        .and_then(Value::as_str)
        .filter(|body| !body.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("FlareSolverr returned no HTML"))
}

async fn search_torrentscsv(client: &Client, query: &str) -> Result<Vec<(String, String, String)>> {
    let response = client
        .get("https://torrents-csv.ml/service/search")
        .query(&[("q", query), ("size", "20")])
        .send()
        .await?
        .error_for_status()?;
    let data: Value = response
        .json()
        .await
        .context("decode TorrentsCSV response")?;
    let mut output = Vec::new();
    for item in data
        .get("torrents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let hash = item
            .get("infohash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if title.is_empty() || hash.len() != 40 {
            continue;
        }
        if let Some(magnet) = build_magnet(hash, title) {
            output.push((title.to_owned(), magnet, "TorrentsCSV".into()));
        }
    }
    Ok(output)
}

async fn search_limetorrents(
    client: &Client,
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let slug = query.replace(' ', "-");
    let url = format!("https://limetorrent.net/search/all/{slug}/seeds/1/");
    let body = fetch_html(client, &url, flaresolverr_url).await?;
    let selector = scraper::Selector::parse("table.table2 a[href$='.html']")
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let details = {
        let document = scraper::Html::parse_document(&body);
        document
            .select(&selector)
            .take(8)
            .filter_map(|link| {
                let title = link.text().collect::<String>().trim().to_owned();
                let detail = url::Url::parse(&url)
                    .ok()?
                    .join(link.value().attr("href").unwrap_or_default())
                    .ok()?
                    .to_string();
                (!title.is_empty()).then_some((title, detail))
            })
            .collect::<Vec<_>>()
    };
    let mut output = Vec::new();
    for (title, detail) in details {
        let detail_body = fetch_html(client, &detail, flaresolverr_url).await?;
        let magnet = crate::utils::cached_regex(r#"magnet:\?xt=urn:btih:[0-9a-fA-F]{40,64}[^\s"'<>]*"#)
            .unwrap()
            .find(&detail_body)
            .map(|value| value.as_str().to_owned());
        if let Some(magnet) = magnet {
            output.push((title, magnet, "LimeTorrents".into()));
        }
    }
    Ok(output)
}

async fn search_torrentz2(
    client: &Client,
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let url = format!(
        "https://torrentz2.nz/search?q={}",
        url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
    );
    let body = fetch_html(client, &url, flaresolverr_url).await?;
    let selector = scraper::Selector::parse("div.results a[href^='/torrent/']")
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let details = {
        let document = scraper::Html::parse_document(&body);
        document
            .select(&selector)
            .take(8)
            .filter_map(|link| {
                let title = link.text().collect::<String>().trim().to_owned();
                let detail = url::Url::parse(&url)
                    .ok()?
                    .join(link.value().attr("href").unwrap_or_default())
                    .ok()?
                    .to_string();
                (!title.is_empty()).then_some((title, detail))
            })
            .collect::<Vec<_>>()
    };
    let mut output = Vec::new();
    for (title, detail) in details {
        let detail_body = fetch_html(client, &detail, flaresolverr_url).await?;
        let magnet = crate::utils::cached_regex(r#"magnet:\?xt=urn:btih:[0-9a-fA-F]{40,64}[^\s"'<>]*"#)
            .unwrap()
            .find(&detail_body)
            .map(|value| value.as_str().to_owned());
        if let Some(magnet) = magnet {
            output.push((title, magnet, "Torrentz2".into()));
        }
    }
    Ok(output)
}

async fn search_bt4g(
    client: &Client,
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let urls = [
        format!("https://bt4gprx.com/search?q={encoded}"),
        format!("https://bt4g.org/search?q={encoded}"),
    ];
    let mut selected_url = None;
    let mut body = None;
    for url in urls {
        if let Ok(value) = fetch_html(client, &url, flaresolverr_url).await {
            selected_url = url::Url::parse(&url).ok();
            body = Some(value);
            break;
        }
    }
    let base = selected_url.context("invalid BT4G base URL")?;
    let body = body.context("BT4G returned no HTML")?;
    let selector = scraper::Selector::parse("a[href*='/torrent/'], a[href*='/detail/']")
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let details = {
        let document = scraper::Html::parse_document(&body);
        document
            .select(&selector)
            .take(20)
            .filter_map(|link| {
                let title = link.text().collect::<String>().trim().to_owned();
                let href = link.value().attr("href").unwrap_or_default();
                (!title.is_empty() && !href.is_empty()).then_some((title, href.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    let mut output = Vec::new();
    for (title, href) in details {
        let detail = base.join(&href).context("invalid BT4G detail URL")?;
        let detail_body = fetch_html(client, detail.as_str(), flaresolverr_url).await?;
        if let Some(magnet) = first_magnet(&detail_body) {
            output.push((title, magnet, "BT4G".into()));
        }
        if output.len() >= 20 {
            break;
        }
    }
    Ok(output)
}

async fn search_1337x(
    client: &Client,
    query: &str,
    flaresolverr_url: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let slug = query.split_whitespace().collect::<Vec<_>>().join("-");
    let url = format!("https://1337x.to/search/{slug}/1/");
    let body = fetch_html(client, &url, flaresolverr_url).await?;
    let selector =
        scraper::Selector::parse("table.table-list a[href^='/torrent/'], a[href^='/torrent/']")
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let base = url::Url::parse("https://1337x.to")?;
    let details = {
        let document = scraper::Html::parse_document(&body);
        document
            .select(&selector)
            .take(20)
            .filter_map(|link| {
                let title = link.text().collect::<String>().trim().to_owned();
                let href = link.value().attr("href").unwrap_or_default();
                (!title.is_empty() && !href.is_empty()).then_some((title, href.to_owned()))
            })
            .collect::<Vec<_>>()
    };
    let mut output = Vec::new();
    for (title, href) in details {
        let detail = base.join(&href).context("invalid 1337x detail URL")?;
        let detail_body = fetch_html(client, detail.as_str(), flaresolverr_url).await?;
        if let Some(magnet) = first_magnet(&detail_body) {
            output.push((title, magnet, "1337x".into()));
        }
        if output.len() >= 20 {
            break;
        }
    }
    Ok(output)
}

fn first_magnet(body: &str) -> Option<String> {
    crate::utils::cached_regex(r#"magnet:\?xt=urn:btih:[0-9a-fA-F]{40,64}[^\s"'<>]*"#)
        .ok()?
        .find(body)
        .map(|value| value.as_str().to_owned())
        .and_then(|magnet| sanitize_magnet(&magnet, None))
}

fn build_magnet(hash: &str, title: &str) -> Option<String> {
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&dn={}",
        hash.to_ascii_lowercase(),
        url::form_urlencoded::byte_serialize(title.as_bytes()).collect::<String>()
    );
    let mut sanitized = sanitize_magnet(&magnet, Some(title))?;
    for tracker in TRACKERS {
        sanitized.push_str("&tr=");
        sanitized.push_str(
            &url::form_urlencoded::byte_serialize(tracker.as_bytes()).collect::<String>(),
        );
    }
    Some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_sanitized_magnet_with_trackers() {
        let magnet =
            build_magnet("0123456789012345678901234567890123456789", "Example Show").unwrap();
        assert!(magnet.contains("xt=urn:btih:0123456789012345678901234567890123456789"));
        assert!(magnet.contains("&dn=Example%20Show") || magnet.contains("&dn=Example+Show"));
        assert!(magnet.contains("&tr="));
    }

    #[test]
    fn extracts_and_sanitizes_html_magnet() {
        let html = r#"<a href="magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Example%20Show&x=bad space">download</a>"#;
        let magnet = first_magnet(html).unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:"));
        assert!(!magnet.contains("bad space"));
    }
}
