use crate::config::Config;
use crate::models::Release;
use crate::utils::magnet_hash;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Archive {
    conn: Connection,
}
#[derive(serde::Serialize)]
pub struct ArchiveEntry {
    pub id: i64,
    pub title: String,
    pub magnet: String,
    pub source: String,
    pub quality_score: i64,
    pub added_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<Release>,
}
#[derive(serde::Serialize)]
pub struct ArchivePage {
    pub items: Vec<ArchiveEntry>,
    pub total: i64,
    pub page: usize,
    pub pages: usize,
}
impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Wait for concurrent writers instead of failing with SQLITE_BUSY, plus
        // FULL synchronous and a bounded WAL for durability.
        crate::database::harden_connection(&conn)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS archive (id INTEGER PRIMARY KEY, title TEXT NOT NULL, magnet TEXT NOT NULL UNIQUE, magnet_hash TEXT, source TEXT, quality_score INTEGER, added_at TEXT NOT NULL); CREATE INDEX IF NOT EXISTS idx_archive_title ON archive(title); CREATE INDEX IF NOT EXISTS idx_archive_added ON archive(added_at DESC); CREATE VIRTUAL TABLE IF NOT EXISTS archive_fts USING fts5(title, content='archive', content_rowid='id'); CREATE TRIGGER IF NOT EXISTS archive_fts_ai AFTER INSERT ON archive BEGIN INSERT INTO archive_fts(rowid,title) VALUES (new.id,new.title); END; CREATE TRIGGER IF NOT EXISTS archive_fts_ad AFTER DELETE ON archive BEGIN INSERT INTO archive_fts(archive_fts,rowid,title) VALUES('delete',old.id,old.title); END; CREATE TRIGGER IF NOT EXISTS archive_fts_au AFTER UPDATE OF title ON archive BEGIN INSERT INTO archive_fts(archive_fts,rowid,title) VALUES('delete',old.id,old.title); INSERT INTO archive_fts(rowid,title) VALUES(new.id,new.title); END;")?;
        let archive_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM archive", [], |row| row.get(0))?;
        let indexed_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM archive_fts", [], |row| row.get(0))?;
        if archive_count != indexed_count {
            conn.execute(
                "INSERT INTO archive_fts(archive_fts) VALUES ('rebuild')",
                [],
            )?;
        }
        // Ricerca case-insensitive per hash (`lower(COALESCE(magnet_hash,''))`):
        // un indice di espressione evita la scansione completa.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_archive_magnet_lower ON archive(lower(COALESCE(magnet_hash,'')));",
        )?;
        Ok(Self { conn })
    }
    pub fn save_batch(&self, releases: &[Release], cfg: &Config) -> Result<()> {
        // Spezza il batch in transazioni più piccole: un'unica transazione da
        // migliaia di righe con l'indice FTS5 generava un WAL enorme (osservato
        // ~458 MB) e una finestra di scrittura molto lunga. Con chunk da 1000
        // righe il WAL resta nell'ordine di pochi MB e un crash recupera un
        // insieme di transazioni più piccolo (l'atomicità resta per chunk).
        for chunk in releases.chunks(1000) {
            let tx = self.conn.unchecked_transaction()?;
            for r in chunk {
                // RSS sources such as Jackett can expose only a `.torrent` URL.
                // Do not collapse all those releases into one empty unique magnet.
                let source = if r.magnet.trim().is_empty() {
                    r.torrent_url.as_deref().unwrap_or_default()
                } else {
                    &r.magnet
                };
                if source.trim().is_empty() {
                    continue;
                }
                tx.execute("INSERT OR IGNORE INTO archive(title,magnet,magnet_hash,source,quality_score,added_at) VALUES (?1,?2,?3,?4,?5,datetime('now'))", params![r.title, source, magnet_hash(source), r.source, cfg.release_score(r)])?;
            }
            tx.commit()?;
            // Trunca il WAL tra i chunk: la write amplification FTS5 non si
            // accumula fino al riavvio.
            let _ = self.checkpoint();
        }
        Ok(())
    }

    /// `PRAGMA quick_check` (vedi `crate::database::quick_check`).
    pub fn quick_check(&self) -> Result<Vec<String>> {
        crate::database::quick_check(&self.conn)
    }

    /// Removes every archived listing for a rejected infohash. Indexers can
    /// publish the same torrent under conflicting season/title metadata, so
    /// leaving the rows visible would make the bad release return on every
    /// archive pass even after it has been blocklisted.
    pub fn remove_hash(&self, hash: &str) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM archive WHERE lower(COALESCE(magnet_hash,''))=lower(?1)",
            [hash],
        )?;
        Ok(removed)
    }

    /// Replaces an ephemeral `.torrent` URL with its durable infohash magnet
    /// once the torrent has been downloaded successfully. If the same magnet
    /// is already archived, the obsolete URL row is simply removed.
    pub fn canonicalize_torrent_url(&self, torrent_url: &str, magnet: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM archive WHERE magnet=?1 AND EXISTS (SELECT 1 FROM archive WHERE magnet=?2)",
            params![torrent_url, magnet],
        )?;
        tx.execute(
            "UPDATE OR IGNORE archive SET magnet=?1, magnet_hash=?2 WHERE magnet=?3",
            params![magnet, magnet_hash(magnet), torrent_url],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn search(&self, query: &str) -> Result<Vec<(String, String, String)>> {
        let query = fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare("SELECT archive.title,archive.magnet,COALESCE(archive.source,'archive') FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1 LIMIT 200")?;
        let rows =
            statement.query_map([query], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.filter_map(Result::ok).collect())
    }
    /// Le `limit` voci più recenti (titolo, magnet, sorgente), usando l'indice
    /// su `added_at`: evita di caricare in memoria l'intero archivio (centinaia
    /// di migliaia di righe) ad ogni calcolo del feed status.
    pub fn recent_entries(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT title,magnet,COALESCE(source,'archive') FROM archive ORDER BY added_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 500_000) as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }
    pub fn optimize(&self, action: &str) -> Result<()> {
        crate::database::optimize_connection(&self.conn, action)
    }

    pub fn size_bytes(&self) -> i64 {
        crate::database::connection_size_bytes(&self.conn)
    }

    /// Truncates the WAL after a checkpoint (see
    /// `crate::database::checkpoint_connection`).
    pub fn checkpoint(&self) -> Result<()> {
        crate::database::checkpoint_connection(&self.conn)
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM archive", [], |r| r.get(0))?)
    }
    pub fn browse(&self, query: &str, limit: usize) -> Result<Vec<ArchiveEntry>> {
        // FTS5 `MATCH` cannot be used inside an `OR` expression (SQLite raises
        // "unable to use function MATCH in the requested context"), so the
        // filtered and unfiltered queries are kept separate.
        let term = fts_query(query);
        let limit = limit.clamp(1, 500) as i64;
        if term.is_empty() {
            let mut statement = self.conn.prepare("SELECT id,title,magnet,COALESCE(source,'archive'),COALESCE(quality_score,0),added_at FROM archive ORDER BY added_at DESC LIMIT ?1")?;
            let rows = statement.query_map([limit], archive_entry_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        } else {
            let mut statement = self.conn.prepare("SELECT archive.id,archive.title,archive.magnet,COALESCE(archive.source,'archive'),COALESCE(archive.quality_score,0),archive.added_at FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1 ORDER BY archive.added_at DESC LIMIT ?2")?;
            let rows = statement.query_map(params![term, limit], archive_entry_from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }
    }
    pub fn browse_page(&self, query: &str, page: usize, limit: usize) -> Result<ArchivePage> {
        let limit = limit.clamp(1, 500);
        let page = page.max(1);
        let (includes, excludes) = parse_archive_filter(query);
        let page_of = |total: i64| {
            let pages = (total as usize).div_ceil(limit).max(1);
            (pages, ((page - 1).min(pages - 1) * limit) as i64)
        };

        // Con termini positivi si usa l'indice FTS (con NOT per le esclusioni).
        if !includes.is_empty() {
            let term = fts_match_expression(&includes, &excludes);
            let total: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1",
                [&term],
                |row| row.get(0),
            )?;
            let (pages, offset) = page_of(total);
            let mut statement = self.conn.prepare("SELECT archive.id,archive.title,archive.magnet,COALESCE(archive.source,'archive'),COALESCE(archive.quality_score,0),archive.added_at FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1 ORDER BY archive.added_at DESC LIMIT ?2 OFFSET ?3")?;
            let items = statement
                .query_map(params![term, limit as i64, offset], archive_entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(ArchivePage {
                items,
                total,
                page,
                pages,
            });
        }

        // Solo esclusioni (o nessun filtro): FTS5 non può iniziare con NOT, si
        // filtra la tabella con NOT LIKE.
        let mut where_sql = String::new();
        let mut filter_binds: Vec<rusqlite::types::Value> = Vec::new();
        for word in &excludes {
            if where_sql.is_empty() {
                where_sql.push_str(" WHERE ");
            } else {
                where_sql.push_str(" AND ");
            }
            filter_binds.push(rusqlite::types::Value::Text(format!("%{word}%")));
            where_sql.push_str(&format!("lower(title) NOT LIKE ?{}", filter_binds.len()));
        }
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM archive{where_sql}"),
            rusqlite::params_from_iter(filter_binds.iter().cloned()),
            |row| row.get(0),
        )?;
        let (pages, offset) = page_of(total);
        filter_binds.push(rusqlite::types::Value::Integer(limit as i64));
        filter_binds.push(rusqlite::types::Value::Integer(offset));
        let count = filter_binds.len();
        let sql = format!(
            "SELECT id,title,magnet,COALESCE(source,'archive'),COALESCE(quality_score,0),added_at FROM archive{where_sql} ORDER BY added_at DESC LIMIT ?{} OFFSET ?{}",
            count - 1,
            count
        );
        let mut statement = self.conn.prepare(&sql)?;
        let items = statement
            .query_map(
                rusqlite::params_from_iter(filter_binds.iter().cloned()),
                archive_entry_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ArchivePage {
            items,
            total,
            page,
            pages,
        })
    }
    pub fn delete_ids(&self, ids: &[i64]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut removed = 0;
        for id in ids.iter().copied().filter(|id| *id > 0) {
            removed += tx.execute("DELETE FROM archive WHERE id=?1", [id])?;
        }
        tx.commit()?;
        Ok(removed)
    }
    pub fn delete(&self, magnet: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM archive WHERE magnet=?1 OR lower(magnet_hash)=lower(?2)",
            params![magnet, magnet_hash(magnet).unwrap_or_default()],
        )? > 0)
    }
    pub fn cleanup_older_than(&self, days: i64) -> Result<usize> {
        self.cleanup_older_than_keeping(days, 0)
    }

    /// Deletes archive rows older than `days` days, always keeping at least the
    /// `keep_min` most recent rows.
    pub fn cleanup_older_than_keeping(&self, days: i64, keep_min: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM archive WHERE added_at < datetime('now', ?1) AND id NOT IN (SELECT id FROM archive ORDER BY added_at DESC LIMIT ?2)",
            params![format!("-{} days", days.max(1)), keep_min.max(0)],
        )?)
    }
}

