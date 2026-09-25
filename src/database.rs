use crate::models::{Release, TorrentMeta};
use crate::parser::parse_quality;
use crate::utils::magnet_hash;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::path::Path;

const DOWNLOAD_HISTORY_RETENTION_DAYS: i64 = 30;

pub type ReadyPending = (String, String, String, i64, i64);
pub type ReadyPendingMovie = (String, String, String, Option<i64>);
pub type SeriesSummary = (String, i64, i64, Option<String>);

pub struct Database {
    pub conn: Connection,
}

/// Real size and default quality score of a file on disk, derived from its
/// name. Missing files yield zeros so callers can keep the stored values.
fn file_stats(path: &str) -> (i64, i64) {
    let size_bytes = std::fs::metadata(path)
        .map(|value| value.len().min(i64::MAX as u64) as i64)
        .unwrap_or(0);
    let quality_score = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(parse_quality)
        .map(|quality| quality.score())
        .unwrap_or(0);
    (size_bytes, quality_score)
}

/// Runs `VACUUM`/`ANALYZE` on an arbitrary connection (used for every Rextto
/// database, not just the series one).
pub fn optimize_connection(conn: &Connection, action: &str) -> Result<()> {
    let statement = match action {
        "vacuum" => "VACUUM;",
        "analyze" => "ANALYZE;",
        other => anyhow::bail!("unsupported database action: {other}"),
    };
    conn.execute_batch(statement)?;
    Ok(())
}

/// Estimated database size in bytes (page_count × page_size).
pub fn connection_size_bytes(conn: &Connection) -> i64 {
    let pages = conn
        .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let page_size = conn
        .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    pages.saturating_mul(page_size)
}

#[derive(Debug, serde::Serialize)]
pub struct MaintenanceReport {
    pub rescored: usize,
    pub old_cycles_removed: usize,
    pub stale_torrents_removed: usize,
}

/// Retention knobs for a housekeeping run. `0` disables the corresponding
/// cleanup so the user can keep as much history as they want.
#[derive(Debug, Clone)]
pub struct HousekeepingParams {
    pub retain_cycles: i64,
    pub error_age_days: i64,
    pub seen_days: i64,
    pub gap_log_days: i64,
    pub upgrade_backup_days: i64,
    pub history_days: i64,
}

impl Default for HousekeepingParams {
    fn default() -> Self {
        Self {
            retain_cycles: 200,
            error_age_days: 7,
            seen_days: 30,
            gap_log_days: 30,
            upgrade_backup_days: 30,
            history_days: 0,
        }
    }
}

impl HousekeepingParams {
    pub fn from_settings(settings: &std::collections::BTreeMap<String, String>) -> Self {
        let number = |key: &str, default: i64| {
            settings
                .get(key)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(default)
        };
        let defaults = Self::default();
        Self {
            retain_cycles: number("housekeeping_retain_cycles", defaults.retain_cycles)
                .clamp(1, 100_000),
            error_age_days: number("housekeeping_error_age_days", defaults.error_age_days).max(1),
            seen_days: number("housekeeping_seen_days", defaults.seen_days).max(0),
            gap_log_days: number("housekeeping_gap_log_days", defaults.gap_log_days).max(0),
            upgrade_backup_days: number(
                "housekeeping_upgrade_backup_days",
                defaults.upgrade_backup_days,
            )
            .max(0),
            history_days: number("housekeeping_history_days", defaults.history_days).max(0),
        }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HousekeepingReport {
    pub old_cycles_removed: usize,
    pub stale_torrents_removed: usize,
    pub seen_removed: usize,
    pub gap_logs_removed: usize,
    pub upgrade_backups_removed: usize,
    pub old_history_removed: usize,
    pub stale_providers_removed: usize,
}

/// Counts of what a maintenance run *would* remove, so the UI can preview a
/// destructive prune before committing it. Mirrors `cleanup` exactly.
#[derive(Debug, Default, serde::Serialize)]
pub struct PrunePreview {
    pub old_cycles_to_remove: usize,
    pub stale_torrents_to_remove: usize,
    pub seen_to_remove: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct StoredTorrent {
    pub hash: String,
    pub name: String,
    pub tag: String,
    pub source: String,
    pub progress: f64,
    pub paused: bool,
    pub total_size: i64,
    pub downloaded: i64,
    pub status: String,
    pub updated_at: String,
    pub kind: String,
    pub series_name: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    pub quality_score: i64,
    pub completed_at: String,
    /// Percorso in libreria/NAS dove la release è stata archiviata (vuoto se non archiviata).
    pub processed_path: String,
    pub error: String,
    /// Motivo per cui la release era stata messa in download (`approved`, `upgrade`, ...).
    pub reason: String,
}

fn stored_torrent_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTorrent> {
    Ok(StoredTorrent {
        hash: row.get(0)?,
        name: row.get(1)?,
        tag: row.get(2)?,
        source: row.get(3)?,
        progress: row.get(4)?,
        paused: row.get::<_, i64>(5)? != 0,
        total_size: row.get(6)?,
        downloaded: row.get(7)?,
        status: row.get(8)?,
        updated_at: row.get(9)?,
        kind: row.get(10)?,
        series_name: row.get(11)?,
        season: row.get(12)?,
        episode: row.get(13)?,
        year: row.get(14)?,
        quality_score: row.get(15)?,
        completed_at: row.get(16)?,
        processed_path: row.get(17)?,
        error: row.get(18)?,
        reason: row.get(19)?,
    })
}

/// Nome del file rinominato in libreria, ricavato dal percorso archiviato.
/// Ritorna vuoto per percorsi vuoti o per le cartelle (import vecchi), così la
/// UI ricade sul titolo originale della release.
fn renamed_file_title(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return String::new();
    }
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    let lower = base.to_ascii_lowercase();
    let is_media = [".mkv", ".mp4", ".avi", ".m4v", ".ts", ".mov", ".wmv", ".flv"]
        .iter()
        .any(|extension| lower.ends_with(extension));
    if !is_media {
        return String::new();
    }
    base.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| base.to_string())
}

/// Qualità "riconoscibile" da un testo (titolo release o nome file): `None` se
/// non contiene alcun token utile, così il ricalcolo non azzera gli score.
fn meaningful_quality(text: &str) -> Option<crate::models::Quality> {
    let quality = parse_quality(text);
    if quality.resolution != "unknown"
        || quality.source != "unknown"
        || quality.codec != "unknown"
        || quality.audio != "unknown"
        || !quality.hdr.is_empty()
        || quality.score() > 0
    {
        Some(quality)
    } else {
        None
    }
}

#[derive(Debug, serde::Serialize)]
pub struct EpisodeView {
    pub id: i64,
    pub series_name: String,
    pub season: i64,
    pub episode: i64,
    pub title: String,
    /// Data di messa in onda/pianificata, memorizzata dalla sincronizzazione
    /// TMDB per non interrogare il provider a ogni apertura del dettaglio.
    pub air_date: String,
    /// Nome del file rinominato in libreria (da `archive_path`), vuoto quando il
    /// file non è ancora archiviato o il percorso è una cartella.
    pub renamed_title: String,
    pub quality_score: i64,
    pub downloaded_at: Option<String>,
    pub archive_path: Option<String>,
    pub size_bytes: i64,
    pub magnet_hash: Option<String>,
    pub magnet_link: Option<String>,
    pub status: String,
    pub error: String,
    pub ignored: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct MovieHistory {
    pub id: i64,
    pub name: String,
    pub year: Option<i64>,
    pub title: String,
    pub quality_score: i64,
    pub downloaded_at: Option<String>,
    pub size_bytes: i64,
    pub magnet_hash: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct DailyConsumption {
    pub date: String,
    pub bytes: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ConsumptionStats {
    pub total_bytes: i64,
    pub last_30_days_bytes: i64,
    pub last_7_days_bytes: i64,
    pub daily_7d: Vec<DailyConsumption>,
}

#[derive(Debug, serde::Serialize)]
pub struct RecentDownload {
    pub kind: String,
    pub name: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub year: Option<i64>,
    pub downloaded_at: String,
    pub size_bytes: i64,
    pub archive_path: Option<String>,
    pub quality_score: i64,
}

/// Gruppo di release "viste nei feed" (stesso titolo, eventualmente con id TMDB).
#[derive(Debug, serde::Serialize)]
pub struct FeedSeenGroup {
    pub group_key: String,
    pub group_name: String,
    pub year: i64,
    pub season: i64,
    pub count: i64,
    pub best_score: i64,
    pub best_resolution: String,
    pub latest_found: String,
    pub first_found: String,
}

/// Singola release "vista nei feed".
#[derive(Debug, serde::Serialize)]
pub struct FeedSeenEntry {
    pub id: i64,
    pub title: String,
    pub name: String,
    pub year: i64,
    pub season: i64,
    pub episode: i64,
    pub resolution: String,
    pub codec: String,
    pub audio: String,
    pub quality_score: i64,
    pub magnet: String,
    pub source: String,
    pub found_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UpgradeBackup {
    kind: String,
    row_id: i64,
    series_id: Option<i64>,
    series_name: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    name: Option<String>,
    year: Option<i64>,
    title: String,
    quality_score: i64,
    magnet_hash: Option<String>,
    magnet_link: Option<String>,
    downloaded_at: Option<String>,
    archive_path: Option<String>,
    size_bytes: i64,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        // Older imports created torrent_meta before lifecycle state existed. Add it
        // before the schema batch creates its index, otherwise startup fails.
        let has_torrent_meta: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='torrent_meta')",
            [],
            |row| row.get(0),
        )?;
        if has_torrent_meta {
            let has_status: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('torrent_meta') WHERE name='status')", [], |row| row.get(0))?;
            if !has_status {
                self.conn.execute(
                    "ALTER TABLE torrent_meta ADD COLUMN status TEXT NOT NULL DEFAULT 'queued'",
                    [],
                )?;
            }
        }
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS series (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, seasons TEXT DEFAULT '1+', quality TEXT DEFAULT '', language TEXT DEFAULT 'ita', enabled INTEGER DEFAULT 1, archive_path TEXT DEFAULT '', tmdb_id TEXT DEFAULT '', aliases TEXT DEFAULT ''); CREATE TABLE IF NOT EXISTS episodes (id INTEGER PRIMARY KEY, series_id INTEGER NOT NULL, season INTEGER NOT NULL, episode INTEGER NOT NULL, title TEXT, quality_score INTEGER NOT NULL DEFAULT 0, is_repack INTEGER DEFAULT 0, magnet_hash TEXT UNIQUE, magnet_link TEXT, downloaded_at TEXT, archive_path TEXT, size_bytes INTEGER DEFAULT 0, original_title TEXT, rename_verified INTEGER DEFAULT 0, UNIQUE(series_id, season, episode)); CREATE TABLE IF NOT EXISTS movies (id INTEGER PRIMARY KEY, name TEXT, year INTEGER, title TEXT, quality_score INTEGER DEFAULT 0, magnet_hash TEXT UNIQUE, magnet_link TEXT, downloaded_at TEXT, size_bytes INTEGER DEFAULT 0, removed_at TEXT); CREATE TABLE IF NOT EXISTS pending_downloads (id INTEGER PRIMARY KEY, series_id INTEGER, season INTEGER, episode INTEGER, best_magnet TEXT, best_quality_score INTEGER, ready_at TEXT); CREATE TABLE IF NOT EXISTS cycle_history (id INTEGER PRIMARY KEY, at TEXT NOT NULL, payload_json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS torrent_meta (hash TEXT PRIMARY KEY, tag TEXT DEFAULT '', source TEXT DEFAULT '', ui_state TEXT DEFAULT '', progress REAL DEFAULT 0, paused INTEGER DEFAULT 0, total_size INTEGER DEFAULT 0, downloaded INTEGER DEFAULT 0, name TEXT DEFAULT '', kind TEXT DEFAULT '', title TEXT DEFAULT '', series_name TEXT DEFAULT '', season INTEGER, episode INTEGER, year INTEGER, quality_score INTEGER DEFAULT 0, metadata_json TEXT DEFAULT '', status TEXT NOT NULL DEFAULT 'queued', completed_at TEXT, processed_path TEXT, error TEXT DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL); CREATE INDEX IF NOT EXISTS idx_episodes_lookup ON episodes(series_id, season, episode); CREATE INDEX IF NOT EXISTS idx_episodes_magnet ON episodes(magnet_hash); CREATE INDEX IF NOT EXISTS idx_episodes_downloaded ON episodes(downloaded_at); CREATE INDEX IF NOT EXISTS idx_movies_magnet ON movies(magnet_hash); CREATE INDEX IF NOT EXISTS idx_movies_removed ON movies(removed_at); CREATE INDEX IF NOT EXISTS idx_torrent_meta_status ON torrent_meta(status); INSERT INTO schema_meta(key,value,updated_at) VALUES ('schema_version','2',datetime('now')) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at;")?;
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS gap_search_log (series_name TEXT NOT NULL, season INTEGER NOT NULL, episode INTEGER NOT NULL, last_searched_at TEXT NOT NULL, PRIMARY KEY(series_name,season,episode)); CREATE TABLE IF NOT EXISTS series_metadata (series_name TEXT NOT NULL, season INTEGER NOT NULL, episode_count INTEGER NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(series_name,season)); CREATE TABLE IF NOT EXISTS episode_metadata (series_name TEXT NOT NULL, season INTEGER NOT NULL, episode INTEGER NOT NULL, air_date TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL, PRIMARY KEY(series_name,season,episode)); CREATE TABLE IF NOT EXISTS ignored_episodes (series_name TEXT NOT NULL, season INTEGER NOT NULL, episode INTEGER NOT NULL, reason TEXT DEFAULT '', created_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY(series_name,season,episode)); CREATE TABLE IF NOT EXISTS upgrade_backup (new_hash TEXT PRIMARY KEY, payload_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now'))); CREATE TABLE IF NOT EXISTS series_status (series_name TEXT PRIMARY KEY, status TEXT NOT NULL DEFAULT '', last_air_date TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL);")?;
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS blocklist (magnet_hash TEXT PRIMARY KEY, title TEXT DEFAULT '', reason TEXT DEFAULT '', created_at TEXT NOT NULL);")?;
        // Movies held back by a delay profile (the series equivalent lives in
        // `pending_downloads`, extended below with a precise `due_at`).
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_movies (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                year INTEGER,
                best_magnet TEXT,
                best_title TEXT,
                best_quality_score INTEGER DEFAULT 0,
                first_seen_at TEXT,
                delay_hours INTEGER DEFAULT 0,
                due_at TEXT,
                status TEXT DEFAULT 'pending',
                downloaded_at TEXT,
                UNIQUE(name, year)
            );",
        )?;
        // Escalating provider backoff (feeds, indexers): a repeatedly failing
        // source is disabled for a growing interval instead of retried on every
        // cycle. Sonarr's `EscalationBackOff` is the reference behaviour.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS provider_status (
                provider TEXT NOT NULL,
                kind TEXT NOT NULL,
                level INTEGER NOT NULL DEFAULT 0,
                initial_failure TEXT,
                most_recent_failure TEXT,
                disabled_till TEXT,
                last_error TEXT DEFAULT '',
                PRIMARY KEY(provider, kind)
            );",
        )?;
        // Identità della release bloccata (come il legacy `download_blocklist`):
        // hash + serie/stagione/episodio o film/anno, per audit e UI.
        for statement in [
            "ALTER TABLE blocklist ADD COLUMN kind TEXT DEFAULT ''",
            "ALTER TABLE blocklist ADD COLUMN series_name TEXT DEFAULT ''",
            "ALTER TABLE blocklist ADD COLUMN season INTEGER",
            "ALTER TABLE blocklist ADD COLUMN episode INTEGER",
            "ALTER TABLE blocklist ADD COLUMN movie_name TEXT DEFAULT ''",
            "ALTER TABLE blocklist ADD COLUMN movie_year INTEGER",
        ] {
            let _ = self.conn.execute(statement, []);
        }
        // "Visti nei feed": tutte le release che passano dalle sorgenti, non solo
        // quelle in libreria. Tabelle separate per film e serie, con chiave di
        // raggruppamento calcolata in Rust per unire le release dello stesso titolo.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS movie_feed_seen (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL UNIQUE,
                name TEXT,
                year INTEGER DEFAULT 0,
                resolution TEXT DEFAULT 'unknown',
                codec TEXT DEFAULT 'unknown',
                audio TEXT DEFAULT 'unknown',
                quality_score INTEGER DEFAULT 0,
                magnet TEXT,
                source TEXT,
                found_at TEXT NOT NULL,
                first_seen_at TEXT,
                group_key TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_mfs_found ON movie_feed_seen(found_at DESC);
            CREATE INDEX IF NOT EXISTS idx_mfs_group ON movie_feed_seen(group_key);
            CREATE TABLE IF NOT EXISTS series_feed_seen (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL UNIQUE,
                name TEXT,
                season INTEGER DEFAULT 0,
                episode INTEGER DEFAULT 0,
                resolution TEXT DEFAULT 'unknown',
                codec TEXT DEFAULT 'unknown',
                audio TEXT DEFAULT 'unknown',
                quality_score INTEGER DEFAULT 0,
                magnet TEXT,
                source TEXT,
                found_at TEXT NOT NULL,
                first_seen_at TEXT,
                group_key TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sfs_found ON series_feed_seen(found_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sfs_group ON series_feed_seen(group_key);",
        )?;
        for statement in [
            "ALTER TABLE pending_downloads ADD COLUMN best_title TEXT DEFAULT ''",
            "ALTER TABLE pending_downloads ADD COLUMN first_seen_at TEXT",
            "ALTER TABLE pending_downloads ADD COLUMN timeframe_hours INTEGER DEFAULT 0",
            "ALTER TABLE pending_downloads ADD COLUMN status TEXT DEFAULT 'pending'",
            "ALTER TABLE pending_downloads ADD COLUMN downloaded_at TEXT",
            "ALTER TABLE pending_downloads ADD COLUMN due_at TEXT",
            "ALTER TABLE torrent_meta ADD COLUMN metadata_json TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN ui_state TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN progress REAL DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN paused INTEGER DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN total_size INTEGER DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN downloaded INTEGER DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN name TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN completed_at TEXT",
            "ALTER TABLE torrent_meta ADD COLUMN processed_path TEXT",
            "ALTER TABLE torrent_meta ADD COLUMN kind TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN title TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN series_name TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN season INTEGER",
            "ALTER TABLE torrent_meta ADD COLUMN episode INTEGER",
            "ALTER TABLE torrent_meta ADD COLUMN year INTEGER",
            "ALTER TABLE torrent_meta ADD COLUMN quality_score INTEGER DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN status TEXT NOT NULL DEFAULT 'queued'",
            "ALTER TABLE torrent_meta ADD COLUMN error TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN removed_at TEXT",
            "ALTER TABLE torrent_meta ADD COLUMN reason TEXT DEFAULT ''",
            "ALTER TABLE torrent_meta ADD COLUMN no_rename INTEGER DEFAULT 0",
            "ALTER TABLE torrent_meta ADD COLUMN created_at TEXT",
            "ALTER TABLE series ADD COLUMN timeframe INTEGER DEFAULT 0",
            "ALTER TABLE series ADD COLUMN ignored_seasons TEXT DEFAULT '[]'",
            "ALTER TABLE series ADD COLUMN subtitle TEXT DEFAULT ''",
            "ALTER TABLE series ADD COLUMN season_subfolders INTEGER DEFAULT 0",
            "ALTER TABLE series ADD COLUMN exclude TEXT DEFAULT ''",
            "ALTER TABLE episodes ADD COLUMN media_info_json TEXT DEFAULT ''",
            "ALTER TABLE movies ADD COLUMN media_info_json TEXT DEFAULT ''",
        ] {
            let _ = self.conn.execute(statement, []);
        }
        // I torrent già rimossi prima dell'introduzione di `removed_at` devono
        // comunque comparire nello "Storico download". Non si toccano le righe
        // concluse ma mai rimosse, che restano in Sessione.
        let _ = self.conn.execute(
            "UPDATE torrent_meta SET removed_at=COALESCE(NULLIF(completed_at,''), updated_at) WHERE status='removed' AND removed_at IS NULL",
            [],
        );
        // Le righe registrate prima di salvare la sorgente hanno il dato dentro
        // `metadata_json`: lo si riporta nella colonna dedicata.
        let _ = self.conn.execute(
            "UPDATE torrent_meta SET source=COALESCE(json_extract(metadata_json,'$.release.source'),'') WHERE COALESCE(source,'')='' AND COALESCE(metadata_json,'')<>''",
            [],
        );
        Ok(())
    }

    /// Default upgrade threshold used by callers without a `Config`.
    pub const DEFAULT_UPGRADE_MIN_SCORE_DIFF: i64 = 200;

    pub fn check_series(&self, release: &Release) -> Result<(bool, String)> {
        self.check_series_scored_inner(
            release,
            release.quality.score(),
            Self::DEFAULT_UPGRADE_MIN_SCORE_DIFF,
            &crate::models::ApprovalContext::default(),
            false,
        )
    }

    pub fn check_series_scored(
        &self,
        release: &Release,
        score: i64,
        min_score_diff: i64,
        context: &crate::models::ApprovalContext,
    ) -> Result<(bool, String)> {
        self.check_series_scored_inner(release, score, min_score_diff, context, false)
    }

    /// Approvazione dell'azione esplicita “Accoda”. Per questa azione un
    /// download incompleto (anche presente nel client) non equivale a una copia:
    /// blocchiamo solo una release già archiviata sul NAS, lo stesso hash attivo
    /// o una release in blacklist. Il ciclo automatico mantiene i controlli
    /// conservativi di `check_series_scored`.
    pub fn check_series_manual_scored(
        &self,
        release: &Release,
        score: i64,
        min_score_diff: i64,
        context: &crate::models::ApprovalContext,
    ) -> Result<(bool, String)> {
        self.check_series_scored_inner(release, score, min_score_diff, context, true)
    }

