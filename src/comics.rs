use crate::notifier::Notifier;
use crate::{config::Config, libtorrent::LibtorrentClient, utils::magnet_hash};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Instant,
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize)]
pub struct ComicMonitored {
    pub id: i64,
    pub title: String,
    pub tag_url: String,
    pub cover_url: String,
    pub publisher: String,
    pub description: String,
    pub from_date: String,
    pub save_path: String,
    pub enabled: bool,
    pub last_checked: Option<String>,
}

pub struct ComicsDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComicPost {
    pub title: String,
    pub url: String,
    pub cover_url: String,
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ComicLinks {
    pub magnets: Vec<String>,
    pub torrents: Vec<String>,
    pub mega: Vec<String>,
    pub direct: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComicTorrent {
    pub hash: String,
    pub title: String,
    pub post_url: String,
    pub save_path: String,
    pub completed_at: Option<String>,
    pub processed_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComicDownload {
    pub id: String,
    pub title: String,
    pub method: String,
    pub status: String,
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes: u64,
    pub eta_seconds: Option<u64>,
    pub error: Option<String>,
    pub updated_at: String,
}

static ACTIVE_HTTP_DOWNLOADS: OnceLock<Mutex<BTreeMap<String, ComicDownload>>> = OnceLock::new();

fn http_downloads_store() -> &'static Mutex<BTreeMap<String, ComicDownload>> {
    ACTIVE_HTTP_DOWNLOADS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn update_http_download(id: &str, update: impl FnOnce(&mut ComicDownload)) {
    if let Some(download) = http_downloads_store().lock().unwrap().get_mut(id) {
        update(download);
        download.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

pub fn http_downloads() -> Vec<ComicDownload> {
    let mut items = http_downloads_store()
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    items
}

fn register_http_download(title: &str, method: &str) -> String {
    let id = format!("comic-{}", uuid::Uuid::new_v4());
    http_downloads_store().lock().unwrap().insert(
        id.clone(),
        ComicDownload {
            id: id.clone(),
            title: title.to_owned(),
            method: method.to_owned(),
            status: "downloading".into(),
            progress: if method == "mega" { -1.0 } else { 0.0 },
            downloaded_bytes: 0,
            total_bytes: None,
            speed_bytes: 0,
            eta_seconds: None,
            error: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    id
}

#[derive(Clone)]
pub struct GetComicsClient {
    client: reqwest::Client,
    base_url: String,
}

impl Default for GetComicsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GetComicsClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("rextto/0.1 comics")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("comics HTTP client"),
            base_url: "https://getcomics.org".into(),
        }
    }

    pub async fn tag_posts(&self, tag_url: &str, from_date: &str) -> Result<Vec<ComicPost>> {
        let url = self.absolute(tag_url)?;
        let html = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(parse_articles(&html, from_date, &url))
    }

    /// Searches GetComics through its WordPress search endpoint. The returned URL
    /// can also be stored as a monitored source and re-used by the regular cycle.
    pub async fn search_posts(&self, query: &str) -> Result<(String, Vec<ComicPost>)> {
        let query = query.trim();
        if query.is_empty() {
            anyhow::bail!("search query is required");
        }
        let url = format!(
            "{}?s={}",
            self.base_url,
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
        );
        let posts = self.tag_posts(&url, "").await?;
        Ok((url, posts))
    }

    pub async fn links(&self, post_url: &str) -> Result<ComicLinks> {
        let html = self
            .client
            .get(post_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse_links(&html, post_url)
    }

    pub async fn download_direct(
        &self,
        url: &str,
        target_dir: &Path,
        title: &str,
    ) -> Result<PathBuf> {
        download_http(&self.client, url, target_dir, title).await
    }
    pub async fn download_torrent(&self, url: &str, target_dir: &Path) -> Result<PathBuf> {
        download_torrent_file(&self.client, url, target_dir).await
    }
    pub async fn resolve_mega(&self, url: &str) -> Result<String> {
        if url::Url::parse(url)
            .ok()
            .and_then(|value| {
                value
                    .host_str()
                    .map(|host| host == "mega.nz" || host == "mega.co.nz")
            })
            .unwrap_or(false)
        {
            return Ok(url.to_owned());
        }
        let response = self.client.get(url).send().await?.error_for_status()?;
        if response
            .url()
            .host_str()
            .is_some_and(|host| host == "mega.nz" || host == "mega.co.nz")
        {
            return Ok(response.url().to_string());
        }
        let body = response.text().await?;
        let pattern = regex::Regex::new(r#"https?://(?:mega\.nz|mega\.co\.nz)/[^\s"'<>]+"#)?;
        let found = pattern
            .find_iter(&body)
            .map(|value| value.as_str().replace("&amp;", "&"))
            .find(|value| {
                url::Url::parse(value).ok().is_some_and(|parsed| {
                    parsed
                        .host_str()
                        .is_some_and(|host| host == "mega.nz" || host == "mega.co.nz")
                })
            });
        found.ok_or_else(|| anyhow::anyhow!("Mega URL not found in GetComics redirect"))
    }

    pub async fn weekly_links(&self, date: &str) -> Result<(String, ComicLinks)> {
        for path in [
            format!("/blog/{date}-weekly-pack/"),
            format!("/{date}-weekly-pack/"),
        ] {
            let url = self.absolute(&path)?;
            if let Ok(response) = self.client.get(&url).send().await {
                if let Ok(response) = response.error_for_status() {
                    let html = response.text().await?;
                    let links = parse_links(&html, &url)?;
                    if !links.magnets.is_empty()
                        || !links.torrents.is_empty()
                        || !links.mega.is_empty()
                        || !links.direct.is_empty()
                    {
                        return Ok((url, links));
                    }
                }
            }
        }
        anyhow::bail!("weekly pack not found")
    }

    fn absolute(&self, path: &str) -> Result<String> {
        Ok(url::Url::parse(&self.base_url)?.join(path)?.to_string())
    }
}

fn parse_articles(html: &str, from_date: &str, page_url: &str) -> Vec<ComicPost> {
    let Ok(article_selector) = scraper::Selector::parse("article.post") else {
        return Vec::new();
    };
    let Ok(title_selector) =
        scraper::Selector::parse("h1.post-title a, h2.post-title a, .post-title a")
    else {
        return Vec::new();
    };
    let Ok(image_selector) = scraper::Selector::parse("img.wp-post-image, .post-img img, img[src]")
    else {
        return Vec::new();
    };
    let Ok(date_selector) =
        scraper::Selector::parse("time[datetime], .post-date, .entry-date, .published")
    else {
        return Vec::new();
    };
    let document = scraper::Html::parse_document(html);
    document
        .select(&article_selector)
        .filter_map(|article| {
            let title = article.select(&title_selector).next()?;
            let title_text = title.text().collect::<String>().trim().to_owned();
            let url = url::Url::parse(page_url)
                .ok()?
                .join(title.value().attr("href")?)
                .ok()?
                .to_string();
            let cover_url = article
                .select(&image_selector)
                .next()
                .and_then(|image| {
                    image
                        .value()
                        .attr("src")
                        .or_else(|| image.value().attr("data-src"))
                })
                .and_then(|value| url::Url::parse(page_url).ok()?.join(value).ok())
                .map(|value| value.to_string())
                .unwrap_or_default();
            let date = article
                .select(&date_selector)
                .next()
                .and_then(|element| {
                    element
                        .value()
                        .attr("datetime")
                        .map(str::to_owned)
                        .or_else(|| Some(element.text().collect::<String>().trim().to_owned()))
                })
                .unwrap_or_default();
            let date = regex::Regex::new(r"(\d{4}-\d{2}-\d{2})")
                .ok()
                .and_then(|pattern| {
                    pattern
                        .captures(&date)
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().to_owned())
                })
                .unwrap_or_default();
            if !from_date.is_empty() && !date.is_empty() && date.as_str() < from_date {
                return None;
            }
            Some(ComicPost {
                title: title_text,
                url,
                cover_url,
                date,
            })
        })
        .collect()
}

pub async fn run_cycle(
    db: &ComicsDb,
    client: &GetComicsClient,
    notifier: &Notifier,
    default_root: &Path,
    torrents: &LibtorrentClient,
    cfg: &Config,
) -> Result<usize> {
    let monitored = db.list_monitored(true)?;
    let mut downloaded = 0;
    for comic in monitored {
        let posts = match client.tag_posts(&comic.tag_url, &comic.from_date).await {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(comic=%comic.title, %error, "comics tag fetch failed");
                db.mark_checked(comic.id)?;
                continue;
            }
        };
        for post in posts {
            if db.already_sent(&post.url)? {
                continue;
            }
            let links = match client.links(&post.url).await {
                Ok(links) => links,
                Err(error) => {
                    tracing::warn!(post=%post.url, %error, "comics post fetch failed");
                    continue;
                }
            };
            if cfg.dry_run {
                tracing::info!(title=%post.title, "dry-run: comic release discovered, download skipped");
                continue;
            }
            let target = if comic.save_path.trim().is_empty() {
                default_root.to_path_buf()
            } else {
                PathBuf::from(&comic.save_path)
            };
            let download_result = if let Some(url) = links.direct.first() {
                client
                    .download_direct(url, &target, &post.title)
                    .await
                    .map(|path| (path, "http", None))
            } else if let Some(url) = links.mega.first() {
                let executable = std::env::var_os("REXTTO_MEGADL")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("megadl"));
                match client.resolve_mega(url).await {
                    Ok(resolved) => download_mega(&executable, &resolved, &target)
                        .await
                        .map(|path| (path, "mega", None)),
                    Err(error) => Err(error),
                }
            } else if let Some(magnet) = links.magnets.first() {
                match torrents.add(magnet, cfg) {
                    Ok(true) => {
                        let hash = magnet_hash(magnet).ok_or_else(|| {
                            anyhow::anyhow!("comic magnet has no valid info hash")
                        })?;
                        let _ = notifier
                            .notify_event(
                                "comic_queued",
                                serde_json::json!({"title": post.title, "magnet_hash": hash, "method": "torrent"}),
                            )
                            .await;
                        Ok((target.join(&post.title), "torrent", Some(hash)))
                    }
                    Ok(false) => Err(anyhow::anyhow!("comic magnet is already queued")),
                    Err(error) => Err(error),
                }
            } else if let Some(url) = links.torrents.first() {
                let torrent_dir = target.join(".torrents");
                match client.download_torrent(url, &torrent_dir).await {
                    Ok(path) => match torrents.add_torrent_file(&path, &target) {
                        Ok(Some(hash)) => {
                            let _ = std::fs::remove_file(&path);
                            let _ = notifier.notify_event("comic_queued", serde_json::json!({"title": post.title, "torrent_url": url, "hash": hash, "method": "torrent"})).await;
                            Ok((target.join(&post.title), "torrent", Some(hash)))
                        }
                        Ok(None) => Err(anyhow::anyhow!("torrent client unavailable")),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                }
            } else {
                tracing::debug!(post=%post.url, "comics post has no safe direct or Mega link; leaving it pending");
                continue;
            };
            match download_result {
                Ok((path, method, torrent_hash)) => {
                    if db.add_history(
                        comic.id,
                        &post.url,
                        &post.title,
                        links
                            .magnets
                            .first()
                            .map(String::as_str)
                            .unwrap_or_default(),
                        links
                            .torrents
                            .first()
                            .map(String::as_str)
                            .unwrap_or_default(),
                    )? {
                        if let Some(hash) = torrent_hash {
                            db.add_torrent(&hash, &post.url, &post.title, &target)?;
                        }
                        downloaded += 1;
                        if let Err(error) = notifier
                            .notify_comic_complete(
                                &post.title,
                                &path.display().to_string(),
                                std::fs::metadata(&path)
                                    .map(|value| value.len())
                                    .unwrap_or(0),
                                method,
                            )
                            .await
                        {
                            tracing::warn!(%error, "comic notification failed");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(post=%post.url, %error, "comic direct download failed")
                }
            }
        }
        db.mark_checked(comic.id)?;
    }
    if db.setting("weekly_enabled", "no")? == "yes" && !cfg.dry_run {
        let weekly_from_date = db.setting("weekly_from_date", "")?;
        for offset in 0..=7 {
            let date =
                (chrono::Utc::now().date_naive() - chrono::Duration::days(offset)).to_string();
            if !weekly_from_date.trim().is_empty() && date < weekly_from_date {
                continue;
            }
            let Ok((_url, links)) = client.weekly_links(&date).await else {
                continue;
            };
            if !db.add_weekly(
                &date,
                links
                    .magnets
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default(),
                links
                    .torrents
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default(),
            )? {
                continue;
            }
            if let Some(magnet) = links.magnets.first() {
                if let Some(hash) = magnet_hash(magnet) {
                    if torrents.add(magnet, cfg)? {
                        db.add_torrent(
                            &hash,
                            &format!("weekly:{date}"),
                            &format!("Weekly Pack {date}"),
                            &cfg.libtorrent_dir,
                        )?;
                        db.mark_weekly_sent(&date)?;
                        let _ = notifier.notify_event("comic_queued", serde_json::json!({"title": format!("Weekly Pack {date}"), "hash": hash, "method": "torrent"})).await;
                    }
                }
            } else if let Some(url) = links.torrents.first() {
                let torrent_dir = default_root.join(".torrents");
                if let Ok(path) = client.download_torrent(url, &torrent_dir).await {
                    if let Ok(Some(hash)) = torrents.add_torrent_file(&path, &cfg.libtorrent_dir) {
                        let _ = std::fs::remove_file(path);
                        db.add_torrent(
                            &hash,
                            &format!("weekly:{date}"),
                            &format!("Weekly Pack {date}"),
                            &cfg.libtorrent_dir,
                        )?;
                        db.mark_weekly_sent(&date)?;
                        let _ = notifier.notify_event("comic_queued", serde_json::json!({"title": format!("Weekly Pack {date}"), "hash": hash, "method": "torrent"})).await;
                    }
                }
            }
            break;
        }
    }
    Ok(downloaded)
}

fn parse_links(html: &str, page_url: &str) -> Result<ComicLinks> {
    let selector =
        scraper::Selector::parse("a[href]").map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let document = scraper::Html::parse_document(html);
    let mut links = ComicLinks::default();
    for anchor in document.select(&selector) {
        let Some(raw) = anchor.value().attr("href") else {
            continue;
        };
        let href = url::Url::parse(page_url)?.join(raw)?.to_string();
        let text = anchor.text().collect::<String>().to_ascii_lowercase();
        if href.starts_with("magnet:") {
            if !links.magnets.contains(&href) {
                links.magnets.push(href);
            }
        } else if href.to_ascii_lowercase().contains("mega.nz")
            || href.to_ascii_lowercase().contains("mega.co.nz")
            || text.contains("mega")
        {
            if !links.mega.contains(&href) {
                links.mega.push(href);
            }
        } else if href.to_ascii_lowercase().ends_with(".torrent") {
            if !links.torrents.contains(&href) {
                links.torrents.push(href);
            }
        } else if text.contains("download") && !text.contains("how to") {
            if !links.direct.contains(&href) {
                links.direct.push(href);
            }
        }
    }
    Ok(links)
}

impl ComicsDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        ensure_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn optimize(&self, action: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        crate::database::optimize_connection(&conn, action)
    }

    pub fn size_bytes(&self) -> i64 {
        let conn = self.conn.lock().unwrap();
        crate::database::connection_size_bytes(&conn)
    }

    pub fn list_monitored(&self, enabled_only: bool) -> Result<Vec<ComicMonitored>> {
        let conn = self.conn.lock().unwrap();
        let sql = if enabled_only {
            "SELECT id,title,tag_url,cover_url,publisher,description,from_date,save_path,enabled,last_checked FROM comics_monitored WHERE enabled=1 ORDER BY title COLLATE NOCASE"
        } else {
            "SELECT id,title,tag_url,cover_url,publisher,description,from_date,save_path,enabled,last_checked FROM comics_monitored ORDER BY title COLLATE NOCASE"
        };
        let mut statement = conn.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok(ComicMonitored {
                id: row.get(0)?,
                title: row.get(1)?,
                tag_url: row.get(2)?,
                cover_url: row.get(3)?,
                publisher: row.get(4)?,
                description: row.get(5)?,
                from_date: row.get(6)?,
                save_path: row.get(7)?,
                enabled: row.get::<_, i64>(8)? != 0,
                last_checked: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn add_monitored(
        &self,
        title: &str,
        tag_url: &str,
        from_date: &str,
        save_path: &str,
    ) -> Result<i64> {
        let normalized = if let Ok(parsed) = url::Url::parse(tag_url) {
            if !matches!(parsed.scheme(), "http" | "https")
                || !parsed.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("getcomics.org")
                        || host.eq_ignore_ascii_case("www.getcomics.org")
                })
            {
                anyhow::bail!("comic tag URL must belong to getcomics.org");
            }
            parsed.to_string()
        } else if tag_url.starts_with('/') {
            tag_url.to_owned()
        } else {
            format!("/{tag_url}")
        };
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO comics_monitored(title,tag_url,from_date,save_path) VALUES (?1,?2,?3,?4) ON CONFLICT(tag_url) DO UPDATE SET title=excluded.title,from_date=excluded.from_date,save_path=excluded.save_path,enabled=1", params![title, normalized, from_date, save_path])?;
        Ok(conn.query_row(
            "SELECT id FROM comics_monitored WHERE tag_url=?1",
            [normalized],
            |row| row.get(0),
        )?)
    }

    pub fn set_enabled(&self, id: i64, enabled: bool) -> Result<bool> {
        Ok(self.conn.lock().unwrap().execute(
            "UPDATE comics_monitored SET enabled=?1 WHERE id=?2",
            params![enabled as i64, id],
        )? > 0)
    }
    pub fn remove_monitored(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM comics_monitored WHERE id=?1", [id])?
            > 0)
    }

    pub fn mark_checked(&self, id: i64) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE comics_monitored SET last_checked=datetime('now') WHERE id=?1",
            [id],
        )?;
        Ok(())
    }
    pub fn already_sent(&self, post_url: &str) -> Result<bool> {
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM comics_history WHERE post_url=?1)",
            [post_url],
            |row| row.get(0),
        )?)
    }
    pub fn add_history(
        &self,
        monitored_id: i64,
        post_url: &str,
        title: &str,
        magnet: &str,
        torrent_url: &str,
    ) -> Result<bool> {
        Ok(self.conn.lock().unwrap().execute("INSERT OR IGNORE INTO comics_history(monitored_id,post_url,title,magnet,torrent_url) VALUES (?1,?2,?3,?4,?5)", params![monitored_id, post_url, title, magnet, torrent_url])? > 0)
    }
    pub fn add_torrent(
        &self,
        hash: &str,
        post_url: &str,
        title: &str,
        save_path: &Path,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute("INSERT OR REPLACE INTO comics_torrents(hash,post_url,title,save_path) VALUES (?1,?2,?3,?4)", params![hash.to_ascii_lowercase(), post_url, title, save_path.to_string_lossy().to_string()])?;
        Ok(())
    }
    pub fn torrent(&self, hash: &str) -> Result<Option<ComicTorrent>> {
        Ok(self.conn.lock().unwrap().query_row("SELECT hash,title,post_url,save_path,completed_at,processed_path FROM comics_torrents WHERE hash=?1", [hash.to_ascii_lowercase()], |row| Ok(ComicTorrent { hash: row.get(0)?, title: row.get(1)?, post_url: row.get(2)?, save_path: row.get(3)?, completed_at: row.get(4)?, processed_path: row.get(5)? })).optional()?)
    }
    pub fn complete_torrent(&self, hash: &str, path: &str) -> Result<()> {
        self.conn.lock().unwrap().execute("UPDATE comics_torrents SET completed_at=datetime('now'),processed_path=?1 WHERE hash=?2", params![path, hash.to_ascii_lowercase()])?;
        Ok(())
    }
    pub fn history(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare("SELECT post_url,title,magnet,torrent_url,sent_at,size_bytes FROM comics_history ORDER BY id DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 500)], |row| Ok(serde_json::json!({"post_url":row.get::<_, String>(0)?,"title":row.get::<_, String>(1)?,"magnet":row.get::<_, String>(2)?,"torrent_url":row.get::<_, String>(3)?,"sent_at":row.get::<_, String>(4)?,"size_bytes":row.get::<_, i64>(5)?})))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn remove_history(&self, post_url: &str) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM comics_history WHERE post_url=?1", [post_url])?
            > 0)
    }
    pub fn weekly(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare("SELECT pack_date,magnet,torrent_url,sent_at,found_at,size_bytes FROM comics_weekly ORDER BY pack_date DESC LIMIT ?1")?;
        let rows = statement.query_map([limit.clamp(1, 200)], |row| Ok(serde_json::json!({"pack_date":row.get::<_, String>(0)?,"magnet":row.get::<_, String>(1)?,"torrent_url":row.get::<_, String>(2)?,"sent_at":row.get::<_, Option<String>>(3)?,"found_at":row.get::<_, String>(4)?,"size_bytes":row.get::<_, i64>(5)?})))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn setting(&self, key: &str, default: &str) -> Result<String> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM comics_settings WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| default.to_owned()))
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.lock().unwrap().execute("INSERT INTO comics_settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
        Ok(())
    }
    pub fn add_weekly(&self, pack_date: &str, magnet: &str, torrent_url: &str) -> Result<bool> {
        Ok(self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO comics_weekly(pack_date,magnet,torrent_url) VALUES (?1,?2,?3)",
            params![pack_date, magnet, torrent_url],
        )? > 0)
    }
    pub fn mark_weekly_sent(&self, pack_date: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE comics_weekly SET sent_at=datetime('now') WHERE pack_date=?1",
            [pack_date],
        )?;
        Ok(())
    }
}