fn archive_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveEntry> {
    let title: String = row.get(1)?;
    let magnet: String = row.get(2)?;
    let source: String = row.get(3)?;
    Ok(ArchiveEntry {
        id: row.get(0)?,
        title: title.clone(),
        magnet: magnet.clone(),
        source: source.clone(),
        quality_score: row.get(4)?,
        added_at: row.get(5)?,
        release: crate::parser::parse_release(&title, &magnet, &format!("archive:{source}")),
    })
}

/// Builds an FTS5 query where every whitespace-separated term is required.
/// Prefix matching keeps searches useful for partial words (e.g. `ocea`).
/// Scompone il filtro dell'archivio in termini positivi e negativi: `-parola`
/// esclude, tutto il resto (anche con `+`) include. È un "regex facilitato"
/// senza vere espressioni regolari.
fn parse_archive_filter(query: &str) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for raw in query.split_whitespace() {
        if raw.is_empty() {
            continue;
        }
        if let Some(word) = raw.strip_prefix('-') {
            if !word.is_empty() {
                excludes.push(word.to_ascii_lowercase());
            }
        } else if let Some(word) = raw.strip_prefix('+') {
            if !word.is_empty() {
                includes.push(word.to_ascii_lowercase());
            }
        } else {
            includes.push(raw.to_ascii_lowercase());
        }
    }
    (includes, excludes)
}

