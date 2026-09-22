use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use regex::Regex;
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{LazyLock, RwLock},
};

/// Global cache of compiled regexes. The parsing/scoring hot paths used to call
/// `Regex::new` for every pattern on every release, which dominated CPU (the
/// same few dozen patterns recompiled millions of times per cycle). Compiled
/// regexes are cheap to clone (they share the compiled program).
/// `RwLock` because lookups vastly outnumber inserts and many scraper tasks
/// parse concurrently.
static REGEX_CACHE: LazyLock<RwLock<HashMap<String, Regex>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn cached_regex(pattern: &str) -> std::result::Result<Regex, regex::Error> {
    if let Some(compiled) = REGEX_CACHE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(pattern)
    {
        return Ok(compiled.clone());
    }
    let compiled = regex::Regex::new(pattern)?;
    let mut cache = REGEX_CACHE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Bound the cache: in practice only a few dozen distinct patterns exist.
    if cache.len() < 8192 {
        cache.insert(pattern.to_owned(), compiled.clone());
    }
    Ok(compiled)
}

/// Parses a date out of arbitrary listing text, like legacy's `parse_date_any`:
/// relative Italian expressions ("oggi", "ieri", "3 giorni fa") and the common
/// numeric formats. Used by the listing age filter.
pub fn parse_date_any(text: &str) -> Option<DateTime<Utc>> {
    if text.trim().is_empty() {
        return None;
    }
    let txt = text.to_lowercase();
    let now = Utc::now();
    if txt.contains("oggi") {
        return Some(now);
    }
    if txt.contains("ieri") {
        return Some(now - Duration::days(1));
    }
    if let Some(capture) = crate::utils::cached_regex(r"(\d+)\s*(minut[oi]|min|or[ae]|h|giorn[oi]|settiman[ae])\s*fa")
        .ok()?
        .captures(&txt)
    {
        let quantity: i64 = capture.get(1)?.as_str().parse().ok()?;
        let unit = capture.get(2)?.as_str();
        let delta = if unit.starts_with("min") {
            Duration::minutes(quantity)
        } else if unit.starts_with("or") || unit == "h" {
            Duration::hours(quantity)
        } else if unit.starts_with("settiman") {
            Duration::weeks(quantity)
        } else {
            Duration::days(quantity)
        };
        return Some(now - delta);
    }
    if let Some(capture) = crate::utils::cached_regex(r"(20\d{2})[-/](\d{1,2})[-/](\d{1,2})")
        .ok()?
        .captures(&txt)
    {
        let year: i32 = capture.get(1)?.as_str().parse().ok()?;
        let month: u32 = capture.get(2)?.as_str().parse().ok()?;
        let day: u32 = capture.get(3)?.as_str().parse().ok()?;
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
        }
    }
    if let Some(capture) = crate::utils::cached_regex(r"(\d{1,2})[-/](\d{1,2})[-/](20\d{2})")
        .ok()?
        .captures(&txt)
    {
        let day: u32 = capture.get(1)?.as_str().parse().ok()?;
        let month: u32 = capture.get(2)?.as_str().parse().ok()?;
        let year: i32 = capture.get(3)?.as_str().parse().ok()?;
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
        }
    }
    None
}

/// FlareSolverr is stateful and becomes unresponsive when many challenges are
/// submitted at once. Share a small queue between RSS and web-search callers.
pub static FLARESOLVERR_LIMITER: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(2));

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn magnet_hash(magnet: &str) -> Option<String> {
    let lower = magnet.to_ascii_lowercase();
    let marker = "urn:btih:";
    let start = lower.find(marker)? + marker.len();
    let value: String = lower[start..]
        .split('&')
        .next()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if value.len() == 40 || value.len() == 32 {
        Some(value)
    } else {
        None
    }
}

/// Parses the timestamp formats used in the databases: SQLite `datetime()`
/// (`YYYY-MM-DD HH:MM:SS`) and RFC3339 (with or without fraction/offset).
pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

/// Parses sizes like `4.5 GB` / `700 MiB` into megabytes (legacy's
/// `parse_size_mb`). Returns 0 when no size is found.
pub fn parse_size_mb(text: &str) -> f64 {
    if text.trim().is_empty() {
        return 0.0;
    }
    let Ok(regex) = crate::utils::cached_regex(r"(?i)([\d.]+)\s*([KMGT])i?B") else {
        return 0.0;
    };
    let Some(captures) = regex.captures(text) else {
        return 0.0;
    };
    let value: f64 = captures
        .get(1)
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(0.0);
    let unit = captures
        .get(2)
        .map(|value| value.as_str().to_ascii_uppercase())
        .unwrap_or_default();
    value
        * match unit.as_str() {
            "K" => 1.0 / 1024.0,
            "M" => 1.0,
            "G" => 1024.0,
            "T" => 1024.0 * 1024.0,
            _ => 1.0,
        }
}