    fn check_series_scored_inner(
        &self,
        release: &Release,
        score: i64,
        min_score_diff: i64,
        context: &crate::models::ApprovalContext,
        manual: bool,
    ) -> Result<(bool, String)> {
        let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        if self.is_blocklisted(&hash)? {
            return Ok((false, "blocklisted".into()));
        }
        // Protezione download attivi (parità col client live del legacy): mai
        // riproporre un hash già nella sessione.
        if context.live.hashes.contains(&hash) {
            return Ok((false, "active_episode".into()));
        }
        if release.is_pack {
            return self.check_series_pack(release, &hash, score, min_score_diff, context, manual);
        }
        let season = release.season.context("series release has no season")?;
        let episode = release.episode.context("series release has no episode")?;
        let series_name = release.series.as_deref().unwrap_or(&release.title);
        self.conn.execute(
            "INSERT OR IGNORE INTO series(name) VALUES (?1)",
            params![series_name],
        )?;
        let sid: i64 = self.conn.query_row(
            "SELECT id FROM series WHERE name = ?1",
            params![series_name],
            |r| r.get(0),
        )?;
        if !manual && context.live.episodes.contains(&(
            crate::parser::normalize_series_name(series_name),
            season,
            episode,
        )) {
            return Ok((false, "active_episode".into()));
        }
        if !manual && self.conn.query_row("SELECT EXISTS(SELECT 1 FROM torrent_meta WHERE lower(series_name)=lower(?1) AND season=?2 AND episode=?3 AND status NOT IN ('completed','error','removed'))", params![series_name, season, episode], |row| row.get::<_, bool>(0))? { return Ok((false, "active_episode".into())); }
        // Core best practice (always on, no user option): never fetch an older
        // episode that is not a recognised gap when a later episode or season is
        // already archived. A genuine quality upgrade over the later archived
        // episode, while still below the profile cutoff, is accepted; gap-fill
        // and manual actions are always exempt. This is what keeps the library
        // monotonic without ever blocking a deliberate backfill.
        if !manual && !context.gap_episode {
            let archived_here: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE series_id=?1 AND season=?2 AND episode=?3 AND (downloaded_at IS NOT NULL OR COALESCE(archive_path,'')<>''))",
                params![sid, season, episode],
                |row| row.get(0),
            )?;
            if !archived_here {
                let later_exists: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM episodes e JOIN series s ON s.id=e.series_id WHERE s.name=?1 AND (e.season > ?2 OR (e.season = ?2 AND e.episode > ?3)) AND (e.downloaded_at IS NOT NULL OR COALESCE(e.archive_path,'')<>''))",
                    params![series_name, season, episode],
                    |row| row.get(0),
                )?;
                if later_exists {
                    // Accept only a genuine quality upgrade over the best later
                    // archived episode; anything equal or worse stays blocked.
                    let candidate_rank = release.quality.resolution_rank();
                    let later_rank = self
                        .later_archived_max_resolution_rank(series_name, season, episode)?;
                    let is_upgrade =
                        later_rank.is_some_and(|later_rank| candidate_rank > later_rank);
                    if !is_upgrade {
                        return Ok((false, "smart_episode".into()));
                    }
                }
            }
        }
        // Considera duplicato solo un episodio già scaricato/archiviato: una
        // riga placeholder (downloaded_at NULL) non deve impedire un retry.
        let same_hash_is_archived: bool = self.conn.query_row(
            if manual {
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE magnet_hash=?1 AND COALESCE(archive_path,'') <> '')"
            } else {
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE magnet_hash=?1 AND (downloaded_at IS NOT NULL OR COALESCE(archive_path,'') <> ''))"
            },
            [&hash],
            |row| row.get::<_, bool>(0),
        )?;
        if same_hash_is_archived {
            return Ok((false, "duplicate".into()));
        }
        let db_row: Option<(i64, i64, String, String)> = self.conn.query_row("SELECT id,quality_score,COALESCE(title,''),COALESCE(archive_path,'') FROM episodes WHERE series_id=?1 AND season=?2 AND episode=?3", params![sid, season, episode], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))).optional()?;
        // Intelligenza archivio (parità col legacy `_best_quality_in_path`): se
        // su disco c'è già un file di qualità uguale o superiore, non scaricare
        // anche se il DB non lo conosce. Se il file su disco è inferiore, il
        // candidato resta valido.
        if let Some((disk_quality, disk_score)) = context.archive.best_for(season, episode) {
            if manual {
                return Ok((false, "duplicate".into()));
            }
            let db_score = db_row
                .as_ref()
                .map(|(_, value, _, _)| *value)
                .unwrap_or(i64::MIN);
            if *disk_score > db_score
                && release
                    .quality
                    .upgrade_reason(disk_quality, score, *disk_score, min_score_diff)
                    .is_none()
            {
                return Ok((false, "duplicate".into()));
            }
        }
        if let Some((id, existing_score, existing_title, archive_path)) = db_row {
            if manual && !archive_path.is_empty() {
                return Ok((false, "duplicate".into()));
            }
            // Quality profile cutoff reached: a real archived file is never
            // replaced. Only enforced when the file exists on disk, so a
            // placeholder never blocks the first download.
            if context.forbid_upgrade && context.archive.best_for(season, episode).is_some() {
                return Ok((false, "upgrades_disabled".into()));
            }
            // legacy upgrade_reason: resolution jump, HDTV→WEB-DL, HDR, first
            // REPACK, or a score gain of at least `min_score_diff`.
            let old_quality = parse_quality(&existing_title);
            if !manual && release
                .quality
                .upgrade_reason(&old_quality, score, existing_score, min_score_diff)
                .is_none()
            {
                return Ok((false, "duplicate".into()));
            }
            let previous = self.conn.query_row("SELECT id,series_id,season,episode,title,quality_score,magnet_hash,magnet_link,downloaded_at,archive_path,size_bytes FROM episodes WHERE id=?1", [id], |row| Ok(UpgradeBackup { kind: "series".into(), row_id: row.get(0)?, series_id: Some(row.get(1)?), series_name: Some(series_name.to_owned()), season: Some(row.get(2)?), episode: Some(row.get(3)?), name: None, year: None, title: row.get::<_, Option<String>>(4)?.unwrap_or_default(), quality_score: row.get(5)?, magnet_hash: row.get(6)?, magnet_link: row.get(7)?, downloaded_at: row.get(8)?, archive_path: row.get(9)?, size_bytes: row.get(10)? }))?;
            self.save_upgrade_backup(&hash, &previous)?;
            // Se l'hash è già usato da un altro episodio (es. placeholder di un
            // pack), non riassegnarlo: mantieni quello esistente.
            let hash_taken_elsewhere: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE magnet_hash=?1 AND NOT (series_id=?2 AND season=?3 AND episode=?4))",
                params![hash, sid, season, episode],
                |row| row.get(0),
            )?;
            let episode_hash = if hash_taken_elsewhere { None } else { Some(hash.as_str()) };
            self.conn.execute("UPDATE episodes SET title=?1,quality_score=?2,magnet_hash=COALESCE(?3,magnet_hash),magnet_link=?4,downloaded_at=NULL,archive_path=NULL WHERE id=?5", params![release.title, score, episode_hash, release.magnet, id])?;
            return Ok((true, "upgrade".into()));
        }
        let hash_taken_elsewhere: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM episodes WHERE magnet_hash=?1)",
            [&hash],
            |row| row.get(0),
        )?;
        let episode_hash = if hash_taken_elsewhere { None } else { Some(hash.as_str()) };
        self.conn.execute("INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![sid, season, episode, release.title, score, episode_hash, release.magnet])?;
        Ok((true, "approved".into()))
    }

    /// Approvazione di un season pack: per un pack completo espande gli episodi
    /// bersaglio a quelli già noti della stagione (e al conteggio TMDB) e
    /// confronta la qualità **episodio per episodio**, invece di limitarsi a
    /// controllare l'esistenza. Così un pack inferiore ai file già presenti
    /// viene rifiutato (`duplicate`) e non finisce nel client.
    fn check_series_pack(
        &self,
        release: &Release,
        hash: &str,
        score: i64,
        min_score_diff: i64,
        context: &crate::models::ApprovalContext,
        manual: bool,
    ) -> Result<(bool, String)> {
        let season = release.season.context("season pack has no season")?;
        let series_name = release.series.as_deref().unwrap_or(&release.title);
        let explicit: Vec<i64> = release
            .episode_range
            .iter()
            .copied()
            .filter(|episode| *episode > 0)
            .collect();
        let complete = release.episode_range.contains(&0) || explicit.is_empty();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO series(name) VALUES (?1)",
            params![series_name],
        )?;
        let series_id: i64 = tx.query_row(
            "SELECT id FROM series WHERE name=?1",
            params![series_name],
            |row| row.get(0),
        )?;
        // Duplicato solo se il pack è già stato scaricato/archiviato: i
        // placeholder non devono impedire un secondo tentativo (retry).
        let same_pack_is_archived: bool = tx.query_row(
            if manual {
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE (magnet_hash=?1 OR magnet_link=?2) AND COALESCE(archive_path,'') <> '')"
            } else {
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE (magnet_hash=?1 OR magnet_link=?2) AND (downloaded_at IS NOT NULL OR COALESCE(archive_path,'') <> ''))"
            },
            params![hash, release.magnet],
            |row| row.get::<_, bool>(0),
        )?;
        if same_pack_is_archived {
            return Ok((false, "duplicate".into()));
        }
        let mut targets: Vec<i64> = explicit.clone();
        if complete {
            let mut statement = tx.prepare(
                "SELECT DISTINCT episode FROM episodes WHERE series_id=?1 AND season=?2 AND episode>0",
            )?;
            let rows = statement.query_map(params![series_id, season], |row| row.get::<_, i64>(0))?;
            for episode in rows {
                targets.push(episode?);
            }
            if let Ok(count) = tx.query_row(
                "SELECT episode_count FROM series_metadata WHERE series_name=?1 AND season=?2",
                params![series_name, season],
                |row| row.get::<_, i64>(0),
            ) {
                if count > 0 && count <= 500 {
                    targets.extend(1..=count);
                }
            }
        }
        targets.retain(|episode| *episode > 0);
        targets.sort_unstable();
        targets.dedup();

        let mut inserted = 0usize;
        let mut upgraded = 0usize;
        // L'hash del pack va assegnato a un solo episodio: se è già presente in
        // tabella (es. il primo episodio di un tentativo precedente) non va
        // riassegnato, altrimenti si viola il vincolo UNIQUE.
        let mut hash_available = !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM episodes WHERE magnet_hash=?1)",
            [hash],
            |row| row.get::<_, bool>(0),
        )?;
        for episode in &targets {
            // Non toccare gli episodi con un download ancora attivo (DB o sessione).
            let active = !manual && (tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM torrent_meta WHERE lower(series_name)=lower(?1) AND season=?2 AND episode=?3 AND status NOT IN ('completed','error','removed'))",
                params![series_name, season, episode],
                |row| row.get::<_, bool>(0),
            ).unwrap_or(false)
                || context.live.episodes.contains(&(
                    crate::parser::normalize_series_name(series_name),
                    season,
                    *episode,
                )));
            if active {
                continue;
            }
            let existing: Option<(i64, i64, String, String)> = tx.query_row(
                "SELECT id,quality_score,COALESCE(title,''),COALESCE(archive_path,'') FROM episodes WHERE series_id=?1 AND season=?2 AND episode=?3",
                params![series_id, season, episode],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).optional()?;
            match existing {
                None => {
                    // File già su disco (non nel DB) con qualità uguale o
                    // superiore: episodio coperto, inutile scaricare il pack.
                    if let Some((disk_quality, disk_score)) = context.archive.best_for(season, *episode) {
                        if release
                            .quality
                            .upgrade_reason(disk_quality, score, *disk_score, min_score_diff)
                            .is_none()
                        {
                            continue;
                        }
                    }
                    let episode_hash = if hash_available {
                        hash_available = false;
                        Some(hash)
                    } else {
                        None
                    };
                    tx.execute("INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![series_id, season, episode, release.title, score, episode_hash, release.magnet])?;
                    inserted += 1;
                }
                Some((id, existing_score, existing_title, archive_path)) => {
                    if manual && (!archive_path.is_empty() || context.archive.best_for(season, *episode).is_some()) {
                        continue;
                    }
                    // Confronta col migliore tra la riga DB e il file su disco.
                    let (old_quality, old_score) = match context.archive.best_for(season, *episode) {
                        Some((disk_quality, disk_score)) if *disk_score > existing_score => {
                            (disk_quality.clone(), *disk_score)
                        }
                        _ => (parse_quality(&existing_title), existing_score),
                    };
                    if manual
                        || (release
                            .quality
                            .upgrade_reason(&old_quality, score, old_score, min_score_diff)
                            .is_some()
                            && !context.forbid_upgrade)
                    {
                        let previous = tx.query_row("SELECT id,series_id,season,episode,title,quality_score,magnet_hash,magnet_link,downloaded_at,archive_path,size_bytes FROM episodes WHERE id=?1", [id], |row| Ok(UpgradeBackup { kind: "series".into(), row_id: row.get(0)?, series_id: Some(row.get(1)?), series_name: Some(series_name.to_owned()), season: Some(row.get(2)?), episode: Some(row.get(3)?), name: None, year: None, title: row.get::<_, Option<String>>(4)?.unwrap_or_default(), quality_score: row.get(5)?, magnet_hash: row.get(6)?, magnet_link: row.get(7)?, downloaded_at: row.get(8)?, archive_path: row.get(9)?, size_bytes: row.get(10)? }))?;
                        self.save_upgrade_backup(hash, &previous)?;
                        // Solo il primo episodio porta il magnet_hash (UNIQUE).
                        let episode_hash = if hash_available {
                            hash_available = false;
                            Some(hash)
                        } else {
                            None
                        };
                        // COALESCE: se non assegniamo l'hash, mantieni quello esistente.
                        tx.execute("UPDATE episodes SET title=?1,quality_score=?2,magnet_hash=COALESCE(?3,magnet_hash),magnet_link=?4,downloaded_at=NULL,archive_path=NULL WHERE id=?5", params![release.title, score, episode_hash, release.magnet, id])?;
                        upgraded += 1;
                    }
                }
            }
        }
        // Stagione sconosciuta (nessun episodio noto): mantieni il placeholder
        // stagionale, come faceva la versione precedente.
        if targets.is_empty() && complete {
            let existing: Option<(i64, i64, String)> = tx.query_row(
                "SELECT id,quality_score,COALESCE(title,'') FROM episodes WHERE series_id=?1 AND season=?2 AND episode=0",
                params![series_id, season],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?;
            match existing {
                None => {
                    let episode_hash = if hash_available {
                        Some(hash)
                    } else {
                        None
                    };
                    tx.execute("INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (?1,?2,0,?3,?4,?5,?6)", params![series_id, season, release.title, score, episode_hash, release.magnet])?;
                    inserted += 1;
                }
                Some((id, existing_score, existing_title)) => {
                    if release
                        .quality
                        .upgrade_reason(&parse_quality(&existing_title), score, existing_score, min_score_diff)
                        .is_some()
                    {
                        let episode_hash = if hash_available {
                            Some(hash)
                        } else {
                            None
                        };
                        tx.execute("UPDATE episodes SET title=?1,quality_score=?2,magnet_hash=COALESCE(?3,magnet_hash),magnet_link=?4,downloaded_at=NULL,archive_path=NULL WHERE id=?5", params![release.title, score, episode_hash, release.magnet, id])?;
                        upgraded += 1;
                    }
                }
            }
        }
        tx.commit()?;
        let approved = inserted + upgraded > 0;
        Ok((
            approved,
            if approved {
                if upgraded > 0 {
                    "upgrade"
                } else {
                    "approved"
                }
            } else {
                "duplicate"
            }
            .into(),
        ))
    }

    pub fn check_movie(&self, release: &Release) -> Result<(bool, String)> {
        self.check_movie_scored(release, release.quality.score(), Self::DEFAULT_UPGRADE_MIN_SCORE_DIFF)
    }

    pub fn check_movie_scored(
        &self,
        release: &Release,
        score: i64,
        min_score_diff: i64,
    ) -> Result<(bool, String)> {
        self.check_movie_scored_with(release, score, min_score_diff, false)
    }

    /// Like [`Self::check_movie_scored`] but, when `forbid_upgrade` is set and a
    /// real movie file is already imported, refuses to replace it (quality
    /// profile with upgrades disabled / cutoff reached).
    pub fn check_movie_scored_with(
        &self,
        release: &Release,
        score: i64,
        min_score_diff: i64,
        forbid_upgrade: bool,
    ) -> Result<(bool, String)> {
        let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        if self.is_blocklisted(&hash)? {
            return Ok((false, "blocklisted".into()));
        }
        if self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM movies WHERE magnet_hash=?1 AND removed_at IS NULL)",
            [&hash],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok((false, "duplicate".into()));
        }
        if let Some(id) = self
            .conn
            .query_row(
                "SELECT id FROM movies WHERE magnet_hash=?1 AND removed_at IS NOT NULL",
                [&hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            self.conn.execute("UPDATE movies SET name=?1,year=?2,title=?1,quality_score=?3,magnet_link=?4,downloaded_at=NULL,removed_at=NULL WHERE id=?5", params![release.title, release.year, score, release.magnet, id])?;
            return Ok((true, "restored".into()));
        }
        if let Some((id, existing_score, metadata_json, downloaded_at)) = self.conn.query_row("SELECT m.id,m.quality_score,COALESCE(t.metadata_json,''),m.downloaded_at FROM movies m LEFT JOIN torrent_meta t ON lower(t.hash)=lower(m.magnet_hash) WHERE m.removed_at IS NULL AND m.name=?1 AND m.year IS ?2", params![release.title, release.year], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?))).optional()? {
            // Quality profile cutoff reached on a real imported file: never
            // replace it.
            if forbid_upgrade && downloaded_at.is_some() {
                return Ok((false, "upgrades_disabled".into()));
            }
            // The movies row stores the configured clean name (not the original
            // release title). When the original torrent metadata is still
            // available, apply the same hard-upgrade rules as series (4K,
            // source, HDR, REPACK and REMUX); old databases without metadata
            // retain the historical score-delta fallback.
            let hard_upgrade = serde_json::from_str::<TorrentMeta>(&metadata_json)
                .ok()
                .map(|metadata| {
                    release.quality.upgrade_reason(
                        &metadata.release.quality,
                        score,
                        existing_score,
                        min_score_diff,
                    )
                    .is_some()
                });
            let upgrade = hard_upgrade.unwrap_or_else(|| {
                score > existing_score && score - existing_score >= min_score_diff
            });
            if !upgrade {
                return Ok((false, "duplicate".into()));
            }
            let previous = self.conn.query_row("SELECT id,name,year,title,quality_score,magnet_hash,magnet_link,downloaded_at,size_bytes FROM movies WHERE id=?1", [id], |row| Ok(UpgradeBackup { kind: "movie".into(), row_id: row.get(0)?, series_id: None, series_name: None, season: None, episode: None, name: Some(row.get::<_, Option<String>>(1)?.unwrap_or_default()), year: row.get(2)?, title: row.get::<_, Option<String>>(3)?.unwrap_or_default(), quality_score: row.get(4)?, magnet_hash: row.get(5)?, magnet_link: row.get(6)?, downloaded_at: row.get(7)?, archive_path: None, size_bytes: row.get(8)? }))?;
            self.save_upgrade_backup(&hash, &previous)?;
            self.conn.execute("UPDATE movies SET title=?1,quality_score=?2,magnet_hash=?3,magnet_link=?4,downloaded_at=NULL WHERE id=?5", params![release.title, score, hash, release.magnet, id])?;
            return Ok((true, "upgrade".into()));
        }
        self.conn.execute("INSERT INTO movies(name,year,title,quality_score,magnet_hash,magnet_link,downloaded_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![release.title, release.year, release.title, score, hash, release.magnet, Option::<String>::None])?;
        Ok((true, "approved".into()))
    }

    pub fn register_torrent(&self, release: &Release) -> Result<()> {
        self.register_torrent_scored(release, release.quality.score())
    }

    pub fn register_torrent_scored(&self, release: &Release, quality_score: i64) -> Result<()> {
        let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO torrent_meta(hash,kind,title,series_name,season,episode,year,quality_score,source,metadata_json,status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'queued',?11,?11) ON CONFLICT(hash) DO UPDATE SET kind=excluded.kind,title=excluded.title,series_name=excluded.series_name,season=excluded.season,episode=excluded.episode,year=excluded.year,quality_score=excluded.quality_score,source=excluded.source,metadata_json=excluded.metadata_json,status=CASE WHEN torrent_meta.status='completed' THEN torrent_meta.status ELSE 'queued' END,updated_at=excluded.updated_at",
            params![hash, release.kind, release.title, release.series, release.season, release.episode, release.year, quality_score, release.source, serde_json::to_string(&TorrentMeta { release: release.clone() })?, now],
        )?;
        Ok(())
    }

    /// Imposta il flag "non rinominare" per un torrent (crea la riga se assente).
    pub fn set_torrent_no_rename(&self, hash: &str, value: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO torrent_meta(hash,no_rename,status,created_at,updated_at) VALUES (?1,?2,'queued',datetime('now'),datetime('now')) ON CONFLICT(hash) DO UPDATE SET no_rename=excluded.no_rename, updated_at=excluded.updated_at",
            params![hash.to_ascii_lowercase(), value as i64],
        )?;
        Ok(())
    }

    pub fn torrent_no_rename(&self, hash: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(no_rename,0) FROM torrent_meta WHERE lower(hash)=lower(?1)",
                [hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            != 0)
    }

    /// Torrents explicitly excluded from renaming (hash, display name).
    pub fn no_rename_torrents(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT hash, COALESCE(NULLIF(name,''), NULLIF(title,''), '') FROM torrent_meta WHERE COALESCE(no_rename,0)!=0 ORDER BY updated_at DESC",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Records a failed source attempt and moves it down the escalation
    /// schedule. `kind` is `feed` or `indexer`; `provider` is the source name.
    pub fn provider_failure(&self, kind: &str, provider: &str, error: &str) -> Result<()> {
        let now = Utc::now();
        let previous: Option<(i64, Option<String>)> = self
            .conn
            .query_row(
                "SELECT level, most_recent_failure FROM provider_status WHERE provider=?1 AND kind=?2",
                params![provider, kind],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let elapsed = previous
            .as_ref()
            .and_then(|(_, last)| last.as_deref())
            .and_then(crate::utils::parse_timestamp)
            .map(|last| (now - last).num_seconds());
        let current = previous.map(|(level, _)| level).unwrap_or(0);
        let level = crate::backoff::next_level(current, elapsed);
        let disabled_till =
            now + chrono::Duration::seconds(crate::backoff::period_secs(level));
        self.conn.execute(
            "INSERT INTO provider_status(provider,kind,level,initial_failure,most_recent_failure,disabled_till,last_error) \
             VALUES (?1,?2,?3,?4,?4,?5,?6) \
             ON CONFLICT(provider,kind) DO UPDATE SET level=excluded.level, \
             most_recent_failure=excluded.most_recent_failure, disabled_till=excluded.disabled_till, \
             last_error=excluded.last_error",
            params![
                provider,
                kind,
                level,
                now.to_rfc3339(),
                disabled_till.to_rfc3339(),
                error
            ],
        )?;
        Ok(())
    }

    /// Records a source success: step the escalation level down and lift the
    /// disabled window. The row is removed entirely once fully recovered.
    pub fn provider_success(&self, kind: &str, provider: &str) -> Result<()> {
        let level: Option<i64> = self
            .conn
            .query_row(
                "SELECT level FROM provider_status WHERE provider=?1 AND kind=?2",
                params![provider, kind],
                |row| row.get(0),
            )
            .optional()?;
        let Some(level) = level else {
            return Ok(());
        };
        let next = crate::backoff::success_level(level);
        if next <= 0 {
            self.conn.execute(
                "DELETE FROM provider_status WHERE provider=?1 AND kind=?2",
                params![provider, kind],
            )?;
        } else {
            self.conn.execute(
                "UPDATE provider_status SET level=?3, disabled_till=NULL WHERE provider=?1 AND kind=?2",
                params![provider, kind, next],
            )?;
        }
        Ok(())
    }

    /// True while a source is inside its disabled window.
    pub fn provider_blocked(&self, kind: &str, provider: &str) -> Result<bool> {
        let disabled: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT disabled_till FROM provider_status WHERE provider=?1 AND kind=?2",
                params![provider, kind],
                |row| row.get(0),
            )
            .optional()?;
        let Some(disabled) = disabled else {
            return Ok(false);
        };
        let Some(until) = disabled.and_then(|value| crate::utils::parse_timestamp(&value)) else {
            return Ok(false);
        };
        Ok(until > Utc::now())
    }

    /// All currently disabled `(kind, provider)` pairs, for the search fan-out
    /// to skip in one query instead of one per source.
    pub fn blocked_providers(&self) -> Result<std::collections::HashSet<(String, String)>> {
        // All timestamps are written by `to_rfc3339()` (UTC, same shape), so a
        // lexicographic comparison is a valid instant comparison and avoids an
        // N+1 parse per row.
        let now = Utc::now().to_rfc3339();
        let mut statement = self.conn.prepare(
            "SELECT kind, provider FROM provider_status WHERE disabled_till IS NOT NULL AND disabled_till > ?1",
        )?;
        let rows = statement.query_map([&now], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?)
    }

    /// Full provider status list for the UI/API.
    pub fn provider_statuses(&self) -> Result<Vec<crate::models::ProviderStatus>> {
        let mut statement = self.conn.prepare(
            "SELECT provider, kind, level, COALESCE(disabled_till,''), \
             COALESCE(most_recent_failure,''), COALESCE(last_error,'') \
             FROM provider_status ORDER BY disabled_till DESC, provider ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(crate::models::ProviderStatus {
                provider: row.get(0)?,
                kind: row.get(1)?,
                level: row.get(2)?,
                disabled_till: row.get(3)?,
                most_recent_failure: row.get(4)?,
                last_error: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Clears one provider (or every provider when `provider` is `None`).
    pub fn clear_provider_status(&self, provider: Option<&str>) -> Result<()> {
        match provider {
            Some(provider) => {
                self.conn.execute(
                    "DELETE FROM provider_status WHERE provider=?1",
                    [provider],
                )?;
            }
            None => {
                self.conn.execute("DELETE FROM provider_status", [])?;
            }
        }
        Ok(())
    }

    /// Persists the `ffprobe` result for a release's archived file. Matches the
    /// row by series/season/episode or movie name/year.
    pub fn set_media_info(
        &self,
        release: &Release,
        info: &crate::mediainfo::MediaInfo,
    ) -> Result<()> {
        let json = serde_json::to_string(info)?;
        if release.kind == "movie" {
            self.conn.execute(
                "UPDATE movies SET media_info_json=?1 WHERE name=?2 AND year IS ?3",
                params![json, release.title, release.year],
            )?;
        } else if let (Some(series), Some(season), Some(episode)) =
            (release.series.as_deref(), release.season, release.episode)
        {
            self.conn.execute(
                "UPDATE episodes SET media_info_json=?1 WHERE series_id=(SELECT id FROM series WHERE name=?2) AND season=?3 AND episode=?4",
                params![json, series, season, episode],
            )?;
        }
        Ok(())
    }

    fn parse_media_info(raw: Option<Option<String>>) -> Option<serde_json::Value> {
        raw.flatten()
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| serde_json::from_str(&value).ok())
    }

    pub fn episode_media_info(
        &self,
        series: &str,
        season: i64,
        episode: i64,
    ) -> Result<Option<serde_json::Value>> {
        let raw: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT media_info_json FROM episodes WHERE series_id=(SELECT id FROM series WHERE name=?1) AND season=?2 AND episode=?3",
                params![series, season, episode],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Self::parse_media_info(raw))
    }

    pub fn movie_media_info(
        &self,
        name: &str,
        year: Option<i64>,
    ) -> Result<Option<serde_json::Value>> {
        let raw: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT media_info_json FROM movies WHERE name=?1 AND year IS ?2",
                params![name, year],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Self::parse_media_info(raw))
    }

    pub fn is_blocklisted(&self, hash: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM blocklist WHERE magnet_hash=?1)",
            [hash.to_ascii_lowercase()],
            |row| row.get(0),
        )?)
    }
    pub fn blocklist(&self, release: &Release, reason: &str) -> Result<()> {
        let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        // Per un film `series` è None: l'identità utile è nome/anno del film.
        let (movie_name, movie_year) = if release.kind == "movie" {
            (release.title.clone(), release.year)
        } else {
            (String::new(), None)
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO blocklist(magnet_hash,title,reason,created_at,kind,series_name,season,episode,movie_name,movie_year) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                hash,
                release.title,
                reason,
                Utc::now().to_rfc3339(),
                release.kind,
                release.series,
                release.season,
                release.episode,
                movie_name,
                movie_year
            ],
        )?;
        Ok(())
    }
    pub fn blocklist_entries(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        let mut statement = self.conn.prepare("SELECT magnet_hash,title,reason,created_at,COALESCE(kind,''),COALESCE(series_name,''),season,episode,COALESCE(movie_name,''),movie_year FROM blocklist ORDER BY created_at DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 1000)], |row| {
            Ok(serde_json::json!({
                "hash": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "reason": row.get::<_, String>(2)?,
                "created_at": row.get::<_, String>(3)?,
                "kind": row.get::<_, String>(4)?,
                "series_name": row.get::<_, String>(5)?,
                "season": row.get::<_, Option<i64>>(6)?,
                "episode": row.get::<_, Option<i64>>(7)?,
                "movie_name": row.get::<_, String>(8)?,
                "movie_year": row.get::<_, Option<i64>>(9)?,
            }))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn remove_blocklist(&self, hash: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM blocklist WHERE lower(magnet_hash)=lower(?1)",
            [hash],
        )? > 0)
    }
    pub fn rollback_release(&self, release: &Release) -> Result<()> {
        let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        self.conn.execute(
            "DELETE FROM episodes WHERE magnet_hash=?1 OR magnet_link=?2",
            params![hash, release.magnet],
        )?;
        self.conn
            .execute("DELETE FROM movies WHERE magnet_hash=?1", [&hash])?;
        Ok(())
    }

    /// Cleans up the database when a torrent is removed by the user (or is
    /// abandoned). An archived row keeps its file but drops the pointer to the
    /// removed release, so the same (or a better) release can be fetched again;
    /// a fresh placeholder is deleted so the retry is unblocked. This also
    /// covers season packs, whose upgrade backup only stores one episode.
    pub fn forget_removed_torrent(&self, hash: &str) -> Result<()> {
        let normalized = hash.to_ascii_lowercase();
        self.conn.execute(
            "UPDATE episodes SET magnet_hash=NULL WHERE lower(COALESCE(magnet_hash,''))=?1 AND (downloaded_at IS NOT NULL OR COALESCE(archive_path,'')<>'')",
            [&normalized],
        )?;
        self.conn.execute(
            "UPDATE movies SET magnet_hash=NULL WHERE lower(COALESCE(magnet_hash,''))=?1 AND (downloaded_at IS NOT NULL OR COALESCE(archive_path,'')<>'')",
            [&normalized],
        )?;
        self.conn.execute(
            "DELETE FROM episodes WHERE lower(COALESCE(magnet_hash,''))=?1 AND downloaded_at IS NULL AND COALESCE(archive_path,'')=''",
            [&normalized],
        )?;
        self.conn.execute(
            "DELETE FROM movies WHERE lower(COALESCE(magnet_hash,''))=?1 AND downloaded_at IS NULL",
            [&normalized],
        )?;
        self.conn.execute("DELETE FROM upgrade_backup WHERE new_hash=?1", [&normalized])?;
        Ok(())
    }

    pub fn queue_pending(&self, release: &Release, timeframe_hours: i64) -> Result<()> {
        self.queue_pending_scored(release, timeframe_hours.max(0) * 60, release.quality.score())
    }

    /// Holds a series release for `delay_minutes` (a delay profile). The best
    /// scoring candidate seen during the window is the one that will be grabbed.
    pub fn queue_pending_scored(
        &self,
        release: &Release,
        delay_minutes: i64,
        quality_score: i64,
    ) -> Result<()> {
        let series_name = release.series.as_deref().unwrap_or(&release.title);
        self.conn.execute(
            "INSERT OR IGNORE INTO series(name) VALUES (?1)",
            params![series_name],
        )?;
        let series_id: i64 = self.conn.query_row(
            "SELECT id FROM series WHERE name=?1",
            params![series_name],
            |row| row.get(0),
        )?;
        let now = Utc::now();
        let due_at = now + chrono::Duration::minutes(delay_minutes.max(0));
        let existing: Option<(i64, i64)> = self.conn.query_row("SELECT id,best_quality_score FROM pending_downloads WHERE series_id=?1 AND season=?2 AND episode=?3 AND status='pending'", params![series_id, release.season, release.episode], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        if let Some((id, score)) = existing {
            if quality_score > score {
                self.conn.execute("UPDATE pending_downloads SET best_title=?1,best_quality_score=?2,best_magnet=?3 WHERE id=?4", params![release.title, quality_score, release.magnet, id])?;
            }
        } else {
            self.conn.execute("INSERT INTO pending_downloads(series_id,season,episode,best_magnet,best_quality_score,ready_at,best_title,first_seen_at,timeframe_hours,due_at,status) VALUES (?1,?2,?3,?4,?5,?6,?7,?6,?8,?9,'pending')", params![series_id, release.season, release.episode, release.magnet, quality_score, now.to_rfc3339(), release.title, (delay_minutes.max(0) + 59) / 60, due_at.to_rfc3339()])?;
        }
        Ok(())
    }

    /// Series releases whose delay window has elapsed. `due_at` is authoritative;
    /// rows written before it existed fall back to `first_seen_at + timeframe_hours`.
    pub fn ready_pending(&self) -> Result<Vec<ReadyPending>> {
        let now = Utc::now();
        let mut statement = self.conn.prepare(
            "SELECT s.name,p.best_title,p.best_magnet,p.season,p.episode,p.due_at,p.first_seen_at,p.ready_at,p.timeframe_hours \
             FROM pending_downloads p JOIN series s ON s.id=p.series_id \
             WHERE p.status='pending' AND s.enabled=1",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?,
            ))
        })?;
        let mut ready = Vec::new();
        for row in rows {
            let (name, title, magnet, season, episode, due_at, first_seen, ready_at, hours) = row?;
            let due = due_at
                .as_deref()
                .and_then(crate::utils::parse_timestamp)
                .or_else(|| {
                    let base = first_seen.as_deref().or(ready_at.as_deref())?;
                    let base = crate::utils::parse_timestamp(base)?;
                    Some(base + chrono::Duration::hours(hours.unwrap_or(0).max(0)))
                });
            if due.is_some_and(|due| now >= due) {
                ready.push((name, title, magnet, season, episode));
            }
        }
        Ok(ready)
    }

    pub fn remove_pending(&self, series_name: &str, season: i64, episode: i64) -> Result<()> {
        self.conn.execute("UPDATE pending_downloads SET status='downloaded',downloaded_at=?1 WHERE series_id=(SELECT id FROM series WHERE name=?2) AND season=?3 AND episode=?4 AND status='pending'", params![Utc::now().to_rfc3339(), series_name, season, episode])?;
        Ok(())
    }

    /// Holds a movie release for `delay_minutes` (delay profile for movies).
    pub fn queue_pending_movie_scored(
        &self,
        release: &Release,
        delay_minutes: i64,
        quality_score: i64,
    ) -> Result<()> {
        let name = release.title.clone();
        let now = Utc::now();
        let due_at = now + chrono::Duration::minutes(delay_minutes.max(0));
        let existing: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id,best_quality_score FROM pending_movies WHERE name=?1 AND year IS ?2 AND status='pending'",
                params![name, release.year],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, score)) = existing {
            if quality_score > score {
                self.conn.execute(
                    "UPDATE pending_movies SET best_title=?1,best_quality_score=?2,best_magnet=?3 WHERE id=?4",
                    params![release.title, quality_score, release.magnet, id],
                )?;
            }
        } else {
            self.conn.execute(
                "INSERT OR REPLACE INTO pending_movies(name,year,best_magnet,best_quality_score,best_title,first_seen_at,delay_hours,due_at,status) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending')",
                params![
                    name,
                    release.year,
                    release.magnet,
                    quality_score,
                    release.title,
                    now.to_rfc3339(),
                    (delay_minutes.max(0) + 59) / 60,
                    due_at.to_rfc3339()
                ],
            )?;
        }
        Ok(())
    }

    /// Movies whose delay window has elapsed.
    pub fn ready_pending_movies(&self) -> Result<Vec<ReadyPendingMovie>> {
        let now = Utc::now();
        let mut statement = self.conn.prepare(
            "SELECT name,best_title,best_magnet,year,due_at,first_seen_at,delay_hours \
             FROM pending_movies WHERE status='pending'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?;
        let mut ready = Vec::new();
        for row in rows {
            let (name, title, magnet, year, due_at, first_seen, hours) = row?;
            let due = due_at
                .as_deref()
                .and_then(crate::utils::parse_timestamp)
                .or_else(|| {
                    let base = crate::utils::parse_timestamp(first_seen.as_deref()?)?;
                    Some(base + chrono::Duration::hours(hours.unwrap_or(0).max(0)))
                });
            if due.is_some_and(|due| now >= due) {
                ready.push((name, title, magnet, year));
            }
        }
        Ok(ready)
    }

    pub fn remove_pending_movie(&self, name: &str, year: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_movies SET status='downloaded',downloaded_at=?1 WHERE name=?2 AND year IS ?3 AND status='pending'",
            params![Utc::now().to_rfc3339(), name, year],
        )?;
        Ok(())
    }

    /// Highest resolution rank among archived episodes *after* the given one
    /// (same season or later). Used to let the opt-in smart-episode guard still
    /// accept a genuine quality upgrade below the profile cutoff.
    pub fn later_archived_max_resolution_rank(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
    ) -> Result<Option<i32>> {
        let mut statement = self.conn.prepare(
            "SELECT COALESCE(e.title,'') FROM episodes e JOIN series s ON s.id=e.series_id \
             WHERE s.name=?1 AND (e.season > ?2 OR (e.season = ?2 AND e.episode > ?3)) \
             AND (e.downloaded_at IS NOT NULL OR COALESCE(e.archive_path,'') <> '')",
        )?;
        let rows = statement.query_map(params![series_name, season, episode], |row| {
            row.get::<_, String>(0)
        })?;
        let mut best: Option<i32> = None;
        for row in rows {
            let rank = crate::parser::parse_quality(&row?).resolution_rank();
            if rank > 0 && best.is_none_or(|current| rank > current) {
                best = Some(rank);
            }
        }
        Ok(best)
    }

    pub fn archive_gaps(&self) -> Result<Vec<(String, i64, i64)>> {
        let mut statement = self.conn.prepare("SELECT s.name,e.season,MAX(e.episode) FROM episodes e JOIN series s ON s.id=e.series_id GROUP BY s.name,e.season")?;
        let seasons = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut metadata = self
            .conn
            .prepare("SELECT series_name,season,episode_count FROM series_metadata")?;
        let expected = metadata
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut season_targets = std::collections::BTreeMap::<(String, i64), i64>::new();
        for (name, season, count) in seasons.into_iter().chain(expected) {
            season_targets
                .entry((name, season))
                .and_modify(|value| *value = (*value).max(count))
                .or_insert(count);
        }
        let mut gaps = Vec::new();
        for ((name, season_number), max_episode) in season_targets {
            let complete_pack = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM episodes e JOIN series s ON s.id=e.series_id WHERE s.name=?1 AND e.season=?2 AND e.episode=0 AND (e.downloaded_at IS NOT NULL OR EXISTS(SELECT 1 FROM torrent_meta t WHERE t.hash=e.magnet_hash AND t.status NOT IN ('error','removed'))))", params![name, season_number], |row| row.get::<_, bool>(0))?;
            if complete_pack {
                continue;
            }
            let mut present = self.conn.prepare("SELECT episode FROM episodes e JOIN series s ON s.id=e.series_id WHERE s.name=?1 AND e.season=?2")?;
            let episodes = present
                .query_map(params![name, season_number], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
            for episode in 1..=max_episode {
                let ignored: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM ignored_episodes WHERE series_name=?1 AND season=?2 AND episode=?3)", params![name, season_number, episode], |row| row.get(0))?;
                if !ignored && !episodes.contains(&episode) {
                    gaps.push((name.clone(), season_number, episode));
                }
            }
        }
        Ok(gaps)
    }

    /// Date cache per la vista globale dei mancanti. La chiave include la
    /// serie perché gli episodi non sono univoci fra titoli diversi.
    pub fn episode_air_dates(&self) -> Result<std::collections::HashMap<(String, i64, i64), String>> {
        let mut statement = self.conn.prepare(
            "SELECT series_name,season,episode,air_date FROM episode_metadata",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ),
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
        Ok(rows)
    }

    /// Episodi attesi di una serie che non hanno ancora un percorso di archivio.
    /// È il criterio del pulsante manuale “Cerca mancanti”: una puntata ancora
    /// nel client può avere alternative nei feed, ma una puntata sul NAS no.
    pub fn unarchived_episodes_for_series(
        &self,
        series_name: &str,
        ignored_seasons: &[i64],
    ) -> Result<Vec<(i64, i64)>> {
        let mut targets = std::collections::BTreeMap::<i64, i64>::new();
        let mut metadata = self.conn.prepare(
            "SELECT season,episode_count FROM series_metadata WHERE series_name=?1",
        )?;
        for row in metadata.query_map([series_name], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (season, count) = row?;
            targets.entry(season).and_modify(|value| *value = (*value).max(count)).or_insert(count);
        }
        let mut known = self.conn.prepare(
            "SELECT e.season,MAX(e.episode) FROM episodes e
             JOIN series s ON s.id=e.series_id
             WHERE s.name=?1 AND e.episode>0 GROUP BY e.season",
        )?;
        for row in known.query_map([series_name], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (season, count) = row?;
            targets.entry(season).and_modify(|value| *value = (*value).max(count)).or_insert(count);
        }
        let mut archived = self.conn.prepare(
            "SELECT e.season,e.episode FROM episodes e
             JOIN series s ON s.id=e.series_id
             WHERE s.name=?1 AND e.episode>0 AND COALESCE(e.archive_path,'')<>''",
        )?;
        let archived = archived
            .query_map([series_name], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
        let mut missing = Vec::new();
        for (season, count) in targets {
            if ignored_seasons.contains(&season) {
                continue;
            }
            for episode in 1..=count {
                let ignored: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM ignored_episodes WHERE series_name=?1 AND season=?2 AND episode=?3)",
                    params![series_name, season, episode],
                    |row| row.get(0),
                )?;
                if !ignored && !archived.contains(&(season, episode)) {
                    missing.push((season, episode));
                }
            }
        }
        Ok(missing)
    }

    /// Episodi della serie; `extra_ignored` sono le stagioni ignorate dalla
    /// configurazione (fonte autorevole) e vengono unite a quelle del DB.
    /// Le stagioni ignorate non compaiono e non risultano "missing".
    pub fn episodes_for_series(
        &self,
        series_name: &str,
        extra_ignored: &[i64],
    ) -> Result<Vec<EpisodeView>> {
        let mut ignored_seasons: Vec<i64> = self
            .conn
            .query_row(
                "SELECT COALESCE(ignored_seasons,'[]') FROM series WHERE name=?1",
                [series_name],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|value| serde_json::from_str::<Vec<i64>>(&value).ok())
            .unwrap_or_default();
        for season in extra_ignored {
            if !ignored_seasons.contains(season) {
                ignored_seasons.push(*season);
            }
        }
        let air_dates = {
            let mut statement = self.conn.prepare(
                "SELECT season,episode,air_date FROM episode_metadata WHERE series_name=?1",
            )?;
            let rows = statement
                .query_map([series_name], |row| {
                    Ok(((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?), row.get::<_, String>(2)?))
                })?
                .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
            rows
        };
        let mut statement = self.conn.prepare("SELECT e.id,s.name,e.season,e.episode,COALESCE(e.title,''),e.quality_score,e.downloaded_at,e.archive_path,COALESCE(e.size_bytes,0),e.magnet_hash,e.magnet_link,CASE WHEN e.downloaded_at IS NOT NULL THEN 'downloaded' ELSE COALESCE(t.status,'missing') END,COALESCE(t.error,''),EXISTS(SELECT 1 FROM ignored_episodes i WHERE i.series_name=s.name AND i.season=e.season AND i.episode=e.episode) FROM episodes e JOIN series s ON s.id=e.series_id LEFT JOIN torrent_meta t ON lower(t.hash)=lower(e.magnet_hash) WHERE s.name=?1 AND e.episode > 0 ORDER BY e.season,e.episode")?;
        let rows = statement.query_map([series_name], |row| {
            let season = row.get::<_, i64>(2)?;
            let episode = row.get::<_, i64>(3)?;
            Ok(EpisodeView {
                id: row.get(0)?,
                series_name: row.get(1)?,
                season,
                episode,
                title: row.get(4)?,
                air_date: air_dates.get(&(season, episode)).cloned().unwrap_or_default(),
                renamed_title: String::new(),
                quality_score: row.get(5)?,
                downloaded_at: row.get(6)?,
                archive_path: row.get(7)?,
                size_bytes: row.get(8)?,
                magnet_hash: row.get(9)?,
                magnet_link: row.get(10)?,
                status: row.get(11)?,
                error: row.get(12)?,
                ignored: row.get(13)?,
            })
        })?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        items.retain(|item| !ignored_seasons.contains(&item.season));
        let expected = {
            let mut metadata = self.conn.prepare("SELECT season,episode_count FROM series_metadata WHERE series_name=?1 ORDER BY season")?;
            let rows = metadata.query_map([series_name], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (season, count) in expected {
            if ignored_seasons.contains(&season) {
                continue;
            }
            for episode in 1..=count {
                if items
                    .iter()
                    .any(|item| item.season == season && item.episode == episode)
                {
                    continue;
                }
                let ignored: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM ignored_episodes WHERE series_name=?1 AND season=?2 AND episode=?3)", params![series_name, season, episode], |value| value.get(0))?;
                items.push(EpisodeView {
                    id: 0,
                    series_name: series_name.to_string(),
                    season,
                    episode,
                    title: String::new(),
                    air_date: air_dates.get(&(season, episode)).cloned().unwrap_or_default(),
                    renamed_title: String::new(),
                    quality_score: 0,
                    downloaded_at: None,
                    archive_path: None,
                    size_bytes: 0,
                    magnet_hash: None,
                    magnet_link: None,
                    status: "missing".into(),
                    error: String::new(),
                    ignored,
                });
            }
        }
        // Titolo mostrato nel dettaglio serie: il nome del file rinominato in
        // libreria, non quello scaricato. La preview di rinomina continua a
        // usare `title` (release originale) e il percorso reale del file.
        for item in items.iter_mut() {
            item.renamed_title = item
                .archive_path
                .as_deref()
                .map(renamed_file_title)
                .unwrap_or_default();
        }
        items.sort_by_key(|item| (item.season, item.episode));
        Ok(items)
    }

    pub fn downloaded_movies(&self, limit: usize) -> Result<Vec<MovieHistory>> {
        let mut statement = self.conn.prepare("SELECT id,COALESCE(name,''),year,COALESCE(title,name,''),quality_score,downloaded_at,COALESCE(size_bytes,0),magnet_hash FROM movies WHERE downloaded_at IS NOT NULL AND removed_at IS NULL ORDER BY downloaded_at DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 2000) as i64], |row| {
            Ok(MovieHistory {
                id: row.get(0)?,
                name: row.get(1)?,
                year: row.get(2)?,
                title: row.get(3)?,
                quality_score: row.get(4)?,
                downloaded_at: row.get(5)?,
                size_bytes: row.get(6)?,
                magnet_hash: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn consumption_stats(&self) -> Result<ConsumptionStats> {
        let total = self.conn.query_row("SELECT COALESCE((SELECT SUM(size_bytes) FROM episodes WHERE downloaded_at IS NOT NULL),0) + COALESCE((SELECT SUM(size_bytes) FROM movies WHERE downloaded_at IS NOT NULL AND removed_at IS NULL),0)", [], |row| row.get::<_, i64>(0))?;
        let last_30 = self.conn.query_row("SELECT COALESCE((SELECT SUM(size_bytes) FROM episodes WHERE datetime(downloaded_at) >= datetime('now','-30 days')),0) + COALESCE((SELECT SUM(size_bytes) FROM movies WHERE datetime(downloaded_at) >= datetime('now','-30 days') AND removed_at IS NULL),0)", [], |row| row.get::<_, i64>(0))?;
        let last_7 = self.conn.query_row("SELECT COALESCE((SELECT SUM(size_bytes) FROM episodes WHERE datetime(downloaded_at) >= datetime('now','-7 days')),0) + COALESCE((SELECT SUM(size_bytes) FROM movies WHERE datetime(downloaded_at) >= datetime('now','-7 days') AND removed_at IS NULL),0)", [], |row| row.get::<_, i64>(0))?;
        let mut statement = self.conn.prepare("SELECT date, SUM(bytes) FROM (SELECT substr(downloaded_at,1,10) AS date, COALESCE(size_bytes,0) AS bytes FROM episodes WHERE datetime(downloaded_at) >= datetime('now','-7 days') UNION ALL SELECT substr(downloaded_at,1,10), COALESCE(size_bytes,0) FROM movies WHERE datetime(downloaded_at) >= datetime('now','-7 days') AND removed_at IS NULL) WHERE date IS NOT NULL GROUP BY date ORDER BY date")?;
        let daily = statement
            .query_map([], |row| {
                Ok(DailyConsumption {
                    date: row.get(0)?,
                    bytes: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ConsumptionStats {
            total_bytes: total,
            last_30_days_bytes: last_30,
            last_7_days_bytes: last_7,
            daily_7d: daily,
        })
    }

    pub fn recent_downloads(&self, limit: usize) -> Result<Vec<RecentDownload>> {
        let mut statement = self.conn.prepare("SELECT kind,name,season,episode,year,downloaded_at,size_bytes,archive_path,quality_score FROM (SELECT 'series' AS kind,s.name AS name,e.season AS season,e.episode AS episode,NULL AS year,e.downloaded_at AS downloaded_at,COALESCE(e.size_bytes,0) AS size_bytes,e.archive_path AS archive_path,e.quality_score AS quality_score FROM episodes e JOIN series s ON s.id=e.series_id WHERE e.downloaded_at IS NOT NULL UNION ALL SELECT 'movie' AS kind,COALESCE(m.name,m.title) AS name,NULL AS season,NULL AS episode,m.year AS year,m.downloaded_at AS downloaded_at,COALESCE(m.size_bytes,0) AS size_bytes,NULL AS archive_path,m.quality_score AS quality_score FROM movies m WHERE m.downloaded_at IS NOT NULL AND m.removed_at IS NULL) ORDER BY downloaded_at DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 2000) as i64], |row| {
            Ok(RecentDownload {
                kind: row.get(0)?,
                name: row.get(1)?,
                season: row.get(2)?,
                episode: row.get(3)?,
                year: row.get(4)?,
                downloaded_at: row.get(5)?,
                size_bytes: row.get(6)?,
                archive_path: row.get(7)?,
                quality_score: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_episode_ignored(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        ignored: bool,
        reason: &str,
    ) -> Result<()> {
        if ignored {
            self.conn.execute("INSERT INTO ignored_episodes(series_name,season,episode,reason) VALUES (?1,?2,?3,?4) ON CONFLICT(series_name,season,episode) DO UPDATE SET reason=excluded.reason", params![series_name, season, episode, reason])?;
        } else {
            self.conn.execute(
                "DELETE FROM ignored_episodes WHERE series_name=?1 AND season=?2 AND episode=?3",
                params![series_name, season, episode],
            )?;
        }
        Ok(())
    }

    pub fn reset_episode(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        remove_history: bool,
    ) -> Result<()> {
        self.conn.execute("DELETE FROM pending_downloads WHERE series_id=(SELECT id FROM series WHERE name=?1) AND season=?2 AND episode=?3", params![series_name, season, episode])?;
        if remove_history {
            self.conn.execute("DELETE FROM episodes WHERE series_id=(SELECT id FROM series WHERE name=?1) AND season=?2 AND episode=?3", params![series_name, season, episode])?;
        }
        self.conn.execute(
            "DELETE FROM ignored_episodes WHERE series_name=?1 AND season=?2 AND episode=?3",
            params![series_name, season, episode],
        )?;
        Ok(())
    }

    pub fn series_metadata_stale(&self, series_name: &str, max_age_hours: i64) -> Result<bool> {
        let updated: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(updated_at) FROM series_metadata WHERE series_name=?1",
                [series_name],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(updated
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
            .is_none_or(|value| {
                Utc::now()
                    .signed_duration_since(value.with_timezone(&Utc))
                    .num_hours()
                    >= max_age_hours
            }))
    }

    pub fn save_series_metadata(&self, series_name: &str, counts: &[(i64, i64)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (season, episode_count) in counts
            .iter()
            .copied()
            .filter(|(season, count)| *season > 0 && *count > 0)
        {
            tx.execute("INSERT INTO series_metadata(series_name,season,episode_count,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(series_name,season) DO UPDATE SET episode_count=excluded.episode_count,updated_at=excluded.updated_at", params![series_name, season, episode_count, Utc::now().to_rfc3339()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Stato TMDB della serie ("Ended", "Returning Series", ...), salvato dal
    /// refresh metadati così l'elenco serie può mostrare "terminata" senza una
    /// chiamata TMDB ad ogni apertura.
    pub fn save_series_status(
        &self,
        series_name: &str,
        status: &str,
        last_air_date: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO series_status(series_name,status,last_air_date,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(series_name) DO UPDATE SET status=excluded.status,last_air_date=excluded.last_air_date,updated_at=excluded.updated_at",
            params![series_name, status, last_air_date, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn series_statuses(&self) -> Result<std::collections::HashMap<String, String>> {
        let mut statement = self
            .conn
            .prepare("SELECT series_name,status FROM series_status")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?)
    }

    /// Cache persistente delle date TMDB. È distinta dalla tabella degli
    /// episodi scaricati, perché una puntata futura/mancante deve poter
    /// mostrare la data anche prima che esista una riga di download.
    pub fn save_episode_air_dates(
        &self,
        series_name: &str,
        episodes: &[(i64, i64, String)],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();
        for (season, episode, air_date) in episodes {
            if *season < 1 || *episode < 1 {
                continue;
            }
            tx.execute(
                "INSERT INTO episode_metadata(series_name,season,episode,air_date,updated_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(series_name,season,episode) DO UPDATE SET air_date=excluded.air_date,updated_at=excluded.updated_at",
                params![series_name, season, episode, air_date, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn sync_archive_file(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        title: &str,
        path: &str,
        size_bytes: i64,
    ) -> Result<()> {
        self.sync_archive_file_scored(
            series_name,
            season,
            episode,
            title,
            path,
            size_bytes,
            parse_quality(title).score(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync_archive_file_scored(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        title: &str,
        path: &str,
        size_bytes: i64,
        quality_score: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO series(name) VALUES (?1)",
            [series_name],
        )?;
        let series_id: i64 = self.conn.query_row(
            "SELECT id FROM series WHERE name=?1",
            [series_name],
            |row| row.get(0),
        )?;
        // Uno score reale dal nome file: prima era 0, quindi ogni file
        // scansionato risultava "inferiore" e veniva ri-scaricato. Il titolo
        // passato dal chiamante è il nome file completo, quindi `parse_quality`
        // vede risoluzione/sorgente/codec.
        // Never let a scan downgrade an episode that already holds a better
        // file: a stale 1080p file next to the kept 2160p one must not reset the
        // stored quality (and make the episode look inferior).
        self.conn.execute("INSERT INTO episodes(series_id,season,episode,title,quality_score,downloaded_at,archive_path,size_bytes) VALUES (?1,?2,?3,?4,?5,datetime('now'),?6,?7) ON CONFLICT(series_id,season,episode) DO UPDATE SET title=CASE WHEN excluded.quality_score>=episodes.quality_score THEN excluded.title ELSE episodes.title END,downloaded_at=excluded.downloaded_at,archive_path=excluded.archive_path,size_bytes=excluded.size_bytes,quality_score=MAX(excluded.quality_score, episodes.quality_score)", params![series_id, season, episode, title, quality_score, path, size_bytes])?;
        Ok(())
    }

    /// Updates the stored archive path of an episode (e.g. after a rename),
    /// together with the real size and (monotonic) quality score of the file.
    pub fn set_episode_archive_path(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        path: &str,
    ) -> Result<()> {
        let (size_bytes, quality_score) = file_stats(path);
        self.conn.execute(
            "UPDATE episodes SET archive_path=?1, downloaded_at=COALESCE(downloaded_at, datetime('now')), size_bytes=CASE WHEN ?2>0 THEN ?2 ELSE size_bytes END, quality_score=CASE WHEN ?3>quality_score THEN ?3 ELSE quality_score END WHERE series_id=(SELECT id FROM series WHERE name=?4) AND season=?5 AND episode=?6",
            params![path, size_bytes, quality_score, series_name, season, episode],
        )?;
        Ok(())
    }

    /// Re-aligne size and quality score of an episode to the file it currently
    /// points to, without touching its title (which keeps the original release
    /// title needed by the "restore source" pass).
    pub fn refresh_episode_file_stats(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        path: &str,
    ) -> Result<()> {
        let (size_bytes, quality_score) = file_stats(path);
        self.conn.execute(
            "UPDATE episodes SET size_bytes=CASE WHEN ?1>0 THEN ?1 ELSE size_bytes END, quality_score=CASE WHEN ?2>quality_score THEN ?2 ELSE quality_score END, downloaded_at=COALESCE(downloaded_at, datetime('now')) WHERE series_id=(SELECT id FROM series WHERE name=?3) AND season=?4 AND episode=?5",
            params![size_bytes, quality_score, series_name, season, episode],
        )?;
        Ok(())
    }

    /// TMDB season episode counts recorded for a series: `(season, count)`.
    pub fn series_season_counts(&self, series_name: &str) -> Result<Vec<(i64, i64)>> {
        let mut statement = self.conn.prepare(
            "SELECT season, episode_count FROM series_metadata WHERE series_name=?1 ORDER BY season",
        )?;
        let rows = statement.query_map([series_name], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn gap_recently_searched(
        &self,
        series_name: &str,
        season: i64,
        episode: i64,
        hours: i64,
    ) -> Result<bool> {
        let found: Option<String> = self.conn.query_row("SELECT last_searched_at FROM gap_search_log WHERE series_name=?1 AND season=?2 AND episode=?3", params![series_name, season, episode], |row| row.get(0)).optional()?;
        Ok(found
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
            .is_some_and(|value| {
                Utc::now()
                    .signed_duration_since(value.with_timezone(&Utc))
                    .num_hours()
                    < hours
            }))
    }

    pub fn mark_gap_searched(&self, series_name: &str, season: i64, episode: i64) -> Result<()> {
        self.conn.execute("INSERT INTO gap_search_log(series_name,season,episode,last_searched_at) VALUES (?1,?2,?3,?4) ON CONFLICT(series_name,season,episode) DO UPDATE SET last_searched_at=excluded.last_searched_at", params![series_name, season, episode, Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn set_torrent_tag(&self, hash: &str, tag: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE torrent_meta SET tag=?1, updated_at=datetime('now') WHERE lower(hash)=lower(?2)",
            rusqlite::params![tag, hash],
        )?;
        Ok(())
    }

    pub fn torrent_tags(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT lower(hash), COALESCE(tag,'') FROM torrent_meta WHERE TRIM(COALESCE(tag,'')) != ''",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Path where a completed torrent was archived (NAS), if any.
    pub fn torrent_processed(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT processed_path FROM torrent_meta WHERE hash=?1",
                [hash.to_ascii_lowercase()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Motivo per cui la release è stata accettata e messa in download
    /// (`approved`, `upgrade`, `gap_fill`, `gap_filled`, `manual`, ...). Serve a
    /// spiegare nei controlli perché un torrent è in sessione.
    pub fn set_torrent_reason(&self, hash: &str, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE torrent_meta SET reason=?2 WHERE hash=?1",
            params![hash.to_ascii_lowercase(), reason],
        )?;
        Ok(())
    }

    /// Dati di contorno mostrati sotto il nome del torrent nella sessione:
    /// `(percorso archiviato, sorgente, motivo del download)`. Una sola query
    /// per torrent, dato che l'endpoint `/api/torrents` è interrogato di continuo.
    pub fn torrent_aux(&self, hash: &str) -> Result<(String, String, String)> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(processed_path,''), COALESCE(source,''), COALESCE(reason,'') FROM torrent_meta WHERE hash=?1",
                [hash.to_ascii_lowercase()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Motivo registrato per un torrent, se presente e non vuoto.
    pub fn torrent_reason(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(reason,'') FROM torrent_meta WHERE hash=?1",
                [hash.to_ascii_lowercase()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .filter(|value| !value.trim().is_empty()))
    }

    pub fn torrent_status(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT COALESCE(status,'queued') FROM torrent_meta WHERE hash=?1",
                [hash.to_ascii_lowercase()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn torrent_meta(&self, hash: &str) -> Result<Option<TorrentMeta>> {
        let normalized = hash.to_ascii_lowercase();
        let metadata: Option<String> = self
            .conn
            .query_row(
                "SELECT metadata_json FROM torrent_meta WHERE hash=?1",
                [&normalized],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(value) = metadata.filter(|value| !value.is_empty()) {
            return Ok(Some(serde_json::from_str::<TorrentMeta>(&value)?));
        }
        if let Some((title, series, season, episode, magnet)) = self.conn.query_row(
            "SELECT e.title,s.name,e.season,e.episode,e.magnet_link FROM episodes e JOIN series s ON s.id=e.series_id WHERE e.magnet_hash=?1",
            [&normalized],
            |row| Ok((row.get::<_, Option<String>>(0)?.unwrap_or_default(), row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<String>>(4)?.unwrap_or_default())),
        ).optional()? {
            return Ok(Some(TorrentMeta { release: Release { torrent_url: None, title, magnet, source: "restored-db".into(), quality: Default::default(), kind: "series".into(), series: Some(series), season: Some(season), episode: Some(episode), is_pack: false, episode_range: vec![episode], year: None, discovered_at: Utc::now(), size_bytes: 0, seeders: -1, peers: -1 } }));
        }
        if let Some((title, year, magnet)) = self
            .conn
            .query_row(
                "SELECT COALESCE(title,name),year,magnet_link FROM movies WHERE magnet_hash=?1",
                [&normalized],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ))
                },
            )
            .optional()?
        {
            return Ok(Some(TorrentMeta {
                release: Release { torrent_url: None,
                    title,
                    magnet,
                    source: "restored-db".into(),
                    quality: Default::default(),
                    kind: "movie".into(),
                    series: None,
                    season: None,
                    episode: None,
                    is_pack: false,
                    episode_range: Vec::new(),
                    year: Some(year),
                    size_bytes: 0,
                    seeders: -1,
                    peers: -1,
                    discovered_at: Utc::now(),
                },
            }));
        }
        Ok(None)
    }

    /// Persists the identity found in the actual torrent name when an indexer
    /// advertised a different season. Move the not-yet-downloaded placeholders
    /// with it, otherwise the old RSS season remains eligible for later cycles.
    pub fn reconcile_pack_release(&self, hash: &str, release: &Release) -> Result<()> {
        let normalized = hash.to_ascii_lowercase();
        let Some(old) = self.torrent_meta(&normalized)? else {
            return Ok(());
        };
        if old.release.kind != "series" || !old.release.is_pack {
            return Ok(());
        }
        let old_magnet = old.release.magnet;
        let metadata_json = serde_json::to_string(&TorrentMeta {
            release: release.clone(),
        })?;
        let now = Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction()?;

        let new_series_id: Option<i64> = if let Some(series) = release.series.as_deref() {
            tx.query_row("SELECT id FROM series WHERE name=?1", [series], |row| row.get(0))
                .optional()?
        } else {
            None
        };
        let Some(new_series_id) = new_series_id else {
            tx.execute(
                "UPDATE torrent_meta SET kind=?1,title=?2,series_name=?3,season=?4,episode=?5,year=?6,quality_score=?7,source=?8,metadata_json=?9,updated_at=?10 WHERE hash=?11",
                params![release.kind, release.title, release.series, release.season, release.episode, release.year, release.quality.score(), release.source, metadata_json, now, normalized],
            )?;
            tx.commit()?;
            return Ok(());
        };
        let targets = release
            .episode_range
            .iter()
            .copied()
            .filter(|episode| *episode >= 0)
            .collect::<std::collections::HashSet<_>>();
        let mut statement = tx.prepare(
            "SELECT id,episode FROM episodes WHERE (lower(COALESCE(magnet_hash,''))=?1 OR magnet_link=?2) AND downloaded_at IS NULL AND COALESCE(archive_path,'')=''",
        )?;
        let rows = statement
            .query_map(params![normalized, old_magnet], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (id, episode) in rows {
            if !targets.contains(&episode) {
                tx.execute("DELETE FROM episodes WHERE id=?1", [id])?;
                continue;
            }
            let conflict: Option<i64> = tx
                .query_row(
                    "SELECT id FROM episodes WHERE series_id=?1 AND season=?2 AND episode=?3 AND id<>?4",
                    params![new_series_id, release.season, episode, id],
                    |row| row.get(0),
                )
                .optional()?;
            if conflict.is_some() {
                tx.execute("DELETE FROM episodes WHERE id=?1", [id])?;
            } else {
                tx.execute(
                    "UPDATE episodes SET series_id=?1,season=?2,title=?3,magnet_link=?4 WHERE id=?5",
                    params![new_series_id, release.season, release.title, release.magnet, id],
                )?;
            }
        }
        tx.execute(
            "UPDATE torrent_meta SET kind=?1,title=?2,series_name=?3,season=?4,episode=?5,year=?6,quality_score=?7,source=?8,metadata_json=?9,updated_at=?10 WHERE hash=?11",
            params![release.kind, release.title, release.series, release.season, release.episode, release.year, release.quality.score(), release.source, metadata_json, now, normalized],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Riconcilia i torrent tracciati ma assenti dalla sessione libtorrent.
    ///
    /// Una riga `torrent_meta` in stato attivo (`queued`/`downloading`/…)
    /// rimasta orfana dopo una rimozione manuale o un riavvio blocca per sempre
    /// il ri-scaricamento (check `active_episode`), e i suoi episodi placeholder
    /// fanno risultare il candidato `duplicate`. Qui la riga viene marcata
    /// `error` e i placeholder non scaricati della stessa release rimossi, così
    /// il ciclo successivo può riprovare.
    pub fn reconcile_missing_torrents(
        &self,
        live_hashes: &std::collections::HashSet<String>,
    ) -> Result<usize> {
        let mut statement = self.conn.prepare(
            "SELECT hash FROM torrent_meta WHERE status NOT IN ('completed','error','removed')",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut missing = Vec::new();
        for row in rows {
            let hash = row?;
            if !live_hashes.contains(&hash.to_ascii_lowercase()) {
                missing.push(hash);
            }
        }
        drop(statement);
        let mut reconciled = 0;
        for hash in missing {
            let release = self.torrent_meta(&hash)?.map(|meta| meta.release);
            self.conn.execute(
                "UPDATE torrent_meta SET status='error', error=CASE WHEN COALESCE(error,'')='' THEN 'missing from session' ELSE error END, updated_at=?2 WHERE hash=?1",
                params![hash.to_ascii_lowercase(), Utc::now().to_rfc3339()],
            )?;
            if let Some(release) = release {
                self.clear_placeholders(&release.magnet)?;
            }
            reconciled += 1;
        }
        // Download **conclusi** non più nella sessione: sono stati "puliti" e
        // devono comparire nello Storico. Gli incompleti spariti (sopra) restano
        // invece esclusi, come richiesto.
        let mut statement = self.conn.prepare(
            "SELECT hash FROM torrent_meta WHERE status='completed' AND removed_at IS NULL",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut cleared = Vec::new();
        for row in rows {
            let hash = row?;
            if !live_hashes.contains(&hash.to_ascii_lowercase()) {
                cleared.push(hash);
            }
        }
        drop(statement);
        for hash in cleared {
            let _ = self.mark_torrent_removed_at(&hash);
        }
        Ok(reconciled)
    }

    /// Segna come `removed` un torrent non concluso tolto dalla sessione e
    /// cancella i suoi placeholder non scaricati: evita che resti "in corso"
    /// nel DB e che il candidato venga visto come `active_episode`/`duplicate`.
    /// Registra anche l'uscita dalla sessione (`removed_at`) per lo Storico.
    pub fn mark_torrent_removed(&self, hash: &str) -> Result<()> {
        let release = self.torrent_meta(hash)?.map(|meta| meta.release);
        self.conn.execute(
            "UPDATE torrent_meta SET status='removed', updated_at=?2 WHERE hash=?1 AND status NOT IN ('completed','error','removed')",
            params![hash.to_ascii_lowercase(), Utc::now().to_rfc3339()],
        )?;
        self.mark_torrent_removed_at(hash)?;
        if let Some(release) = release {
            self.clear_placeholders(&release.magnet)?;
        }
        Ok(())
    }

    /// Registra che un torrent ha lasciato la sessione: è il momento in cui entra
    /// nello "Storico download". Non altera stato/esito già scritti
    /// (`completed`/`error`, `processed_path`), quindi serve ai percorsi di
    /// completamento dove l'esito è già stato deciso.
    pub fn mark_torrent_removed_at(&self, hash: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE torrent_meta SET removed_at=?2, updated_at=?2 WHERE hash=?1 AND removed_at IS NULL",
            params![hash.to_ascii_lowercase(), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// `created_at` / `completed_at` for a torrent (used for download duration).
    pub fn torrent_times(&self, hash: &str) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT created_at, completed_at FROM torrent_meta WHERE lower(hash)=lower(?1)",
                [hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn stored_torrents(&self, limit: usize) -> Result<Vec<StoredTorrent>> {
        self.stored_torrents_query(limit, true)
    }

    /// Voci della tabella "Storico download": torrent **usciti dalla sessione**
    /// (puliti con "Pulisci completati", rimossi a mano o rimossi dall'app dopo
    /// il postprocess). La Sessione mostra invece solo i torrent ancora nel
    /// client, quindi non c'è più sovrapposizione fra le due viste.
    ///
    /// Include anche gli esiti negativi (`status='error'`, es. pack scartato
    /// come inferiore) così l'utente vede *cosa è stato fatto* e il motivo.
    /// Sono esclusi i download annullati prima di concludersi (`status='removed'`
    /// con progresso incompleto) e le righe senza nome.
    /// Il nome usa, nell'ordine, `name`, `title` o `series_name`.
    /// Mantiene nello storico solo i download conclusi negli ultimi 30 giorni.
    /// Ritorna `(items_della_pagina, totale)`. `offset` è l'indice di partenza.
    pub fn completed_torrents(
        &self,
        offset: usize,
        limit: usize,
        query: &str,
    ) -> Result<(Vec<StoredTorrent>, i64)> {
        let mut where_clause = format!("FROM torrent_meta
             WHERE removed_at IS NOT NULL
               AND (status IN ('completed','error') OR COALESCE(progress,0) >= 1)
               AND datetime(COALESCE(NULLIF(completed_at,''), NULLIF(removed_at,''), updated_at)) >= datetime('now', '-{} days')
               AND COALESCE(NULLIF(name,''),NULLIF(title,''),NULLIF(series_name,'')) <> ''", DOWNLOAD_HISTORY_RETENTION_DAYS);
        // Ricerca intelligente: ogni parola deve comparire in almeno uno dei
        // campi utili dello storico, così `silo nas` trova anche titoli con
        // parole separate fra nome e cartella, senza scaricare tutte le pagine.
        let terms = query
            .split_whitespace()
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .take(8)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for _ in &terms {
            where_clause.push_str(" AND lower(COALESCE(name,'') || ' ' || COALESCE(title,'') || ' ' || COALESCE(series_name,'') || ' ' || COALESCE(tag,'') || ' ' || COALESCE(kind,'') || ' ' || COALESCE(status,'') || ' ' || COALESCE(processed_path,'')) LIKE '%' || lower(?) || '%'");
        }
        let total: i64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) {where_clause}"),
                params_from_iter(terms.iter()),
                |row| row.get(0),
            )?;
        let mut statement = self.conn.prepare(&format!(
            "SELECT hash,
                    COALESCE(NULLIF(name,''),NULLIF(title,''),NULLIF(series_name,''),hash),
                    COALESCE(tag,''),COALESCE(source,''),COALESCE(progress,0),COALESCE(paused,0),
                    COALESCE(total_size,0),COALESCE(downloaded,0),COALESCE(status,'queued'),COALESCE(updated_at,''),
                    COALESCE(kind,''),COALESCE(series_name,''),COALESCE(season,0),COALESCE(episode,0),
                    COALESCE(year,0),COALESCE(quality_score,0),COALESCE(completed_at,''),COALESCE(processed_path,''),COALESCE(error,''),COALESCE(reason,'')
              {where_clause}
              ORDER BY COALESCE(NULLIF(removed_at,''), NULLIF(completed_at,''), updated_at) DESC LIMIT ? OFFSET ?"
        ))?;
        let mut values = terms;
        values.push(limit.clamp(1, 2000).to_string());
        values.push(offset.to_string());
        let rows = statement.query_map(params_from_iter(values.iter()), stored_torrent_from_row)?;
        Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
    }

    pub fn stored_torrents_all(&self, limit: usize) -> Result<Vec<StoredTorrent>> {
        self.stored_torrents_query(limit, false)
    }

    fn stored_torrents_query(&self, limit: usize, active_only: bool) -> Result<Vec<StoredTorrent>> {
        let filter = if active_only {
            "AND progress > 0 AND progress < 1"
        } else {
            ""
        };
        let mut statement = self.conn.prepare(&format!(
            "SELECT hash,COALESCE(name,''),COALESCE(tag,''),COALESCE(source,''),COALESCE(progress,0),COALESCE(paused,0),
                    COALESCE(total_size,0),COALESCE(downloaded,0),COALESCE(status,'queued'),COALESCE(updated_at,''),
                    COALESCE(kind,''),COALESCE(series_name,''),COALESCE(season,0),COALESCE(episode,0),
                    COALESCE(year,0),COALESCE(quality_score,0),COALESCE(completed_at,''),COALESCE(processed_path,''),COALESCE(error,''),COALESCE(reason,'')
             FROM torrent_meta WHERE TRIM(COALESCE(name,'')) != '' {filter} ORDER BY updated_at DESC LIMIT ?1"
        ))?;
        let rows = statement.query_map(
            [limit.clamp(1, 2000) as i64],
            stored_torrent_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_torrent_completed(&self, hash: &str, path: &str, size_bytes: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute("UPDATE episodes SET downloaded_at=?1,archive_path=?2,size_bytes=?3 WHERE magnet_hash=?4", params![now, path, size_bytes, hash])?;
        self.conn.execute(
            "UPDATE movies SET downloaded_at=?1,size_bytes=?2 WHERE magnet_hash=?3",
            params![now, size_bytes, hash],
        )?;
        self.conn.execute("UPDATE torrent_meta SET status='completed',completed_at=?1,processed_path=?2,error='',updated_at=?1 WHERE hash=?3", params![now, path, hash.to_ascii_lowercase()])?;
        Ok(())
    }

    pub fn mark_release_completed(
        &self,
        release: &Release,
        path: &str,
        size_bytes: i64,
    ) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        let now = Utc::now().to_rfc3339();
        if release.kind == "series" {
            if let (Some(series), Some(season)) = (release.series.as_deref(), release.season) {
                let series_id: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM series WHERE name=?1", [series], |row| {
                        row.get(0)
                    })
                    .optional()?;
                if let Some(series_id) = series_id {
                    if release.is_pack && release.episode_range.contains(&0)
                    {
                        self.conn.execute("UPDATE episodes SET downloaded_at=?1,archive_path=?2,size_bytes=?3 WHERE series_id=?4 AND season=?5 AND episode=0", params![now, path, size_bytes, series_id, season])?;
                    }
                    let episodes = if release.is_pack
                        && (release.episode_range.is_empty()
                            || release.episode_range.contains(&0))
                    {
                        let count: Option<i64> = self.conn.query_row("SELECT MAX(episode_count) FROM series_metadata WHERE series_name=?1 AND season=?2", params![series, season], |row| row.get(0)).optional()?.flatten();
                        count
                            .filter(|value| *value > 0)
                            .map(|value| (1..=value).collect())
                            .unwrap_or_default()
                    } else if release.episode_range.is_empty() {
                        vec![release.episode.unwrap_or_default()]
                    } else {
                        release
                            .episode_range
                            .iter()
                            .copied()
                            .filter(|episode| *episode > 0)
                            .collect()
                    };
                    for episode in episodes {
                        self.conn.execute("UPDATE episodes SET downloaded_at=?1,archive_path=?2,size_bytes=?3 WHERE series_id=?4 AND season=?5 AND episode=?6", params![now, path, size_bytes, series_id, season, episode])?;
                    }
                }
            }
        } else {
            self.conn.execute(
                "UPDATE movies SET downloaded_at=?1,size_bytes=?2 WHERE magnet_hash=?3",
                params![now, size_bytes, hash],
            )?;
        }
        self.conn.execute("UPDATE torrent_meta SET status='completed',completed_at=?1,processed_path=?2,error='',updated_at=?1 WHERE hash=?3", params![now, path, hash])?;
            Ok(())
        })();
        match result {
            Ok(()) => match self.conn.execute_batch("COMMIT") {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(error.into())
                }
            },
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn mark_pack_completed(
        &self,
        release: &Release,
        episodes: &[(i64, String, i64, i64)],
        path: &str,
        size_bytes: i64,
    ) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let hash = magnet_hash(&release.magnet).context("invalid magnet hash")?;
        let now = Utc::now().to_rfc3339();
        if let (Some(series), Some(season)) = (release.series.as_deref(), release.season) {
            if let Some(series_id) = self
                .conn
                .query_row("SELECT id FROM series WHERE name=?1", [series], |row| {
                    row.get::<_, i64>(0)
                })
                .optional()?
            {
                if release.episode_range.contains(&0) {
                    self.conn.execute("UPDATE episodes SET downloaded_at=?1,archive_path=?2,size_bytes=?3 WHERE series_id=?4 AND season=?5 AND episode=0", params![now, path, size_bytes, series_id, season])?;
                }
                for (episode, episode_path, episode_size, episode_score) in episodes {
                    // `quality_score` follows the file that was actually kept:
                    // after an upgrade it must not stay at the old (lower) value
                    // or the episode looks inferior and gets downloaded again.
                    self.conn.execute("UPDATE episodes SET downloaded_at=?1,archive_path=?2,size_bytes=?3,quality_score=CASE WHEN ?4>quality_score THEN ?4 ELSE quality_score END WHERE series_id=?5 AND season=?6 AND episode=?7", params![now, episode_path, episode_size, episode_score, series_id, season, episode])?;
                }
            }
        }
        self.conn.execute("UPDATE torrent_meta SET status='completed',completed_at=?1,processed_path=?2,error='',updated_at=?1 WHERE hash=?3", params![now, path, hash])?;
            Ok(())
        })();
        match result {
            Ok(()) => match self.conn.execute_batch("COMMIT") {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(error.into())
                }
            },
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// Elimina i placeholder (non scaricati) di una release. Usato quando un
    /// download fallisce/scade: senza questa pulizia i suoi placeholder di
    /// qualità superiore bloccherebbero per sempre il ripiego su una release
    /// inferiore ma disponibile.
    fn clear_placeholders(&self, magnet: &str) -> Result<()> {
        let digest = magnet_hash(magnet).unwrap_or_default();
        self.conn.execute(
            "DELETE FROM episodes WHERE (lower(magnet_hash)=lower(?1) OR magnet_link=?2) AND downloaded_at IS NULL AND COALESCE(archive_path,'')=''",
            params![digest, magnet],
        )?;
        Ok(())
    }

    pub fn mark_torrent_error(&self, hash: &str, error: &str) -> Result<()> {
        let release = self.torrent_meta(hash)?.map(|meta| meta.release);
        self.conn.execute(
            "UPDATE torrent_meta SET status='error',error=?,updated_at=? WHERE hash=?",
            params![error, Utc::now().to_rfc3339(), hash.to_ascii_lowercase()],
        )?;
        // Fallito: i suoi placeholder non scaricati non devono più bloccare un
        // ripiego su una release inferiore (es. il 1080p dopo un 4K fermo).
        if let Some(release) = release {
            self.clear_placeholders(&release.magnet)?;
        }
        Ok(())
    }

    fn save_upgrade_backup(&self, new_hash: &str, backup: &UpgradeBackup) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO upgrade_backup(new_hash,payload_json,created_at) VALUES (?1,?2,?3)", params![new_hash.to_ascii_lowercase(), serde_json::to_string(backup)?, Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn restore_upgrade(&self, hash: &str) -> Result<bool> {
        let normalized = hash.to_ascii_lowercase();
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload_json FROM upgrade_backup WHERE new_hash=?1",
                [&normalized],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(false);
        };
        let backup: UpgradeBackup = serde_json::from_str(&payload)?;
        if backup.kind == "series" {
            self.conn.execute("UPDATE episodes SET quality_score=?1,magnet_hash=?2,magnet_link=?3,downloaded_at=?4,archive_path=?5,size_bytes=?6,title=?7 WHERE id=?8", params![backup.quality_score, backup.magnet_hash, backup.magnet_link, backup.downloaded_at, backup.archive_path, backup.size_bytes, backup.title, backup.row_id])?;
        } else {
            self.conn.execute("UPDATE movies SET name=?1,year=?2,title=?3,quality_score=?4,magnet_hash=?5,magnet_link=?6,downloaded_at=?7,size_bytes=?8,removed_at=NULL WHERE id=?9", params![backup.name, backup.year, backup.title, backup.quality_score, backup.magnet_hash, backup.magnet_link, backup.downloaded_at, backup.size_bytes, backup.row_id])?;
        }
        self.conn.execute(
            "DELETE FROM upgrade_backup WHERE new_hash=?1",
            [&normalized],
        )?;
        Ok(true)
    }

    pub fn save_cycle(&self, value: &crate::models::CycleStats) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cycle_history(at,payload_json) VALUES (?1,?2)",
            params![Utc::now().to_rfc3339(), serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn recent_cycles(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        let mut statement = self
            .conn
            .prepare("SELECT at,payload_json FROM cycle_history ORDER BY id DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 1000)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows
            .filter_map(Result::ok)
            .map(|(at, payload)| {
                let parsed = match serde_json::from_str::<serde_json::Value>(&payload) {
                    Ok(value) => value,
                    Err(_) => serde_json::json!({"raw":payload.clone()}),
                };
                if let Some(object) = parsed.as_object() {
                    let mut value = object.clone();
                    value.insert("at".into(), serde_json::Value::String(at));
                    serde_json::Value::Object(value)
                } else {
                    serde_json::json!({"at":at,"payload":payload})
                }
            })
            .collect())
    }
    /// Ricalcola gli score con i pesi configurati e li normalizza.
    ///
    /// Oltre alle release ancora tracciate in `torrent_meta`, aggiorna anche
    /// episodi/film importati o scansionati (senza `torrent_meta`): il loro
    /// `quality_score` era lo score *base* mentre le approvazioni usano i pesi
    /// personalizzati, e questo li faceva apparire inferiori → riscaricamenti.
    /// La qualità si ricava dal titolo o, in mancanza, dal nome del file
    /// archiviato; se non è riconoscibile il record resta invariato.
    pub fn rescore(&self, settings: &std::collections::BTreeMap<String, String>) -> Result<usize> {
        let mut statement = self
            .conn
            .prepare("SELECT hash,metadata_json FROM torrent_meta WHERE metadata_json!=''")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed = 0;
        for (hash, json) in rows {
            let Ok(meta) = serde_json::from_str::<TorrentMeta>(&json) else {
                continue;
            };
            let score = meta.release.quality.score_with_settings(settings);
            changed += self.conn.execute(
                "UPDATE episodes SET quality_score=?1 WHERE lower(magnet_hash)=lower(?2)",
                params![score, hash],
            )?;
            changed += self.conn.execute(
                "UPDATE movies SET quality_score=?1 WHERE lower(magnet_hash)=lower(?2)",
                params![score, hash],
            )?;
            changed += self.conn.execute(
                "UPDATE torrent_meta SET quality_score=?1,updated_at=?2 WHERE lower(hash)=lower(?3)",
                params![score, Utc::now().to_rfc3339(), hash],
            )?;
        }
        // Episodi senza una release tracciata: normalizza dal titolo o dal file.
        let episodes = self
            .conn
            .prepare(
                "SELECT e.id, COALESCE(e.title,''), COALESCE(e.archive_path,'') FROM episodes e
                 WHERE e.magnet_hash IS NULL
                    OR NOT EXISTS(SELECT 1 FROM torrent_meta t WHERE lower(t.hash)=lower(e.magnet_hash) AND COALESCE(t.metadata_json,'') != '')",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, title, path) in episodes {
            let quality = meaningful_quality(&title).or_else(|| {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                meaningful_quality(name)
            });
            if let Some(quality) = quality {
                changed += self.conn.execute(
                    "UPDATE episodes SET quality_score=?1 WHERE id=?2",
                    params![quality.score_with_settings(settings), id],
                )?;
            }
        }
        // Film senza una release tracciata.
        let movies = self
            .conn
            .prepare(
                "SELECT m.id, COALESCE(NULLIF(m.title,''), m.name, '') FROM movies m
                 WHERE m.magnet_hash IS NULL
                    OR NOT EXISTS(SELECT 1 FROM torrent_meta t WHERE lower(t.hash)=lower(m.magnet_hash) AND COALESCE(t.metadata_json,'') != '')",
            )?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, title) in movies {
            if let Some(quality) = meaningful_quality(&title) {
                changed += self.conn.execute(
                    "UPDATE movies SET quality_score=?1 WHERE id=?2",
                    params![quality.score_with_settings(settings), id],
                )?;
            }
        }
        Ok(changed)
    }
    pub fn cleanup(&self, retain_cycles: i64, error_age_days: i64) -> Result<MaintenanceReport> {
        let old_cycles_removed = self.conn.execute("DELETE FROM cycle_history WHERE id NOT IN (SELECT id FROM cycle_history ORDER BY id DESC LIMIT ?1)", [retain_cycles.clamp(1, 10000)])?;
        let stale_torrents_removed = self.conn.execute(
            "DELETE FROM torrent_meta WHERE status='error' AND updated_at < datetime('now', ?1)",
            [format!("-{} days", error_age_days.max(1))],
        )?;
        Ok(MaintenanceReport {
            rescored: 0,
            old_cycles_removed,
            stale_torrents_removed,
        })
    }
    /// Periodic data hygiene: bounded tables are trimmed and (caller-side, via
    /// `run_db_action`) the databases are compacted. Sonarr's `HousekeepingService`
    /// is the reference behaviour. The report is serialisable for the UI/API.
    pub fn housekeeping(&self, params: &HousekeepingParams) -> Result<HousekeepingReport> {
        let cleanup = self.cleanup(params.retain_cycles, params.error_age_days)?;
        let now = Utc::now();
        let cutoff = |days: i64| {
            (now - chrono::Duration::days(days.max(0))).to_rfc3339()
        };
        let seen_removed = if params.seen_days > 0 {
            self.prune_seen_older_than(params.seen_days)?
        } else {
            0
        };
        let gap_logs_removed = if params.gap_log_days > 0 {
            self.conn.execute(
                "DELETE FROM gap_search_log WHERE last_searched_at < ?1",
                [cutoff(params.gap_log_days)],
            )?
        } else {
            0
        };
        let upgrade_backups_removed = if params.upgrade_backup_days > 0 {
            self.conn.execute(
                "DELETE FROM upgrade_backup WHERE created_at < ?1",
                [cutoff(params.upgrade_backup_days)],
            )?
        } else {
            0
        };
        // Removed-torrent history is only trimmed when the user opts in
        // (`history_days > 0`): it powers the UI history, so the default keeps it.
        let old_history_removed = if params.history_days > 0 {
            self.conn.execute(
                "DELETE FROM torrent_meta WHERE removed_at IS NOT NULL AND removed_at < ?1",
                [cutoff(params.history_days)],
            )?
        } else {
            0
        };
        // Expired provider backoff rows older than a week are dead state.
        let stale_providers_removed = self.conn.execute(
            "DELETE FROM provider_status WHERE disabled_till IS NOT NULL AND disabled_till < ?1",
            [cutoff(7)],
        )?;
        Ok(HousekeepingReport {
            old_cycles_removed: cleanup.old_cycles_removed,
            stale_torrents_removed: cleanup.stale_torrents_removed,
            seen_removed,
            gap_logs_removed,
            upgrade_backups_removed,
            old_history_removed,
            stale_providers_removed,
        })
    }

    pub fn count_keyword(&self, keyword: &str) -> Result<i64> {
        self.count_keywords(&[keyword.to_string()])
    }

    pub fn count_keywords(&self, keywords: &[String]) -> Result<i64> {
        let keywords = normalized_keyword_terms(keywords);
        if keywords.is_empty() {
            return Ok(0);
        }
        let torrent_clause = keyword_clause(&["name", "title", "series_name"], keywords.len());
        let movie_clause = keyword_clause(&["COALESCE(title,name)"], keywords.len());
        let episode_clause = keyword_clause(&["s.name"], keywords.len());
        let series_clause = keyword_clause(&["name"], keywords.len());
        let seen_clause = keyword_clause(&["COALESCE(name,title)"], keywords.len());
        let mut bindings = keyword_bindings(&keywords, &["name", "title", "series_name"]);
        bindings.extend(keyword_bindings(&keywords, &["COALESCE(title,name)"]));
        bindings.extend(keyword_bindings(&keywords, &["s.name"]));
        bindings.extend(keyword_bindings(&keywords, &["name"]));
        bindings.extend(keyword_bindings(&keywords, &["COALESCE(name,title)"]));
        bindings.extend(keyword_bindings(&keywords, &["COALESCE(name,title)"]));
        let count: i64 = self.conn.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM torrent_meta WHERE {torrent_clause}) \
                 + (SELECT COUNT(*) FROM movies WHERE {movie_clause}) \
                 + (SELECT COUNT(*) FROM episodes e JOIN series s ON s.id=e.series_id WHERE {episode_clause}) \
                 + (SELECT COUNT(*) FROM series WHERE {series_clause}) \
                 + (SELECT COUNT(*) FROM movie_feed_seen WHERE {seen_clause}) \
                 + (SELECT COUNT(*) FROM series_feed_seen WHERE {seen_clause})"
            ),
            params_from_iter(bindings.iter()),
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn prune_keyword(&self, keyword: &str) -> Result<usize> {
        self.prune_keywords(&[keyword.to_string()])
    }

    pub fn prune_keywords(&self, keywords: &[String]) -> Result<usize> {
        let keywords = normalized_keyword_terms(keywords);
        if keywords.is_empty() {
            return Ok(0);
        }
        let mut removed = 0;
        let torrent_clause = keyword_clause(&["name", "title", "series_name"], keywords.len());
        let torrent_bindings = keyword_bindings(&keywords, &["name", "title", "series_name"]);
        removed += self.conn.execute(
            &format!("DELETE FROM torrent_meta WHERE {torrent_clause}"),
            params_from_iter(torrent_bindings.iter()),
        )?;
        let movie_clause = keyword_clause(&["COALESCE(title,name)"], keywords.len());
        let movie_bindings = keyword_bindings(&keywords, &["COALESCE(title,name)"]);
        removed += self.conn.execute(
            &format!("DELETE FROM movies WHERE {movie_clause}"),
            params_from_iter(movie_bindings.iter()),
        )?;
        let episode_clause = keyword_clause(&["name"], keywords.len());
        let episode_bindings = keyword_bindings(&keywords, &["name"]);
        removed += self.conn.execute(
            &format!("DELETE FROM episodes WHERE series_id IN (SELECT id FROM series WHERE {episode_clause})"),
            params_from_iter(episode_bindings.iter()),
        )?;
        let series_clause = keyword_clause(&["name"], keywords.len());
        let series_bindings = keyword_bindings(&keywords, &["name"]);
        removed += self.conn.execute(
            &format!("DELETE FROM series WHERE {series_clause}"),
            params_from_iter(series_bindings.iter()),
        )?;
        let seen_clause = keyword_clause(&["COALESCE(name,title)"], keywords.len());
        let seen_bindings = keyword_bindings(&keywords, &["COALESCE(name,title)"]);
        removed += self.conn.execute(
            &format!("DELETE FROM movie_feed_seen WHERE {seen_clause}"),
            params_from_iter(seen_bindings.iter()),
        )?;
        removed += self.conn.execute(
            &format!("DELETE FROM series_feed_seen WHERE {seen_clause}"),
            params_from_iter(seen_bindings.iter()),
        )?;
        Ok(removed)
    }

    /// Anteprima della pulizia per parola chiave: elenca gli elementi che
    /// corrispondono (torrent, film, episodi, serie e "visti nei feed") senza
    /// rimuoverli, così l'utente vede *cosa* verrebbe eliminato.
    pub fn search_keywords(
        &self,
        keywords: &[String],
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let keywords = normalized_keyword_terms(keywords);
        if keywords.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 1000);
        let queries: Vec<(&str, &str, &[&str])> = vec![
            (
                "Torrent",
                "SELECT COALESCE(NULLIF(name,''),NULLIF(title,''),series_name,hash), COALESCE(status,'') FROM torrent_meta WHERE ",
                &["name", "title", "series_name"],
            ),
            (
                "Film",
                "SELECT COALESCE(NULLIF(title,''),name,''), CAST(COALESCE(year,0) AS TEXT) FROM movies WHERE ",
                &["COALESCE(title,name)"],
            ),
            (
                "Episodio",
                "SELECT s.name || ' S' || printf('%02d',e.season) || 'E' || printf('%02d',e.episode), COALESCE(e.title,'') FROM episodes e JOIN series s ON s.id=e.series_id WHERE ",
                &["s.name"],
            ),
            (
                "Serie",
                "SELECT name, COALESCE(quality,'') FROM series WHERE ",
                &["name"],
            ),
            (
                "Film (feed)",
                "SELECT COALESCE(NULLIF(name,''),title), 'visto nei feed' FROM movie_feed_seen WHERE ",
                &["COALESCE(name,title)"],
            ),
            (
                "Serie (feed)",
                "SELECT COALESCE(NULLIF(name,''),title), 'visto nei feed' FROM series_feed_seen WHERE ",
                &["COALESCE(name,title)"],
            ),
        ];
        let mut items = Vec::new();
        for (source, sql, columns) in queries {
            let clause = keyword_clause(columns, keywords.len());
            let bindings = keyword_bindings(&keywords, columns);
            let mut statement = self.conn.prepare(&format!("{sql}{clause} LIMIT {limit}"))?;
            let rows = statement.query_map(params_from_iter(bindings.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (title, detail) = row?;
                items.push(serde_json::json!({
                    "source": source,
                    "title": title,
                    "detail": detail,
                }));
            }
        }
        Ok(items)
    }

    /// Numero di gruppi distinti "visti nei feed" (film, serie).
    pub fn seen_counts(&self) -> Result<(i64, i64)> {
        let movies: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT group_key) FROM movie_feed_seen WHERE group_key IS NOT NULL AND group_key <> ''",
            [],
            |row| row.get(0),
        )?;
        let series: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT group_key) FROM series_feed_seen WHERE group_key IS NOT NULL AND group_key <> ''",
            [],
            |row| row.get(0),
        )?;
        Ok((movies, series))
    }

    /// Elimina le release "viste" più vecchie di `days` giorni (0 = nessuna pulizia).
    /// Counts what `cleanup` + `prune_seen_older_than` would remove, without
    /// deleting anything.
    pub fn prune_preview(&self, retain_cycles: i64, error_age_days: i64, seen_days: i64) -> Result<PrunePreview> {
        let retain = retain_cycles.clamp(1, 10000);
        let old_cycles: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cycle_history WHERE id NOT IN (SELECT id FROM cycle_history ORDER BY id DESC LIMIT ?1)",
            [retain],
            |row| row.get(0),
        )?;
        let stale_torrents: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM torrent_meta WHERE status='error' AND updated_at < datetime('now', ?1)",
            [format!("-{} days", error_age_days.max(1))],
            |row| row.get(0),
        )?;
        let seen_to_remove = if seen_days <= 0 {
            0usize
        } else {
            let cutoff = (Utc::now() - chrono::Duration::days(seen_days.max(1))).to_rfc3339();
            let count: i64 = self.conn.query_row(
                "SELECT (SELECT COUNT(*) FROM movie_feed_seen WHERE found_at < ?1) + (SELECT COUNT(*) FROM series_feed_seen WHERE found_at < ?1)",
                [&cutoff],
                |row| row.get(0),
            )?;
            count.max(0) as usize
        };
        Ok(PrunePreview {
            old_cycles_to_remove: old_cycles.max(0) as usize,
            stale_torrents_to_remove: stale_torrents.max(0) as usize,
            seen_to_remove,
        })
    }

    /// Deletes specific "seen in feed" rows by id (targeted cleanup after a
    /// preview). Returns `(movie rows, series rows)` removed.
    pub fn prune_seen_by_ids(&self, movie_ids: &[i64], series_ids: &[i64]) -> Result<(usize, usize)> {
        let mut movies = 0usize;
        for id in movie_ids.iter().take(1000) {
            movies += self
                .conn
                .execute("DELETE FROM movie_feed_seen WHERE id=?1", [id])?;
        }
        let mut series = 0usize;
        for id in series_ids.iter().take(1000) {
            series += self
                .conn
                .execute("DELETE FROM series_feed_seen WHERE id=?1", [id])?;
        }
        Ok((movies, series))
    }

    pub fn prune_seen_older_than(&self, days: i64) -> Result<usize> {
        if days <= 0 {
            return Ok(0);
        }
        let cutoff = (Utc::now() - chrono::Duration::days(days.max(1))).to_rfc3339();
        let mut removed = self.conn.execute(
            "DELETE FROM movie_feed_seen WHERE found_at < ?1",
            [&cutoff],
        )?;
        removed += self.conn.execute(
            "DELETE FROM series_feed_seen WHERE found_at < ?1",
            [&cutoff],
        )?;
        Ok(removed)
    }

    /// (series name, archive path) for downloaded episodes with a stored
    /// archive path; used by the periodic rename verification.
    pub fn archived_episode_files(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT COALESCE(s.name,''), COALESCE(e.archive_path,'') FROM episodes e JOIN series s ON s.id=e.series_id WHERE COALESCE(e.archive_path,'')<>'' AND e.downloaded_at IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn pending_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM pending_downloads", [], |r| r.get(0))?)
    }

    /// Runs SQLite maintenance: `vacuum` compacts the file, `analyze` refreshes stats.
    pub fn optimize(&self, action: &str) -> Result<()> {
        optimize_connection(&self.conn, action)
    }

    /// Dimensione stimata del database in byte (page_count × page_size).
    pub fn db_size_bytes(&self) -> i64 {
        connection_size_bytes(&self.conn)
    }

    /// Numero totale di righe nelle tabelle principali.
    pub fn db_total_rows(&self) -> i64 {
        [
            "series",
            "episodes",
            "movies",
            "pending_downloads",
            "cycle_history",
            "torrent_meta",
            "gap_search_log",
            "ignored_episodes",
            "blocklist",
        ]
        .iter()
        .map(|table| {
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0)
        })
        .sum()
    }

    /// Per-series aggregate: (name, total episodes, downloaded episodes, last download).
    pub fn series_summaries(&self) -> Result<Vec<SeriesSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT s.name, COUNT(e.id), COALESCE(SUM(CASE WHEN e.downloaded_at IS NOT NULL THEN 1 ELSE 0 END),0), MAX(e.downloaded_at) FROM series s LEFT JOIN episodes e ON e.series_id=s.id GROUP BY s.name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn info(&self) -> Result<serde_json::Value> {
        let mut counts = serde_json::Map::new();
        for table in [
            "series",
            "episodes",
            "movies",
            "pending_downloads",
            "cycle_history",
            "torrent_meta",
            "gap_search_log",
            "ignored_episodes",
            "blocklist",
        ] {
            let count: i64 = self
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap_or(0);
            counts.insert(table.into(), serde_json::json!(count));
        }
        let downloaded_episodes: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE downloaded_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let downloaded_movies: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM movies WHERE downloaded_at IS NOT NULL AND removed_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(serde_json::json!({
            "counts": counts,
            "downloaded_episodes": downloaded_episodes,
            "downloaded_movies": downloaded_movies,
        }))
    }

    pub fn has_data(&self) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM series) + (SELECT COUNT(*) FROM movies) + (SELECT COUNT(*) FROM episodes)",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn reset_movie_by_name(&self, name: &str) -> Result<usize> {
        let reset = self.conn.execute(
            "UPDATE movies SET downloaded_at=NULL, removed_at=NULL WHERE name=?1",
            [name],
        )?;
        Ok(reset)
    }

    /// La configurazione film usa un id stabile, mentre lo storico dei file
    /// usa ancora nome+anno. Quando l'utente corregge il metadata tramite
    /// TMDB/TVDB, aggiorna quell'identità senza toccare hash, qualità, date o
    /// stato di download.
    pub fn rename_movie_identity(
        &self,
        old_name: &str,
        old_year: &str,
        new_name: &str,
        new_year: &str,
    ) -> Result<usize> {
        let old_year = old_year.trim().parse::<i64>().ok();
        let new_year = new_year.trim().parse::<i64>().ok();
        Ok(self.conn.execute(
            "UPDATE movies SET name=?1, year=?2 WHERE name=?3 AND year IS ?4",
            params![new_name, new_year, old_name, old_year],
        )?)
    }

    /// Registra nel "visto nei feed" tutte le release di un ciclo, in un'unica
    /// transazione. Le release già note vengono aggiornate (found_at, magnet,
    /// qualità) mantenendo `first_seen_at` originale.
    pub fn record_seen_batch(&self, releases: &[Release]) -> Result<()> {
        if releases.is_empty() {
            return Ok(());
        }
        let transaction = self.conn.unchecked_transaction()?;
        for release in releases {
            if release.kind == "series" {
                insert_series_seen(&transaction, release)?;
            } else {
                insert_movie_seen(&transaction, release)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn movies_seen_grouped(
        &self,
        offset: usize,
        limit: usize,
        query: &str,
    ) -> Result<(Vec<FeedSeenGroup>, i64)> {
        let like = seen_like_pattern(query);
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT group_key) FROM movie_feed_seen
             WHERE group_key IS NOT NULL AND group_key <> '' AND COALESCE(name,title) LIKE ?1",
            [&like],
            |row| row.get(0),
        )?;
        let mut statement = self.conn.prepare(
            "SELECT group_key,
                    MAX(COALESCE(NULLIF(name,''),title)) AS group_name,
                    MAX(year) AS year,
                    0 AS season,
                    COUNT(*) AS cnt,
                    MAX(quality_score) AS best_score,
                    SUBSTR(MAX(PRINTF('%010d', quality_score) || COALESCE(resolution,'unknown')),11) AS best_resolution,
                    MAX(found_at) AS latest_found,
                    MIN(COALESCE(first_seen_at,found_at)) AS first_found
             FROM movie_feed_seen
             WHERE group_key IS NOT NULL AND group_key <> '' AND COALESCE(name,title) LIKE ?3
             GROUP BY group_key ORDER BY latest_found DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = statement.query_map(
            params![limit.clamp(1, 500) as i64, offset as i64, like],
            feed_seen_group_from_row,
        )?;
        let groups = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((groups, total))
    }

    pub fn series_seen_grouped(
        &self,
        offset: usize,
        limit: usize,
        query: &str,
    ) -> Result<(Vec<FeedSeenGroup>, i64)> {
        let like = seen_like_pattern(query);
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT group_key) FROM series_feed_seen
             WHERE group_key IS NOT NULL AND group_key <> '' AND COALESCE(name,title) LIKE ?1",
            [&like],
            |row| row.get(0),
        )?;
        let mut statement = self.conn.prepare(
            "SELECT group_key,
                    MAX(COALESCE(NULLIF(name,''),title)) AS group_name,
                    0 AS year,
                    MAX(season) AS season,
                    COUNT(*) AS cnt,
                    MAX(quality_score) AS best_score,
                    SUBSTR(MAX(PRINTF('%010d', quality_score) || COALESCE(resolution,'unknown')),11) AS best_resolution,
                    MAX(found_at) AS latest_found,
                    MIN(COALESCE(first_seen_at,found_at)) AS first_found
             FROM series_feed_seen
             WHERE group_key IS NOT NULL AND group_key <> '' AND COALESCE(name,title) LIKE ?3
             GROUP BY group_key ORDER BY latest_found DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = statement.query_map(
            params![limit.clamp(1, 500) as i64, offset as i64, like],
            feed_seen_group_from_row,
        )?;
        let groups = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((groups, total))
    }

    /// Release di un gruppo "visto" (film o serie), dalla migliore qualità.
    pub fn seen_by_group(&self, kind: &str, group_key: &str, limit: usize) -> Result<Vec<FeedSeenEntry>> {
        let table = if kind == "series" {
            "series_feed_seen"
        } else {
            "movie_feed_seen"
        };
        let sql = format!(
            "SELECT id,title,COALESCE(name,''),{year},{season},{episode},COALESCE(resolution,'unknown'),COALESCE(codec,'unknown'),COALESCE(audio,'unknown'),COALESCE(quality_score,0),COALESCE(magnet,''),COALESCE(source,''),found_at
             FROM {table} WHERE group_key=?1 ORDER BY quality_score DESC, found_at DESC LIMIT ?2",
            year = if kind == "series" { "0" } else { "COALESCE(year,0)" },
            season = if kind == "series" { "COALESCE(season,0)" } else { "0" },
            episode = if kind == "series" { "COALESCE(episode,0)" } else { "0" }
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(
            params![group_key, limit.clamp(1, 1000) as i64],
            feed_seen_entry_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Release di una puntata già viste durante le scansioni RSS/HTML. Il nome
    /// della serie viene verificato dal chiamante, così può applicare anche gli
    /// alias configurati con la stessa normalizzazione del parser.
    pub fn series_feed_for_episode(
        &self,
        season: i64,
        episode: i64,
        limit: usize,
    ) -> Result<Vec<(String, String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT title,COALESCE(magnet,''),COALESCE(source,'')
             FROM series_feed_seen
             WHERE season=?1 AND episode=?2
             ORDER BY quality_score DESC, found_at DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![season, episode, limit.clamp(1, 200) as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Pattern LIKE per la ricerca nei gruppi "visti", con la stessa semantica del
/// legacy: `*`/`?` sono wildcard, altrimenti la parola è cercata come sottostringa.
fn seen_like_pattern(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return "%".to_string();
    }
    if trimmed.contains('*') || trimmed.contains('?') {
        trimmed.replace('*', "%").replace('?', "_")
    } else {
        format!("%{trimmed}%")
    }
}

fn insert_movie_seen(conn: &Connection, release: &Release) -> Result<()> {
    let name = crate::utils::extract_clean_movie_name(&release.title);
    let group_key = crate::utils::condensed_key(&name);
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO movie_feed_seen (title,name,year,resolution,codec,audio,quality_score,magnet,source,found_at,first_seen_at,group_key)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,?11)
         ON CONFLICT(title) DO UPDATE SET
             found_at=excluded.found_at,
             magnet=excluded.magnet,
             source=excluded.source,
             name=excluded.name,
             year=excluded.year,
             quality_score=excluded.quality_score,
             resolution=excluded.resolution,
             codec=excluded.codec,
             audio=excluded.audio,
             group_key=excluded.group_key,
             first_seen_at=COALESCE(movie_feed_seen.first_seen_at, excluded.first_seen_at)",
        params![
            release.title,
            name,
            release.year.unwrap_or(0),
            release.quality.resolution,
            release.quality.codec,
            release.quality.audio,
            release.quality.score(),
            release.magnet,
            release.source,
            now,
            group_key,
        ],
    )?;
    Ok(())
}

fn insert_series_seen(conn: &Connection, release: &Release) -> Result<()> {
    let name = release
        .series
        .clone()
        .unwrap_or_else(|| release.title.clone());
    let group_key = crate::utils::condensed_key(&name);
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO series_feed_seen (title,name,season,episode,resolution,codec,audio,quality_score,magnet,source,found_at,first_seen_at,group_key)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?12)
         ON CONFLICT(title) DO UPDATE SET
             found_at=excluded.found_at,
             magnet=excluded.magnet,
             source=excluded.source,
             name=excluded.name,
             season=excluded.season,
             episode=excluded.episode,
             quality_score=excluded.quality_score,
             resolution=excluded.resolution,
             codec=excluded.codec,
             audio=excluded.audio,
             group_key=excluded.group_key,
             first_seen_at=COALESCE(series_feed_seen.first_seen_at, excluded.first_seen_at)",
        params![
            release.title,
            name,
            release.season.unwrap_or(0),
            release.episode.unwrap_or(0),
            release.quality.resolution,
            release.quality.codec,
            release.quality.audio,
            release.quality.score(),
            release.magnet,
            release.source,
            now,
            group_key,
        ],
    )?;
    Ok(())
}

fn feed_seen_group_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedSeenGroup> {
    Ok(FeedSeenGroup {
        group_key: row.get(0)?,
        group_name: row.get(1)?,
        year: row.get(2)?,
        season: row.get(3)?,
        count: row.get(4)?,
        best_score: row.get(5)?,
        best_resolution: row.get(6)?,
        latest_found: row.get(7)?,
        first_found: row.get(8)?,
    })
}

fn feed_seen_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedSeenEntry> {
    Ok(FeedSeenEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        name: row.get(2)?,
        year: row.get(3)?,
        season: row.get(4)?,
        episode: row.get(5)?,
        resolution: row.get(6)?,
        codec: row.get(7)?,
        audio: row.get(8)?,
        quality_score: row.get(9)?,
        magnet: row.get(10)?,
        source: row.get(11)?,
        found_at: row.get(12)?,
    })
}

fn normalized_keyword_terms(keywords: &[String]) -> Vec<String> {
    keywords
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .collect()
}

fn keyword_clause(columns: &[&str], term_count: usize) -> String {
    (0..term_count)
        .map(|_| {
            let clause = columns
                .iter()
                .map(|column| format!("{column} LIKE ?"))
                .collect::<Vec<_>>()
                .join(" OR ");
            format!("({clause})")
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn keyword_bindings(keywords: &[String], columns: &[&str]) -> Vec<String> {
    keywords
        .iter()
        .flat_map(|keyword| {
            let pattern = format!("%{keyword}%");
            std::iter::repeat_n(pattern, columns.len())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Quality;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn release() -> Release {
        Release { torrent_url: None,
            title: "Example.S01E01.1080p".into(),
            magnet: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "1080p".into(),
                ..Default::default()
            },
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        }
    }

    #[test]
    fn reconciles_torrents_missing_from_session() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-reconcile-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = release();
        let digest = magnet_hash(&release.magnet).unwrap();
        db.register_torrent(&release).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Example')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (1,1,1,'Example.S01E01.1080p',100,?1,?2)",
                params![digest, release.magnet],
            )
            .unwrap();

        // Sessione vuota: il torrent è orfano → marcato error e placeholder rimosso.
        let removed = db
            .reconcile_missing_torrents(&std::collections::HashSet::new())
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            db.torrent_status(&digest).unwrap().as_deref(),
            Some("error")
        );
        let leftover: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE magnet_hash=?1",
                [&digest],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0, "placeholder rimosso per permettere il retry");

        // Con il torrent vivo nella sessione non viene toccato.
        db.register_torrent(&release).unwrap();
        let mut live = std::collections::HashSet::new();
        live.insert(digest.clone());
        assert_eq!(db.reconcile_missing_torrents(&live).unwrap(), 0);
        assert_eq!(
            db.torrent_status(&digest).unwrap().as_deref(),
            Some("queued")
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn reconciles_pack_placeholders_when_torrent_season_differs() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-pack-season-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Example')", [])
            .unwrap();
        let mut advertised = release();
        advertised.title = "Example.S06E01-06.1080p".into();
        advertised.season = Some(6);
        advertised.is_pack = true;
        advertised.episode_range = (1..=6).collect();
        let hash = magnet_hash(&advertised.magnet).unwrap();
        db.register_torrent(&advertised).unwrap();
        for episode in 1..=6 {
            db.conn
                .execute(
                    "INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (1,6,?1,?2,100,?3,?4)",
                    params![
                        episode,
                        advertised.title,
                        (episode == 1).then_some(hash.as_str()),
                        advertised.magnet,
                    ],
                )
                .unwrap();
        }
        let mut actual = advertised.clone();
        actual.title = "Example.S05E01-06.1080p".into();
        actual.season = Some(5);
        db.reconcile_pack_release(&hash, &actual).unwrap();

        assert_eq!(db.torrent_meta(&hash).unwrap().unwrap().release.season, Some(5));
        let old_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM episodes WHERE season=6", [], |row| row.get(0))
            .unwrap();
        let new_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM episodes WHERE season=5", [], |row| row.get(0))
            .unwrap();
        assert_eq!(old_count, 0);
        assert_eq!(new_count, 6);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn failed_release_clears_placeholders_to_allow_fallback() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-fallback-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Example')", [])
            .unwrap();
        let pack = Release { torrent_url: None,
            title: "Example.S01E01-08.2160p".into(),
            magnet: "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "2160p".into(),
                ..Default::default()
            },
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: true,
            episode_range: vec![1, 2, 3, 4, 5, 6, 7, 8],
            year: None,
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        db.register_torrent(&pack).unwrap();
        let digest = magnet_hash(&pack.magnet).unwrap();
        for episode in 1..=8 {
            // Solo il primo episodio porta l'hash (UNIQUE); gli altri il link.
            let hash = (episode == 1).then_some(digest.as_str());
            db.conn
                .execute(
                    "INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link) VALUES (1,1,?1,?2,1880,?3,?4)",
                    params![episode, pack.title, hash, pack.magnet],
                )
                .unwrap();
        }

        // Il 4K è andato in stallo: marcatura errore + pulizia dei placeholder.
        db.mark_torrent_error(&digest, "stalled download").unwrap();
        let leftover: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM episodes WHERE series_id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(leftover, 0, "i placeholder non devono bloccare il ripiego");

        // A questo punto una release inferiore ma disponibile (1080p) viene
        // approvata come ripiego automatico.
        let fallback = Release { torrent_url: None,
            title: "Example.S01.1080p".into(),
            magnet: "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "1080p".into(),
                ..Default::default()
            },
            kind: "series".into(),
            series: Some("Example".into()),
            season: Some(1),
            episode: Some(0),
            is_pack: true,
            episode_range: vec![0],
            year: None,
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let fallback_hash = magnet_hash(&fallback.magnet).unwrap();
        let score = fallback.quality.score();
        let (approved, reason) = db
            .check_series_pack(
                &fallback,
                &fallback_hash,
                score,
                200,
                &crate::models::ApprovalContext::default(),
                false,
            )
            .unwrap();
        assert!(approved, "il 1080p deve essere approvato: {reason}");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn archive_index_blocks_release_already_present_on_disk() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-archiveindex-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = release();
        let score = release.quality.score();

        // Un 4K è già su disco ma non nel DB: la release 1080p va scartata.
        let mut index = crate::models::ArchiveQualityIndex::default();
        index.best.insert(
            (1, 1),
            (
                Quality {
                    resolution: "2160p".into(),
                    ..Default::default()
                },
                2000,
            ),
        );
        let context = crate::models::ApprovalContext {
            archive: index,
            live: crate::models::LiveDownloads::default(),
            forbid_upgrade: false, gap_episode: false,
        };
        let (approved, reason) = db
            .check_series_scored(&release, score, 200, &context)
            .unwrap();
        assert!(!approved, "atteso duplicate dal disco, ottenuto {reason}");

        // Senza indice (nessun file su disco) il candidato è approvato.
        let (approved, _) = db
            .check_series_scored(
                &release,
                score,
                200,
                &crate::models::ApprovalContext::default(),
            )
            .unwrap();
        assert!(approved);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn live_session_blocks_episode_already_downloading() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-live-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = release();
        let score = release.quality.score();

        let mut live = crate::models::LiveDownloads::default();
        live.episodes
            .insert((crate::parser::normalize_series_name("Example"), 1, 1));
        let context = crate::models::ApprovalContext {
            archive: Default::default(),
            live,
            forbid_upgrade: false, gap_episode: false,
        };
        let (approved, reason) = db
            .check_series_scored(&release, score, 200, &context)
            .unwrap();
        assert!(!approved, "atteso active_episode, ottenuto {reason}");
        assert_eq!(reason, "active_episode");

        // Stesso hash vivo → bloccato anche da solo.
        let mut live = crate::models::LiveDownloads::default();
        live.hashes.insert(
            magnet_hash(&release.magnet).unwrap().to_ascii_lowercase(),
        );
        let context = crate::models::ApprovalContext {
            archive: Default::default(),
            live,
            forbid_upgrade: false, gap_episode: false,
        };
        let (approved, reason) = db
            .check_series_scored(&release, score, 200, &context)
            .unwrap();
        assert!(!approved, "atteso active_episode per hash, ottenuto {reason}");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn excludes_ignored_seasons_from_series_episodes() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-ignored-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute(
                "INSERT INTO series(id,name,ignored_seasons) VALUES (1,'Show','[2]')",
                [],
            )
            .unwrap();
        for (season, count) in [(1, 2), (2, 3)] {
            db.conn
                .execute(
                    "INSERT INTO series_metadata(series_name,season,episode_count,updated_at) VALUES ('Show',?1,?2,datetime('now'))",
                    params![season, count],
                )
                .unwrap();
        }
        let items = db.episodes_for_series("Show", &[]).unwrap();
        assert_eq!(items.len(), 2, "solo gli episodi della stagione 1");
        assert!(items.iter().all(|item| item.season == 1));
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn excludes_config_seasons_passed_as_extra_ignored() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-extra-ignored-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Show')", [])
            .unwrap();
        for (season, count) in [(1, 2), (2, 3)] {
            db.conn
                .execute(
                    "INSERT INTO series_metadata(series_name,season,episode_count,updated_at) VALUES ('Show',?1,?2,datetime('now'))",
                    params![season, count],
                )
                .unwrap();
        }
        // Ignora la stagione 1 passata dalla configurazione (non dal DB).
        let items = db.episodes_for_series("Show", &[1]).unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|item| item.season == 2));
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn torrent_no_rename_flag_round_trips_case_insensitively() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-norename-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        assert!(!db.torrent_no_rename("ABCdef").unwrap());
        db.set_torrent_no_rename("ABCdef", true).unwrap();
        assert!(db.torrent_no_rename("abcdef").unwrap());
        db.set_torrent_no_rename("abcdef", false).unwrap();
        assert!(!db.torrent_no_rename("ABCDEF").unwrap());
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn torrent_metadata_round_trips_and_completion_updates_normalized_state() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = release();
        db.check_series(&release).unwrap();
        let mut later = release.clone();
        later.episode = Some(3);
        later.episode_range = vec![3];
        later.title = "Example.S01E03.1080p".into();
        later.magnet = "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd".into();
        db.check_series(&later).unwrap();
        assert_eq!(db.archive_gaps().unwrap(), vec![("Example".into(), 1, 2)]);
        db.save_series_metadata("Example", &[(1, 5)]).unwrap();
        assert_eq!(
            db.archive_gaps().unwrap(),
            vec![
                ("Example".into(), 1, 2),
                ("Example".into(), 1, 4),
                ("Example".into(), 1, 5)
            ]
        );
        assert!(!db.gap_recently_searched("Example", 1, 2, 23).unwrap());
        db.mark_gap_searched("Example", 1, 2).unwrap();
        assert!(db.gap_recently_searched("Example", 1, 2, 23).unwrap());
        db.register_torrent(&release).unwrap();
        let hash = magnet_hash(&release.magnet).unwrap();
        assert_eq!(
            db.torrent_meta(&hash).unwrap().unwrap().release.title,
            release.title
        );
        db.mark_torrent_completed(&hash, "/nas/example", 42)
            .unwrap();
        let status: String = db
            .conn
            .query_row(
                "SELECT status FROM torrent_meta WHERE hash=?1",
                [&hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
        let size: i64 = db
            .conn
            .query_row(
                "SELECT size_bytes FROM episodes WHERE magnet_hash=?1",
                [&hash],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(size, 42);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn manual_missing_search_uses_archive_presence_not_client_state() {
        let path = std::env::temp_dir().join(format!(
            "rextto-manual-gaps-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn.execute("INSERT INTO series(id,name) VALUES (1,'Example')", []).unwrap();
        db.save_series_metadata("Example", &[(1, 3)]).unwrap();
        // La puntata 1 è solo nel client: va ancora proposta dalla ricerca manuale.
        db.conn.execute("INSERT INTO episodes(series_id,season,episode,title,archive_path) VALUES (1,1,1,'Example S01E01','')", []).unwrap();
        // La puntata 2 è sul NAS: non va cercata.
        db.conn.execute("INSERT INTO episodes(series_id,season,episode,title,archive_path) VALUES (1,1,2,'Example S01E02','/nas/Example S01E02.mkv')", []).unwrap();
        assert_eq!(
            db.unarchived_episodes_for_series("Example", &[]).unwrap(),
            vec![(1, 1), (1, 3)]
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn season_pack_registers_all_episodes_and_rolls_back_as_one_release() {
        let path = std::env::temp_dir().join(format!(
            "rextto-pack-db-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let mut pack = release();
        pack.title = "Example.S01.Pack".into();
        pack.magnet = "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd".into();
        pack.episode = Some(1);
        pack.is_pack = true;
        pack.episode_range = vec![1, 2, 3];
        assert_eq!(db.check_series(&pack).unwrap(), (true, "approved".into()));
        assert_eq!(db.archive_gaps().unwrap(), Vec::<(String, i64, i64)>::new());
        db.rollback_release(&pack).unwrap();
        assert!(
            db.conn
                .query_row("SELECT COUNT(*) FROM episodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap()
                == 0
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn complete_pack_suppresses_metadata_gaps_only_while_queued_or_completed() {
        let path =
            std::env::temp_dir().join(format!("rextto-complete-pack-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let mut pack = release();
        pack.title = "Example.S01.COMPLETE.1080p".into();
        pack.magnet = "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd".into();
        pack.episode = Some(0);
        pack.is_pack = true;
        pack.episode_range = vec![0];
        assert_eq!(db.check_series(&pack).unwrap(), (true, "approved".into()));
        db.save_series_metadata("Example", &[(1, 3)]).unwrap();
        assert_eq!(db.archive_gaps().unwrap().len(), 3);
        db.register_torrent(&pack).unwrap();
        assert!(db.archive_gaps().unwrap().is_empty());
        let hash = magnet_hash(&pack.magnet).unwrap();
        db.mark_torrent_error(&hash, "failed").unwrap();
        assert_eq!(db.archive_gaps().unwrap().len(), 3);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn migrates_legacy_torrent_metadata_before_creating_status_index() {
        let path =
            std::env::temp_dir().join(format!("rextto-legacy-meta-{}", uuid::Uuid::new_v4()));
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch("CREATE TABLE torrent_meta (hash TEXT PRIMARY KEY, tag TEXT DEFAULT '', source TEXT DEFAULT '', updated_at TEXT NOT NULL);").unwrap();
        drop(legacy);
        let db = Database::open(&path).unwrap();
        let has_status: bool = db.conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('torrent_meta') WHERE name='status')", [], |row| row.get(0)).unwrap();
        let has_index: bool = db.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_torrent_meta_status')", [], |row| row.get(0)).unwrap();
        assert!(has_status);
        assert!(has_index);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn restores_soft_deleted_movie_by_magnet_hash() {
        let path = std::env::temp_dir().join(format!(
            "rextto-movie-db-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = Release { torrent_url: None,
            title: "Example Movie".into(),
            magnet: "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "1080p".into(),
                ..Default::default()
            },
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        assert_eq!(db.check_movie(&release).unwrap(), (true, "approved".into()));
        db.conn
            .execute(
                "UPDATE movies SET removed_at=datetime('now') WHERE magnet_hash=?1",
                [magnet_hash(&release.magnet).unwrap()],
            )
            .unwrap();
        assert_eq!(db.check_movie(&release).unwrap(), (true, "restored".into()));
        assert!(db
            .conn
            .query_row(
                "SELECT removed_at IS NULL FROM movies WHERE magnet_hash=?1",
                [magnet_hash(&release.magnet).unwrap()],
                |row| row.get::<_, bool>(0)
            )
            .unwrap());
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn movie_remux_upgrade_is_allowed_with_small_score_delta() {
        let path = std::env::temp_dir().join(format!(
            "rextto-movie-upgrade-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let old = Release {
            torrent_url: None,
            title: "Example Movie".into(),
            magnet: "magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "2160p".into(),
                source: "webdl".into(),
                ..Default::default()
            },
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        assert_eq!(db.check_movie(&old).unwrap(), (true, "approved".into()));
        db.register_torrent(&old).unwrap();

        let mut upgrade = old.clone();
        upgrade.magnet =
            "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        upgrade.quality.source = "remux".into();
        assert_eq!(
            db.check_movie_scored(&upgrade, upgrade.quality.score(), 200)
                .unwrap(),
            (true, "upgrade".into())
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn recent_downloads_merges_series_and_movies() {
        let path = std::env::temp_dir().join(format!(
            "rextto-recent-db-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let release = release();
        db.check_series(&release).unwrap();
        let hash = magnet_hash(&release.magnet).unwrap();
        db.register_torrent(&release).unwrap();
        db.mark_torrent_completed(&hash, "/nas/example", 42)
            .unwrap();
        let movie = Release { torrent_url: None,
            title: "Example Movie".into(),
            magnet: "magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678".into(),
            source: "rss".into(),
            quality: Quality {
                resolution: "1080p".into(),
                ..Default::default()
            },
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        db.check_movie(&movie).unwrap();
        db.conn
            .execute(
                "UPDATE movies SET downloaded_at=datetime('now'), size_bytes=99 WHERE magnet_hash=?1",
                [magnet_hash(&movie.magnet).unwrap()],
            )
            .unwrap();
        let downloads = db.recent_downloads(10).unwrap();
        assert_eq!(downloads.len(), 2);
        assert!(downloads
            .iter()
            .any(|item| item.kind == "series" && item.size_bytes == 42));
        assert!(downloads
            .iter()
            .any(|item| item.kind == "movie" && item.size_bytes == 99));
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn records_and_groups_seen_movies_and_series() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-seen-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let movie = |title: &str, resolution: &str| Release { torrent_url: None,
            title: title.into(),
            magnet: format!(
                "magnet:?xt=urn:btih:{}",
                crate::utils::stable_id(title)
            ),
            source: "rss".into(),
            quality: Quality {
                resolution: resolution.into(),
                ..Default::default()
            },
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2024),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let mut series_release = release();
        series_release.series = Some("Example Show".into());
        db.record_seen_batch(&[
            movie("The.Veil.2024.1080p.BluRay", "1080p"),
            movie("The.Veil.2024.2160p.WEB-DL", "2160p"),
            series_release,
        ])
        .unwrap();
        let (groups, total) = db.movies_seen_grouped(0, 50, "").unwrap();
        assert_eq!(total, 1, "le due release dello stesso film formano un gruppo");
        assert_eq!(groups[0].group_name, "The Veil");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].best_resolution, "2160p");
        let entries = db.seen_by_group("movie", &groups[0].group_key, 50).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].resolution, "2160p");
        let (series_groups, series_total) = db.series_seen_grouped(0, 50, "").unwrap();
        // La ricerca testuale filtra i gruppi.
        let (filtered, filtered_total) = db.movies_seen_grouped(0, 50, "veil").unwrap();
        assert_eq!(filtered_total, 1);
        assert_eq!(filtered[0].count, 2);
        let (none, none_total) = db.movies_seen_grouped(0, 50, "inesistente").unwrap();
        assert_eq!(none_total, 0);
        assert!(none.is_empty());
        assert_eq!(series_total, 1);
        assert_eq!(series_groups[0].group_name, "Example Show");
        assert_eq!(series_groups[0].season, 1);
        // Pulizia per età: le righe retrodatate vengono rimosse.
        db.conn
            .execute(
                "UPDATE movie_feed_seen SET found_at='2000-01-01T00:00:00+00:00'",
                [],
            )
            .unwrap();
        assert_eq!(db.prune_seen_older_than(30).unwrap(), 2);
        assert_eq!(db.seen_counts().unwrap(), (0, 1));
        assert_eq!(db.prune_seen_older_than(0).unwrap(), 0);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn prune_preview_counts_without_deleting() {
        use crate::models::CycleStats;
        let path = std::env::temp_dir().join(format!(
            "rextto-prune-preview-{}",
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        for _ in 0..5 {
            db.save_cycle(&CycleStats::default()).unwrap();
        }
        db.conn
            .execute(
                "INSERT INTO torrent_meta(hash,status,updated_at) VALUES \
                 ('h1','error', datetime('now','-30 days')),\
                 ('h2','error', datetime('now','-1 days'))",
                [],
            )
            .unwrap();
        let preview = db.prune_preview(2, 7, 0).unwrap();
        assert_eq!(preview.old_cycles_to_remove, 3);
        assert_eq!(preview.stale_torrents_to_remove, 1);
        // The preview must not delete anything.
        let cycles: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM cycle_history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cycles, 5);
        let report = db.cleanup(2, 7).unwrap();
        assert_eq!(report.old_cycles_removed, 3);
        assert_eq!(report.stale_torrents_removed, 1);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn extracts_renamed_title_from_archive_path() {
        assert_eq!(
            renamed_file_title("/home/user/SerieTV/Show - S01E01 - Pilot - [1080p].mkv"),
            "Show - S01E01 - Pilot - [1080p]"
        );
        // Le cartelle (import vecchi) e i percorsi vuoti non hanno un nome file.
        assert_eq!(renamed_file_title("/home/user/SerieTV/Show/"), "");
        assert_eq!(renamed_file_title(""), "");
    }

    #[test]
    fn season_pack_inferior_to_existing_episodes_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-pack-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Reacher')", [])
            .unwrap();
        // Episodi 2160p già archiviati (score alto).
        for episode in 1..=3 {
            db.conn
                .execute(
                    "INSERT INTO episodes(series_id,season,episode,title,quality_score,downloaded_at,archive_path) VALUES (1,4,?1,?2,1510,datetime('now'),'/x')",
                    params![
                        episode,
                        format!("Reacher.S04E0{episode}.ITA.ENG.2160p.AMZN.WEB-DL.DDP5.1.DV.HDR.H.265-MeM")
                    ],
                )
                .unwrap();
        }
        let pack = Release { torrent_url: None,
            title: "Reacher - Season 04 (2026) [1080p H265 ITA ENG EAC3 SUB ITA ENG WEB-DL]".into(),
            magnet: "magnet:?xt=urn:btih:1111111111111111111111111111111111111111".into(),
            source: "ExtTo".into(),
            quality: parse_quality(
                "Reacher - Season 04 (2026) [1080p H265 ITA ENG EAC3 SUB ITA ENG WEB-DL]",
            ),
            kind: "series".into(),
            series: Some("Reacher".into()),
            season: Some(4),
            episode: Some(0),
            is_pack: true,
            episode_range: vec![0],
            year: Some(2026),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let score = pack.quality.score();
        assert!(score < 1510, "il pack 1080p deve avere score inferiore");
        let (approved, reason) = db.check_series_scored(&pack, score, 50, &crate::models::ApprovalContext::default()).unwrap();
        assert!(!approved, "pack inferiore non deve essere approvato");
        assert_eq!(reason, "duplicate");

        // Pack per una stagione senza episodi: approvato (gap fill).
        let mut gap = pack.clone();
        gap.season = Some(5);
        gap.title = "Reacher - Season 05 (2027) [1080p H265 ITA ENG]".into();
        gap.magnet = "magnet:?xt=urn:btih:2222222222222222222222222222222222222222".into();
        let (approved_gap, _) = db.check_series_scored(&gap, score, 50, &crate::models::ApprovalContext::default()).unwrap();
        assert!(approved_gap, "pack per stagione vuota deve essere approvato");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn manual_pack_can_replace_unarchived_higher_quality_placeholders() {
        let path = std::env::temp_dir().join(format!("rextto-manual-pack-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        db.conn.execute("INSERT INTO series(id,name) VALUES (1,'Neagley')", []).unwrap();
        db.save_series_metadata("Neagley", &[(1, 2)]).unwrap();
        for episode in 1..=2 {
            db.conn.execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score) VALUES (1,1,?1,?2,1880)",
                params![episode, format!("Neagley.S01E0{episode}.2160p.DV.HDR.H.265")],
            ).unwrap();
        }
        let pack = Release { torrent_url: None,
            title: "Neagley.S01E01-02.1080p.AMZN.WEB-DL.ITA.ENG.DDP5.1.H.264-G66".into(),
            magnet: "magnet:?xt=urn:btih:abababababababababababababababababababab".into(),
            source: "ExtTo".into(),
            quality: parse_quality("Neagley.S01E01-02.1080p.AMZN.WEB-DL.ITA.ENG.DDP5.1.H.264-G66"),
            kind: "series".into(), series: Some("Neagley".into()), season: Some(1), episode: Some(1),
            is_pack: true, episode_range: vec![1, 2], year: None, discovered_at: Utc::now(),
            size_bytes: 0, seeders: -1, peers: -1,
        };
        let (approved, reason) = db.check_series_manual_scored(
            &pack, pack.quality.score(), 200, &crate::models::ApprovalContext::default(),
        ).unwrap();
        assert!(approved, "un pack manuale deve poter sostituire placeholder non archiviati: {reason}");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn season_pack_can_be_retried_when_placeholder_not_downloaded() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-retry-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Neagley')", [])
            .unwrap();
        let hash = "3333333333333333333333333333333333333333";
        let magnet = format!("magnet:?xt=urn:btih:{hash}");
        // Placeholder del pack precedente: esistono ma NON scaricati.
        for episode in 1..=2 {
            // Come nel flusso reale: solo il primo episodio porta il magnet_hash.
            let episode_hash: Option<&str> = if episode == 1 { Some(hash) } else { None };
            db.conn
                .execute(
                    "INSERT INTO episodes(series_id,season,episode,title,quality_score,magnet_hash,magnet_link,downloaded_at) VALUES (1,1,?1,?2,1680,?3,?4,NULL)",
                    params![
                        episode,
                        format!("Neagley.S01E0{episode}.2160p.DV.HDR.H.265-G66"),
                        episode_hash,
                        magnet
                    ],
                )
                .unwrap();
        }
        let pack = Release { torrent_url: None,
            title: "Neagley.S01E01-02.2160p.AMZN.WEB-DL.ITA.ENG.DDP5.1.DV.HDR.H.265-G66".into(),
            magnet: magnet.clone(),
            source: "ExtTo".into(),
            quality: parse_quality(
                "Neagley.S01E01-02.2160p.AMZN.WEB-DL.ITA.ENG.DDP5.1.DV.HDR.H.265-G66",
            ),
            kind: "series".into(),
            series: Some("Neagley".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: true,
            episode_range: vec![1, 2],
            year: Some(2026),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let score = pack.quality.score();
        // Retry consentito: i placeholder non sono "già scaricati".
        let (approved, _) = db.check_series_scored(&pack, score, 50, &crate::models::ApprovalContext::default()).unwrap();
        assert!(approved, "un placeholder non scaricato non deve bloccare il retry");
        // Ora l'episodio risulta scaricato: stesso magnet -> duplicato.
        db.conn
            .execute(
                "UPDATE episodes SET downloaded_at=datetime('now') WHERE magnet_hash=?1 OR magnet_link=?2",
                params![hash, magnet],
            )
            .unwrap();
        let (approved_again, reason) = db.check_series_scored(&pack, score, 50, &crate::models::ApprovalContext::default()).unwrap();
        assert!(!approved_again);
        assert_eq!(reason, "duplicate");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn rescore_normalizes_base_scores_with_settings() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-rescore-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Show')", [])
            .unwrap();
        // Titolo generico: la qualità va recuperata dal nome file.
        db.conn
            .execute(
                "INSERT INTO episodes(id,series_id,season,episode,title,quality_score,archive_path,downloaded_at) VALUES (1,1,1,1,'Prova di fiducia',980,'/nas/Show - S01E01 - Prova - [1080p][h265].mkv',datetime('now'))",
                [],
            )
            .unwrap();
        // Titolo con qualità.
        db.conn
            .execute(
                "INSERT INTO episodes(id,series_id,season,episode,title,quality_score,downloaded_at) VALUES (2,1,1,2,'Show.S01E02.1080p.WEB-DL.H.265',1000,datetime('now'))",
                [],
            )
            .unwrap();
        // Niente qualità né file: resta invariato.
        db.conn
            .execute(
                "INSERT INTO episodes(id,series_id,season,episode,title,quality_score) VALUES (3,1,1,3,'Episodio 3',500)",
                [],
            )
            .unwrap();
        let mut settings = std::collections::BTreeMap::new();
        settings.insert("score_res_1080p".to_string(), "1500".to_string());
        db.rescore(&settings).unwrap();
        let score = |id: i64| -> i64 {
            db.conn
                .query_row("SELECT quality_score FROM episodes WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        assert_eq!(score(1), 1700, "1080p h265 base 1200 + 500");
        assert_eq!(score(2), 1900, "1080p webdl h265 1400 + 500");
        assert_eq!(score(3), 500, "senza qualità non va toccato");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn history_search_filters_across_name_tag_and_path() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-history-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let insert = |hash: &str, name: &str, tag: &str, path: &str| {
            db.conn
                .execute(
                    "INSERT INTO torrent_meta(hash,name,tag,status,removed_at,progress,processed_path,updated_at) VALUES (?1,?2,?3,'completed',datetime('now'),1,?4,datetime('now'))",
                    params![hash, name, tag, path],
                )
                .unwrap();
        };
        insert("aaaa", "Silo.S01E01.1080p", "nas", "/media/Silo/S01E01.mkv");
        insert("bbbb", "Altro.Film.2024", "temp", "/tmp/altro.mkv");
        insert("cccc", "Silo.S01E02.2160p", "nas", "/media/Silo/S01E02.mkv");

        let search = |query: &str| db.completed_torrents(0, 10, query).unwrap();
        // Una parola cerca in tutti i campi utili.
        assert_eq!(search("silo").1, 2);
        // Più parole devono comparire tutte (ricerca intelligente).
        assert_eq!(search("silo nas").1, 2);
        assert_eq!(search("silo temp").1, 0);
        // Anche tag e percorso sono indicizzati.
        assert_eq!(search("nas").1, 2);
        assert_eq!(search("/tmp/altro").1, 1);
        // La paginazione resta coerente col filtro.
        let (page, total) = db.completed_torrents(1, 1, "silo").unwrap();
        assert_eq!(total, 2);
        assert_eq!(page.len(), 1);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn completed_history_keeps_only_the_last_30_days() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-history-retention-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        let timestamp = |modifier: &str| {
            db.conn
                .query_row("SELECT datetime('now', ?1)", [modifier], |row| row.get::<_, String>(0))
                .unwrap()
        };
        let insert = |hash: &str, name: &str, completed_at: &str| {
            db.conn
                .execute(
                    "INSERT INTO torrent_meta(hash,name,status,removed_at,completed_at,progress,updated_at) VALUES (?1,?2,'completed',?3,?3,1,?3)",
                    params![hash, name, completed_at],
                )
                .unwrap();
        };
        insert("recent", "Recent download", &timestamp("-29 days"));
        insert("old", "Old download", &timestamp("-31 days"));

        let (items, total) = db.completed_torrents(0, 10, "").unwrap();
        assert_eq!(total, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Recent download");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn movie_identity_rename_preserves_download_state() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-movie-rename-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute(
                "INSERT INTO movies(name,year,title,quality_score,magnet_hash,magnet_link,downloaded_at,size_bytes) VALUES ('Titolo Vecchio',2020,'Titolo.Vecchio.2020',1234,'hash1','magnet:?xt=urn:btih:hash1',datetime('now'),999)",
                [],
            )
            .unwrap();
        let updated = db
            .rename_movie_identity("Titolo Vecchio", "2020", "Titolo Nuovo", "2021")
            .unwrap();
        assert_eq!(updated, 1);
        let (name, year, score, size, downloaded): (String, i64, i64, i64, Option<String>) = db
            .conn
            .query_row(
                "SELECT name,year,quality_score,size_bytes,downloaded_at FROM movies WHERE magnet_hash='hash1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(name, "Titolo Nuovo");
        assert_eq!(year, 2021);
        // Lo stato di download resta intatto.
        assert_eq!(score, 1234);
        assert_eq!(size, 999);
        assert!(downloaded.is_some());
        // Un anno diverso non intacca righe non corrispondenti.
        assert_eq!(
            db.rename_movie_identity("Titolo Nuovo", "1900", "X", "1901")
                .unwrap(),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn episode_air_dates_persist_for_unmaterialized_episodes() {
        let path = std::env::temp_dir().join(format!(
            "rextto-db-air-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(id,name) VALUES (1,'Show')", [])
            .unwrap();
        db.save_series_metadata("Show", &[(1, 2)]).unwrap();
        // Solo l'episodio 1 esiste davvero; il 2 è atteso dai metadati TMDB.
        db.conn
            .execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score) VALUES (1,1,1,'Show.S01E01',900)",
                [],
            )
            .unwrap();
        db.save_episode_air_dates(
            "Show",
            &[
                (1, 1, "2024-01-01".to_string()),
                (1, 2, "2024-01-08".to_string()),
            ],
        )
        .unwrap();
        let items = db.episodes_for_series("Show", &[]).unwrap();
        assert_eq!(items.len(), 2);
        let by_episode = |episode: i64| {
            items
                .iter()
                .find(|item| item.episode == episode)
                .unwrap()
                .air_date
                .clone()
        };
        assert_eq!(by_episode(1), "2024-01-01");
        // Anche l'episodio atteso, mai materializzato, mostra la data.
        assert_eq!(by_episode(2), "2024-01-08");
        assert_eq!(db.episode_air_dates().unwrap().len(), 2);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn housekeeping_trims_bounded_tables() {
        let path = std::env::temp_dir().join(format!(
            "rextto-housekeeping-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        for index in 0..5 {
            db.conn
                .execute(
                    "INSERT INTO cycle_history(at,payload_json) VALUES (?1,'{}')",
                    [format!("2020-01-0{}T00:00:00+00:00", index + 1)],
                )
                .unwrap();
        }
        db.conn
            .execute(
                "INSERT INTO gap_search_log(series_name,season,episode,last_searched_at) VALUES ('A',1,1,'2000-01-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO gap_search_log(series_name,season,episode,last_searched_at) VALUES ('A',1,2,?1)",
                [Utc::now().to_rfc3339()],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO upgrade_backup(new_hash,payload_json,created_at) VALUES ('h1','{}','2000-01-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
        let report = db
            .housekeeping(&HousekeepingParams {
                retain_cycles: 1,
                error_age_days: 7,
                seen_days: 0,
                gap_log_days: 30,
                upgrade_backup_days: 30,
                history_days: 0,
            })
            .unwrap();
        assert_eq!(report.old_cycles_removed, 4);
        assert_eq!(report.gap_logs_removed, 1);
        assert_eq!(report.upgrade_backups_removed, 1);
        let remaining: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM gap_search_log", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn media_info_roundtrips_for_episodes_and_movies() {
        let path = std::env::temp_dir().join(format!(
            "rextto-media-info-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        let info = crate::mediainfo::MediaInfo {
            video_codec: "hevc".into(),
            bit_depth: 10,
            hdr: "HDR10".into(),
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        db.conn
            .execute("INSERT INTO series(name) VALUES ('Show')", [])
            .unwrap();
        let series_id: i64 = db
            .conn
            .query_row("SELECT id FROM series WHERE name='Show'", [], |row| row.get(0))
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO episodes(series_id,season,episode,title) VALUES (?1,1,2,'Show.S01E02')",
                [series_id],
            )
            .unwrap();
        let mut release = release();
        release.kind = "series".into();
        release.series = Some("Show".into());
        release.season = Some(1);
        release.episode = Some(2);
        db.set_media_info(&release, &info).unwrap();
        let stored = db.episode_media_info("Show", 1, 2).unwrap().unwrap();
        assert_eq!(stored["hdr"], "HDR10");
        assert_eq!(stored["bit_depth"], 10);

        // A movie matches by name/year.
        let mut movie = release.clone();
        movie.kind = "movie".into();
        movie.title = "The Film".into();
        movie.year = Some(2026);
        movie.series = None;
        movie.season = None;
        movie.episode = None;
        db.conn
            .execute(
                "INSERT INTO movies(name,year,title) VALUES ('The Film',2026,'The Film')",
                [],
            )
            .unwrap();
        db.set_media_info(&movie, &info).unwrap();
        let stored = db.movie_media_info("The Film", Some(2026)).unwrap().unwrap();
        assert_eq!(stored["video_codec"], "hevc");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn delay_pending_waits_for_the_due_time() {
        let path = std::env::temp_dir().join(format!(
            "rextto-delay-pending-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();

        let mut series = release();
        series.kind = "series".into();
        series.series = Some("Show".into());
        series.season = Some(1);
        series.episode = Some(1);

        // Zero delay is immediately ready.
        db.queue_pending_scored(&series, 0, 100).unwrap();
        assert_eq!(db.ready_pending().unwrap().len(), 1);
        db.remove_pending("Show", 1, 1).unwrap();
        assert!(db.ready_pending().unwrap().is_empty());

        // A 60-minute delay is not ready yet, and a better candidate replaces
        // the stored magnet/score while waiting.
        db.queue_pending_scored(&series, 60, 100).unwrap();
        assert!(db.ready_pending().unwrap().is_empty());
        let mut better = series.clone();
        better.title = "Show.S01E01.better".into();
        better.magnet = "magnet:?xt=urn:btih:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        db.queue_pending_scored(&better, 60, 500).unwrap();
        let score: i64 = db
            .conn
            .query_row(
                "SELECT best_quality_score FROM pending_downloads WHERE season=1 AND episode=1 AND status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(score, 500);

        // Movies use their own table.
        let mut movie = release();
        movie.kind = "movie".into();
        movie.title = "The Film".into();
        movie.year = Some(2026);
        db.queue_pending_movie_scored(&movie, 0, 200).unwrap();
        assert_eq!(db.ready_pending_movies().unwrap().len(), 1);
        db.remove_pending_movie("The Film", Some(2026)).unwrap();
        assert!(db.ready_pending_movies().unwrap().is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn later_archived_rank_reports_the_best_following_episode() {
        let path = std::env::temp_dir().join(format!(
            "rextto-later-rank-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(name) VALUES ('Show')", [])
            .unwrap();
        let series_id: i64 = db
            .conn
            .query_row("SELECT id FROM series WHERE name='Show'", [], |row| row.get(0))
            .unwrap();
        for (episode, title) in [
            (5, "Show.S01E05.1080p.WEB-DL"),
            (6, "Show.S01E06.720p.WEB-DL"),
        ] {
            db.conn
                .execute(
                    "INSERT INTO episodes(series_id,season,episode,title,downloaded_at,archive_path) \
                     VALUES (?1,1,?2,?3,'2024-01-01T00:00:00+00:00','/nas/f.mkv')",
                    params![series_id, episode, title],
                )
                .unwrap();
        }
        // Best later episode for E01 is E05 (1080p, rank 5).
        assert_eq!(
            db.later_archived_max_resolution_rank("Show", 1, 1).unwrap(),
            Some(5)
        );
        // For E05 the only later episode is E06 (720p, rank 4).
        assert_eq!(
            db.later_archived_max_resolution_rank("Show", 1, 5).unwrap(),
            Some(4)
        );
        // Nothing after E06.
        assert_eq!(db.later_archived_max_resolution_rank("Show", 1, 6).unwrap(), None);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn disable_upgrades_blocks_replacement() {
        let path = std::env::temp_dir().join(format!(
            "rextto-disable-upgrades-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(name) VALUES ('Show')", [])
            .unwrap();
        let series_id: i64 = db
            .conn
            .query_row("SELECT id FROM series WHERE name='Show'", [], |row| row.get(0))
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score,downloaded_at,archive_path) \
                 VALUES (?1,1,1,'Show.S01E01.1080p',1000,'2024-01-01T00:00:00+00:00','/nas/e1.mkv')",
                [series_id],
            )
            .unwrap();

        let mut candidate = release();
        candidate.kind = "series".into();
        candidate.series = Some("Show".into());
        candidate.season = Some(1);
        candidate.episode = Some(1);
        candidate.is_pack = false;
        candidate.quality = Quality {
            resolution: "2160p".into(),
            source: "webdl".into(),
            ..Default::default()
        };
        let score = candidate.quality.score();

        let mut index = crate::models::ArchiveQualityIndex::default();
        index.best.insert(
            (1, 1),
            (
                Quality {
                    resolution: "1080p".into(),
                    ..Default::default()
                },
                1000,
            ),
        );
        // With upgrades enabled the 2160p release is a real upgrade.
        let allowed_context = crate::models::ApprovalContext {
            archive: index.clone(),
            live: Default::default(),
            forbid_upgrade: false, gap_episode: false,
        };
        let (approved, reason) = db
            .check_series_scored(&candidate, score, 200, &allowed_context)
            .unwrap();
        assert!(approved, "expected upgrade, got {reason}");

        // Reset the row to its archived 1080p state, then forbid upgrades.
        db.conn
            .execute(
                "UPDATE episodes SET title='Show.S01E01.1080p', quality_score=1000, magnet_hash=NULL, downloaded_at='2024-01-01T00:00:00+00:00', archive_path='/nas/e1.mkv' WHERE series_id=?1 AND season=1 AND episode=1",
                [series_id],
            )
            .unwrap();
        let cutoff_context = crate::models::ApprovalContext {
            archive: index,
            live: Default::default(),
            forbid_upgrade: true, gap_episode: false,
        };
        let (approved, reason) = db
            .check_series_scored(&candidate, score, 200, &cutoff_context)
            .unwrap();
        assert!(!approved);
        assert_eq!(reason, "upgrades_disabled");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn smart_episode_is_always_on_but_exempts_gaps_and_upgrades() {
        let path = std::env::temp_dir().join(format!(
            "rextto-smart-episode-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        db.conn
            .execute("INSERT INTO series(name) VALUES ('Show')", [])
            .unwrap();
        let series_id: i64 = db
            .conn
            .query_row("SELECT id FROM series WHERE name='Show'", [], |row| row.get(0))
            .unwrap();
        // A later episode is already archived on the NAS.
        db.conn
            .execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score,downloaded_at,archive_path) \
                 VALUES (?1,1,5,'Show.S01E05.1080p',1000,'2024-01-01T00:00:00+00:00','/nas/e5.mkv')",
                [series_id],
            )
            .unwrap();
        let clear_episode = || {
            db.conn
                .execute(
                    "DELETE FROM episodes WHERE series_id=?1 AND season=1 AND episode=1",
                    [series_id],
                )
                .unwrap();
        };

        let mut candidate = release();
        candidate.kind = "series".into();
        candidate.series = Some("Show".into());
        candidate.season = Some(1);
        candidate.episode = Some(1);
        candidate.is_pack = false;
        candidate.quality = Quality {
            resolution: "1080p".into(),
            source: "webdl".into(),
            ..Default::default()
        };
        let score = candidate.quality.score();

        // Always on: same-quality older episode is refused with no option set.
        let (approved, reason) = db
            .check_series_scored(&candidate, score, 200, &crate::models::ApprovalContext::default())
            .unwrap();
        assert!(!approved);
        assert_eq!(reason, "smart_episode");

        // Gap-fill is exempt: the same candidate is accepted.
        let gap_context = crate::models::ApprovalContext {
            gap_episode: true,
            ..Default::default()
        };
        let (approved, reason) = db
            .check_series_scored(&candidate, score, 200, &gap_context)
            .unwrap();
        assert!(approved, "expected gap approval, got {reason}");
        clear_episode();

        // A genuine quality upgrade over the later episode is accepted.
        candidate.quality = Quality {
            resolution: "2160p".into(),
            source: "webdl".into(),
            ..Default::default()
        };
        let (approved, reason) = db
            .check_series_scored(
                &candidate,
                candidate.quality.score(),
                200,
                &crate::models::ApprovalContext::default(),
            )
            .unwrap();
        assert!(approved, "expected upgrade approval, got {reason}");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn provider_backoff_escalates_and_recovers() {
        let path = std::env::temp_dir().join(format!(
            "rextto-provider-status-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).unwrap();
        assert!(!db.provider_blocked("indexer", "Prowlarr").unwrap());
        assert!(db.blocked_providers().unwrap().is_empty());

        // First failure disables the provider.
        db.provider_failure("indexer", "Prowlarr", "HTTP 503")
            .unwrap();
        assert!(db.provider_blocked("indexer", "Prowlarr").unwrap());
        assert!(db
            .blocked_providers()
            .unwrap()
            .contains(&("indexer".to_string(), "Prowlarr".to_string())));
        let statuses = db.provider_statuses().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].level, 1);
        assert_eq!(statuses[0].last_error, "HTTP 503");

        // A success steps the level down and clears the block.
        db.provider_success("indexer", "Prowlarr").unwrap();
        assert!(!db.provider_blocked("indexer", "Prowlarr").unwrap());
        assert!(db.provider_statuses().unwrap().is_empty());

        // Manual reset clears everything.
        db.provider_failure("feed", "https://example.test/rss", "timeout")
            .unwrap();
        db.clear_provider_status(None).unwrap();
        assert!(db.provider_statuses().unwrap().is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
