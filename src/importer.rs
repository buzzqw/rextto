use crate::{comics, utils::magnet_hash};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags, Row};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub snapshot_dir: String,
    pub config_copied: bool,
    pub config_settings: usize,
    pub config_movies: usize,
    pub config_torrent_limits: usize,
    pub series: usize,
    pub episodes: usize,
    pub movies: usize,
    pub archive: usize,
    pub torrent_meta: usize,
    pub torrent_state: usize,
    pub comics: comics::ComicsImportReport,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

fn snapshot_database(source: &Path, destination: &Path) -> Result<()> {
    let source_conn = readonly(source)?;
    let mut destination_conn = Connection::open(destination)
        .with_context(|| format!("create snapshot {}", destination.display()))?;
    let backup = rusqlite::backup::Backup::new(&source_conn, &mut destination_conn)?;
    backup.run_to_completion(128, Duration::from_millis(20), None)?;
    Ok(())
}

type Snapshots = (PathBuf, Option<PathBuf>, Option<PathBuf>, Option<PathBuf>);

fn make_snapshots(
    source_dir: &Path,
    _destination_dir: &Path,
) -> Result<Snapshots> {
    let snapshot_dir = std::env::temp_dir().join(format!("rextto-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&snapshot_dir)?;
    let series = source_dir.join("extto_series.db");
    anyhow::ensure!(
        series.exists(),
        "missing source database {}",
        series.display()
    );
    let series_snapshot = snapshot_dir.join("extto_series.db");
    snapshot_database(&series, &series_snapshot)?;
    let archive_snapshot = source_dir
        .join("extto_archive.db")
        .is_file()
        .then(|| snapshot_dir.join("extto_archive.db"));
    if let Some(path) = &archive_snapshot {
        snapshot_database(&source_dir.join("extto_archive.db"), path)?;
    }
    let comics_snapshot = source_dir
        .join("comics.db")
        .is_file()
        .then(|| snapshot_dir.join("comics.db"));
    if let Some(path) = &comics_snapshot {
        snapshot_database(&source_dir.join("comics.db"), path)?;
    }
    let config_snapshot = source_dir
        .join("extto_config.db")
        .is_file()
        .then(|| snapshot_dir.join("extto_config.db"));
    if let Some(path) = &config_snapshot {
        snapshot_database(&source_dir.join("extto_config.db"), path)?;
    }
    Ok((
        snapshot_dir,
        archive_snapshot,
        comics_snapshot,
        config_snapshot,
    ))
}

fn readonly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", path.display()))
}

fn import_torrent_state(source_dir: &Path, destination_dir: &Path, report: &mut ImportReport) {
    let candidates = [
        source_dir.join("extto_torrents_state"),
        source_dir.join("torrents_state"),
        source_dir
            .parent()
            .unwrap_or(source_dir)
            .join("extto_torrents_state"),
    ];
    let Some(source) = candidates.into_iter().find(|path| path.is_dir()) else {
        report
            .warnings
            .push("torrent state directory not found: active session not imported".into());
        return;
    };
    let target = destination_dir.join(crate::constants::DEFAULT_STATE_DIR);
    if let Err(error) = std::fs::create_dir_all(&target) {
        report.warnings.push(format!(
            "cannot create torrent state directory {}: {error}",
            target.display()
        ));
        return;
    }
    let mut copied = 0;
    for entry in std::fs::read_dir(&source).into_iter().flatten().flatten() {
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if extension != "fastresume" && extension != "torrent" {
            continue;
        }
        if let Some(name) = path.file_name() {
            if std::fs::copy(&path, target.join(name)).is_ok() {
                copied += 1;
            }
        }
    }
    report.torrent_state = copied;
    tracing::info!(source = %source.display(), copied, "imported libtorrent session state");
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )?)
}

