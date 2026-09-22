use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct I18nDb {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Serialize)]
pub struct Translation {
    pub lang: String,
    pub key: String,
    pub value: String,
}

impl I18nDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS translations (lang TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(lang,key)); CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn storage_language(lang: &str) -> &str {
        match lang {
            "it" | "ita" => "ita",
            "en" | "eng" => "eng",
            "de" | "deu" => "deu",
            "fr" | "fra" => "fra",
            "es" | "spa" => "spa",
            _ => lang,
        }
    }
    fn public_language(lang: &str) -> String {
        match lang {
            "ita" => "it",
            "eng" => "en",
            "deu" => "de",
            "fra" => "fr",
            "spa" => "es",
            value => value,
        }
        .to_owned()
    }
    pub fn language(&self) -> Result<String> {
        Ok(Self::public_language(
            &self
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT value FROM settings WHERE key='ui_language'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_else(|_| "ita".into()),
        ))
    }
    pub fn set_language(&self, lang: &str) -> Result<()> {
        let lang = Self::storage_language(lang);
        self.conn.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES ('ui_language',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [lang])?;
        Ok(())
    }
    pub fn list(&self, lang: &str) -> Result<Vec<Translation>> {
        let storage = Self::storage_language(lang);
        let conn = self.conn.lock().unwrap();
        let mut statement =
            conn.prepare("SELECT lang,key,value FROM translations WHERE lang=?1 ORDER BY key")?;
        let rows = statement.query_map([storage], |row| {
            Ok(Translation {
                lang: Self::public_language(&row.get::<_, String>(0)?),
                key: row.get(1)?,
                value: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn set(&self, lang: &str, key: &str, value: &str) -> Result<()> {
        let lang = Self::storage_language(lang);
        self.conn.lock().unwrap().execute("INSERT INTO translations(lang,key,value) VALUES (?1,?2,?3) ON CONFLICT(lang,key) DO UPDATE SET value=excluded.value", params![lang, key, value])?;
        Ok(())
    }
    pub fn set_bulk(&self, lang: &str, values: &HashMap<String, String>) -> Result<()> {
        let lang = Self::storage_language(lang);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for (key, value) in values {
            tx.execute("INSERT INTO translations(lang,key,value) VALUES (?1,?2,?3) ON CONFLICT(lang,key) DO UPDATE SET value=excluded.value", params![lang, key, value])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct I18n {
    values: HashMap<String, String>,
}
impl I18n {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        };
        Ok(Self {
            values: serde_yaml::from_slice(&std::fs::read(path)?)?,
        })
    }
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.values.get(key).map(String::as_str).unwrap_or(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_language_and_translations_in_config_db() {
        let path = std::env::temp_dir().join(format!("rextto-i18n-{}.db", std::process::id()));
        let db = I18nDb::open(&path).unwrap();
        db.set_language("en").unwrap();
        db.set("en", "dashboard.title", "Dashboard").unwrap();
        assert_eq!(db.language().unwrap(), "en");
        assert_eq!(db.list("en").unwrap()[0].value, "Dashboard");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn accepts_extto_language_codes() {
        let path = std::env::temp_dir().join(format!("rextto-i18n-code-{}.db", std::process::id()));
        let db = I18nDb::open(&path).unwrap();
        db.set("eng", "Dashboard", "Dashboard").unwrap();
        db.set_language("eng").unwrap();
        assert_eq!(db.language().unwrap(), "en");
        assert_eq!(db.list("en").unwrap()[0].lang, "en");
        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}