pub async fn download_http(
    client: &reqwest::Client,
    url: &str,
    target_dir: &Path,
    title: &str,
) -> Result<PathBuf> {
    let id = register_http_download(title, "http");
    let result = async {
        let parsed = url::Url::parse(url).context("invalid comic download URL")?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            anyhow::bail!("invalid comic download URL");
        }
        std::fs::create_dir_all(target_dir)?;
        let mut response = client.get(url).send().await?.error_for_status()?;
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"))
        {
            anyhow::bail!("comic download returned HTML instead of a file");
        }
        let total = response.content_length();
        update_http_download(&id, |download| download.total_bytes = total);
        let filename = format!(
            "{}.cbz",
            title
                .chars()
                .map(
                    |value| if value.is_ascii_alphanumeric() || matches!(value, '.' | '-' | '_') {
                        value
                    } else {
                        ' '
                    }
                )
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        let destination = target_dir.join(filename);
        let temporary = destination.with_extension("part");
        let mut file = tokio::fs::File::create(&temporary).await?;
        let started = Instant::now();
        let mut downloaded = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            let speed = (downloaded as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
            update_http_download(&id, |download| {
                download.downloaded_bytes = downloaded;
                download.speed_bytes = speed;
                download.progress = total
                    .map(|size| {
                        if size == 0 {
                            0.0
                        } else {
                            (downloaded as f64 * 100.0 / size as f64).min(100.0)
                        }
                    })
                    .unwrap_or(-1.0);
                download.eta_seconds = total.and_then(|size| {
                    (speed > 0 && downloaded < size).then_some((size - downloaded) / speed)
                });
            });
        }
        file.flush().await?;
        if downloaded == 0 {
            anyhow::bail!("comic download is empty");
        }
        std::fs::rename(&temporary, &destination)?;
        Ok(destination)
    }
    .await;
    match &result {
        Ok(_) => update_http_download(&id, |download| {
            download.status = "completed".into();
            download.progress = 100.0;
            download.speed_bytes = 0;
            download.eta_seconds = None;
        }),
        Err(error) => update_http_download(&id, |download| {
            download.status = "error".into();
            download.error = Some(error.to_string());
            download.speed_bytes = 0;
            download.eta_seconds = None;
        }),
    }
    result
}