/// Percent-encodes a tracker URL for the magnet query, keeping the separators
/// that are meaningful inside a tracker URL readable (legacy's `MAGNET_SAFE_DN`
/// equivalent). `&` and `=` are encoded so they cannot break the query string.
fn encode_tracker(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'?' | b'.' | b'_' | b'-' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn sanitize_magnet(input: &str, fallback_title: Option<&str>) -> Option<String> {
    let input = input.trim();
    if !input.starts_with("magnet:?") {
        return None;
    }
    let url = url::Url::parse(input).ok()?;
    let hash = magnet_hash(input)?;
    let mut out = format!("magnet:?xt=urn:btih:{hash}");
    if let Some(name) = url
        .query_pairs()
        .find(|(k, _)| k == "dn")
        .map(|(_, v)| v.to_string())
        .or_else(|| fallback_title.map(str::to_owned))
    {
        out.push_str("&dn=");
        out.push_str(&url::form_urlencoded::byte_serialize(name.as_bytes()).collect::<String>());
    }
    // legacy preserves the announce trackers (sorted and de-duplicated); dropping
    // them hurts peer discovery on public magnets.
    let mut trackers: Vec<String> = url
        .query_pairs()
        .filter(|(key, _)| key == "tr")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.trim().is_empty())
        .collect();
    trackers.sort();
    trackers.dedup();
    for tracker in trackers {
        out.push_str("&tr=");
        out.push_str(&encode_tracker(&tracker));
    }
    Some(out)
}

/// Removes API keys from URLs embedded in HTTP error messages before logging.
pub fn redact_url_secrets(input: &str) -> String {
    let mut output = input.to_string();
    let mut search_from = 0;
    loop {
        let lower = output[search_from..].to_ascii_lowercase();
        let Some(relative_start) = lower.find("apikey=").or_else(|| lower.find("api_key=")) else {
            break;
        };
        let key_start = search_from + relative_start;
        let value_start = output[key_start..]
            .find('=')
            .map(|offset| key_start + offset + 1)
            .unwrap_or(output.len());
        if value_start >= output.len() {
            break;
        }
        let value_end = output[value_start..]
            .find(|character: char| matches!(character, '&' | ' ' | ')' | ']' | '"'))
            .map(|offset| value_start + offset)
            .unwrap_or(output.len());
        if output[value_start..value_end].eq_ignore_ascii_case("[redacted]") {
            search_from = value_end;
            continue;
        }
        output.replace_range(value_start..value_end, "[redacted]");
        search_from = value_start + "[redacted]".len();
    }
    output
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

pub fn stable_id(value: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet_hash_accepts_hex40_and_base32_32_only() {
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:0123456789012345678901234567890123456789").as_deref(),
            Some("0123456789012345678901234567890123456789")
        );
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:ABCDEFGHIJKLMNOPQRSTUVWXYZ234567")
                .map(|hash| hash.len()),
            Some(32)
        );
        assert!(magnet_hash("magnet:?xt=urn:btih:short").is_none());
        assert!(magnet_hash("not a magnet").is_none());
    }

    #[test]
    fn sanitize_magnet_keeps_hash_display_name_and_trackers() {
        let out = sanitize_magnet(
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Example.Show&tr=udp://tracker.example:80&tr=udp://tracker.example:80&tr=https://other.example/announce",
            None,
        )
        .unwrap();
        assert!(out.starts_with("magnet:?xt=urn:btih:0123456789012345678901234567890123456789"));
        assert!(out.contains("dn=Example"));
        // Trackers preserved, sorted and de-duplicated.
        assert_eq!(out.matches("&tr=").count(), 2);
        assert!(out.contains("tr=https://other.example/announce"));
        assert!(out.contains("tr=udp://tracker.example:80"));
        assert!(sanitize_magnet("http://example.com/file.torrent", None).is_none());
    }

    #[test]
    fn sanitize_magnet_uses_fallback_title_when_missing() {
        let out = sanitize_magnet(
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            Some("Fallback Title"),
        )
        .unwrap();
        assert!(out.contains("dn="));
        assert!(out.contains("Fallback"));
    }

    #[test]
    fn parses_size_strings_into_megabytes() {
        assert_eq!(parse_size_mb("Size: 4.5 GB"), 4.5 * 1024.0);
        assert_eq!(parse_size_mb("700 MB"), 700.0);
        assert!((parse_size_mb("500 KiB") - 500.0 / 1024.0).abs() < 1e-9);
        assert_eq!(parse_size_mb("nessuna dimensione"), 0.0);
    }

    #[test]
    fn parses_relative_and_absolute_dates_from_text() {
        let now = Utc::now();
        let today = parse_date_any("pubblicato oggi").unwrap();
        assert!((today - now).num_seconds().abs() < 5);
        let yesterday = parse_date_any("ieri").unwrap();
        assert!((yesterday - (now - Duration::days(1))).num_seconds().abs() <= 5);
        let days = parse_date_any("3 giorni fa").unwrap();
        assert!((days - (now - Duration::days(3))).num_seconds().abs() <= 5);
        let ymd = parse_date_any("data: 2026-09-21").unwrap().to_rfc3339();
        assert_eq!(ymd, "2026-09-21T00:00:00+00:00");
        let dmy = parse_date_any("21/09/2026").unwrap().to_rfc3339();
        assert_eq!(dmy, "2026-09-21T00:00:00+00:00");
        assert!(parse_date_any("nessuna data qui").is_none());
    }

    #[test]
    fn redacts_api_keys_in_error_urls() {
        let value = redact_url_secrets(
            "request failed for http://localhost/api?query=Lanterns&apikey=secret123&type=search",
        );
        assert!(value.contains("apikey=[redacted]"));
        assert!(!value.contains("secret123"));
    }

    #[test]
    fn redacts_multiple_api_keys_without_looping() {
        let value = redact_url_secrets("apikey=one&api_key=two");
        assert_eq!(value, "apikey=[redacted]&api_key=[redacted]");
    }

    #[test]
    fn stable_id_is_deterministic_and_hex() {
        assert_eq!(stable_id("example"), stable_id("example"));
        assert_eq!(stable_id("example").len(), 40);
        assert!(stable_id("example").chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(stable_id("a"), stable_id("b"));
    }
}
