use crate::config::Config;
use crate::messages;
use anyhow::{anyhow, Result};
use hmac::{Hmac, Mac};
use lettre::{
    message::Mailbox,
    transport::smtp::{authentication::Credentials, AsyncSmtpTransport},
    AsyncTransport, Message, Tokio1Executor,
};
use reqwest::Client;
use serde::Serialize;
use sha2::Sha256;
use std::path::Path;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct Notifier {
    client: Client,
    telegram_enabled: bool,
    telegram_bot_token: Option<String>,
    telegram_chat_id: Option<String>,
    webhook_url: Option<String>,
    webhook_secret: Option<String>,
    email_enabled: bool,
    email_smtp: String,
    email_from: Option<String>,
    email_to: Option<String>,
    email_password: Option<String>,
    last_telegram: Arc<Mutex<Option<Instant>>>,
    /// External event hooks (see `crate::hooks`). Reloadable at runtime when
    /// the user edits them in the UI.
    hooks: Arc<std::sync::RwLock<Vec<crate::hooks::EventHook>>>,
}

#[derive(Serialize)]
struct TelegramMessage<'a> {
    chat_id: &'a str,
    text: &'a str,
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            telegram_enabled: false,
            telegram_bot_token: None,
            telegram_chat_id: None,
            webhook_url: None,
            webhook_secret: None,
            email_enabled: false,
            email_smtp: "smtp.gmail.com:587".into(),
            email_from: None,
            email_to: None,
            email_password: None,
            last_telegram: Arc::new(Mutex::new(None)),
            hooks: Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }

    pub fn from_config(cfg: &Config) -> Self {
        Self {
            client: Client::new(),
            telegram_enabled: cfg.notify_telegram,
            telegram_bot_token: cfg.telegram_bot_token.clone(),
            telegram_chat_id: cfg.telegram_chat_id.clone(),
            webhook_url: cfg.notify_webhook_url.clone(),
            webhook_secret: cfg.notify_webhook_secret.clone(),
            email_enabled: cfg.notify_email,
            email_smtp: cfg.email_smtp.clone(),
            email_from: cfg.email_from.clone(),
            email_to: cfg.email_to.clone(),
            email_password: cfg.email_password.clone(),
            last_telegram: Arc::new(Mutex::new(None)),
            hooks: Arc::new(std::sync::RwLock::new(crate::hooks::load_hooks(
                &cfg.settings,
            ))),
        }
    }

    /// Replaces the event hooks, e.g. after the settings API saved them.
    pub fn reload_hooks(&self, hooks: Vec<crate::hooks::EventHook>) {
        if let Ok(mut current) = self.hooks.write() {
            *current = hooks;
        }
    }

    pub fn event_hooks(&self) -> Vec<crate::hooks::EventHook> {
        self.hooks
            .read()
            .map(|hooks| hooks.clone())
            .unwrap_or_default()
    }

    pub fn enabled(&self) -> bool {
        (self.telegram_enabled
            && self.telegram_bot_token.is_some()
            && self.telegram_chat_id.is_some())
            || self.webhook_url.is_some()
            || (self.email_enabled
                && self.email_from.is_some()
                && self.email_to.is_some()
                && self.email_password.is_some())
    }

    pub async fn notify(&self, text: &str) -> Result<()> {
        self.notify_event("message", serde_json::json!({"text": text}))
            .await
    }

    pub async fn notify_comic_complete(
        &self,
        title: &str,
        path: &str,
        size_bytes: u64,
        method: &str,
    ) -> Result<()> {
        self.notify_event("comic_completed", serde_json::json!({"title": title, "path": path, "size_bytes": size_bytes, "method": method})).await
    }

    pub async fn notify_backup_document(&self, path: &Path, caption: &str) -> Result<bool> {
        let (Some(token), Some(chat)) = (&self.telegram_bot_token, &self.telegram_chat_id) else {
            return Ok(false);
        };
        if !self.telegram_enabled || tokio::fs::metadata(path).await?.len() > 49 * 1024 * 1024 {
            return Ok(false);
        }
        self.throttle_telegram().await;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("rextto-backup.zip")
            .to_owned();
        let document =
            reqwest::multipart::Part::bytes(tokio::fs::read(path).await?).file_name(filename);
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat.clone())
            .text("caption", caption.to_owned())
            .part("document", document);
        self.client
            .post(format!("https://api.telegram.org/bot{token}/sendDocument"))
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;
        Ok(true)
    }

    pub async fn notify_event(&self, event: &str, data: serde_json::Value) -> Result<()> {
        // External hooks run on a detached task so they never delay the
        // notification delivery or the caller.
        let hooks = self.event_hooks();
        if !hooks.is_empty() {
            crate::hooks::dispatch(hooks, event.to_string(), data.clone());
        }
        let mut first_error = None;
        if self.telegram_enabled {
            if let (Some(token), Some(chat)) = (&self.telegram_bot_token, &self.telegram_chat_id) {
                self.throttle_telegram().await;
                let mut result = self
                    .client
                    .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
                    .json(&TelegramMessage {
                        chat_id: chat,
                        text: &format_event(event, &data),
                    })
                    .send()
                    .await
                    .and_then(|response| response.error_for_status());
                for attempt in 0..2 {
                    if result.is_ok() {
                        break;
                    }
                    result = self
                        .client
                        .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
                        .json(&TelegramMessage {
                            chat_id: chat,
                            text: &format_event(event, &data),
                        })
                        .send()
                        .await
                        .and_then(|response| response.error_for_status());
                    if result.is_err() {
                        tokio::time::sleep(Duration::from_secs(1_u64 << attempt)).await;
                    }
                }
                if let Err(error) = result {
                    first_error = Some(anyhow!(error));
                }
            }
        }
        if let Some(url) = &self.webhook_url {
            let body = serde_json::to_vec(&serde_json::json!({"event": event, "data": data}))?;
            let signature = if let Some(secret) = &self.webhook_secret {
                let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
                    .map_err(|_| anyhow!("invalid webhook secret"))?;
                mac.update(&body);
                Some(format!(
                    "sha256={}",
                    hex_bytes(&mac.finalize().into_bytes())
                ))
            } else {
                None
            };
            let mut request = self
                .client
                .post(url)
                .header("content-type", "application/json")
                .body(body.clone());
            if let Some(signature) = &signature {
                request = request.header("x-rextto-signature", signature);
            }
            let mut result = request
                .send()
                .await
                .and_then(|response| response.error_for_status());
            for attempt in 0..2 {
                if result.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1_u64 << attempt)).await;
                let mut retry = self
                    .client
                    .post(url)
                    .header("content-type", "application/json")
                    .body(body.clone());
                if let Some(signature) = &signature {
                    retry = retry.header("x-rextto-signature", signature);
                }
                result = retry
                    .send()
                    .await
                    .and_then(|response| response.error_for_status());
            }
            if let Err(error) = result {
                if first_error.is_none() {
                    first_error = Some(anyhow!(error));
                }
            }
        }
        if self.email_enabled {
            if let Err(error) = self.send_email(event, &format_event(event, &data)).await {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    async fn throttle_telegram(&self) {
        let mut last = self.last_telegram.lock().await;
        if let Some(previous) = *last {
            let elapsed = previous.elapsed();
            if elapsed < Duration::from_secs(1) {
                tokio::time::sleep(Duration::from_secs(1) - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn send_email(&self, event: &str, body: &str) -> Result<()> {
        let (Some(from), Some(to), Some(password)) =
            (&self.email_from, &self.email_to, &self.email_password)
        else {
            return Ok(());
        };
        let (host, port) = self
            .email_smtp
            .rsplit_once(':')
            .map(|(host, port)| (host, port.parse::<u16>().unwrap_or(587)))
            .unwrap_or((self.email_smtp.as_str(), 587));
        let mut message = Message::builder()
            .from(from.parse::<Mailbox>()?)
            .subject(format!("Rextto [{event}]"));
        for recipient in to
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            message = message.to(recipient.parse::<Mailbox>()?);
        }
        let message = message.body(body.to_owned())?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(host)?
            .port(port)
            .credentials(Credentials::new(from.clone(), password.clone()))
            .build();
        transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|error| anyhow!(error))
    }
}

fn format_event(event: &str, data: &serde_json::Value) -> String {
    let text = |key: &str| {
        data.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let value_text = |value: &serde_json::Value, key: &str, fallback: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    let series_episode = || {
        let series = text("series");
        let season = data.get("season").and_then(serde_json::Value::as_i64);
        let episode = data.get("episode").and_then(serde_json::Value::as_i64);
        match (series.is_empty(), season, episode) {
            (false, Some(season), Some(episode)) => format!("{series} S{season:02}E{episode:02}"),
            (false, _, _) => series,
            _ => {
                let title = text("title");
                if title.is_empty() {
                    text("name")
                } else {
                    title
                }
            }
        }
    };
    let episode_code = || match (
        data.get("season").and_then(serde_json::Value::as_i64),
        data.get("episode").and_then(serde_json::Value::as_i64),
    ) {
        (Some(season), Some(episode)) => format!("S{season:02}E{episode:02}"),
        _ => "-".into(),
    };
    let reason_code = text("reason");
    let reason = match reason_code.as_str() {
        "upgrade" => messages::pick("⬆️ Qualità superiore trovata", "⬆️ Better quality found"),
        "gap_filled" => messages::pick("🔎 Episodio mancante trovato", "🔎 Missing episode found"),
        "restored" => messages::pick("♻️ Download ripristinato", "♻️ Download restored"),
        "approved" => messages::pick("🆕 Nuovo episodio approvato", "🆕 New episode approved"),
        value if !value.is_empty() => value,
        _ => messages::pick("✅ Release approvata", "✅ Release approved"),
    };
    match event {
        "message" => text("text"),
        "download_started" => {
            if text("kind") == "movie" {
                format!(
                    "{}\n\n🏷️ Release: {}\n🏆 {} {}\n⚙️ {}: {}",
                    messages::pick("🎬 FILM IN DOWNLOAD!", "🎬 MOVIE DOWNLOAD STARTED!"),
                    text("title"),
                    messages::pick("Score Qualità:", "Quality score:"),
                    data.get("quality_score").and_then(serde_json::Value::as_i64).unwrap_or(0),
                    messages::pick("Motivo", "Reason"),
                    reason
                )
            } else {
                format!(
                    "{}\n\n🎬 {}: {}\n▶️ {}: {}\n🏷️ Release: {}\n🏆 {} {}\n⚙️ {}: {}",
                    messages::pick("📺 NUOVO EPISODIO IN DOWNLOAD!", "📺 NEW EPISODE DOWNLOAD STARTED!"),
                    messages::pick("Serie", "Series"),
                    text("series"),
                    messages::pick("Episodio", "Episode"),
                    episode_code(),
                    text("title"),
                    messages::pick("Score Qualità:", "Quality score:"),
                    data.get("quality_score").and_then(serde_json::Value::as_i64).unwrap_or(0),
                    messages::pick("Motivo", "Reason"),
                    reason
                )
            }
        }
        "torrent_completed" => {
            let duration = data
                .get("duration_seconds")
                .and_then(serde_json::Value::as_i64)
                .filter(|value| *value > 0);
            let speed = data
                .get("average_speed_bps")
                .and_then(serde_json::Value::as_i64)
                .filter(|value| *value > 0);
            let stats = match (duration, speed) {
                (Some(duration), Some(speed)) => {
                    format!("  ⏱️ {}  🚀 {}/s", format_duration(duration), format_bytes(speed))
                }
                (Some(duration), None) => format!("  ⏱️ {}", format_duration(duration)),
                (None, Some(speed)) => format!("  🚀 {}/s", format_bytes(speed)),
                (None, None) => String::new(),
            };
            format!(
                "{}\n\n📺 {}\n\n💾 {}{}\n{}: {}",
                messages::pick("✅ DOWNLOAD COMPLETATO", "✅ DOWNLOAD COMPLETE"),
                series_episode(),
                format_bytes(data.get("size_bytes").and_then(serde_json::Value::as_i64).unwrap_or(0)),
                stats,
                messages::pick("📂 Archiviato in", "📂 Archived to"),
                text("path")
            )
        }
        "season_pack_completed" => {
            let episodes = data
                .get("episodes")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items.iter().map(|item| {
                        format!(
                            "✅ {} - S{:02}E{:02} - {}",
                            value_text(item, "series", ""),
                            item.get("season").and_then(serde_json::Value::as_i64).unwrap_or(0),
                            item.get("episode").and_then(serde_json::Value::as_i64).unwrap_or(0),
                            value_text(item, "path", "")
                        )
                    }).collect::<Vec<_>>().join("\n")
                })
                .unwrap_or_default();
            format!(
                "{}\n\n🎬 {} S{:02}\n\n💾 {}\n\n✅ {} {} {}\n{}\n\n📂 {}",
                messages::pick(
                    "✅ DOWNLOAD COMPLETATO — SEASON PACK ARCHIVIATO",
                    "✅ DOWNLOAD COMPLETE — SEASON PACK ARCHIVED"
                ),
                value_text(data, "series", "Release"),
                data.get("season").and_then(serde_json::Value::as_i64).unwrap_or(0),
                format_bytes(data.get("size_bytes").and_then(serde_json::Value::as_i64).unwrap_or(0)),
                data.get("new_count").and_then(serde_json::Value::as_i64).unwrap_or(0),
                messages::pick("nuovi", "new"),
                data.get("discarded_count").and_then(serde_json::Value::as_i64).filter(|value| *value > 0).map(|value| format!(" 🗑️ {value} {}", messages::pick("inferiori scartati", "inferior discarded"))).unwrap_or_default(),
                episodes,
                value_text(data, "path", "")
            )
        }
        "torrent_error" => format!(
            "Rextto: {} — {}",
            messages::pick("errore torrent", "torrent error"),
            text("error")
        ),
        "gap_filled" => format!(
            "Rextto: {} — {}",
            messages::pick("gap riempito", "gap filled"),
            series_episode()
        ),
        "series_complete" => format!(
            "Rextto: {} — {}",
            messages::pick("serie completata", "series complete"),
            text("series")
        ),
        "movie" => format!(
            "Rextto: {} «{}» {}",
            messages::pick("film", "movie"),
            text("title"),
            messages::pick("disponibile", "available")
        ),
        "download_failed" => format!(
            "Rextto: {} — {}",
            messages::pick("download fallito", "download failed"),
            text("title")
        ),
        "comic_queued" => format!(
            "{}\n\n🏷️ {}: {}\n⚙️ {}: {}\n✨ {}",
            messages::pick("📚 NUOVO FUMETTO IN DOWNLOAD!", "📚 NEW COMIC DOWNLOAD STARTED!"),
            messages::pick("Titolo", "Title"),
            text("title"),
            messages::pick("Metodo", "Method"),
            text("method"),
            messages::pick("Download avviato con successo.", "Download started successfully.")
        ),
        "comic_completed" => format!(
            "{}\n\n🏷️ {}: {}\n⚙️ {}: {}\n💾 {}: {}\n📂 {}: {}",
            messages::pick("✅ FUMETTO SCARICATO!", "✅ COMIC DOWNLOADED!"),
            messages::pick("Titolo", "Title"),
            text("title"),
            messages::pick("Metodo", "Method"),
            text("method"),
            messages::pick("Dimensione", "Size"),
            format_bytes(data.get("size_bytes").and_then(serde_json::Value::as_i64).unwrap_or(0)),
            messages::pick("File salvato nella cartella fumetti", "File saved in the comics folder"),
            text("path")
        ),
        "comic_error" => format!(
            "Rextto: {} «{}» — {}",
            messages::pick("errore fumetto", "comic error"),
            text("title"),
            text("error")
        ),
        "comic_pending" => format!(
            "{}\n\n🏷️ {}: {}\n🔗 {}",
            messages::pick(
                "⏳ FUMETTO PRESENTE MA NON ANCORA SCARICABILE",
                "⏳ COMIC FOUND BUT NOT YET DOWNLOADABLE"
            ),
            messages::pick("Titolo", "Title"),
            text("title"),
            messages::pick(
                "GetComics non ha ancora pubblicato un pulsante Download Now o un link torrent. Rextto riproverà al prossimo ciclo.",
                "GetComics has not published a Download Now button or torrent link yet. Rextto will retry on the next cycle."
            )
        ),
        "backup_completed" => format!(
            "Rextto: {} — {}",
            messages::pick("backup completato", "backup completed"),
            text("path")
        ),
        _ => {
            if let Some(value) = data.get("text").and_then(serde_json::Value::as_str) {
                format!("Rextto [{event}] {value}")
            } else {
                format!("Rextto [{event}] {data}")
            }
        }
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn format_bytes(value: i64) -> String {
    let mut amount = value.max(0) as f64;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut index = 0;
    while amount >= 1024.0 && index < units.len() - 1 {
        amount /= 1024.0;
        index += 1;
    }
    if index == 0 {
        format!("{} {}", amount as i64, units[index])
    } else {
        format!("{amount:.2} {}", units[index])
    }
}

/// Human duration, e.g. "6h 36m 55s".
fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {secs}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_notifier_is_a_noop() {
        let notifier = Notifier::new();
        assert!(!notifier.enabled());
    }

    #[test]
    fn hmac_encoding_is_lowercase_hex() {
        assert_eq!(hex_bytes(&[0, 15, 255]), "000fff");
    }

    #[test]
    fn formats_known_events_in_italian() {
        let episode = serde_json::json!({"series": "FBI", "season": 2, "episode": 5});
        let started = format_event("download_started", &episode);
        assert!(started.contains("Serie: FBI"));
        assert!(started.contains("S02E05"));
        assert!(format_event("gap_filled", &episode).starts_with("Rextto: gap riempito"));
        assert_eq!(
            format_event("message", &serde_json::json!({"text": "ciao"})),
            "ciao"
        );
        let movie = serde_json::json!({"kind": "movie", "title": "Dune"});
        assert!(format_event("download_started", &movie).contains("Dune"));
    }
}