pub async fn download_torrent_file(
    client: &reqwest::Client,
    url: &str,
    target_dir: &Path,
) -> Result<PathBuf> {
    let parsed = url::Url::parse(url).context("invalid torrent URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        anyhow::bail!("refusing non-HTTP torrent URL");
    }
    std::fs::create_dir_all(target_dir)?;
    let response = client.get(url).send().await?.error_for_status()?;
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"))
    {
        anyhow::bail!("torrent URL returned HTML instead of a torrent file");
    }
    let bytes = response.bytes().await?;
    if bytes.is_empty() {
        anyhow::bail!("torrent file is empty");
    }
    let destination = target_dir.join(format!("{}.torrent", crate::utils::stable_id(url)));
    let temporary = destination.with_extension("part");
    std::fs::write(&temporary, &bytes)?;
    std::fs::rename(&temporary, &destination)?;
    Ok(destination)
}

pub async fn download_mega(executable: &Path, url: &str, target_dir: &Path) -> Result<PathBuf> {
    let id = register_http_download(url, "mega");
    let result = download_mega_inner(executable, url, target_dir).await;
    match &result {
        Ok(_) => update_http_download(&id, |download| {
            download.status = "completed".into();
            download.progress = 100.0;
        }),
        Err(error) => update_http_download(&id, |download| {
            download.status = "error".into();
            download.error = Some(error.to_string());
        }),
    }
    result
}