fn table_columns(conn: &Connection, name: &str) -> Result<std::collections::HashSet<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn column_or(columns: &std::collections::HashSet<String>, name: &str, fallback: &str) -> String {
    if columns.contains(name) {
        name.into()
    } else {
        fallback.into()
    }
}

fn text(row: &Row<'_>, index: usize) -> String {
    row.get::<_, Option<String>>(index)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn number(row: &Row<'_>, index: usize) -> i64 {
    row.get::<_, Option<i64>>(index).ok().flatten().unwrap_or(0)
}

pub fn import_extto(source_dir: &Path, destination_dir: &Path) -> Result<ImportReport> {
    let source_root = std::fs::canonicalize(source_dir)
        .with_context(|| format!("resolve import source {}", source_dir.display()))?;
    std::fs::create_dir_all(destination_dir)?;
    let destination_root = std::fs::canonicalize(destination_dir)?;
    anyhow::ensure!(
        source_root != destination_root
            && !source_root.starts_with(&destination_root)
            && !destination_root.starts_with(&source_root),
        "import source and destination must be separate directories"
    );
    let (snapshot_dir, archive_snapshot, _comics_snapshot, config_snapshot) =
        make_snapshots(source_dir, destination_dir)?;
    let source_db = readonly(&snapshot_dir.join("extto_series.db"))?;
    let target_path = destination_dir.join("rextto_series.db");
    // Migrate the destination schema before opening the transaction used by the import.
    crate::database::Database::open(&target_path)?;
    let target = Connection::open(&target_path)?;
    target.pragma_update(None, "journal_mode", "WAL")?;
    let tx = target.unchecked_transaction()?;
    let mut report = ImportReport {
        snapshot_dir: snapshot_dir.display().to_string(),
        ..Default::default()
    };
    let mut series_ids = HashMap::new();

    if has_table(&source_db, "series")? {
        let mut stmt = source_db.prepare(
            "SELECT id,name,seasons,quality_requirement,language,enabled,archive_path,tmdb_id,aliases,timeframe,ignored_seasons,subtitle,season_subfolders,exclude FROM series",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                number(r, 0),
                text(r, 1),
                text(r, 2),
                text(r, 3),
                text(r, 4),
                number(r, 5),
                text(r, 6),
                text(r, 7),
                text(r, 8),
                number(r, 9),
                text(r, 10),
                text(r, 11),
                number(r, 12),
                text(r, 13),
            ))
        })?;
        for row in rows {
            let (
                old_id,
                name,
                seasons,
                quality,
                language,
                enabled,
                archive_path,
                tmdb_id,
                aliases,
                timeframe,
                ignored_seasons,
                subtitle,
                season_subfolders,
                exclude,
            ) = row?;
            if name.is_empty() {
                report.skipped += 1;
                continue;
            }
            let inserted = tx.execute(
                "INSERT INTO series(id,name,seasons,quality,language,enabled,archive_path,tmdb_id,aliases,timeframe,ignored_seasons,subtitle,season_subfolders,exclude) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(name) DO UPDATE SET seasons=excluded.seasons,quality=excluded.quality,language=excluded.language,enabled=excluded.enabled,archive_path=excluded.archive_path,tmdb_id=excluded.tmdb_id,aliases=excluded.aliases,timeframe=excluded.timeframe,ignored_seasons=excluded.ignored_seasons,subtitle=excluded.subtitle,season_subfolders=excluded.season_subfolders,exclude=excluded.exclude",
                rusqlite::params![old_id, name, seasons, quality, language, enabled, archive_path, tmdb_id, aliases, timeframe, ignored_seasons, subtitle, season_subfolders, exclude],
            )?;
            let new_id: i64 =
                tx.query_row("SELECT id FROM series WHERE name=?1", [&name], |r| r.get(0))?;
            series_ids.insert(old_id, new_id);
            if inserted > 0 {
                report.series += 1;
            }
        }
    }

    if has_table(&source_db, "episodes")? {
        let mut stmt = source_db.prepare(
            "SELECT series_id,season,episode,title,quality_score,is_repack,magnet_hash,magnet_link,downloaded_at,archive_path,size_bytes,original_title,rename_verified FROM episodes",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                number(r, 0),
                number(r, 1),
                number(r, 2),
                text(r, 3),
                number(r, 4),
                number(r, 5),
                text(r, 6),
                text(r, 7),
                text(r, 8),
                text(r, 9),
                number(r, 10),
                text(r, 11),
                number(r, 12),
            ))
        })?;
        for row in rows {
            let (
                old_sid,
                season,
                episode,
                title,
                score,
                repack,
                hash,
                magnet,
                downloaded,
                archive_path,
                size,
                original,
                verified,
            ) = row?;
            let Some(sid) = series_ids.get(&old_sid) else {
                report.skipped += 1;
                continue;
            };
            let hash = if hash.is_empty() {
                magnet_hash(&magnet)
            } else {
                Some(hash)
            };
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO episodes(series_id,season,episode,title,quality_score,is_repack,magnet_hash,magnet_link,downloaded_at,archive_path,size_bytes,original_title,rename_verified) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                rusqlite::params![sid, season, episode, title, score, repack, hash, magnet, downloaded, archive_path, size, original, verified],
            )?;
            if inserted > 0 {
                report.episodes += 1;
            } else {
                report.skipped += 1;
            }
        }
    }

    if has_table(&source_db, "movies")? {
        let mut stmt = source_db.prepare(
            "SELECT name,year,title,quality_score,magnet_hash,magnet_link,downloaded_at,size_bytes,removed_at FROM movies",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                text(r, 0),
                number(r, 1),
                text(r, 2),
                number(r, 3),
                text(r, 4),
                text(r, 5),
                text(r, 6),
                number(r, 7),
                text(r, 8),
            ))
        })?;
        for row in rows {
            let (name, year, title, score, hash, magnet, downloaded, size, removed) = row?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO movies(name,year,title,quality_score,magnet_hash,magnet_link,downloaded_at,size_bytes,removed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                rusqlite::params![name, year, title, score, if hash.is_empty() { magnet_hash(&magnet) } else { Some(hash) }, magnet, downloaded, size, if removed.is_empty() { None::<String> } else { Some(removed) }],
            )?;
            if inserted > 0 {
                report.movies += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    if has_table(&source_db, "torrent_meta")? {
        let columns = table_columns(&source_db, "torrent_meta")?;
        let query = format!(
            "SELECT hash, {}, {}, {}, {}, {}, {}, {}, {} FROM torrent_meta",
            column_or(&columns, "tag", "''"),
            column_or(&columns, "dl_source", "''"),
            column_or(&columns, "ui_state", "''"),
            column_or(&columns, "progress", "0"),
            column_or(&columns, "paused", "0"),
            column_or(&columns, "total_size", "0"),
            column_or(&columns, "downloaded", "0"),
            column_or(&columns, "name", "''")
        );
        let mut statement = source_db.prepare(&query)?;
        let rows = statement.query_map([], |row| {
            Ok((
                text(row, 0),
                text(row, 1),
                text(row, 2),
                text(row, 3),
                row.get::<_, f64>(4).unwrap_or(0.0),
                number(row, 5),
                number(row, 6),
                number(row, 7),
                text(row, 8),
            ))
        })?;
        for row in rows {
            let (hash, tag, source, ui_state, progress, paused, total_size, downloaded, name) =
                row?;
            if hash.trim().is_empty() {
                continue;
            }
            let inserted = tx.execute(
                "INSERT INTO torrent_meta(hash,tag,source,ui_state,progress,paused,total_size,downloaded,name,status,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',?10) ON CONFLICT(hash) DO UPDATE SET tag=excluded.tag,source=excluded.source,ui_state=excluded.ui_state,progress=excluded.progress,paused=excluded.paused,total_size=excluded.total_size,downloaded=excluded.downloaded,name=excluded.name,updated_at=excluded.updated_at",
                rusqlite::params![hash.to_ascii_lowercase(), tag, source, ui_state, progress, paused, total_size, downloaded, name, Utc::now().to_rfc3339()],
            )?;
            if inserted > 0 {
                report.torrent_meta += 1;
            }
        }
    }
    tx.commit()?;
    drop(source_db);

    if let Some(source_archive) = archive_snapshot {
        let archive_db = readonly(&source_archive)?;
        let target_archive = Connection::open(destination_dir.join("rextto_archive.db"))?;
        target_archive.execute_batch("CREATE TABLE IF NOT EXISTS archive (id INTEGER PRIMARY KEY, title TEXT NOT NULL, magnet TEXT NOT NULL UNIQUE, magnet_hash TEXT, source TEXT, quality_score INTEGER, added_at TEXT NOT NULL);")?;
        if has_table(&archive_db, "archive")? {
            let tx = target_archive.unchecked_transaction()?;
            let mut stmt =
                archive_db.prepare("SELECT title,magnet,source,added_at FROM archive")?;
            let rows =
                stmt.query_map([], |r| Ok((text(r, 0), text(r, 1), text(r, 2), text(r, 3))))?;
            for row in rows {
                let (title, magnet, source, at) = row?;
                let hash = magnet_hash(&magnet).unwrap_or_default();
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO archive(title,magnet,magnet_hash,source,quality_score,added_at) VALUES (?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![title, magnet, hash, source, 0_i64, at],
                )?;
                if inserted > 0 {
                    report.archive += 1;
                } else {
                    report.skipped += 1;
                }
            }
            tx.commit()?;
        }
    }

    let comics_conn = Connection::open(destination_dir.join("rextto_comics.db"))?;
    report.comics = comics::import_from_extto(&snapshot_dir, &comics_conn)?;
    if let Some(config_snapshot) = config_snapshot {
        let config_conn = readonly(&config_snapshot)?;
        report.config_settings = config_conn
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .unwrap_or(0);
        report.config_movies = config_conn
            .query_row("SELECT COUNT(*) FROM movies_config", [], |row| row.get(0))
            .unwrap_or(0);
        report.config_torrent_limits = config_conn
            .query_row("SELECT COUNT(*) FROM torrent_limits", [], |row| row.get(0))
            .unwrap_or(0);
        let target_config = destination_dir.join("rextto_config.db");
        let target = Connection::open(&target_config)?;
        target.execute_batch("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS translations (lang TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(lang,key)); CREATE TABLE IF NOT EXISTS movies_config (id INTEGER PRIMARY KEY, name TEXT NOT NULL, year TEXT DEFAULT '', quality TEXT DEFAULT '', language TEXT DEFAULT '', enabled INTEGER DEFAULT 1, subtitle TEXT DEFAULT '', exclude TEXT DEFAULT '', language_requirements TEXT DEFAULT '', subtitle_requirements TEXT DEFAULT ''); CREATE TABLE IF NOT EXISTS torrent_limits (info_hash TEXT PRIMARY KEY, dl_bytes INTEGER NOT NULL DEFAULT -1, ul_bytes INTEGER NOT NULL DEFAULT -1, updated_at TEXT NOT NULL DEFAULT (datetime('now')));")?;
        let _ = target.execute(
            "ALTER TABLE movies_config ADD COLUMN language_requirements TEXT DEFAULT ''",
            [],
        );
        let _ = target.execute(
            "ALTER TABLE movies_config ADD COLUMN subtitle_requirements TEXT DEFAULT ''",
            [],
        );
        let existing_settings: i64 =
            target.query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))?;
        let existing_translations: i64 =
            target.query_row("SELECT COUNT(*) FROM translations", [], |row| row.get(0))?;
        let existing_movies: i64 =
            target.query_row("SELECT COUNT(*) FROM movies_config", [], |row| row.get(0))?;
        let fresh = existing_settings == 0 && existing_translations == 0 && existing_movies == 0;
        if fresh {
            if has_table(&config_conn, "settings")? {
                let mut source = config_conn.prepare("SELECT key,value FROM settings")?;
                let rows = source
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (key, value) in rows {
                    target.execute(
                        "INSERT OR REPLACE INTO settings(key,value) VALUES (?1,?2)",
                        rusqlite::params![key, value],
                    )?;
                }
            }
            if has_table(&config_conn, "translations")? {
                let mut source = config_conn.prepare("SELECT lang,key,value FROM translations")?;
                let rows = source
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (lang, key, value) in rows {
                    target.execute(
                        "INSERT OR REPLACE INTO translations(lang,key,value) VALUES (?1,?2,?3)",
                        rusqlite::params![lang, key, value],
                    )?;
                }
            }
            if has_table(&config_conn, "movies_config")? {
                let columns = table_columns(&config_conn, "movies_config")?;
                let query = format!(
                    "SELECT {},{},{},{},{},{},{},{},{},{} FROM movies_config",
                    column_or(&columns, "id", "0"),
                    column_or(&columns, "name", "''"),
                    column_or(&columns, "year", "''"),
                    column_or(&columns, "quality", "''"),
                    column_or(&columns, "language", "''"),
                    column_or(&columns, "enabled", "1"),
                    column_or(&columns, "subtitle", "''"),
                    column_or(&columns, "exclude", "''"),
                    column_or(&columns, "language_requirements", "''"),
                    column_or(&columns, "subtitle_requirements", "''")
                );
                let mut source = config_conn.prepare(&query)?;
                let rows = source
                    .query_map([], |row| {
                        Ok((
                            number(row, 0),
                            text(row, 1),
                            text(row, 2),
                            text(row, 3),
                            text(row, 4),
                            number(row, 5),
                            text(row, 6),
                            text(row, 7),
                            text(row, 8),
                            text(row, 9),
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (
                    id,
                    name,
                    year,
                    quality,
                    language,
                    enabled,
                    subtitle,
                    exclude,
                    language_requirements,
                    subtitle_requirements,
                ) in rows
                {
                    target.execute("INSERT OR REPLACE INTO movies_config(id,name,year,quality,language,enabled,subtitle,exclude,language_requirements,subtitle_requirements) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", rusqlite::params![id, name, year, quality, language, enabled, subtitle, exclude, language_requirements, subtitle_requirements])?;
                }
            }
            if has_table(&config_conn, "torrent_limits")? {
                let columns = table_columns(&config_conn, "torrent_limits")?;
                let query = format!(
                    "SELECT {},{},{} FROM torrent_limits",
                    column_or(&columns, "info_hash", "''"),
                    column_or(&columns, "dl_bytes", "-1"),
                    column_or(&columns, "ul_bytes", "-1")
                );
                let mut source = config_conn.prepare(&query)?;
                let rows = source
                    .query_map([], |row| Ok((text(row, 0), number(row, 1), number(row, 2))))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (hash, dl, ul) in rows {
                    if !hash.trim().is_empty() {
                        target.execute("INSERT OR REPLACE INTO torrent_limits(info_hash,dl_bytes,ul_bytes) VALUES (?1,?2,?3)", rusqlite::params![hash.to_ascii_lowercase(), dl, ul])?;
                    }
                }
            }
            report.config_copied = true;
        } else {
            report
                .warnings
                .push("rextto_config.db esiste gia': configurazione preservata".into());
        }
    }
    import_torrent_state(source_dir, destination_dir, &mut report);
    let _ = std::fs::remove_dir_all(&snapshot_dir);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_an_import_into_its_own_source_directory() {
        let root = std::env::temp_dir().join(format!("rextto-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let error = import_extto(&root, &root).unwrap_err();
        assert!(error.to_string().contains("separate directories"));
        let _ = std::fs::remove_dir_all(root);
    }
}
