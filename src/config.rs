use crate::constants::*;
use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SeriesConfig {
    pub name: String,
    pub seasons: String,
    pub quality: String,
    pub language: String,
    pub archive_path: String,
    pub timeframe: i64,
    pub aliases: Vec<String>,
    pub tmdb_id: String,
    #[serde(default)]
    pub tvdb_id: String,
    pub subtitle: String,
    #[serde(default)]
    pub exclude: String,
    pub enabled: bool,
    pub ignored_seasons: Vec<i64>,
    #[serde(default)]
    pub season_subfolders: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MovieConfig {
    pub id: i64,
    pub name: String,
    pub year: String,
    /// Identificativi e dati editoriali: non intervengono nei filtri o nello
    /// stato del download e possono essere aggiornati dalla scelta TMDB/TVDB.
    #[serde(default)]
    pub tmdb_id: String,
    #[serde(default)]
    pub tvdb_id: String,
    #[serde(default)]
    pub original_title: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub poster_path: String,
    pub quality: String,
    pub language: String,
    pub enabled: bool,
    pub subtitle: String,
    pub exclude: String,
    #[serde(default)]
    pub language_requirements: String,
    #[serde(default)]
    pub subtitle_requirements: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexerConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// A per-source content filter: when a release's `source` contains `source`
/// (case-insensitive), any of `keywords` found in the title blocks the release.
/// Lets the user silence one feed/indexer/engine without disabling it entirely.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceFilter {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// Parses the `source_filters` setting (a JSON array); bad input yields none.
pub fn parse_source_filters(raw: &str) -> Vec<SourceFilter> {
    serde_json::from_str(raw).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibtorrentSettings {
    pub port_min: u16,
    pub port_max: u16,
    pub download_limit_kib: i64,
    pub upload_limit_kib: i64,
    pub seed_ratio: f64,
    pub seed_time_minutes: i64,
    pub seed_time_days: i64,
    pub stop_at_ratio: bool,
    pub active_downloads: i64,
    pub active_seeds: i64,
    pub active_limit: i64,
    pub dht: bool,
    pub pex: bool,
    pub lsd: bool,
    pub upnp: bool,
    pub natpmp: bool,
    pub dynamic_queue: bool,
    pub dynamic_queue_min: i64,
    pub dynamic_queue_max: i64,
    pub auto_remove_completed: bool,
    // Extended session settings.
    pub connections_limit: i64,
    pub upload_slots_limit: i64,
    pub half_open_limit: i64,
    pub alert_queue_size: i64,
    pub max_connections_per_torrent: i64,
    pub max_uploads_per_torrent: i64,
    pub aio_threads: i64,
    pub cache_size: i64,
    pub cache_expiry: i64,
    pub announce_interval: i64,
    pub torrent_connect_boost: i64,
    pub utp: bool,
    pub prefer_rc4: bool,
    pub announce_to_all_trackers: bool,
    pub announce_to_all_tiers: bool,
    pub allow_multiple_connections_per_ip: bool,
    pub apply_ip_filter: bool,
    pub encryption: i64,
    pub proxy_type: i64,
    pub proxy_host: String,
    pub proxy_port: i64,
    pub proxy_user: String,
    pub proxy_password: String,
    pub ip_filter_path: String,
    pub listen_interfaces: String,
    /// Interfaccia forzata per il traffico in uscita (killswitch VPN). Quando è
    /// impostata, libtorrent annuncia e si connette solo da questa scheda.
    pub outgoing_interface: String,
    pub dht_bootstrap_nodes: String,
}

impl Default for LibtorrentSettings {
    fn default() -> Self {
        Self {
            port_min: 6881,
            port_max: 6891,
            download_limit_kib: 0,
            upload_limit_kib: 0,
            seed_ratio: 0.0,
            seed_time_minutes: 0,
            seed_time_days: 0,
            stop_at_ratio: false,
            active_downloads: 3,
            active_seeds: 3,
            active_limit: 5,
            dht: true,
            pex: true,
            lsd: true,
            upnp: true,
            natpmp: true,
            dynamic_queue: false,
            dynamic_queue_min: 1,
            dynamic_queue_max: 10,
            auto_remove_completed: false,
            connections_limit: 200,
            upload_slots_limit: -1,
            half_open_limit: -1,
            alert_queue_size: 1000,
            max_connections_per_torrent: -1,
            max_uploads_per_torrent: -1,
            aio_threads: -1,
            cache_size: -1,
            cache_expiry: 300,
            announce_interval: 1800,
            torrent_connect_boost: 50,
            utp: true,
            prefer_rc4: false,
            announce_to_all_trackers: false,
            announce_to_all_tiers: false,
            allow_multiple_connections_per_ip: true,
            apply_ip_filter: true,
            encryption: 1,
            proxy_type: 0,
            proxy_host: String::new(),
            proxy_port: 0,
            proxy_user: String::new(),
            proxy_password: String::new(),
            ip_filter_path: String::new(),
            listen_interfaces: String::new(),
            outgoing_interface: String::new(),
            dht_bootstrap_nodes: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub data_dir: PathBuf,
    pub listen: String,
    pub engine_listen: String,
    pub import_source_dir: PathBuf,
    pub refresh_secs: u64,
    pub active: bool,
    pub dry_run: bool,
    pub feed_urls: Vec<String>,
    #[serde(default)]
    pub blacklist: Vec<String>,
    #[serde(default)]
    pub content_filters: Vec<String>,
    #[serde(default)]
    pub source_filters: Vec<SourceFilter>,
    #[serde(default)]
    pub max_release_age_days: i64,
    pub series: Vec<SeriesConfig>,
    pub movies: Vec<MovieConfig>,
    #[serde(default)]
    pub indexers: Vec<IndexerConfig>,
    #[serde(default)]
    pub websearch_engines: Vec<String>,
    #[serde(default)]
    pub flaresolverr_url: Option<String>,
    pub settings: BTreeMap<String, String>,
    pub tmdb_api_key: Option<String>,
    pub rename_episodes: bool,
    #[serde(default = "default_rename_format")]
    pub rename_format: String,
    #[serde(default = "default_rename_template")]
    pub rename_template: String,
    pub archive_root: Option<PathBuf>,
    pub trash_path: Option<PathBuf>,
    pub notify_telegram: bool,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    pub notify_webhook_url: Option<String>,
    pub notify_webhook_secret: Option<String>,
    pub notify_email: bool,
    pub email_smtp: String,
    pub email_from: Option<String>,
    pub email_to: Option<String>,
    pub email_password: Option<String>,
    pub cleanup_upgrades: bool,
    pub cleanup_min_score_diff: i64,
    pub upgrade_min_score_diff: i64,
    pub cleanup_action: String,
    pub libtorrent_enabled: bool,
    pub libtorrent: LibtorrentSettings,
    pub libtorrent_dir: PathBuf,
    pub libtorrent_temp_dir: Option<PathBuf>,
    pub state_dir: PathBuf,
    #[serde(default = "default_api_token")]
    pub api_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LegacyMigrationReport {
    pub files_found: Vec<String>,
    pub settings_imported: usize,
    pub series_imported: usize,
    pub movies_imported: usize,
    pub migrated: bool,
    pub renamed: Vec<String>,
}

fn default_rename_format() -> String {
    "base".into()
}
fn default_enabled() -> bool {
    true
}
fn default_rename_template() -> String {
    "{Serie} - {Stagione}{Episodio} - {Titolo} [{Risoluzione}][{Lingue}]".into()
}
fn default_blacklist() -> Vec<String> {
    [
        "cam",
        "camrip",
        "ts",
        "telesync",
        "telecine",
        "scr",
        "screener",
        "workprint",
        "sample",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn default_api_token() -> Option<String> {
    env::var("REXTTO_API_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
}
fn parse_list(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value).unwrap_or_else(|_| {
        value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn content_filter_ranges(filter: &str) -> Option<&'static [(u32, u32)]> {
    match filter {
        "[cjk]" => Some(&[
            (0x4E00, 0x9FFF),
            (0x3400, 0x4DBF),
            (0xF900, 0xFAFF),
            (0x3040, 0x30FF),
            (0xAC00, 0xD7AF),
            (0x3000, 0x303F),
        ]),
        "[cirillico]" => Some(&[(0x0400, 0x04FF), (0x0500, 0x052F)]),
        "[arabo]" => Some(&[
            (0x0600, 0x06FF),
            (0x0750, 0x077F),
            (0xFB50, 0xFDFF),
            (0xFE70, 0xFEFF),
        ]),
        "[ebraico]" => Some(&[(0x0590, 0x05FF), (0xFB00, 0xFB4F)]),
        "[thai]" => Some(&[(0x0E00, 0x0E7F)]),
        _ => None,
    }
}

/// legacy blacklist semantics: exclude when a pattern matches the title as a word.
fn title_is_blacklisted(title: &str, patterns: &[String]) -> bool {
    if title.is_empty() || patterns.is_empty() {
        return false;
    }
    let normalized = title.to_lowercase().replace(['.', '_', '-'], " ");
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim().to_lowercase();
        !pattern.is_empty()
            && crate::utils::cached_regex(&format!(r"\b{}\b", regex::escape(&pattern)))
                .map(|re| re.is_match(&normalized))
                .unwrap_or(false)
    })
}

/// legacy content-filter semantics: exclude when the title contains a filtered
/// token (word) or a filtered Unicode script (e.g. `[non-latino]`).
fn title_is_content_filtered(title: &str, filters: &[String]) -> bool {
    if title.is_empty() || filters.is_empty() {
        return false;
    }
    let normalized = title.to_lowercase().replace(['.', '_', '-'], " ");
    for filter in filters {
        let filter = filter.trim().to_lowercase();
        if filter.is_empty() {
            continue;
        }
        if filter == "[non-latino]" {
            if title
                .chars()
                .any(|character| character.is_alphabetic() && (character as u32) > 0x024F)
            {
                return true;
            }
            continue;
        }
        if filter == "[porno]" || filter == "[adulto]" {
            static ADULT_FILTER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let re = ADULT_FILTER.get_or_init(|| {
                const ADULT_KEYWORDS: &[&str] = &[
                    "porn",
                    "porno",
                    "xxx",
                    "milf",
                    "anal",
                    "creampie",
                    "hentai",
                    "onlyfans",
                    "brazzers",
                    "bangbros",
                    "tushy",
                    "vixen",
                    "legalporno",
                    "japorn",
                    "fakings",
                    "erotic",
                    "erotico",
                    "erotica",
                    "hardcore",
                    "blowjob",
                    "cumshot",
                    "deepthroat",
                    "squirt",
                    "stepmom",
                    "stepsister",
                    "sextape",
                    "nsfw",
                    "xhamster",
                    "xvideos",
                    "pornhub",
                    "redtube",
                    "youporn",
                    "chaturbate",
                    "camgirl",
                ];
                crate::utils::cached_regex(&format!(r"\b({})\b", ADULT_KEYWORDS.join("|")))
                    .expect("adult filter regex")
            });
            if re.is_match(&normalized) {
                return true;
            }
            continue;
        }
        if let Some(ranges) = content_filter_ranges(&filter) {
            if title.chars().any(|character| {
                let code = character as u32;
                ranges
                    .iter()
                    .any(|(low, high)| code >= *low && code <= *high)
            }) {
                return true;
            }
            continue;
        }
        if crate::utils::cached_regex(&format!(r"\b{}\b", regex::escape(&filter)))
            .map(|re| re.is_match(&normalized))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn normalize_language_code(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "it" | "ita" | "italian" => "ita".into(),
        "en" | "eng" | "english" => "eng".into(),
        value => value.into(),
    }
}

/// Parses a language requirement, supporting both the legacy legacy JSON form
/// (`[{"language": "ita", "required": true}]`) and a plain comma-separated list
/// (`ita,eng`). Only entries with `required != false` are returned.
fn parse_language_requirements(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            return items
                .iter()
                .filter(|item| {
                    item.get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true)
                })
                .filter_map(|item| item.get("language").and_then(serde_json::Value::as_str))
                .map(normalize_language_code)
                .filter(|value| !value.is_empty())
                .collect();
        }
    }
    trimmed
        .split([',', '+'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_language_code)
        .collect()
}

/// Cleans a stored `language_requirements`: drops empty/`-` entries and returns
/// the canonical JSON form (or an empty string).
pub fn sanitize_language_requirements(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return String::new();
    }
    let entries: Vec<(String, bool)> = if trimmed.starts_with('[') {
        match serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            Ok(items) => items
                .iter()
                .filter_map(|item| {
                    let language = item
                        .get("language")
                        .and_then(serde_json::Value::as_str)?
                        .trim();
                    if language.is_empty() || language == "-" {
                        return None;
                    }
                    let required = item
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    Some((normalize_language_code(language), required))
                })
                .filter(|(language, _)| !language.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        trimmed
            .split([',', '+'])
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "-")
            .map(|value| (normalize_language_code(value), true))
            .collect()
    };
    if entries.is_empty() {
        return String::new();
    }
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|(language, required)| {
            serde_json::json!({"language": language, "required": required})
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_default()
}

/// Cleans a stored `subtitle_requirements` into a plain comma list.
pub fn sanitize_subtitle_requirements(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return String::new();
    }
    if trimmed.starts_with('[') {
        let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) else {
            return String::new();
        };
        let languages: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.get("language")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| item.as_str())
            })
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "-")
            .map(normalize_language_code)
            .filter(|value| !value.is_empty())
            .collect();
        return languages.join(",");
    }
    trimmed.to_string()
}

/// Opens the shared `rextto_config.db` (used by the daemon, i18n and libtorrent)
/// with the pragmas required for concurrent access: WAL journal mode and a busy
/// timeout, so a concurrent writer waits instead of failing with `SQLITE_BUSY`
/// and silently losing the update.
pub fn open_config_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    // WAL is persisted in the file header, but re-applying it is harmless.
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

/// One-time cleanup of legacy/imported dirty movie requirement rows.
pub fn cleanup_movie_requirements(data_dir: &Path) -> Result<()> {
    let path = data_dir.join("rextto_config.db");
    if !path.is_file() {
        return Ok(());
    }
    let conn = open_config_db(&path)?;
    let rows: Vec<(i64, String, String)> = conn
        .prepare("SELECT id,COALESCE(language_requirements,''),COALESCE(subtitle_requirements,'') FROM movies_config")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .filter_map(Result::ok)
        .collect();
    for (id, language, subtitle) in rows {
        let clean_language = sanitize_language_requirements(&language);
        let clean_subtitle = sanitize_subtitle_requirements(&subtitle);
        if clean_language != language || clean_subtitle != subtitle {
            conn.execute(
                "UPDATE movies_config SET language_requirements=?1, subtitle_requirements=?2 WHERE id=?3",
                rusqlite::params![clean_language, clean_subtitle, id],
            )?;
        }
    }
    Ok(())
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = PathBuf::from(env::var("REXTTO_DATA_DIR").unwrap_or_else(|_| "data".into()));
        Self {
            state_dir: data_dir.join(DEFAULT_STATE_DIR),
            libtorrent_dir: data_dir.join("downloads"),
            libtorrent_temp_dir: Some(data_dir.join("incomplete")),
            archive_root: None,
            trash_path: Some(data_dir.join("trash")),
            notify_telegram: false,
            telegram_bot_token: None,
            telegram_chat_id: None,
            notify_webhook_url: None,
            notify_webhook_secret: None,
            notify_email: false,
            email_smtp: "smtp.gmail.com:587".into(),
            email_from: None,
            email_to: None,
            email_password: None,
            cleanup_upgrades: false,
            cleanup_min_score_diff: 0,
            upgrade_min_score_diff: 200,
            cleanup_action: "move".into(),
            listen: env::var("REXTTO_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.into()),
            engine_listen: env::var("REXTTO_ENGINE_LISTEN")
                .unwrap_or_else(|_| DEFAULT_ENGINE_LISTEN.into()),
            import_source_dir: env::var("REXTTO_IMPORT_SOURCE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| data_dir.clone()),
            active: false,
            dry_run: true,
            data_dir,
            refresh_secs: DEFAULT_REFRESH_SECS,
            feed_urls: Vec::new(),
            blacklist: default_blacklist(),
            content_filters: Vec::new(),
            source_filters: Vec::new(),
            max_release_age_days: 0,
            series: Vec::new(),
            movies: Vec::new(),
            settings: BTreeMap::new(),
            indexers: Vec::new(),
            websearch_engines: Vec::new(),
            flaresolverr_url: None,
            tmdb_api_key: None,
            rename_episodes: false,
            libtorrent_enabled: false,
            libtorrent: LibtorrentSettings::default(),
            rename_format: "base".into(),
            rename_template: "{Serie} - {Stagione}{Episodio} - {Titolo} [{Risoluzione}][{Lingue}]".into(),
            api_token: default_api_token(),
        }
    }
}

impl Config {
    /// BCP-47 language used for TMDB API calls (e.g. `it-IT`).
    pub fn tmdb_language(&self) -> String {
        self.settings
            .get("tmdb_language")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("it-IT")
            .to_string()
    }

    /// Default language for acquisitions and rename fallback (e.g. `ita`).
    pub fn default_language(&self) -> String {
        self.settings
            .get("default_language")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("ita")
            .to_string()
    }

    /// Number of listing pages to fetch per feed (legacy `MAX_PAGES`, default 3).
    /// This used to be read from `stop_on_old_page_threshold`, which in legacy is
    /// a 0..1 ratio — a value like `0.8` could not parse as a page count and
    /// silently fell back to 3.
    pub fn feed_max_pages(&self) -> usize {
        Self::number_setting(&self.settings, "feed_max_pages", 3usize).clamp(1, 10)
    }

    /// legacy `stop_on_old_page_threshold`: ratio (0..=1) of old releases on a
    /// page above which the listing walk stops early. Only meaningful together
    /// with `max_release_age_days`.
    pub fn stop_on_old_page_ratio(&self) -> f64 {
        self.settings
            .get("stop_on_old_page_threshold")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| (0.0..=1.0).contains(value))
            .unwrap_or(0.8)
    }

    /// When true, log debug-level diagnostics (legacy `debug_*`).
    pub fn debug_enabled(&self) -> bool {
        Self::bool_setting(self.settings.get("debug_enabled"))
            || Self::bool_setting(self.settings.get("debug_mode"))
    }

    /// TheTVDB API key, when configured.
    pub fn tvdb_api_key(&self) -> Option<String> {
        self.settings
            .get("tvdb_api_key")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// Preferred TVDB language (falls back to the default acquisition language).
    pub fn tvdb_language(&self) -> String {
        self.settings
            .get("tvdb_language")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.default_language())
    }

    /// Configured RAM disk directory, when non-empty.
    pub fn ramdisk_dir(&self) -> Option<PathBuf> {
        Self::path_setting(self.settings.get("libtorrent_ramdisk_dir"))
    }

    /// True when the RAM disk download tier is active. A configured directory
    /// enables it unless `libtorrent_ramdisk_enabled` is explicitly falsy.
    pub fn ramdisk_enabled(&self) -> bool {
        let explicitly_off = self
            .settings
            .get("libtorrent_ramdisk_enabled")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "" | "no" | "false" | "0"
                )
            })
            .unwrap_or(false);
        !explicitly_off && self.ramdisk_dir().is_some()
    }

    /// Maximum size (bytes) for a single torrent admitted to the RAM disk.
    pub fn ramdisk_threshold_bytes(&self) -> u64 {
        Self::gib_setting(&self.settings, "libtorrent_ramdisk_threshold_gb", 3.5)
    }

    /// Free space (bytes) to keep on the RAM disk after a download finishes.
    pub fn ramdisk_margin_bytes(&self) -> u64 {
        Self::gib_setting(&self.settings, "libtorrent_ramdisk_margin_gb", 0.5)
    }

    /// Explicit free-space floor in bytes; `0` derives it from the margin.
    pub fn ramdisk_min_free_bytes(&self) -> u64 {
        self.settings
            .get("libtorrent_ramdisk_min_free_bytes")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0)
    }

    fn bool_setting(value: Option<&String>) -> bool {
        matches!(
            value.map(String::as_str),
            Some("yes" | "true" | "1" | "True" | "TRUE")
        )
    }

    fn bool_setting_or(settings: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
        settings
            .get(key)
            .map(|value| Self::bool_setting(Some(value)))
            .unwrap_or(default)
    }

    fn path_setting(value: Option<&String>) -> Option<PathBuf> {
        value
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
    }

    fn number_setting<T: std::str::FromStr>(
        settings: &BTreeMap<String, String>,
        key: &str,
        default: T,
    ) -> T {
        settings
            .get(key)
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    /// Reads a GiB-sized setting (e.g. `3.5`) and returns it as bytes.
    fn gib_setting(settings: &BTreeMap<String, String>, key: &str, default: f64) -> u64 {
        let gib = settings
            .get(key)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or(default);
        if gib <= 0.0 {
            0
        } else {
            (gib * 1024.0 * 1024.0 * 1024.0) as u64
        }
    }

    pub fn find_series_match(&self, name: &str, season: Option<i64>) -> Option<&SeriesConfig> {
        let normalized = crate::parser::normalize_series_name(name);
        self.series.iter().find(|series| {
            if !series.enabled
                || season.is_some_and(|value| {
                    series.ignored_seasons.contains(&value)
                        || !Self::season_allowed(&series.seasons, value)
                })
            {
                return false;
            }
            crate::parser::series_names_match(&series.name, &normalized)
                || series
                    .aliases
                    .iter()
                    .any(|alias| crate::parser::series_names_match(alias, &normalized))
        })
    }

    /// Finds a configured series by name/alias ignoring the monitored season and
    /// enabled filters. Used to rename/operate on already-archived files, which
    /// must not be skipped just because their season is not monitored.
    pub fn find_series_by_name(&self, name: &str) -> Option<&SeriesConfig> {
        let normalized = crate::parser::normalize_series_name(name);
        self.series.iter().find(|series| {
            crate::parser::series_names_match(&series.name, &normalized)
                || series
                    .aliases
                    .iter()
                    .any(|alias| crate::parser::series_names_match(alias, &normalized))
        })
    }

    /// Cartella archivio effettiva di una serie: `archive_path` se configurato,
    /// altrimenti una sottocartella di `archive_root` il cui nome matcha la serie
    /// (o un alias). Porting dell'auto-detect del legacy: serve sia alla
    /// destinazione dei download sia alla scansione dei file su disco.
    pub fn resolve_archive_path(&self, series: &SeriesConfig) -> Option<std::path::PathBuf> {
        let configured = series.archive_path.trim();
        if !configured.is_empty() {
            return Some(std::path::PathBuf::from(configured));
        }
        let root = self.archive_root.as_deref()?;
        let entries = std::fs::read_dir(root).ok()?;
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let matches = crate::parser::series_names_match(&series.name, &name)
                || series
                    .aliases
                    .iter()
                    .any(|alias| crate::parser::series_names_match(alias, &name));
            if matches {
                candidates.push(path);
            }
        }
        candidates.sort();
        candidates.into_iter().next()
    }

    fn season_allowed(specification: &str, season: i64) -> bool {
        let specification = specification.trim();
        if specification.is_empty() || specification == "*" {
            return true;
        }
        specification
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .any(|part| {
                if let Some(minimum) = part
                    .strip_suffix('+')
                    .and_then(|value| value.trim().parse::<i64>().ok())
                {
                    return season >= minimum;
                }
                if let Some((start, end)) = part.split_once('-').and_then(|(start, end)| {
                    Some((
                        start.trim().parse::<i64>().ok()?,
                        end.trim().parse::<i64>().ok()?,
                    ))
                }) {
                    return (start..=end).contains(&season);
                }
                part.parse::<i64>().ok() == Some(season)
            })
    }

    pub fn season_allowed_for_scan(specification: &str, season: i64) -> bool {
        Self::season_allowed(specification, season)
    }

    pub fn find_movie_match(&self, title: &str, year: Option<i64>) -> Option<&MovieConfig> {
        // legacy rejects obvious non-movies before matching (episodes, sport,
        // wrestling, magazines, videogames, console ROMs, some music).
        if !crate::parser::passes_movie_filter(title) {
            return None;
        }
        let title = title.to_ascii_lowercase();
        self.movies.iter().find(|movie| {
            if !movie.enabled {
                return false;
            }
            let name_words = movie
                .name
                .to_ascii_lowercase()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if name_words.len() < 2
                || !name_words.iter().all(|word| {
                    title
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|token| token == word)
                })
            {
                return false;
            }
            if !movie.exclude.is_empty()
                && movie
                    .exclude
                    .split(',')
                    .map(str::trim)
                    .any(|word| !word.is_empty() && title.contains(&word.to_ascii_lowercase()))
            {
                return false;
            }
            let Some(configured_year) = movie.year.parse::<i64>().ok() else {
                return false;
            };
            year.is_some_and(|found| (found - configured_year).abs() <= 1)
        })
    }

    pub fn quality_allowed(
        quality: &crate::models::Quality,
        requirement: &str,
        language: &str,
        subtitle: &str,
    ) -> bool {
        let requirement = requirement.to_ascii_lowercase();
        let resolution = |value: &str| {
            ["2160p", "1080p", "720p", "576p", "480p"]
                .iter()
                .find(|candidate| value.contains(**candidate))
                .and_then(|candidate| candidate.trim_end_matches('p').parse::<i64>().ok())
        };
        let actual_resolution = quality.resolution.trim_end_matches('p').parse::<i64>().ok();
        let (minimum, maximum) = if requirement.trim_start().starts_with('<') {
            (None, resolution(&requirement))
        } else if let Some((minimum, maximum)) = requirement.split_once('-') {
            (resolution(minimum), resolution(maximum))
        } else {
            (resolution(&requirement), None)
        };
        if minimum.is_some_and(|minimum| actual_resolution.is_none_or(|actual| actual < minimum))
            || maximum
                .is_some_and(|maximum| actual_resolution.is_none_or(|actual| actual > maximum))
        {
            return false;
        }
        let requested = language
            .split(',')
            .map(normalize_language_code)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if !requested.is_empty() {
            let detected = if quality.languages.is_empty() {
                vec![normalize_language_code(&quality.language)]
            } else {
                quality
                    .languages
                    .iter()
                    .map(|value| normalize_language_code(value))
                    .collect()
            };
            if !detected.iter().any(|detected| requested.contains(detected)) {
                return false;
            }
        }
        if matches!(subtitle.to_ascii_lowercase().as_str(), "yes" | "true" | "1")
            && !quality.has_subtitle
        {
            return false;
        }
        true
    }

    pub fn movie_release_allowed(movie: &MovieConfig, quality: &crate::models::Quality) -> bool {
        let required_languages = parse_language_requirements(&movie.language_requirements);
        let language = if required_languages.is_empty() {
            parse_language_requirements(&movie.language)
        } else {
            required_languages
        }
        .join(",");
        if !Self::quality_allowed(quality, &movie.quality, &language, "") {
            return false;
        }
        let subtitle_raw = if movie.subtitle_requirements.trim().is_empty() {
            movie.subtitle.trim()
        } else {
            movie.subtitle_requirements.trim()
        };
        let subtitle_flag = matches!(
            subtitle_raw.to_ascii_lowercase().as_str(),
            "yes" | "true" | "1" | "sub" | "subs"
        );
        let required = parse_language_requirements(subtitle_raw);
        if required.is_empty() {
            return !subtitle_flag || quality.has_subtitle;
        }
        required.iter().any(|value| {
            (matches!(value.as_str(), "yes" | "true" | "1" | "sub" | "subs")
                && quality.has_subtitle)
                || quality
                    .subtitle_languages
                    .iter()
                    .map(|item| normalize_language_code(item))
                    .any(|item| &item == value)
        })
    }

    pub fn series_release_allowed(
        series: &SeriesConfig,
        quality: &crate::models::Quality,
        title: &str,
    ) -> bool {
        if !Self::quality_allowed(quality, &series.quality, &series.language, &series.subtitle) {
            return false;
        }
        series
            .exclude
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .all(|word| {
                !title
                    .to_ascii_lowercase()
                    .contains(&word.to_ascii_lowercase())
            })
    }

    pub fn release_allowed(&self, release: &crate::models::Release) -> bool {
        self.release_denied_reason(release).is_none()
            && self.source_filter_denied_reason(release).is_none()
    }

    /// Reason a release is refused by a per-source filter, if any. Unlike the
    /// global filters this needs the dynamic source/keyword names, so it returns
    /// an owned string.
    pub fn source_filter_denied_reason(
        &self,
        release: &crate::models::Release,
    ) -> Option<String> {
        if self.source_filters.is_empty() {
            return None;
        }
        let source = release.source.to_ascii_lowercase();
        let title = release.title.to_ascii_lowercase();
        for filter in &self.source_filters {
            if !filter.enabled {
                continue;
            }
            let needle = filter.source.trim().to_ascii_lowercase();
            if needle.is_empty() || !source.contains(&needle) {
                continue;
            }
            for keyword in &filter.keywords {
                let keyword = keyword.trim().to_ascii_lowercase();
                if !keyword.is_empty() && title.contains(&keyword) {
                    return Some(format!(
                        "blocked by source filter '{}' (keyword '{keyword}')",
                        filter.source.trim()
                    ));
                }
            }
        }
        None
    }

    /// Human-readable reason a release is refused by the global filters, or
    /// `None` when it is allowed. Used to make every filter decision visible in
    /// the log instead of silently dropping releases.
    pub fn release_denied_reason(&self, release: &crate::models::Release) -> Option<&'static str> {
        if title_is_blacklisted(&release.title, &self.blacklist) {
            return Some("title matches the blacklist");
        }
        if title_is_content_filtered(&release.title, &self.content_filters) {
            return Some("title matches a content filter");
        }
        if self.max_release_age_days > 0
            && Utc::now()
                .signed_duration_since(release.discovered_at)
                .num_days()
                > self.max_release_age_days
        {
            return Some("older than max_release_age_days");
        }
        None
    }

    fn load_config_db(&mut self) -> Result<()> {
        let path = self.data_dir.join("rextto_config.db");
        if !path.is_file() {
            return Ok(());
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // Wait for concurrent writers instead of failing with SQLITE_BUSY.
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        let Ok(mut stmt) = conn.prepare("SELECT key,value FROM settings") else {
            return Ok(());
        };
        for row in stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (key, value) = row?;
            self.settings.insert(key, value);
        }
        if let Some(value) = self.settings.get("url") {
            self.feed_urls = serde_json::from_str(value).unwrap_or_else(|_| vec![value.clone()]);
        }
        self.blacklist = self
            .settings
            .get("blacklist")
            .map(|value| parse_list(value))
            .unwrap_or_else(default_blacklist);
        self.content_filters = self
            .settings
            .get("content_filters")
            .or_else(|| self.settings.get("content_filter"))
            .map(|value| parse_list(value))
            .unwrap_or_default();
        self.source_filters = self
            .settings
            .get("source_filters")
            .map(|value| parse_source_filters(value))
            .unwrap_or_default();
        self.max_release_age_days = self
            .settings
            .get("max_release_age_days")
            .or_else(|| self.settings.get("max_age_days"))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        if let Some(value) = self.settings.get("_migrated_series") {
            self.series = serde_json::from_str(value).unwrap_or_default();
        }
        if self.series.is_empty() {
            let series_path = self.data_dir.join("rextto_series.db");
            if series_path.is_file() {
                let series_conn = Connection::open_with_flags(
                    series_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                if let Ok(mut series_stmt) = series_conn.prepare("SELECT name,seasons,quality,language,archive_path,COALESCE(timeframe,0),aliases,tmdb_id,COALESCE(subtitle,''),enabled,COALESCE(ignored_seasons,'[]'),COALESCE(exclude,'') FROM series") {
                    self.series = series_stmt.query_map([], |row| Ok(SeriesConfig {
                        name: row.get(0)?, seasons: row.get(1)?, quality: row.get(2)?, language: row.get(3)?, archive_path: row.get(4)?, timeframe: row.get(5)?,
                        aliases: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_else(|_| row.get::<_, String>(6).unwrap_or_default().split(',').map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned).collect()),
                        tmdb_id: row.get(7)?, tvdb_id: String::new(), subtitle: row.get(8)?, enabled: row.get::<_, i64>(9)? != 0,
                        ignored_seasons: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(), exclude: row.get(11)?,
                        season_subfolders: false,
                    })).ok().into_iter().flatten().filter_map(Result::ok).collect();
                };
            }
        }
        let movie_query = if conn
            .prepare("SELECT tmdb_id,tvdb_id,original_title,overview,poster_path FROM movies_config")
            .is_ok()
        {
            "SELECT id,name,year,quality,language,enabled,subtitle,exclude,language_requirements,subtitle_requirements,tmdb_id,tvdb_id,original_title,overview,poster_path FROM movies_config"
        } else if conn
            .prepare("SELECT language_requirements,subtitle_requirements FROM movies_config")
            .is_ok()
        {
            "SELECT id,name,year,quality,language,enabled,subtitle,exclude,language_requirements,subtitle_requirements,'','','','','' FROM movies_config"
        } else if conn.prepare("SELECT exclude FROM movies_config").is_ok() {
            "SELECT id,name,year,quality,language,enabled,subtitle,exclude,'','','','','','','' FROM movies_config"
        } else {
            "SELECT id,name,year,quality,language,enabled,subtitle,'','','','','','','','' FROM movies_config"
        };
        if let Ok(mut stmt) = conn.prepare(movie_query) {
            self.movies = stmt
                .query_map([], |row| {
                    Ok(MovieConfig {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        year: row.get(2)?,
                        quality: row.get(3)?,
                        language: row.get(4)?,
                        enabled: row.get::<_, i64>(5)? != 0,
                        subtitle: row.get(6)?,
                        exclude: row.get(7)?,
                        language_requirements: sanitize_language_requirements(
                            &row.get::<_, String>(8).unwrap_or_default(),
                        ),
                        subtitle_requirements: sanitize_subtitle_requirements(
                            &row.get::<_, String>(9).unwrap_or_default(),
                        ),
                        tmdb_id: row.get(10).unwrap_or_default(),
                        tvdb_id: row.get(11).unwrap_or_default(),
                        original_title: row.get(12).unwrap_or_default(),
                        overview: row.get(13).unwrap_or_default(),
                        poster_path: row.get(14).unwrap_or_default(),
                    })
                })?
                .filter_map(Result::ok)
                .collect();
        }
        if let Some(value) = self.settings.get("indexers") {
            self.indexers = serde_json::from_str(value).unwrap_or_default();
        }
        if let Some(value) = self.settings.get("websearch_engines") {
            self.websearch_engines = serde_json::from_str(value).unwrap_or_else(|_| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect()
            });
        }
        self.flaresolverr_url = self
            .settings
            .get("flaresolverr_url")
            .filter(|value| !value.trim().is_empty())
            .cloned();
        let legacy_indexers = [
            ("jackett", "jackett_url", "jackett_api"),
            ("prowlarr", "prowlarr_url", "prowlarr_api"),
        ];
        for (name, url_key, key_key) in legacy_indexers {
            if self
                .indexers
                .iter()
                .any(|indexer| indexer.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            if let Some(url) = self
                .settings
                .get(url_key)
                .filter(|url| !url.trim().is_empty())
            {
                self.indexers.push(IndexerConfig {
                    name: name.into(),
                    url: url.clone(),
                    api_key: self.settings.get(key_key).cloned().unwrap_or_default(),
                    enabled: true,
                });
            }
        }
        self.refresh_secs = self
            .settings
            .get("refresh_interval")
            .and_then(|value| value.parse().ok())
            .unwrap_or(self.refresh_secs);
        if self.settings.contains_key("active") {
            self.active = Self::bool_setting(self.settings.get("active"));
        }
        self.tmdb_api_key = self
            .settings
            .get("tmdb_api_key")
            .filter(|value| !value.is_empty())
            .cloned();
        self.rename_episodes = Self::bool_setting(self.settings.get("rename_episodes"));
        self.rename_format = self
            .settings
            .get("rename_format")
            .filter(|value| {
                matches!(
                    value.as_str(),
                    "base" | "standard" | "full" | "completo" | "custom"
                )
            })
            .cloned()
            .unwrap_or_else(|| "base".into());
        self.rename_template = self
            .settings
            .get("rename_template")
            .cloned()
            .unwrap_or_else(|| self.rename_template.clone());
        self.api_token = self
            .settings
            .get("api_token")
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .or_else(|| self.api_token.clone());
        self.archive_root = Self::path_setting(self.settings.get("archive_root"));
        self.trash_path =
            Self::path_setting(self.settings.get("trash_path")).or_else(|| self.trash_path.clone());
        self.notify_telegram = Self::bool_setting(self.settings.get("notify_telegram"));
        self.telegram_bot_token = self
            .settings
            .get("telegram_bot_token")
            .filter(|value| !value.is_empty())
            .cloned();
        self.telegram_chat_id = self
            .settings
            .get("telegram_chat_id")
            .filter(|value| !value.is_empty())
            .cloned();
        self.notify_webhook_url = self
            .settings
            .get("notify_webhook_url")
            .filter(|value| !value.is_empty())
            .cloned();
        self.notify_webhook_secret = self
            .settings
            .get("notify_webhook_secret")
            .filter(|value| !value.is_empty())
            .cloned();
        self.notify_email = Self::bool_setting(self.settings.get("notify_email"));
        self.email_smtp = self
            .settings
            .get("email_smtp")
            .cloned()
            .unwrap_or_else(|| "smtp.gmail.com:587".into());
        self.email_from = self
            .settings
            .get("email_from")
            .filter(|value| !value.is_empty())
            .cloned();
        self.email_to = self
            .settings
            .get("email_to")
            .filter(|value| !value.is_empty())
            .cloned();
        self.email_password = self
            .settings
            .get("email_password")
            .filter(|value| !value.is_empty())
            .cloned();
        self.cleanup_upgrades = Self::bool_setting(self.settings.get("cleanup_upgrades"));
        self.cleanup_min_score_diff =
            Self::number_setting(&self.settings, "cleanup_min_score_diff", 0);
        self.upgrade_min_score_diff =
            Self::number_setting(&self.settings, "upgrade_min_score_diff", 200).max(0);
        self.cleanup_action = self
            .settings
            .get("cleanup_action")
            .filter(|value| matches!(value.as_str(), "move" | "delete"))
            .cloned()
            .unwrap_or_else(|| "move".into());
        self.libtorrent_enabled = Self::bool_setting(self.settings.get("libtorrent_enabled"));
        self.libtorrent = LibtorrentSettings {
            port_min: Self::number_setting(&self.settings, "libtorrent_port_min", 6881),
            port_max: Self::number_setting(&self.settings, "libtorrent_port_max", 6891),
            download_limit_kib: Self::number_setting(&self.settings, "libtorrent_dl_limit", 0),
            upload_limit_kib: Self::number_setting(&self.settings, "libtorrent_ul_limit", 0),
            seed_ratio: Self::number_setting(&self.settings, "libtorrent_seed_ratio", 0.0),
            seed_time_minutes: Self::number_setting(&self.settings, "libtorrent_seed_time", 0),
            seed_time_days: Self::number_setting(&self.settings, "libtorrent_seed_time_days", 0),
            stop_at_ratio: Self::bool_setting(self.settings.get("libtorrent_stop_at_ratio")),
            active_downloads: Self::number_setting(
                &self.settings,
                "libtorrent_active_downloads",
                3,
            ),
            active_seeds: Self::number_setting(&self.settings, "libtorrent_active_seeds", 3),
            active_limit: Self::number_setting(&self.settings, "libtorrent_active_limit", 5),
            dht: Self::bool_setting_or(&self.settings, "libtorrent_dht", true),
            pex: Self::bool_setting_or(&self.settings, "libtorrent_pex", true),
            lsd: Self::bool_setting_or(&self.settings, "libtorrent_lsd", true),
            upnp: Self::bool_setting_or(&self.settings, "libtorrent_upnp", true),
            natpmp: Self::bool_setting_or(&self.settings, "libtorrent_natpmp", true),
            dynamic_queue: Self::bool_setting_or(&self.settings, "libtorrent_dynamic_queue", false),
            dynamic_queue_min: Self::number_setting(
                &self.settings,
                "libtorrent_dynamic_queue_min",
                1,
            ),
            dynamic_queue_max: Self::number_setting(
                &self.settings,
                "libtorrent_dynamic_queue_max",
                10,
            ),
            auto_remove_completed: Self::bool_setting(self.settings.get("auto_remove_completed")),
            connections_limit: Self::number_setting(
                &self.settings,
                "libtorrent_connections_limit",
                200,
            ),
            upload_slots_limit: Self::number_setting(
                &self.settings,
                "libtorrent_upload_slots_limit",
                -1,
            ),
            half_open_limit: Self::number_setting(&self.settings, "libtorrent_half_open_limit", -1),
            alert_queue_size: Self::number_setting(
                &self.settings,
                "libtorrent_alert_queue_size",
                1000,
            ),
            max_connections_per_torrent: Self::number_setting(
                &self.settings,
                "libtorrent_max_connections_per_torrent",
                -1,
            ),
            max_uploads_per_torrent: Self::number_setting(
                &self.settings,
                "libtorrent_max_uploads_per_torrent",
                -1,
            ),
            aio_threads: Self::number_setting(&self.settings, "libtorrent_aio_threads", -1),
            cache_size: Self::number_setting(&self.settings, "libtorrent_cache_size", -1),
            cache_expiry: Self::number_setting(&self.settings, "libtorrent_cache_expiry", 300),
            announce_interval: Self::number_setting(
                &self.settings,
                "libtorrent_announce_interval",
                1800,
            ),
            torrent_connect_boost: Self::number_setting(
                &self.settings,
                "libtorrent_torrent_connect_boost",
                50,
            ),
            utp: Self::bool_setting_or(&self.settings, "libtorrent_utp", true),
            prefer_rc4: Self::bool_setting(self.settings.get("libtorrent_prefer_rc4")),
            announce_to_all_trackers: Self::bool_setting(
                self.settings.get("libtorrent_announce_to_all_trackers"),
            ),
            announce_to_all_tiers: Self::bool_setting(
                self.settings.get("libtorrent_announce_to_all_tiers"),
            ),
            allow_multiple_connections_per_ip: Self::bool_setting_or(
                &self.settings,
                "libtorrent_allow_multiple_connections_per_ip",
                true,
            ),
            apply_ip_filter: Self::bool_setting_or(
                &self.settings,
                "libtorrent_apply_ip_filter",
                true,
            ),
            encryption: Self::number_setting(&self.settings, "libtorrent_encryption", 1),
            proxy_type: Self::number_setting(&self.settings, "libtorrent_proxy_type", 0),
            proxy_host: self
                .settings
                .get("libtorrent_proxy_host")
                .cloned()
                .unwrap_or_default(),
            proxy_port: Self::number_setting(&self.settings, "libtorrent_proxy_port", 0),
            proxy_user: self
                .settings
                .get("libtorrent_proxy_user")
                .cloned()
                .unwrap_or_default(),
            proxy_password: self
                .settings
                .get("libtorrent_proxy_password")
                .cloned()
                .unwrap_or_default(),
            ip_filter_path: self
                .settings
                .get("libtorrent_ipfilter_url")
                .cloned()
                .unwrap_or_default(),
            listen_interfaces: self
                .settings
                .get("libtorrent_listen_interfaces")
                .cloned()
                .unwrap_or_default(),
            outgoing_interface: self
                .settings
                .get("libtorrent_outgoing_interface")
                .cloned()
                .unwrap_or_default(),
            dht_bootstrap_nodes: self
                .settings
                .get("libtorrent_dht_bootstrap_nodes")
                .cloned()
                .unwrap_or_default(),
        };
        self.libtorrent_dir = Self::path_setting(self.settings.get("libtorrent_dir"))
            .unwrap_or_else(|| self.libtorrent_dir.clone());
        self.libtorrent_temp_dir = Self::path_setting(self.settings.get("libtorrent_temp_dir"));
        Ok(())
    }

    pub fn from_env(mut self) -> Self {
        if let Ok(value) = env::var("REXTTO_REFRESH_SECS") {
            if let Ok(seconds) = value.parse() {
                self.refresh_secs = seconds;
            }
        }
        if let Ok(value) = env::var("REXTTO_ACTIVE") {
            self.active = value == "1" || value.eq_ignore_ascii_case("true");
        }
        if let Ok(value) = env::var("REXTTO_DRY_RUN") {
            self.dry_run = value != "0";
        }
        self.libtorrent_enabled = env::var("REXTTO_LIBTORRENT")
            .map(|value| value == "1")
            .unwrap_or(self.libtorrent_enabled);
        self
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut cfg = if path.exists() {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            Self::default()
        };
        let roots = [
            path.parent().unwrap_or_else(|| Path::new(".")),
            cfg.data_dir.as_path(),
            cfg.import_source_dir.as_path(),
        ];
        let _ = Self::migrate_legacy_files(&cfg.data_dir, &roots)?;
        cfg.load_config_db()?;
        Ok(cfg.from_env())
    }

    pub fn inspect_legacy_files(roots: &[&Path]) -> LegacyMigrationReport {
        let mut report = LegacyMigrationReport::default();
        for root in roots {
            for name in ["extto.conf", "series.txt", "movies.txt"] {
                let path = root.join(name);
                let display = path.display().to_string();
                if path.is_file() && !report.files_found.iter().any(|value| value == &display) {
                    report.files_found.push(display);
                }
            }
        }
        report
    }

    pub fn migrate_legacy_files(data_dir: &Path, roots: &[&Path]) -> Result<LegacyMigrationReport> {
        let mut report = Self::inspect_legacy_files(roots);
        if report.files_found.is_empty() {
            return Ok(report);
        }
        std::fs::create_dir_all(data_dir)?;
        let config_db = data_dir.join("rextto_config.db");
        if config_db.is_file() {
            let conn = open_config_db(&config_db)?;
            let count = conn
                .query_row("SELECT COUNT(*) FROM settings", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0);
            if count > 0 {
                return Ok(report);
            }
        }
        let root_file = |name: &str| {
            roots
                .iter()
                .map(|root| root.join(name))
                .find(|path| path.is_file())
        };
        let mut settings = BTreeMap::<String, String>::new();
        if let Some(path) = root_file("extto.conf") {
            let mut repeated = BTreeMap::<String, Vec<String>>::new();
            for line in std::fs::read_to_string(path)?.lines().map(str::trim) {
                if line.is_empty() || line.starts_with('#') || !line.starts_with('@') {
                    continue;
                }
                let Some((key, value)) = line[1..].split_once('=') else {
                    continue;
                };
                let key = key.trim().to_owned();
                let value = value
                    .split('#')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if key.is_empty() || value.is_empty() {
                    continue;
                }
                if matches!(
                    key.as_str(),
                    "url" | "feed_url" | "websearch_engines" | "blacklist" | "content_filters"
                ) {
                    repeated.entry(key).or_default().push(value);
                } else {
                    settings.insert(key, value);
                }
            }
            for (key, values) in repeated {
                settings.insert(key, serde_json::to_string(&values)?);
            }
        }
        let parse_bool = |value: &str| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "yes" | "true" | "1"
            )
        };
        let mut series = Vec::new();
        if let Some(path) = root_file("series.txt") {
            for line in std::fs::read_to_string(path)?
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
                if parts.len() < 5 {
                    continue;
                }
                let mut item = SeriesConfig {
                    name: parts[0].into(),
                    seasons: parts[1].into(),
                    quality: parts[2].into(),
                    language: parts[3].into(),
                    enabled: parse_bool(parts[4]),
                    ..Default::default()
                };
                for extra in parts.iter().skip(5) {
                    if let Some(value) = extra.strip_prefix("alias=") {
                        item.aliases = value
                            .split(',')
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned)
                            .collect();
                    } else if let Some(value) = extra.strip_prefix("ignored:") {
                        item.ignored_seasons = value
                            .split(',')
                            .filter_map(|value| value.trim().parse().ok())
                            .collect();
                    } else if let Some(value) = extra.strip_prefix("tmdb=") {
                        item.tmdb_id = value.trim().into();
                    } else if let Some(value) = extra.strip_prefix("subtitle=") {
                        item.subtitle = value.trim().into();
                    } else if let Some(value) = extra
                        .strip_prefix("timeframe:")
                        .and_then(|value| value.strip_suffix('h'))
                    {
                        item.timeframe = value.parse().unwrap_or(0);
                    } else if item.archive_path.is_empty() {
                        item.archive_path = (*extra).into();
                    }
                }
                series.push(item);
            }
        }
        let mut movies = Vec::new();
        if let Some(path) = root_file("movies.txt") {
            for line in std::fs::read_to_string(path)?
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
                if parts.len() < 3 {
                    continue;
                }
                movies.push(MovieConfig {
                    name: parts[0].into(),
                    year: parts.get(1).copied().unwrap_or_default().into(),
                    quality: parts.get(2).copied().unwrap_or("any").into(),
                    language: parts.get(3).copied().unwrap_or("ita").into(),
                    enabled: parts.get(4).map(|value| parse_bool(value)).unwrap_or(true),
                    subtitle: parts.get(5).copied().unwrap_or_default().into(),
                    ..Default::default()
                });
            }
        }
        for (key, value) in &settings {
            Self::save_setting(data_dir, key, value)?;
        }
        if !series.is_empty() || !movies.is_empty() {
            Self::save_library(data_dir, &series, &movies)?;
        }
        report.settings_imported = settings.len();
        report.series_imported = series.len();
        report.movies_imported = movies.len();
        report.migrated = true;
        Ok(report)
    }

    pub fn rename_legacy_files(roots: &[&Path]) -> Result<LegacyMigrationReport> {
        let mut report = Self::inspect_legacy_files(roots);
        for file in report.files_found.clone() {
            let target = format!("{file}.old");
            if !Path::new(&target).exists() {
                std::fs::rename(&file, &target)?;
                report.renamed.push(file);
            }
        }
        Ok(report)
    }

    pub fn save_setting(data_dir: &Path, key: &str, value: &str) -> Result<()> {
        let conn = open_config_db(&data_dir.join("rextto_config.db"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        conn.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key, value])?;
        Ok(())
    }

    pub fn delete_setting(data_dir: &Path, key: &str) -> Result<bool> {
        let conn = open_config_db(&data_dir.join("rextto_config.db"))?;
        let removed = conn.execute("DELETE FROM settings WHERE key=?1", [key])?;
        Ok(removed > 0)
    }

    pub fn save_library(
        data_dir: &Path,
        series: &[SeriesConfig],
        movies: &[MovieConfig],
    ) -> Result<()> {
        let mut conn = open_config_db(&data_dir.join("rextto_config.db"))?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS movies_config (id INTEGER PRIMARY KEY, name TEXT NOT NULL, year TEXT DEFAULT '', quality TEXT DEFAULT '', language TEXT DEFAULT '', enabled INTEGER DEFAULT 1, subtitle TEXT DEFAULT '', exclude TEXT DEFAULT '', language_requirements TEXT DEFAULT '', subtitle_requirements TEXT DEFAULT '', tmdb_id TEXT DEFAULT '', tvdb_id TEXT DEFAULT '', original_title TEXT DEFAULT '', overview TEXT DEFAULT '', poster_path TEXT DEFAULT '');")?;
        let _ = conn.execute(
            "ALTER TABLE movies_config ADD COLUMN exclude TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE movies_config ADD COLUMN language_requirements TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE movies_config ADD COLUMN subtitle_requirements TEXT NOT NULL DEFAULT ''",
            [],
        );
        for column in [
            "tmdb_id TEXT NOT NULL DEFAULT ''",
            "tvdb_id TEXT NOT NULL DEFAULT ''",
            "original_title TEXT NOT NULL DEFAULT ''",
            "overview TEXT NOT NULL DEFAULT ''",
            "poster_path TEXT NOT NULL DEFAULT ''",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE movies_config ADD COLUMN {column}"), []);
        }
        // Conserva gli id esistenti anche quando il client invia `id: 0`
        // (film aggiunti di recente), abbinando per nome+anno. Evita di
        // riassegnare un id nuovo ad ogni salvataggio e i conseguenti dettagli
        // film vuoti quando si clicca una voce appena aggiunta.
        let mut existing_ids: std::collections::HashMap<(String, String), i64> =
            std::collections::HashMap::new();
        {
            let mut statement = conn.prepare("SELECT id,name,year FROM movies_config")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows.flatten() {
                existing_ids.insert((row.1.to_lowercase(), row.2), row.0);
            }
        }
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO settings(key,value) VALUES ('_migrated_series',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![serde_json::to_string(series)?])?;
        tx.execute("DELETE FROM movies_config", [])?;
        for movie in movies {
            let id = if movie.id > 0 {
                movie.id
            } else {
                existing_ids
                    .get(&(movie.name.to_lowercase(), movie.year.clone()))
                    .copied()
                    .unwrap_or(0)
            };
            if id > 0 {
                tx.execute("INSERT INTO movies_config(id,name,year,quality,language,enabled,subtitle,exclude,language_requirements,subtitle_requirements,tmdb_id,tvdb_id,original_title,overview,poster_path) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)", rusqlite::params![id, movie.name, movie.year, movie.quality, movie.language, movie.enabled as i64, movie.subtitle, movie.exclude, movie.language_requirements, movie.subtitle_requirements, movie.tmdb_id, movie.tvdb_id, movie.original_title, movie.overview, movie.poster_path])?;
            } else {
                tx.execute("INSERT INTO movies_config(name,year,quality,language,enabled,subtitle,exclude,language_requirements,subtitle_requirements,tmdb_id,tvdb_id,original_title,overview,poster_path) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)", rusqlite::params![movie.name, movie.year, movie.quality, movie.language, movie.enabled as i64, movie.subtitle, movie.exclude, movie.language_requirements, movie.subtitle_requirements, movie.tmdb_id, movie.tvdb_id, movie.original_title, movie.overview, movie.poster_path])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prepare_dirs(&self) -> Result<()> {
        for path in [&self.data_dir, &self.state_dir, &self.libtorrent_dir] {
            std::fs::create_dir_all(path)?;
        }
        if let Some(path) = &self.libtorrent_temp_dir {
            std::fs::create_dir_all(path)?;
        }
        if let Some(path) = &self.trash_path {
            std::fs::create_dir_all(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_archive_path_auto_detects_series_folder() {
        let root = std::env::temp_dir().join(format!("rextto-archive-{}", uuid::Uuid::new_v4()));
        let base = root.join("archive-root");
        std::fs::create_dir_all(base.join("Example Show")).unwrap();
        let mut cfg = Config::default();
        cfg.archive_root = Some(base.clone());

        let series = SeriesConfig {
            name: "Example Show".into(),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolve_archive_path(&series),
            Some(base.join("Example Show"))
        );

        // `archive_path` configurato ha la priorità sull'auto-detect.
        let configured = SeriesConfig {
            name: "Example Show".into(),
            archive_path: "/custom/path".into(),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolve_archive_path(&configured),
            Some(std::path::PathBuf::from("/custom/path"))
        );

        // Nessuna cartella corrispondente: nessun risultato.
        let other = SeriesConfig {
            name: "Altra Serie".into(),
            ..Default::default()
        };
        assert_eq!(cfg.resolve_archive_path(&other), None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_dirs_does_not_create_the_import_source() {
        let root = std::env::temp_dir().join(format!("rextto-config-{}", uuid::Uuid::new_v4()));
        let mut cfg = Config::default();
        cfg.data_dir = root.join("data");
        cfg.state_dir = cfg.data_dir.join("state");
        cfg.libtorrent_dir = cfg.data_dir.join("downloads");
        cfg.libtorrent_temp_dir = Some(cfg.data_dir.join("incomplete"));
        cfg.trash_path = Some(cfg.data_dir.join("trash"));
        cfg.import_source_dir = root.join("external-legacy");

        cfg.prepare_dirs().unwrap();
        assert!(cfg.data_dir.is_dir());
        assert!(!cfg.import_source_dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[test]
    fn normalizes_language_codes() {
        assert_eq!(normalize_language_code("IT"), "ita");
        assert_eq!(normalize_language_code("ITA"), "ita");
        assert_eq!(normalize_language_code("italian"), "ita");
        assert_eq!(normalize_language_code("English"), "eng");
        assert_eq!(normalize_language_code("eng"), "eng");
        assert_eq!(normalize_language_code(" custom "), "custom");
    }

    #[test]
    fn language_requirements_skip_non_required_and_parse_csv() {
        let json = r#"[{"language":"ita","required":true},{"language":"eng","required":false},{"language":"fra"}]"#;
        assert_eq!(
            parse_language_requirements(json),
            vec!["ita".to_string(), "fra".to_string()]
        );
        assert_eq!(
            parse_language_requirements("ita, eng"),
            vec!["ita".to_string(), "eng".to_string()]
        );
        assert_eq!(
            parse_language_requirements("EN+ita"),
            vec!["eng".to_string(), "ita".to_string()]
        );
        assert!(parse_language_requirements("   ").is_empty());
    }

    #[test]
    fn preserves_movie_ids_when_client_sends_zero() {
        let dir = std::env::temp_dir().join(format!("rextto-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let movie = |id: i64| MovieConfig {
            id,
            name: "Afterburn".into(),
            year: "2025".into(),
            ..Default::default()
        };
        let read_id = || {
            let conn = Connection::open(dir.join("rextto_config.db")).unwrap();
            conn.query_row(
                "SELECT id FROM movies_config WHERE name='Afterburn'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
        };
        Config::save_library(&dir, &[], &[movie(0)]).unwrap();
        let assigned = read_id();
        assert!(assigned > 0);
        // Un secondo salvataggio con `id: 0` (come farebbe la UI) non deve
        // riassegnare un id diverso.
        Config::save_library(&dir, &[], &[movie(0)]).unwrap();
        assert_eq!(read_id(), assigned);
        // Un id esplicito valido viene mantenuto.
        Config::save_library(&dir, &[], &[movie(12345)]).unwrap();
        assert_eq!(read_id(), 12345);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn matches_enabled_series_alias_and_respects_ignored_season() {
        let mut cfg = Config::default();
        cfg.series.push(SeriesConfig {
            name: "Grey's Anatomy".into(),
            aliases: vec!["Greys Anatomy".into()],
            enabled: true,
            ignored_seasons: vec![14],
            ..Default::default()
        });
        assert_eq!(
            cfg.find_series_match("Greys.Anatomy", Some(15))
                .map(|series| series.name.as_str()),
            Some("Grey's Anatomy")
        );
        assert!(cfg.find_series_match("Greys Anatomy", Some(14)).is_none());
    }

    #[test]
    fn matches_only_configured_season_ranges() {
        let mut cfg = Config::default();
        cfg.series.push(SeriesConfig {
            name: "Example".into(),
            seasons: "1,3-4,7+".into(),
            enabled: true,
            ..Default::default()
        });
        assert!(cfg.find_series_match("Example", Some(1)).is_some());
        assert!(cfg.find_series_match("Example", Some(4)).is_some());
        assert!(cfg.find_series_match("Example", Some(8)).is_some());
        assert!(cfg.find_series_match("Example", Some(2)).is_none());
    }

    #[test]
    fn content_filter_excludes_not_requires() {
        let filters = vec!["[non-latino]".to_string()];
        assert!(!title_is_content_filtered(
            "Example S01E01 1080p ITA",
            &filters
        ));
        assert!(title_is_content_filtered("Пример сериала 1080p", &filters));
        let words = vec!["anime".to_string()];
        assert!(title_is_content_filtered("Some.Anime.S01E01", &words));
        assert!(!title_is_content_filtered("Some.Show.S01E01", &words));
        let adult = vec!["[porno]".to_string()];
        assert!(title_is_content_filtered(
            "Brazzers.Exxtra.22.03.13.MILF",
            &adult
        ));
        assert!(title_is_content_filtered("My.Porn.Movie.2024", &adult));
        assert!(!title_is_content_filtered(
            "Analisi.Di.Un.Film.2024",
            &adult
        ));
        assert!(!title_is_content_filtered("The.Analyst.2024.1080p", &adult));
    }

    #[test]
    fn blacklist_uses_word_boundaries() {
        let patterns = vec!["ts".to_string()];
        assert!(!title_is_blacklisted("Subtitles.ITA.1080p", &patterns));
        assert!(title_is_blacklisted("Movie.TS.1080p", &patterns));
    }

    #[test]
    fn movie_language_requirements_json_is_parsed() {
        let movie = MovieConfig {
            name: "Example".into(),
            quality: "any".into(),
            language: "ita".into(),
            language_requirements: "[{\"language\": \"ita\", \"required\": true}]".into(),
            ..Default::default()
        };
        let mut italian = crate::models::Quality::default();
        italian.languages = vec!["ita".into()];
        italian.language = "ita".into();
        assert!(Config::movie_release_allowed(&movie, &italian));
        let mut english = crate::models::Quality::default();
        english.languages = vec!["eng".into()];
        english.language = "eng".into();
        assert!(!Config::movie_release_allowed(&movie, &english));
    }

    #[test]
    fn movie_language_requirements_plain_list_is_parsed() {
        let movie = MovieConfig {
            name: "Example".into(),
            quality: "any".into(),
            language: "ita,eng".into(),
            ..Default::default()
        };
        let mut english = crate::models::Quality::default();
        english.languages = vec!["eng".into()];
        english.language = "eng".into();
        assert!(Config::movie_release_allowed(&movie, &english));
    }

    #[test]
    fn movie_matching_requires_year_and_honors_exclusions() {
        let mut cfg = Config::default();
        cfg.movies.push(MovieConfig {
            name: "The Batman".into(),
            year: "2022".into(),
            enabled: true,
            exclude: "animated,extended".into(),
            ..Default::default()
        });
        assert!(cfg
            .find_movie_match("The.Batman.2022.1080p.WEB-DL", Some(2022))
            .is_some());
        assert!(cfg
            .find_movie_match("The.Batman.2025.1080p.WEB-DL", Some(2025))
            .is_none());
        assert!(cfg
            .find_movie_match("The.Batman.2022.Extended.1080p", Some(2022))
            .is_none());
        cfg.movies.push(MovieConfig {
            name: "No Year Movie".into(),
            enabled: true,
            ..Default::default()
        });
        assert!(cfg
            .find_movie_match("No.Year.Movie.2022", Some(2022))
            .is_none());
    }

    #[test]
    fn quality_rules_support_ranges_and_multi_language_requirements() {
        let quality = crate::models::Quality {
            resolution: "1080p".into(),
            language: "ita".into(),
            languages: vec!["ita".into(), "eng".into()],
            ..Default::default()
        };
        assert!(Config::quality_allowed(
            &quality,
            "720p-1080p",
            "ita,eng",
            ""
        ));
        assert!(Config::quality_allowed(
            &crate::models::Quality {
                resolution: "2160p".into(),
                ..quality.clone()
            },
            "1080p",
            "ita",
            ""
        ));
        assert!(!Config::quality_allowed(
            &crate::models::Quality {
                resolution: "2160p".into(),
                ..quality.clone()
            },
            "720p-1080p",
            "ita",
            ""
        ));
        assert!(!Config::quality_allowed(
            &crate::models::Quality {
                language: "deu".into(),
                languages: vec!["deu".into()],
                ..quality
            },
            "",
            "ita,eng",
            ""
        ));
        let movie = MovieConfig {
            language: "eng".into(),
            subtitle_requirements: "ita".into(),
            ..Default::default()
        };
        assert!(Config::movie_release_allowed(
            &movie,
            &crate::models::Quality {
                language: "eng".into(),
                languages: vec!["eng".into()],
                has_subtitle: true,
                subtitle_languages: vec!["ita".into()],
                ..Default::default()
            }
        ));
        assert!(!Config::movie_release_allowed(
            &movie,
            &crate::models::Quality {
                language: "eng".into(),
                languages: vec!["eng".into()],
                has_subtitle: true,
                subtitle_languages: vec!["eng".into()],
                ..Default::default()
            }
        ));
    }

    #[test]
    fn global_filters_reject_blacklisted_and_stale_releases() {
        let mut cfg = Config::default();
        cfg.blacklist = vec!["cam".into()];
        cfg.content_filters = vec!["[non-latino]".into()];
        cfg.max_release_age_days = 7;
        let release = |title: &str, age_days: i64| crate::models::Release {
            title: title.into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "test".into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2026),
            discovered_at: Utc::now() - chrono::Duration::days(age_days),
        };
        assert!(cfg.release_allowed(&release("Movie.1080p.ITA", 1)));
        assert!(!cfg.release_allowed(&release("Movie.1080p.CAM.ITA", 1)));
        assert!(!cfg.release_allowed(&release("Фильм.1080p.ITA", 1)));
        assert!(cfg.release_allowed(&release("Movie.1080p.ENG", 1)));
        assert!(!cfg.release_allowed(&release("Movie.1080p.ITA", 8)));
    }

    #[test]
    fn source_filters_block_only_their_own_source() {
        let mut cfg = Config::default();
        cfg.source_filters = vec![SourceFilter {
            source: "ExtTo - MIRCrewRS".into(),
            keywords: vec!["x265".into()],
            enabled: true,
        }];
        let release = |title: &str, source: &str| crate::models::Release {
            title: title.into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: source.into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2026),
            discovered_at: Utc::now(),
        };
        // Blocked: matching source and keyword.
        assert!(!cfg.release_allowed(&release("Film 1080p x265", "ExtTo - MIRCrewRS")));
        // Different source: allowed even with the keyword.
        assert!(cfg.release_allowed(&release("Film 1080p x265", "jackett")));
        // Same source, no keyword: allowed.
        assert!(cfg.release_allowed(&release("Film 1080p h264", "ExtTo - MIRCrewRS")));
        // Disabled filter never blocks.
        cfg.source_filters[0].enabled = false;
        assert!(cfg.release_allowed(&release("Film 1080p x265", "ExtTo - MIRCrewRS")));
    }

    #[test]
    fn saves_and_reloads_library_configuration() {
        let dir = std::env::temp_dir().join(format!("rextto-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let series = vec![SeriesConfig {
            name: "Example".into(),
            enabled: true,
            ..Default::default()
        }];
        let movies = vec![MovieConfig {
            name: "Example Movie".into(),
            year: "2024".into(),
            enabled: true,
            ..Default::default()
        }];
        Config::save_library(&dir, &series, &movies).unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        cfg.load_config_db().unwrap();
        assert_eq!(cfg.series[0].name, "Example");
        assert_eq!(cfg.movies[0].name, "Example Movie");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_and_reloads_movie_static_metadata() {
        let dir = std::env::temp_dir().join(format!("rextto-movie-meta-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let movies = vec![MovieConfig {
            name: "Titolo Italiano".into(),
            year: "2024".into(),
            tmdb_id: "12345".into(),
            original_title: "Original Title".into(),
            overview: "Trama del film.".into(),
            poster_path: "/poster.jpg".into(),
            quality: "1080p".into(),
            language: "ita".into(),
            enabled: true,
            ..Default::default()
        }];
        Config::save_library(&dir, &[], &movies).unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        cfg.load_config_db().unwrap();
        let movie = &cfg.movies[0];
        assert_eq!(movie.tmdb_id, "12345");
        assert_eq!(movie.original_title, "Original Title");
        assert_eq!(movie.overview, "Trama del film.");
        assert_eq!(movie.poster_path, "/poster.jpg");
        // I dati di download restano separati e invariati.
        assert_eq!(movie.quality, "1080p");
        assert_eq!(movie.language, "ita");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn persists_active_setting_without_forcing_dry_run_defaults() {
        let dir = std::env::temp_dir().join(format!("rextto-active-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Config::save_setting(&dir, "active", "yes").unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        cfg.load_config_db().unwrap();
        assert!(cfg.active);
        assert!(cfg.dry_run);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn persists_global_filters_from_json_or_csv_values() {
        let dir = std::env::temp_dir().join(format!("rextto-filters-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Config::save_setting(&dir, "blacklist", r#"["cam","ts"]"#).unwrap();
        Config::save_setting(&dir, "content_filters", "ita,web-dl").unwrap();
        Config::save_setting(&dir, "max_release_age_days", "14").unwrap();
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        cfg.load_config_db().unwrap();
        assert_eq!(cfg.blacklist, vec!["cam", "ts"]);
        assert_eq!(cfg.content_filters, vec!["ita", "web-dl"]);
        assert_eq!(cfg.max_release_age_days, 14);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_imported_series_and_legacy_extto_filter_keys() {
        let dir =
            std::env::temp_dir().join(format!("rextto-legacy-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Config::save_setting(&dir, "content_filter", "ita").unwrap();
        Config::save_setting(&dir, "max_age_days", "21").unwrap();
        let db = crate::database::Database::open(&dir.join("rextto_series.db")).unwrap();
        db.conn.execute("INSERT INTO series(name,seasons,quality,language,enabled,archive_path,tmdb_id,aliases,timeframe,ignored_seasons,subtitle,exclude) VALUES ('Imported Show','1-3','1080p','ita',1,'/nas/shows','123','[\"Imported Alias\"]',48,'[2]','yes','cam')", []).unwrap();
        drop(db);
        let mut cfg = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        cfg.load_config_db().unwrap();
        assert_eq!(cfg.content_filters, vec!["ita"]);
        assert_eq!(cfg.max_release_age_days, 21);
        assert_eq!(cfg.series.len(), 1);
        assert_eq!(cfg.series[0].timeframe, 48);
        assert_eq!(cfg.series[0].ignored_seasons, vec![2]);
        assert_eq!(cfg.series[0].exclude, "cam");
        let _ = std::fs::remove_dir_all(dir);
    }
}