async fn download_mega_inner(executable: &Path, url: &str, target_dir: &Path) -> Result<PathBuf> {
    let parsed = url::Url::parse(url).context("invalid Mega URL")?;
    if !matches!(parsed.host_str(), Some("mega.nz" | "mega.co.nz")) {
        anyhow::bail!("refusing non-Mega URL");
    }
    std::fs::create_dir_all(target_dir)?;
    let before = std::fs::read_dir(target_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .collect::<std::collections::HashSet<_>>();
    let status = tokio::process::Command::new(executable)
        .arg("--path")
        .arg(target_dir)
        .arg(url)
        .status()
        .await
        .context("start Mega downloader")?;
    if !status.success() {
        anyhow::bail!("Mega downloader exited with {status}");
    }
    std::fs::read_dir(target_dir)?
        .filter_map(|entry| entry.ok())
        .find(|entry| !before.contains(&entry.file_name()) && entry.path().is_file())
        .map(|entry| entry.path())
        .ok_or_else(|| anyhow::anyhow!("Mega downloader completed without creating a file"))
}

#[derive(Debug, Default, Serialize)]
pub struct ComicsImportReport {
    pub monitored: usize,
    pub history: usize,
    pub weekly: usize,
    pub settings: usize,
    pub skipped: usize,
}

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS comics_monitored (
            id INTEGER PRIMARY KEY, title TEXT NOT NULL, tag_url TEXT NOT NULL UNIQUE,
            cover_url TEXT DEFAULT '', publisher TEXT DEFAULT '', description TEXT DEFAULT '',
            from_date TEXT NOT NULL, save_path TEXT DEFAULT '', enabled INTEGER DEFAULT 1,
            added_at TEXT DEFAULT (datetime('now')), last_checked TEXT
        );
        CREATE TABLE IF NOT EXISTS comics_history (
            id INTEGER PRIMARY KEY, monitored_id INTEGER NOT NULL, post_url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL, magnet TEXT DEFAULT '', torrent_url TEXT DEFAULT '',
            sent_at TEXT DEFAULT (datetime('now')), size_bytes INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS comics_weekly (
            id INTEGER PRIMARY KEY, pack_date TEXT NOT NULL UNIQUE, magnet TEXT DEFAULT '',
            torrent_url TEXT DEFAULT '', sent_at TEXT, found_at TEXT DEFAULT (datetime('now')),
            size_bytes INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS comics_torrents (
            hash TEXT PRIMARY KEY, post_url TEXT NOT NULL, title TEXT NOT NULL,
            save_path TEXT NOT NULL, queued_at TEXT DEFAULT (datetime('now')),
            completed_at TEXT, processed_path TEXT
        );
        CREATE TABLE IF NOT EXISTS comics_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    Ok(())
}

fn readonly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open source database {}", path.display()))
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )?)
}

