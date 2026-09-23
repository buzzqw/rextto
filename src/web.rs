use crate::{
    archive::Archive,
    backup,
    comics::{self, ComicMonitored, ComicsDb, GetComicsClient},
    config::{Config, IndexerConfig, MovieConfig, SeriesConfig, SourceFilter},
    database::Database,
    engine::Engine,
    health,
    i18n::I18nDb,
    importer,
    integrations::{token_string, SimklClient, TraktClient},
    libtorrent::LibtorrentClient,
    models::{CycleStats, Release, TorrentEvent},
    notifier::Notifier,
    orchestrator, parser, postprocess,
    tmdb::TmdbClient,
};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

/// Short-lived response cache for endpoints that call TMDB (calendar, recent
/// downloads). Avoids repeating dozens of TMDB requests on every UI load/poll.
static RESPONSE_CACHE: OnceLock<Mutex<HashMap<String, (Instant, serde_json::Value)>>> =
    OnceLock::new();

fn cache_get(key: &str, ttl: Duration) -> Option<serde_json::Value> {
    let cache = RESPONSE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = cache.lock().unwrap();
    guard
        .get(key)
        .and_then(|(at, value)| (at.elapsed() < ttl).then(|| value.clone()))
}

fn cache_put(key: &str, value: &serde_json::Value) {
    let cache = RESPONSE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    cache
        .lock()
        .unwrap()
        .insert(key.to_string(), (Instant::now(), value.clone()));
}
use tower_http::services::ServeDir;
use tracing_subscriber::{reload, EnvFilter};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Config,
    pub config_path: std::path::PathBuf,
    pub i18n: Arc<I18nDb>,
    pub db: Arc<Mutex<Database>>,
    pub archive: Arc<Mutex<Archive>>,
    pub comics: Arc<ComicsDb>,
    pub engine: Arc<Engine>,
    pub torrents: Arc<LibtorrentClient>,
    pub torrent_events: Arc<Mutex<Vec<TorrentEvent>>>,
    pub notifier: Arc<Notifier>,
    pub tmdb: Arc<TmdbClient>,
    pub last_cycle: Arc<Mutex<CycleStats>>,
    pub cycle_lock: Arc<tokio::sync::Mutex<()>>,
    pub log_reload: Arc<Mutex<reload::Handle<EnvFilter, tracing_subscriber::Registry>>>,
    pub rename_progress: Arc<Mutex<RenameProgress>>,
}

/// Progress of a background rename-all job, polled by `/api/rename-progress`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RenameProgress {
    pub running: bool,
    pub current: usize,
    pub total: usize,
    pub series: String,
    pub message: String,
    pub errors: usize,
}

/// Series whose archive is currently being written by a torrent
/// post-processing run (pack copy/rename).
///
/// The manual and periodic rename repair scans the archive and would otherwise
/// rename or trash files while they are still being copied, leaving transient
/// duplicates and half-written paths in the database. While a series is listed
/// here the repair pass skips it.
static ARCHIVE_IMPORT_BUSY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn archive_import_busy() -> &'static Mutex<HashSet<String>> {
    ARCHIVE_IMPORT_BUSY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// RAII marker for "this series is being imported right now".
struct ArchiveImportGuard {
    series: Option<String>,
}

impl ArchiveImportGuard {
    fn acquire(series: Option<&str>) -> Self {
        let series = series
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(name) = &series {
            archive_import_busy().lock().unwrap().insert(name.clone());
        }
        Self { series }
    }
}

impl Drop for ArchiveImportGuard {
    fn drop(&mut self) {
        if let Some(name) = &self.series {
            archive_import_busy().lock().unwrap().remove(name);
        }
    }
}

#[derive(serde::Deserialize)]
pub struct LogLevel {
    pub level: String,
}
#[derive(serde::Deserialize)]
pub struct ComicInput {
    pub title: String,
    pub tag_url: String,
    pub from_date: String,
    pub save_path: String,
}
#[derive(serde::Deserialize)]
pub struct ComicLinksInput {
    pub url: String,
}
#[derive(serde::Deserialize)]
pub struct ComicExploreInput {
    pub query: String,
}
#[derive(serde::Deserialize)]
pub struct ComicDownloadInput {
    pub url: String,
    pub method: String,
    pub title: String,
    #[serde(default)]
    pub post_url: String,
    #[serde(default)]
    pub save_path: String,
}
#[derive(serde::Deserialize)]
pub struct ComicWeeklyInput {
    pub date: String,
}
#[derive(serde::Deserialize)]
pub struct ComicEnabled {
    pub enabled: bool,
}
#[derive(serde::Deserialize)]
pub struct SettingInput {
    pub key: String,
    pub value: String,
}
#[derive(serde::Deserialize)]
pub struct AuthCode {
    pub code: String,
}
#[derive(serde::Deserialize, Default)]
pub struct LibraryInput {
    #[serde(default)]
    pub series: Vec<crate::config::SeriesConfig>,
    #[serde(default)]
    pub movies: Vec<crate::config::MovieConfig>,
}
#[derive(serde::Deserialize)]
pub struct ScrobbleInput {
    pub action: Option<String>,
    pub payload: serde_json::Value,
}
#[derive(serde::Deserialize)]
pub struct I18nInput {
    pub lang: String,
    pub key: String,
    pub value: String,
}
#[derive(serde::Deserialize)]
pub struct LanguageInput {
    pub lang: String,
}
#[derive(serde::Deserialize)]
pub struct I18nYamlInput {
    pub yaml: String,
}
#[derive(serde::Deserialize)]
pub struct I18nQuery {
    pub lang: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct ArchiveQuery {
    pub query: Option<String>,
    pub q: Option<String>,
    pub page: Option<usize>,
    pub limit: Option<usize>,
}
#[derive(serde::Deserialize)]
pub struct SeenQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
    /// Ricerca testuale nei gruppi (`*`/`?` come wildcard).
    pub q: Option<String>,
    /// Chiave di gruppo (`group_key`) per il dettaglio di un titolo.
    pub key: Option<String>,
    pub group: Option<String>,
    pub kind: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
    pub q: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct LogQuery {
    pub limit: Option<usize>,
}
#[derive(serde::Deserialize, Default)]
pub struct RunNowQuery {
    pub domain: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct AddTorrent {
    pub magnet: String,
    #[serde(default)]
    pub save_path: Option<String>,
    /// Se true il torrent viene aggiunto in pausa (non parte subito).
    #[serde(default)]
    pub start_paused: bool,
    /// Se true il torrent non viene rinominato al termine.
    #[serde(default)]
    pub no_rename: bool,
}
#[derive(serde::Deserialize)]
pub struct ScorePreviewInput {
    pub title: String,
}
#[derive(serde::Deserialize, Default)]
pub struct NoRenameInput {
    #[serde(default)]
    pub value: bool,
}
#[derive(serde::Deserialize)]
pub struct RemoveTorrentInput {
    pub hash: String,
    #[serde(default)]
    pub delete_files: bool,
}
#[derive(serde::Deserialize, Default)]
pub struct RemoveCompletedInput {
    #[serde(default)]
    pub delete_files: bool,
}
#[derive(serde::Deserialize, Default)]
pub struct CleanTrashInput {
    /// When true, remove every trash entry instead of applying retention.
    #[serde(default)]
    pub force: bool,
}
#[derive(serde::Deserialize, Default)]
pub struct RemoveOptionsInput {
    #[serde(default)]
    pub delete_files: bool,
    #[serde(default)]
    pub blocklist: bool,
}
#[derive(serde::Deserialize)]
pub struct TorrentLimitsInput {
    pub hash: String,
    pub dl_kbps: i64,
    pub ul_kbps: i64,
    #[serde(default)]
    pub seed_ratio: Option<f64>,
    #[serde(default)]
    pub seed_days: Option<i64>,
}
#[derive(serde::Deserialize)]
pub struct TorrentHashInput {
    pub hash: String,
}
#[derive(serde::Deserialize)]
pub struct SequentialInput {
    #[serde(default)]
    pub enabled: bool,
}
#[derive(serde::Deserialize)]
pub struct TorrentTagInput {
    pub hash: String,
    #[serde(default)]
    pub tag: String,
}
#[derive(serde::Deserialize)]
pub struct SearchInput {
    pub query: String,
}
#[derive(serde::Deserialize)]
pub struct ManualSearchQuery {
    pub q: Option<String>,
    pub query: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct SearchAddInput {
    pub release: Release,
}
#[derive(serde::Deserialize)]
pub struct ArchiveAddInput {
    pub title: String,
    pub magnet: String,
    #[serde(default)]
    pub source: String,
}
#[derive(serde::Deserialize)]
pub struct ArchiveDeleteInput {
    #[serde(default)]
    pub magnet: String,
    #[serde(default)]
    pub ids: Vec<i64>,
}
#[derive(serde::Deserialize)]
pub struct ArchiveBatchItem {
    pub title: String,
    pub magnet: String,
    #[serde(default)]
    pub source: String,
}
#[derive(serde::Deserialize)]
pub struct ArchiveBatchInput {
    pub items: Vec<ArchiveBatchItem>,
}
#[derive(serde::Deserialize)]
pub struct IgnoreEpisodeInput {
    pub ignored: bool,
    #[serde(default)]
    pub reason: String,
}
#[derive(serde::Deserialize)]
pub struct MissingSearchInput {
    pub series: String,
    pub season: i64,
    pub episode: i64,
}
#[derive(serde::Deserialize, Default)]
pub struct ArchiveScanInput {
    pub path: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct TmdbQuery {
    pub query: String,
    #[serde(default = "default_tmdb_kind")]
    pub kind: String,
}
#[derive(serde::Deserialize)]
pub struct TmdbAddInput {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub year: String,
    pub tmdb_id: String,
    #[serde(default)]
    pub tvdb_id: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub seasons: String,
    #[serde(default)]
    pub archive_path: String,
    #[serde(default)]
    pub exclude: String,
}
fn default_tmdb_kind() -> String {
    "series".into()
}
#[derive(serde::Deserialize, Default)]
pub struct DiscoverInput {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct TorrentLimits {
    pub download_limit: i64,
    pub upload_limit: i64,
    #[serde(default)]
    pub seed_ratio: Option<f64>,
    #[serde(default)]
    pub seed_days: Option<i64>,
}
#[derive(serde::Deserialize)]
pub struct StoragePath {
    pub path: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TagDirRule {
    pub tag: String,
    #[serde(default)]
    pub temp_dir: String,
    pub final_dir: String,
}
#[derive(serde::Deserialize)]
pub struct SeasonToggleInput {
    pub season: i64,
    pub enabled: bool,
}
#[derive(serde::Deserialize, Default)]
pub struct SeriesPathInput {
    #[serde(default)]
    pub archive_path: Option<String>,
    #[serde(default)]
    pub timeframe: Option<i64>,
}
#[derive(serde::Deserialize)]
pub struct MovieUpdateInput {
    pub name: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub exclude: String,
    #[serde(default)]
    pub language_requirements: String,
    #[serde(default)]
    pub subtitle_requirements: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub tmdb_id: String,
    #[serde(default)]
    pub tvdb_id: String,
}
#[derive(serde::Deserialize)]
pub struct MovieMetadataSearchInput {
    pub query: String,
    #[serde(default = "default_metadata_source")]
    pub source: String,
}
#[derive(serde::Deserialize)]
pub struct MovieMetadataApplyInput {
    pub id: String,
    #[serde(default = "default_metadata_source")]
    pub source: String,
}
fn default_metadata_source() -> String {
    "tmdb".to_string()
}
#[derive(serde::Deserialize, Default)]
pub struct PruneInput {
    #[serde(default)]
    pub retain_cycles: Option<i64>,
    #[serde(default)]
    pub error_age_days: Option<i64>,
    /// Età in giorni oltre la quale eliminare le release "viste nei feed"
    /// (assente o 0 = conserva tutto).
    #[serde(default)]
    pub seen_retention_days: Option<i64>,
    /// When true, only report what would be removed (no deletion).
    #[serde(default)]
    pub preview: bool,
}
#[derive(serde::Deserialize, Default)]
pub struct DbActionInput {
    #[serde(default)]
    pub action: String,
}
#[derive(serde::Deserialize, Default)]
pub struct TestNotificationInput {
    #[serde(default)]
    pub message: Option<String>,
}
#[derive(serde::Deserialize)]
pub struct SettingsPatch {
    #[serde(flatten)]
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(serde::Deserialize, Default)]
pub struct SourceFiltersInput {
    #[serde(default)]
    pub filters: Vec<SourceFilter>,
}

#[derive(serde::Deserialize, Default)]
pub struct PruneByIdsInput {
    #[serde(default)]
    pub movie_seen: Vec<i64>,
    #[serde(default)]
    pub series_seen: Vec<i64>,
}

/// Removes specific "seen in feed" rows by id, e.g. the ones selected in the
/// feed preview. Ids come from `/api/movies/seen` and `/api/series/seen`.
async fn db_prune_by_ids(
    State(s): State<AppState>,
    Json(input): Json<PruneByIdsInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify the database"})),
        );
    }
    match s
        .db
        .lock()
        .unwrap()
        .prune_seen_by_ids(&input.movie_seen, &input.series_seen)
    {
        Ok((movies, series)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "movie_seen_removed": movies,
                "series_seen_removed": series,
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn source_filters_view(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"filters":cfg.source_filters})),
    )
}

/// Replaces the per-source filters. Stored as a JSON array in the `source_filters`
/// setting, so no schema change is needed and it is picked up on the next
/// config reload.
async fn save_source_filters(
    State(s): State<AppState>,
    Json(input): Json<SourceFiltersInput>,
) -> impl IntoResponse {
    if input.filters.len() > 200 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"too many source filters (max 200)"})),
        )
            .into_response();
    }
    let payload = match serde_json::to_string(&input.filters) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
                .into_response()
        }
    };
    if payload.len() > 100_000 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"ok":false,"error":"source filters too large"})),
        )
            .into_response();
    }
    match Config::save_setting(&s.cfg.data_dir, "source_filters", &payload) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"filters":input.filters})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        )
            .into_response(),
    }
}
#[derive(serde::Deserialize)]
pub struct ComicCheckLinksInput {
    pub urls: Vec<String>,
}

fn setup_marker(cfg: &Config) -> std::path::PathBuf {
    cfg.data_dir.join(".rextto-setup.json")
}
fn setup_complete(cfg: &Config) -> bool {
    setup_marker(cfg).is_file()
}
fn latest_config(state: &AppState) -> Config {
    Config::load(&state.config_path).unwrap_or_else(|_| state.cfg.clone())
}
fn complete_setup(cfg: &Config) -> anyhow::Result<()> {
    std::fs::write(
        setup_marker(cfg),
        serde_json::to_vec_pretty(
            &serde_json::json!({"completed_at": chrono::Utc::now().to_rfc3339()}),
        )?,
    )?;
    Ok(())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/magnet", get(magnet_handler))
        .route("/favicon.ico", get(favicon))
        .route("/pkg/ui_bg.wasm", get(wasm_alias))
        .nest_service("/pkg", ServeDir::new("ui/target/site/pkg"))
        .route("/api/auth", get(auth_status))
        .route("/api/i18n", get(i18n_list).post(i18n_set))
        .route("/api/i18n/language", post(i18n_language))
        .route("/api/i18n/languages", get(i18n_languages))
        .route("/api/i18n/active", get(i18n_active).post(i18n_language))
        .route("/api/i18n/export/{lang}", get(i18n_export))
        .route("/api/i18n/import/{lang}", post(i18n_import))
        .route("/api/i18n/{lang}", delete(i18n_delete_lang))
        .route("/api/health", get(health_api))
        .route("/api/status", get(status))
        .route("/api/logs", get(logs))
        .route("/api/logs/stream", get(logs_stream))
        .route("/api/notifications/stream", get(notifications_stream))
        .route("/api/config", get(config_view).post(save_config_root))
        .route("/api/config/migration-status", get(migration_status))
        .route("/api/config/migrate", post(migrate_config))
        .route("/api/config/rename-old", post(rename_old_config))
        .route("/api/config/library", get(library_view).post(save_library))
        .route("/api/config/series", post(save_series_config))
        .route("/api/config/movies", post(save_movies_config))
        .route("/api/config/settings", post(save_setting))
        .route("/api/config/settings/{key}", delete(delete_setting))
        .route(
            "/api/config/source-filters",
            get(source_filters_view).post(save_source_filters),
        )
        .route(
            "/api/tag-dir-rules",
            get(tag_dir_rules).post(save_tag_dir_rules),
        )
        .route("/api/trakt/status", get(trakt_status))
        .route("/api/trakt/auth/start", post(trakt_auth_start))
        .route("/api/trakt/auth/poll", post(trakt_auth_poll))
        .route("/api/trakt/auth/refresh", post(trakt_auth_refresh))
        .route("/api/trakt/auth/revoke", post(trakt_auth_revoke))
        .route("/api/trakt/watchlist", get(trakt_watchlist))
        .route("/api/trakt/calendar", get(trakt_calendar))
        .route("/api/trakt/scrobble", post(trakt_scrobble))
        .route("/api/simkl/status", get(simkl_status))
        .route("/api/simkl/auth/start", post(simkl_auth_start))
        .route("/api/simkl/auth/poll", post(simkl_auth_poll))
        .route("/api/simkl/auth/revoke", post(simkl_auth_revoke))
        .route("/api/simkl/watchlist", get(simkl_watchlist))
        .route("/api/simkl/calendar", get(simkl_calendar))
        .route("/api/simkl/scrobble", post(simkl_scrobble))
        .route("/api/cycles", get(cycles))
        .route("/api/cycle-history", get(cycles))
        .route("/api/stats", get(stats_api))
        .route("/api/recent-downloads", get(recent_downloads))
        .route("/api/last_cycle", get(last_cycle))
        .route("/api/last_cycles", get(cycles))
        .route("/api/gaps", get(gaps))
        .route("/api/series/{name}/episodes", get(series_episodes))
        .route("/api/series/{name}/scan-archive", post(scan_series_archive))
        .route("/api/series/{name}/toggle-season", post(toggle_season))
        .route(
            "/api/series/{name}/search-missing",
            post(series_search_missing),
        )
        .route("/api/series/{name}/path", post(set_series_path))
        .route("/api/series/{name}/metadata", post(series_metadata_refresh))
        .route("/api/series/{name}/info", get(series_info))
        .route(
            "/api/series/{name}/rename-preview",
            post(series_rename_preview),
        )
        .route(
            "/api/series/{name}/rename-execute",
            post(series_rename_execute),
        )
        .route("/api/rename-progress", get(rename_progress_view))
        .route("/api/rename-all", post(rename_all))
        .route("/api/series/{name}", get(series_detail))
        .route("/api/series", get(series_list))
        .route("/api/series/all-missing", get(gaps))
        .route("/api/series/calendar", get(calendar))
        .route("/api/movies/history", get(movie_history))
        .route("/api/movies", get(movie_list))
        .route(
            "/api/movies/{id}",
            get(movie_detail).post(update_movie).delete(delete_movie),
        )
        .route(
            "/api/movies/{id}/metadata/search",
            post(movie_metadata_search),
        )
        .route("/api/movies/{id}/metadata", post(apply_movie_metadata))
        .route("/api/movies/{id}/redownload", post(redownload_movie))
        .route("/api/movies/{id}/search", post(movie_search))
        .route(
            "/api/episodes/{series}/{season}/{episode}/ignore",
            post(ignore_episode),
        )
        .route(
            "/api/episodes/{series}/{season}/{episode}/force",
            post(force_episode),
        )
        .route(
            "/api/episodes/{series}/{season}/{episode}/redownload",
            post(redownload_episode),
        )
        .route(
            "/api/episodes/{series}/{season}/{episode}/search",
            post(search_episode),
        )
        .route(
            "/api/episodes/{series}/{season}/{episode}",
            axum::routing::delete(delete_episode),
        )
        .route("/api/missing/search", post(search_missing))
        .route("/api/calendar", get(calendar))
        .route("/api/scan-all-archives", post(scan_all_archives))
        .route("/api/database/rescore", post(rescore_database))
        .route("/api/score/preview", post(score_preview))
        .route("/api/database/cleanup", post(cleanup_database))
        .route("/api/log-level", post(set_log_level))
        .route("/api/log_level", get(log_level_get).post(set_log_level))
        .route("/api/comics", get(comics).post(add_comic))
        .route("/api/comics/explore", post(comic_explore))
        .route("/api/comics/downloads", get(comic_downloads))
        .route("/api/comics/links", post(comic_links))
        .route("/api/comics/download", post(comic_download))
        .route("/api/comics/weekly/links", post(comic_weekly_links))
        .route("/api/comics/weekly/settings", post(comic_weekly_settings))
        .route("/api/comics/{id}/enabled", post(set_comic_enabled))
        .route("/api/comics/{id}", axum::routing::delete(remove_comic))
        .route("/api/comics/history", get(comics_history))
        .route("/api/comics/history/delete", post(delete_comics_history))
        .route("/api/comics/weekly", get(comics_weekly))
        .route("/api/browse_dir", get(browse_dir))
        .route("/api/mkdir", post(make_directory))
        .route("/api/trash/delete", post(delete_trash_entries))
        .route("/api/backup", get(list_backups).post(create_backup))
        .route("/api/backup/list", get(list_backups))
        .route(
            "/api/backup/settings",
            get(backup_settings).post(save_backup_settings),
        )
        .route("/api/backup/send-telegram", post(backup_send_telegram))
        .route("/api/backup/test-ftp", post(backup_test_ftp))
        .route("/api/test-notification", post(test_notification))
        .route("/api/sources/health", get(sources_health))
        .route("/api/flaresolverr/test", post(flaresolverr_test))
        .route("/api/config/check-ports", get(check_ports))
        .route("/api/network/interfaces", get(network_interfaces_view))
        .route("/api/db/info", get(db_info))
        .route("/api/db/action", post(db_action))
        .route("/api/db/prune", post(db_prune))
        .route("/api/db/prune/preview", post(db_prune_preview))
        .route("/api/db/prune-by-ids", post(db_prune_by_ids))
        .route("/api/db/prune-keyword", post(db_prune_keyword))
        .route("/api/trash", get(trash_entries))
        .route("/api/send-magnet", post(send_magnet))
        .route("/api/upload-torrent", post(upload_torrent))
        .route(
            "/api/browser-handlers/download",
            get(browser_handler_download),
        )
        .route(
            "/api/trakt/settings",
            get(trakt_settings).post(save_trakt_settings),
        )
        .route(
            "/api/simkl/settings",
            get(simkl_settings).post(save_simkl_settings),
        )
        .route("/api/trakt/mark-watched", post(trakt_scrobble))
        .route("/api/simkl/mark-watched", post(simkl_scrobble))
        .route("/api/trakt/watchlist/import", post(trakt_watchlist_import))
        .route("/api/simkl/watchlist/import", post(simkl_watchlist_import))
        .route("/api/comics/check-links", post(comic_check_links))
        .route("/api/comics/cycle", post(comic_cycle))
        .route("/api/tmdb/discover", post(tmdb_discover))
        .route("/api/system/stats", get(system_stats))
        .route("/api/setup", get(setup_status))
        .route("/api/setup/import", post(setup_import))
        .route("/api/setup/complete", post(setup_complete_existing))
        .route("/api/torrents", get(torrents))
        .route("/api/torrents/stats", get(torrent_stats))
        .route("/api/torrents/add", post(add_torrent))
        .route("/api/torrents/remove", post(remove_torrent_legacy))
        .route(
            "/api/torrents/remove_completed",
            post(remove_completed_torrents),
        )
        .route("/api/torrents/set_limits", post(set_torrent_limits_legacy))
        .route("/api/torrents/peers", post(torrent_peers_legacy))
        .route("/api/torrents/history", get(torrent_history))
        .route("/api/archive", get(archive_entries))
        .route("/api/archive/add", post(add_archive_entry))
        .route("/api/archive/delete", post(delete_archive_entry))
        .route("/api/archive/batch-download", post(batch_archive_download))
        .route("/api/movies/seen/grouped", get(movies_seen_grouped_view))
        .route("/api/movies/seen", get(movies_seen_view))
        .route("/api/series/seen/grouped", get(series_seen_grouped_view))
        .route("/api/series/seen", get(series_seen_view))
        .route("/api/torrent-events", get(torrent_events))
        .route("/api/torrent-no-rename", get(torrent_no_rename_list))
        .route("/api/torrents", post(add_torrent))
        .route("/api/blocklist", get(blocklist_entries))
        .route("/api/blocklist/{hash}/remove", post(remove_blocklist_entry))
        .route(
            "/api/torrents/{hash}/mark_failed",
            post(mark_torrent_failed),
        )
        .route("/api/maintenance/clean-trash", post(clean_trash))
        .route(
            "/api/maintenance/clean-duplicates",
            post(clean_duplicates),
        )
        .route("/api/maintenance/restore-source", post(restore_source))
        .route("/api/search", post(manual_search))
        .route("/api/manual-search", get(manual_search_get))
        .route("/api/search/add", post(add_search_result))
        .route("/api/tmdb/search", post(tmdb_search))
        .route("/api/tvdb/search", post(tvdb_search))
        .route("/api/tvdb/series/{id}", get(tvdb_series))
        .route("/api/tmdb/add", post(tmdb_add))
        .route("/api/torrents/{hash}/pause", post(pause_torrent))
        .route("/api/torrents/{hash}/resume", post(resume_torrent))
        .route("/api/torrents/{hash}/restart", post(restart_torrent))
        .route("/api/torrents/pin", post(pin_torrent))
        .route("/api/torrents/unpin", post(unpin_torrent))
        .route("/api/torrents/sequential", post(set_sequential))
        .route("/api/torrent-tags", get(torrent_tags).post(set_torrent_tag))
        .route("/api/torrents/{hash}/recheck", post(recheck_torrent))
        .route(
            "/api/torrents/{hash}/no_rename",
            post(set_torrent_no_rename),
        )
        .route("/api/torrents/{hash}/reannounce", post(reannounce_torrent))
        .route("/api/torrents/{hash}/storage", post(move_torrent_storage))
        .route("/api/torrents/{hash}/limits", post(set_torrent_limits))
        .route("/api/set-speed-limits", post(set_speed_limits))
        .route("/api/torrents/temp-limits", post(set_temp_limits))
        .route("/api/torrents/ipfilter_update", post(ipfilter_update))
        .route("/api/torrents/ipfilter_status", get(ipfilter_status))
        .route(
            "/api/torrents/apply_settings",
            post(apply_libtorrent_settings),
        )
        .route(
            "/api/torrents/optimize_settings",
            post(optimize_libtorrent_settings),
        )
        .route("/api/service/restart", post(service_restart))
        .route("/api/services", get(services_status))
        .route(
            "/api/libtorrent/check-update",
            post(libtorrent_check_update),
        )
        .route("/api/jellyfin/refresh", post(jellyfin_refresh))
        .route("/api/jellyfin/test", post(jellyfin_test))
        .route("/api/plex/refresh", post(plex_refresh))
        .route("/api/plex/test", post(plex_test))
        .route("/api/feed/status", get(feed_status))
        .route("/feed.xml", get(magnet_feed))
        .route("/api/feed.xml", get(magnet_feed))
        .route("/api/torrents/{hash}/peers", get(torrent_peers))
        .route("/api/torrents/{hash}/trackers", get(torrent_trackers))
        .route("/api/torrents/{hash}/files", get(torrent_files))
        .route(
            "/api/torrents/{hash}",
            get(torrent_details).delete(remove_torrent),
        )
        .route(
            "/api/torrents/{hash}/remove",
            post(remove_torrent_with_options),
        )
        .route("/api/run_now", get(run_now).post(run_now))
        .route("/api/run-now", post(run_now))
        .layer(middleware::from_fn_with_state(state.clone(), api_auth))
        .with_state(state)
}

async fn index() -> Html<String> {
    let bundle = FsPath::new("ui/target/site/pkg/ui.js");
    if bundle.is_file() {
        return Html(
            r#"<!doctype html>
<html lang="it">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Rextto</title>
  <link rel="stylesheet" href="/pkg/ui.css">
</head>
<body>
  <script type="module">
    import init, { hydrate } from "/pkg/ui.js";
    init().then(() => hydrate());
  </script>
</body>
</html>"#
                .to_string(),
        );
    }
    Html(include_str!("../web/index.html").to_string())
}
/// Pagina di atterraggio per i link `magnet:` quando Rextto è registrato come
/// gestore nel browser: inoltra il magnet alla sessione e mostra l'esito.
async fn magnet_handler() -> Html<&'static str> {
    Html(MAGNET_PAGE)
}
const MAGNET_PAGE: &str = r#"<!doctype html>
<html lang="it">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Rextto — aggiungi magnet</title>
  <style>
    body { font-family: Inter, system-ui, sans-serif; background: #0a0f1a; color: #e9f0fb; display: grid; place-items: center; min-height: 100vh; margin: 0; }
    main { max-width: 560px; padding: 28px; text-align: center; }
    h1 { font-size: 20px; margin: 0 0 8px; }
    p { color: #9aaccb; }
    a { color: #6cb8ff; }
    .ok { color: #7ce7bd; }
    .err { color: #ff9fb0; word-break: break-word; }
  </style>
</head>
<body>
  <main>
    <h1>Rextto</h1>
    <p id="status">Invio del magnet alla sessione…</p>
    <p><a href="/">Torna a Rextto</a></p>
  </main>
  <script>
    const magnet = new URLSearchParams(location.search).get("url") || decodeURIComponent(location.hash.slice(1));
    const status = document.getElementById("status");
    if (!magnet) {
      status.textContent = "Nessun magnet ricevuto.";
      status.className = "err";
    } else {
      fetch("/api/send-magnet", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ magnet }),
      })
        .then(async (response) => {
          const payload = await response.json().catch(() => ({}));
          if (!response.ok) throw payload.error || ("HTTP " + response.status);
          status.textContent = "Magnet aggiunto alla sessione.";
          status.className = "ok";
        })
        .catch((error) => {
          status.textContent = "Errore: " + error;
          status.className = "err";
        });
    }
  </script>
</body>
</html>"#;
async fn wasm_alias() -> Response {
    match tokio::fs::read("ui/target/site/pkg/ui.wasm").await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/wasm")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn favicon() -> Response {
    match tokio::fs::read("ui/target/site/favicon.ico").await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/x-icon")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn auth_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"required":s.cfg.api_token.is_some()}))
}
async fn i18n_list(State(s): State<AppState>, Query(query): Query<I18nQuery>) -> impl IntoResponse {
    let lang = query
        .lang
        .unwrap_or_else(|| s.i18n.language().unwrap_or_else(|_| "it".into()));
    match s.i18n.list(&lang) {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"lang":lang,"items":items})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn i18n_set(State(s): State<AppState>, Json(input): Json<I18nInput>) -> impl IntoResponse {
    if input.lang.len() > 16
        || input.key.trim().is_empty()
        || input.key.len() > 256
        || input.value.len() > 4096
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid translation"})),
        );
    }
    match s.i18n.set(&input.lang, &input.key, &input.value) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn i18n_language(
    State(s): State<AppState>,
    Json(input): Json<LanguageInput>,
) -> impl IntoResponse {
    if !matches!(input.lang.as_str(), "it" | "en") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"language must be it or en"})),
        );
    }
    match s.i18n.set_language(&input.lang) {
        Ok(()) => {
            // Keep backend notifications/log summaries in the same language.
            crate::messages::set_language(&input.lang);
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"lang":input.lang,"restart_required":true})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn i18n_languages() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"ok":true,"items":[{"lang":"it","name":"Italiano"},{"lang":"en","name":"English"}]}),
    )
}
async fn i18n_active(State(s): State<AppState>) -> impl IntoResponse {
    match s.i18n.language() {
        Ok(lang) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"lang":lang})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn i18n_export(State(s): State<AppState>, Path(lang): Path<String>) -> impl IntoResponse {
    match s.i18n.list(&lang) {
        Ok(items) => {
            let values = items
                .into_iter()
                .map(|item| (item.key, item.value))
                .collect::<std::collections::BTreeMap<_, _>>();
            match serde_yaml::to_string(&values) {
                Ok(yaml) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/yaml; charset=utf-8")],
                    yaml,
                )
                    .into_response(),
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                )
                    .into_response(),
            }
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        )
            .into_response(),
    }
}
async fn i18n_delete_lang(
    State(s): State<AppState>,
    Path(lang): Path<String>,
) -> impl IntoResponse {
    if lang.trim().is_empty() || lang.len() > 32 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid language code"})),
        );
    }
    match s.i18n.delete_lang(&lang) {
        Ok(removed) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"lang":lang,"removed":removed})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn i18n_import(
    State(s): State<AppState>,
    Path(lang): Path<String>,
    Json(input): Json<I18nYamlInput>,
) -> impl IntoResponse {
    if input.yaml.len() > 2_000_000 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"ok":false,"error":"translation file too large"})),
        );
    }
    let values =
        match serde_yaml::from_str::<std::collections::HashMap<String, String>>(&input.yaml) {
            Ok(values) => values,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                )
            }
        };
    match s.i18n.set_bulk(&lang, &values) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"imported":values.len()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn api_auth(
    State(s): State<AppState>,
    request: Request,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path();
    if !path.starts_with("/api/")
        || matches!(path, "/api/auth" | "/api/health" | "/api/status")
        || s.cfg.api_token.is_none()
    {
        return next.run(request).await;
    }
    let supplied = request
        .headers()
        .get("x-rextto-token")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
        });
    if supplied == s.cfg.api_token.as_deref() {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"ok":false,"error":"API token required"})),
    )
        .into_response()
}
async fn health_api(State(s): State<AppState>) -> Json<health::Health> {
    let trash = s
        .cfg
        .trash_path
        .clone()
        .unwrap_or_else(|| s.cfg.data_dir.join("trash"));
    let ramdisk = s
        .cfg
        .settings
        .get("libtorrent_ramdisk_dir")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    Json(health::check_with_paths(&health::HealthPaths {
        data_dir: &s.cfg.data_dir,
        trash_path: &trash,
        download_path: &s.cfg.libtorrent_dir,
        archive_root: s.cfg.archive_root.as_deref(),
        ramdisk_path: ramdisk.as_deref(),
    }))
}
async fn setup_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let marker = setup_complete(&s.cfg);
    let mut completed = marker;
    let mut auto_completed = false;
    if !marker {
        let has_series = s.db.lock().unwrap().has_data().unwrap_or(false);
        let has_archive = s.archive.lock().unwrap().count().unwrap_or(0) > 0;
        if has_series || has_archive {
            if complete_setup(&s.cfg).is_ok() {
                completed = true;
                auto_completed = true;
            }
        }
    }
    Json(serde_json::json!({
        "completed": completed,
        "import_source": s.cfg.import_source_dir,
        "auto_completed": auto_completed,
    }))
}
async fn status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let (seen_movies, seen_series) = s.db.lock().unwrap().seen_counts().unwrap_or((0, 0));
    Json(serde_json::json!({
        "name":"rextto", "version":format!("1.0.{}", env!("REXTTO_BUILD")), "active":cfg.active,
        "dry_run":cfg.dry_run, "setup_completed":setup_complete(&cfg),
        "last_cycle":*s.last_cycle.lock().unwrap(), "torrent_stats":s.torrents.stats(),
        "seen": {"movies": seen_movies, "series": seen_series, "groups": seen_movies + seen_series}
    }))
}
async fn logs(State(s): State<AppState>, Query(query): Query<LogQuery>) -> Json<serde_json::Value> {
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);
    let current = s.cfg.data_dir.join("rextto.log");
    let mut lines = std::fs::read_to_string(&current)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.len() < limit {
        // Include the most recent rotated backup to fill the requested window.
        let backup = s.cfg.data_dir.join("rextto.log.1");
        if let Ok(previous) = std::fs::read_to_string(&backup) {
            let mut previous = previous.lines().map(str::to_owned).collect::<Vec<_>>();
            previous.extend(lines);
            lines = previous;
        }
    }
    let start = lines.len().saturating_sub(limit);
    Json(serde_json::json!({"items":lines[start..].to_vec()}))
}

/// Stream SSE del log: invia le ultime righe alla connessione e poi segue il
/// file (`rextto.log`) con un polling interno di 1 secondo, ripartendo da zero
/// se il file viene troncato dalla rotazione.
async fn logs_stream(State(s): State<AppState>) -> impl IntoResponse {
    let path = s.cfg.data_dir.join("rextto.log");
    let stream = async_stream::stream! {
        let mut sent = 0usize;
        if let Ok(contents) = tokio::fs::read_to_string(&path).await {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(200);
            for (offset, line) in lines[start..].iter().enumerate() {
                yield Ok::<Event, Infallible>(
                    Event::default().id((start + offset).to_string()).data((*line).to_string()),
                );
            }
            sent = lines.len();
        }
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Ok(contents) = tokio::fs::read_to_string(&path).await else {
                continue;
            };
            let lines: Vec<&str> = contents.lines().collect();
            if lines.len() < sent {
                sent = 0;
            }
            for offset in sent..lines.len() {
                yield Ok::<Event, Infallible>(
                    Event::default().id(offset.to_string()).data(lines[offset].to_string()),
                );
            }
            sent = lines.len();
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Server-sent stream of torrent lifecycle events (metadata received, finished,
/// storage moved). New entries appended to the shared buffer are pushed as they
/// appear, so the Activity view can react in real time instead of polling
/// `/api/torrent-events`.
async fn notifications_stream(State(s): State<AppState>) -> impl IntoResponse {
    let events = s.torrent_events.clone();
    let stream = async_stream::stream! {
        let mut cursor = { events.lock().unwrap().len() };
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let batch = {
                let guard = events.lock().unwrap();
                // The buffer drops its oldest entries once it exceeds its cap.
                if guard.len() < cursor {
                    cursor = 0;
                }
                let items = guard[cursor..].to_vec();
                cursor = guard.len();
                items
            };
            for item in batch {
                let data = serde_json::to_string(&item).unwrap_or_default();
                yield Ok::<Event, Infallible>(Event::default().event("torrent").data(data));
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn config_view(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    Json(serde_json::json!({
        "active": cfg.active,
        "dry_run": cfg.dry_run,
        "refresh_secs": cfg.refresh_secs,
        "series_count": cfg.series.len(),
        "movie_count": cfg.movies.len(),
        "feed_urls": cfg.feed_urls,
        "indexers": cfg.indexers.iter().map(|indexer| serde_json::json!({"name":indexer.name,"url":indexer.url,"api_key":indexer.api_key,"enabled":indexer.enabled})).collect::<Vec<_>>(),
        "indexer_api_keys_configured": cfg.indexers.iter().filter(|indexer| !indexer.api_key.trim().is_empty()).count(),
        "flaresolverr_url": cfg.flaresolverr_url,
        "websearch_engines": cfg.websearch_engines,
        "blacklist": cfg.blacklist,
        "content_filters": cfg.content_filters,
        "source_filters": cfg.source_filters,
        "max_release_age_days": cfg.max_release_age_days,
        "gap_fill_max_per_series": cfg.settings.get("gap_fill_max_per_series").and_then(|value| value.parse::<usize>().ok()).unwrap_or(0),
        "gap_fill_max_per_cycle": cfg.settings.get("gap_fill_max_per_cycle").and_then(|value| value.parse::<usize>().ok()).unwrap_or(30),
        "gap_filling": cfg.settings.get("gap_filling").map(|value| matches!(value.as_str(), "yes" | "true" | "1")).unwrap_or(true),
        "gap_deep_interval_hours": cfg.settings.get("gap_deep_interval_hours").and_then(|value| value.parse::<i64>().ok()).unwrap_or(6),
        "gap_deep_max_per_cycle": cfg.settings.get("gap_deep_max_per_cycle").and_then(|value| value.parse::<usize>().ok()).unwrap_or(5),
        "libtorrent_sched_enabled": cfg.settings.get("libtorrent_sched_enabled").cloned().unwrap_or_else(|| "false".into()),
        "libtorrent_sequential": cfg.settings.get("libtorrent_sequential").cloned().unwrap_or_else(|| "false".into()),
        "libtorrent_extra_settings": cfg.settings.get("libtorrent_extra_settings").cloned().unwrap_or_default(),
        "libtorrent_sched_start": cfg.settings.get("libtorrent_sched_start").cloned().unwrap_or_else(|| "23:00".into()),
        "libtorrent_sched_end": cfg.settings.get("libtorrent_sched_end").cloned().unwrap_or_else(|| "08:00".into()),
        "libtorrent_sched_days": cfg.settings.get("libtorrent_sched_days").cloned().unwrap_or_default(),
        "libtorrent_sched_dl_limit": cfg.settings.get("libtorrent_sched_dl_limit").cloned().unwrap_or_else(|| "0".into()),
        "libtorrent_sched_ul_limit": cfg.settings.get("libtorrent_sched_ul_limit").cloned().unwrap_or_else(|| "0".into()),
        "libtorrent_temp_dl_limit": cfg.settings.get("libtorrent_temp_dl_limit").cloned().unwrap_or_else(|| "0".into()),
        "libtorrent_temp_ul_limit": cfg.settings.get("libtorrent_temp_ul_limit").cloned().unwrap_or_else(|| "0".into()),
        "libtorrent_temp_limit_enabled": cfg.settings.get("libtorrent_temp_limit_enabled").cloned().unwrap_or_else(|| "0".into()),
        "libtorrent_temp_limit_until": cfg.settings.get("libtorrent_temp_limit_until").cloned().unwrap_or_else(|| "0".into()),
        "score_settings": cfg.settings.iter().filter(|(key, _)| key.starts_with("score_")).collect::<std::collections::BTreeMap<_, _>>(),
        "flaresolverr_configured": cfg.flaresolverr_url.is_some(),
        "tmdb_configured": cfg.tmdb_api_key.is_some(),
        "jellyfin_configured": cfg.settings.get("jellyfin_url").is_some_and(|value| !value.trim().is_empty())
            && cfg.settings.get("jellyfin_api_key").is_some_and(|value| !value.trim().is_empty()),
        "jellyfin_url": cfg.settings.get("jellyfin_url").cloned().unwrap_or_default(),
        "plex_configured": cfg.settings.get("plex_url").is_some_and(|value| !value.trim().is_empty())
            && cfg.settings.get("plex_token").is_some_and(|value| !value.trim().is_empty()),
        "plex_url": cfg.settings.get("plex_url").cloned().unwrap_or_default(),
        "tmdb_api_key": cfg.tmdb_api_key.clone().unwrap_or_default(),
        "tmdb_language": cfg.tmdb_language(),
        "default_language": cfg.default_language(),
        "tvdb_api_key": cfg.tvdb_api_key().unwrap_or_default(),
        "tvdb_configured": cfg.tvdb_api_key().is_some(),
        "tvdb_language": cfg.tvdb_language(),
        "trakt_configured": !cfg.settings.get("trakt_client_id").unwrap_or(&String::new()).is_empty(),
        "trakt_authenticated": !cfg.settings.get("trakt_access_token").unwrap_or(&String::new()).is_empty(),
        "simkl_configured": !cfg.settings.get("simkl_client_id").unwrap_or(&String::new()).is_empty(),
        "simkl_authenticated": !cfg.settings.get("simkl_access_token").unwrap_or(&String::new()).is_empty(),
        "backup_send_telegram": cfg.settings.get("backup_send_telegram").is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
        "notify_telegram": cfg.notify_telegram,
        "notify_email": cfg.notify_email,
        "telegram_configured": cfg.telegram_bot_token.is_some() && cfg.telegram_chat_id.is_some(),
        "telegram_chat_id": cfg.telegram_chat_id,
        "email_smtp": cfg.email_smtp,
        "email_from": cfg.email_from,
        "email_to": cfg.email_to,
        "email_password_configured": cfg.email_password.is_some(),
        "webhook_configured": cfg.notify_webhook_url.is_some(),
        "notify_webhook_url": cfg.notify_webhook_url,
        "backup_retention": cfg.settings.get("backup_retention").cloned().unwrap_or_else(|| "5".into()),
        "backup_cloud_dir": cfg.settings.get("backup_cloud_dir").cloned().unwrap_or_default(),
        "min_free_space_gb": cfg.settings.get("min_free_space_gb").cloned().unwrap_or_else(|| "0".into()),
        "trash_retention_days": cfg.settings.get("trash_retention_days").cloned().unwrap_or_else(|| "0".into()),
        "archive_retention_days": cfg.settings.get("archive_retention_days").cloned().unwrap_or_else(|| "0".into()),
        "archive_cleanup_enabled": cfg.settings.get("archive_cleanup_enabled").cloned().unwrap_or_else(|| "false".into()),
        "archive_max_age_days": cfg.settings.get("archive_max_age_days").cloned().unwrap_or_else(|| "0".into()),
        "archive_keep_min": cfg.settings.get("archive_keep_min").cloned().unwrap_or_else(|| "0".into()),
        "stop_on_old_page_threshold": cfg.settings.get("stop_on_old_page_threshold").cloned().unwrap_or_else(|| "3".into()),
        "debug_enabled": cfg.settings.get("debug_enabled").cloned().unwrap_or_else(|| "false".into()),
        "move_episodes": cfg.settings.get("move_episodes").cloned().unwrap_or_else(|| "false".into()),
        "rename_verify_interval": cfg.settings.get("rename_verify_interval").cloned().unwrap_or_else(|| "6".into()),
        "cleanup_upgrades": cfg.cleanup_upgrades,
        "cleanup_action": cfg.cleanup_action,
        "auto_remove_completed": cfg.libtorrent.auto_remove_completed,
        "rename_format": cfg.rename_format,
        "rename_template": cfg.rename_template,
        "rename_episodes": cfg.rename_episodes,
        "libtorrent": {
            "enabled": cfg.libtorrent_enabled,
            "version": crate::libtorrent::libtorrent_version(),
            "active_downloads": cfg.libtorrent.active_downloads,
            "active_seeds": cfg.libtorrent.active_seeds,
            "active_limit": cfg.libtorrent.active_limit,
            "seed_ratio": cfg.libtorrent.seed_ratio,
            "seed_time_minutes": cfg.libtorrent.seed_time_minutes,
            "seed_time_days": cfg.libtorrent.seed_time_days,
            "download_limit_kib": cfg.libtorrent.download_limit_kib,
            "upload_limit_kib": cfg.libtorrent.upload_limit_kib,
            "stop_at_ratio": cfg.libtorrent.stop_at_ratio,
            "dynamic_queue": cfg.libtorrent.dynamic_queue,
            "dynamic_queue_min": cfg.libtorrent.dynamic_queue_min,
            "dynamic_queue_max": cfg.libtorrent.dynamic_queue_max,
            "auto_remove_completed": cfg.libtorrent.auto_remove_completed,
            "dht": cfg.libtorrent.dht,
            "pex": cfg.libtorrent.pex,
            "lsd": cfg.libtorrent.lsd,
            "upnp": cfg.libtorrent.upnp,
            "natpmp": cfg.libtorrent.natpmp,
            "connections_limit": cfg.libtorrent.connections_limit,
            "upload_slots_limit": cfg.libtorrent.upload_slots_limit,
            "half_open_limit": cfg.libtorrent.half_open_limit,
            "alert_queue_size": cfg.libtorrent.alert_queue_size,
            "max_connections_per_torrent": cfg.libtorrent.max_connections_per_torrent,
            "max_uploads_per_torrent": cfg.libtorrent.max_uploads_per_torrent,
            "aio_threads": cfg.libtorrent.aio_threads,
            "cache_size": cfg.libtorrent.cache_size,
            "cache_expiry": cfg.libtorrent.cache_expiry,
            "announce_interval": cfg.libtorrent.announce_interval,
            "torrent_connect_boost": cfg.libtorrent.torrent_connect_boost,
            "utp": cfg.libtorrent.utp,
            "prefer_rc4": cfg.libtorrent.prefer_rc4,
            "announce_to_all_trackers": cfg.libtorrent.announce_to_all_trackers,
            "announce_to_all_tiers": cfg.libtorrent.announce_to_all_tiers,
            "allow_multiple_connections_per_ip": cfg.libtorrent.allow_multiple_connections_per_ip,
            "apply_ip_filter": cfg.libtorrent.apply_ip_filter,
            "encryption": cfg.libtorrent.encryption,
            "proxy_type": cfg.libtorrent.proxy_type,
            "proxy_host": cfg.libtorrent.proxy_host,
            "proxy_port": cfg.libtorrent.proxy_port,
            "ip_filter_path": cfg.libtorrent.ip_filter_path,
            "listen_interfaces": cfg.libtorrent.listen_interfaces,
            "outgoing_interface": cfg.libtorrent.outgoing_interface,
            "dht_bootstrap_nodes": cfg.libtorrent.dht_bootstrap_nodes,
            "port_min": cfg.libtorrent.port_min,
            "port_max": cfg.libtorrent.port_max,
            "ramdisk_enabled": cfg.ramdisk_enabled(),
            "ramdisk_threshold_gb": cfg.settings.get("libtorrent_ramdisk_threshold_gb").cloned().unwrap_or_else(|| "3.5".into()),
            "ramdisk_margin_gb": cfg.settings.get("libtorrent_ramdisk_margin_gb").cloned().unwrap_or_else(|| "0.5".into()),
            "ramdisk_min_free_bytes": cfg.settings.get("libtorrent_ramdisk_min_free_bytes").cloned().unwrap_or_default(),
            "auto_optimize": cfg.settings.get("libtorrent_auto_optimize").cloned().unwrap_or_else(|| "false".into())
        },
         "paths": {"data_dir":cfg.data_dir,"libtorrent_dir":cfg.libtorrent_dir,"libtorrent_temp_dir":cfg.libtorrent_temp_dir,"libtorrent_ramdisk_dir":cfg.settings.get("libtorrent_ramdisk_dir"),"state_dir":cfg.state_dir,"archive_root":cfg.archive_root,"trash_path":cfg.trash_path}
    }))
}
async fn save_config_root(
    State(s): State<AppState>,
    Json(input): Json<LibraryInput>,
) -> impl IntoResponse {
    save_library(State(s), Json(input)).await
}
fn legacy_roots<'a>(state: &'a AppState, cfg: &'a Config) -> [&'a std::path::Path; 3] {
    [
        state
            .config_path
            .parent()
            .unwrap_or_else(|| FsPath::new(".")),
        cfg.data_dir.as_path(),
        cfg.import_source_dir.as_path(),
    ]
}
async fn migration_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let report = Config::inspect_legacy_files(&legacy_roots(&s, &cfg));
    Json(
        serde_json::json!({"ok":true,"source_available":!report.files_found.is_empty(),"target_exists":cfg.data_dir.join("rextto_config.db").is_file(),"can_migrate":!report.files_found.is_empty(),"files":report.files_found}),
    )
}
async fn migrate_config(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let roots = legacy_roots(&s, &cfg);
    match Config::migrate_legacy_files(&cfg.data_dir, &roots) {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"report":report})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn rename_old_config(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match Config::rename_legacy_files(&legacy_roots(&s, &cfg)) {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"report":report})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn tag_dir_rules(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let rules = cfg
        .settings
        .get("tag_dir_rules")
        .and_then(|value| serde_json::from_str::<Vec<TagDirRule>>(value).ok())
        .unwrap_or_default();
    Json(serde_json::json!({"ok":true,"items":rules}))
}
async fn save_tag_dir_rules(
    State(s): State<AppState>,
    Json(rules): Json<Vec<TagDirRule>>,
) -> impl IntoResponse {
    if rules.len() > 100
        || rules.iter().any(|rule| {
            rule.tag.trim().is_empty()
                || rule.final_dir.trim().is_empty()
                || rule.tag.len() > 128
                || rule.final_dir.len() > 4096
                || rule.temp_dir.len() > 4096
        })
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid tag directory rules"})),
        );
    }
    match serde_json::to_string(&rules).and_then(|value| {
        Config::save_setting(&s.cfg.data_dir, "tag_dir_rules", &value)
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error)))
    }) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":rules})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn library_view(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let db = s.db.lock().unwrap();
    let statuses = db.series_statuses().unwrap_or_default();
    let series = cfg
        .series
        .iter()
        .map(|series| {
            // Usa lo stesso insieme di episodi del dettaglio: include le
            // puntate TMDB note ma non ancora materializzate nella tabella
            // downloads. Il vecchio COUNT(episodes) mostrava falsi 140/140
            // mentre il dettaglio, correttamente, riportava 140/143.
            let mut ignored_seasons = series.ignored_seasons.clone();
            for (season, _) in db.series_season_counts(&series.name).unwrap_or_default() {
                if !Config::season_allowed_for_scan(&series.seasons, season)
                    && !ignored_seasons.contains(&season)
                {
                    ignored_seasons.push(season);
                }
            }
            let episodes = db
                .episodes_for_series(&series.name, &ignored_seasons)
                .unwrap_or_default();
            let total = episodes.len() as i64;
            let downloaded = episodes
                .iter()
                .filter(|episode| {
                    episode.status == "downloaded"
                        || episode.archive_path.as_deref().is_some_and(|path| !path.is_empty())
                })
                .count() as i64;
            let last = episodes
                .iter()
                .filter_map(|episode| episode.downloaded_at.clone())
                .max();
            serde_json::json!({
                "name": series.name,
                "seasons": series.seasons,
                "quality": series.quality,
                "language": series.language,
                "archive_path": series.archive_path,
                "enabled": series.enabled,
                "aliases": series.aliases,
                "tmdb_id": series.tmdb_id,
                "subtitle": series.subtitle,
                "exclude": series.exclude,
                "ignored_seasons": series.ignored_seasons,
                "season_subfolders": series.season_subfolders,
                "timeframe": series.timeframe,
                "episodes_total": total,
                "episodes_downloaded": downloaded,
                "last_downloaded_at": last,
                "tmdb_status": statuses.get(&series.name).cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({"series":series,"movies":cfg.movies}))
}
async fn save_library(
    State(s): State<AppState>,
    Json(input): Json<LibraryInput>,
) -> impl IntoResponse {
    if input.series.len() > 500
        || input.movies.len() > 500
        || input
            .series
            .iter()
            .any(|series| series.name.trim().is_empty())
        || input
            .movies
            .iter()
            .any(|movie| movie.name.trim().is_empty())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"invalid or oversized library configuration"}),
            ),
        );
    }
    match Config::save_library(&s.cfg.data_dir, &input.series, &input.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"restart_required":true,"series":input.series.len(),"movies":input.movies.len()}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn save_series_config(
    State(s): State<AppState>,
    Json(series): Json<Vec<SeriesConfig>>,
) -> impl IntoResponse {
    if series.len() > 500 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"too many series"})),
        );
    }
    let cfg = latest_config(&s);
    match Config::save_library(&s.cfg.data_dir, &series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"series":series.len()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn save_movies_config(
    State(s): State<AppState>,
    Json(movies): Json<Vec<MovieConfig>>,
) -> impl IntoResponse {
    if movies.len() > 500 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"too many movies"})),
        );
    }
    let cfg = latest_config(&s);
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"movies":movies.len()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn series_list(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    Json(
        serde_json::json!({"ok":true,"items":cfg.series.iter().enumerate().map(|(index, series)| serde_json::json!({"id":index + 1,"name":series.name,"seasons":series.seasons,"quality":series.quality,"language":series.language,"archive_path":series.archive_path,"enabled":series.enabled,"aliases":series.aliases,"tmdb_id":series.tmdb_id,"timeframe":series.timeframe})).collect::<Vec<_>>()}),
    )
}
async fn movie_list(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    Json(serde_json::json!({"ok":true,"items":cfg.movies}))
}
fn find_series<'a>(cfg: &'a Config, name: &str) -> Option<&'a SeriesConfig> {
    cfg.series
        .iter()
        .find(|series| series.name == name || series.aliases.iter().any(|alias| alias == name))
}
async fn series_detail(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    let db = s.db.lock().unwrap();
    // `ignored_seasons` gestisce i toggle espliciti; il campo `seasons`
    // (es. `8+`, `1-3,5`) è un secondo vincolo e deve dare la stessa vista
    // nel dettaglio, nei gap e nella ricerca manuale.
    let mut ignored_seasons = series.ignored_seasons.clone();
    for (season, _) in db.series_season_counts(&series.name).unwrap_or_default() {
        if !Config::season_allowed_for_scan(&series.seasons, season)
            && !ignored_seasons.contains(&season)
        {
            ignored_seasons.push(season);
        }
    }
    let episodes = match db.episodes_for_series(&series.name, &ignored_seasons) {
        Ok(items) => items,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    let gaps =
        db
            .archive_gaps()
            .unwrap_or_default()
            .into_iter()
            .filter(|(series_name, season, _)| {
                series_name == &series.name
                    && !ignored_seasons.contains(season)
                    && Config::season_allowed_for_scan(&series.seasons, *season)
            })
            .map(|(_, season, episode)| serde_json::json!({"season":season,"episode":episode}))
            .collect::<Vec<_>>();
    let metadata =
        db
            .series_season_counts(&series.name)
            .unwrap_or_default()
            .into_iter()
            .map(|(season, count)| serde_json::json!({"season": season, "count": count}))
            .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"series":series,"episodes":episodes,"gaps":gaps,"metadata":metadata}),
        ),
    )
}
async fn toggle_season(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(input): Json<SeasonToggleInput>,
) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let Some(series) = cfg
        .series
        .iter_mut()
        .find(|series| series.name == name || series.aliases.iter().any(|alias| alias == &name))
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    if input.enabled {
        series
            .ignored_seasons
            .retain(|season| *season != input.season);
    } else if !series.ignored_seasons.contains(&input.season) {
        series.ignored_seasons.push(input.season);
        series.ignored_seasons.sort_unstable();
    }
    let ignored_seasons = series.ignored_seasons.clone();
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"ignored_seasons":ignored_seasons})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn set_series_path(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(input): Json<SeriesPathInput>,
) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let Some(series) = cfg
        .series
        .iter_mut()
        .find(|series| series.name == name || series.aliases.iter().any(|alias| alias == &name))
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    if let Some(path) = input.archive_path {
        series.archive_path = path.trim().to_string();
    }
    if let Some(timeframe) = input.timeframe {
        series.timeframe = timeframe.max(0);
    }
    let archive_path = series.archive_path.clone();
    let timeframe = series.timeframe;
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"archive_path":archive_path,"timeframe":timeframe})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn series_search_missing(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    let gaps = s
        .db
        .lock()
        .unwrap()
        .unarchived_episodes_for_series(&series.name, &series.ignored_seasons)
        .unwrap_or_default()
        .into_iter()
        // La ricerca manuale deve rispettare anche l'intervallo delle stagioni
        // monitorate (es. `8+`), oltre alle stagioni esplicitamente ignorate.
        // Prima questo filtro era applicato dal ciclo automatico ma non dal
        // pulsante UI "Cerca mancanti".
        .filter(|(season, _)| {
            !series.ignored_seasons.contains(season)
                && Config::season_allowed_for_scan(&series.seasons, *season)
        })
        // Come il flusso legacy, cerca una porzione significativa della serie
        // senza trasformare una singola azione UI in centinaia di query.
        .take(15)
        .collect::<Vec<_>>();
    // Le ricerche locali (feed/archivio) restano puntuali; per le sorgenti
    // remote usiamo invece una sola query della serie, poi distribuiamo le
    // release alle puntate corrispondenti. Evita 15 ricerche web/indexer in
    // sequenza per un singolo clic.
    let live_releases = if gaps.is_empty() {
        Vec::new()
    } else {
        s.engine.search_query_manual(&cfg, &series.name).await
    };
    let mut results = Vec::new();
    for (season, episode) in &gaps {
        let mut episode_results = stored_series_episode_sources(&s, &series, *season, *episode);
        for release in live_releases
            .iter()
            .filter(|release| release_matches_series_episode(release, &series, *season, *episode))
        {
            episode_results
                .push(serde_json::json!({"release":release,"origin":"Indexer / web"}));
        }
        for mut result in finalize_episode_search_results(episode_results, &cfg, &series) {
            result["season"] = serde_json::json!(season);
            result["episode"] = serde_json::json!(episode);
            results.push(result);
        }
    }
    let mut seen = HashSet::new();
    results.retain(|result| {
        result
            .get("release")
            .and_then(|release| release.get("magnet"))
            .and_then(serde_json::Value::as_str)
            .and_then(crate::utils::magnet_hash)
            .is_some_and(|hash| {
                let origin = result
                    .get("origin")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                seen.insert(format!("{hash}:{origin}"))
            })
    });
    results.sort_by_key(|result| {
        std::cmp::Reverse(
            result
                .get("release")
                .and_then(|release| serde_json::from_value::<Release>(release.clone()).ok())
                .map(|release| release.quality.score_with_settings(&cfg.settings))
                .unwrap_or_default(),
        )
    });
    {
        let db = s.db.lock().unwrap();
        for (season, episode) in &gaps {
            let _ = db.mark_gap_searched(&series.name, *season, *episode);
        }
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({
                "ok": true,
                "series": series.name,
                "searched": gaps.len(),
                "episodes": gaps.into_iter().map(|(season, episode)| {
                    serde_json::json!({"season": season, "episode": episode})
                }).collect::<Vec<_>>(),
                "results": results,
            }),
        ),
    )
}
async fn series_metadata_refresh(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    let Some(api_key) = cfg.tmdb_api_key.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
        );
    };
    let tmdb = TmdbClient::with_language(Some(api_key), cfg.tmdb_language());
    let resolved = if series.tmdb_id.trim().is_empty() {
        match tmdb.resolve_series_id(&series.name).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"ok":false,"error":"series not found on TMDB"})),
                )
            }
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                )
            }
        }
    } else {
        series.tmdb_id.clone()
    };
    let counts = match tmdb.season_counts(&resolved).await {
        Ok(counts) => counts,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    let mut values = counts.into_iter().collect::<Vec<(i64, i64)>>();
    values.sort_unstable_by_key(|(season, _)| *season);
    // Le date sono recuperate solo nel refresh metadati e poi restano nella DB:
    // l'apertura del dettaglio serie è quindi interamente locale.
    let mut air_dates = Vec::new();
    let mut air_date_errors = 0usize;
    for (season, _) in values.iter().copied().filter(|(season, _)| *season > 0) {
        match tmdb.season_episodes(&resolved, season).await {
            Ok(episodes) => air_dates.extend(episodes.into_iter().filter_map(|episode| {
                let episode_season = episode.season_number.unwrap_or(season);
                let episode_number = episode.episode_number?;
                (episode_season == season).then_some((
                    season,
                    episode_number,
                    episode.air_date.unwrap_or_default(),
                ))
            })),
            Err(error) => {
                air_date_errors += 1;
                tracing::warn!(series=%series.name, season, %error, "TMDB season air-date refresh failed");
            }
        }
    }
    if let Err(error) =
        s.db.lock()
            .unwrap()
            .save_series_metadata(&series.name, &values)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    // Stato TMDB salvato per l'indicatore "terminata" nell'elenco serie.
    if let Ok(Some(info)) = tmdb.series_info(&series.name, Some(resolved.as_str())).await {
        let status = info
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let last_air_date = info
            .get("last_air_date")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if let Err(error) = s
            .db
            .lock()
            .unwrap()
            .save_series_status(&series.name, status, last_air_date)
        {
            tracing::warn!(series=%series.name, %error, "series status save failed");
        }
    }
    let mut stored = false;
    if series.tmdb_id.trim().is_empty() {
        if let Some(item) = cfg.series.iter_mut().find(|item| {
            item.name == series.name || item.aliases.iter().any(|alias| alias == &name)
        }) {
            item.tmdb_id = resolved.clone();
            stored = true;
        }
        if stored {
            let _ = Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies);
        }
    }
    if let Err(error) = s
        .db
        .lock()
        .unwrap()
        .save_episode_air_dates(&series.name, &air_dates)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "series": series.name,
                "tmdb_id": resolved,
                "tmdb_id_stored": stored,
                "air_dates_updated": air_dates.len(),
                "air_date_errors": air_date_errors,
            "seasons": values.iter().map(|(season, count)| serde_json::json!({"season": season, "episodes": count})).collect::<Vec<_>>(),
        })),
    )
}
async fn series_info(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    let tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    let tmdb_id = (!series.tmdb_id.trim().is_empty()).then(|| series.tmdb_id.as_str());
    match tmdb.series_info(&series.name, tmdb_id).await {
        Ok(Some(value)) => {
            let poster = value
                .get("poster_path")
                .and_then(serde_json::Value::as_str)
                .map(|path| format!("https://image.tmdb.org/t/p/w300{path}"));
            let network = value
                .get("networks")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let year = value
                .get("first_air_date")
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().take(4).collect::<String>());
            // Cast: preferisci TVDB (link alla pagina persona TVDB) quando la
            // serie ha un id TVDB, altrimenti fallback a TMDB.
            let tvdb =
                crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
            let mut tvdb_id = series.tvdb_id.trim().parse::<i64>().ok();
            if tvdb_id.is_none() {
                if let Ok(results) = tvdb.search_series(&series.name).await {
                    tvdb_id = results
                        .first()
                        .and_then(|item| item.get("tvdb_id"))
                        .and_then(|value| {
                            value
                                .as_i64()
                                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                        });
                }
            }
            let mut cast: Vec<serde_json::Value> = Vec::new();
            if let Some(tvdb_id) = tvdb_id {
                if let Ok(characters) = tvdb.series_characters(tvdb_id).await {
                    for character in characters {
                        if let (Some(name), Some(person)) = (
                            character.get("name").and_then(serde_json::Value::as_str),
                            character.get("tvdb_id").and_then(serde_json::Value::as_i64),
                        ) {
                            cast.push(serde_json::json!({
                                "name": name,
                                "url": format!("https://thetvdb.com/dereferrer/people/{person}"),
                            }));
                        }
                    }
                }
            }
            if cast.is_empty() {
                let person_cast = match value.get("id").and_then(serde_json::Value::as_i64) {
                    Some(id) => tmdb.series_cast(&id.to_string()).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                for (person, name) in person_cast {
                    cast.push(serde_json::json!({
                        "name": name,
                        "url": format!("https://www.themoviedb.org/person/{person}"),
                    }));
                }
            }
            let genres = value
                .get("genres")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let country = value
                .get("origin_country")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.first())
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    value
                        .get("production_countries")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|item| item.get("iso_3166_1"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                });
            let last_air_date = value
                .get("last_air_date")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let tvdb_url = match tvdb_id {
                Some(id) => format!("https://thetvdb.com/dereferrer/series/{id}"),
                None => format!(
                    "https://thetvdb.com/search?query={}",
                    url::form_urlencoded::byte_serialize(series.name.as_bytes())
                        .collect::<String>()
                ),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"info":{
                    "name": value.get("name"),
                    "overview": value.get("overview"),
                    "poster": poster,
                    "network": network,
                    "year": year,
                    "seasons": value.get("number_of_seasons"),
                    "episodes_total": value.get("number_of_episodes"),
                    "vote": value.get("vote_average"),
                    "status": value.get("status"),
                    "country": country,
                    "last_air_date": last_air_date,
                    "last_episode": value.get("last_episode_to_air"),
                    "next_episode": value.get("next_episode_to_air"),
                    "tmdb_id": value.get("id"),
                    "tvdb_id": series.tvdb_id.clone(),
                    "tvdb_url": tvdb_url,
                    "cast": cast,
                    "genres": genres,
                }})),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"info":null})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
#[derive(serde::Deserialize, Default)]
pub struct RenameInput {
    /// Rinomina anche i file già conformi al formato (default: no).
    #[serde(default)]
    pub force: bool,
    /// Ripristina solo la sorgente andata persa nei nomi (usa il titolo DB).
    #[serde(default)]
    pub source_only: bool,
}

async fn series_rename_preview(
    State(s): State<AppState>,
    Path(name): Path<String>,
    input: Option<Json<RenameInput>>,
) -> impl IntoResponse {
    series_rename_apply(
        &s,
        &name,
        false,
        input.as_ref().is_some_and(|Json(input)| input.force),
        input.as_ref().is_some_and(|Json(input)| input.source_only),
    )
    .await
}
async fn series_rename_execute(
    State(s): State<AppState>,
    Path(name): Path<String>,
    input: Option<Json<RenameInput>>,
) -> impl IntoResponse {
    series_rename_apply(
        &s,
        &name,
        true,
        input.as_ref().is_some_and(|Json(input)| input.force),
        input.as_ref().is_some_and(|Json(input)| input.source_only),
    )
    .await
}

async fn rename_progress_view(State(s): State<AppState>) -> impl IntoResponse {
    let progress = s.rename_progress.lock().unwrap().clone();
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"progress":progress})),
    )
}

/// Starts a rename over every enabled series in the background and returns
/// immediately; progress is polled via `/api/rename-progress`. Per-series rename
/// stays synchronous for backward compatibility.
async fn rename_all(
    State(s): State<AppState>,
    input: Option<Json<RenameInput>>,
) -> impl IntoResponse {
    let force = input.as_ref().is_some_and(|Json(input)| input.force);
    let source_only = input
        .as_ref()
        .is_some_and(|Json(input)| input.source_only);
    let cfg = latest_config(&s);
    if !cfg.rename_episodes {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"rename is disabled"})),
        );
    }
    {
        let mut progress = s.rename_progress.lock().unwrap();
        if progress.running {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"a rename is already running"})),
            );
        }
        *progress = RenameProgress {
            running: true,
            current: 0,
            total: 0,
            series: String::new(),
            message: "starting".into(),
            errors: 0,
        };
    }
    let names = cfg
        .series
        .iter()
        .filter(|series| series.enabled)
        .map(|series| series.name.clone())
        .collect::<Vec<_>>();
    let total = names.len();
    {
        let mut progress = s.rename_progress.lock().unwrap();
        progress.total = total;
        progress.message = "running".into();
    }
    // Run on a dedicated thread with its own runtime: the archive is on NFS and
    // blocking filesystem I/O must not stall the daemon's async runtime.
    let state = s.clone();
    std::thread::spawn(move || {
        let mut errors = 0usize;
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(async {
                for (index, name) in names.iter().enumerate() {
                    {
                        let mut progress = state.rename_progress.lock().unwrap();
                        progress.current = index;
                        progress.series = name.clone();
                    }
                    let (status, _) =
                        series_rename_apply(&state, name, true, force, source_only).await;
                    if status != StatusCode::OK {
                        errors += 1;
                    }
                }
            });
        }
        let mut progress = state.rename_progress.lock().unwrap();
        progress.running = false;
        progress.current = total;
        progress.series = String::new();
        progress.errors = errors;
        progress.message = if errors == 0 {
            "completed".into()
        } else {
            format!("completed with {errors} error(s)")
        };
        tracing::info!(total, errors, "background rename-all finished");
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"ok":true,"total":total})),
    )
}

async fn series_rename_apply(
    s: &AppState,
    name: &str,
    execute: bool,
    force: bool,
    source_only: bool,
) -> (StatusCode, Json<serde_json::Value>) {
    let cfg = latest_config(s);
    let Some(series) = find_series(&cfg, name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    if !cfg.rename_episodes {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"rename is disabled"})),
        );
    }
    // Never rename a series whose archive is currently being written by a
    // torrent import: the repair would move files that are still being copied.
    if archive_import_busy().lock().unwrap().contains(&series.name) {
        tracing::info!(series = %series.name, "rename skipped: archive import in progress");
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok":true,
                "skipped":true,
                "reason":"archive import in progress",
            })),
        );
    }
    let episodes =
        s.db.lock()
            .unwrap()
            .episodes_for_series(&series.name, &series.ignored_seasons)
            .unwrap_or_default();
    let tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    let mut items = Vec::new();
    let mut already_ok = Vec::new();
    let mut considered = 0usize;
    let mut with_path = 0usize;
    for episode in &episodes {
        considered += 1;
        let Some(archive_path) = episode
            .archive_path
            .clone()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let configured_path = FsPath::new(archive_path.as_str());
        if !configured_path.exists() {
            continue;
        }
        // Older imports may store the series directory instead of the actual
        // episode file. Resolve it before checking the naming convention;
        // otherwise a correctly named S06E01 is treated as a file named after
        // the directory and gets an unnecessary rename.
        let path = if configured_path.is_dir() {
            let matches = postprocess::video_files(configured_path)
                .unwrap_or_default()
                .into_iter()
                .filter(|file| {
                    file.file_name()
                        .and_then(|value| value.to_str())
                        .map(|name| {
                            let marker = format!("s{:02}e{:02}", episode.season, episode.episode);
                            name.to_ascii_lowercase().contains(&marker)
                        })
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                continue;
            }
            matches[0].clone()
        } else {
            configured_path.to_path_buf()
        };
        with_path += 1;
        // La sorgente (che MediaInfo non fornisce) va recuperata dal titolo
        // ORIGINALE della release, non dal nome file attuale: se un rename
        // precedente l'ha persa, così viene ripristinata invece di restare
        // `unknown` per sempre. I campi assenti nel titolo originale ricadono
        // sul nome file.
        let fallback_quality = crate::parser::merge_quality(
            crate::parser::parse_quality(&episode.title),
            crate::parser::parse_quality(
                path.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default(),
            ),
        );
        let release = Release { torrent_url: None,
            title: episode.title.clone(),
            magnet: String::new(),
            source: "archive".into(),
            quality: fallback_quality,
            kind: "series".into(),
            series: Some(series.name.clone()),
            season: Some(episode.season),
            episode: Some(episode.episode),
            is_pack: false,
            episode_range: vec![episode.episode],
            year: None,
            discovered_at: chrono::Utc::now(),
        };
        // Modalità "ripristina sorgente": rinomina solo i file il cui nome ha
        // perso la sorgente che invece il titolo originale (DB) conosce.
        let db_source = crate::parser::parse_quality(&episode.title).source;
        let file_source = crate::parser::parse_quality(
            path.file_name().and_then(|value| value.to_str()).unwrap_or_default(),
        )
        .source;
        let needs_source = db_source != "unknown"
            && !db_source.trim().is_empty()
            && file_source == "unknown";
        if source_only {
            if !needs_source {
                already_ok.push(archive_path);
                continue;
            }
        } else if !force && postprocess::episode_name_conforms(&path, &release, &cfg) {
            // Il video è già corretto: in esecuzione ripulisci comunque i
            // sidecar spuri/doppi (operazione economica, senza TMDB/MediaInfo).
            if execute {
                postprocess::apply_sidecars(&path, &path, &cfg);
                // Riallinea score/dimensione al file reale: dopo un upgrade il
                // DB può essere rimasto alla qualità precedente (es. titolo
                // 1080p con file 2160p), così l'episodio non viene ri-scaricato.
                let _ = s.db.lock().unwrap().refresh_episode_file_stats(
                    &series.name,
                    episode.season,
                    episode.episode,
                    &path.display().to_string(),
                );
            }
            already_ok.push(archive_path);
            continue;
        }
        let result = if execute {
            postprocess::rename_episode(&path, &release, &cfg, &tmdb).await
        } else {
            postprocess::preview_episode_rename(&path, &release, &cfg, &tmdb).await
        };
        match result {
            Ok(Some(target)) => {
                if postprocess::same_path(&path, &target) {
                    // No-op: the file already carries the target name (the cheap
                    // conform check can miss it). Never log or count this as a
                    // rename, but still realign the stored score/size to the
                    // file so an upgraded episode is not marked as the old one.
                    if execute {
                        let _ = s.db.lock().unwrap().refresh_episode_file_stats(
                            &series.name,
                            episode.season,
                            episode.episode,
                            &target.display().to_string(),
                        );
                    }
                    already_ok.push(archive_path);
                    continue;
                }
                tracing::info!(
                    series = %series.name,
                    season = episode.season,
                    episode = episode.episode,
                    from = %path.display(),
                    to = %target.display(),
                    execute,
                    "rename file"
                );
                if execute {
                    // Il file rinominato ha un nuovo percorso: aggiorna il DB,
                    // altrimenti la prossima anteprima non trova più il file.
                    if let Err(error) = s.db.lock().unwrap().set_episode_archive_path(
                        &series.name,
                        episode.season,
                        episode.episode,
                        &target.display().to_string(),
                    ) {
                        tracing::warn!(%error, "rename: failed to update DB path");
                    }
                }
                items.push(serde_json::json!({
                    "season": episode.season,
                    "episode": episode.episode,
                    "from": path.display().to_string(),
                    "to": target.display().to_string(),
                    "executed": execute,
                }))
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    series = %series.name,
                    season = episode.season,
                    episode = episode.episode,
                    %error,
                    "rename file failed"
                );
                items.push(serde_json::json!({
                    "season": episode.season,
                    "episode": episode.episode,
                    "from": archive_path,
                    "error": error.to_string(),
                }))
            }
        }
    }
    // The database may not know about files copied manually or by an older
    // post-processing run. Scan the series archive as well, so those files
    // are renamed and, when an existing episode is better, moved to trash.
    let tracked_paths = episodes
        .iter()
        .filter_map(|episode| episode.archive_path.as_deref())
        .map(FsPath::new)
        .filter(|path| path.exists())
        .map(|path| path.to_path_buf())
        .collect::<HashSet<_>>();
    // In modalità ripristino sorgente non scansioniamo i file non tracciati:
    // la sorgente si recupera solo dal titolo originale nel DB.
    let archive_scan = if source_only {
        Ok(Vec::<std::path::PathBuf>::new())
    } else {
        postprocess::video_files(FsPath::new(&series.archive_path))
    };
    if let Ok(files) = archive_scan {
        let pattern = regex::Regex::new(
            r"(?i)^(?P<name>.+?)[ ._-]+(?:s(?P<s>\d{1,2})e|(?P<ns>\d{1,2})x)(?P<e>\d{1,4})",
        )
        .expect("series archive episode pattern");
        for file in files {
            if tracked_paths.contains(&file) {
                continue;
            }
            let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(captures) = pattern.captures(name) else {
                continue;
            };
            let Some(file_series) = captures.name("name").map(|value| value.as_str()) else {
                continue;
            };
            if !crate::parser::series_names_match(&series.name, file_series) {
                continue;
            }
            let Some(season) = captures
                .name("s")
                .or_else(|| captures.name("ns"))
                .and_then(|value| value.as_str().parse::<i64>().ok())
            else {
                continue;
            };
            let Some(episode_number) = captures
                .name("e")
                .and_then(|value| value.as_str().parse::<i64>().ok())
            else {
                continue;
            };
            let known = episodes
                .iter()
                .find(|episode| episode.season == season && episode.episode == episode_number);
            let release = Release { torrent_url: None,
                title: known
                    .map(|episode| episode.title.clone())
                    .unwrap_or_else(|| format!("Episodio {episode_number}")),
                magnet: String::new(),
                source: "archive-scan".into(),
                quality: crate::parser::parse_quality(name),
                kind: "series".into(),
                series: Some(series.name.clone()),
                season: Some(season),
                episode: Some(episode_number),
                is_pack: false,
                episode_range: vec![episode_number],
                year: None,
                discovered_at: chrono::Utc::now(),
            };
            if !force && postprocess::episode_name_conforms(&file, &release, &cfg) {
                // Già conforme al formato ma non tracciato: collega comunque il
                // file al DB, così il prossimo ciclo sa che l'episodio c'è.
                if execute {
                    link_archive_file(&s.db, &series.name, season, episode_number, &file);
                }
                continue;
            }
            let score = release.quality.score_with_settings(&cfg.settings);
            if execute {
                match crate::cleaner::discard_if_inferior(
                    &cfg,
                    &series.name,
                    season,
                    episode_number,
                    score,
                    &file,
                    FsPath::new(&series.archive_path),
                ) {
                    Ok(true) => {
                        let _ = postprocess::discard_sidecars(&file, &cfg);
                        items.push(serde_json::json!({
                            "season": season,
                            "episode": episode_number,
                            "from": file.display().to_string(),
                            "discarded": true,
                        }));
                        continue;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(file=%file.display(), %error, "archive duplicate check failed");
                        continue;
                    }
                }
            }
            if !execute {
                if let Ok(Some(target)) =
                    postprocess::preview_episode_rename(&file, &release, &cfg, &tmdb).await
                {
                    items.push(serde_json::json!({
                        "season": season,
                        "episode": episode_number,
                        "from": file.display().to_string(),
                        "to": target.display().to_string(),
                        "executed": false,
                    }));
                }
                continue;
            }
            match postprocess::rename_episode(&file, &release, &cfg, &tmdb).await {
                Ok(Some(target)) => {
                    let _ = crate::cleaner::cleanup_old_episode(
                        &cfg,
                        &series.name,
                        season,
                        episode_number,
                        score,
                        &target,
                        target.parent().unwrap_or(FsPath::new(&series.archive_path)),
                    );
                    link_archive_file(&s.db, &series.name, season, episode_number, &target);
                    items.push(serde_json::json!({
                        "season": season,
                        "episode": episode_number,
                        "from": file.display().to_string(),
                        "to": target.display().to_string(),
                        "executed": true,
                    }));
                }
                Ok(None) => {
                    // Nessuna rinomina necessaria: collega il file esistente.
                    link_archive_file(&s.db, &series.name, season, episode_number, &file);
                }
                Err(error) => {
                    // Never surface API keys embedded in URLs (e.g. TMDB) in logs.
                    let error = crate::utils::redact_url_secrets(&error.to_string());
                    tracing::warn!(file=%file.display(), %error, "archive file rename failed")
                }
            }
        }
    }
    // Pulizia dei duplicati chiaramente inferiori (risoluzione più bassa) nella
    // cartella della serie. Copre anche le librerie ereditate dove, accanto al
    // 1080p, è rimasto il vecchio 480p/720p. Solo in esecuzione.
    let mut duplicates_removed = 0usize;
    if execute && cfg.cleanup_upgrades && !source_only {
        let protected = protected_torrent_paths(&s.torrents);
        match crate::cleaner::cleanup_inferior_duplicates_in_dir(
            &cfg,
            &series.name,
            FsPath::new(&series.archive_path),
            &protected,
        ) {
            Ok(removed) => duplicates_removed = removed,
            Err(error) => tracing::warn!(series=%series.name, %error, "duplicate cleanup failed"),
        }
    }
    let already_ok_count = already_ok.len();
    let discarded_count = items
        .iter()
        .filter(|item| item.get("discarded").and_then(serde_json::Value::as_bool) == Some(true))
        .count();
    let error_count = items
        .iter()
        .filter(|item| item.get("error").is_some())
        .count();
    let renamed_count = items.len().saturating_sub(discarded_count + error_count);
    let note = match (force, source_only) {
        (true, _) => " [force]",
        (_, true) => " [source only]",
        _ => "",
    };
    if execute {
        // Un'esecuzione che non cambia nulla (tutto già corretto) non merita una
        // riga INFO ad ogni ciclo: la si tiene solo a livello debug.
        let changed = renamed_count > 0
            || discarded_count > 0
            || duplicates_removed > 0
            || error_count > 0;
        if changed {
            tracing::info!(
                "🗂 rename '{}': {} renamed, {} already correct, {} discarded, {} duplicates removed, {} errors — {}/{} files present{}",
                series.name,
                renamed_count,
                already_ok_count,
                discarded_count,
                duplicates_removed,
                error_count,
                with_path,
                considered,
                note
            );
        } else {
            tracing::debug!(
                "🗂 rename '{}': nothing to change ({} already correct, {}/{} files present){}",
                series.name,
                already_ok_count,
                with_path,
                considered,
                note
            );
        }
    } else if items.is_empty() {
        tracing::info!(
            "🗂 rename preview '{}': nothing to rename ({} already correct, {}/{} files present){}",
            series.name,
            already_ok_count,
            with_path,
            considered,
            note
        );
    } else {
        tracing::info!(
            "🗂 rename preview '{}': {} file(s) would change, {} already correct — {}/{} files present{}",
            series.name,
            items.len(),
            already_ok_count,
            with_path,
            considered,
            note
        );
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"series":series.name,"execute":execute,"force":force,"episodes":considered,"with_path":with_path,"items":items,"already_ok":already_ok,"already_ok_count":already_ok_count,"renamed_count":renamed_count,"discarded_count":discarded_count,"duplicates_removed":duplicates_removed,"error_count":error_count}),
        ),
    )
}
async fn movie_detail(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(movie) = cfg.movies.iter().find(|movie| movie.id == id).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    let history =
        s.db.lock()
            .unwrap()
            .downloaded_movies(200)
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item.name == movie.name)
            .collect::<Vec<_>>();
    let query = if movie.year.trim().is_empty() {
        movie.name.clone()
    } else {
        format!("{} {}", movie.name, movie.year)
    };
    let matches = s
        .archive
        .lock()
        .unwrap()
        .search(&query)
        .unwrap_or_default()
        .into_iter()
        .take(20)
        .map(|(title, magnet, source)| serde_json::json!({"title":title,"magnet":magnet,"source":source}))
        .collect::<Vec<_>>();
    // I dati scelti nella modale sono persistenti e non dipendono da una nuova
    // ricerca ogni volta che si apre il dettaglio. Per le vecchie voci senza
    // cache manteniamo il fallback TMDB esistente.
    let stored_metadata = serde_json::json!({
        "id": movie.tmdb_id.parse::<i64>().ok(),
        "title": movie.name,
        "original_title": movie.original_title,
        "overview": movie.overview,
        "poster_path": movie.poster_path,
        "release_date": movie.year,
        "source": if movie.tvdb_id.is_empty() { "tmdb" } else { "tvdb" },
    });
    let has_stored_metadata = !movie.tmdb_id.is_empty()
        || !movie.tvdb_id.is_empty()
        || !movie.overview.is_empty()
        || !movie.poster_path.is_empty();
    let (metadata, cast) = match cfg.tmdb_api_key.clone() {
        Some(key) => {
            let tmdb = TmdbClient::with_language(Some(key), cfg.tmdb_language());
            // Un ID TMDB inserito a mano ha priorità: recupera i dettagli
            // aggiornati da TMDB invece di usare la cache o la ricerca per nome.
            let metadata = if let Ok(tmdb_id) = movie.tmdb_id.trim().parse::<i64>() {
                match tmdb.movie_details(&tmdb_id.to_string()).await {
                    Ok(details) => serde_json::json!({
                        "id": details.id,
                        "title": details.title,
                        "original_title": details.original_title,
                        "overview": details.overview,
                        "poster_path": details.poster_path,
                        "release_date": details.release_date,
                        "source": "tmdb",
                    }),
                    Err(_) => stored_metadata,
                }
            } else if has_stored_metadata {
                stored_metadata
            } else {
                tmdb.search_movie(&movie.name, movie.year.parse().ok())
                    .await
                    .ok()
                    .flatten()
                    .and_then(|item| serde_json::to_value(item).ok())
                    .unwrap_or_default()
            };
            let tmdb_id = movie
                .tmdb_id
                .parse::<i64>()
                .ok()
                .or_else(|| metadata.get("id").and_then(serde_json::Value::as_i64));
            let cast = match tmdb_id {
                Some(tmdb_id) => tmdb
                    .movie_credits(&tmdb_id.to_string())
                    .await
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            (metadata, cast)
        }
        None => (stored_metadata, Vec::new()),
    };
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"movie":movie,"history":history,"metadata":metadata,"cast":cast,"matches":matches}),
        ),
    )
}

async fn movie_metadata_search(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<MovieMetadataSearchInput>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    if !cfg.movies.iter().any(|movie| movie.id == id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    }
    let query = input.query.trim();
    if query.is_empty() || query.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"query must contain 1-256 characters"})),
        );
    }
    let source = input.source.trim().to_ascii_lowercase();
    let result = if source == "tvdb" {
        let tvdb = crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
        if !tvdb.configured() {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"TVDB API key is not configured"})),
            );
        }
        tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, tvdb.search_movies(query)).await
    } else if source == "tmdb" {
        let Some(key) = cfg.tmdb_api_key.clone() else {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
            );
        };
        let tmdb = TmdbClient::with_language(Some(key), cfg.tmdb_language());
        tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, tmdb.search_movies(query)).await
            .map(|result| result.map(|items| items.into_iter().filter_map(|item| serde_json::to_value(item).ok()).collect()))
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"unsupported metadata source"})),
        );
    };
    match result {
        Ok(Ok(items)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"source":source,"items":items})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"ok":false,"error":"metadata provider timed out"})),
        ),
    }
}

async fn apply_movie_metadata(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<MovieMetadataApplyInput>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(current) = cfg.movies.iter().find(|movie| movie.id == id).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    let source = input.source.trim().to_ascii_lowercase();
    let external_id = input.id.trim();
    if external_id.is_empty() || external_id.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid metadata id"})),
        );
    }
    let details = if source == "tmdb" {
        let Some(key) = cfg.tmdb_api_key.clone() else {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
            );
        };
        let tmdb = TmdbClient::with_language(Some(key), cfg.tmdb_language());
        match tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, tmdb.movie_details(external_id)).await {
            Ok(Ok(item)) => serde_json::json!({
                "title": item.title,
                "original_title": item.original_title,
                "overview": item.overview,
                "poster_path": item.poster_path,
                "release_date": item.release_date,
            }),
            Ok(Err(error)) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"ok":false,"error":error.to_string()}))),
            Err(_) => return (StatusCode::GATEWAY_TIMEOUT, Json(serde_json::json!({"ok":false,"error":"TMDB timed out"}))),
        }
    } else if source == "tvdb" {
        let tvdb = crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
        if !tvdb.configured() {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"TVDB API key is not configured"})),
            );
        }
        match tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, tvdb.movie_details(external_id)).await {
            Ok(Ok(item)) => serde_json::json!({
                "title": item.get("name"),
                "original_title": item.get("originalName"),
                "overview": item.get("overview"),
                "poster_path": item.get("image").or_else(|| item.get("image_url")),
                "release_date": item.get("year"),
            }),
            Ok(Err(error)) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"ok":false,"error":error.to_string()}))),
            Err(_) => return (StatusCode::GATEWAY_TIMEOUT, Json(serde_json::json!({"ok":false,"error":"TVDB timed out"}))),
        }
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"unsupported metadata source"})),
        );
    };
    let title = details
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&current.name)
        .to_string();
    let date = details
        .get("release_date")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let metadata_year = date
        .get(..4)
        .filter(|value| value.chars().all(|value| value.is_ascii_digit()))
        .unwrap_or_default();
    let year = if metadata_year.is_empty() {
        current.year.clone()
    } else {
        metadata_year.to_string()
    };
    let mut updated = latest_config(&s);
    let Some(movie) = updated.movies.iter_mut().find(|movie| movie.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    movie.name = title.clone();
    movie.year = year.clone();
    movie.original_title = details
        .get("original_title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    movie.overview = details
        .get("overview")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    movie.poster_path = details
        .get("poster_path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if source == "tmdb" {
        movie.tmdb_id = external_id.to_string();
        movie.tvdb_id.clear();
    } else {
        movie.tvdb_id = external_id.to_string();
        movie.tmdb_id.clear();
    }
    if let Err(error) = Config::save_library(&s.cfg.data_dir, &updated.series, &updated.movies) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    if let Err(error) = s.db.lock().unwrap().rename_movie_identity(
        &current.name,
        &current.year,
        &title,
        &year,
    ) {
        tracing::error!(%error, movie_id=id, "movie metadata saved but download identity update failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":"metadata saved, but could not preserve download history"})),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"movie": updated.movies.into_iter().find(|movie| movie.id == id)})),
    )
}
async fn movie_search(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(movie) = cfg.movies.iter().find(|movie| movie.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    let query = if movie.year.trim().is_empty() {
        movie.name.clone()
    } else {
        format!("{} {}", movie.name, movie.year)
    };
    let mut results = s.engine.search_query(&cfg, &query).await;
    for (title, magnet, source) in s.archive.lock().unwrap().search(&query).unwrap_or_default() {
        if let Some(release) =
            crate::parser::parse_release(&title, &magnet, &format!("archive:{source}"))
        {
            results.push(release);
        }
    }
    let mut seen = HashSet::new();
    results.retain(|release| {
        // Il pulsante “Accoda” usa gli stessi requisiti del film monitorato.
        // Senza questo filtro la ricerca poteva mostrare (ad es.) una release
        // solo ENG o sotto la qualità minima, per poi rifiutarla al clic.
        release.kind == "movie"
            && cfg
                .find_movie_match(&release.title, release.year)
                .is_some_and(|matched| {
                    matched.id == movie.id && Config::movie_release_allowed(movie, &release.quality)
                })
            && crate::utils::magnet_hash(&release.magnet).is_some_and(|hash| seen.insert(hash))
    });
    results.sort_by_key(|release| {
        std::cmp::Reverse(release.quality.score_with_settings(&cfg.settings))
    });
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"query":query,"results":results})),
    )
}
async fn update_movie(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<MovieUpdateInput>,
) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let Some(movie) = cfg.movies.iter_mut().find(|movie| movie.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    if input.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"movie name is required"})),
        );
    }
    movie.name = input.name.trim().to_string();
    movie.year = input.year;
    movie.quality = input.quality;
    movie.language = input.language;
    movie.subtitle = input.subtitle;
    movie.exclude = input.exclude;
    // Never persist the dirty "-" / "[]" placeholders the editor may send.
    movie.language_requirements =
        crate::config::sanitize_language_requirements(&input.language_requirements);
    movie.subtitle_requirements =
        crate::config::sanitize_subtitle_requirements(&input.subtitle_requirements);
    movie.tmdb_id = input.tmdb_id.trim().to_string();
    movie.tvdb_id = input.tvdb_id.trim().to_string();
    if let Some(enabled) = input.enabled {
        movie.enabled = enabled;
    }
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn delete_movie(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let before = cfg.movies.len();
    cfg.movies.retain(|movie| movie.id != id);
    if before == cfg.movies.len() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    }
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn redownload_movie(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(movie) = cfg.movies.iter().find(|movie| movie.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"movie not found"})),
        );
    };
    let name = movie.name.clone();
    match s.db.lock().unwrap().reset_movie_by_name(&name) {
        Ok(reset) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"reset":reset,"message":"film rimesso in coda al prossimo ciclo"}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
#[derive(serde::Deserialize)]
pub struct SourceQuery {
    pub q: Option<String>,
}
async fn sources_health(
    State(s): State<AppState>,
    Query(query): Query<SourceQuery>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    // Some sources are slow (Prowlarr can take 30s+) and FlareSolverr
    // challenges can take longer still, so keep this generous. Intermittent
    // failures on a single source are expected and not a configuration error.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .unwrap_or_default();
    let term = query.q.filter(|value| !value.trim().is_empty());
    let mut set = tokio::task::JoinSet::new();
    match term.clone() {
        Some(term) => {
            let feed_max_pages = cfg.feed_max_pages();
            let max_age_days = cfg.max_release_age_days;
            let old_ratio = cfg.stop_on_old_page_ratio();
            for feed in cfg.feed_urls.clone() {
                let client = client.clone();
                let flaresolverr = cfg.flaresolverr_url.clone();
                let term = term.clone();
                set.spawn(async move {
                    let _ = &term;
                    let (ok, results, error) =
                        match crate::rss::fetch_feed(
                            &client,
                            &feed,
                            flaresolverr.as_deref(),
                            feed_max_pages,
                            max_age_days,
                            old_ratio,
                        )
                        .await
                        {
                            Ok(items) => (true, items.len() as i64, None),
                            Err(error) => (false, 0, Some(error.to_string())),
                        };
                    serde_json::json!({"kind":"feed","name":feed,"ok":ok,"results":results,"error":error})
                });
            }
            for indexer in cfg
                .indexers
                .iter()
                .filter(|indexer| indexer.enabled)
                .cloned()
            {
                let client = client.clone();
                let term = term.clone();
                set.spawn(async move {
                    let (ok, results, error) =
                        match crate::rss::fetch_torznab(&client, &indexer, &term).await {
                            Ok(items) => (true, items.len() as i64, None),
                            Err(error) => (false, 0, Some(error.to_string())),
                        };
                    serde_json::json!({"kind":"indexer","name":indexer.name,"ok":ok,"results":results,"error":error})
                });
            }
            for engine in cfg.websearch_engines.clone() {
                let client = client.clone();
                let flaresolverr = cfg.flaresolverr_url.clone();
                let term = term.clone();
                set.spawn(async move {
                    let engine_list = vec![engine.clone()];
                    let (ok, results, error) =
                        match crate::websearch::search(&client, &engine_list, &term, flaresolverr.as_deref()).await {
                            Ok(items) => (true, items.len() as i64, None),
                            Err(error) => (false, 0, Some(error.to_string())),
                        };
                    serde_json::json!({"kind":"engine","name":engine,"ok":ok,"results":results,"error":error})
                });
            }
        }
        None => {
            for feed in cfg.feed_urls.clone() {
                let client = client.clone();
                set.spawn(async move {
                    let (ok, status, error) = match client.get(&feed).send().await {
                        Ok(response) => (
                            response.status().is_success(),
                            response.status().as_u16() as i64,
                            None,
                        ),
                        Err(error) => (false, 0, Some(error.to_string())),
                    };
                    serde_json::json!({"kind":"feed","name":feed,"url":feed,"ok":ok,"status":status,"error":error})
                });
            }
            for indexer in cfg
                .indexers
                .iter()
                .filter(|indexer| indexer.enabled)
                .cloned()
            {
                let client = client.clone();
                set.spawn(async move {
                    let (ok, results, error) =
                        match crate::rss::fetch_torznab(&client, &indexer, "ita").await {
                            Ok(items) => (true, items.len() as i64, None),
                            Err(error) => (false, 0, Some(error.to_string())),
                        };
                    serde_json::json!({"kind":"indexer","name":indexer.name,"url":indexer.url,"ok":ok,"results":results,"error":error})
                });
            }
            for engine in cfg.websearch_engines.clone() {
                let client = client.clone();
                let flaresolverr = cfg.flaresolverr_url.clone();
                set.spawn(async move {
                    let engine_list = vec![engine.clone()];
                    let (ok, results, error) =
                        match crate::websearch::search(&client, &engine_list, "ita", flaresolverr.as_deref()).await {
                            Ok(items) => (true, items.len() as i64, None),
                            Err(error) => (false, 0, Some(error.to_string())),
                        };
                    serde_json::json!({"kind":"engine","name":engine,"ok":ok,"results":results,"error":error})
                });
            }
        }
    }
    let mut items = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(item) = joined {
            items.push(item);
        }
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"query":term,"items":items,"flaresolverr":cfg.flaresolverr_url.is_some(),"tmdb":cfg.tmdb_api_key.is_some()}),
        ),
    )
}
async fn check_ports(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let mut ports = Vec::new();
    for port in cfg.libtorrent.port_min..=cfg.libtorrent.port_max {
        let available = std::net::TcpListener::bind(("0.0.0.0", port)).is_ok();
        ports.push(serde_json::json!({"port":port,"available":available}));
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"port_min":cfg.libtorrent.port_min,"port_max":cfg.libtorrent.port_max,"ports":ports}),
        ),
    )
}

/// Elenca le interfacce di rete con IPv4 e tipo, per il killswitch VPN.
/// Formato compatibile con il legacy: `{"interfaces": {"wg0": {"ip": "…", "type": "VPN"}}}`.
async fn network_interfaces_view() -> Json<serde_json::Value> {
    let mut interfaces = serde_json::Map::new();
    for interface in crate::utils::network_interfaces() {
        interfaces.insert(
            interface.name,
            serde_json::json!({"ip": interface.ip, "type": interface.kind}),
        );
    }
    Json(serde_json::json!({"ok": true, "interfaces": interfaces}))
}

fn directory_size(path: &FsPath) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let child = entry.path();
            if child.is_dir() {
                directory_size(&child)
            } else {
                entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            }
        })
        .sum()
}
async fn trash_entries(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let path = cfg
        .trash_path
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("trash"));
    let mut items = Vec::new();
    let mut total_bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(&path) {
        for entry in entries.flatten() {
            let child = entry.path();
            let is_dir = child.is_dir();
            let size = if is_dir {
                directory_size(&child)
            } else {
                entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            };
            total_bytes = total_bytes.saturating_add(size);
            items.push(serde_json::json!({
                "name": entry.file_name().to_string_lossy(),
                "path": child.display().to_string(),
                "size_bytes": size,
                "is_dir": is_dir,
            }));
        }
    }
    items.sort_by(|left, right| {
        right
            .get("size_bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .cmp(
                &left
                    .get("size_bytes")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
    });
    let retention_days = cfg
        .settings
        .get("trash_retention_days")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "path": path.display().to_string(),
            "exists": path.is_dir(),
            "count": items.len(),
            "total_bytes": total_bytes,
            "retention_days": retention_days,
            "items": items,
        })),
    )
}
#[derive(serde::Deserialize, Default)]
pub struct TrashDeleteInput {
    /// Nomi (un solo componente) da eliminare dal cestino.
    #[serde(default)]
    pub names: Vec<String>,
    /// Elimina tutto il contenuto del cestino.
    #[serde(default)]
    pub all: bool,
}

/// Elimina dal cestino singoli elementi (per nome) oppure tutto. Accetta solo
/// nomi di un componente per impedire traversal fuori dalla cartella trash.
async fn delete_trash_entries(
    State(s): State<AppState>,
    Json(input): Json<TrashDeleteInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not delete trash"})),
        );
    }
    let cfg = latest_config(&s);
    let root = cfg
        .trash_path
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("trash"));
    let names = if input.all {
        std::fs::read_dir(&root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    } else {
        input.names
    };
    if names.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"nessun elemento da eliminare"})),
        );
    }
    let mut removed = 0usize;
    let mut bytes = 0u64;
    let mut errors: Vec<String> = Vec::new();
    for name in names {
        let candidate = std::path::Path::new(&name);
        // Un solo componente, niente "." / "..": resta dentro il trash.
        if name.trim().is_empty()
            || candidate.components().count() != 1
            || matches!(name.as_str(), "." | "..")
        {
            errors.push(name);
            continue;
        }
        let target = root.join(candidate);
        let Ok(metadata) = std::fs::symlink_metadata(&target) else {
            errors.push(name);
            continue;
        };
        let size = if metadata.is_dir() {
            directory_size(&target)
        } else {
            metadata.len()
        };
        let outcome = if metadata.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        match outcome {
            Ok(()) => {
                removed += 1;
                bytes = bytes.saturating_add(size);
            }
            Err(_) => errors.push(name),
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"removed":removed,"bytes":bytes,"errors":errors})),
    )
}

async fn db_info(State(s): State<AppState>) -> impl IntoResponse {
    let info = match s.db.lock().unwrap().info() {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    let files = [
        "rextto_series.db",
        "rextto_archive.db",
        "rextto_config.db",
        "rextto_comics.db",
    ]
    .iter()
    .map(|name| {
        let path = s.cfg.data_dir.join(name);
        let size_bytes = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        serde_json::json!({"name":name,"path":path,"size_bytes":size_bytes,"exists":path.is_file()})
    })
    .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"info":info,"files":files})),
    )
}
async fn db_action(
    State(s): State<AppState>,
    Json(input): Json<DbActionInput>,
) -> impl IntoResponse {
    let action = input.action.trim().to_ascii_lowercase();
    if !matches!(action.as_str(), "vacuum" | "analyze") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"action must be vacuum or analyze"})),
        );
    }
    // VACUUM/ANALYZE are blocking SQLite calls: run them on the blocking pool
    // so a slow compaction cannot stall a runtime worker. The guard is scoped
    // inside `run_db_action`; locking `s.db` a second time inside a match arm
    // used to self-deadlock (the scrutinee temporary stays alive for the whole
    // `match`), freezing the daemon.
    let db = s.db.clone();
    let archive = s.archive.clone();
    let comics = s.comics.clone();
    let data_dir = s.cfg.data_dir.clone();
    let task_action = action.clone();
    match tokio::task::spawn_blocking(move || {
        run_db_action(&db, &archive, &comics, &data_dir, &task_action)
    })
    .await
    {
        Ok(Ok((before_size, before_rows, after_size, after_rows))) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "action": action,
                "before": {"size_bytes": before_size, "rows": before_rows},
                "after": {"size_bytes": after_size, "rows": after_rows},
            })),
        ),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":format!("maintenance task failed: {error}")})),
        ),
    }
}

/// Blocking body of `/api/db/action`: measures before/after and runs the
/// requested SQLite maintenance. Kept separate so the mutex guard has a single
/// short scope and the function is unit-testable.
fn run_db_action(
    db: &Arc<Mutex<Database>>,
    archive: &Arc<Mutex<Archive>>,
    comics: &ComicsDb,
    data_dir: &std::path::Path,
    action: &str,
) -> anyhow::Result<(i64, i64, i64, i64)> {
    let config_path = data_dir.join("rextto_config.db");
    let config_size = |path: &std::path::Path| -> i64 {
        rusqlite::Connection::open(path)
            .map(|conn| crate::database::connection_size_bytes(&conn))
            .unwrap_or(0)
    };

    // Measure and optimize each database separately (no nested mutexes).
    let (mut before_size, mut before_rows) = {
        let database = db.lock().unwrap();
        (database.db_size_bytes(), database.db_total_rows())
    };
    {
        let archive = archive.lock().unwrap();
        before_size += archive.size_bytes();
        before_rows += archive.count().unwrap_or(0);
    }
    before_size += comics.size_bytes() + config_size(&config_path);

    {
        let database = db.lock().unwrap();
        database.optimize(action)?;
    }
    {
        let archive = archive.lock().unwrap();
        archive.optimize(action)?;
    }
    comics.optimize(action)?;
    // rextto_config.db has no long-lived handle in AppState; open it here.
    if let Ok(conn) = crate::config::open_config_db(&config_path) {
        let _ = crate::database::optimize_connection(&conn, action);
    }

    let (mut after_size, mut after_rows) = {
        let database = db.lock().unwrap();
        (database.db_size_bytes(), database.db_total_rows())
    };
    {
        let archive = archive.lock().unwrap();
        after_size += archive.size_bytes();
        after_rows += archive.count().unwrap_or(0);
    }
    after_size += comics.size_bytes() + config_size(&config_path);
    Ok((before_size, before_rows, after_size, after_rows))
}
async fn db_prune(State(s): State<AppState>, Json(input): Json<PruneInput>) -> impl IntoResponse {
    if input.preview {
        let retain = input.retain_cycles.unwrap_or(50).clamp(1, 10000);
        let error_age = input.error_age_days.unwrap_or(7).max(1);
        let seen_days = input.seen_retention_days.unwrap_or(0);
        return match (|| -> anyhow::Result<crate::database::PrunePreview> {
            let db = s.db.lock().unwrap();
            db.prune_preview(retain, error_age, seen_days)
        })() {
            Ok(preview) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "preview": true,
                    "report": preview,
                    "seen_removed": preview.seen_to_remove,
                })),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        };
    }
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify the database"})),
        );
    }
    let retain = input.retain_cycles.unwrap_or(50).clamp(1, 10000);
    let error_age = input.error_age_days.unwrap_or(7).max(1);
    let seen_days = input.seen_retention_days.unwrap_or(0);
    let result = (|| -> anyhow::Result<(crate::database::MaintenanceReport, usize)> {
        let db = s.db.lock().unwrap();
        let report = db.cleanup(retain, error_age)?;
        let seen_removed = db.prune_seen_older_than(seen_days)?;
        Ok((report, seen_removed))
    })();
    match result {
        Ok((report, seen_removed)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"report":report,"seen_removed":seen_removed})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
/// Read-only preview of the age/error maintenance prune: reports how many
/// cycle-history rows, stale failed torrents and feed-seen entries would be
/// removed, without touching the database.
async fn db_prune_preview(
    State(s): State<AppState>,
    Json(input): Json<PruneInput>,
) -> impl IntoResponse {
    let retain = input.retain_cycles.unwrap_or(50).clamp(1, 10000);
    let error_age = input.error_age_days.unwrap_or(7).max(1);
    let seen_days = input.seen_retention_days.unwrap_or(0);
    let result = (|| -> anyhow::Result<crate::database::PrunePreview> {
        let db = s.db.lock().unwrap();
        db.prune_preview(retain, error_age, seen_days)
    })();
    match result {
        Ok(preview) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "preview": true,
                "report": preview,
                "seen_removed": preview.seen_to_remove,
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn flaresolverr_test(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(url) = cfg
        .flaresolverr_url
        .clone()
        .filter(|value| !value.trim().is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"URL FlareSolverr non configurato"})),
        );
    };
    let endpoint = format!("{}/v1", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    match client
        .post(&endpoint)
        .json(&serde_json::json!({"cmd": "sessions.list"}))
        .send()
        .await
    {
        Ok(response) => match response.json::<serde_json::Value>().await {
            Ok(value) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": value.get("status").and_then(serde_json::Value::as_str) == Some("ok"),
                    "status": value.get("status"),
                    "sessions": value.get("sessions").cloned().unwrap_or_else(|| serde_json::json!([])),
                })),
            ),
            Err(error) => (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        },
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
#[derive(serde::Deserialize)]
pub struct KeywordPruneInput {
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub preview: bool,
}
async fn db_prune_keyword(
    State(s): State<AppState>,
    Json(input): Json<KeywordPruneInput>,
) -> impl IntoResponse {
    let mut keywords = input.keywords;
    if keywords.is_empty() {
        keywords = input
            .keyword
            .split([',', ';', '|', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
    }
    keywords.retain(|value| !value.trim().is_empty() && value.len() <= 128);
    if keywords.is_empty() || keywords.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"inserire da 1 a 64 parole, ciascuna di massimo 128 caratteri"}),
            ),
        );
    }
    if input.preview {
        let result = {
            let db = s.db.lock().unwrap();
            db.count_keywords(&keywords).and_then(|count| {
                db.search_keywords(&keywords, 300)
                    .map(|items| (count, items))
            })
        };
        return match result {
            Ok((count, items)) => (
                StatusCode::OK,
                Json(
                    serde_json::json!({"ok":true,"preview":true,"count":count,"keywords":keywords,"items":items}),
                ),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        };
    }
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify the database"})),
        );
    }
    match s.db.lock().unwrap().prune_keywords(&keywords) {
        Ok(removed) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"removed":removed,"keywords":keywords})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn test_notification(
    State(s): State<AppState>,
    input: Option<Json<TestNotificationInput>>,
) -> impl IntoResponse {
    let message = input
        .and_then(|Json(input)| input.message)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "Rextto: notifica di test".into());
    let notifier = Notifier::from_config(&latest_config(&s));
    match notifier.notify(&message).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"enabled":notifier.enabled()})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn backup_settings(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let value = |key: &str, default: &str| {
        cfg.settings
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.into())
    };
    Json(serde_json::json!({
        "ok": true,
        "retention": value("backup_retention", "5"),
        "schedule_hours": value("backup_schedule_hours", "0"),
        "schedule_at": value("backup_schedule_at", ""),
        "send_telegram": cfg.settings.get("backup_send_telegram").is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
        "ftp_host": value("backup_ftp_host", ""),
        "ftp_user": value("backup_ftp_user", ""),
        "ftp_path": value("backup_ftp_path", ""),
        "cloud_dir": value("backup_cloud_dir", ""),
        "ftp_password_configured": !cfg.settings.get("backup_ftp_password").map(|value| value.is_empty()).unwrap_or(true),
    }))
}
fn settings_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(if *value { "true" } else { "false" }.into()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
async fn save_backup_settings(
    State(s): State<AppState>,
    Json(input): Json<SettingsPatch>,
) -> impl IntoResponse {
    let allowed = [
        "backup_retention",
        "backup_schedule_hours",
        "backup_schedule_at",
        "backup_send_telegram",
        "backup_ftp_host",
        "backup_ftp_user",
        "backup_ftp_password",
        "backup_ftp_path",
        "backup_cloud_dir",
    ];
    let mut saved = Vec::new();
    for (key, value) in input.values {
        if !allowed.contains(&key.as_str()) || value.is_null() {
            continue;
        }
        let Some(value) = settings_value(&value) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":"invalid backup setting value"})),
            );
        };
        if value.len() > 4096 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":"backup setting too long"})),
            );
        }
        if let Err(error) = Config::save_setting(&s.cfg.data_dir, &key, &value) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            );
        }
        saved.push(key);
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"saved":saved})),
    )
}
async fn backup_send_telegram(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let notifier = Notifier::from_config(&cfg);
    let data_dir = cfg.data_dir.clone();
    let root = data_dir.join("backups");
    let retain = cfg
        .settings
        .get("backup_retention")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5)
        .clamp(1, 100);
    let result =
        tokio::task::spawn_blocking(move || backup::create_snapshot(&data_dir, &root, retain))
            .await;
    match result {
        Ok(Ok(path)) => {
            let sent = notifier
                .notify_backup_document(&path, "Rextto backup")
                .await
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"path":path,"sent":sent})),
            )
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct FtpTestInput {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// Tests FTP connectivity. Accepts the values currently typed in the form,
/// falling back to the saved settings (notably for the password, which the UI
/// never sends back).
async fn backup_test_ftp(
    State(s): State<AppState>,
    input: Option<Json<FtpTestInput>>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let input = input.map(|Json(value)| value).unwrap_or_default();
    let pick = |inline: Option<String>, key: &str| -> String {
        inline
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| cfg.settings.get(key).cloned())
            .unwrap_or_default()
    };
    let host = pick(input.host, "backup_ftp_host");
    let user = pick(input.user, "backup_ftp_user");
    let password = pick(input.password, "backup_ftp_password");
    let path = pick(input.path, "backup_ftp_path");
    if host.is_empty() || user.is_empty() || password.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"host, utente e password FTP sono obbligatori"})),
        );
    }
    let report = match tokio::task::spawn_blocking(move || {
        backup::test_ftp(&host, &user, &password, &path)
    })
    .await
    {
        Ok(report) => report,
        Err(error) => {
            tracing::error!(%error, "test FTP: task failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":format!("test FTP: {error}")})),
            );
        }
    };
    // Always 200 so the UI can show exactly which step succeeded/failed.
    tracing::info!(
        host = %report.host,
        path = %report.path,
        test_file = %report.test_file,
        connected = report.connected,
        logged_in = report.logged_in,
        entered_path = report.entered_path,
        uploaded = report.uploaded,
        deleted = report.deleted,
        error = %report.error.as_deref().unwrap_or("none"),
        "FTP test result"
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": report.ok(),
            "host": report.host,
            "user": report.user,
            "path": report.path,
            "test_file": report.test_file,
            "connected": report.connected,
            "logged_in": report.logged_in,
            "entered_path": report.entered_path,
            "uploaded": report.uploaded,
            "deleted": report.deleted,
            "error": report.error,
        })),
    )
}

const HANDLER_MAGNET: &str = r##"#!/bin/bash
# Rextto - handler protocollo magnet: invia il link a Rextto.
REXTTO_URL="${REXTTO_URL:-__REXTTO_URL__}"
LOG="/tmp/rextto-magnet.log"
MAGNET="${*}"

echo "$(date '+%F %T') magnet=${MAGNET}" >> "$LOG"
if [[ -z "$MAGNET" ]]; then
  notify-send "Rextto" "Nessun link magnet ricevuto" --icon=dialog-error 2>/dev/null
  exit 1
fi
JSON=$(python3 -c "import json,sys; print(json.dumps({'magnet': sys.argv[1]}))" "$MAGNET")
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${REXTTO_URL}/api/send-magnet" \
  -H 'Content-Type: application/json' --data-raw "$JSON" --max-time 10 2>/dev/null)
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "202" ]]; then
  notify-send "Rextto" "Torrent aggiunto" --icon=emblem-downloads 2>/dev/null
else
  notify-send "Rextto" "Rextto non raggiungibile (${REXTTO_URL}) - HTTP ${HTTP_CODE}" --icon=dialog-error 2>/dev/null
  echo "$(date '+%F %T') HTTP ${HTTP_CODE} ${RESPONSE}" >> "$LOG"
fi
"##;

const HANDLER_TORRENT: &str = r##"#!/bin/bash
# Rextto - handler file .torrent: invia il file locale (o l'URL) a Rextto.
REXTTO_URL="${REXTTO_URL:-__REXTTO_URL__}"
LOG="/tmp/rextto-torrent.log"
INPUT="$1"

if [[ -z "$INPUT" ]]; then
  notify-send "Rextto" "Nessun file .torrent ricevuto" --icon=dialog-error 2>/dev/null
  exit 1
fi

if [[ "$INPUT" == http://* || "$INPUT" == https://* ]]; then
  JSON=$(python3 -c "import json,sys; print(json.dumps({'magnet': sys.argv[1]}))" "$INPUT")
  RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${REXTTO_URL}/api/send-magnet" \
    -H 'Content-Type: application/json' --data-raw "$JSON" --max-time 30 2>/dev/null)
else
  FILEPATH="${INPUT#file://}"
  if [[ ! -f "$FILEPATH" ]]; then
    notify-send "Rextto" "File non trovato: $FILEPATH" --icon=dialog-error 2>/dev/null
    exit 1
  fi
  RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${REXTTO_URL}/api/upload-torrent" \
    --data-binary @"$FILEPATH" -H 'Content-Type: application/x-bittorrent' --max-time 30 2>/dev/null)
fi
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "202" ]]; then
  notify-send "Rextto" "Torrent aggiunto" --icon=emblem-downloads 2>/dev/null
else
  notify-send "Rextto" "Rextto non raggiungibile (${REXTTO_URL}) - HTTP ${HTTP_CODE}" --icon=dialog-error 2>/dev/null
  echo "$(date '+%F %T') HTTP ${HTTP_CODE} ${RESPONSE}" >> "$LOG"
fi
"##;

const HANDLER_MAGNET_DESKTOP: &str = r##"[Desktop Entry]
Name=Rextto Magnet Handler
Comment=Invia link magnet a Rextto
Exec=/usr/local/bin/rextto-magnet %U
Type=Application
MimeType=x-scheme-handler/magnet;
NoDisplay=true
StartupNotify=false
Terminal=false
"##;

const HANDLER_TORRENT_DESKTOP: &str = r##"[Desktop Entry]
Name=Rextto Torrent Handler
Comment=Invia file .torrent a Rextto
Exec=/usr/local/bin/rextto-torrent %U
Type=Application
MimeType=application/x-bittorrent;x-scheme-handler/magnet;
NoDisplay=true
StartupNotify=false
Terminal=false
"##;

const HANDLER_INSTALL: &str = r##"#!/bin/bash
# Rextto - installazione handler magnet/.torrent (Linux, xdg-utils).
# Richiede: curl, python3, xdg-utils, libnotify (opzionale).
set -e

REXTTO_URL="${REXTTO_URL:-__REXTTO_URL__}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Rextto - installazione handler torrent/magnet"
echo "URL Rextto: $REXTTO_URL"
echo ""

for cmd in curl python3 xdg-mime xdg-open; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "Mancante: $cmd (sudo apt install xdg-utils curl python3)"
    exit 1
  fi
done

echo "Installo gli script in /usr/local/bin ..."
for f in rextto-magnet rextto-torrent; do
  sudo cp "$SCRIPT_DIR/$f" "/usr/local/bin/$f"
  sudo chmod +x "/usr/local/bin/$f"
  echo "  $f"
done

DESKTOP_DIR="$HOME/.local/share/applications"
mkdir -p "$DESKTOP_DIR"
for f in rextto-magnet.desktop rextto-torrent.desktop; do
  cp "$SCRIPT_DIR/$f" "$DESKTOP_DIR/$f"
  echo "  $f"
done

echo "Registro i MIME handler ..."
xdg-mime default rextto-magnet.desktop x-scheme-handler/magnet
xdg-mime default rextto-torrent.desktop application/x-bittorrent
update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true

for PREFS in $(find "$HOME/.mozilla/firefox" -name prefs.js 2>/dev/null); do
  sed -i '/network.protocol-handler.expose.magnet/d' "$PREFS"
  sed -i '/network.protocol-handler.external.magnet/d' "$PREFS"
  echo 'user_pref("network.protocol-handler.expose.magnet", false);' >> "$PREFS"
  echo 'user_pref("network.protocol-handler.external.magnet", true);' >> "$PREFS"
  echo "  Firefox: $PREFS"
done

if curl -s --max-time 3 "$REXTTO_URL/api/status" &>/dev/null; then
  echo "Rextto raggiungibile."
else
  echo "Attenzione: Rextto non raggiungibile su $REXTTO_URL"
fi

echo ""
echo "Fatto. Riavvia i browser aperti."
echo "Verifica: xdg-mime query default x-scheme-handler/magnet"
"##;

#[derive(serde::Deserialize, Default)]
pub struct HandlerQuery {
    #[serde(default)]
    pub file: String,
}

/// Serves the OS-level protocol handlers (magnet/.torrent) with this server's
/// URL injected. `navigator.registerProtocolHandler` only works on HTTPS or
/// localhost, which is why a LAN deployment needs these xdg-open handlers.
async fn browser_handler_download(
    Query(query): Query<HandlerQuery>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1:5000");
    let base = format!("http://{host}");
    let requested = query.file.trim();
    let (template, mime) = match requested {
        "rextto-magnet" => (HANDLER_MAGNET, "text/x-shellscript"),
        "rextto-torrent" => (HANDLER_TORRENT, "text/x-shellscript"),
        "rextto-magnet.desktop" => (HANDLER_MAGNET_DESKTOP, "text/plain"),
        "rextto-torrent.desktop" => (HANDLER_TORRENT_DESKTOP, "text/plain"),
        "install.sh" => (HANDLER_INSTALL, "text/x-shellscript"),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":"file non valido"})),
            )
                .into_response();
        }
    };
    let body = template.replace("__REXTTO_URL__", &base);
    let mut response = body.into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(
        axum::http::header::CONTENT_TYPE,
        format!("{mime}; charset=utf-8").parse().unwrap(),
    );
    response_headers.insert(
        axum::http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{requested}\"").parse().unwrap(),
    );
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        "no-cache".parse().unwrap(),
    );
    response
}

async fn send_magnet(
    State(s): State<AppState>,
    Json(input): Json<AddTorrent>,
) -> impl IntoResponse {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    let cfg = latest_config(&s);
    let target = input.magnet.trim().to_string();
    if target.starts_with("magnet:") {
        let preferred = input
            .save_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::Path::new);
        return match s.torrents.add_with_path(&target, &cfg, preferred) {
            Ok(true) => {
                let hash = crate::utils::magnet_hash(&target);
                let mut paused = false;
                if input.start_paused {
                    if let Some(hash) = &hash {
                        paused = s.torrents.pause(hash).unwrap_or(false);
                    }
                }
                if input.no_rename {
                    if let Some(hash) = &hash {
                        let _ = s.db.lock().unwrap().set_torrent_no_rename(hash, true);
                    }
                }
                (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({"ok":true,"kind":"magnet","start_paused":paused})),
                )
            }
            Ok(false) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"duplicate"})),
            ),
            Err(error) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        };
    }
    if !target.starts_with("http://") && !target.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"magnet or http(s) URL required"})),
        );
    }
    if cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not download torrent files"})),
        );
    }
    let response = match reqwest::get(&target).await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if !response.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(
                serde_json::json!({"ok":false,"error":format!("remote torrent returned {}", response.status())}),
            ),
        );
    }
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if bytes.len() > 5_000_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"torrent file is too large"})),
        );
    }
    let dir = cfg
        .libtorrent_temp_dir
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("incomplete"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("upload-{}.torrent", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::write(&path, &bytes) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    match s.torrents.add_torrent_file(&path, &cfg.libtorrent_dir) {
        Ok(Some(hash)) => {
            let _ = std::fs::remove_file(&path);
            if input.no_rename {
                let _ = s.db.lock().unwrap().set_torrent_no_rename(&hash, true);
            }
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({"ok":true,"kind":"torrent","hash":hash})),
            )
        }
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"torrent client unavailable"})),
        ),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    }
}
async fn upload_torrent(State(s): State<AppState>, body: axum::body::Bytes) -> impl IntoResponse {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not add torrents"})),
        );
    }
    if body.is_empty() || body.len() > 5_000_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"torrent file is empty or too large"})),
        );
    }
    let cfg = latest_config(&s);
    let dir = cfg
        .libtorrent_temp_dir
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("incomplete"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("upload-{}.torrent", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::write(&path, &body) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    match s.torrents.add_torrent_file(&path, &cfg.libtorrent_dir) {
        Ok(Some(hash)) => {
            let _ = std::fs::remove_file(&path);
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({"ok":true,"kind":"torrent","hash":hash})),
            )
        }
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"torrent client unavailable"})),
        ),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    }
}
async fn trakt_settings(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let client = TraktClient::from_settings(&cfg.settings);
    Json(serde_json::json!({
        "ok": true,
        "configured": client.configured(),
        "authenticated": client.authenticated(),
        "client_id": cfg.settings.get("trakt_client_id").cloned().unwrap_or_default(),
        "client_id_configured": !cfg.settings.get("trakt_client_id").map(|value| value.is_empty()).unwrap_or(true),
        "client_secret_configured": !cfg.settings.get("trakt_client_secret").map(|value| value.is_empty()).unwrap_or(true),
        "watchlist_sync": cfg.settings.get("trakt_watchlist_sync").is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
        "scrobble_enabled": cfg.settings.get("trakt_scrobble_enabled").is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
        "calendar_days": cfg.settings.get("trakt_calendar_days").cloned().unwrap_or_else(|| "7".into()),
    }))
}
async fn save_trakt_settings(
    State(s): State<AppState>,
    Json(input): Json<SettingsPatch>,
) -> impl IntoResponse {
    save_provider_settings(
        &s,
        input,
        &[
            "trakt_client_id",
            "trakt_client_secret",
            "trakt_watchlist_sync",
            "trakt_scrobble_enabled",
            "trakt_calendar_days",
        ],
    )
}
async fn simkl_settings(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let client = SimklClient::from_settings(&cfg.settings);
    Json(serde_json::json!({
        "ok": true,
        "configured": client.configured(),
        "authenticated": client.authenticated(),
        "client_id": cfg.settings.get("simkl_client_id").cloned().unwrap_or_default(),
        "client_id_configured": !cfg.settings.get("simkl_client_id").map(|value| value.is_empty()).unwrap_or(true),
        "watchlist_status": cfg.settings.get("simkl_watchlist_status").cloned().unwrap_or_else(|| "plantowatch".into()),
        "calendar_days": cfg.settings.get("simkl_calendar_days").cloned().unwrap_or_else(|| "7".into()),
        "mark_watched": cfg.settings.get("simkl_mark_watched").is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1")),
    }))
}
async fn save_simkl_settings(
    State(s): State<AppState>,
    Json(input): Json<SettingsPatch>,
) -> impl IntoResponse {
    save_provider_settings(
        &s,
        input,
        &[
            "simkl_client_id",
            "simkl_watchlist_status",
            "simkl_calendar_days",
            "simkl_mark_watched",
        ],
    )
}
fn save_provider_settings(
    s: &AppState,
    input: SettingsPatch,
    allowed: &[&str],
) -> impl IntoResponse {
    let mut saved = Vec::new();
    for (key, value) in input.values {
        if !allowed.contains(&key.as_str()) || value.is_null() {
            continue;
        }
        let Some(value) = settings_value(&value) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":"invalid setting value"})),
            );
        };
        if value.len() > 4096 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":"setting too long"})),
            );
        }
        if let Err(error) = Config::save_setting(&s.cfg.data_dir, &key, &value) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            );
        }
        saved.push(key);
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"saved":saved})),
    )
}
fn collect_watchlist_entries(
    value: &serde_json::Value,
    kind: &str,
    out: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_watchlist_entries(item, kind, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(title) = map.get("title").and_then(serde_json::Value::as_str) {
                let year = map
                    .get("year")
                    .and_then(serde_json::Value::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                out.push((title.to_string(), year));
            } else {
                for child in map.values() {
                    collect_watchlist_entries(child, kind, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_watchlist(value: &serde_json::Value, out: &mut Vec<(String, String, String)>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_watchlist(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            let mut typed = false;
            for (key, child) in map {
                let kind = match key.as_str() {
                    "show" | "shows" | "anime" | "series" | "tv" => Some("series"),
                    "movie" | "movies" => Some("movie"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    typed = true;
                    let mut entries = Vec::new();
                    collect_watchlist_entries(child, kind, &mut entries);
                    for (title, year) in entries {
                        out.push((kind.to_string(), title, year));
                    }
                }
            }
            if !typed {
                for child in map.values() {
                    collect_watchlist(child, out);
                }
            }
        }
        _ => {}
    }
}

fn apply_watchlist_import(
    cfg: &mut Config,
    entries: &[(String, String, String)],
) -> serde_json::Value {
    let mut series_added = 0;
    let mut movies_added = 0;
    for (kind, title, year) in entries {
        if title.trim().is_empty() {
            continue;
        }
        if kind == "series" {
            if cfg
                .series
                .iter()
                .any(|item| item.name.eq_ignore_ascii_case(title))
            {
                continue;
            }
            cfg.series.push(SeriesConfig {
                name: title.clone(),
                seasons: "1+".into(),
                language: "ita".into(),
                enabled: true,
                ..Default::default()
            });
            series_added += 1;
        } else {
            if cfg
                .movies
                .iter()
                .any(|item| item.name.eq_ignore_ascii_case(title))
            {
                continue;
            }
            cfg.movies.push(MovieConfig {
                name: title.clone(),
                year: year.clone(),
                language: "ita".into(),
                enabled: true,
                ..Default::default()
            });
            movies_added += 1;
        }
    }
    serde_json::json!({
        "series_added": series_added,
        "movies_added": movies_added,
        "series_total": cfg.series.len(),
        "movies_total": cfg.movies.len(),
    })
}

async fn trakt_watchlist_import(State(s): State<AppState>) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let client = TraktClient::from_settings(&cfg.settings);
    let value = match client.watchlist().await {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    let mut entries = Vec::new();
    collect_watchlist(&value, &mut entries);
    let report = apply_watchlist_import(&mut cfg, &entries);
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"source":"trakt","found":entries.len(),"report":report}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn simkl_watchlist_import(State(s): State<AppState>) -> impl IntoResponse {
    let mut cfg = latest_config(&s);
    let client = SimklClient::from_settings(&cfg.settings);
    let value = match client.watchlist().await {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    let mut entries = Vec::new();
    collect_watchlist(&value, &mut entries);
    let report = apply_watchlist_import(&mut cfg, &entries);
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"source":"simkl","found":entries.len(),"report":report}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn comic_check_links(Json(input): Json<ComicCheckLinksInput>) -> impl IntoResponse {
    if input.urls.is_empty() || input.urls.len() > 20 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"provide 1-20 comic post URLs"})),
        );
    }
    let client = GetComicsClient::new();
    let mut items = Vec::new();
    for url in input.urls {
        match client.links(url.trim()).await {
            Ok(links) => items.push(serde_json::json!({"url":url,"ok":true,"links":links})),
            Err(error) => {
                items.push(serde_json::json!({"url":url,"ok":false,"error":error.to_string()}))
            }
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"items":items})),
    )
}
async fn comic_cycle(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let client = GetComicsClient::new();
    let default_root = cfg.data_dir.join("comics");
    let _guard = s.cycle_lock.lock().await;
    match comics::run_cycle(
        s.comics.as_ref(),
        &client,
        s.notifier.as_ref(),
        &default_root,
        s.torrents.as_ref(),
        &cfg,
    )
    .await
    {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"downloaded":count,"dry_run":cfg.dry_run})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn system_stats(State(s): State<AppState>) -> Json<serde_json::Value> {
    let trash = s
        .cfg
        .trash_path
        .clone()
        .unwrap_or_else(|| s.cfg.data_dir.join("trash"));
    let ramdisk = s
        .cfg
        .settings
        .get("libtorrent_ramdisk_dir")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    let health = health::check_with_paths(&health::HealthPaths {
        data_dir: &s.cfg.data_dir,
        trash_path: &trash,
        download_path: &s.cfg.libtorrent_dir,
        archive_root: s.cfg.archive_root.as_deref(),
        ramdisk_path: ramdisk.as_deref(),
    });
    Json(
        serde_json::json!({"ok":true,"health":health,"torrent_stats":s.torrents.stats(),"last_cycle":*s.last_cycle.lock().unwrap()}),
    )
}

async fn trakt_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let client = TraktClient::from_settings(&cfg.settings);
    Json(
        serde_json::json!({"configured":client.configured(),"authenticated":client.authenticated()}),
    )
}
async fn trakt_auth_start(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings)
        .device_start()
        .await
    {
        Ok(value) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"data":value})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn trakt_auth_poll(
    State(s): State<AppState>,
    Json(input): Json<AuthCode>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"ok":false,"error":"dry-run does not save integration tokens"}),
            ),
        );
    }
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings)
        .device_poll(&input.code)
        .await
    {
        Ok(value) => match (
            token_string(&value, "access_token"),
            token_string(&value, "refresh_token"),
        ) {
            (Ok(access), Ok(refresh)) => {
                let saved = Config::save_setting(&cfg.data_dir, "trakt_access_token", &access)
                    .and_then(|_| {
                        Config::save_setting(&cfg.data_dir, "trakt_refresh_token", &refresh)
                    });
                match saved {
                    Ok(()) => (
                        StatusCode::OK,
                        Json(serde_json::json!({"ok":true,"authenticated":true})),
                    ),
                    Err(error) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                    ),
                }
            }
            _ => (
                StatusCode::BAD_GATEWAY,
                Json(
                    serde_json::json!({"ok":false,"error":"Trakt response did not contain tokens","data":value}),
                ),
            ),
        },
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn trakt_watchlist(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings).watchlist().await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn trakt_calendar(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings).calendar(7).await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn trakt_auth_refresh(State(s): State<AppState>) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"ok":false,"error":"dry-run does not save integration tokens"}),
            ),
        );
    }
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings).refresh().await {
        Ok(value) => match (
            token_string(&value, "access_token"),
            token_string(&value, "refresh_token"),
        ) {
            (Ok(access), Ok(refresh)) => {
                match Config::save_setting(&cfg.data_dir, "trakt_access_token", &access).and_then(
                    |_| Config::save_setting(&cfg.data_dir, "trakt_refresh_token", &refresh),
                ) {
                    Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
                    Err(error) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                    ),
                }
            }
            _ => (
                StatusCode::BAD_GATEWAY,
                Json(
                    serde_json::json!({"ok":false,"error":"Trakt response did not contain refresh tokens"}),
                ),
            ),
        },
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn trakt_auth_revoke(State(s): State<AppState>) -> impl IntoResponse {
    revoke_integration_tokens(&s, &["trakt_access_token", "trakt_refresh_token"])
}
async fn trakt_scrobble(
    State(s): State<AppState>,
    Json(input): Json<ScrobbleInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not scrobble"})),
        );
    }
    let cfg = latest_config(&s);
    match TraktClient::from_settings(&cfg.settings)
        .scrobble(input.action.as_deref().unwrap_or("stop"), input.payload)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn simkl_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let client = SimklClient::from_settings(&cfg.settings);
    Json(
        serde_json::json!({"configured":client.configured(),"authenticated":client.authenticated()}),
    )
}
async fn simkl_auth_start(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match SimklClient::from_settings(&cfg.settings).pin_start().await {
        Ok(value) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"data":value})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn simkl_auth_poll(
    State(s): State<AppState>,
    Json(input): Json<AuthCode>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"ok":false,"error":"dry-run does not save integration tokens"}),
            ),
        );
    }
    let cfg = latest_config(&s);
    match SimklClient::from_settings(&cfg.settings)
        .pin_poll(&input.code)
        .await
    {
        Ok(value) => match token_string(&value, "access_token") {
            Ok(access) => {
                match Config::save_setting(&cfg.data_dir, "simkl_access_token", &access) {
                    Ok(()) => (
                        StatusCode::OK,
                        Json(serde_json::json!({"ok":true,"authenticated":true})),
                    ),
                    Err(error) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                    ),
                }
            }
            Err(_) => (
                StatusCode::BAD_GATEWAY,
                Json(
                    serde_json::json!({"ok":false,"error":"Simkl response did not contain an access token","data":value}),
                ),
            ),
        },
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn simkl_auth_revoke(State(s): State<AppState>) -> impl IntoResponse {
    revoke_integration_tokens(&s, &["simkl_access_token"])
}
async fn simkl_watchlist(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match SimklClient::from_settings(&cfg.settings).watchlist().await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn simkl_calendar(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    match SimklClient::from_settings(&cfg.settings).calendar().await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn simkl_scrobble(
    State(s): State<AppState>,
    Json(input): Json<ScrobbleInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not mark watched items"})),
        );
    }
    let cfg = latest_config(&s);
    match SimklClient::from_settings(&cfg.settings)
        .mark_watched(input.payload)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn save_setting(
    State(s): State<AppState>,
    Json(input): Json<SettingInput>,
) -> impl IntoResponse {
    let allowed = input.key.starts_with("libtorrent_")
        || input.key.starts_with("score_")
        || input.key.starts_with("tvdb_")
        || input.key.starts_with("trakt_")
        || input.key.starts_with("simkl_")
        || input.key.starts_with("backup_")
        || input.key.starts_with("notify_")
        || input.key.starts_with("jellyfin_")
        || input.key.starts_with("plex_")
        || matches!(
            input.key.as_str(),
            "active"
                | "refresh_interval"
                | "url"
                | "indexers"
                | "websearch_engines"
                | "blacklist"
                | "content_filters"
                | "max_release_age_days"
                | "gap_fill_max_per_series"
                | "gap_fill_max_per_cycle"
                | "gap_filling"
                | "gap_deep_interval_hours"
                | "gap_deep_max_per_cycle"
                | "flaresolverr_url"
                | "tmdb_api_key"
                | "tmdb_language"
                | "default_language"
                | "rename_episodes"
                | "rename_format"
                | "rename_template"
                | "api_token"
                | "archive_root"
                | "trash_path"
                | "libtorrent_dir"
                | "libtorrent_temp_dir"
                | "libtorrent_ramdisk_dir"
                | "libtorrent_extra_settings"
                | "cleanup_upgrades"
                | "cleanup_min_score_diff"
                | "upgrade_min_score_diff"
                | "cleanup_action"
                | "min_free_space_gb"
                | "trash_retention_days"
                | "archive_retention_days"
                | "archive_cleanup_enabled"
                | "archive_max_age_days"
                | "archive_keep_min"
                | "stop_on_old_page_threshold"
                | "debug_enabled"
                | "move_episodes"
                | "rename_verify_interval"
                | "auto_remove_completed"
                | "telegram_bot_token"
                | "telegram_chat_id"
                | "email_smtp"
                | "email_from"
                | "email_to"
                | "email_password"
        );
    if !allowed || input.key.len() > 128 || input.value.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"setting is not writable through this endpoint"}),
            ),
        );
    }
    let mut value = input.value;
    if input.key == "indexers" {
        let mut indexers = match serde_json::from_str::<Vec<IndexerConfig>>(&value) {
            Ok(value) => value,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"ok":false,"error":"indexers must be a JSON array"})),
                )
            }
        };
        let current = latest_config(&s);
        for indexer in &mut indexers {
            if indexer.api_key.is_empty() {
                indexer.api_key = current
                    .indexers
                    .iter()
                    .find(|old| old.name == indexer.name && old.url == indexer.url)
                    .map(|old| old.api_key.clone())
                    .unwrap_or_default();
            }
        }
        value = match serde_json::to_string(&indexers) {
            Ok(value) => value,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                )
            }
        };
    }
    if value.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"setting is too large"})),
        );
    }
    match Config::save_setting(&s.cfg.data_dir, &input.key, &value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"restart_required":true})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn delete_setting(State(s): State<AppState>, Path(key): Path<String>) -> impl IntoResponse {
    if !key.starts_with("score_group_") || key.len() <= "score_group_".len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"only custom score groups can be deleted here"}),
            ),
        );
    }
    match Config::delete_setting(&s.cfg.data_dir, &key) {
        Ok(removed) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"removed":removed,"key":key})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
fn revoke_integration_tokens(s: &AppState, keys: &[&str]) -> (StatusCode, Json<serde_json::Value>) {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"ok":false,"error":"dry-run does not modify integration tokens"}),
            ),
        );
    }
    let result = keys
        .iter()
        .try_for_each(|key| Config::save_setting(&s.cfg.data_dir, key, ""));
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"authenticated":false})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn cycles(State(s): State<AppState>) -> impl IntoResponse {
    match s.db.lock().unwrap().recent_cycles(100) {
        Ok(cycles) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"cycles":cycles})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn stats_api(State(s): State<AppState>) -> Json<serde_json::Value> {
    let consumption = s.db.lock().unwrap().consumption_stats().ok();
    Json(
        serde_json::json!({"last_cycle":*s.last_cycle.lock().unwrap(),"torrent_stats":s.torrents.stats(),"consumption":consumption}),
    )
}
async fn recent_downloads(State(s): State<AppState>) -> impl IntoResponse {
    if let Some(cached) = cache_get("recent_downloads", Duration::from_secs(30)) {
        return Json(cached).into_response();
    }
    let items = match s.db.lock().unwrap().recent_downloads(100) {
        Ok(items) => items,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
                .into_response()
        }
    };
    let cfg = latest_config(&s);
    let tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    // Le ricerche TMDB per i poster vengono eseguite in parallelo (prima erano
    // 100 chiamate sequenziali, ~3 s): manteniamo l'ordine con l'indice.
    let mut set = tokio::task::JoinSet::new();
    for (index, item) in items.into_iter().enumerate() {
        let tmdb = tmdb.clone();
        set.spawn(async move {
            let mut value = serde_json::to_value(&item).unwrap_or_default();
            let poster = if item.kind == "series" {
                tmdb.poster_for_series(&item.name).await.ok().flatten()
            } else {
                tmdb.search_movie(&item.name, item.year)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|movie| movie.poster_path)
            };
            if let Some(path) = poster {
                value["poster"] =
                    serde_json::Value::String(format!("https://image.tmdb.org/t/p/w154{path}"));
            }
            (index, value)
        });
    }
    let mut collected = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(entry) = joined {
            collected.push(entry);
        }
    }
    collected.sort_by_key(|(index, _)| *index);
    let out = collected
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    let response = serde_json::json!({"ok":true,"items":out});
    cache_put("recent_downloads", &response);
    Json(response).into_response()
}
async fn last_cycle(State(s): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({"ok":true,"cycle":*s.last_cycle.lock().unwrap()}))
}
async fn gaps(State(s): State<AppState>) -> impl IntoResponse {
    let db = s.db.lock().unwrap();
    match db.archive_gaps() {
        Ok(items) => {
            let cfg = latest_config(&s);
            let air_dates = db.episode_air_dates().unwrap_or_default();
            let items = items.into_iter().filter(|(series, season, _)| cfg.find_series_match(series, Some(*season)).is_some()).map(|(series, season, episode)| {
                let air_date = air_dates.get(&(series.clone(), season, episode)).cloned().unwrap_or_default();
                serde_json::json!({"series":series,"season":season,"episode":episode,"air_date":air_date})
            }).collect::<Vec<_>>();
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"count":items.len(),"items":items})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn series_episodes(State(s): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let ignored = find_series(&cfg, &name)
        .map(|series| series.ignored_seasons.clone())
        .unwrap_or_default();
    match s.db.lock().unwrap().episodes_for_series(&name, &ignored) {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":items})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
fn parse_season_episode(name: &str) -> Option<(i64, i64)> {
    static PATTERNS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            regex::Regex::new(
                r"(?i)\bs(?:tagione)?\s*(\d{1,2})\s*e(?:p(?:isodio)?)?\.?\s*(\d{1,4})",
            )
            .expect("season pattern"),
            regex::Regex::new(r"(?i)\b(\d{1,2})\s*x\s*(\d{1,4})").expect("1x05 pattern"),
            regex::Regex::new(r"(?i)stagione\s*(\d{1,2}).*?episodio\s*(\d{1,4})")
                .expect("italian pattern"),
        ]
    });
    for pattern in patterns {
        if let Some(capture) = pattern.captures(name) {
            let season = capture
                .get(1)
                .and_then(|value| value.as_str().parse::<i64>().ok());
            let episode = capture
                .get(2)
                .and_then(|value| value.as_str().parse::<i64>().ok());
            if let (Some(season), Some(episode)) = (season, episode) {
                return Some((season, episode));
            }
        }
    }
    None
}

/// Registra un file d'archivio nel DB (percorso, titolo con i tag di qualità e
/// score): così i confronti di upgrade sanno che l'episodio è già presente e non
/// riscaricano una release inferiore. Usato dalla verifica di rinomina per i
/// file non ancora tracciati.
fn link_archive_file(
    db: &Arc<Mutex<Database>>,
    series: &str,
    season: i64,
    episode: i64,
    path: &FsPath,
) {
    let Some(title) = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let size = path
        .metadata()
        .map(|value| value.len().min(i64::MAX as u64) as i64)
        .unwrap_or(0);
    if let Err(error) = db.lock().unwrap().sync_archive_file(
        series,
        season,
        episode,
        title,
        &path.display().to_string(),
        size,
    ) {
        tracing::warn!(%error, series, season, episode, "archive file linking failed");
    }
}

fn scan_archive_path(
    db: &Arc<Mutex<Database>>,
    series: &SeriesConfig,
    path: &FsPath,
) -> anyhow::Result<(usize, usize)> {
    if !path.is_dir() {
        anyhow::bail!("archive path is not a directory: {}", path.display());
    }
    let mut found = 0;
    let mut updated = 0;
    for file in postprocess::video_files(path)? {
        let Some(name) = file.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some((season, episode)) = parse_season_episode(name) else {
            continue;
        };
        if season <= 0 || episode <= 0 {
            continue;
        }
        found += 1;
        let size = file
            .metadata()
            .map(|value| value.len().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        // Il titolo salvato è il nome file completo (senza estensione): contiene
        // i tag di qualità, così lo score e i confronti di upgrade restano
        // corretti. Usare solo il titolo dell'episodio faceva perdere
        // risoluzione/sorgente e ogni file scansionato risultava inferiore.
        let title = FsPath::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(name)
            .to_string();
        db.lock().unwrap().sync_archive_file(
            &series.name,
            season,
            episode,
            &title,
            &file.display().to_string(),
            size,
        )?;
        updated += 1;
    }
    Ok((found, updated))
}
async fn scan_series_archive(
    State(s): State<AppState>,
    Path(name): Path<String>,
    input: Option<Json<ArchiveScanInput>>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(series) = cfg
        .series
        .iter()
        .find(|series| series.name == name || series.aliases.iter().any(|alias| alias == &name))
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    // Skip while a torrent import is writing this archive: a scan would record
    // files that are still being copied/renamed.
    if archive_import_busy().lock().unwrap().contains(&series.name) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok":true,
                "skipped":true,
                "reason":"archive import in progress",
            })),
        );
    }
    let requested_path = input.and_then(|Json(input)| input.path);
    let path = requested_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(FsPath::new)
        .map(FsPath::to_path_buf)
        .unwrap_or_else(|| FsPath::new(&series.archive_path).to_path_buf());
    match scan_archive_path(&s.db, series, &path) {
        Ok((found, updated)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"found":found,"updated":updated,"series":series.name,"path":path}),
            ),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn scan_all_archives(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let mut updated = 0usize;
    let mut errors = Vec::new();
    for series in cfg
        .series
        .iter()
        .filter(|series| series.enabled && !series.archive_path.trim().is_empty())
        .filter(|series| !archive_import_busy().lock().unwrap().contains(&series.name))
    {
        match scan_archive_path(&s.db, series, FsPath::new(&series.archive_path)) {
            Ok((_found, count)) => updated += count,
            Err(error) => {
                errors.push(serde_json::json!({"series":series.name,"error":error.to_string()}))
            }
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"updated":updated,"errors":errors})),
    )
}
async fn movie_history(State(s): State<AppState>) -> impl IntoResponse {
    match s.db.lock().unwrap().downloaded_movies(500) {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":items})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn ignore_episode(
    State(s): State<AppState>,
    Path((series, season, episode)): Path<(String, i64, i64)>,
    Json(input): Json<IgnoreEpisodeInput>,
) -> impl IntoResponse {
    match s.db.lock().unwrap().set_episode_ignored(
        &series,
        season,
        episode,
        input.ignored,
        &input.reason,
    ) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"ignored":input.ignored})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn force_episode(
    State(s): State<AppState>,
    Path((series, season, episode)): Path<(String, i64, i64)>,
) -> impl IntoResponse {
    match s
        .db
        .lock()
        .unwrap()
        .reset_episode(&series, season, episode, false)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"forced":true})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn redownload_episode(
    State(s): State<AppState>,
    Path((series, season, episode)): Path<(String, i64, i64)>,
) -> impl IntoResponse {
    match s
        .db
        .lock()
        .unwrap()
        .reset_episode(&series, season, episode, true)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"redownload":true,"message":"episodio reso nuovamente disponibile al prossimo ciclo"}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn search_episode(
    State(s): State<AppState>,
    Path((series_name, season, episode)): Path<(String, i64, i64)>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &series_name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    if series.ignored_seasons.contains(&season)
        || !Config::season_allowed_for_scan(&series.seasons, season)
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"ok":false,"error":"season is not monitored"})),
        );
    }
    let query = format!("{} S{season:02}E{episode:02}", series.name);
    let results = search_series_episode_sources(
        &s,
        &cfg,
        &series,
        season,
        episode,
    )
    .await;
    let feed_matches = results
        .iter()
        .filter(|result| result.get("origin").and_then(serde_json::Value::as_str) == Some("Feed RSS"))
        .count();
    let _ =
        s.db.lock()
            .unwrap()
            .mark_gap_searched(&series.name, season, episode);
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"query":query,"results":results,"feed_matches":feed_matches}),
        ),
    )
}

/// Cerca una puntata nei feed RSS già acquisiti, nell'archivio storico e nelle
/// sorgenti live (indexer/web). I risultati mantengono l'origine per mostrarla
/// nella riga dell'episodio e permettere all'utente di scegliere consapevolmente.
async fn search_series_episode_sources(
    s: &AppState,
    cfg: &Config,
    series: &SeriesConfig,
    season: i64,
    episode: i64,
) -> Vec<serde_json::Value> {
    let mut results = stored_series_episode_sources(s, series, season, episode);
    let query = format!("{} S{season:02}E{episode:02}", series.name);

    // Ricerca live sugli indexer Torznab e sui motori web configurati. I
    // motori web sono limitati a 15 s, gli indexer no.
    for release in s.engine.search_query_manual(cfg, &query).await {
        if release_matches_series_episode(&release, series, season, episode) {
            results.push(serde_json::json!({"release":release,"origin":"Indexer / web"}));
        }
    }
    finalize_episode_search_results(results, cfg, series)
}

fn release_matches_series_episode(
    release: &Release,
    series: &SeriesConfig,
    season: i64,
    episode: i64,
) -> bool {
    release.kind == "series"
        && release.season == Some(season)
        // Una ricerca puntuale deve proporre anche i pack che contengono la
        // puntata (S01E01-08) e i pack completi di stagione (S01). Prima
        // venivano accettate solo release con `episode` identico, perciò una
        // puntata 5 risultava senza alternative quando era disponibile solo
        // un pack iniziato dalla puntata 1.
        && if release.episode_range.is_empty() {
            release.episode == Some(episode)
        } else {
            release.episode_range.contains(&episode) || release.episode_range.contains(&0)
        }
        && release.series.as_deref().is_some_and(|name| {
            crate::parser::series_names_match(&series.name, name)
                || series
                    .aliases
                    .iter()
                    .any(|alias| crate::parser::series_names_match(alias, name))
        })
}

/// Risultati già disponibili localmente: non produce mai traffico di rete.
fn stored_series_episode_sources(
    s: &AppState,
    series: &SeriesConfig,
    season: i64,
    episode: i64,
) -> Vec<serde_json::Value> {
    let query = format!("{} S{season:02}E{episode:02}", series.name);
    let mut results = Vec::new();

    // Feed RSS/HTML già scanditi dal ciclo: sono immediati da interrogare e
    // non richiedono un nuovo polling della sorgente remota.
    for (title, magnet, source) in s
        .db
        .lock()
        .unwrap()
        .series_feed_for_episode(season, episode, 200)
        .unwrap_or_default()
    {
        if let Some(release) = crate::parser::parse_release(&title, &magnet, &source) {
            if release_matches_series_episode(&release, series, season, episode) {
                results.push(serde_json::json!({"release":release,"origin":"Feed RSS"}));
            }
        }
    }

    // Archivio delle release precedentemente acquisite da feed/indexer/web.
    for (title, magnet, source) in s.archive.lock().unwrap().search(&query).unwrap_or_default() {
        if let Some(release) = crate::parser::parse_release(&title, &magnet, &source) {
            if release_matches_series_episode(&release, series, season, episode) {
                results.push(serde_json::json!({"release":release,"origin":"Archivio"}));
            }
        }
    }
    results
}

fn finalize_episode_search_results(
    mut results: Vec<serde_json::Value>,
    cfg: &Config,
    series: &SeriesConfig,
) -> Vec<serde_json::Value> {
    // I risultati manuali devono superare gli stessi filtri applicati al click
    // su "Accoda". Senza questo passaggio la UI mostrava, come "compatibili",
    // release con qualità/lingua/esclusioni non ammesse e il click falliva solo
    // dopo con un messaggio generico.
    results.retain(|result| {
        result
            .get("release")
            .and_then(|release| serde_json::from_value::<Release>(release.clone()).ok())
            .is_some_and(|release| {
                cfg.release_allowed(&release)
                    && Config::series_release_allowed(series, &release.quality, &release.title)
            })
    });
    let mut seen = HashSet::new();
    results.retain(|result| {
        result
            .get("release")
            .and_then(|release| release.get("magnet"))
            .and_then(serde_json::Value::as_str)
            .and_then(crate::utils::magnet_hash)
            .is_some_and(|hash| {
                let origin = result
                    .get("origin")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                seen.insert(format!("{hash}:{origin}"))
            })
    });
    results.sort_by_key(|result| {
        std::cmp::Reverse(
            result
                .get("release")
                .and_then(|release| serde_json::from_value::<Release>(release.clone()).ok())
                .map(|release| release.quality.score_with_settings(&cfg.settings))
                .unwrap_or_default(),
        )
    });
    results
}
async fn delete_episode(
    State(s): State<AppState>,
    Path((series, season, episode)): Path<(String, i64, i64)>,
) -> impl IntoResponse {
    match s
        .db
        .lock()
        .unwrap()
        .reset_episode(&series, season, episode, true)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"deleted":true})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn search_missing(
    State(s): State<AppState>,
    Json(input): Json<MissingSearchInput>,
) -> impl IntoResponse {
    if input.series.trim().is_empty() || input.season < 1 || input.episode < 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid missing episode"})),
        );
    }
    let cfg = latest_config(&s);
    let Some(series) = find_series(&cfg, &input.series).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"series not found"})),
        );
    };
    if series.ignored_seasons.contains(&input.season)
        || !Config::season_allowed_for_scan(&series.seasons, input.season)
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"ok":false,"error":"season is not monitored"})),
        );
    }
    let query = format!(
        "{} S{:02}E{:02}",
        input.series.trim(),
        input.season,
        input.episode
    );
    let mut results = s.engine.search_query_manual(&cfg, &query).await;
    for (title, magnet, source) in s.archive.lock().unwrap().search(&query).unwrap_or_default() {
        if let Some(release) =
            crate::parser::parse_release(&title, &magnet, &format!("archive:{source}"))
        {
            results.push(release);
        }
    }
    let mut seen = HashSet::new();
    results.retain(|release| {
        release_matches_series_episode(release, &series, input.season, input.episode)
            && cfg.release_allowed(release)
            && Config::series_release_allowed(&series, &release.quality, &release.title)
            && crate::utils::magnet_hash(&release.magnet).is_some_and(|hash| seen.insert(hash))
    });
    results.sort_by_key(|release| {
        std::cmp::Reverse(release.quality.score_with_settings(&s.cfg.settings))
    });
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"query":query,"results":results})),
    )
}
async fn calendar(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    if cfg.tmdb_api_key.is_none() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
        );
    }
    if let Some(cached) = cache_get("calendar", Duration::from_secs(120)) {
        return (StatusCode::OK, Json(cached));
    }
    let tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    // Una chiamata TMDB per serie (id + prossimo episodio + poster) in
    // parallelo: prima erano ~2 chiamate sequenziali per serie (~2,6 s).
    let mut set = tokio::task::JoinSet::new();
    for series in cfg.series.iter().filter(|series| series.enabled).cloned() {
        let tmdb = tmdb.clone();
        set.spawn(async move {
            let tmdb_id = if series.tmdb_id.trim().is_empty() {
                tmdb.resolve_series_id(&series.name).await.ok().flatten()
            } else {
                Some(series.tmdb_id.clone())
            };
            let tmdb_id = tmdb_id?;
            match tmdb.next_episode(&tmdb_id).await {
                Ok(Some(episode)) => {
                    let poster = tmdb
                        .poster_for_series(&series.name)
                        .await
                        .ok()
                        .flatten()
                        .map(|path| format!("https://image.tmdb.org/t/p/w154{path}"));
                    Some(serde_json::json!({"series":series.name,"tmdb_id":tmdb_id,"episode":episode,"poster":poster}))
                }
                Ok(None) => None,
                Err(error) => {
                    tracing::debug!(series=%series.name, %error, "TMDB calendar lookup failed");
                    None
                }
            }
        });
    }
    let mut items = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(Some(item)) = joined {
            items.push(item);
        }
    }
    sort_calendar_items(&mut items);
    let response = serde_json::json!({"ok":true,"items":items});
    cache_put("calendar", &response);
    (StatusCode::OK, Json(response))
}

/// Ordina le voci del calendario per data di messa in onda (ISO, ordine
/// lessicografico); le voci senza data finiscono in fondo.
fn sort_calendar_items(items: &mut [serde_json::Value]) {
    let air_date = |value: &serde_json::Value| {
        value
            .get("episode")
            .and_then(|episode| episode.get("air_date"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    items.sort_by(|a, b| {
        let (left, right) = (air_date(a), air_date(b));
        match (left.is_empty(), right.is_empty()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => left.cmp(&right),
        }
    });
}
/// Simulatore punteggi: analizza un titolo release e restituisce campi,
/// scomposizione del punteggio base, punteggio con le impostazioni correnti e
/// se la release verrebbe accettata/monitorata.
async fn score_preview(
    State(s): State<AppState>,
    Json(input): Json<ScorePreviewInput>,
) -> impl IntoResponse {
    let title = input.title.trim();
    if title.is_empty() || title.len() > 512 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"titolo non valido (1-512 caratteri)"})),
        );
    }
    let cfg = latest_config(&s);
    let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
    let Some(release) = parser::parse_release(title, magnet, "simulator") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"impossibile analizzare il titolo"})),
        );
    };
    let quality = release.quality.clone();
    let breakdown = quality
        .score_breakdown()
        .into_iter()
        .map(|(label, value)| serde_json::json!({"label": label, "value": value}))
        .collect::<Vec<_>>();
    let probe = release
        .series
        .clone()
        .unwrap_or_else(|| release.title.clone());
    let matched_series = cfg
        .series
        .iter()
        .find(|series| {
            series.enabled
                && (parser::series_names_match(&series.name, &probe)
                    || series
                        .aliases
                        .iter()
                        .any(|alias| parser::series_names_match(alias, &probe)))
        })
        .map(|series| series.name.clone());
    let matched_movie = cfg
        .movies
        .iter()
        .find(|movie| movie.enabled && parser::series_names_match(&movie.name, &probe))
        .map(|movie| movie.name.clone());
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "kind": release.kind,
            "series": release.series,
            "season": release.season,
            "episode": release.episode,
            "is_pack": release.is_pack,
            "year": release.year,
            "quality": quality,
            "base_score": quality.score(),
            "score": quality.score_with_settings(&cfg.settings),
            "breakdown": breakdown,
            "allowed": cfg.release_allowed(&release),
            "matched_series": matched_series,
            "matched_movie": matched_movie,
        })),
    )
}
async fn rescore_database(State(s): State<AppState>) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify database scores"})),
        );
    }
    let cfg = latest_config(&s);
    match s.db.lock().unwrap().rescore(&cfg.settings) {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"updated":count})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn cleanup_database(State(s): State<AppState>) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not clean database"})),
        );
    }
    let cfg = latest_config(&s);
    let archive_days = cfg
        .settings
        .get("archive_max_age_days")
        .and_then(|value| value.parse::<i64>().ok())
        .or_else(|| {
            cfg.settings
                .get("archive_retention_days")
                .and_then(|value| value.parse::<i64>().ok())
        })
        .unwrap_or(0);
    let archive_keep_min = cfg
        .settings
        .get("archive_keep_min")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let archive_cleanup_enabled = cfg
        .settings
        .get("archive_cleanup_enabled")
        .map(|value| matches!(value.as_str(), "yes" | "true" | "1"))
        .unwrap_or(false)
        || cfg.settings.contains_key("archive_retention_days");
    let archive_removed = if archive_cleanup_enabled && archive_days > 0 {
        s.archive
            .lock()
            .unwrap()
            .cleanup_older_than_keeping(archive_days, archive_keep_min)
            .unwrap_or(0)
    } else {
        0
    };
    match s.db.lock().unwrap().cleanup(1000, 30) {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"report":report,"archive_removed":archive_removed})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn set_log_level(
    State(s): State<AppState>,
    Json(input): Json<LogLevel>,
) -> impl IntoResponse {
    if input.level.len() > 128 || input.level.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid log level"})),
        );
    }
    let filter = match EnvFilter::try_new(&input.level) {
        Ok(filter) => filter,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    match s.log_reload.lock().unwrap().reload(filter) {
        Ok(()) => {
            tracing::info!(level=%input.level, "runtime log level changed");
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"level":input.level})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn log_level_get() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok":true,"level":"runtime"}))
}
async fn comics(State(s): State<AppState>) -> Json<Vec<ComicMonitored>> {
    Json(s.comics.list_monitored(false).unwrap_or_default())
}
async fn comic_downloads() -> Json<Vec<comics::ComicDownload>> {
    Json(comics::http_downloads())
}
async fn comic_links(Json(input): Json<ComicLinksInput>) -> impl IntoResponse {
    if input.url.trim().is_empty() || input.url.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"post URL is required"})),
        );
    }
    match GetComicsClient::new().links(input.url.trim()).await {
        Ok(links) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"links":links})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn comic_explore(Json(input): Json<ComicExploreInput>) -> impl IntoResponse {
    if input.query.trim().is_empty() || input.query.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"query di ricerca non valida"})),
        );
    }
    match GetComicsClient::new()
        .search_posts(input.query.trim())
        .await
    {
        Ok((url, posts)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"query_url":url,"items":posts})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn comic_download(
    State(s): State<AppState>,
    Json(input): Json<ComicDownloadInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"ok":false,"error":"download manuali disabilitati in dry-run"}),
            ),
        );
    }
    if input.url.trim().is_empty() || input.title.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"url e titolo sono obbligatori"})),
        );
    }
    let cfg = latest_config(&s);
    let monitored_paths = s
        .comics
        .list_monitored(false)
        .unwrap_or_default()
        .into_iter()
        .map(|comic| comic.save_path)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let target = if input.save_path.trim().is_empty() {
        cfg.data_dir.join("comics")
    } else {
        PathBuf::from(input.save_path.trim())
    };
    let allowed = target.starts_with(&cfg.data_dir)
        || monitored_paths.iter().any(|path| target.starts_with(path));
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"ok":false,"error":"percorso comics non configurato"})),
        );
    }
    let client = GetComicsClient::new();
    let result = match input.method.to_ascii_lowercase().as_str() {
        "direct" | "http" => client
            .download_direct(input.url.trim(), &target, input.title.trim())
            .await
            .map(|path| serde_json::json!({"path":path,"method":"http"})),
        "mega" => {
            let executable = std::env::var_os("REXTTO_MEGADL")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("megadl"));
            match client.resolve_mega(input.url.trim()).await {
                Ok(url) => crate::comics::download_mega(&executable, &url, &target)
                    .await
                    .map(|path| serde_json::json!({"path":path,"method":"mega"})),
                Err(error) => Err(error),
            }
        }
        "torrent" => match client.download_torrent(input.url.trim(), &target).await {
            Ok(path) => match s.torrents.add_torrent_file(&path, &target) {
                Ok(Some(hash)) => {
                    let _ = std::fs::remove_file(&path);
                    if !input.post_url.trim().is_empty() {
                        let _ = s.comics.add_torrent(
                            &hash,
                            input.post_url.trim(),
                            input.title.trim(),
                            &target,
                        );
                    }
                    Ok(serde_json::json!({"hash":hash,"method":"torrent"}))
                }
                Ok(None) => Err(anyhow::anyhow!("torrent non aggiunto")),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        },
        "magnet" => match s.torrents.add(&input.url, &cfg) {
            Ok(true) => {
                if let Some(hash) = crate::utils::magnet_hash(&input.url) {
                    let _ = s.comics.add_torrent(
                        &hash,
                        input.post_url.trim(),
                        input.title.trim(),
                        &target,
                    );
                }
                Ok(serde_json::json!({"method":"magnet"}))
            }
            Ok(false) => Err(anyhow::anyhow!("magnet duplicato")),
            Err(error) => Err(error),
        },
        _ => Err(anyhow::anyhow!("metodo comics non supportato")),
    };
    match result {
        Ok(value) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"ok":true,"result":value})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn comic_weekly_links(Json(input): Json<ComicWeeklyInput>) -> impl IntoResponse {
    if !regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$")
        .is_ok_and(|pattern| pattern.is_match(input.date.trim()))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"data weekly non valida"})),
        );
    }
    match GetComicsClient::new().weekly_links(input.date.trim()).await {
        Ok((url, links)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"post_url":url,"links":links,"date":input.date})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn comics_history(State(s): State<AppState>) -> impl IntoResponse {
    match s.comics.history(100) {
        Ok(value) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":value})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn delete_comics_history(
    State(s): State<AppState>,
    Json(input): Json<ComicLinksInput>,
) -> impl IntoResponse {
    if input.url.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"post URL is required"})),
        );
    }
    match s.comics.remove_history(input.url.trim()) {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"deleted":deleted})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn comics_weekly(State(s): State<AppState>) -> impl IntoResponse {
    let enabled = s
        .comics
        .setting("weekly_enabled", "no")
        .map(|value| matches!(value.as_str(), "yes" | "true" | "1"))
        .unwrap_or(false);
    let from_date = s.comics.setting("weekly_from_date", "").unwrap_or_default();
    let check_interval_secs = s
        .comics
        .setting("comics_check_interval", "604800")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(604800)
        .max(0);
    let last_check_ts = s
        .comics
        .setting("last_comics_check_ts", "0")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    match s.comics.weekly(100) {
        Ok(value) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"weekly_enabled":enabled,"weekly_from_date":from_date,"items":value,"check_interval_secs":check_interval_secs,"last_check_ts":last_check_ts}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
#[derive(serde::Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}
async fn browse_dir(Query(query): Query<BrowseQuery>) -> impl IntoResponse {
    let requested = query
        .path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
    let path = std::path::PathBuf::from(&requested);
    let canonical = match std::fs::canonicalize(&path) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if !canonical.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"not a directory"})),
        );
    }
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&canonical) {
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                dirs.push(child.to_string_lossy().to_string());
            }
        }
    }
    dirs.sort();
    let parent = canonical
        .parent()
        .map(|value| value.to_string_lossy().to_string());
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "path": canonical.to_string_lossy(),
            "parent": parent,
            "dirs": dirs,
        })),
    )
}
#[derive(serde::Deserialize)]
pub struct MkdirInput {
    pub path: String,
}
async fn make_directory(Json(input): Json<MkdirInput>) -> impl IntoResponse {
    let path = input.path.trim();
    if path.is_empty() || path.contains('\0') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"invalid path"})),
        );
    }
    match std::fs::create_dir_all(path) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"path":path})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
#[derive(serde::Deserialize)]
pub struct ComicWeeklySettings {
    pub enabled: bool,
    #[serde(default)]
    pub from_date: Option<String>,
}
async fn comic_weekly_settings(
    State(s): State<AppState>,
    Json(input): Json<ComicWeeklySettings>,
) -> impl IntoResponse {
    let from_date = input.from_date.unwrap_or_default().trim().to_string();
    if !from_date.is_empty() && chrono::NaiveDate::parse_from_str(&from_date, "%Y-%m-%d").is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"data weekly non valida: usare YYYY-MM-DD"}),
            ),
        );
    }
    match s
        .comics
        .set_setting("weekly_enabled", if input.enabled { "yes" } else { "no" })
    {
        Ok(()) => match s.comics.set_setting("weekly_from_date", &from_date) {
            Ok(()) => (
                StatusCode::OK,
                Json(
                    serde_json::json!({"ok":true,"weekly_enabled":input.enabled,"weekly_from_date":from_date}),
                ),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        },
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn add_comic(State(s): State<AppState>, Json(input): Json<ComicInput>) -> impl IntoResponse {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    if input.title.trim().is_empty() || input.tag_url.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"title and tag_url are required"})),
        );
    }
    match s.comics.add_monitored(
        &input.title,
        &input.tag_url,
        &input.from_date,
        &input.save_path,
    ) {
        Ok(id) => (StatusCode::OK, Json(serde_json::json!({"ok":true,"id":id}))),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn set_comic_enabled(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ComicEnabled>,
) -> impl IntoResponse {
    match s.comics.set_enabled(id, input.enabled) {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"comic not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn remove_comic(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match s.comics.remove_monitored(id) {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"comic not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
struct BackupSteps {
    path: std::path::PathBuf,
    ftp_uploaded: bool,
    ftp_error: Option<String>,
    cloud_copied: bool,
    cloud_error: Option<String>,
}

/// Creates the snapshot and runs the FTP/cloud copies, logging every step. FTP
/// and cloud failures are recorded (not fatal): the local snapshot is still
/// valid and the caller reports exactly what happened.
fn run_backup_steps(
    data_dir: &std::path::Path,
    root: &std::path::Path,
    retain: usize,
    ftp: Option<(String, String, String, String)>,
    cloud_dir: Option<String>,
) -> anyhow::Result<BackupSteps> {
    let path = backup::create_snapshot(data_dir, root, retain)?;
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    tracing::info!(path = %path.display(), size_bytes = size, "backup snapshot created");

    let (mut ftp_uploaded, mut ftp_error) = (false, None);
    if let Some((host, user, password, remote)) = ftp {
        match backup::upload_ftp(&path, &host, &user, &password, &remote) {
            Ok(()) => {
                ftp_uploaded = true;
                tracing::info!(path = %path.display(), host = %host, remote = %remote, "backup uploaded to FTP");
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(path = %path.display(), host = %host, %error, "backup FTP upload failed");
                ftp_error = Some(message);
            }
        }
    }

    let (mut cloud_copied, mut cloud_error) = (false, None);
    if let Some(directory) = cloud_dir {
        match backup::copy_snapshot(&path, std::path::Path::new(&directory)) {
            Ok(destination) => {
                cloud_copied = true;
                tracing::info!(dest = %destination.display(), "backup copied to cloud folder");
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(directory = %directory, %error, "backup cloud copy failed");
                cloud_error = Some(message);
            }
        }
    }

    Ok(BackupSteps {
        path,
        ftp_uploaded,
        ftp_error,
        cloud_copied,
        cloud_error,
    })
}

fn backup_ftp_config(cfg: &Config) -> Option<(String, String, String, String)> {
    let host = cfg
        .settings
        .get("backup_ftp_host")
        .cloned()
        .unwrap_or_default();
    let user = cfg
        .settings
        .get("backup_ftp_user")
        .cloned()
        .unwrap_or_default();
    let password = cfg
        .settings
        .get("backup_ftp_password")
        .cloned()
        .unwrap_or_default();
    let path = cfg
        .settings
        .get("backup_ftp_path")
        .cloned()
        .unwrap_or_default();
    (!host.trim().is_empty() && !user.trim().is_empty() && !password.is_empty())
        .then_some((host, user, password, path))
}

fn backup_cloud_dir(cfg: &Config) -> Option<String> {
    cfg.settings
        .get("backup_cloud_dir")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn backup_retention(cfg: &Config) -> usize {
    cfg.settings
        .get("backup_retention")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5)
        .clamp(1, 100)
}

/// Percorso del file di stato con l'ora dell'ultimo backup automatico.
fn backup_state_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(".rextto-backup-last")
}

fn load_last_backup(data_dir: &std::path::Path) -> Option<chrono::DateTime<chrono::Local>> {
    let text = std::fs::read_to_string(backup_state_path(data_dir)).ok()?;
    chrono::DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|value| value.with_timezone(&chrono::Local))
}

fn save_last_backup(data_dir: &std::path::Path, when: chrono::DateTime<chrono::Local>) {
    let _ = std::fs::write(backup_state_path(data_dir), when.to_rfc3339());
}

/// True se è il momento di eseguire un backup automatico.
///
/// `backup_schedule_at` (HH:MM) ha la precedenza: backup giornaliero a
/// quell'ora **locale**, al più uno al giorno. Altrimenti si usa l'intervallo
/// `backup_schedule_hours` (0 = disattivo), misurato dall'ultimo backup.
fn backup_due(cfg: &Config, last: Option<chrono::DateTime<chrono::Local>>) -> bool {
    use chrono::{Local, NaiveTime};
    if let Some(at) = cfg
        .settings
        .get("backup_schedule_at")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        // Formato non valido: ignora l'orario e ricadi sull'intervallo in ore.
        if let Ok(time) = NaiveTime::parse_from_str(at, "%H:%M") {
            let now = Local::now();
            let already_today = last
                .map(|value| value.date_naive() == now.date_naive())
                .unwrap_or(false);
            return !already_today && now.time() >= time;
        }
    }
    let hours = cfg
        .settings
        .get("backup_schedule_hours")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if hours <= 0 {
        return false;
    }
    match last {
        None => true,
        Some(value) => (Local::now() - value).num_seconds() >= hours * 3600,
    }
}

/// Esegue un backup automatico: snapshot locale, FTP, copia cloud e notifiche.
async fn execute_scheduled_backup(cfg: &Config, notifier: &Notifier) -> anyhow::Result<()> {
    let data_dir = cfg.data_dir.clone();
    let root = data_dir.join("backups");
    let retain = backup_retention(cfg);
    let ftp = backup_ftp_config(cfg);
    let cloud_dir = backup_cloud_dir(cfg);
    let send_telegram = cfg
        .settings
        .get("backup_send_telegram")
        .is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1"));
    let steps = tokio::task::spawn_blocking(move || {
        run_backup_steps(&data_dir, &root, retain, ftp, cloud_dir)
    })
    .await??;
    let telegram_uploaded = if send_telegram {
        notifier
            .notify_backup_document(
                &steps.path,
                &format!(
                    "Rextto backup: {}",
                    steps
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("snapshot.zip")
                ),
            )
            .await
            .unwrap_or(false)
    } else {
        false
    };
    tracing::info!(
        path = %steps.path.display(),
        ftp_uploaded = steps.ftp_uploaded,
        ftp_error = %steps.ftp_error.as_deref().unwrap_or("none"),
        cloud_copied = steps.cloud_copied,
        cloud_error = %steps.cloud_error.as_deref().unwrap_or("none"),
        telegram_uploaded,
        "scheduled backup completed"
    );
    let _ = notifier
        .notify_event(
            "backup_completed",
            serde_json::json!({
                "path": steps.path,
                "scheduled": true,
                "ftp_uploaded": steps.ftp_uploaded,
                "ftp_error": steps.ftp_error,
                "cloud_copied": steps.cloud_copied,
                "cloud_error": steps.cloud_error,
                "telegram_uploaded": telegram_uploaded,
            }),
        )
        .await;
    Ok(())
}

/// Worker dedicato ai backup automatici: controlla ogni minuto, così l'orario
/// giornaliero è preciso (il ciclo dello scheduler gira solo ogni
/// `refresh_secs`). Persiste l'ora dell'ultimo backup per non ripeterlo ad ogni
/// riavvio.
async fn backup_worker(state: AppState) {
    let mut last = load_last_backup(&state.cfg.data_dir);
    loop {
        let cfg = match Config::load(&state.config_path) {
            Ok(cfg) => cfg,
            Err(error) => {
                tracing::warn!(%error, "backup scheduler config reload failed");
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
        };
        if cfg.active && backup_due(&cfg, last) {
            let notifier = Notifier::from_config(&cfg);
            match execute_scheduled_backup(&cfg, &notifier).await {
                Ok(()) => {
                    let now = chrono::Local::now();
                    last = Some(now);
                    save_last_backup(&cfg.data_dir, now);
                }
                Err(error) => tracing::error!(%error, "scheduled backup failed"),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn create_backup(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let notifier = Notifier::from_config(&cfg);
    let data_dir = cfg.data_dir.clone();
    let root = data_dir.join("backups");
    let retain = backup_retention(&cfg);
    let ftp = backup_ftp_config(&cfg);
    let cloud_dir = backup_cloud_dir(&cfg);
    let send_telegram = cfg
        .settings
        .get("backup_send_telegram")
        .is_some_and(|value| matches!(value.as_str(), "yes" | "true" | "1"));
    let result = tokio::task::spawn_blocking(move || {
        run_backup_steps(&data_dir, &root, retain, ftp, cloud_dir)
    })
    .await;
    match result {
        Ok(Ok(steps)) => {
            let telegram_uploaded = if send_telegram {
                let uploaded = notifier
                    .notify_backup_document(
                        &steps.path,
                        &format!(
                            "Rextto backup: {}",
                            steps
                                .path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("snapshot.zip")
                        ),
                    )
                    .await
                    .unwrap_or(false);
                tracing::info!(uploaded, "backup Telegram send completed");
                uploaded
            } else {
                false
            };
            tracing::info!(
                path = %steps.path.display(),
                ftp_uploaded = steps.ftp_uploaded,
                ftp_error = %steps.ftp_error.as_deref().unwrap_or("none"),
                cloud_copied = steps.cloud_copied,
                cloud_error = %steps.cloud_error.as_deref().unwrap_or("none"),
                telegram_uploaded,
                "backup completed"
            );
            let _ = notifier
                .notify_event(
                    "backup_completed",
                    serde_json::json!({
                        "path": steps.path,
                        "ftp_uploaded": steps.ftp_uploaded,
                        "ftp_error": steps.ftp_error,
                        "cloud_copied": steps.cloud_copied,
                        "cloud_error": steps.cloud_error,
                        "telegram_uploaded": telegram_uploaded,
                    }),
                )
                .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "path": steps.path,
                    "ftp_uploaded": steps.ftp_uploaded,
                    "ftp_error": steps.ftp_error,
                    "cloud_copied": steps.cloud_copied,
                    "cloud_error": steps.cloud_error,
                    "telegram_uploaded": telegram_uploaded,
                })),
            )
        }
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
/// Data locale leggibile di un backup. Per i vecchi nomi `snapshot-<epoch>.zip`
/// usa l'istante nel nome (più affidabile se il file è stato copiato), altrimenti
/// ricade sulla data di modifica del file.
fn backup_label(name: &str, modified: Option<u64>) -> String {
    use chrono::TimeZone;
    let from_name = name
        .strip_prefix("snapshot-")
        .and_then(|rest| rest.strip_suffix(".zip"))
        .and_then(|value| value.parse::<i64>().ok());
    let seconds = from_name.or_else(|| modified.map(|value| value as i64));
    seconds
        .and_then(|value| chrono::Local.timestamp_opt(value, 0).single())
        .map(|value| value.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_else(|| "data sconosciuta".to_string())
}
async fn list_backups(State(s): State<AppState>) -> Json<serde_json::Value> {
    let root = s.cfg.data_dir.join("backups");
    let mut items = std::fs::read_dir(&root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "zip")
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs());
            Some(serde_json::json!({
                "name": name,
                "path": entry.path(),
                "size_bytes": metadata.len(),
                "modified": modified,
                "label": backup_label(&name, modified),
            }))
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        let modified = |item: &serde_json::Value| {
            item.get("modified").and_then(serde_json::Value::as_u64)
        };
        modified(right).cmp(&modified(left)).then_with(|| {
            right
                .get("name")
                .and_then(|value| value.as_str())
                .cmp(&left.get("name").and_then(|value| value.as_str()))
        })
    });
    Json(serde_json::json!({"items":items}))
}
async fn setup_import(State(s): State<AppState>) -> impl IntoResponse {
    if setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"setup already completed"})),
        );
    }
    let source = s.cfg.import_source_dir.clone();
    if !source.join("extto_series.db").is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"ok":false,"error":"copy extto_series.db, extto_archive.db, extto_config.db and comics.db into import-source first"}),
            ),
        );
    }
    let data_dir = s.cfg.data_dir.clone();
    let imported =
        tokio::task::spawn_blocking(move || importer::import_extto(&source, &data_dir)).await;
    match imported {
        Ok(Ok(report)) => match complete_setup(&s.cfg) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"report":report})),
            ),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":e.to_string()})),
            ),
        },
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":e.to_string()})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":e.to_string()})),
        ),
    }
}
async fn setup_complete_existing(State(s): State<AppState>) -> impl IntoResponse {
    match complete_setup(&s.cfg) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":e.to_string()})),
        ),
    }
}
async fn torrents(State(s): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let live = s.torrents.list();
    if live.is_empty() && s.cfg.dry_run {
        return Json(decorate_torrents(&s, dry_run_session_preview(&s)));
    }
    Json(decorate_torrents(&s, live))
}

/// Adds UI-only flags (e.g. whether the completed files were archived on the NAS).
fn decorate_torrents(
    s: &AppState,
    items: Vec<crate::models::TorrentView>,
) -> Vec<serde_json::Value> {
    let db = s.db.lock().unwrap();
    items
        .into_iter()
        .map(|torrent| {
            let (processed_path, source, reason) = db
                .torrent_aux(&torrent.hash)
                .unwrap_or_default();
            let archived = !processed_path.trim().is_empty();
            let mut value = serde_json::to_value(&torrent).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("archived".into(), serde_json::Value::Bool(archived));
                // Sorgente (indexer/RSS/web) e motivo del download: mostrati
                // sotto il nome del file nella Sessione torrent.
                object.insert("source".into(), serde_json::Value::String(source));
                object.insert("reason".into(), serde_json::Value::String(reason));
            }
            value
        })
        .collect()
}
fn dry_run_session_preview(s: &AppState) -> Vec<crate::models::TorrentView> {
    let hashes: HashSet<String> = std::fs::read_dir(&s.cfg.state_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "fastresume")
        })
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase())
        })
        .collect();
    if hashes.is_empty() {
        return Vec::new();
    }
    s.db.lock()
        .unwrap()
        .stored_torrents_all(500)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| hashes.contains(&item.hash.to_ascii_lowercase()))
        .map(|item| crate::models::TorrentView {
            hash: item.hash,
            name: item.name,
            progress: (item.progress * 100.0).clamp(0.0, 100.0),
            state: if item.paused {
                "restored · in pausa"
            } else {
                "restored · dry-run"
            }
            .into(),
            download_rate: 0,
            upload_rate: 0,
            save_path: s.cfg.libtorrent_dir.display().to_string(),
            download_limit: -1,
            upload_limit: -1,
            all_time_upload: 0,
            all_time_download: item.downloaded,
            seeding_seconds: 0,
            queue_position: 0,
            num_peers: 0,
            num_seeds: 0,
            seed_ratio: -1.0,
            seed_days: -1,
            has_metadata: true,
            auto_managed: false,
            torrent_version: String::new(),
            total_size: 0,
            total_done: 0,
        })
        .collect()
}
async fn torrent_stats(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok":true,"stats":s.torrents.stats()}))
}
async fn torrent_history(
    State(s): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(10).clamp(1, 200);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;
    let query = query.q.unwrap_or_default();
    match s.db.lock().unwrap().completed_torrents(offset, limit, &query) {
        Ok((items, total)) => {
            let pages = ((total as usize + limit - 1) / limit).max(1);
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"items":items,"total":total,"page":page,"pages":pages})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn archive_entries(
    State(s): State<AppState>,
    Query(query): Query<ArchiveQuery>,
) -> impl IntoResponse {
    let term = query.q.or(query.query).unwrap_or_default();
    match s.archive.lock().unwrap().browse_page(
        &term,
        query.page.unwrap_or(1),
        query.limit.unwrap_or(200),
    ) {
        Ok(page) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"success":true,"items":page.items,"total":page.total,"page":page.page,"pages":page.pages}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn add_archive_entry(
    State(s): State<AppState>,
    Json(input): Json<ArchiveAddInput>,
) -> impl IntoResponse {
    let Some(release) = crate::parser::parse_release(
        &input.title,
        &input.magnet,
        if input.source.trim().is_empty() {
            "archive"
        } else {
            &input.source
        },
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"release non riconosciuta"})),
        );
    };
    add_release(&s, release)
}
async fn delete_archive_entry(
    State(s): State<AppState>,
    Json(input): Json<ArchiveDeleteInput>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify archive"})),
        );
    }
    if input.ids.is_empty() && input.magnet.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"magnet or ids is required"})),
        );
    }
    let result = if !input.ids.is_empty() {
        s.archive.lock().unwrap().delete_ids(&input.ids)
    } else {
        s.archive
            .lock()
            .unwrap()
            .delete(input.magnet.trim())
            .map(|deleted| usize::from(deleted))
    };
    match result {
        Ok(deleted) if deleted > 0 => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"deleted":deleted})),
        ),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"archive entry not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn batch_archive_download(
    State(s): State<AppState>,
    Json(input): Json<ArchiveBatchInput>,
) -> impl IntoResponse {
    if input.items.is_empty() || input.items.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"batch must contain 1-100 releases"})),
        );
    }
    let mut accepted = 0usize;
    let mut rejected = Vec::new();
    for item in input.items {
        let Some(release) = crate::parser::parse_release(
            &item.title,
            &item.magnet,
            if item.source.trim().is_empty() {
                "archive"
            } else {
                &item.source
            },
        ) else {
            rejected
                .push(serde_json::json!({"title":item.title,"reason":"release non riconosciuta"}));
            continue;
        };
        let (status, _) = add_release(&s, release);
        if status.is_success() || status == StatusCode::ACCEPTED {
            accepted += 1;
        } else {
            rejected.push(serde_json::json!({"title":item.title,"status":status.as_u16()}));
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"accepted":accepted,"rejected":rejected})),
    )
}
async fn blocklist_entries(State(s): State<AppState>) -> impl IntoResponse {
    match s.db.lock().unwrap().blocklist_entries(500) {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":items})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn remove_blocklist_entry(
    State(s): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    match s.db.lock().unwrap().remove_blocklist(&hash) {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"hash not blocklisted"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn mark_torrent_failed(
    State(s): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    let metadata = match s.db.lock().unwrap().torrent_meta(&hash) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if let Some(metadata) = metadata {
        let _ =
            s.db.lock()
                .unwrap()
                .blocklist(&metadata.release, "manual_failed");
    }
    let restored = s.db.lock().unwrap().restore_upgrade(&hash).unwrap_or(false);
    let _ =
        s.db.lock()
            .unwrap()
            .mark_torrent_error(&hash, "manual failure");
    let removed = s
        .torrents
        .list()
        .iter()
        .any(|torrent| torrent.hash.eq_ignore_ascii_case(&hash));
    remove_failed_torrent(&s.torrents, &hash);
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"removed":removed,"upgrade_restored":restored})),
    )
}
/// Removes trash contents. When `older_than_days` is positive, only entries
/// whose modification time is older than that many days are deleted.
fn remove_trash_contents(root: &FsPath, older_than_days: i64) -> anyhow::Result<(usize, u64)> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    if !root.is_dir() {
        return Ok((0, 0));
    }
    let cutoff = if older_than_days > 0 {
        SystemTime::now().checked_sub(Duration::from_secs(older_than_days as u64 * 86_400))
    } else {
        None
    };
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            let (nested_files, nested_bytes) = remove_trash_contents(&path, older_than_days)?;
            files += nested_files;
            bytes += nested_bytes;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        } else {
            if let Some(cutoff) = cutoff {
                let too_new = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .map(|modified| modified > cutoff)
                    .unwrap_or(false);
                if too_new {
                    continue;
                }
            }
            bytes = bytes.saturating_add(entry.metadata().map(|value| value.len()).unwrap_or(0));
            files += 1;
            std::fs::remove_file(path)?;
        }
    }
    Ok((files, bytes))
}
#[derive(serde::Deserialize, Default)]
pub struct DuplicatesInput {
    /// `true` = sposta in trash; assente/`false` = anteprima.
    #[serde(default)]
    pub execute: bool,
}

/// Percorsi assoluti dei file appartenti ai torrent attualmente in sessione:
/// la pulizia dei duplicati non deve mai toccarli (romperebbe seed/download).
fn protected_torrent_paths(
    torrents: &crate::libtorrent::LibtorrentClient,
) -> std::collections::HashSet<std::path::PathBuf> {
    let mut protected = std::collections::HashSet::new();
    for torrent in torrents.list() {
        let base = std::path::Path::new(&torrent.save_path);
        if let Ok(Some(files)) = torrents.files(&torrent.hash) {
            for file in files {
                protected.insert(base.join(&file.path));
            }
        }
    }
    protected
}

/// Anteprima o pulizia dei duplicati video inferiori (risoluzione più bassa)
/// nella libreria delle serie. Conservativo: non tocca la stessa risoluzione.
async fn clean_duplicates(
    State(s): State<AppState>,
    input: Option<Json<DuplicatesInput>>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let execute = input.map(|Json(input)| input.execute).unwrap_or(false);
    let protected = protected_torrent_paths(&s.torrents);
    let preferred = cfg.default_language();
    let mut candidates = Vec::new();
    let mut removed = 0usize;
    for series in cfg
        .series
        .iter()
        .filter(|series| !series.archive_path.trim().is_empty())
    {
        let directory = FsPath::new(&series.archive_path);
        if execute {
            match crate::cleaner::cleanup_inferior_duplicates_in_dir(
                &cfg,
                &series.name,
                directory,
                &protected,
            ) {
                Ok(count) => removed += count,
                Err(error) => tracing::warn!(series=%series.name, %error, "duplicate cleanup failed"),
            }
        } else {
            match crate::cleaner::find_inferior_duplicates_in_dir(
                &series.name,
                directory,
                &protected,
                &preferred,
            ) {
                Ok(found) => candidates.extend(found),
                Err(error) => tracing::warn!(series=%series.name, %error, "duplicate scan failed"),
            }
        }
    }
    if execute {
        (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"execute":true,"removed":removed})),
        )
    } else {
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"execute":false,"count":candidates.len(),"items":candidates}),
            ),
        )
    }
}

#[derive(serde::Deserialize, Default)]
pub struct RestoreSourceInput {
    #[serde(default)]
    pub execute: bool,
}

/// Ripristina il token sorgente nei nomi dei file archiviati che l'hanno perso,
/// recuperandolo dal titolo originale della release nel DB (non lo inventa).
async fn restore_source(
    State(s): State<AppState>,
    input: Option<Json<RestoreSourceInput>>,
) -> impl IntoResponse {
    let execute = input.map(|Json(input)| input.execute).unwrap_or(false);
    if execute && s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not modify files"})),
        );
    }
    let cfg = latest_config(&s);
    let mut items = Vec::new();
    let mut renamed = 0usize;
    let mut errors = 0usize;
    let mut series_count = 0usize;
    for series in cfg
        .series
        .iter()
        .filter(|series| series.enabled && !series.archive_path.trim().is_empty())
    {
        series_count += 1;
        let episodes = s
            .db
            .lock()
            .unwrap()
            .episodes_for_series(&series.name, &series.ignored_seasons)
            .unwrap_or_default();
        for episode in episodes {
            let Some(archive_path) = episode
                .archive_path
                .clone()
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            let source = crate::parser::parse_quality(&episode.title).source;
            if source.trim().is_empty() || source.eq_ignore_ascii_case("unknown") {
                continue;
            }
            let path = FsPath::new(&archive_path);
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(new_name) = postprocess::restore_source_token(name, &source) else {
                continue;
            };
            let target = path.parent().unwrap_or_else(|| FsPath::new(".")).join(&new_name);
            if target == path {
                continue;
            }
            if execute {
                if let Err(error) = postprocess::rename_sidecars(path, &target, &cfg) {
                    tracing::warn!(%error, "restore source: sidecar rename failed");
                }
                match std::fs::rename(path, &target) {
                    Ok(()) => {
                        let _ = s.db.lock().unwrap().set_episode_archive_path(
                            &series.name,
                            episode.season,
                            episode.episode,
                            &target.display().to_string(),
                        );
                        renamed += 1;
                        items.push(serde_json::json!({"series": series.name, "season": episode.season, "episode": episode.episode, "from": archive_path, "to": target.display().to_string()}));
                    }
                    Err(error) => {
                        errors += 1;
                        tracing::warn!(%error, "restore source: rename failed");
                    }
                }
            } else {
                items.push(serde_json::json!({"series": series.name, "season": episode.season, "episode": episode.episode, "from": archive_path, "to": target.display().to_string()}));
            }
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "execute": execute,
            "series": series_count,
            "renamed": renamed,
            "errors": errors,
            "count": items.len(),
            "items": items,
        })),
    )
}

async fn clean_trash(
    State(s): State<AppState>,
    input: Option<Json<CleanTrashInput>>,
) -> impl IntoResponse {
    if s.cfg.dry_run {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"dry-run does not delete trash"})),
        );
    }
    let Some(root) = s.cfg.trash_path.as_deref() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"trash path is not configured"})),
        );
    };
    let force = input.map(|Json(value)| value.force).unwrap_or(false);
    let retention_days = if force {
        0
    } else {
        latest_config(&s)
            .settings
            .get("trash_retention_days")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0)
    };
    match remove_trash_contents(root, retention_days) {
        Ok((files, bytes)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"files":files,"bytes":bytes,"retention_days":retention_days}),
            ),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
/// Elenca i gruppi "visti nei feed" (film o serie) con paginazione.
fn seen_grouped(
    state: &AppState,
    kind: &str,
    query: &SeenQuery,
) -> (StatusCode, Json<serde_json::Value>) {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;
    let query_text = query.q.clone().unwrap_or_default();
    let result = {
        let db = state.db.lock().unwrap();
        if kind == "series" {
            db.series_seen_grouped(offset, limit, &query_text)
        } else {
            db.movies_seen_grouped(offset, limit, &query_text)
        }
    };
    match result {
        Ok((groups, total)) => {
            let pages = ((total as usize + limit - 1) / limit).max(1);
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"groups":groups,"total":total,"page":page,"pages":pages})),
            )
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

/// Release di un singolo gruppo "visto" (film o serie).
fn seen_entries(
    state: &AppState,
    kind: &str,
    query: &SeenQuery,
) -> (StatusCode, Json<serde_json::Value>) {
    let group = query
        .key
        .clone()
        .or_else(|| query.group.clone())
        .unwrap_or_default();
    if group.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"key required"})),
        );
    }
    match state
        .db
        .lock()
        .unwrap()
        .seen_by_group(kind, group.trim(), query.limit.unwrap_or(500))
    {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"items":items})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn movies_seen_grouped_view(
    State(s): State<AppState>,
    Query(query): Query<SeenQuery>,
) -> impl IntoResponse {
    seen_grouped(&s, "movie", &query)
}

async fn series_seen_grouped_view(
    State(s): State<AppState>,
    Query(query): Query<SeenQuery>,
) -> impl IntoResponse {
    seen_grouped(&s, "series", &query)
}

async fn movies_seen_view(
    State(s): State<AppState>,
    Query(query): Query<SeenQuery>,
) -> impl IntoResponse {
    seen_entries(&s, "movie", &query)
}

async fn series_seen_view(
    State(s): State<AppState>,
    Query(query): Query<SeenQuery>,
) -> impl IntoResponse {
    seen_entries(&s, "series", &query)
}

/// Lists the torrents explicitly excluded from renaming (the per-torrent
/// `no_rename` flag), so the UI can show them in one place.
async fn torrent_no_rename_list(State(s): State<AppState>) -> impl IntoResponse {
    match s.db.lock().unwrap().no_rename_torrents() {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "torrents": items
                    .into_iter()
                    .map(|(hash, name)| serde_json::json!({"hash": hash, "name": name}))
                    .collect::<Vec<_>>(),
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn torrent_events(State(s): State<AppState>) -> Json<Vec<crate::models::TorrentEvent>> {
    // Return a snapshot (do not drain): the Activity page polls periodically and
    // a drained buffer left it empty whenever no event happened since the last
    // poll.
    let events = s.torrent_events.lock().unwrap();
    let start = events.len().saturating_sub(100);
    Json(events[start..].to_vec())
}
async fn add_torrent(
    State(s): State<AppState>,
    Json(input): Json<AddTorrent>,
) -> impl IntoResponse {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    match s.torrents.add(&input.magnet, &s.cfg) {
        Ok(true) => (StatusCode::ACCEPTED, Json(serde_json::json!({"ok":true}))),
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"duplicate"})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":e.to_string()})),
        ),
    }
}
async fn manual_search(
    State(s): State<AppState>,
    Json(input): Json<SearchInput>,
) -> impl IntoResponse {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    let query = input.query.trim();
    if query.is_empty() || query.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"query must contain 1-256 characters"})),
        );
    }
    let _cycle_guard = s.cycle_lock.lock().await;
    let mut results = s.engine.search_query(&s.cfg, query).await;
    for (title, magnet, source) in s.archive.lock().unwrap().search(query).unwrap_or_default() {
        if let Some(release) =
            crate::parser::parse_release(&title, &magnet, &format!("archive:{source}"))
        {
            if s.cfg.release_allowed(&release) {
                results.push(release);
            }
        }
    }
    let mut seen = HashSet::new();
    results.retain(|release| {
        crate::utils::magnet_hash(&release.magnet).is_some_and(|hash| seen.insert(hash))
    });
    results.sort_by_key(|release| {
        std::cmp::Reverse(release.quality.score_with_settings(&s.cfg.settings))
    });
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok":true,"query":query,"results":results})),
    )
}
async fn manual_search_get(
    State(s): State<AppState>,
    Query(input): Query<ManualSearchQuery>,
) -> impl IntoResponse {
    let query = input.q.or(input.query).unwrap_or_default();
    manual_search(State(s), Json(SearchInput { query })).await
}
#[derive(serde::Deserialize)]
pub struct TvdbQuery {
    pub query: String,
}

async fn tvdb_search(State(s): State<AppState>, Json(input): Json<TvdbQuery>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let client = crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
    if !client.configured() {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(serde_json::json!({"ok":false,"error":"TVDB API key non configurata"})),
        );
    }
    let query = input.query.trim();
    if query.is_empty() || query.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"query must contain 1-256 characters"})),
        );
    }
    match tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, client.search_series(query)).await {
        Ok(Ok(results)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"results":results})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("TVDB non ha risposto entro {}s", EXTERNAL_SEARCH_TIMEOUT.as_secs())
            })),
        ),
    }
}

async fn tvdb_series(State(s): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let client = crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
    if !client.configured() {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(serde_json::json!({"ok":false,"error":"TVDB API key non configurata"})),
        );
    }
    match client.series_extended(id).await {
        Ok(series) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"series":series})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

/// Tetto massimo per le ricerche su servizi esterni avviate dalla UI (TMDB,
/// fallback TVDB). Evita che un upstream lento tenga bloccata la richiesta.
const EXTERNAL_SEARCH_TIMEOUT: Duration = Duration::from_secs(12);

async fn tmdb_search(State(s): State<AppState>, Json(input): Json<TmdbQuery>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(api_key) = cfg.tmdb_api_key.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
        );
    };
    let query = input.query.trim();
    if query.is_empty() || query.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"query must contain 1-256 characters"})),
        );
    }
    let is_movie = input.kind.eq_ignore_ascii_case("movie");
    let tmdb = TmdbClient::new(Some(api_key));
    let lookup = async {
        if is_movie {
            tmdb.search_movies(query).await
        } else {
            tmdb.search_series(query).await
        }
    };
    // Limite rigido: il client TMDB può attendere fino a 20 s e ritentare, quindi
    // senza questo tetto la richiesta (e la pagina che l'ha avviata) poteva
    // restare in attesa molto a lungo.
    let result = match tokio::time::timeout(EXTERNAL_SEARCH_TIMEOUT, lookup).await {
        Ok(result) => result,
        Err(_) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({
                    "ok": false,
                    "error": format!(
                        "TMDB non ha risposto entro {}s",
                        EXTERNAL_SEARCH_TIMEOUT.as_secs()
                    )
                })),
            )
        }
    };
    match result {
        Ok(items) if !items.is_empty() => {
            let items = items
                .into_iter()
                .map(|item| {
                    let mut value = serde_json::to_value(&item).unwrap_or_default();
                    value["external"] = serde_json::Value::String("tmdb".into());
                    value
                })
                .collect::<Vec<_>>();
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"ok":true,"kind":input.kind,"items":items,"source":"tmdb"}),
                ),
            )
        }
        // Fallback TVDB (solo serie) quando TMDB non trova nulla: resta
        // "dietro le quinte", l'utente vede un solo pulsante "Cerca su TMDB".
        Ok(_) if is_movie => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"kind":input.kind,"items":[],"source":"tmdb"})),
        ),
        Ok(_) => {
            let tvdb =
                crate::tvdb::TvdbClient::with_language(cfg.tvdb_api_key(), cfg.tvdb_language());
            let items = match tokio::time::timeout(
                EXTERNAL_SEARCH_TIMEOUT,
                tvdb.search_series(query),
            )
            .await
            {
                Ok(Ok(results)) => results
                    .into_iter()
                    .map(|item| {
                        serde_json::json!({
                            "id": 0,
                            "name": item.get("name"),
                            "overview": item.get("overview"),
                            "poster": item.get("image"),
                            "first_air_date": item.get("year"),
                            "external": "tvdb",
                            "tvdb_id": item.get("tvdb_id"),
                        })
                    })
                    .collect::<Vec<_>>(),
                Ok(Err(error)) => {
                    tracing::debug!(%error, "TVDB fallback search failed");
                    Vec::new()
                }
                Err(_) => {
                    tracing::debug!("TVDB fallback search timed out");
                    Vec::new()
                }
            };
            let source = if items.is_empty() { "tmdb" } else { "tvdb" };
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"kind":"series","items":items,"source":source})),
            )
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn tmdb_discover(
    State(s): State<AppState>,
    input: Option<Json<DiscoverInput>>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let Some(api_key) = cfg.tmdb_api_key.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"TMDB API key is not configured"})),
        );
    };
    let input = input.map(|Json(input)| input).unwrap_or_default();
    let kind = input.kind.unwrap_or_else(|| "series".into());
    let window = input.window.unwrap_or_else(|| "week".into());
    let tmdb = TmdbClient::with_language(Some(api_key), cfg.tmdb_language());
    let result = match input.mode.as_deref() {
        Some("popular") => tmdb.popular(&kind).await,
        Some("top_rated" | "now_playing" | "upcoming") => {
            tmdb.category(&kind, input.mode.as_deref().unwrap_or_default())
                .await
        }
        Some("trending") | None => tmdb.trending(&kind, &window).await,
        Some(mode) => Err(anyhow::anyhow!("unsupported TMDB discovery mode: {mode}")),
    };
    match result {
        Ok(items) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"kind":kind,"items":items})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn tmdb_add(State(s): State<AppState>, Json(input): Json<TmdbAddInput>) -> impl IntoResponse {
    if input.name.trim().is_empty() || input.tmdb_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"TMDB item is incomplete"})),
        );
    }
    let mut cfg = match Config::load(&s.config_path) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if input.kind.eq_ignore_ascii_case("movie") {
        if cfg.movies.iter().any(|movie| {
            movie.name.eq_ignore_ascii_case(input.name.trim()) && movie.year == input.year
        }) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"film già presente"})),
            );
        }
        cfg.movies.push(MovieConfig {
            id: 0,
            name: input.name.trim().into(),
            year: input.year.trim().into(),
            tmdb_id: input.tmdb_id.trim().into(),
            tvdb_id: String::new(),
            original_title: String::new(),
            overview: String::new(),
            poster_path: String::new(),
            quality: input.quality.trim().into(),
            language: if input.language.trim().is_empty() {
                "ita".into()
            } else {
                input.language.trim().into()
            },
            enabled: true,
            subtitle: String::new(),
            exclude: input.exclude.trim().into(),
            language_requirements: String::new(),
            subtitle_requirements: String::new(),
        });
    } else {
        if cfg
            .series
            .iter()
            .any(|series| series.name.eq_ignore_ascii_case(input.name.trim()))
        {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"serie già presente"})),
            );
        }
        cfg.series.push(SeriesConfig {
            name: input.name.trim().into(),
            seasons: if input.seasons.trim().is_empty() {
                "1+".into()
            } else {
                input.seasons.trim().into()
            },
            quality: input.quality.trim().into(),
            language: if input.language.trim().is_empty() {
                "ita".into()
            } else {
                input.language.trim().into()
            },
            archive_path: input.archive_path.trim().into(),
            timeframe: 0,
            aliases: Vec::new(),
            tmdb_id: input.tmdb_id.trim().into(),
            tvdb_id: input.tvdb_id.trim().into(),
            subtitle: String::new(),
            exclude: input.exclude.trim().into(),
            enabled: true,
            ignored_seasons: Vec::new(),
            season_subfolders: false,
        });
    }
    match Config::save_library(&s.cfg.data_dir, &cfg.series, &cfg.movies) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"name":input.name,"restart_required":true})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn add_search_result(
    State(s): State<AppState>,
    Json(input): Json<SearchAddInput>,
) -> impl IntoResponse {
    add_release(&s, input.release)
}
fn add_release(s: &AppState, mut release: Release) -> (StatusCode, Json<serde_json::Value>) {
    if !setup_complete(&s.cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    if !s.cfg.release_allowed(&release) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"ok":false,"error":"release esclusa dai filtri globali"})),
        );
    }
    let approved = if release.kind == "series" {
        let Some(series) = release
            .series
            .as_deref()
            .and_then(|name| s.cfg.find_series_match(name, release.season))
        else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"ok":false,"error":"serie non monitorata"})),
            );
        };
        if !Config::series_release_allowed(series, &release.quality, &release.title) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(
                    serde_json::json!({"ok":false,"error":"release non compatibile con la configurazione"}),
                ),
            );
        }
        release.series = Some(series.name.clone());
        let resolved_archive = s.cfg.resolve_archive_path(series);
        let archive_index = crate::cleaner::index_archive(
            &series.name,
            resolved_archive.as_deref().unwrap_or(FsPath::new("")),
            &s.cfg.settings,
        );
        let mut live = crate::models::LiveDownloads::default();
        for torrent in s.torrents.list() {
            live.hashes.insert(torrent.hash.to_ascii_lowercase());
            if let Some(key) = crate::parser::parse_episode_key(&torrent.name) {
                live.episodes.insert(key);
            }
        }
        let context = crate::models::ApprovalContext {
            archive: archive_index,
            live,
        };
        s.db.lock().unwrap().check_series_manual_scored(
            &release,
            release.quality.score_with_settings(&s.cfg.settings),
            s.cfg.upgrade_min_score_diff,
            &context,
        )
    } else {
        let Some(movie) = s.cfg.find_movie_match(&release.title, release.year) else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"ok":false,"error":"film non monitorato"})),
            );
        };
        if !Config::quality_allowed(
            &release.quality,
            &movie.quality,
            &movie.language,
            &movie.subtitle,
        ) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(
                    serde_json::json!({"ok":false,"error":"release non compatibile con la configurazione"}),
                ),
            );
        }
        release.title = movie.name.clone();
        release.year = movie.year.parse::<i64>().ok().or(release.year);
        s.db.lock().unwrap().check_movie_scored(
            &release,
            release.quality.score_with_settings(&s.cfg.settings),
            s.cfg.upgrade_min_score_diff,
        )
    };
    let Ok((approved, reason)) = approved else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":"database check failed"})),
        );
    };
    if !approved {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":reason})),
        );
    }
    match s.torrents.add(&release.magnet, &s.cfg) {
        Ok(true) => match s.db.lock().unwrap().register_torrent(&release) {
            Ok(()) => {
                if let Some(hash) = crate::utils::magnet_hash(&release.magnet) {
                    let _ = s
                        .db
                        .lock()
                        .unwrap()
                        .set_torrent_reason(&hash, "manual");
                }
                (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({"ok":true,"title":release.title})),
                )
            }
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            ),
        },
        Ok(false) => {
            let _ = s.db.lock().unwrap().rollback_release(&release);
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"ok":false,"error":"torrent duplicate"})),
            )
        }
        Err(error) => {
            let _ = s.db.lock().unwrap().rollback_release(&release);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    }
}
async fn pause_torrent(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    torrent_action(s.torrents.pause(&hash))
}
async fn resume_torrent(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    torrent_action(s.torrents.resume(&hash))
}
async fn restart_torrent(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    torrent_action(s.torrents.restart(&hash))
}
async fn pin_torrent(
    State(s): State<AppState>,
    Json(input): Json<TorrentHashInput>,
) -> impl IntoResponse {
    let hash = input.hash.to_ascii_lowercase();
    let _ = Config::save_setting(&s.cfg.data_dir, "libtorrent_pinned_hash", &hash);
    torrent_action(s.torrents.set_pin(&hash, true))
}
async fn unpin_torrent(State(s): State<AppState>) -> impl IntoResponse {
    let _ = Config::save_setting(&s.cfg.data_dir, "libtorrent_pinned_hash", "");
    torrent_action(Ok(true))
}
async fn set_sequential(
    State(s): State<AppState>,
    Json(input): Json<SequentialInput>,
) -> impl IntoResponse {
    let _ = Config::save_setting(
        &s.cfg.data_dir,
        "libtorrent_sequential",
        if input.enabled { "true" } else { "false" },
    );
    torrent_action(s.torrents.set_sequential(input.enabled))
}
async fn torrent_tags(State(s): State<AppState>) -> impl IntoResponse {
    let items = s.db.lock().unwrap().torrent_tags().unwrap_or_default();
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"items": items.into_iter().map(|(hash, tag)| serde_json::json!({"hash": hash, "tag": tag})).collect::<Vec<_>>()}),
        ),
    )
}
async fn set_torrent_tag(
    State(s): State<AppState>,
    Json(input): Json<TorrentTagInput>,
) -> impl IntoResponse {
    match s
        .db
        .lock()
        .unwrap()
        .set_torrent_tag(&input.hash, input.tag.trim())
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn recheck_torrent(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    torrent_action(s.torrents.force_recheck(&hash))
}
async fn reannounce_torrent(
    State(s): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    torrent_action(s.torrents.reannounce(&hash))
}
async fn move_torrent_storage(
    State(s): State<AppState>,
    Path(hash): Path<String>,
    Json(input): Json<StoragePath>,
) -> impl IntoResponse {
    if input.path.trim().is_empty() || input.path.len() > 4096 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"storage path is required"})),
        );
    }
    let cfg = latest_config(&s);
    let destination = FsPath::new(input.path.trim());
    let allowed = destination.starts_with(&cfg.libtorrent_dir)
        || cfg
            .libtorrent_temp_dir
            .as_deref()
            .is_some_and(|root| destination.starts_with(root))
        || cfg
            .archive_root
            .as_deref()
            .is_some_and(|root| destination.starts_with(root));
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(
                serde_json::json!({"ok":false,"error":"storage path must be inside a configured Rextto directory"}),
            ),
        );
    }
    torrent_action(s.torrents.move_storage(&hash, destination))
}
async fn set_torrent_limits(
    State(s): State<AppState>,
    Path(hash): Path<String>,
    Json(input): Json<TorrentLimits>,
) -> impl IntoResponse {
    torrent_action(s.torrents.set_limits(
        &hash,
        input.download_limit,
        input.upload_limit,
        input.seed_ratio.unwrap_or(-1.0),
        input.seed_days.unwrap_or(-1),
    ))
}
async fn torrent_peers(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    match s.torrents.peers(&hash) {
        Ok(Some(peers)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"peers":peers})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"torrent unavailable in dry-run"})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn torrent_trackers(
    State(s): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    match s.torrents.trackers(&hash) {
        Ok(Some(trackers)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"trackers":trackers})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"torrent unavailable in dry-run"})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn torrent_files(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    match s.torrents.files(&hash) {
        Ok(Some(files)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"files":files})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"torrent unavailable in dry-run"})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn torrent_peers_legacy(
    State(s): State<AppState>,
    Json(input): Json<TorrentHashInput>,
) -> impl IntoResponse {
    torrent_peers(State(s), Path(input.hash)).await
}
/// True se i file in download di un season pack possono essere cancellati:
/// il pack è già stato copiato in libreria (status `completed`), quindi la copia
/// in `libtorrent_dir` è ridondante. Porting di `_mark_pack_copied` del legacy.
fn torrent_files_are_disposable(db: &Database, hash: &str) -> bool {
    let Ok(Some(meta)) = db.torrent_meta(hash) else {
        return false;
    };
    if !meta.release.is_pack {
        return false;
    }
    matches!(
        db.torrent_status(hash).ok().flatten().as_deref(),
        Some("completed")
    )
}

async fn remove_torrent_legacy(
    State(s): State<AppState>,
    Json(input): Json<RemoveTorrentInput>,
) -> impl IntoResponse {
    let delete_files = input.delete_files
        || torrent_files_are_disposable(&s.db.lock().unwrap(), &input.hash);
    let removed = s.torrents.remove(&input.hash, delete_files);
    if matches!(removed, Ok(true)) {
        // Coerenza DB/sessione: il torrent non esiste più, non deve restare
        // "queued" né bloccare un nuovo tentativo, né risorgere al riavvio.
        let _ = s.db.lock().unwrap().mark_torrent_removed(&input.hash);
    }
    torrent_action(removed)
}
async fn remove_completed_torrents(
    State(s): State<AppState>,
    Json(input): Json<RemoveCompletedInput>,
) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let global_ratio = cfg
        .libtorrent
        .stop_at_ratio
        .then_some(cfg.libtorrent.seed_ratio)
        .filter(|value| *value > 0.0);
    let global_time = if cfg.libtorrent.seed_time_days > 0 {
        Some(cfg.libtorrent.seed_time_days.saturating_mul(86_400))
    } else {
        (cfg.libtorrent.seed_time_minutes > 0)
            .then_some(cfg.libtorrent.seed_time_minutes.saturating_mul(60))
    };
    let mut removed = Vec::new();
    let mut skipped = 0usize;
    for torrent in s.torrents.list() {
        let completed =
            torrent.progress >= 100.0 || matches!(torrent.state.as_str(), "finished" | "seeding");
        if !completed {
            continue;
        }
        let infinite = torrent.seed_ratio == 0.0 || torrent.seed_days == 0;
        if infinite {
            skipped += 1;
            continue;
        }
        let ratio_limit = if torrent.seed_ratio > 0.0 {
            Some(torrent.seed_ratio)
        } else {
            global_ratio
        };
        let time_limit = if torrent.seed_days > 0 {
            Some(torrent.seed_days.saturating_mul(86_400))
        } else {
            global_time
        };
        let ratio_reached = ratio_limit.is_some_and(|limit| {
            torrent.all_time_download > 0
                && torrent.all_time_upload as f64 / torrent.all_time_download as f64 >= limit
        });
        let time_reached = time_limit.is_some_and(|limit| torrent.seeding_seconds >= limit);
        if !ratio_reached && !time_reached {
            skipped += 1;
            continue;
        }
        let delete_files = input.delete_files
            || torrent_files_are_disposable(&s.db.lock().unwrap(), &torrent.hash);
        match s.torrents.remove(&torrent.hash, delete_files) {
            Ok(true) => {
                let _ = s.db.lock().unwrap().mark_torrent_removed(&torrent.hash);
                removed.push(torrent.hash)
            }
            Ok(false) => skipped += 1,
            Err(error) => {
                tracing::warn!(hash=%torrent.hash, %error, "completed torrent removal failed")
            }
        }
    }
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"success":true,"removed":removed.len(),"skipped":skipped,"items":removed}),
        ),
    )
}
async fn fetch_ipfilter(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error.to_string())?
        .to_vec();
    let lower = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if lower.ends_with(".gz") || bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut text = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut text)
            .map_err(|error| error.to_string())?;
        return Ok(text.into_bytes());
    }
    if lower.ends_with(".zip") || bytes.starts_with(b"PK\x03\x04") {
        let reader = std::io::Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(reader).map_err(|error| error.to_string())?;
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
            let name = file.name().to_ascii_lowercase();
            if name.ends_with(".p2p")
                || name.ends_with(".dat")
                || name.ends_with(".txt")
                || name.ends_with(".list")
            {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut file, &mut text)
                    .map_err(|error| error.to_string())?;
                return Ok(text.into_bytes());
            }
        }
        return Err("archivio zip senza lista riconosciuta".into());
    }
    let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(300)]).to_ascii_lowercase();
    if preview.contains("<html") || preview.contains("<body") {
        return Err("l'URL ha restituito una pagina web invece della lista".into());
    }
    Ok(bytes)
}

fn count_ipfilter_rules(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .filter(|line| {
                    let line = line.trim();
                    !line.is_empty() && !line.starts_with('#') && line.contains('-')
                })
                .count()
        })
        .unwrap_or(0)
}

async fn ipfilter_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    let cfg = latest_config(&s);
    let target = cfg.libtorrent.ip_filter_path.trim().to_string();
    let cached = cfg.data_dir.join("ipfilter.dat");
    let is_url = target.starts_with("http://") || target.starts_with("https://");
    let path = if is_url {
        cached.clone()
    } else if target.is_empty() {
        cached.clone()
    } else {
        std::path::PathBuf::from(&target)
    };
    let active = !target.is_empty() && path.exists();
    let rules = if active {
        count_ipfilter_rules(&path)
    } else {
        0
    };
    Json(serde_json::json!({
        "ok": true,
        "configured": !target.is_empty(),
        "url": target,
        "path": path.display().to_string(),
        "active": active,
        "rules": rules,
    }))
}

async fn jellyfin_refresh(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let url = cfg
        .settings
        .get("jellyfin_url")
        .cloned()
        .unwrap_or_default();
    let key = cfg
        .settings
        .get("jellyfin_api_key")
        .cloned()
        .unwrap_or_default();
    if url.trim().is_empty() || key.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"Jellyfin non configurato"})),
        );
    }
    let endpoint = format!("{}/Library/Refresh", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    match client
        .post(&endpoint)
        .header("Authorization", format!("MediaBrowser Token=\"{key}\""))
        .header("X-Emby-Token", key)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            (StatusCode::OK, Json(serde_json::json!({"ok":true})))
        }
        Ok(response) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":format!("HTTP {}", response.status())})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn plex_refresh(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let url = cfg.settings.get("plex_url").cloned().unwrap_or_default();
    let token = cfg.settings.get("plex_token").cloned().unwrap_or_default();
    if url.trim().is_empty() || token.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"Plex non configurato"})),
        );
    }
    let endpoint = format!("{}/library/sections/all/refresh", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    match client
        .get(&endpoint)
        .header("X-Plex-Token", token)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            (StatusCode::OK, Json(serde_json::json!({"ok":true})))
        }
        Ok(response) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":format!("HTTP {}", response.status())})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

/// Estrae un attributo XML semplice (`nome="valore"`) senza un parser completo:
/// serve solo a mostrare un'informazione amichevole nei test di integrazione.
fn xml_attribute(body: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Messaggio d'errore leggibile per un test di integrazione andato male.
fn media_test_error(provider: &str, status: u16) -> String {
    match status {
        401 | 403 => format!("{provider}: credenziali non valide"),
        404 => format!("{provider}: endpoint non trovato, controlla l'URL"),
        _ => format!("{provider}: HTTP {status}"),
    }
}

/// Verifica URL e API key di Jellyfin con `/System/Info` e riporta nome/versione
/// del server. Non modifica nulla.
async fn jellyfin_test(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let url = cfg.settings.get("jellyfin_url").cloned().unwrap_or_default();
    let key = cfg.settings.get("jellyfin_api_key").cloned().unwrap_or_default();
    if url.trim().is_empty() || key.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"Jellyfin non configurato"})),
        );
    }
    let endpoint = format!("{}/System/Info", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    match client
        .get(&endpoint)
        .header("Authorization", format!("MediaBrowser Token=\"{key}\""))
        .header("X-Emby-Token", key)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            let info: serde_json::Value = response.json().await.unwrap_or_default();
            let server = info
                .get("ServerName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let version = info
                .get("Version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"server":server,"version":version})),
            )
        }
        Ok(response) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":media_test_error("Jellyfin", response.status().as_u16())})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

/// Verifica URL e token di Plex con `/identity` e prova a leggere il nome del
/// server. Non avvia refresh di libreria.
async fn plex_test(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let url = cfg.settings.get("plex_url").cloned().unwrap_or_default();
    let token = cfg.settings.get("plex_token").cloned().unwrap_or_default();
    if url.trim().is_empty() || token.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"Plex non configurato"})),
        );
    }
    let base = url.trim_end_matches('/').to_string();
    let token = token.trim().to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    match client
        .get(format!("{base}/identity"))
        .header("X-Plex-Token", token.clone())
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            let identity = response.text().await.unwrap_or_default();
            let machine = xml_attribute(&identity, "machineIdentifier").unwrap_or_default();
            let server = match client
                .get(format!("{base}/"))
                .header("X-Plex-Token", token)
                .send()
                .await
            {
                Ok(root) if root.status().is_success() => root
                    .text()
                    .await
                    .map(|body| xml_attribute(&body, "friendlyName").unwrap_or_default())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok":true,"server":server,"version":"","machine":machine})),
            )
        }
        Ok(response) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":media_test_error("Plex", response.status().as_u16())})),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

/// Refresh configured media servers after an actual torrent import. legacy
/// does this automatically after move/rename; keeping it here also covers
/// season packs and completion recovered after a restart.
async fn refresh_media_libraries(cfg: &Config) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    let jellyfin_url = cfg
        .settings
        .get("jellyfin_url")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    let jellyfin_key = cfg
        .settings
        .get("jellyfin_api_key")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    if !jellyfin_url.is_empty() && !jellyfin_key.is_empty() {
        let endpoint = format!("{}/Library/Refresh", jellyfin_url.trim_end_matches('/'));
        match client
            .post(endpoint)
            .header(
                "Authorization",
                format!("MediaBrowser Token=\"{jellyfin_key}\""),
            )
            .header("X-Emby-Token", jellyfin_key)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                tracing::info!("Jellyfin library refresh requested")
            }
            Ok(response) => {
                tracing::warn!(status=%response.status(), "Jellyfin library refresh failed")
            }
            Err(error) => tracing::warn!(%error, "Jellyfin library refresh failed"),
        }
    }
    let plex_url = cfg
        .settings
        .get("plex_url")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    let plex_token = cfg
        .settings
        .get("plex_token")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    if !plex_url.is_empty() && !plex_token.is_empty() {
        let endpoint = format!(
            "{}/library/sections/all/refresh",
            plex_url.trim_end_matches('/')
        );
        match client
            .get(endpoint)
            .header("X-Plex-Token", plex_token)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                tracing::info!("Plex library refresh requested")
            }
            Ok(response) => {
                tracing::warn!(status=%response.status(), "Plex library refresh failed")
            }
            Err(error) => tracing::warn!(%error, "Plex library refresh failed"),
        }
    }
}
static FEED_STATUS_CACHE: Mutex<Option<(Instant, serde_json::Value)>> = Mutex::new(None);
/// Quante voci recenti dell'archivio considerare per il feed status.
const FEED_STATUS_WINDOW: usize = 40_000;

/// Calcola il matching fra titoli monitorati e release recenti dei feed e
/// aggiorna la cache condivisa. Separato così sia l'endpoint sia il worker
/// periodico usano la stessa logica e "Dal feed" resta fresco anche fra un
/// ciclo e l'altro.
fn compute_feed_status(s: &AppState) -> serde_json::Value {
    let cfg = latest_config(s);
    // Carica solo le voci recenti dell'archivio e fai il matching in memoria:
    // evita sia centinaia di scansioni complete sia di tenere in RAM l'intero
    // archivio (che può superare le centinaia di migliaia di righe).
    let entries: Vec<(String, String, String, String)> = s
        .archive
        .lock()
        .unwrap()
        .recent_entries(FEED_STATUS_WINDOW)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, source)| {
            !source.starts_with("archive:") && !source.starts_with("timeframe")
        })
        .map(|(title, magnet, source)| (title.to_lowercase(), title, magnet, source))
        .collect();
    let find = |query: &str| -> Vec<serde_json::Value> {
        let words = query
            .split_whitespace()
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if words.is_empty() {
            return Vec::new();
        }
        entries
            .iter()
            .filter(|(lower, _, _, _)| words.iter().all(|word| lower.contains(word.as_str())))
            .take(8)
            .map(|(_, title, magnet, source)| {
                serde_json::json!({"title": title, "magnet": magnet, "source": source})
            })
            .collect()
    };
    let mut items = Vec::new();
    for series in cfg.series.iter().filter(|series| series.enabled).take(120) {
        items.push(
            serde_json::json!({"kind": "series", "name": series.name, "matches": find(&series.name)}),
        );
    }
    for movie in cfg.movies.iter().filter(|movie| movie.enabled).take(120) {
        let query = format!("{} {}", movie.name, movie.year);
        items.push(
            serde_json::json!({"kind": "movie", "name": movie.name, "matches": find(&query)}),
        );
    }
    let value = serde_json::json!({"ok": true, "items": items});
    *FEED_STATUS_CACHE.lock().unwrap() = Some((Instant::now(), value.clone()));
    value
}

async fn feed_status(State(s): State<AppState>) -> Json<serde_json::Value> {
    // Risposta cache per un minuto: la dashboard e i tab "dal feed" la
    // richiedono ripetutamente e il calcolo tocca l'intero archivio.
    if let Some((computed_at, value)) = FEED_STATUS_CACHE.lock().unwrap().as_ref() {
        if computed_at.elapsed() < Duration::from_secs(60) {
            return Json(value.clone());
        }
    }
    Json(compute_feed_status(&s))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Builds the rolling magnet feed (legacy `generate_xml`): RSS 2.0 where each
/// item's `<link>` is the magnet, for external consumers.
fn build_magnet_feed(entries: &[(String, String, String)]) -> String {
    let mut xml = String::with_capacity(entries.len() * 256 + 256);
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<rss version=\"2.0\"><channel>");
    xml.push_str("<title>Rextto Magnet Feed</title>");
    xml.push_str("<description>Feed automatico di magnet link da Rextto</description>");
    xml.push_str(&format!(
        "<lastBuildDate>{}</lastBuildDate>",
        chrono::Utc::now().to_rfc2822()
    ));
    for (title, magnet, source) in entries {
        xml.push_str("<item><title>");
        xml.push_str(&xml_escape(title));
        xml.push_str("</title><link>");
        xml.push_str(&xml_escape(magnet));
        xml.push_str("</link><description>");
        xml.push_str(&xml_escape(title));
        if !source.is_empty() {
            xml.push_str(" [");
            xml.push_str(&xml_escape(source));
            xml.push(']');
        }
        xml.push_str("</description></item>");
    }
    xml.push_str("</channel></rss>");
    xml
}

async fn magnet_feed(State(s): State<AppState>) -> impl IntoResponse {
    let entries = s
        .archive
        .lock()
        .unwrap()
        .recent_entries(1000)
        .unwrap_or_default();
    let xml = build_magnet_feed(&entries);
    // Keep a file copy for consumers that read the path instead of HTTP, like
    // legacy's `extto_magnet_feed.xml`.
    let _ = std::fs::write(s.cfg.data_dir.join("rextto_magnet_feed.xml"), &xml);
    (
        [(axum::http::header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        xml,
    )
}

async fn apply_libtorrent_settings(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    torrent_action(s.torrents.apply_settings(&cfg))
}

/// Valori libtorrent consigliati in base a RAM/risorse. Separati dal comando,
/// così il pulsante e l'ottimizzazione continua condividono la stessa logica.
struct LibtorrentOptimization {
    changes: Vec<(&'static str, String)>,
    memory_mb: u64,
    cache_size: i64,
    cache_mb: i64,
    queue_mb: i64,
    send_buffer_kb: i64,
    peer_list: i64,
}

fn libtorrent_optimization(cfg: &Config) -> LibtorrentOptimization {
    let trash = cfg
        .trash_path
        .clone()
        .unwrap_or_else(|| cfg.data_dir.join("trash"));
    let ramdisk = cfg
        .settings
        .get("libtorrent_ramdisk_dir")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    let health = health::check_with_paths(&health::HealthPaths {
        data_dir: &cfg.data_dir,
        trash_path: &trash,
        download_path: &cfg.libtorrent_dir,
        archive_root: cfg.archive_root.as_deref(),
        ramdisk_path: ramdisk.as_deref(),
    });
    let memory_mb = health.memory_total_bytes / (1024 * 1024);
    // legacy non deriva la coda dal tetto di banda: mantiene una base stabile e
    // lascia che la coda dinamica si adatti a runtime. I suggerimenti di memoria
    // sono separati e mirano ai burst di scrittura su NFS/NAS.
    let (cache_size, queue_mb, send_buffer_kb, peer_list): (i64, i64, i64, i64) =
        if memory_mb > 0 && memory_mb < 2048 {
            (0, 8, 256, 100)
        } else if memory_mb < 4096 {
            (1024, 32, 512, 200)
        } else if memory_mb < 8192 {
            (8192, 64, 1024, 300)
        } else if memory_mb < 16384 {
            (16384, 64, 1024, 500)
        } else {
            (32768, 128, 2048, 500)
        };
    let cache_mb = cache_size * 16 / 1024;
    let active_downloads = 3;
    let active_seeds = 3;
    let active_limit = 5;
    let mut extra_settings = cfg
        .settings
        .get("libtorrent_extra_settings")
        .cloned()
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            let key = line.split('=').next().map(str::trim).unwrap_or_default();
            !matches!(
                key,
                "max_queued_disk_bytes" | "send_buffer_watermark" | "max_peerlist_size"
            )
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    extra_settings.extend([
        format!("max_queued_disk_bytes={}", queue_mb * 1024 * 1024),
        format!("send_buffer_watermark={}", send_buffer_kb * 1024),
        format!("max_peerlist_size={peer_list}"),
    ]);
    let changes = vec![
        ("libtorrent_active_downloads", active_downloads.to_string()),
        ("libtorrent_active_seeds", active_seeds.to_string()),
        ("libtorrent_active_limit", active_limit.to_string()),
        ("libtorrent_dynamic_queue", "yes".to_string()),
        ("libtorrent_dynamic_queue_min", "1".to_string()),
        ("libtorrent_dynamic_queue_max", "10".to_string()),
        ("libtorrent_cache_size", cache_size.to_string()),
        ("libtorrent_cache_expiry", "300".to_string()),
        ("libtorrent_extra_settings", extra_settings.join("\n")),
    ];
    LibtorrentOptimization {
        changes,
        memory_mb,
        cache_size,
        cache_mb,
        queue_mb,
        send_buffer_kb,
        peer_list,
    }
}

/// Applica l'ottimizzazione **solo se qualche valore è cambiato**, così il
/// worker continuo non riscrive la configurazione ad ogni giro. Ritorna
/// `(modificato, memoria_rilevata_mb)`.
async fn apply_libtorrent_optimization(s: &AppState) -> Result<(bool, u64), String> {
    let cfg = latest_config(s);
    let optimization = libtorrent_optimization(&cfg);
    let changed = optimization
        .changes
        .iter()
        .any(|(key, value)| cfg.settings.get(*key).map(String::as_str) != Some(value.as_str()));
    if !changed {
        return Ok((false, optimization.memory_mb));
    }
    for (key, value) in &optimization.changes {
        Config::save_setting(&s.cfg.data_dir, key, value).map_err(|error| format!("{key}: {error}"))?;
    }
    let optimized = latest_config(s);
    s.torrents
        .apply_settings(&optimized)
        .map_err(|error| error.to_string())?;
    Ok((true, optimization.memory_mb))
}

async fn optimize_libtorrent_settings(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let optimization = libtorrent_optimization(&cfg);
    for (key, value) in &optimization.changes {
        if let Err(error) = Config::save_setting(&s.cfg.data_dir, key, value) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":format!("{key}: {error}")})),
            );
        }
    }
    let optimized = latest_config(&s);
    if let Err(error) = s.torrents.apply_settings(&optimized) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "applied": true,
            "hardware": {"memory_mb": optimization.memory_mb},
            "settings": {"active_downloads": 3, "active_seeds": 3, "active_limit": 5, "dynamic_queue": true, "dynamic_queue_min": 1, "dynamic_queue_max": 10, "cache_blocks": optimization.cache_size, "cache_mb": optimization.cache_mb, "queue_mb": optimization.queue_mb, "send_buffer_kb": optimization.send_buffer_kb, "peer_list": optimization.peer_list},
            "explanation": format!("Base: 3 download, 3 seed, limite 5. Coda dinamica: active_downloads tra 1 e 10, a passi di uno, dopo campioni coerenti e con raffreddamento di 10 minuti. RAM rilevata: {} MB; cache: {} blocchi ({} MB), coda disco: {} MB, send-buffer: {} KiB, peer-list: {}. Connessioni e limiti globali di banda lasciati invariati.", optimization.memory_mb, optimization.cache_size, optimization.cache_mb, optimization.queue_mb, optimization.send_buffer_kb, optimization.peer_list),
            "bandwidth_limits_preserved": true,
            "connections_limit_preserved": true
        })),
    )
}

/// Privileged helper installed once by root (see `scripts/rextto-restart` and
/// `systemd/rextto.sudoers`). It lets the daemon user restart the service
/// without a password, without granting a general-purpose sudo rule.
const RESTART_HELPER: &str = "/usr/local/bin/rextto-restart";

#[derive(serde::Deserialize, Default)]
pub struct ServiceActionInput {
    #[serde(default)]
    pub action: Option<String>,
}

/// Read-only status of the systemd units Rextto cares about.
async fn services_status(State(s): State<AppState>) -> impl IntoResponse {
    // Only Rextto's own unit is relevant (legacy is not part of this stack).
    let unit = "rextto.service";
    let field = |verb: &str| {
        std::process::Command::new("systemctl")
            .args([verb, unit])
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".into())
    };
    let service = serde_json::json!({
        "unit": unit,
        "active": field("is-active"),
        "enabled": field("is-enabled"),
    });
    // Check the enabled indexers (Jackett/Prowlarr/...): that is what actually
    // matters for acquisitions.
    let cfg = latest_config(&s);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let mut indexers = Vec::new();
    for indexer in cfg.indexers.iter().filter(|indexer| indexer.enabled) {
        let status = match client.get(&indexer.url).send().await {
            Ok(response) => Some(response.status().as_u16()),
            Err(_) => None,
        };
        indexers.push(serde_json::json!({
            "name": indexer.name,
            "url": indexer.url,
            "reachable": status.is_some(),
            "status": status,
        }));
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "services": [service], "indexers": indexers})),
    )
}

async fn service_restart(input: Option<Json<ServiceActionInput>>) -> impl IntoResponse {
    let action = input
        .and_then(|Json(input)| input.action)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "restart" | "start" | "stop"))
        .unwrap_or_else(|| "restart".to_string());
    if !FsPath::new(RESTART_HELPER).exists() {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(serde_json::json!({
                "ok": false,
                "error": "Helper di riavvio non installato. Da root, una volta: \
            sudo install -m 0755 scripts/rextto-restart /usr/local/bin/rextto-restart && \
            sudo install -m 0440 systemd/rextto.sudoers /etc/sudoers.d/rextto"
            })),
        );
    }
    let allowed = std::process::Command::new("sudo")
        .args(["-n", "-l", RESTART_HELPER])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "ok": false,
                "error": "Utente non autorizzato al riavvio: installa la regola sudoers systemd/rextto.sudoers."
            })),
        );
    }
    // Apply asynchronously so this HTTP response can be flushed first.
    let requested = action.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        let _ = std::process::Command::new("sudo")
            .args(["-n", RESTART_HELPER, &requested])
            .output();
    });
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok": true, "action": action, "message": "azione sul servizio richiesta"}),
        ),
    )
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .trim_start_matches('v')
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

/// True when `candidate` is a strictly newer version than `current`.
fn version_is_newer(candidate: &str, current: &str) -> bool {
    let mut left = version_parts(candidate);
    let mut right = version_parts(current);
    while left.len() < right.len() {
        left.push(0);
    }
    while right.len() < left.len() {
        right.push(0);
    }
    left > right
}

async fn libtorrent_check_update() -> impl IntoResponse {
    let installed = crate::libtorrent::libtorrent_version();
    let latest = match fetch_latest_libtorrent_release().await {
        Ok(version) => version,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "ok": false,
                    "installed": installed,
                    "error": error
                })),
            )
        }
    };
    let apt_candidate = libtorrent_apt_candidate();
    let update_available = version_is_newer(&latest, &installed);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "installed": installed,
            "latest": latest,
            "apt_candidate": apt_candidate,
            "update_available": update_available,
            "releases_url": "https://github.com/arvidn/libtorrent/releases"
        })),
    )
}

async fn fetch_latest_libtorrent_release() -> std::result::Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("rextto-libtorrent-check")
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get("https://api.github.com/repos/arvidn/libtorrent/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub ha risposto {}", response.status()));
    }
    let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .map(|tag| tag.trim_start_matches('v').to_string())
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| "risposta GitHub senza tag_name".to_string())
}

/// Version offered by the configured apt repositories, when available.
fn libtorrent_apt_candidate() -> Option<String> {
    for package in [
        "libtorrent-rasterbar-dev",
        "libtorrent-rasterbar2.0t64",
        "libtorrent-rasterbar2.0",
    ] {
        let Ok(output) = std::process::Command::new("apt-cache")
            .args(["policy", package])
            // apt-cache is localized: force the C locale so "Candidate:" parses.
            .env("LC_ALL", "C")
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(candidate) = text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Candidate:")
                .map(|value| value.trim().to_string())
        }) {
            if !candidate.is_empty() && candidate != "(none)" {
                return Some(candidate);
            }
        }
    }
    None
}
async fn ipfilter_update(State(s): State<AppState>) -> impl IntoResponse {
    let cfg = latest_config(&s);
    let target = cfg.libtorrent.ip_filter_path.trim().to_string();
    if target.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":"Nessun IP filter configurato"})),
        );
    }
    let local_path = if target.starts_with("http://") || target.starts_with("https://") {
        match fetch_ipfilter(&target).await {
            Ok(bytes) => {
                let path = cfg.data_dir.join("ipfilter.dat");
                if let Err(error) = std::fs::write(&path, &bytes) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"ok":false,"error":error.to_string()})),
                    );
                }
                path
            }
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"ok":false,"error":error})),
                )
            }
        }
    } else {
        std::path::PathBuf::from(&target)
    };
    match s.torrents.load_ipfilter(&local_path) {
        Ok(rules) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok":true,"rules":rules,"path":local_path.display().to_string()}),
            ),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

#[derive(serde::Deserialize)]
pub struct TempLimitsInput {
    #[serde(default)]
    pub download_kib: i64,
    #[serde(default)]
    pub upload_kib: i64,
    #[serde(default)]
    pub minutes: i64,
    #[serde(default)]
    pub clear: bool,
}

async fn set_temp_limits(
    State(s): State<AppState>,
    Json(input): Json<TempLimitsInput>,
) -> impl IntoResponse {
    let dl = input.download_kib.max(0);
    let ul = input.upload_kib.max(0);
    let minutes = input.minutes.clamp(0, 24 * 60);
    let until = if input.clear || minutes == 0 {
        0
    } else {
        chrono::Utc::now().timestamp() + minutes * 60
    };
    for (key, value) in [
        ("libtorrent_temp_dl_limit", dl.to_string()),
        ("libtorrent_temp_ul_limit", ul.to_string()),
        (
            "libtorrent_temp_limit_enabled",
            if input.clear { "0".into() } else { "1".into() },
        ),
        ("libtorrent_temp_limit_until", until.to_string()),
    ] {
        if let Err(error) = Config::save_setting(&s.cfg.data_dir, key, &value) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"ok":false,"error":format!("salvataggio limite temporaneo: {error}")}),
                ),
            );
        }
    }
    let cfg = latest_config(&s);
    apply_speed_policy(&cfg, &s.torrents);
    (
        StatusCode::OK,
        Json(
            serde_json::json!({"ok":true,"until":until,"enabled":!input.clear,"permanent":!input.clear && minutes == 0,"download_kib":dl,"upload_kib":ul}),
        ),
    )
}

#[derive(serde::Deserialize)]
struct SpeedLimitsInput {
    download_kib: i64,
    upload_kib: i64,
}

async fn set_speed_limits(
    State(s): State<AppState>,
    Json(input): Json<SpeedLimitsInput>,
) -> impl IntoResponse {
    let dl = input.download_kib.max(0);
    let ul = input.upload_kib.max(0);
    if let Err(error) =
        Config::save_setting(&s.cfg.data_dir, "libtorrent_dl_limit", &dl.to_string())
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        );
    }
    let _ = Config::save_setting(&s.cfg.data_dir, "libtorrent_ul_limit", &ul.to_string());
    match s.torrents.set_global_speed_limits(dl, ul) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"download_kib":dl,"upload_kib":ul})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}
async fn set_torrent_limits_legacy(
    State(s): State<AppState>,
    Json(input): Json<TorrentLimitsInput>,
) -> impl IntoResponse {
    torrent_action(s.torrents.set_limits(
        &input.hash,
        input.dl_kbps.saturating_mul(1024),
        input.ul_kbps.saturating_mul(1024),
        input.seed_ratio.unwrap_or(-1.0),
        input.seed_days.unwrap_or(-1),
    ))
}
async fn remove_torrent(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    let delete_files = torrent_files_are_disposable(&s.db.lock().unwrap(), &hash);
    let removed = s.torrents.remove(&hash, delete_files);
    if matches!(removed, Ok(true)) {
        let db = s.db.lock().unwrap();
        let _ = db.mark_torrent_removed(&hash);
        let _ = db.forget_removed_torrent(&hash);
    }
    torrent_action(removed)
}
async fn remove_torrent_with_options(
    State(s): State<AppState>,
    Path(hash): Path<String>,
    Json(input): Json<RemoveOptionsInput>,
) -> impl IntoResponse {
    if input.blocklist {
        let release =
            s.db.lock()
                .unwrap()
                .torrent_meta(&hash)
                .ok()
                .flatten()
                .map(|meta| meta.release);
        if let Some(release) = release {
            let _ = s.db.lock().unwrap().blocklist(&release, "manual");
        }
    }
    let delete_files =
        input.delete_files || torrent_files_are_disposable(&s.db.lock().unwrap(), &hash);
    let removed = s.torrents.remove(&hash, delete_files);
    if matches!(removed, Ok(true)) {
        let db = s.db.lock().unwrap();
        let _ = db.mark_torrent_removed(&hash);
        let _ = db.forget_removed_torrent(&hash);
    }
    torrent_action(removed)
}
async fn set_torrent_no_rename(
    State(s): State<AppState>,
    Path(hash): Path<String>,
    Json(input): Json<NoRenameInput>,
) -> impl IntoResponse {
    match s
        .db
        .lock()
        .unwrap()
        .set_torrent_no_rename(&hash, input.value)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok":true,"no_rename":input.value})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok":false,"error":error.to_string()})),
        ),
    }
}

async fn torrent_details(State(s): State<AppState>, Path(hash): Path<String>) -> impl IntoResponse {
    let magnet =
        s.db.lock()
            .unwrap()
            .torrent_meta(&hash)
            .ok()
            .flatten()
            .map(|meta| meta.release.magnet)
            .unwrap_or_default();
    match s
        .torrents
        .list()
        .into_iter()
        .find(|torrent| torrent.hash.eq_ignore_ascii_case(&hash))
    {
        Some(torrent) => {
            let no_rename =
                s.db.lock()
                    .unwrap()
                    .torrent_no_rename(&hash)
                    .unwrap_or(false);
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"ok":true,"torrent":torrent,"magnet":magnet,"no_rename":no_rename}),
                ),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"torrent not found"})),
        ),
    }
}
fn torrent_action(result: anyhow::Result<bool>) -> (StatusCode, Json<serde_json::Value>) {
    match result {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({"ok":true}))),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"ok":false,"error":"torrent unavailable in dry-run"})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok":false,"error":e.to_string()})),
        ),
    }
}

fn current_speed_limits(cfg: &Config) -> (i64, i64) {
    let now_ts = chrono::Utc::now().timestamp();
    // A scheduled speed window has priority over a temporary limit. Once the
    // window ends, a permanent temporary limit becomes active again.
    if let Some(limits) = scheduled_speed_limits(cfg) {
        return limits;
    }
    let temp_until = cfg
        .settings
        .get("libtorrent_temp_limit_until")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let temp_enabled = cfg
        .settings
        .get("libtorrent_temp_limit_enabled")
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        // Backward compatibility for a previously active timed limit.
        .unwrap_or(temp_until > now_ts);
    if temp_enabled && (temp_until == 0 || temp_until > now_ts) {
        return (
            cfg.settings
                .get("libtorrent_temp_dl_limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            cfg.settings
                .get("libtorrent_temp_ul_limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        );
    }
    let base_dl = cfg.libtorrent.download_limit_kib.max(0);
    let base_ul = cfg.libtorrent.upload_limit_kib.max(0);
    (base_dl, base_ul)
}

fn scheduled_speed_limits(cfg: &Config) -> Option<(i64, i64)> {
    let enabled = matches!(
        cfg.settings
            .get("libtorrent_sched_enabled")
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("yes") | Some("true") | Some("1")
    );
    if !enabled {
        return None;
    }
    let now = chrono::Local::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1) - 1;
    let days = cfg
        .settings
        .get("libtorrent_sched_days")
        .cloned()
        .unwrap_or_default();
    let active_days: Vec<i64> = days
        .split(',')
        .filter_map(|day| day.trim().parse().ok())
        .collect();
    let day_ok = active_days.contains(&weekday);
    let parse_time = |key: &str, default: (u32, u32)| -> (u32, u32) {
        cfg.settings
            .get(key)
            .and_then(|value| {
                let mut parts = value.split(':');
                let hour = parts.next()?.trim().parse().ok()?;
                let minute = parts.next()?.trim().parse().ok()?;
                Some((hour, minute))
            })
            .unwrap_or(default)
    };
    let (start_h, start_m) = parse_time("libtorrent_sched_start", (23, 0));
    let (end_h, end_m) = parse_time("libtorrent_sched_end", (8, 0));
    let now_minutes = now.format("%H").to_string().parse::<u32>().unwrap_or(0) * 60
        + now.format("%M").to_string().parse::<u32>().unwrap_or(0);
    let start_minutes = start_h * 60 + start_m;
    let end_minutes = end_h * 60 + end_m;
    let in_time = if start_minutes <= end_minutes {
        now_minutes >= start_minutes && now_minutes < end_minutes
    } else {
        now_minutes >= start_minutes || now_minutes < end_minutes
    };
    if day_ok && in_time {
        Some((
            cfg.settings
                .get("libtorrent_sched_dl_limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            cfg.settings
                .get("libtorrent_sched_ul_limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        ))
    } else {
        None
    }
}

fn apply_speed_policy(cfg: &Config, torrents: &LibtorrentClient) {
    let (download_kib, upload_kib) = current_speed_limits(cfg);
    if let Err(error) = torrents.set_global_speed_limits(download_kib, upload_kib) {
        tracing::debug!(%error, "speed policy apply failed");
    }
}

/// Resident set size of the current process in KiB (Linux `/proc/self/statm`).
fn resident_kb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|text| {
            text.split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u64>().ok())
        })
        .map(|pages| pages.saturating_mul(4))
        .unwrap_or(0)
}

async fn torrent_event_worker(
    config_path: PathBuf,
    fallback: Config,
    torrents: Arc<LibtorrentClient>,
    db: Arc<Mutex<Database>>,
    comics: Arc<ComicsDb>,
    event_log: Arc<Mutex<Vec<TorrentEvent>>>,
) {
    let mut move_requests = HashSet::new();
    let mut metadata_wait_start = HashMap::new();
    let mut metadata_first_seen = HashMap::new();
    let mut stall_wait_start = HashMap::new();
    // Periodic RAM-disk reconciliation: the metadata event fires once, so a
    // missed event (restart/race) used to leave an oversized torrent on the
    // tmpfs forever. `ramdisk_attempts` rate-limits retries per hash.
    let mut ramdisk_attempts: HashMap<String, Instant> = HashMap::new();
    let mut last_ramdisk_check = Instant::now() - Duration::from_secs(120);
    let mut last_metadata_promotion = Instant::now() - Duration::from_secs(30);
    let mut last_queue_priority = Instant::now() - Duration::from_secs(60);
    let mut last_dynamic_adjustment = Instant::now() - Duration::from_secs(90);
    let mut last_speed_policy = Instant::now() - Duration::from_secs(60);
    let mut last_speed: Option<(i64, i64)> = None;
    let mut last_debug_log = Instant::now() - Duration::from_secs(300);
    let mut last_pin: Option<String> = None;
    let mut last_sequential: Option<bool> = None;
    let mut cfg = fallback.clone();
    let mut notifier = Notifier::from_config(&cfg);
    let mut tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
    let mut last_config_reload = Instant::now() - Duration::from_secs(5);
    let mut last_fingerprint = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(750)).await;
        let now = Instant::now();
        if now.duration_since(last_config_reload) >= Duration::from_secs(5) {
            cfg = Config::load(&config_path)
                .map(|mut cfg| {
                    if fallback.dry_run {
                        cfg.dry_run = true;
                    }
                    cfg
                })
                .unwrap_or_else(|_| fallback.clone());
            // Ricrea notifier/client TMDB solo se la configurazione rilevante è
            // cambiata: prima venivano ricostruiti (con nuovi client HTTP) ogni 5s.
            let fingerprint = format!(
                "{}|{}|{}",
                cfg.tmdb_api_key.clone().unwrap_or_default(),
                cfg.tmdb_language(),
                serde_json::to_string(&cfg.settings).unwrap_or_default()
            );
            if fingerprint != last_fingerprint {
                notifier = Notifier::from_config(&cfg);
                tmdb = TmdbClient::with_language(cfg.tmdb_api_key.clone(), cfg.tmdb_language());
                last_fingerprint = fingerprint;
            }
            last_config_reload = now;
        }
        if now.duration_since(last_metadata_promotion) >= Duration::from_secs(30) {
            torrents.promote_metadata();
            last_metadata_promotion = now;
        }
        if now.duration_since(last_queue_priority) >= Duration::from_secs(60) {
            torrents.prioritize_queue();
            last_queue_priority = now;
        }
        if now.duration_since(last_dynamic_adjustment) >= Duration::from_secs(90) {
            torrents.adjust_queue(&cfg);
            last_dynamic_adjustment = now;
        }
        if now.duration_since(last_speed_policy) >= Duration::from_secs(60) {
            // Log only when the effective limits change (informative, not noisy).
            let (download_kib, upload_kib) = current_speed_limits(&cfg);
            if last_speed != Some((download_kib, upload_kib)) {
                match torrents.set_global_speed_limits(download_kib, upload_kib) {
                    Ok(_) => {
                        tracing::info!(download_kib, upload_kib, "global speed limits applied")
                    }
                    Err(error) => tracing::debug!(%error, "speed policy apply failed"),
                }
                last_speed = Some((download_kib, upload_kib));
            }
            last_speed_policy = now;
        }
        // `debug_enabled`: periodic diagnostics for crashes/RAM/loops.
        if cfg.debug_enabled() && now.duration_since(last_debug_log) >= Duration::from_secs(300) {
            last_debug_log = now;
            tracing::debug!(
                resident_kb = resident_kb(),
                torrents = torrents.list().len(),
                "debug diagnostics"
            );
        }
        let pinned = cfg
            .settings
            .get("libtorrent_pinned_hash")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        if last_pin != pinned {
            if let Some(hash) = &pinned {
                if let Err(error) = torrents.set_pin(hash, true) {
                    tracing::debug!(%error, "pin apply failed");
                }
            }
            last_pin = pinned;
        }
        let sequential = matches!(
            cfg.settings
                .get("libtorrent_sequential")
                .map(|value| value.to_ascii_lowercase())
                .as_deref(),
            Some("true") | Some("yes") | Some("1")
        );
        if last_sequential != Some(sequential) {
            if let Err(error) = torrents.set_sequential(sequential) {
                tracing::debug!(%error, "sequential apply failed");
            }
            last_sequential = Some(sequential);
        }
        monitor_metadata(
            &cfg,
            &torrents,
            &db,
            &notifier,
            &mut metadata_wait_start,
            &mut metadata_first_seen,
        )
        .await;
        monitor_stalled(&cfg, &torrents, &db, &notifier, &mut stall_wait_start).await;
        if now.duration_since(last_ramdisk_check) >= Duration::from_secs(30) {
            reconcile_ramdisk(&cfg, &torrents, &mut ramdisk_attempts);
            last_ramdisk_check = now;
        }
        let mut torrent_events = torrents.poll_events();
        let mut event_hashes = torrent_events
            .iter()
            .map(|event| event.hash.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        // A torrent may reach 100% while Rextto is restarting or while its
        // completion event is already gone from libtorrent's alert queue.
        // Recreate the event from the persisted torrent status so postprocess
        // and notifications are not silently skipped.
        for torrent in torrents
            .list()
            .into_iter()
            .filter(|torrent| torrent.has_metadata && torrent.progress >= 99.99)
        {
            let hash = torrent.hash.to_ascii_lowercase();
            if event_hashes.contains(&hash) {
                continue;
            }
            let pending = {
                let database = db.lock().unwrap();
                database.torrent_meta(&hash).ok().flatten().is_some()
                    && database
                        .torrent_status(&hash)
                        .ok()
                        .flatten()
                        .is_some_and(|status| {
                            !matches!(status.as_str(), "completed" | "error" | "removed")
                        })
            };
            if pending {
                tracing::debug!(
                    hash = %hash,
                    name = %torrent.name,
                    save_path = %torrent.save_path,
                    progress = torrent.progress,
                    "recovering completed torrent without completion event"
                );
                torrent_events.push(TorrentEvent {
                    kind: "torrent_finished".into(),
                    hash: hash.clone(),
                    name: torrent.name,
                    save_path: torrent.save_path,
                });
                event_hashes.insert(hash);
            }
        }
        for event in torrent_events {
            let public_event = event.clone();
            let hash = event.hash.clone();
            if let Ok(Some(comic)) = comics.torrent(&hash) {
                if matches!(event.kind.as_str(), "torrent_finished" | "storage_moved") {
                    let result = comics.complete_torrent(&hash, &event.save_path);
                    match result {
                        Ok(()) => {
                            match notifier.notify_event("comic_completed", serde_json::json!({"hash": hash, "title": comic.title, "post_url": comic.post_url, "path": event.save_path})).await {
                                Ok(()) => tracing::info!(hash=%hash, "completion notification sent"),
                                Err(error) => tracing::warn!(hash=%hash, %error, "completion notification failed"),
                            }
                            tracing::info!(hash=%event.hash, title=%comic.title, path=%event.save_path, "comic torrent completed");
                        }
                        Err(error) => {
                            let _ = notifier.notify_event("comic_error", serde_json::json!({"hash": hash, "title": comic.title, "error": error.to_string()})).await;
                            tracing::error!(%error, hash=%event.hash, "comic torrent completion persistence failed");
                        }
                    }
                }
                let mut events = event_log.lock().unwrap();
                events.push(public_event);
                if events.len() > 512 {
                    let overflow = events.len() - 512;
                    events.drain(..overflow);
                }
                continue;
            }
            let processed = match handle_torrent_event(
                &cfg,
                &torrents,
                &db,
                &mut move_requests,
                event.clone(),
                &tmdb,
                &notifier,
            )
            .await
            {
                Ok(processed) => processed,
                Err(error) => {
                    let restored = db.lock().unwrap().restore_upgrade(&hash).unwrap_or(false);
                    let _ = db
                        .lock()
                        .unwrap()
                        .mark_torrent_error(&hash, &error.to_string());
                    let _ = notifier.notify_event("torrent_error", serde_json::json!({"hash": hash, "error": error.to_string(), "upgrade_restored": restored})).await;
                    tracing::error!(%error, "torrent completion handling failed");
                    false
                }
            };
            if processed && matches!(event.kind.as_str(), "torrent_finished" | "storage_moved") {
                refresh_media_libraries(&cfg).await;
            }
            if processed && cfg.libtorrent.auto_remove_completed {
                let is_pack = db
                    .lock()
                    .unwrap()
                    .torrent_meta(&event.hash)
                    .ok()
                    .flatten()
                    .map(|meta| meta.release.is_pack)
                    .unwrap_or(false);
                if let Ok(true) = torrents.remove(&event.hash, is_pack) {
                    let _ = db.lock().unwrap().mark_torrent_removed_at(&event.hash);
                }
            }
            // A completed single episode/movie is moved into the archive and
            // renamed, so the file is no longer at the path libtorrent tracks.
            // It can no longer seed and would re-download the release on the
            // next check (seen as a duplicate). Drop those torrents, but keep
            // no-rename/in-place completions seeding as before.
            if processed && matches!(event.kind.as_str(), "torrent_finished" | "storage_moved") {
                let is_pack = db
                    .lock()
                    .unwrap()
                    .torrent_meta(&event.hash)
                    .ok()
                    .flatten()
                    .map(|meta| meta.release.is_pack)
                    .unwrap_or(false);
                if !is_pack && !postprocess::completion_path(&event).exists() {
                    if let Ok(true) = torrents.remove(&event.hash, false) {
                        let _ = db.lock().unwrap().mark_torrent_removed_at(&event.hash);
                        tracing::info!(
                            hash = %event.hash,
                            "completed torrent removed: file renamed into the archive"
                        );
                    }
                }
            }
            if processed && matches!(event.kind.as_str(), "torrent_finished" | "storage_moved") {
                let completion_meta = db.lock().unwrap().torrent_meta(&event.hash).ok().flatten();
                let is_pack = completion_meta
                    .as_ref()
                    .map(|meta| meta.release.is_pack)
                    .unwrap_or(false);
                if !is_pack {
                    let processed_path = db
                        .lock()
                        .unwrap()
                        .torrent_processed(&event.hash)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| event.save_path.clone());
                    let size_bytes =
                        postprocess::size_of_path(std::path::Path::new(&processed_path))
                            .unwrap_or(0);
                    let title = completion_meta
                        .as_ref()
                        .map(|meta| &meta.release.title)
                        .unwrap_or(&event.name);
                    // Download duration and average speed for the notification.
                    let (duration_seconds, average_speed_bps) = db
                        .lock()
                        .unwrap()
                        .torrent_times(&event.hash)
                        .ok()
                        .flatten()
                        .and_then(|(created, completed)| {
                            let created = crate::utils::parse_timestamp(&created)?;
                            let completed = completed
                                .as_deref()
                                .and_then(crate::utils::parse_timestamp)
                                .unwrap_or_else(chrono::Utc::now);
                            let seconds = (completed - created).num_seconds().max(1);
                            Some((seconds, (size_bytes as f64 / seconds as f64) as i64))
                        })
                        .map(|(seconds, speed)| (Some(seconds), Some(speed)))
                        .unwrap_or((None, None));
                    let notification = notifier.notify_event("torrent_completed", serde_json::json!({
                        "hash": &event.hash,
                        "name": &event.name,
                        "title": title,
                        "kind": completion_meta.as_ref().map(|meta| &meta.release.kind),
                        "series": completion_meta.as_ref().and_then(|meta| meta.release.series.as_ref()),
                        "season": completion_meta.as_ref().and_then(|meta| meta.release.season),
                        "episode": completion_meta.as_ref().and_then(|meta| meta.release.episode),
                        "path": processed_path,
                        "size_bytes": size_bytes,
                        "duration_seconds": duration_seconds,
                        "average_speed_bps": average_speed_bps
                    })).await;
                    match notification {
                        Ok(()) => {
                            tracing::info!(hash=%event.hash, event="torrent_completed", "completion notification sent")
                        }
                        Err(error) => {
                            tracing::warn!(hash=%event.hash, event="torrent_completed", %error, "completion notification failed")
                        }
                    }
                }
            }
            let mut events = event_log.lock().unwrap();
            events.push(public_event);
            if events.len() > 512 {
                let overflow = events.len() - 512;
                events.drain(..overflow);
            }
        }
        // Enforce the seed policy only after handling this tick's events: with a
        // very low seed limit a just-finished torrent could otherwise be removed
        // before its `torrent_finished` event is post-processed and archived.
        enforce_seed_policy(&cfg, &torrents, &db);
    }
}

/// Aggiorna il timer di stallo di un download. Come EXTTO: il timer si azzera
/// appena arriva traffico o si vede anche un solo peer, perché un torrent con
/// fonti non è "morto" anche se resta a 0 byte. Scade solo dopo `timeout` di
/// assenza totale di peer e traffico.
fn stall_expired(
    entry: &mut (Instant, i64),
    now: Instant,
    done: i64,
    download_rate: u64,
    num_peers: i32,
    timeout: Duration,
) -> bool {
    if done > entry.1 || download_rate > 0 || num_peers > 0 {
        entry.0 = now;
        if done > entry.1 {
            entry.1 = done;
        }
    }
    now.duration_since(entry.0) >= timeout
}

/// Rimuove un torrent fallito. I file parziali vengono cancellati solo se il
/// download non era completato; per un torrent già completo/in seed non si tocca
/// la libreria. Il resume viene comunque eliminato da `LibtorrentClient::remove`,
/// così il torrent non risorge al riavvio.
fn remove_failed_torrent(torrents: &LibtorrentClient, hash: &str) {
    let incomplete = torrents
        .list()
        .into_iter()
        .find(|torrent| torrent.hash.eq_ignore_ascii_case(hash))
        .is_some_and(|torrent| {
            torrent.progress < 99.99 && !matches!(torrent.state.as_str(), "seeding" | "finished")
        });
    match torrents.remove(hash, incomplete) {
        Ok(true) => tracing::info!(hash, delete_files = incomplete, "failed torrent removed"),
        Ok(false) => tracing::debug!(hash, "failed torrent already removed"),
        Err(error) => tracing::warn!(hash, %error, "failed torrent removal failed"),
    }
}

async fn monitor_stalled(
    cfg: &Config,
    torrents: &LibtorrentClient,
    db: &Arc<Mutex<Database>>,
    notifier: &Notifier,
    watch: &mut HashMap<String, (Instant, i64)>,
) {
    let stall_minutes = cfg
        .settings
        .get("libtorrent_stall_timeout_min")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(10080.0);
    if stall_minutes <= 0.0 {
        watch.clear();
        return;
    }
    let timeout = Duration::from_secs_f64(stall_minutes * 60.0);
    let now = Instant::now();
    let mut live = HashSet::new();
    for torrent in torrents.list() {
        live.insert(torrent.hash.clone());
        if torrent.state != "downloading" || torrent.progress >= 100.0 {
            watch.remove(&torrent.hash);
            continue;
        }
        let entry = watch
            .entry(torrent.hash.clone())
            .or_insert((now, torrent.total_done));
        if !stall_expired(
            entry,
            now,
            torrent.total_done,
            torrent.download_rate,
            torrent.num_peers,
            timeout,
        ) {
            continue;
        }
        let mut failed_title = String::new();
        if let Some(metadata) = db
            .lock()
            .unwrap()
            .torrent_meta(&torrent.hash)
            .ok()
            .flatten()
        {
            let _ = db.lock().unwrap().blocklist(&metadata.release, "stalled");
            failed_title = metadata.release.title.clone();
        }
        let restored = db
            .lock()
            .unwrap()
            .restore_upgrade(&torrent.hash)
            .unwrap_or(false);
        let _ = db
            .lock()
            .unwrap()
            .mark_torrent_error(&torrent.hash, "stalled download");
        tracing::warn!(
            hash = %torrent.hash,
            title = %failed_title,
            progress = torrent.progress,
            stall_minutes,
            "❌ DOWNLOAD FAILED — stalled (no peers or traffic within the timeout)"
        );
        remove_failed_torrent(torrents, &torrent.hash);
        let _ = notifier.notify_event("download_failed", serde_json::json!({"hash":torrent.hash,"title":failed_title,"error":"stalled download","upgrade_restored":restored})).await;
        watch.remove(&torrent.hash);
    }
    watch.retain(|hash, _| live.contains(hash));
}

async fn monitor_metadata(
    cfg: &Config,
    torrents: &LibtorrentClient,
    db: &Arc<Mutex<Database>>,
    notifier: &Notifier,
    wait_start: &mut HashMap<String, Instant>,
    first_seen: &mut HashMap<String, Instant>,
) {
    let timeout = Duration::from_secs(600);
    let giveup_minutes = cfg
        .settings
        .get("libtorrent_metadata_giveup_min")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1440.0);
    if giveup_minutes <= 0.0 {
        wait_start.clear();
        first_seen.clear();
        return;
    }
    let giveup = Duration::from_secs_f64(giveup_minutes * 60.0);
    let now = Instant::now();
    let mut live = HashSet::new();
    for torrent in torrents.list() {
        live.insert(torrent.hash.clone());
        if torrent.has_metadata {
            wait_start.remove(&torrent.hash);
            first_seen.remove(&torrent.hash);
            continue;
        }
        // A paused magnet is waiting for a download slot and must not age out.
        if torrent.state == "paused" {
            wait_start.remove(&torrent.hash);
            first_seen.remove(&torrent.hash);
            continue;
        }
        if torrent.state != "downloading_metadata" {
            wait_start.remove(&torrent.hash);
            first_seen.remove(&torrent.hash);
            continue;
        }
        let started = *first_seen.entry(torrent.hash.clone()).or_insert(now);
        let retry_at = *wait_start.entry(torrent.hash.clone()).or_insert(now);
        if now.duration_since(started) >= giveup {
            let metadata = db
                .lock()
                .unwrap()
                .torrent_meta(&torrent.hash)
                .ok()
                .flatten();
            if let Some(metadata) = metadata {
                let _ = db
                    .lock()
                    .unwrap()
                    .blocklist(&metadata.release, "dead_magnet");
            }
            let restored = db
                .lock()
                .unwrap()
                .restore_upgrade(&torrent.hash)
                .unwrap_or(false);
            let _ = db
                .lock()
                .unwrap()
                .mark_torrent_error(&torrent.hash, "metadata timeout");
            tracing::warn!(
                hash = %torrent.hash,
                giveup_minutes,
                "❌ DOWNLOAD FAILED — no metadata (dead magnet) within the give-up window"
            );
            remove_failed_torrent(torrents, &torrent.hash);
            let _ = notifier.notify_event("torrent_error", serde_json::json!({"hash":torrent.hash,"error":"metadata timeout","upgrade_restored":restored})).await;
            wait_start.remove(&torrent.hash);
            first_seen.remove(&torrent.hash);
        } else if now.duration_since(retry_at) >= timeout {
            match torrents.reannounce(&torrent.hash) {
                Ok(true) => {
                    tracing::warn!(hash=%torrent.hash, "torrent metadata still unavailable; reannouncing")
                }
                Ok(false) => {
                    tracing::debug!(hash=%torrent.hash, "metadata reannounce unavailable in current mode")
                }
                Err(error) => {
                    tracing::debug!(hash=%torrent.hash, %error, "metadata reannounce failed")
                }
            }
            wait_start.insert(torrent.hash, now);
        }
    }
    wait_start.retain(|hash, _| live.contains(hash));
    first_seen.retain(|hash, _| live.contains(hash));
}

/// Whether a completed torrent with an infinite-seed override was stopped by
/// our own seed policy and therefore must be resumed. A torrent paused by
/// libtorrent's queue keeps `auto_managed` set and must be left to the queue.
fn needs_infinite_seed_resume(torrent: &crate::models::TorrentView) -> bool {
    (torrent.seed_ratio == 0.0 || torrent.seed_days == 0)
        && torrent.progress >= 100.0
        && torrent.state == "paused"
        && !torrent.auto_managed
}

fn enforce_seed_policy(cfg: &Config, torrents: &LibtorrentClient, db: &Arc<Mutex<Database>>) {
    let ratio_limit = cfg
        .libtorrent
        .stop_at_ratio
        .then_some(cfg.libtorrent.seed_ratio)
        .filter(|ratio| *ratio > 0.0);
    // legacy semantics: days is the authoritative long-duration setting;
    // minutes remains as a backward-compatible fallback for sub-day limits.
    let time_limit = if cfg.libtorrent.seed_time_days > 0 {
        Some(cfg.libtorrent.seed_time_days.saturating_mul(86_400))
    } else {
        (cfg.libtorrent.seed_time_minutes > 0)
            .then_some(cfg.libtorrent.seed_time_minutes.saturating_mul(60))
    };
    // An infinite-seed override means the torrent must keep seeding: if a
    // previous seed limit left it paused after completion, resume it.
    //
    // `!auto_managed` is essential: libtorrent's queue pauses torrents past
    // `active_seeds` keeping `auto_managed` set. Those are legitimately
    // queued (legacy shows them as "In Coda (Seeding)") and resuming them every
    // tick is both futile and a log flood — libtorrent simply pauses them
    // again at the next auto-manage interval. Only a torrent paused by our own
    // seed policy (which clears `auto_managed`) needs the override resume.
    for torrent in torrents.list() {
        if needs_infinite_seed_resume(&torrent) {
            match torrents.resume(&torrent.hash) {
                Ok(true) => {
                    tracing::info!(hash=%torrent.hash, "torrent resumed for infinite seeding")
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(hash=%torrent.hash, %error, "resume for infinite seeding failed")
                }
            }
        }
    }
    for torrent in torrents
        .list()
        .into_iter()
        .filter(|torrent| torrent.state == "seeding")
    {
        let has_override = torrent.seed_ratio >= 0.0 || torrent.seed_days >= 0;
        // legacy treats either explicit zero as an infinite per-torrent seed rule.
        if has_override && (torrent.seed_ratio == 0.0 || torrent.seed_days == 0) {
            continue;
        }
        let torrent_ratio_limit = if torrent.seed_ratio > 0.0 {
            Some(torrent.seed_ratio)
        } else if torrent.seed_ratio < 0.0 {
            ratio_limit
        } else {
            None
        };
        let torrent_time_limit = if torrent.seed_days > 0 {
            Some(torrent.seed_days.saturating_mul(86_400))
        } else if torrent.seed_days < 0 {
            time_limit
        } else {
            None
        };
        let ratio_reached = torrent_ratio_limit.is_some_and(|limit| {
            torrent.all_time_download > 0
                && (torrent.all_time_upload as f64 / torrent.all_time_download as f64) >= limit
        });
        let time_reached = torrent_time_limit.is_some_and(|limit| torrent.seeding_seconds >= limit);
        if ratio_reached || time_reached {
            match torrents.pause(&torrent.hash) {
                Ok(true) => {
                    tracing::info!(hash=%torrent.hash, ratio_reached, time_reached, "torrent paused at seed limit")
                }
                Ok(false) => {
                    tracing::warn!(hash=%torrent.hash, "torrent seed limit could not be applied in current mode")
                }
                Err(error) => {
                    tracing::warn!(hash=%torrent.hash, %error, "failed to stop torrent at seed limit")
                }
            }
            post_seed_relocate(cfg, torrents, &torrent);
            if cfg.libtorrent.auto_remove_completed {
                let is_pack = db
                    .lock()
                    .unwrap()
                    .torrent_meta(&torrent.hash)
                    .ok()
                    .flatten()
                    .map(|meta| meta.release.is_pack)
                    .unwrap_or(false);
                match torrents.remove(&torrent.hash, is_pack) {
                    Ok(true) => {
                        let _ = db.lock().unwrap().mark_torrent_removed_at(&torrent.hash);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(hash=%torrent.hash, %error, "failed to remove torrent at seed limit")
                    }
                }
            }
        }
    }
}

/// legacy keeps completed data in RAM/temp while it is seeding, then moves it
/// to the durable final directory once the seed policy stops the torrent.
fn post_seed_relocate(
    cfg: &Config,
    torrents: &LibtorrentClient,
    torrent: &crate::models::TorrentView,
) {
    let current = FsPath::new(&torrent.save_path);
    let in_ramdisk = cfg
        .ramdisk_dir()
        .is_some_and(|path| crate::libtorrent::path_on_ramdisk(current, &path));
    let in_temp = cfg
        .libtorrent_temp_dir
        .as_deref()
        .is_some_and(|path| postprocess::same_path(current, path));
    if !in_ramdisk && !in_temp {
        return;
    }
    let destination = cfg.libtorrent_dir.clone();
    if postprocess::same_path(current, &destination) {
        return;
    }
    if let Err(error) = postprocess::validate_destination_from(current, &destination) {
        tracing::warn!(hash=%torrent.hash, %error, "post-seeding relocation refused");
        return;
    }
    match torrents.move_storage(&torrent.hash, &destination) {
        Ok(true) => {
            tracing::info!(
                hash = %torrent.hash,
                from = %current.display(),
                to = %destination.display(),
                size = %crate::logging::human_bytes_i64(torrent.total_size),
                "📁 MOVING TO NAS — post-seeding relocation"
            )
        }
        Ok(false) => tracing::debug!(hash=%torrent.hash, "post-seeding relocation was not applied"),
        Err(error) => tracing::warn!(hash=%torrent.hash, %error, "post-seeding relocation failed"),
    }
}

/// legacy parity for the 3-tier download layout (RAM disk / temp / final):
/// when metadata arrives, a torrent sitting on the RAM disk is moved to the
/// temporary (or final) directory if it would exceed the configured per-torrent
/// threshold or leave less than the safety margin free once written.
/// Reason a torrent currently on the RAM disk should move to disk, if any;
/// returns the human-readable reason and the destination directory.
fn ramdisk_relocation(
    cfg: &Config,
    torrents: &LibtorrentClient,
    hash: &str,
    save_path: &str,
) -> Option<(String, PathBuf)> {
    if !cfg.ramdisk_enabled() {
        return None;
    }
    let ramdisk = cfg.ramdisk_dir()?;
    let current = FsPath::new(save_path);
    if !crate::libtorrent::path_on_ramdisk(current, &ramdisk) {
        return None;
    }
    let torrent = torrents
        .list()
        .into_iter()
        .find(|torrent| torrent.hash.eq_ignore_ascii_case(hash))?;
    let free = crate::libtorrent::free_space_bytes(&ramdisk)?;
    let uncommitted = torrents.ramdisk_uncommitted_bytes(&ramdisk, hash);
    let reason = crate::libtorrent::ramdisk_fits(
        cfg.ramdisk_threshold_bytes(),
        cfg.ramdisk_margin_bytes(),
        free,
        uncommitted,
        torrent.total_size.max(0) as u64,
    )
    .err()?;
    let destination = cfg
        .libtorrent_temp_dir
        .clone()
        .unwrap_or_else(|| cfg.libtorrent_dir.clone());
    if postprocess::same_path(current, &destination) {
        return None;
    }
    clear_empty_destination(&destination, &torrent.name);
    Some((reason, destination))
}

/// `LibtorrentClient::move_storage` uses `fail_if_exist`: a leftover *empty*
/// destination directory (from an earlier attempt) makes the move fail
/// silently. Remove it so the relocation can proceed. A non-empty destination
/// is left alone (real data) and the move failure is surfaced by the native
/// `storage_moved_failed` alert.
fn clear_empty_destination(destination: &FsPath, name: &str) {
    if name.trim().is_empty() {
        return;
    }
    let target = destination.join(name);
    if !target.is_dir() {
        return;
    }
    let empty = std::fs::read_dir(&target)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false);
    if !empty {
        return;
    }
    match std::fs::remove_dir(&target) {
        Ok(()) => tracing::info!(
            target = %target.display(),
            "removed empty destination directory before storage move"
        ),
        Err(error) => tracing::warn!(
            target = %target.display(),
            %error,
            "cannot remove empty destination directory"
        ),
    }
}

fn enforce_ramdisk_capacity(cfg: &Config, torrents: &LibtorrentClient, event: &TorrentEvent) {
    let Some((reason, destination)) =
        ramdisk_relocation(cfg, torrents, &event.hash, &event.save_path)
    else {
        return;
    };
    match torrents.move_storage(&event.hash, &destination) {
        Ok(true) => tracing::info!(
            hash=%event.hash,
            %reason,
            destination=%destination.display(),
            "torrent does not fit on the RAM disk, moving to disk"
        ),
        Ok(false) => {
            tracing::warn!(hash=%event.hash, %reason, "RAM disk relocation was not applied")
        }
        Err(error) => tracing::warn!(hash=%event.hash, %error, "RAM disk relocation failed"),
    }
}

/// Self-healing sweep: move every torrent that no longer fits off the RAM disk.
/// Runs periodically so a lost `metadata_received` event can no longer pin an
/// oversized torrent (and fill the tmpfs) until the next restart.
fn reconcile_ramdisk(
    cfg: &Config,
    torrents: &LibtorrentClient,
    attempts: &mut HashMap<String, Instant>,
) {
    if !cfg.ramdisk_enabled() {
        attempts.clear();
        return;
    }
    let Some(ramdisk) = cfg.ramdisk_dir() else {
        attempts.clear();
        return;
    };
    let now = Instant::now();
    // Move at most a few per sweep: each relocation triggers a recheck, so a
    // large batch would compete with the ongoing downloads for disk I/O.
    let mut moved = 0usize;
    for torrent in torrents.list() {
        if moved >= 3 {
            break;
        }
        let hash = torrent.hash.to_ascii_lowercase();
        // Wait for the real size and leave seeding data to `post_seed_relocate`.
        if torrent.total_size <= 0
            || matches!(torrent.state.as_str(), "seeding" | "finished")
            || !crate::libtorrent::path_on_ramdisk(FsPath::new(&torrent.save_path), &ramdisk)
        {
            continue;
        }
        if attempts
            .get(&hash)
            .is_some_and(|at| now.duration_since(*at) < Duration::from_secs(600))
        {
            continue;
        }
        let Some((reason, destination)) =
            ramdisk_relocation(cfg, torrents, &hash, &torrent.save_path)
        else {
            attempts.remove(&hash);
            continue;
        };
        attempts.insert(hash.clone(), now);
        moved += 1;
        match torrents.move_storage(&hash, &destination) {
            Ok(true) => tracing::info!(
                hash = %hash,
                %reason,
                destination = %destination.display(),
                "🔁 RAM disk reconciliation: moving torrent to disk"
            ),
            Ok(false) => tracing::warn!(
                hash = %hash,
                %reason,
                "RAM disk reconciliation: relocation not applied"
            ),
            Err(error) => {
                tracing::warn!(hash = %hash, %error, "RAM disk reconciliation failed")
            }
        }
    }
}

/// A completed download that cannot be imported must not keep occupying the
/// RAM disk. Keep it recoverable in the configured trash, then remove the
/// torrent handle because its storage path is no longer active.
/// Scrive nella cartella del pack un file di nota quando la release viene
/// rifiutata, così resta chiaro perché era presente.
fn write_rejection_marker(source: &std::path::Path, reason: &str) {
    if !source.is_dir() {
        return;
    }
    let content = format!(
        "Rifiutato per minore qualità.\nMotivo: {reason}\nData: {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    if let Err(error) = std::fs::write(source.join("RIFIUTATO.txt"), content) {
        tracing::warn!(%error, source = %source.display(), "could not write rejection marker");
    }
}

fn discard_completed_source(
    cfg: &Config,
    db: &Arc<Mutex<Database>>,
    torrents: &LibtorrentClient,
    event: &TorrentEvent,
    reason: &str,
) {
    let source = postprocess::completion_path(event);
    if source.exists() {
        write_rejection_marker(&source, reason);
        if let Some(trash) = cfg.trash_path.as_deref() {
            match crate::cleaner::move_to_trash(&source, trash) {
                Ok(target) => tracing::info!(
                    hash = %event.hash,
                    source = %source.display(),
                    trash = %target.display(),
                    "rejected completed download moved to trash"
                ),
                Err(error) => tracing::error!(
                    hash = %event.hash,
                    source = %source.display(),
                    %error,
                    "could not move rejected completed download to trash"
                ),
            }
        } else {
            let result = if source.is_dir() {
                std::fs::remove_dir_all(&source)
            } else {
                std::fs::remove_file(&source)
            };
            if let Err(error) = result {
                tracing::error!(
                    hash = %event.hash,
                    source = %source.display(),
                    %error,
                    "could not remove rejected completed download"
                );
            } else {
                tracing::info!(
                    hash = %event.hash,
                    source = %source.display(),
                    "rejected completed download removed"
                );
            }
        }
    }
    match torrents.remove(&event.hash, false) {
        Ok(true) => {
            // Il torrent ha lasciato la sessione: entra nello Storico con l'esito.
            let _ = db.lock().unwrap().mark_torrent_removed_at(&event.hash);
            tracing::info!(hash = %event.hash, "rejected completed torrent removed")
        }
        Ok(false) => tracing::debug!(hash = %event.hash, "rejected torrent was already removed"),
        Err(error) => tracing::warn!(hash = %event.hash, %error, "rejected torrent removal failed"),
    }
}

async fn handle_torrent_event(
    cfg: &Config,
    torrents: &LibtorrentClient,
    db: &Arc<Mutex<Database>>,
    move_requests: &mut HashSet<String>,
    event: TorrentEvent,
    tmdb: &TmdbClient,
    notifier: &Notifier,
) -> anyhow::Result<bool> {
    tracing::debug!(
        hash = %event.hash,
        kind = %event.kind,
        name = %event.name,
        save_path = %event.save_path,
        "torrent completion processing started"
    );
    let metadata = { db.lock().unwrap().torrent_meta(&event.hash)? };
    let Some(metadata) = metadata else {
        tracing::warn!(hash=%event.hash, kind=%event.kind, "torrent alert has no registered release metadata");
        return Ok(false);
    };
    Ok(match event.kind.as_str() {
        // legacy parity: at `add()` the size is unknown and the RAM disk is used
        // first; once metadata arrives the real size decides if it still fits.
        "metadata_received" => {
            if let Some(torrent) = torrents
                .list()
                .into_iter()
                .find(|torrent| torrent.hash.eq_ignore_ascii_case(&event.hash))
            {
                tracing::info!(
                    hash = %event.hash,
                    title = %metadata.release.title,
                    torrent_name = %torrent.name,
                    size = %crate::logging::human_bytes_i64(torrent.total_size),
                    "📦 DOWNLOAD METADATA RECEIVED"
                );
            }
            enforce_ramdisk_capacity(cfg, torrents, &event);
            false
        }
        "torrent_finished" => {
            // Completion can be triggered twice (a recovered event after a
            // restart, or storage_moved): never post-process an already
            // completed release again, or the renamed file is looked up under
            // its original name and fails.
            if db.lock().unwrap().torrent_status(&event.hash)?.as_deref() == Some("completed") {
                tracing::debug!(hash=%event.hash, "ignoring completion for already completed torrent");
                return Ok(false);
            }
            let destination = postprocess::destination_for(&metadata.release, cfg);
            let current = FsPath::new(&event.save_path);
            if metadata.release.is_pack {
                if let Some(destination) = destination {
                    let source = postprocess::completion_path(&event);
                    let size = postprocess::size_of_path(&source)?;
                    let matching = postprocess::matching_pack_files(&source, &metadata.release)?;
                    if matching.is_empty() {
                        let error = "season pack filenames do not match declared season";
                        tracing::warn!(
                            hash = %event.hash,
                            declared_season = %metadata
                                .release
                                .season
                                .map(|season| season.to_string())
                                .unwrap_or_else(|| "unknown".into()),
                            source = %source.display(),
                            "season pack rejected"
                        );
                        db.lock().unwrap().mark_torrent_error(&event.hash, error)?;
                        discard_completed_source(cfg, db, torrents, &event, error);
                        return Ok(false);
                    }
                    // Serialize against the periodic/manual rename repair for
                    // this series: it scans the archive and would otherwise
                    // rename/trash files while they are still being copied,
                    // producing transient duplicates and stale DB paths.
                    let _import_guard =
                        ArchiveImportGuard::acquire(metadata.release.series.as_deref());
                    // Copy and process one episode at a time so a partially
                    // copied file is never visible and the old, inferior file
                    // is removed as soon as its replacement is in place.
                    let mut processed = Vec::new();
                    for file in &matching {
                        let Some(placed) = postprocess::stage_pack_file(
                            file,
                            &source,
                            &destination,
                            cfg,
                            metadata.release.quality.score_with_settings(&cfg.settings),
                        )?
                        else {
                            continue;
                        };
                        let mut partial = postprocess::process_pack_files(
                            &[(placed, file.clone())],
                            &metadata.release,
                            cfg,
                            tmdb,
                        )
                        .await?;
                        processed.append(&mut partial);
                    }
                    if !processed.is_empty() && processed.iter().all(|item| item.discarded) {
                        let restored = db.lock().unwrap().restore_upgrade(&event.hash)?;
                        if !restored {
                            db.lock().unwrap().rollback_release(&metadata.release)?;
                        }
                        db.lock().unwrap().mark_torrent_error(
                            &event.hash,
                            "season pack inferior to existing files",
                        )?;
                        // Esito definitivo: il pack è stato scartato per intero.
                        // Esce dalla sessione (finisce nello Storico con il
                        // motivo) e la sorgente va nel cestino, così non resta
                        // a occupare spazio senza un badge NAS.
                        discard_completed_source(
                            cfg,
                            db,
                            torrents,
                            &event,
                            "season pack inferior to existing files",
                        );
                        return Ok(false);
                    }
                    let entries = processed
                        .iter()
                        .filter(|item| !item.discarded)
                        .map(|item| {
                            (
                                item.episode,
                                item.path.display().to_string(),
                                item.size_bytes,
                                item.quality_score,
                            )
                        })
                        .collect::<Vec<_>>();
                    db.lock().unwrap().mark_pack_completed(
                        &metadata.release,
                        &entries,
                        &destination.display().to_string(),
                        size,
                    )?;
                    let episodes = processed
                        .iter()
                        .filter(|item| !item.discarded)
                        .map(|item| serde_json::json!({"series": &metadata.release.series, "season": metadata.release.season, "episode": item.episode, "path": &item.path}))
                        .collect::<Vec<_>>();
                    tracing::info!(
                        hash = %event.hash,
                        title = %metadata.release.title,
                        destination = %destination.display(),
                        size = %crate::logging::human_bytes_i64(size),
                        episode_count = episodes.len(),
                        "🎉 SEASON PACK COMPLETE — archived to NAS"
                    );
                    let notification = notifier.notify_event("season_pack_completed", serde_json::json!({
                        "series": &metadata.release.series,
                        "season": metadata.release.season,
                        "title": &metadata.release.title,
                        "path": destination.display().to_string(),
                        "size_bytes": size,
                        "new_count": episodes.len(),
                        "discarded_count": processed.iter().filter(|item| item.discarded).count(),
                        "episodes": episodes,
                    })).await;
                    match notification {
                        Ok(()) => {
                            tracing::info!(hash=%event.hash, event="season_pack_completed", "completion notification sent")
                        }
                        Err(error) => {
                            tracing::warn!(hash=%event.hash, event="season_pack_completed", %error, "completion notification failed")
                        }
                    }
                    // A pack is copied into the library, never moved out of its
                    // torrent storage here: libtorrent must retain the exact
                    // original tree to seed and verify it. `auto_remove_completed`
                    // and the normal ratio/time cleanup are the only code paths
                    // allowed to discard this now-redundant source tree.
                    tracing::info!(
                        hash=%event.hash,
                        destination=%destination.display(),
                        size=%crate::logging::human_bytes_i64(size),
                        "📁 SEASON PACK COPIED TO NAS (source kept for seeding)"
                    );
                    true
                } else {
                    complete_torrent(cfg, db, torrents, &event, &metadata.release, tmdb).await?
                }
            } else if let Some(destination) = destination {
                if postprocess::same_path(current, &destination) {
                    complete_torrent(cfg, db, torrents, &event, &metadata.release, tmdb).await?
                } else if move_requests.insert(event.hash.clone()) {
                    postprocess::validate_destination_from(current, &destination)?;
                    tracing::info!(
                        hash = %event.hash,
                        title = %metadata.release.title,
                        from = %current.display(),
                        to = %destination.display(),
                        "📁 MOVING TO NAS — completed download leaves the work folder"
                    );
                    if !torrents.move_storage(&event.hash, &destination)? {
                        move_requests.remove(&event.hash);
                    }
                    false
                } else {
                    false
                }
            } else {
                complete_torrent(cfg, db, torrents, &event, &metadata.release, tmdb).await?
            }
        }
        "storage_move_failed" => {
            tracing::warn!(
                hash = %event.hash,
                name = %event.name,
                save_path = %event.save_path,
                "storage move failed (destination may already exist); torrent kept in place"
            );
            false
        }
        "storage_moved" => {
            // A post-seeding relocation happens after the release was already
            // committed. Do not run rename/copy/pack processing a second time.
            if db.lock().unwrap().torrent_status(&event.hash)?.as_deref() == Some("completed") {
                move_requests.remove(&event.hash);
                tracing::debug!(hash=%event.hash, "ignoring storage move for already completed torrent");
                return Ok(false);
            }
            let done = torrents
                .list()
                .into_iter()
                .find(|torrent| torrent.hash.eq_ignore_ascii_case(&event.hash))
                .is_some_and(|torrent| {
                    torrent.progress >= 100.0
                        || matches!(torrent.state.as_str(), "finished" | "seeding")
                });
            if !done {
                tracing::debug!(hash=%event.hash, "ignoring storage move before torrent completion");
                return Ok(false);
            }
            move_requests.remove(&event.hash);
            complete_torrent(cfg, db, torrents, &event, &metadata.release, tmdb).await?
        }
        _ => false,
    })
}

async fn complete_torrent(
    cfg: &Config,
    db: &Arc<Mutex<Database>>,
    torrents: &LibtorrentClient,
    event: &TorrentEvent,
    release: &crate::models::Release,
    tmdb: &TmdbClient,
) -> anyhow::Result<bool> {
    // Keep the rename repair off this series while the completion (resolve,
    // rename, dedup) runs: it would otherwise move the file mid-operation and
    // make `size_of_path` fail, leaving libtorrent free to re-download it.
    let _import_guard = ArchiveImportGuard::acquire(release.series.as_deref());
    let mut path = postprocess::completion_path(event);
    if !path.exists() {
        // The file may already have been moved and renamed (a second completion
        // pass, or the periodic rename repair). Resolve the archived file for
        // this episode instead of failing the whole completion and rolling the
        // release back.
        let resolved = if release.kind == "series" {
            release
                .series
                .as_deref()
                .and_then(|name| cfg.find_series_by_name(name))
                .and_then(|series| cfg.resolve_archive_path(series))
                .and_then(|dir| match (release.season, release.episode) {
                    (Some(season), Some(episode)) => postprocess::find_episode_file(
                        &dir, season, episode,
                    )
                    // Duplicates may coexist (e.g. a partial re-download next
                    // to the kept file): fall back to the best match instead of
                    // giving up and letting the torrent re-download.
                    .or_else(|| postprocess::best_episode_file(&dir, season, episode)),
                    _ => None,
                })
        } else {
            None
        };
        match resolved {
            Some(found) => {
                tracing::info!(
                    hash = %event.hash,
                    path = %found.display(),
                    "completed file already renamed; using the archived file"
                );
                path = found;
            }
            None => {
                tracing::warn!(
                    hash = %event.hash,
                    path = %path.display(),
                    "completed torrent file not found (already moved or renamed); skipping completion"
                );
                return Ok(false);
            }
        }
    }
    let size = postprocess::size_of_path(&path)?;
    let no_rename = db
        .lock()
        .unwrap()
        .torrent_no_rename(&event.hash)
        .unwrap_or(false);
    let renamed = if no_rename {
        tracing::info!(hash=%event.hash, "rename skipped: torrent marked no-rename");
        None
    } else if release.kind == "movie" {
        postprocess::rename_movie(&path, release, cfg, tmdb).await?
    } else {
        postprocess::rename_episode(&path, release, cfg, tmdb).await?
    };
    let processed_path = renamed.as_deref().unwrap_or(&path);
    let mut discarded = false;
    if release.kind == "series" && !release.is_pack {
        if let (Some(series), Some(season), Some(episode)) =
            (release.series.as_deref(), release.season, release.episode)
        {
            let new_file = renamed.clone().or_else(|| {
                postprocess::video_files(&path)
                    .ok()
                    .and_then(|files| (files.len() == 1).then(|| files[0].clone()))
            });
            let archive = if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            };
            if let Some(new_file) = new_file {
                let score = release.quality.score_with_settings(&cfg.settings);
                if crate::cleaner::discard_if_inferior(
                    cfg, series, season, episode, score, &new_file, archive,
                )? {
                    discarded = true;
                } else {
                    let _ = crate::cleaner::cleanup_old_episode(
                        cfg, series, season, episode, score, &new_file, archive,
                    )?;
                }
            }
        }
    } else if release.kind == "movie" {
        if let Some(new_file) = renamed.clone().or_else(|| {
            postprocess::video_files(&path)
                .ok()
                .and_then(|files| (files.len() == 1).then(|| files[0].clone()))
        }) {
            let archive = if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            };
            let score = release.quality.score_with_settings(&cfg.settings);
            if crate::cleaner::discard_if_inferior_movie(
                cfg,
                &release.title,
                release.year,
                score,
                &new_file,
                archive,
            )? {
                discarded = true;
            } else {
                let _ = crate::cleaner::cleanup_old_movie(
                    cfg,
                    &release.title,
                    release.year,
                    score,
                    &new_file,
                    archive,
                )?;
            }
        }
    }
    if discarded {
        let restored = db.lock().unwrap().restore_upgrade(&event.hash)?;
        if !restored {
            db.lock().unwrap().rollback_release(release)?;
        }
        db.lock()
            .unwrap()
            .mark_torrent_error(&event.hash, "release inferior to existing file")?;
        tracing::warn!(hash=%event.hash, "completed release discarded as inferior");
        return Ok(false);
    }
    db.lock().unwrap().mark_release_completed(
        release,
        &processed_path.display().to_string(),
        size,
    )?;
    // Download duration and average speed, reconstructed from the DB timestamps
    // (the completion alert may arrive after the process restarted).
    let (duration_seconds, average_speed) = db
        .lock()
        .unwrap()
        .torrent_times(&event.hash)
        .ok()
        .flatten()
        .and_then(|(created, completed)| {
            let created = crate::utils::parse_timestamp(&created)?;
            let completed = completed
                .as_deref()
                .and_then(crate::utils::parse_timestamp)
                .unwrap_or_else(chrono::Utc::now);
            let seconds = (completed - created).num_seconds().max(1);
            Some((seconds, size.max(1) / seconds))
        })
        .unwrap_or((0, 0));
    tracing::info!(
        hash = %event.hash,
        title = %release.title,
        path = %processed_path.display(),
        size = %crate::logging::human_bytes_i64(size),
        duration = %crate::logging::human_duration(duration_seconds),
        average_speed = %crate::logging::human_rate(average_speed),
        renamed = renamed.is_some(),
        "🎉 DOWNLOAD COMPLETE — file processed and saved"
    );
    // Se il file è stato rinominato o collegato a un file già esistente, i dati
    // del torrent non sono più al nome atteso (o sono un doppione) e libtorrent
    // ripartirebbe da 0: banda sprecata su contenuto già archiviato. Il torrent
    // va tolto. Vale per episodi singoli e film; i pack sono gestiti a parte
    // (copia con sorgente conservata per il seeding).
    if renamed.is_some() && !postprocess::same_path(processed_path, &path) {
        match torrents.remove(&event.hash, false) {
            Ok(true) => {
                // Ha lasciato la sessione: entra nello Storico con il percorso NAS.
                let _ = db.lock().unwrap().mark_torrent_removed_at(&event.hash);
                tracing::info!(
                    hash = %event.hash,
                    "torrent removed after rename: archived under a different path"
                )
            }
            Ok(false) => tracing::debug!(hash=%event.hash, "renamed torrent already removed"),
            Err(error) => {
                tracing::warn!(hash=%event.hash, %error, "renamed torrent removal failed")
            }
        }
    }
    Ok(true)
}
#[axum::debug_handler]
async fn run_now(State(s): State<AppState>, Query(query): Query<RunNowQuery>) -> impl IntoResponse {
    let cfg = match Config::load(&s.config_path) {
        Ok(mut cfg) => {
            if s.cfg.dry_run {
                cfg.dry_run = true;
            }
            cfg
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"ok":false,"error":error.to_string()})),
            )
        }
    };
    if !setup_complete(&cfg) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"complete the initial setup first"})),
        );
    }
    if !cfg.active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"ok":false,"error":"daemon inactive"})),
        );
    }
    let domain = query
        .domain
        .as_deref()
        .filter(|value| matches!(*value, "series" | "movies" | "comics"))
        .map(str::to_owned);
    let running = s.cycle_lock.try_lock().is_err();
    tracing::info!(
        domain = %domain.as_deref().unwrap_or("full"),
        queued = running,
        "manual cycle requested"
    );
    let task_domain = domain.clone();
    tokio::spawn(async move {
        let _cycle_guard = s.cycle_lock.lock().await;
        tracing::info!(
            domain = %task_domain.as_deref().unwrap_or("full"),
            "manual cycle started"
        );
        // Publish the start time immediately so the dashboard does not show
        // "non avviato" while the cycle is still running.
        *s.last_cycle.lock().unwrap() = crate::models::CycleStats {
            last_started_at: Some(chrono::Utc::now()),
            ..Default::default()
        };
        let notifier = Notifier::from_config(&cfg);
        match orchestrator::run_cycle_domain(
            &cfg,
            &s.engine,
            &s.db,
            &s.archive,
            &s.comics,
            &notifier,
            &s.torrents,
            task_domain.as_deref(),
        )
        .await
        {
            Ok(stats) => {
                tracing::info!(
                    downloads_started = stats.downloads_started,
                    gaps_filled = stats.gaps_filled,
                    "manual cycle completed"
                );
                *s.last_cycle.lock().unwrap() = stats;
            }
            Err(error) => tracing::error!(%error, "manual cycle failed"),
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "ok": true,
            "started": !running,
            "queued": running,
            "message": if running { "Ciclo già in corso: richiesta accodata" } else { "Ciclo avviato" }
        })),
    )
}

async fn cycle_worker(state: AppState) {
    let mut last_rename_check = Instant::now() - Duration::from_secs(6 * 3600);
    // Guards against overlapping archive repairs (they can be slow on NFS).
    let rename_repair_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut last_inactive_log: Option<Instant> = None;
    loop {
        let cfg = match Config::load(&state.config_path) {
            Ok(mut cfg) => {
                if state.cfg.dry_run {
                    cfg.dry_run = true;
                }
                cfg
            }
            Err(error) => {
                tracing::error!(%error, "scheduled config reload failed");
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
        };
        if cfg.active {
            let _cycle_guard = state.cycle_lock.lock().await;
            // Publish the start time immediately (see the manual path above).
            *state.last_cycle.lock().unwrap() = crate::models::CycleStats {
                last_started_at: Some(chrono::Utc::now()),
                ..Default::default()
            };
            let notifier = Notifier::from_config(&cfg);
            match orchestrator::run_cycle(
                &cfg,
                &state.engine,
                &state.db,
                &state.archive,
                &state.comics,
                &notifier,
                &state.torrents,
            )
            .await
            {
                Ok(stats) => {
                    *state.last_cycle.lock().unwrap() = stats;
                    tracing::info!("scheduled cycle completed");
                }
                Err(error) => tracing::error!(%error, "scheduled cycle failed"),
            }
            let rename_hours = cfg
                .settings
                .get("rename_verify_interval")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(6);
            if rename_hours > 0
                && last_rename_check.elapsed()
                    >= Duration::from_secs(rename_hours.saturating_mul(3600))
            {
                last_rename_check = Instant::now();
                if cfg.rename_episodes {
                    // The archive lives on a (possibly slow) NFS mount, so run
                    // the repair on a dedicated thread with its own runtime:
                    // blocking filesystem I/O there cannot stall the daemon's
                    // async runtime, keeping the API/UI responsive.
                    if !rename_repair_running.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        let state = state.clone();
                        let names = cfg
                            .series
                            .iter()
                            .map(|series| series.name.clone())
                            .collect::<Vec<_>>();
                        let flag = rename_repair_running.clone();
                        std::thread::spawn(move || {
                            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                            {
                                runtime.block_on(async {
                                    for name in &names {
                                        let _ = series_rename_apply(
                                            &state, name, true, false, false,
                                        )
                                        .await;
                                    }
                                });
                            }
                            tracing::info!(
                                series = names.len(),
                                "periodic archive rename repair completed"
                            );
                            flag.store(false, std::sync::atomic::Ordering::SeqCst);
                        });
                    }
                } else if let Ok(files) = state.db.lock().unwrap().archived_episode_files() {
                    let missing = files
                        .iter()
                        .filter(|(_, path)| !std::path::Path::new(path).exists())
                        .count();
                    tracing::debug!(
                        missing,
                        total = files.len(),
                        "rename verify: rename disabled"
                    );
                }
            }
        } else if last_inactive_log
            .map(|at| at.elapsed() >= Duration::from_secs(900))
            .unwrap_or(true)
        {
            // Evita un log vuoto quando il daemon non è attivo o è in dry-run.
            tracing::info!(
                dry_run = cfg.dry_run,
                refresh_secs = cfg.refresh_secs,
                "automatic cycle paused: daemon not active (enable active mode to resume)"
            );
            last_inactive_log = Some(Instant::now());
        }
        tokio::time::sleep(Duration::from_secs(cfg.refresh_secs.max(1))).await;
    }
}

/// Ottimizzazione continua libtorrent: quando `libtorrent_auto_optimize` è
/// attivo rivaluta periodicamente cache, buffer e coda in base alle risorse e
/// applica solo i valori cambiati (nessuna riscrittura inutile).
async fn optimize_worker(state: AppState) {
    const OPTIMIZE_PERIOD: Duration = Duration::from_secs(15 * 60);
    loop {
        tokio::time::sleep(OPTIMIZE_PERIOD).await;
        let enabled = Config::load(&state.config_path)
            .ok()
            .and_then(|cfg| cfg.settings.get("libtorrent_auto_optimize").cloned())
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        if !enabled {
            continue;
        }
        match apply_libtorrent_optimization(&state).await {
            Ok((true, memory_mb)) => {
                tracing::info!(memory_mb, "libtorrent continuous optimization applied")
            }
            Ok((false, _)) => {
                tracing::debug!("libtorrent continuous optimization: values already optimal")
            }
            Err(error) => tracing::warn!(%error, "libtorrent continuous optimization failed"),
        }
    }
}

/// True se la cartella contiene almeno un file (anche in sottocartelle).
fn dir_has_files(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir_has_files(&path) {
                return true;
            }
        } else {
            return true;
        }
    }
    false
}

/// Rimuove le cartelle **vuote** rimaste in `libtorrent_temp_dir` (residui di
/// download completati o rimossi). Salta le cartelle dei torrent attivi e quelle
/// troppo recenti, per non disturbare un download appena avviato.
fn cleanup_empty_temp_dirs(cfg: &Config, torrents: &LibtorrentClient) {
    let Some(temp) = cfg.libtorrent_temp_dir.as_deref() else {
        return;
    };
    let live = torrents.list();
    let active = live
        .iter()
        .map(|torrent| torrent.name.clone())
        .collect::<std::collections::HashSet<_>>();
    // Cartelle (e sottocartelle) usate dai torrent attivi: mai toccarle.
    let active_paths = live
        .iter()
        .map(|torrent| std::path::PathBuf::from(torrent.save_path.clone()))
        .collect::<Vec<_>>();
    let Ok(entries) = std::fs::read_dir(temp) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if active.contains(&name) {
            continue;
        }
        if active_paths.iter().any(|active| active.starts_with(&path)) {
            continue;
        }
        if dir_has_files(&path) {
            continue;
        }
        // Solo se è lì da almeno un'ora: evita di rimuovere la cartella di un
        // download appena creato e non ancora popolato.
        if let Ok(modified) = std::fs::metadata(&path).and_then(|meta| meta.modified()) {
            if modified
                .elapsed()
                .map(|elapsed| elapsed.as_secs() < 3600)
                .unwrap_or(true)
            {
                continue;
            }
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!(dir = %path.display(), "removed empty temp folder")
            }
            Err(error) => {
                tracing::debug!(dir = %path.display(), %error, "could not remove empty temp folder")
            }
        }
    }
}

/// Pulizia periodica delle cartelle vuote nella temp libtorrent.
async fn temp_cleanup_worker(state: AppState) {
    const CLEANUP_PERIOD: Duration = Duration::from_secs(30 * 60);
    loop {
        tokio::time::sleep(CLEANUP_PERIOD).await;
        let cfg = latest_config(&state);
        cleanup_empty_temp_dirs(&cfg, &state.torrents);
    }
}

pub async fn serve(
    state: AppState,
    workers: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
) -> anyhow::Result<()> {
    let web_addr: SocketAddr = state.cfg.listen.parse()?;
    let engine_addr: SocketAddr = state.cfg.engine_listen.parse()?;
    let web_listener = tokio::net::TcpListener::bind(web_addr).await?;
    let engine_listener = tokio::net::TcpListener::bind(engine_addr).await?;
    tracing::info!(
        %web_addr,
        %engine_addr,
        dry_run = state.cfg.dry_run,
        active = state.cfg.active,
        libtorrent = %crate::libtorrent::libtorrent_version(),
        "rextto daemon ready"
    );
    let worker = tokio::spawn(torrent_event_worker(
        state.config_path.clone(),
        state.cfg.clone(),
        state.torrents.clone(),
        state.db.clone(),
        state.comics.clone(),
        state.torrent_events.clone(),
    ));
    let cycle = tokio::spawn(cycle_worker(state.clone()));
    let backups = tokio::spawn(backup_worker(state.clone()));
    let optimize = tokio::spawn(optimize_worker(state.clone()));
    let temp_cleanup = tokio::spawn(temp_cleanup_worker(state.clone()));
    // Register the long-lived workers so `main` can stop them *before* it
    // touches the native libtorrent session. They must never be running while
    // `torrents.shutdown` waits for `save_resume_data` alerts, or they would
    // drain those alerts and the fastresume data would be lost.
    {
        let mut registry = workers.lock().unwrap();
        registry.push(worker);
        registry.push(cycle);
        registry.push(backups);
        registry.push(optimize);
        registry.push(temp_cleanup);
    }
    let app = router(state);
    let result = tokio::try_join!(
        axum::serve(web_listener, app.clone()),
        axum::serve(engine_listener, app)
    );
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn torrent_view(state: &str, auto_managed: bool, seed_days: i64) -> crate::models::TorrentView {
        crate::models::TorrentView {
            hash: "h".into(),
            name: "n".into(),
            progress: 100.0,
            state: state.into(),
            download_rate: 0,
            upload_rate: 0,
            save_path: String::new(),
            download_limit: 0,
            upload_limit: 0,
            all_time_upload: 0,
            all_time_download: 0,
            seeding_seconds: 0,
            queue_position: 0,
            num_peers: 0,
            num_seeds: 0,
            seed_ratio: -1.0,
            seed_days,
            has_metadata: true,
            auto_managed,
            torrent_version: String::new(),
            total_size: 0,
            total_done: 0,
        }
    }

    #[test]
    fn infinite_seed_resume_skips_queue_paused_torrents() {
        // Queue-paused by libtorrent (auto_managed=true): leave it alone.
        assert!(!needs_infinite_seed_resume(&torrent_view("paused", true, 0)));
        // Stopped by our seed policy (auto_managed cleared): resume it.
        assert!(needs_infinite_seed_resume(&torrent_view("paused", false, 0)));
        // No infinite override (seed_days>0): never force a resume.
        assert!(!needs_infinite_seed_resume(&torrent_view("paused", false, 3)));
        // Already seeding: nothing to do.
        assert!(!needs_infinite_seed_resume(&torrent_view("seeding", false, 0)));
    }

    #[test]
    fn episode_search_matches_partial_and_complete_season_packs() {
        let series = SeriesConfig {
            name: "Example Show".into(),
            ..Default::default()
        };
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        let partial = crate::parser::parse_release(
            "Example.Show.S01E01-08.1080p.WEB-DL.ITA",
            magnet,
            "test",
        )
        .unwrap();
        let complete = crate::parser::parse_release(
            "Example.Show.S01.1080p.WEB-DL.ITA",
            magnet,
            "test",
        )
        .unwrap();

        assert!(release_matches_series_episode(&partial, &series, 1, 5));
        assert!(release_matches_series_episode(&complete, &series, 1, 5));
        assert!(!release_matches_series_episode(&partial, &series, 1, 9));
    }

    #[test]
    fn stall_timer_resets_on_peers_like_extto() {
        let timeout = Duration::from_secs(3600);
        let t0 = Instant::now();
        // Nessun peer e nessun traffico per oltre il timeout: scade.
        let mut entry = (t0, 0_i64);
        assert!(!stall_expired(
            &mut entry,
            t0 + Duration::from_secs(1800),
            0,
            0,
            0,
            timeout
        ));
        assert!(stall_expired(
            &mut entry,
            t0 + Duration::from_secs(3700),
            0,
            0,
            0,
            timeout
        ));
        // Basta un peer (senza byte nuovi) per azzerare il timer, come EXTTO.
        assert!(!stall_expired(
            &mut entry,
            t0 + Duration::from_secs(3800),
            0,
            0,
            2,
            timeout
        ));
        // Poi di nuovo senza peer per un altro timeout: scade.
        assert!(stall_expired(
            &mut entry,
            t0 + Duration::from_secs(3800 + 3700),
            0,
            0,
            0,
            timeout
        ));
        // Byte nuovi azzerano comunque il timer.
        assert!(!stall_expired(
            &mut entry,
            t0 + Duration::from_secs(8000),
            5,
            0,
            0,
            timeout
        ));
    }

    #[test]
    fn libtorrent_version_comparison_detects_newer_releases() {
        assert!(version_is_newer("2.1.1", "2.0.11.0"));
        assert!(version_is_newer("2.0.14", "2.0.11.0"));
        assert!(!version_is_newer("2.0.11", "2.0.11.0"));
        assert!(!version_is_newer("1.2.19", "2.0.11.0"));
        assert_eq!(version_parts("v2.0.11.0"), vec![2, 0, 11, 0]);
    }

    #[test]
    fn sorts_calendar_items_by_air_date_with_missing_last() {
        let item = |name: &str, date: Option<&str>| {
            serde_json::json!({
                "series": name,
                "episode": {"air_date": date, "season_number": 1, "episode_number": 1}
            })
        };
        let mut items = vec![
            item("Terza", Some("2026-10-05")),
            item("Senza data", None),
            item("Prima", Some("2026-09-22")),
        ];
        sort_calendar_items(&mut items);
        let names = items
            .iter()
            .map(|value| {
                value
                    .get("series")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Prima", "Terza", "Senza data"]);
    }

    #[test]
    fn parses_season_episode_from_archive_filenames() {
        assert_eq!(
            parse_season_episode("Alien Earth - S01E01 - Neverland - [2160p].mkv"),
            Some((1, 1))
        );
        assert_eq!(
            parse_season_episode("FBI Stagione 2 Ep. 03 1080p.mkv"),
            Some((2, 3))
        );
        assert_eq!(parse_season_episode("Show 1x05 ITA.mkv"), Some((1, 5)));
        assert_eq!(
            parse_season_episode("Serie - Stagione 4 Episodio 12.mkv"),
            Some((4, 12))
        );
        assert_eq!(parse_season_episode("film 2025 1080p.mkv"), None);
    }

    #[test]
    fn clear_empty_destination_removes_only_empty_dirs() {
        let root = std::env::temp_dir().join(format!("rextto-cleardst-{}", uuid::Uuid::new_v4()));
        let empty = root.join("Empty");
        std::fs::create_dir_all(&empty).unwrap();
        clear_empty_destination(&root, "Empty");
        assert!(!empty.exists(), "empty destination must be removed");
        let full = root.join("Full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("file.mkv"), b"x").unwrap();
        clear_empty_destination(&root, "Full");
        assert!(full.exists(), "non-empty destination must be kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn test_state() -> (AppState, PathBuf) {
        let root = std::env::temp_dir().join(format!("rextto-web-{}", uuid::Uuid::new_v4()));
        let mut cfg = Config {
            data_dir: root.clone(),
            api_token: Some("test-token".into()),
            ..Config::default()
        };
        cfg.state_dir = root.join("state");
        cfg.libtorrent_dir = root.join("downloads");
        cfg.libtorrent_temp_dir = Some(root.join("incomplete"));
        cfg.import_source_dir = root.join("import-source");
        cfg.prepare_dirs().unwrap();
        let config_path = root.join("rextto.json");
        std::fs::write(&config_path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        let (_layer, log_reload): (_, reload::Handle<EnvFilter, tracing_subscriber::Registry>) =
            reload::Layer::new(EnvFilter::new("rextto=warn"));
        let state = AppState {
            i18n: Arc::new(I18nDb::open(&root.join("rextto_config.db")).unwrap()),
            db: Arc::new(Mutex::new(
                Database::open(&root.join("rextto_series.db")).unwrap(),
            )),
            archive: Arc::new(Mutex::new(
                Archive::open(&root.join("rextto_archive.db")).unwrap(),
            )),
            comics: Arc::new(ComicsDb::open(&root.join("rextto_comics.db")).unwrap()),
            engine: Arc::new(Engine::new()),
            torrents: Arc::new(LibtorrentClient::new(&cfg).unwrap()),
            torrent_events: Arc::new(Mutex::new(Vec::new())),
            notifier: Arc::new(Notifier::from_config(&cfg)),
            tmdb: Arc::new(TmdbClient::new(None)),
            last_cycle: Arc::new(Mutex::new(CycleStats::default())),
            cycle_lock: Arc::new(tokio::sync::Mutex::new(())),
            log_reload: Arc::new(Mutex::new(log_reload)),
            rename_progress: Arc::new(Mutex::new(RenameProgress::default())),
            cfg,
            config_path,
        };
        (state, root)
    }

    #[test]
    fn magnet_feed_builds_rss_with_escaped_magnets() {
        let entries = vec![(
            "Movie & Show <2024>".to_string(),
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=A&B".to_string(),
            "ExtTo - X".to_string(),
        )];
        let xml = build_magnet_feed(&entries);
        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.contains("<title>Movie &amp; Show &lt;2024&gt;</title>"));
        // The magnet is XML-escaped, not raw.
        assert!(xml.contains("&amp;dn=A&amp;B"));
        assert!(xml.contains("[ExtTo - X]"));
        assert!(xml.ends_with("</channel></rss>"));
    }

    #[tokio::test]
    async fn backup_list_returns_string_names() {
        let (state, root) = test_state();
        let backups = root.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("snapshot-1.zip"), b"zip").unwrap();
        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/backup")
                    .header("x-rextto-token", "test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["items"][0]["name"], "snapshot-1.zip");
        // L'etichetta leggibile è presente (data locale del backup).
        assert!(value["items"][0]["label"]
            .as_str()
            .is_some_and(|label| label.contains('/')));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn db_action_vacuum_returns_without_deadlock() {
        let (state, root) = test_state();
        let app = router(state);
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            app.clone().oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/db/action")
                    .header("x-rextto-token", "test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"action":"vacuum"}"#))
                    .unwrap(),
            ),
        )
        .await
        .expect("VACUUM must not deadlock the daemon")
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn api_enforces_auth_and_persists_an_allowed_setting() {
        let (state, root) = test_state();
        let app = router(state);
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config/settings")
                    .header("x-rextto-token", "test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"key":"refresh_interval","value":"900"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::OK);
        let config = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .header("x-rextto-token", "test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(config.status(), StatusCode::OK);
        let body = to_bytes(config.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["refresh_secs"],
            900
        );
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn score_preview_no_rename_and_language_endpoints() {
        let (state, root) = test_state();
        let app = router(state);
        let post = |uri: &'static str, body: &'static str| {
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("x-rextto-token", "test-token")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap()
        };

        let preview = app
            .clone()
            .oneshot(post(
                "/api/score/preview",
                r#"{"title":"Example.Show.S01E01.1080p.WEB-DL.H264.DDP5.1.ITA"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        let body = to_bytes(preview.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["kind"], "series");
        assert_eq!(value["season"], 1);
        assert_eq!(value["episode"], 1);
        assert!(value["score"].as_i64().unwrap() > 1000);
        assert!(value["breakdown"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        let rename = app
            .clone()
            .oneshot(post("/api/torrents/abcdef/no_rename", r#"{"value":true}"#))
            .await
            .unwrap();
        assert_eq!(rename.status(), StatusCode::OK);
        let body = to_bytes(rename.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["no_rename"],
            true
        );

        let language = app
            .clone()
            .oneshot(post("/api/i18n/active", r#"{"lang":"en"}"#))
            .await
            .unwrap();
        assert_eq!(language.status(), StatusCode::OK);
        let active = app
            .oneshot(
                Request::builder()
                    .uri("/api/i18n/active")
                    .header("x-rextto-token", "test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(active.status(), StatusCode::OK);
        let body = to_bytes(active.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["lang"],
            "en"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn backup_due_interval_and_daily_time() {
        use chrono::{Duration as ChronoDuration, Local};
        let mut cfg = Config::default();
        // Nessuna schedulazione.
        assert!(!backup_due(&cfg, None));
        // Intervallo in ore: mai fatto -> subito; recente -> no; vecchio -> sì.
        cfg.settings.insert("backup_schedule_hours".into(), "24".into());
        assert!(backup_due(&cfg, None));
        assert!(!backup_due(&cfg, Some(Local::now() - ChronoDuration::hours(1))));
        assert!(backup_due(&cfg, Some(Local::now() - ChronoDuration::hours(25))));
        // Orario giornaliero con precedenza: a mezzanotte è sempre "passato".
        cfg.settings.insert("backup_schedule_at".into(), "00:00".into());
        assert!(backup_due(&cfg, None));
        assert!(!backup_due(&cfg, Some(Local::now())));
        assert!(backup_due(&cfg, Some(Local::now() - ChronoDuration::days(1))));
        // Orario non valido: ricade sull'intervallo (recente -> no).
        cfg.settings.insert("backup_schedule_at".into(), "boh".into());
        assert!(!backup_due(&cfg, Some(Local::now() - ChronoDuration::hours(1))));
    }
}
