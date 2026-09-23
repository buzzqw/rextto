//! Minimal TVDB (TheTVDB) v4 client: login, series search and extended data.
//! The API key lives in the Rextto settings (`tvdb_api_key`); the login token
//! is cached in memory for ~23 hours.

use anyhow::{bail, Result};
use reqwest::Client;
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const API: &str = "https://api4.thetvdb.com/v4";

#[derive(Clone)]
pub struct TvdbClient {
    client: Client,
    key: Option<String>,
    language: String,
    token: Arc<Mutex<Option<(String, Instant)>>>,
}

impl TvdbClient {
    pub fn new(key: Option<String>) -> Self {
        Self::with_language(key, "ita".to_string())
    }

    pub fn with_language(key: Option<String>, language: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            key,
            language: if language.trim().is_empty() {
                "ita".to_string()
            } else {
                language
            },
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn configured(&self) -> bool {
        self.key
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
    }

    async fn token(&self) -> Result<String> {
        let key = self
            .key
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("TVDB API key non configurata"))?;
        {
            let cache = self.token.lock().unwrap();
            if let Some((token, expiry)) = cache.as_ref() {
                if Instant::now() < *expiry {
                    return Ok(token.clone());
                }
            }
        }
        let response = self
            .client
            .post(format!("{API}/login"))
            .json(&serde_json::json!({ "apikey": key }))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB login HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        let token = value
            .get("data")
            .and_then(|data| data.get("token"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("TVDB login senza token"))?
            .to_string();
        let expiry = Instant::now() + Duration::from_secs(23 * 3600);
        *self.token.lock().unwrap() = Some((token.clone(), expiry));
        Ok(token)
    }

    /// Searches series by name. Returns compact results ready for the UI.
    pub async fn search_series(&self, query: &str) -> Result<Vec<Value>> {
        let token = self.token().await?;
        let response = self
            .client
            .get(format!("{API}/search"))
            .query(&[("query", query), ("type", "series")])
            .header("Accept-Language", &self.language)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB search HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        let items = value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(items
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "tvdb_id": item.get("tvdb_id").or_else(|| item.get("objectID")),
                    "name": item.get("name"),
                    "year": item.get("year"),
                    "image": item.get("image_url").or_else(|| item.get("thumbnail")),
                    "overview": item.get("overview"),
                })
            })
            .collect())
    }

    /// Ricerca film TVDB, normalizzata nello stesso formato compatto usato per
    /// le serie. È usata soltanto dalla scelta esplicita dei metadati film.
    pub async fn search_movies(&self, query: &str) -> Result<Vec<Value>> {
        let token = self.token().await?;
        let response = self
            .client
            .get(format!("{API}/search"))
            .query(&[("query", query), ("type", "movie")])
            .header("Accept-Language", &self.language)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB movie search HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        Ok(value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "id": item.get("tvdb_id").or_else(|| item.get("movie_id")).or_else(|| item.get("objectID")),
                    "title": item.get("name"),
                    "original_title": item.get("originalName"),
                    "release_date": item.get("year"),
                    "overview": item.get("overview"),
                    "poster_path": item.get("image_url").or_else(|| item.get("thumbnail")),
                })
            })
            .collect())
    }

    /// Dati editoriali di un film TVDB selezionato. Il payload resta JSON per
    /// tollerare i campi opzionali/variabili dell'API v4.
    pub async fn movie_details(&self, id: &str) -> Result<Value> {
        let id = id
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("invalid TVDB movie id"))?;
        let token = self.token().await?;
        let response = self
            .client
            .get(format!("{API}/movies/{id}/extended"))
            .header("Accept-Language", &self.language)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB movie details HTTP {}", response.status());
        }
        Ok(response
            .json::<Value>()
            .await?
            .get("data")
            .cloned()
            .unwrap_or_default())
    }

    /// Main characters/actors as `{name, tvdb_id}` (TVDB people id).
    pub async fn series_characters(&self, id: i64) -> Result<Vec<Value>> {
        let token = self.token().await?;
        let response = self
            .client
            .get(format!("{API}/series/{id}/extended"))
            .header("Accept-Language", &self.language)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB series HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        let characters = value
            .get("data")
            .and_then(|data| data.get("characters"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for character in characters.iter().take(10) {
            let name = character
                .get("personName")
                .and_then(Value::as_str)
                .or_else(|| character.get("name").and_then(Value::as_str));
            let person = character.get("peopleId").and_then(Value::as_i64);
            if let (Some(name), Some(person)) = (name, person) {
                out.push(serde_json::json!({"name": name, "tvdb_id": person}));
            }
        }
        Ok(out)
    }

    /// Extended series data (name, year, image, seasons summary, ...).
    pub async fn series_extended(&self, id: i64) -> Result<Value> {
        let token = self.token().await?;
        let response = self
            .client
            .get(format!("{API}/series/{id}/extended"))
            .header("Accept-Language", &self.language)
            .bearer_auth(token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("TVDB series HTTP {}", response.status());
        }
        let value: Value = response.json().await?;
        Ok(value.get("data").cloned().unwrap_or(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_configuration_state() {
        assert!(!TvdbClient::new(None).configured());
        assert!(!TvdbClient::new(Some("   ".into())).configured());
        assert!(TvdbClient::new(Some("abc".into())).configured());
    }
}