/// Espressione FTS5: i termini positivi sono in AND, le esclusioni in NOT.
/// Richiede almeno un termine positivo (FTS5 non accetta un NOT iniziale).
fn fts_match_expression(includes: &[String], excludes: &[String]) -> String {
    let quote = |word: &str| format!("\"{}\"*", word.replace('"', "\"\""));
    let positive = includes
        .iter()
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" AND ");
    if excludes.is_empty() {
        return positive;
    }
    let negative = excludes
        .iter()
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" NOT ");
    format!("({positive}) NOT {negative}")
}

fn fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn recent_entries_respect_the_limit() {
        let path = std::env::temp_dir().join(format!("rextto-archive-{}", uuid::Uuid::new_v4()));
        let archive = Archive::open(&path).unwrap();
        let release = |index: usize| Release { torrent_url: None,
            title: format!("Release {index}"),
            magnet: format!("magnet:?xt=urn:btih:{:040x}", index),
            source: format!("source-{index}"),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2026),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let releases: Vec<Release> = (1..=5).map(release).collect();
        archive.save_batch(&releases, &Config::default()).unwrap();
        let recent = archive.recent_entries(3).unwrap();
        assert_eq!(recent.len(), 3);
        assert!(archive.recent_entries(100).unwrap().len() == 5);
        drop(archive);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn retains_distinct_hashes_and_deduplicates_the_same_magnet() {
        let path = std::env::temp_dir().join(format!("rextto-archive-{}", uuid::Uuid::new_v4()));
        let archive = Archive::open(&path).unwrap();
        let release = |magnet: &str, title: &str| Release { torrent_url: None,
            title: title.into(),
            magnet: magnet.into(),
            source: "test".into(),
            quality: Default::default(),
            kind: "movie".into(),
            series: None,
            season: None,
            episode: None,
            is_pack: false,
            episode_range: Vec::new(),
            year: Some(2026),
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        let first = release(
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "Example Movie 1080p",
        );
        let second = release(
            "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd",
            "Example Movie 2160p",
        );
        archive
            .save_batch(&[first.clone(), first, second], &Config::default())
            .unwrap();
        assert_eq!(archive.count().unwrap(), 2);
        assert_eq!(archive.search("Example Movie").unwrap().len(), 2);
        assert_eq!(
            archive.browse_page("Example 1080p", 1, 100).unwrap().total,
            1
        );
        assert_eq!(
            archive
                .browse_page("Example Missing", 1, 100)
                .unwrap()
                .total,
            0
        );
        drop(archive);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn stores_and_canonicalizes_torrent_urls() {
        let path = std::env::temp_dir().join(format!("rextto-archive-{}", uuid::Uuid::new_v4()));
        let archive = Archive::open(&path).unwrap();
        let torrent_url = "http://jackett:9117/dl/test/?path=ZXhhbXBsZQ";
        let release = Release {
            torrent_url: Some(torrent_url.into()),
            title: "Example.Show.S01E01.1080p".into(),
            magnet: String::new(),
            source: "Jackett RSS - Test".into(),
            quality: Default::default(),
            kind: "series".into(),
            series: Some("Example Show".into()),
            season: Some(1),
            episode: Some(1),
            is_pack: false,
            episode_range: vec![1],
            year: None,
            size_bytes: 0,
            seeders: -1,
            peers: -1,
            discovered_at: Utc::now(),
        };
        archive.save_batch(&[release], &Config::default()).unwrap();
        assert_eq!(archive.count().unwrap(), 1);
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        archive.canonicalize_torrent_url(torrent_url, magnet).unwrap();
        let entries = archive.search("Example Show").unwrap();
        assert_eq!(entries[0].1, magnet);
        drop(archive);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