pub fn import_from_extto(source: &Path, destination: &Connection) -> Result<ComicsImportReport> {
    ensure_schema(destination)?;
    let source_db = source.join("comics.db");
    if !source_db.exists() {
        return Ok(ComicsImportReport::default());
    }
    let source = readonly(&source_db)?;
    let tx = destination.unchecked_transaction()?;
    let mut report = ComicsImportReport::default();
    let mut id_map = std::collections::HashMap::new();

    if has_table(&source, "comics_monitored")? {
        let mut stmt = source.prepare("SELECT id,title,tag_url,cover_url,publisher,description,from_date,save_path,enabled,added_at,last_checked FROM comics_monitored")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3).unwrap_or_default(),
                r.get::<_, String>(4).unwrap_or_default(),
                r.get::<_, String>(5).unwrap_or_default(),
                r.get::<_, String>(6)?,
                r.get::<_, String>(7).unwrap_or_default(),
                r.get::<_, i64>(8).unwrap_or(1),
                r.get::<_, String>(9).unwrap_or_default(),
                r.get::<_, Option<String>>(10)?,
            ))
        })?;
        for row in rows {
            let (
                old_id,
                title,
                tag_url,
                cover,
                publisher,
                description,
                from_date,
                save_path,
                enabled,
                added_at,
                last_checked,
            ) = row?;
            let inserted = tx.execute("INSERT OR IGNORE INTO comics_monitored(id,title,tag_url,cover_url,publisher,description,from_date,save_path,enabled,added_at,last_checked) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![old_id, title, tag_url, cover, publisher, description, from_date, save_path, enabled, added_at, last_checked])?;
            let new_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM comics_monitored WHERE tag_url=?1",
                    [&tag_url],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(new_id) = new_id {
                id_map.insert(old_id, new_id);
                if inserted > 0 {
                    report.monitored += 1;
                }
            } else {
                report.skipped += 1;
            }
        }
    }
    if has_table(&source, "comics_history")? {
        let mut stmt = source.prepare("SELECT id,monitored_id,post_url,title,magnet,torrent_url,sent_at,size_bytes FROM comics_history")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4).unwrap_or_default(),
                r.get::<_, String>(5).unwrap_or_default(),
                r.get::<_, String>(6).unwrap_or_default(),
                r.get::<_, i64>(7).unwrap_or(0),
            ))
        })?;
        for row in rows {
            let (id, old_monitored, post_url, title, magnet, torrent_url, sent_at, size) = row?;
            let Some(monitored_id) = id_map.get(&old_monitored) else {
                report.skipped += 1;
                continue;
            };
            let inserted = tx.execute("INSERT OR IGNORE INTO comics_history(id,monitored_id,post_url,title,magnet,torrent_url,sent_at,size_bytes) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![id, monitored_id, post_url, title, magnet, torrent_url, sent_at, size])?;
            if inserted > 0 {
                report.history += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    if has_table(&source, "comics_weekly")? {
        let mut stmt = source.prepare(
            "SELECT id,pack_date,magnet,torrent_url,sent_at,found_at,size_bytes FROM comics_weekly",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2).unwrap_or_default(),
                r.get::<_, String>(3).unwrap_or_default(),
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5).unwrap_or_default(),
                r.get::<_, i64>(6).unwrap_or(0),
            ))
        })?;
        for row in rows {
            let (id, date, magnet, torrent_url, sent_at, found_at, size) = row?;
            let inserted = tx.execute("INSERT OR IGNORE INTO comics_weekly(id,pack_date,magnet,torrent_url,sent_at,found_at,size_bytes) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![id, date, magnet, torrent_url, sent_at, found_at, size])?;
            if inserted > 0 {
                report.weekly += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    if has_table(&source, "comics_settings")? {
        let mut stmt = source.prepare("SELECT key,value FROM comics_settings")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (key, value) = row?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO comics_settings(key,value) VALUES (?1,?2)",
                params![key, value],
            )?;
            if inserted > 0 {
                report.settings += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn monitored_history_and_weekly_records_are_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "rextto-comics-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = ComicsDb::open(&path).unwrap();
        let id = db
            .add_monitored("Example", "tag/example", "2026-01-01", "comics")
            .unwrap();
        assert_eq!(
            db.add_monitored("Example updated", "tag/example", "2026-02-01", "comics")
                .unwrap(),
            id
        );
        assert_eq!(db.list_monitored(true).unwrap().len(), 1);
        assert!(db
            .add_history(
                id,
                "https://example/post",
                "Example 001",
                "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
                ""
            )
            .unwrap());
        assert!(!db
            .add_history(id, "https://example/post", "Example 001", "", "")
            .unwrap());
        assert!(db.already_sent("https://example/post").unwrap());
        assert!(db
            .add_weekly("2026-02-01", "", "https://example/weekly")
            .unwrap());
        assert!(!db
            .add_weekly("2026-02-01", "", "https://example/weekly")
            .unwrap());
        assert!(db.set_enabled(id, false).unwrap());
        assert!(!db
            .list_monitored(true)
            .unwrap()
            .iter()
            .any(|comic| comic.id == id));
        assert!(db.remove_monitored(id).unwrap());
        assert!(!db.remove_monitored(id).unwrap());
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn parses_posts_and_classifies_download_links() {
        let html = r#"<article class='post'><h2 class='post-title'><a href='/post/example'>Example</a></h2><time datetime='2026-09-19T10:00:00'></time><img src='/cover.jpg'></article><a href='magnet:?xt=urn:btih:0123456789012345678901234567890123456789'>torrent</a><a href='/dlds/1'>MEGA</a><a href='/file.torrent'>torrent file</a><a href='/download/file'>Download now</a>"#;
        let posts = parse_articles(html, "2026-01-01", "https://getcomics.org/tag/example/");
        assert_eq!(posts[0].title, "Example");
        assert_eq!(posts[0].date, "2026-09-19");
        let links = parse_links(html, "https://getcomics.org/post/example").unwrap();
        assert_eq!(links.magnets.len(), 1);
        assert_eq!(links.mega.len(), 1);
        assert_eq!(links.torrents.len(), 1);
        assert_eq!(links.direct.len(), 1);
    }

    #[test]
    fn monitored_comic_rejects_remote_tag_host() {
        let path =
            std::env::temp_dir().join(format!("rextto-comics-host-{}", uuid::Uuid::new_v4()));
        let db = ComicsDb::open(&path).unwrap();
        assert!(db
            .add_monitored("Example", "https://example.invalid/tag/example", "", "")
            .is_err());
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
