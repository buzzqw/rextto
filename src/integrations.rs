use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct TraktClient {
    client: Client,
    client_id: String,
    client_secret: String,
    access_token: String,
    refresh_token: String,
}

impl TraktClient {
    pub fn from_settings(settings: &BTreeMap<String, String>) -> Self {
        Self {
            client: Client::new(),
            client_id: settings.get("trakt_client_id").cloned().unwrap_or_default(),
            client_secret: settings
                .get("trakt_client_secret")
                .cloned()
                .unwrap_or_default(),
            access_token: settings
                .get("trakt_access_token")
                .cloned()
                .unwrap_or_default(),
            refresh_token: settings
                .get("trakt_refresh_token")
                .cloned()
                .unwrap_or_default(),
        }
    }
    fn headers(&self, auth: bool) -> Result<reqwest::header::HeaderMap> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("trakt-api-key", self.client_id.parse()?);
        headers.insert("trakt-api-version", "2".parse()?);
        if auth && !self.access_token.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token).parse()?,
            );
        }
        Ok(headers)
    }
    pub async fn device_start(&self) -> Result<Value> {
        Ok(self
            .client
            .post("https://api.trakt.tv/oauth/device/code")
            .headers(self.headers(false)?)
            .json(&serde_json::json!({"client_id":self.client_id}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn device_poll(&self, code: &str) -> Result<Value> {
        Ok(self.client.post("https://api.trakt.tv/oauth/device/token").headers(self.headers(false)?).json(&serde_json::json!({"code":code,"client_id":self.client_id,"client_secret":self.client_secret})).send().await?.error_for_status()?.json().await?)
    }
    pub async fn refresh(&self) -> Result<Value> {
        Ok(self.client.post("https://api.trakt.tv/oauth/token").headers(self.headers(false)?).json(&serde_json::json!({"refresh_token":self.refresh_token,"client_id":self.client_id,"client_secret":self.client_secret,"grant_type":"refresh_token"})).send().await?.error_for_status()?.json().await?)
    }
    pub async fn watchlist(&self) -> Result<Value> {
        self.get("/sync/watchlist/shows").await
    }
    pub async fn calendar(&self, days: i64) -> Result<Value> {
        self.get(&format!(
            "/calendars/my/shows/{}/{}",
            chrono::Utc::now().format("%Y-%m-%d"),
            days.clamp(1, 31)
        ))
        .await
    }
    pub async fn scrobble(&self, action: &str, payload: Value) -> Result<Value> {
        let action = match action {
            "start" | "pause" | "stop" => action,
            _ => anyhow::bail!("invalid Trakt scrobble action"),
        };
        Ok(self
            .client
            .post(format!("https://api.trakt.tv/scrobble/{action}"))
            .headers(self.headers(true)?)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .client
            .get(format!("https://api.trakt.tv{path}"))
            .headers(self.headers(true)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub fn configured(&self) -> bool {
        !self.client_id.is_empty()
    }
    pub fn authenticated(&self) -> bool {
        !self.access_token.is_empty()
    }
}

#[derive(Clone)]
pub struct SimklClient {
    client: Client,
    client_id: String,
    access_token: String,
}

impl SimklClient {
    pub fn from_settings(settings: &BTreeMap<String, String>) -> Self {
        Self {
            client: Client::new(),
            client_id: settings.get("simkl_client_id").cloned().unwrap_or_default(),
            access_token: settings
                .get("simkl_access_token")
                .cloned()
                .unwrap_or_default(),
        }
    }
    fn params(&self) -> Vec<(&str, String)> {
        vec![
            ("client_id", self.client_id.clone()),
            ("app-name", "rextto".into()),
            ("app-version", "1".into()),
        ]
    }
    fn headers(&self, auth: bool) -> Result<reqwest::header::HeaderMap> {
        let mut headers = reqwest::header::HeaderMap::new();
        if auth && !self.access_token.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token).parse()?,
            );
        }
        Ok(headers)
    }
    pub async fn pin_start(&self) -> Result<Value> {
        Ok(self
            .client
            .get("https://api.simkl.com/oauth/pin")
            .query(&self.params())
            .headers(self.headers(false)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn pin_poll(&self, code: &str) -> Result<Value> {
        Ok(self
            .client
            .get(format!("https://api.simkl.com/oauth/pin/{code}"))
            .query(&self.params())
            .headers(self.headers(false)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn watchlist(&self) -> Result<Value> {
        self.get("/sync/all-items").await
    }
    pub async fn calendar(&self) -> Result<Value> {
        Ok(self
            .client
            .get("https://data.simkl.in/calendars")
            .query(&self.params())
            .headers(self.headers(true)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn mark_watched(&self, payload: Value) -> Result<Value> {
        Ok(self
            .client
            .post("https://api.simkl.com/sync/history")
            .query(&self.params())
            .headers(self.headers(true)?)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .client
            .get(format!("https://api.simkl.com{path}"))
            .query(&self.params())
            .headers(self.headers(true)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub fn configured(&self) -> bool {
        !self.client_id.is_empty()
    }
    pub fn authenticated(&self) -> bool {
        !self.access_token.is_empty()
    }
}

pub fn token_string(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .context(format!("integration response missing {key}"))
}
