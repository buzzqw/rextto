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
        // Wait for concurrent writers instead of failing with SQLITE_BUSY.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
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
        Ok(Self { conn })
    }
    pub fn save_batch(&self, releases: &[Release]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for r in releases {
            tx.execute("INSERT OR IGNORE INTO archive(title,magnet,magnet_hash,source,quality_score,added_at) VALUES (?1,?2,?3,?4,?5,datetime('now'))", params![r.title, r.magnet, magnet_hash(&r.magnet), r.source, r.quality.score()])?;
        }
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
        let term = fts_query(query);
        // See `browse`: a filtered and an unfiltered query, never MATCH inside OR.
        let total: i64 = if term.is_empty() {
            self.conn
                .query_row("SELECT COUNT(*) FROM archive", [], |row| row.get(0))?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*) FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1",
                [&term],
                |row| row.get(0),
            )?
        };
        let pages = ((total as usize + limit - 1) / limit).max(1);
        let offset = ((page - 1).min(pages - 1) * limit) as i64;
        let items = if term.is_empty() {
            let mut statement = self.conn.prepare("SELECT id,title,magnet,COALESCE(source,'archive'),COALESCE(quality_score,0),added_at FROM archive ORDER BY added_at DESC LIMIT ?1 OFFSET ?2")?;
            let rows = statement.query_map(params![limit as i64, offset], archive_entry_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut statement = self.conn.prepare("SELECT archive.id,archive.title,archive.magnet,COALESCE(archive.source,'archive'),COALESCE(archive.quality_score,0),archive.added_at FROM archive JOIN archive_fts ON archive_fts.rowid=archive.id WHERE archive_fts MATCH ?1 ORDER BY archive.added_at DESC LIMIT ?2 OFFSET ?3")?;
            let rows = statement.query_map(params![term, limit as i64, offset], archive_entry_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
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
    Ok(ArchiveEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        magnet: row.get(2)?,
        source: row.get(3)?,
        quality_score: row.get(4)?,
        added_at: row.get(5)?,
    })
}

/// Builds an FTS5 query where every whitespace-separated term is required.
/// Prefix matching keeps searches useful for partial words (e.g. `ocea`).
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
        let release = |index: usize| Release {
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
            discovered_at: Utc::now(),
        };
        let releases: Vec<Release> = (1..=5).map(release).collect();
        archive.save_batch(&releases).unwrap();
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
        let release = |magnet: &str, title: &str| Release {
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
        archive.save_batch(&[first.clone(), first, second]).unwrap();
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
}
