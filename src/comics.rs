use crate::notifier::Notifier;
use crate::{config::Config, libtorrent::LibtorrentClient, utils::magnet_hash};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize)]
pub struct ComicMonitored {
    pub id: i64,
    pub title: String,
    pub tag_url: String,
    pub post_url: String,
    pub cover_url: String,
    pub publisher: String,
    pub description: String,
    pub from_date: String,
    pub save_path: String,
    pub enabled: bool,
    pub last_checked: Option<String>,
    pub latest_downloaded_title: String,
}

pub struct ComicsDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComicPost {
    pub title: String,
    pub url: String,
    pub tag_url: String,
    pub cover_url: String,
    pub publisher: String,
    pub description: String,
    pub date: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ComicLinks {
    pub magnets: Vec<String>,
    pub torrents: Vec<String>,
    pub mega: Vec<String>,
    pub direct: Vec<String>,
    pub download_now: Vec<String>,
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
        let url = self.resolve_direct_url(url).await?;
        download_http(&self.client, &url, target_dir, title).await
    }
    /// Resolves a GetComics redirect and starts the HTTP download in the
    /// background, returning the id used by the Comics download list.
    pub async fn start_direct_download(
        &self,
        url: &str,
        target_dir: &Path,
        title: &str,
    ) -> Result<String> {
        let url = self.resolve_direct_url(url).await?;
        Ok(start_http_download(
            self.client.clone(),
            url,
            target_dir.to_path_buf(),
            title.to_owned(),
        ))
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

    /// GetComics currently exposes PixelDrain files through `/dls/...`, which
    /// redirects to a PixelDrain HTML page. The HTML page is not downloadable
    /// by `download_http`; convert the public file page to PixelDrain's binary
    /// API endpoint first.
    async fn resolve_direct_url(&self, url: &str) -> Result<String> {
        let parsed = url::Url::parse(url)?;
        let is_getcomics_redirect = parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("getcomics.org")
                || host.eq_ignore_ascii_case("www.getcomics.org")
        }) && parsed.path().starts_with("/dls/");
        if !is_getcomics_redirect {
            return Ok(url.to_owned());
        }
        let response = self.client.get(url).send().await?.error_for_status()?;
        let redirected = response.url();
        if redirected.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("pixeldrain.com")
                || host.eq_ignore_ascii_case("www.pixeldrain.com")
        }) {
            let mut segments = redirected.path_segments().into_iter().flatten();
            if segments.next() == Some("u") {
                if let Some(file_id) = segments.next().filter(|value| !value.is_empty()) {
                    return Ok(format!(
                        "https://pixeldrain.com/api/file/{file_id}?download=1"
                    ));
                }
            }
        }
        Ok(redirected.to_string())
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
                    // Tollera errori di parsing: prosegue con le altre strategie.
                    let links = parse_links(&html, &url).unwrap_or_default();
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
        // Strategia 3 (come il legacy extto): ricerca testuale
        // "YYYY.MM.DD Weekly Pack" e verifica i link del post trovato.
        if let Ok(parsed) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            let query = format!("{} Weekly Pack", parsed.format("%Y.%m.%d"));
            if let Ok((_url, posts)) = self.search_posts(&query).await {
                for post in posts {
                    let title = post.title.to_lowercase();
                    if !title.contains("weekly") || !title.contains("pack") {
                        continue;
                    }
                    if let Ok(links) = self.links(&post.url).await {
                        if !links.magnets.is_empty()
                            || !links.torrents.is_empty()
                            || !links.mega.is_empty()
                            || !links.direct.is_empty()
                        {
                            return Ok((post.url, links));
                        }
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

/// Data canonica `YYYY-MM-DD` dal titolo di un weekly pack (che può usare
/// separatori con punto o trattino, es. "2026.09.16 Weekly Pack").
fn extract_pack_date(title: &str) -> Option<String> {
    let pattern = crate::utils::cached_regex(r"(\d{4})[.\-](\d{2})[.\-](\d{2})").ok()?;
    let captures = pattern.captures(title)?;
    Some(format!(
        "{}-{}-{}",
        captures.get(1)?.as_str(),
        captures.get(2)?.as_str(),
        captures.get(3)?.as_str()
    ))
}

/// Titolo "pulito" per la ricerca per nome su GetComics: rimuove il numero
/// dell'albo (`#41`) e l'anno (`(2026)`), come faceva il legacy extto. Serve a
/// trovare le uscite anche quando il tag salvato è obsoleto o inesistente.
fn clean_search_title(title: &str) -> String {
    let base = title.split('#').next().unwrap_or(title).trim();
    let without_year = base
        .strip_suffix(')')
        .and_then(|value| value.rsplit_once('('))
        .filter(|(_, year)| year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()))
        .map(|(name, _)| name.trim())
        .unwrap_or(base);
    without_year
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == ' ' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extracts a comic issue number from titles such as `Poison Ivy #41 (2026)`.
/// A monitored issue must not be replaced by a newer issue or a collected
/// edition merely because the tag URL is stale and the name search is broad.
fn issue_number(title: &str) -> Option<String> {
    let after_marker = title.split('#').nth(1)?;
    let digits = after_marker
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn matches_monitored_issue(monitored_title: &str, post_title: &str) -> bool {
    match (issue_number(monitored_title), issue_number(post_title)) {
        (Some(monitored), Some(post)) => monitored == post,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn extract_tag_url(article: scraper::element_ref::ElementRef<'_>, page_url: &str, title: &str) -> String {
    let Ok(selector) = scraper::Selector::parse("a[href*='/tag/']") else {
        return String::new();
    };
    let title_slug = title
        .to_ascii_lowercase()
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
        .collect::<String>();
    let mut fallback = String::new();
    for anchor in article.select(&selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Ok(url) = url::Url::parse(page_url).and_then(|base| base.join(href)) else {
            continue;
        };
        let value = url.to_string();
        if fallback.is_empty() {
            fallback = value.clone();
        }
        if url
            .path_segments()
            .into_iter()
            .flatten()
            .any(|segment| title_slug.contains(segment))
        {
            return value;
        }
    }
    if !fallback.is_empty() {
        return fallback;
    }
    let mut words = title_slug
        .split('-')
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if words.last().is_some_and(|word| word.len() == 4 && word.chars().all(|c| c.is_ascii_digit())) {
        words.pop();
    }
    if !words.is_empty() {
        return format!("https://getcomics.org/tag/{}/", words.join("-"));
    }
    String::new()
}

fn extract_publisher(article: scraper::element_ref::ElementRef<'_>) -> String {
    let Ok(selector) = scraper::Selector::parse("a[href*='/cat/'], a[href*='/tag/']") else {
        return String::new();
    };
    let known = [
        ("dc", "DC Comics"),
        ("marvel", "Marvel"),
        ("image", "Image Comics"),
        ("dark-horse", "Dark Horse"),
        ("idw", "IDW"),
        ("dynamite", "Dynamite"),
        ("boom", "BOOM! Studios"),
    ];
    for anchor in article.select(&selector) {
        let value = format!(
            "{} {}",
            anchor.value().attr("href").unwrap_or_default(),
            anchor.text().collect::<String>()
        )
        .to_ascii_lowercase();
        if let Some((_, publisher)) = known.iter().find(|(needle, _)| value.contains(needle)) {
            return (*publisher).to_owned();
        }
    }
    String::new()
}

fn extract_description(article: scraper::element_ref::ElementRef<'_>) -> String {
    let Ok(selector) = scraper::Selector::parse(".post-info p, .entry-content p") else {
        return String::new();
    };
    article
        .select(&selector)
        .next()
        .map(|element| element.text().collect::<String>().trim().chars().take(300).collect())
        .unwrap_or_default()
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
                title: title_text.clone(),
                url,
                tag_url: extract_tag_url(article, page_url, &title_text),
                cover_url,
                publisher: extract_publisher(article),
                description: extract_description(article),
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
    tracing::info!(monitored = monitored.len(), "comics cycle: checking monitored titles");
    for comic in monitored {
        tracing::info!(comic=%comic.title, tag=%comic.tag_url, "comics: checking title");
        // A numbered monitor (e.g. `Poison Ivy #41 (2026)`) identifies one
        // exact issue. Its publication date may be older than the generic
        // monitoring start date, so that date must not hide the requested issue.
        let exact_issue = issue_number(&comic.title).is_some();
        let tag_from_date = if exact_issue { "" } else { &comic.from_date };
        // Un risultato scelto dalla ricerca è un post preciso: non va
        // sostituito da una nuova ricerca generica dello stesso nome.
        let exact_post_url = if !comic.post_url.trim().is_empty() {
            Some(comic.post_url.clone())
        } else if !comic.tag_url.contains("/tag/") {
            // Compatibilità con le righe create dalle versioni precedenti.
            Some(comic.tag_url.clone())
        } else {
            None
        };
        // Anche col tag obsoleto (404) la ricerca per nome, come nel legacy
        // extto, trova comunque le nuove uscite: uniamo le due fonti.
        let (mut posts, tag_error) = if let Some(post_url) = exact_post_url.as_deref() {
            (
                vec![ComicPost {
                    title: comic.title.clone(),
                    url: post_url.to_owned(),
                    tag_url: comic.tag_url.clone(),
                    cover_url: comic.cover_url.clone(),
                    publisher: comic.publisher.clone(),
                    description: comic.description.clone(),
                    date: comic.from_date.clone(),
                }],
                None,
            )
        } else {
            match client.tag_posts(&comic.tag_url, tag_from_date).await {
                Ok(posts) => (posts, None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            }
        };
        let clean = clean_search_title(&comic.title);
        let search_posts = if exact_post_url.is_some() || clean.is_empty() {
            Vec::new()
        } else {
            match client.search_posts(&clean).await {
                Ok((_url, posts)) => posts,
                Err(error) => {
                    tracing::warn!(comic=%comic.title, %error, "comics: name search failed");
                    Vec::new()
                }
            }
        };
        match (tag_error.as_deref(), search_posts.is_empty()) {
            (Some(error), false) => {
                tracing::debug!(comic=%comic.title, %error, "comics: tag unavailable, using name search")
            }
            (Some(error), true) => {
                tracing::warn!(comic=%comic.title, %error, "comics tag fetch failed and name search returned nothing")
            }
            _ => {}
        }
        let mut merged: BTreeMap<String, ComicPost> = BTreeMap::new();
        for post in posts.drain(..).chain(search_posts) {
            if !matches_monitored_issue(&comic.title, &post.title) {
                tracing::debug!(
                    comic = %comic.title,
                    post = %post.title,
                    "comics: ignoring post for a different issue or collection"
                );
                continue;
            }
            if !exact_issue
                && !comic.from_date.trim().is_empty()
                && !post.date.is_empty()
                && post.date.as_str() < comic.from_date.as_str()
            {
                continue;
            }
            merged.insert(post.url.clone(), post);
        }
        let posts = merged.into_values().collect::<Vec<_>>();
        tracing::info!(comic=%comic.title, posts=posts.len(), "comics: candidate posts");
        let mut queued = 0usize;
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
                tracing::info!(post=%post.url, title=%post.title, "comics post found but has no Download Now or torrent link yet; leaving it pending");
                let _ = notifier
                    .notify_event(
                        "comic_pending",
                        serde_json::json!({
                            "title": post.title,
                            "post_url": post.url,
                            "kind": "comic",
                        }),
                    )
                    .await;
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
                        queued += 1;
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
        tracing::info!(comic=%comic.title, queued, "comics: title checked");
    }
    if db.setting("weekly_enabled", "no")? == "yes" && !cfg.dry_run {
        let weekly_from_date = db.setting("weekly_from_date", "")?;
        tracing::info!(from_date=%weekly_from_date, "comics: checking weekly packs");
        // 1A. Ritenta i pack registrati ma mai inviati (come il legacy extto).
        for (date, magnet, torrent_url) in db.pending_weekly().unwrap_or_default() {
            match send_weekly_pack(
                db,
                client,
                torrents,
                notifier,
                default_root,
                cfg,
                &date,
                &magnet,
                &torrent_url,
                "",
            )
            .await
            {
                Ok(true) => tracing::info!(date=%date, "comics: pending weekly pack sent"),
                Ok(false) => {
                    tracing::debug!(date=%date, "comics: pending weekly pack not accepted")
                }
                Err(error) => {
                    tracing::warn!(date=%date, %error, "comics: pending weekly pack failed")
                }
            }
        }
        // 1B. Cerca nuovi pack: aggiorna anche le righe esistenti con link
        // vuoto, così una riga preesistente non blocca il download.
        // 1B. Cerca i pack recenti con UNA sola ricerca (come il legacy extto):
        // così non si fanno decine di richieste per data (che su GetComics
        // finiscono in rate-limit e facevano fallire il pack giusto).
        let mut weekly_checked = 0usize;
        match client.search_posts("Weekly Pack").await {
            Ok((_url, posts)) => {
                for post in posts {
                    let title = post.title.to_lowercase();
                    if !title.contains("weekly") || !title.contains("pack") {
                        continue;
                    }
                    let Some(date) = extract_pack_date(&post.title) else {
                        continue;
                    };
                    if !weekly_from_date.trim().is_empty() && date < weekly_from_date {
                        continue;
                    }
                    weekly_checked += 1;
                    let Ok(links) = client.links(&post.url).await else {
                        tracing::debug!(date = %date, post = %post.url, "comics: weekly pack links failed");
                        continue;
                    };
                    let magnet = links
                        .magnets
                        .first()
                        .map(String::as_str)
                        .unwrap_or_default();
                    let torrent_url = links
                        .torrents
                        .first()
                        .map(String::as_str)
                        .unwrap_or_default();
                    let direct_url = links
                        .download_now
                        .first()
                        .or_else(|| links.direct.first())
                        .map(String::as_str)
                        .unwrap_or_default();
                    if magnet.is_empty() && torrent_url.is_empty() && direct_url.is_empty() {
                        let _ = db.upsert_weekly_links(&date, "", "")?;
                        tracing::info!(date = %date, "comics: weekly pack found without Download Now or torrent link yet");
                        let _ = notifier
                            .notify_event(
                                "comic_pending",
                                serde_json::json!({
                                    "title": format!("Weekly Pack {date}"),
                                    "post_url": post.url,
                                    "kind": "weekly",
                                }),
                            )
                            .await;
                        continue;
                    }
                    let eligible = db.upsert_weekly_links(&date, magnet, torrent_url)?;
                    let should_send = !db.weekly_sent(&date)? && (eligible || !direct_url.is_empty());
                    tracing::info!(date = %date, eligible = should_send, "comics: weekly pack recorded");
                    if !should_send {
                        continue;
                    }
                    match send_weekly_pack(
                        db,
                        client,
                        torrents,
                        notifier,
                        default_root,
                        cfg,
                        &date,
                        magnet,
                        torrent_url,
                        direct_url,
                    )
                    .await
                    {
                        Ok(true) => {
                            tracing::info!(date=%date, "comics: weekly pack queued");
                            break;
                        }
                        Ok(false) => {
                            tracing::debug!(date=%date, "comics: weekly pack not accepted")
                        }
                        Err(error) => {
                            tracing::warn!(date=%date, %error, "comics: weekly pack failed")
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "comics: weekly pack search failed");
            }
        }
        tracing::info!(checked = weekly_checked, "comics: weekly check done");
    } else if db.setting("weekly_enabled", "no")? != "yes" {
        tracing::info!("comics: weekly packs disabled");
    }
    Ok(downloaded)
}

/// Invia al client un weekly pack (magnet o file `.torrent`). Ritorna `true` se
/// è stato accettato e marcato come inviato.
#[allow(clippy::too_many_arguments)]
async fn send_weekly_pack(
    db: &ComicsDb,
    client: &GetComicsClient,
    torrents: &LibtorrentClient,
    notifier: &Notifier,
    default_root: &Path,
    cfg: &Config,
    date: &str,
    magnet: &str,
    torrent_url: &str,
    direct_url: &str,
) -> Result<bool> {
    if !direct_url.is_empty() {
        let path = client
            .download_direct(direct_url, default_root, &format!("Weekly Pack {date}"))
            .await?;
        db.mark_weekly_sent(date)?;
        let _ = notifier
            .notify_comic_complete(
                &format!("Weekly Pack {date}"),
                &path.display().to_string(),
                std::fs::metadata(&path)
                    .map(|value| value.len())
                    .unwrap_or(0),
                "http",
            )
            .await;
        return Ok(true);
    }
    if !magnet.is_empty() {
        if let Some(hash) = magnet_hash(magnet) {
            if torrents.add(magnet, cfg)? {
                db.add_torrent(
                    &hash,
                    &format!("weekly:{date}"),
                    &format!("Weekly Pack {date}"),
                    &cfg.libtorrent_dir,
                )?;
                db.mark_weekly_sent(date)?;
                let _ = notifier
                    .notify_event(
                        "comic_queued",
                        serde_json::json!({"title": format!("Weekly Pack {date}"), "hash": hash, "method": "torrent"}),
                    )
                    .await;
                return Ok(true);
            }
        }
    } else if !torrent_url.is_empty() {
        let torrent_dir = default_root.join(".torrents");
        if let Ok(path) = client.download_torrent(torrent_url, &torrent_dir).await {
            if let Ok(Some(hash)) = torrents.add_torrent_file(&path, &cfg.libtorrent_dir) {
                let _ = std::fs::remove_file(path);
                db.add_torrent(
                    &hash,
                    &format!("weekly:{date}"),
                    &format!("Weekly Pack {date}"),
                    &cfg.libtorrent_dir,
                )?;
                db.mark_weekly_sent(date)?;
                let _ = notifier
                    .notify_event(
                        "comic_queued",
                        serde_json::json!({"title": format!("Weekly Pack {date}"), "hash": hash, "method": "torrent"}),
                    )
                    .await;
                return Ok(true);
            }
        }
    }
    Ok(false)
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
        // GetComics sometimes renders a decorative "Download Now" anchor with
        // an empty href before the real file-host link. Joining an empty href
        // to the post URL would incorrectly turn the post page itself into a
        // download candidate.
        if raw.trim().is_empty() || raw.trim() == "#" {
            continue;
        }
        // Un singolo href non valido (es. magnet con caratteri strani) non deve
        // far fallire l'intero parsing della pagina: si salta solo quell'anchor.
        let Ok(joined) = url::Url::parse(page_url).and_then(|base| base.join(raw)) else {
            continue;
        };
        let href = joined.to_string();
        let text = anchor.text().collect::<String>().to_ascii_lowercase();
        let normalized_text = text.replace(['-', '_'], " ");
        let title_attr = anchor
            .value()
            .attr("title")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_download_now = normalized_text.contains("download now")
            || title_attr.replace(['-', '_'], " ").contains("download now");
        let path = joined.path().to_ascii_lowercase();
        // The site's help link contains "download" but is not a file. The
        // hyphen in "how-to" previously bypassed the text check and became
        // the first direct link, causing every comic download to fetch the
        // help page instead of the file.
        if path.contains("/how-to-download") || normalized_text.contains("how to") {
            continue;
        }
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
        } else if ((path.starts_with("/dls/")
            && joined.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("getcomics.org")
                    || host.eq_ignore_ascii_case("www.getcomics.org")
            }))
            || [".cbr", ".cbz", ".rar", ".zip"]
                .iter()
                .any(|extension| path.ends_with(extension))
            || text.contains("download")
            || is_download_now)
            && !links.direct.contains(&href)
        {
            if is_download_now && !links.download_now.contains(&href) {
                links.download_now.push(href.clone());
            }
            links.direct.push(href);
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
            "SELECT m.id,m.title,m.tag_url,m.post_url,m.cover_url,m.publisher,m.description,m.from_date,m.save_path,m.enabled,m.last_checked,COALESCE((SELECT h.title FROM comics_history h WHERE h.monitored_id=m.id ORDER BY h.id DESC LIMIT 1),'') FROM comics_monitored m WHERE m.enabled=1 ORDER BY m.title COLLATE NOCASE"
        } else {
            "SELECT m.id,m.title,m.tag_url,m.post_url,m.cover_url,m.publisher,m.description,m.from_date,m.save_path,m.enabled,m.last_checked,COALESCE((SELECT h.title FROM comics_history h WHERE h.monitored_id=m.id ORDER BY h.id DESC LIMIT 1),'') FROM comics_monitored m ORDER BY m.title COLLATE NOCASE"
        };
        let mut statement = conn.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok(ComicMonitored {
                id: row.get(0)?,
                title: row.get(1)?,
                tag_url: row.get(2)?,
                post_url: row.get(3)?,
                cover_url: row.get(4)?,
                publisher: row.get(5)?,
                description: row.get(6)?,
                from_date: row.get(7)?,
                save_path: row.get(8)?,
                enabled: row.get::<_, i64>(9)? != 0,
                last_checked: row.get(10)?,
                latest_downloaded_title: row.get(11)?,
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
        self.add_monitored_with_metadata(
            title,
            tag_url,
            "",
            "",
            "",
            "",
            from_date,
            save_path,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_monitored_with_metadata(
        &self,
        title: &str,
        tag_url: &str,
        post_url: &str,
        cover_url: &str,
        publisher: &str,
        description: &str,
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
            let mut path = parsed.path().to_owned();
            if let Some(query) = parsed.query() {
                path.push('?');
                path.push_str(query);
            }
            path
        } else if tag_url.starts_with('/') {
            tag_url.to_owned()
        } else {
            format!("/{tag_url}")
        };
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO comics_monitored(title,tag_url,post_url,cover_url,publisher,description,from_date,save_path) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(tag_url) DO UPDATE SET title=excluded.title,post_url=CASE WHEN excluded.post_url<>'' THEN excluded.post_url ELSE comics_monitored.post_url END,cover_url=CASE WHEN excluded.cover_url<>'' THEN excluded.cover_url ELSE comics_monitored.cover_url END,publisher=CASE WHEN excluded.publisher<>'' THEN excluded.publisher ELSE comics_monitored.publisher END,description=CASE WHEN excluded.description<>'' THEN excluded.description ELSE comics_monitored.description END,from_date=excluded.from_date,save_path=excluded.save_path,enabled=1", params![title, normalized, post_url, cover_url, publisher, description, from_date, save_path])?;
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

    pub fn weekly_sent(&self, pack_date: &str) -> Result<bool> {
        Ok(self.conn.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM comics_weekly WHERE pack_date=?1 AND sent_at IS NOT NULL)",
            [pack_date],
            |row| row.get(0),
        )?)
    }

    /// Registra i link di un weekly pack, **riempiendo** anche una riga già
    /// presente ma creata prima che il torrent fosse disponibile (caso previsto
    /// dal legacy extto). Ritorna `true` se la riga ora ha un link ed è ancora
    /// da inviare, così il ciclo può scaricarla.
    pub fn upsert_weekly_links(
        &self,
        pack_date: &str,
        magnet: &str,
        torrent_url: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO comics_weekly(pack_date,magnet,torrent_url) VALUES (?1,?2,?3)
             ON CONFLICT(pack_date) DO UPDATE SET
                 magnet=CASE WHEN COALESCE(comics_weekly.magnet,'')='' THEN excluded.magnet ELSE comics_weekly.magnet END,
                 torrent_url=CASE WHEN COALESCE(comics_weekly.torrent_url,'')='' THEN excluded.torrent_url ELSE comics_weekly.torrent_url END",
            params![pack_date, magnet, torrent_url],
        )?;
        Ok(conn.query_row(
            "SELECT sent_at IS NULL AND (COALESCE(magnet,'')<>'' OR COALESCE(torrent_url,'')<>'') FROM comics_weekly WHERE pack_date=?1",
            [pack_date],
            |row| row.get(0),
        )?)
    }

    /// Weekly pack registrati ma mai inviati che hanno già un link: da ritentare
    /// ad ogni ciclo, come il legacy extto.
    pub fn pending_weekly(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT pack_date,COALESCE(magnet,''),COALESCE(torrent_url,'') FROM comics_weekly
             WHERE sent_at IS NULL AND (COALESCE(magnet,'')<>'' OR COALESCE(torrent_url,'')<>'')
             ORDER BY pack_date",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Tentativi per un download diretto: gli host di file dietro Cloudflare a
/// volte chiudono lo stream a metà ("error decoding response body" è proprio
/// questo, non un problema di compressione). Riprendendo dal file `.part` con
/// una Range request non si riscarica da zero.
const HTTP_DOWNLOAD_ATTEMPTS: u32 = 3;
/// Timeout del singolo tentativo di download. Il timeout di 15 s del client è
/// pensato per le pagine HTML e taglierebbe i fumetti da decine/centinaia di MB.
const HTTP_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(1800);

pub async fn download_http(
    client: &reqwest::Client,
    url: &str,
    target_dir: &Path,
    title: &str,
) -> Result<PathBuf> {
    let id = register_http_download(title, "http");
    download_http_registered(client, url, target_dir, title, &id).await
}

fn start_http_download(
    client: reqwest::Client,
    url: String,
    target_dir: PathBuf,
    title: String,
) -> String {
    let id = register_http_download(&title, "http");
    tracing::info!(title=%title, download_id=%id, "comic HTTP download started");
    let download_id = id.clone();
    tokio::spawn(async move {
        if let Err(error) = download_http_registered(
            &client,
            &url,
            &target_dir,
            &title,
            &download_id,
        )
        .await
        {
            tracing::warn!(title=%title, download_id=%download_id, %error, "comic background download failed");
        }
    });
    id
}

async fn download_http_registered(
    client: &reqwest::Client,
    url: &str,
    target_dir: &Path,
    title: &str,
    id: &str,
) -> Result<PathBuf> {
    let mut outcome: Option<PathBuf> = None;
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=HTTP_DOWNLOAD_ATTEMPTS {
        match download_http_once(client, &id, url, target_dir, title).await {
            Ok(path) => {
                outcome = Some(path);
                break;
            }
            Err(error) => {
                let error = error.context(format!("GET {url}"));
                if attempt < HTTP_DOWNLOAD_ATTEMPTS {
                    tracing::warn!(attempt, %error, "comic download attempt failed; retrying");
                    tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                }
                last_error = Some(error);
            }
        }
    }
    let result = match (outcome, last_error) {
        (Some(path), _) => Ok(path),
        (None, Some(error)) => Err(error),
        (None, None) => Err(anyhow::anyhow!("comic download failed")),
    };
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
    if let Ok(path) = &result {
        tracing::info!(title=%title, download_id=%id, path=%path.display(), "comic HTTP download completed");
    }
    result
}

async fn download_http_once(
    client: &reqwest::Client,
    id: &str,
    url: &str,
    target_dir: &Path,
    title: &str,
) -> Result<PathBuf> {
    let parsed = url::Url::parse(url).context("invalid comic download URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        anyhow::bail!("invalid comic download URL");
    }
    std::fs::create_dir_all(target_dir)?;
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
    let offset = std::fs::metadata(&temporary)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut request = client
        .get(url)
        // File binari: nessuna decompressione, così un eventuale errore del
        // decoder non può coinvolgere il contenuto.
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .timeout(HTTP_DOWNLOAD_TIMEOUT);
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let response = request.send().await?;
    // Il file `.part` era già completo: il server risponde 416 e basta rinominarlo.
    if offset > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        std::fs::rename(&temporary, &destination)?;
        return Ok(destination);
    }
    let mut response = response.error_for_status()?;
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"))
    {
        anyhow::bail!("comic download returned HTML instead of a file");
    }
    // Riprende solo se il server conferma la Range (206); altrimenti riparte da zero.
    let resumed = offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if offset > 0 && !resumed {
        std::fs::remove_file(&temporary).ok();
    }
    let mut file = if resumed {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&temporary)
            .await?
    } else {
        tokio::fs::File::create(&temporary).await?
    };
    let base = if resumed { offset } else { 0 };
    let total = response.content_length().map(|value| base + value);
    update_http_download(id, |download| {
        download.total_bytes = total;
        download.downloaded_bytes = base;
    });
    let started = Instant::now();
    let mut downloaded = base;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        let speed =
            ((downloaded - base) as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
        update_http_download(id, |download| {
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
            post_url TEXT DEFAULT '',
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
    let columns = conn
        .prepare("PRAGMA table_info(comics_monitored)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|name| name == "post_url") {
        conn.execute(
            "ALTER TABLE comics_monitored ADD COLUMN post_url TEXT DEFAULT ''",
            [],
        )?;
    }
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
    fn cleans_comic_search_title() {
        assert_eq!(clean_search_title("Poison Ivy #41 (2026)"), "Poison Ivy");
        assert_eq!(clean_search_title("Spawn #373 (2026)"), "Spawn");
        assert_eq!(clean_search_title("Batman (2026)"), "Batman");
        assert_eq!(
            clean_search_title("The Amazing Spider-Man #1"),
            "The Amazing Spider Man"
        );
        assert_eq!(clean_search_title(""), "");
    }

    #[test]
    fn monitored_issue_does_not_accept_newer_issue_or_collection() {
        assert_eq!(issue_number("Poison Ivy #41 (2026)"), Some("41".into()));
        assert!(matches_monitored_issue(
            "Poison Ivy #41 (2026)",
            "Poison Ivy #41 (2026)"
        ));
        assert!(!matches_monitored_issue(
            "Poison Ivy #41 (2026)",
            "Poison Ivy #47 (2026)"
        ));
        assert!(!matches_monitored_issue(
            "Poison Ivy #41 (2026)",
            "Poison Ivy Vol. 7 – Amuse-Bouche (TPB) (2026)"
        ));
        assert!(matches_monitored_issue("Batman (2026)", "Batman Vol. 1 (2026)"));
    }

    #[test]
    fn search_results_keep_the_selected_post_metadata() {
        let html = r#"
            <article class="post">
              <h2 class="post-title"><a href="/dc/poison-ivy-41-2026/">Poison Ivy #41 (2026)</a></h2>
              <img class="wp-post-image" src="/cover.jpg">
              <time datetime="2026-02-04">February 4, 2026</time>
              <div class="post-info"><p>Digital comic description.</p></div>
              <a href="/tag/poison-ivy-41/">Poison Ivy #41</a>
              <a href="/cat/dc/">DC Comics</a>
            </article>
        "#;
        let posts = parse_articles(html, "", "https://getcomics.org/?s=Poison+Ivy");
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].url, "https://getcomics.org/dc/poison-ivy-41-2026/");
        assert_eq!(posts[0].tag_url, "https://getcomics.org/tag/poison-ivy-41/");
        assert_eq!(posts[0].publisher, "DC Comics");
        assert_eq!(posts[0].description, "Digital comic description.");
    }

    #[tokio::test]
    #[ignore]
    async fn debug_weekly_links_network() {
        let client = GetComicsClient::new();
        for date in ["2026-09-16", "2026-09-23", "2026-09-09"] {
            match client.weekly_links(date).await {
                Ok((url, links)) => println!(
                    "{date}: OK {url} magnets={} torrents={} mega={} direct={}",
                    links.magnets.len(),
                    links.torrents.len(),
                    links.mega.len(),
                    links.direct.len()
                ),
                Err(error) => println!("{date}: ERR {error}"),
            }
        }
    }

    #[test]
    fn parse_links_ignores_invalid_hrefs() {
        // Un href non valido non deve far fallire il parsing degli altri link.
        let html = r#"<a href="magnet:?xt=urn:btih:0123456789012345678901234567890123456789">ok</a><a href="http://[bad">x</a><a href="/file.torrent">t</a>"#;
        let links = parse_links(html, "https://getcomics.org/post/").expect("parse");
        assert_eq!(links.magnets.len(), 1);
        assert_eq!(links.torrents.len(), 1);
    }

    #[test]
    fn parse_links_skips_help_page_and_keeps_getcomics_file_redirect() {
        let html = r#"
            <a href="https://getcomics.info/how-to-download/">how-to download page</a>
            <a href="https://getcomics.org/dls/pixeldrain-token">PIXELDRAIN</a>
            <a href="https://datanodes.to/file/example.cbr">DATANODES</a>
        "#;
        let links = parse_links(html, "https://getcomics.org/post/example").expect("parse");
        assert_eq!(links.direct, vec![
            "https://getcomics.org/dls/pixeldrain-token",
            "https://datanodes.to/file/example.cbr",
        ]);
    }

    #[test]
    fn parse_links_identifies_download_now_button() {
        let html = r#"
            <a href="" title="Download Now">DOWNLOAD NOW</a>
            <a href="/dls/download-now-token">Download Now</a>
            <a href="/dls/download-now-title" title="Download Now">file host</a>
            <a href="https://datanodes.to/file/example.cbr">Alternative download</a>
        "#;
        let links = parse_links(html, "https://getcomics.org/post/example").expect("parse");
        assert_eq!(
            links.download_now,
            vec![
                "https://getcomics.org/dls/download-now-token",
                "https://getcomics.org/dls/download-now-title"
            ]
        );
        assert_eq!(links.direct.len(), 3);
    }

    /// Riproduce lo stream troncato dietro Cloudflare ("error decoding response
    /// body"): il server dichiara la lunghezza piena ma chiude a metà. Il retry
    /// deve riprendere dal file `.part` con una Range request, non da zero.
    #[tokio::test]
    async fn download_http_resumes_after_truncated_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let payload: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served_payload = payload.clone();
        let server = tokio::spawn(async move {
            let mut connections = 0usize;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0_u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase();
                let offset = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .and_then(|value| value.split('-').next())
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if offset >= served_payload.len() {
                    let _ = socket
                        .write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    continue;
                }
                connections += 1;
                // La lunghezza dichiarata è sempre quella piena: al primo giro il
                // server scrive solo metà corpo, come un peer che chiude a metà.
                let length = served_payload.len() - offset;
                let write_end = if connections == 1 {
                    offset + length / 2
                } else {
                    served_payload.len()
                };
                let status = if offset > 0 {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n"
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&served_payload[offset..write_end]).await;
                let _ = socket.shutdown().await;
            }
        });

        let dir = std::env::temp_dir().join(format!("rextto-comic-dl-{}", uuid::Uuid::new_v4()));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        let url = format!("http://{addr}/file.cbz");
        let path = download_http(&client, &url, &dir, "Resume Test")
            .await
            .expect("download should resume and complete");
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        assert!(!path.with_extension("part").exists());
        server.abort();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_weekly_fills_missing_links_and_reports_eligibility() {
        let path = std::env::temp_dir().join(format!(
            "rextto-comics-weekly-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = ComicsDb::open(&path).unwrap();
        // Riga registrata senza link: pack trovato ma torrent non ancora pronto.
        assert!(!db.upsert_weekly_links("2026-09-16", "", "").unwrap());
        assert!(db.pending_weekly().unwrap().is_empty());
        // Arriva il magnet: la riga esistente va riempita e diventa eleggibile.
        assert!(db
            .upsert_weekly_links(
                "2026-09-16",
                "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
                "",
            )
            .unwrap());
        assert_eq!(db.pending_weekly().unwrap().len(), 1);
        // Dopo l'invio non è più pending.
        db.mark_weekly_sent("2026-09-16").unwrap();
        assert!(db.pending_weekly().unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

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
