use anyhow::Result;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct TmdbClient {
    client: Client,
    key: Option<String>,
    language: String,
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub results: Vec<TmdbItem>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbItem {
    pub id: i64,
    pub name: Option<String>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub first_air_date: Option<String>,
    pub release_date: Option<String>,
    pub vote_average: Option<f64>,
}
#[derive(Debug, Deserialize)]
struct EpisodeResult {
    name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastMember {
    pub name: Option<String>,
    pub character: Option<String>,
    pub order: Option<i64>,
}
#[derive(Debug, Deserialize)]
struct CreditsResponse {
    #[serde(default)]
    cast: Vec<CastMember>,
}
#[derive(Debug, Deserialize)]
struct SeriesDetails {
    seasons: Vec<SeasonSummary>,
    next_episode_to_air: Option<TmdbEpisode>,
}
#[derive(Debug, Deserialize)]
struct SeasonSummary {
    season_number: Option<i64>,
    episode_count: Option<i64>,
}
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct TmdbEpisode {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub season_number: Option<i64>,
    pub episode_number: Option<i64>,
    pub air_date: Option<String>,
}

impl TmdbClient {
    pub fn new(key: Option<String>) -> Self {
        Self::with_language(key, "it-IT".to_string())
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
                "it-IT".to_string()
            } else {
                language
            },
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str, params: &[(&str, &str)]) -> Result<T> {
        let key = self
            .key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("TMDB API key is not configured"))?;
        for attempt in 0..3 {
            let response = self
                .client
                .get(url)
                .query(params)
                .query(&[("api_key", key)])
                .send()
                .await?;
            if response.status().is_success() {
                return Ok(response.json::<T>().await?);
            }
            let retryable =
                response.status().as_u16() == 429 || response.status().is_server_error();
            if !retryable || attempt == 2 {
                return Err(response.error_for_status().unwrap_err().into());
            }
            tokio::time::sleep(Duration::from_millis(250 * (attempt + 1) as u64)).await;
        }
        unreachable!()
    }

    pub async fn search_series(&self, name: &str) -> Result<Vec<TmdbItem>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        Ok(self
            .get_json::<SearchResult>(
                "https://api.themoviedb.org/3/search/tv",
                &[("query", name), ("language", self.language.as_str())],
            )
            .await?
            .results)
    }

    pub async fn search_movies(&self, name: &str) -> Result<Vec<TmdbItem>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        Ok(self
            .get_json::<SearchResult>(
                "https://api.themoviedb.org/3/search/movie",
                &[("query", name), ("language", self.language.as_str())],
            )
            .await?
            .results)
    }

    pub async fn trending(&self, kind: &str, window: &str) -> Result<Vec<TmdbItem>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        let media = if kind == "movie" { "movie" } else { "tv" };
        let window = if window == "day" { "day" } else { "week" };
        let url = format!("https://api.themoviedb.org/3/trending/{media}/{window}");
        Ok(self
            .get_json::<SearchResult>(&url, &[("language", self.language.as_str())])
            .await?
            .results)
    }

    pub async fn popular(&self, kind: &str) -> Result<Vec<TmdbItem>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        let path = if kind == "movie" {
            "movie/popular"
        } else {
            "tv/popular"
        };
        let url = format!("https://api.themoviedb.org/3/{path}");
        Ok(self
            .get_json::<SearchResult>(&url, &[("language", self.language.as_str())])
            .await?
            .results)
    }

    /// Curated TMDB categories used by the discovery view. TV has no exact
    /// counterpart for the two movie-only categories, so use the closest
    /// official TV endpoints instead of silently returning movie results.
    pub async fn category(&self, kind: &str, category: &str) -> Result<Vec<TmdbItem>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        let path = category_path(kind, category)
            .ok_or_else(|| anyhow::anyhow!("unsupported TMDB category: {category}"))?;
        let url = format!("https://api.themoviedb.org/3/{path}");
        Ok(self
            .get_json::<SearchResult>(&url, &[("language", self.language.as_str())])
            .await?
            .results)
    }

    pub async fn search_movie(&self, name: &str, year: Option<i64>) -> Result<Option<TmdbItem>> {
        if self.key.is_none() {
            return Ok(None);
        }
        let year_value = year.map(|value| value.to_string());
        let mut params = vec![("query", name), ("language", self.language.as_str())];
        if let Some(value) = year_value.as_deref() {
            params.push(("year", value));
        }
        Ok(self
            .get_json::<SearchResult>("https://api.themoviedb.org/3/search/movie", &params)
            .await?
            .results
            .into_iter()
            .next())
    }

    /// Cast principale di un film (massimo 12 membri, ordinati per rilevanza).
    pub async fn movie_credits(&self, tmdb_id: &str) -> Result<Vec<CastMember>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        let Some(id) = tmdb_id.trim().parse::<i64>().ok() else {
            return Ok(Vec::new());
        };
        let response = self
            .get_json::<CreditsResponse>(
                &format!("https://api.themoviedb.org/3/movie/{id}/credits"),
                &[("language", self.language.as_str())],
            )
            .await?;
        let mut cast = response.cast;
        cast.sort_by_key(|member| member.order.unwrap_or(i64::MAX));
        cast.truncate(12);
        Ok(cast)
    }

    pub async fn resolve_series_id(&self, name: &str) -> Result<Option<String>> {
        let cache_key = format!("series:{name}");
        if let Some(value) = self.cache.lock().unwrap().get(&cache_key).cloned() {
            return Ok(value);
        }
        let result = self
            .search_series(name)
            .await?
            .into_iter()
            .next()
            .map(|item| item.id.to_string());
        self.cache.lock().unwrap().insert(cache_key, result.clone());
        Ok(result)
    }

    pub async fn poster_for_series(&self, name: &str) -> Result<Option<String>> {
        if self.key.is_none() {
            return Ok(None);
        }
        // I poster cambiano raramente: cache per evitare una ricerca TMDB ad
        // ogni elenco (calendario, ultimi download, ...).
        let cache_key = format!("poster:{name}");
        if let Some(value) = self.cache.lock().unwrap().get(&cache_key).cloned() {
            return Ok(value);
        }
        let result = self
            .search_series(name)
            .await?
            .into_iter()
            .next()
            .and_then(|item| item.poster_path);
        self.cache.lock().unwrap().insert(cache_key, result.clone());
        Ok(result)
    }

    /// Extended TV metadata for the series detail header (name, overview, poster,
    /// network, year, seasons). Uses the stored TMDB id when available, otherwise
    /// resolves it by name.
    pub async fn series_info(
        &self,
        name: &str,
        tmdb_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>> {
        if self.key.is_none() {
            return Ok(None);
        }
        // Use the stored id only when it is a numeric TMDB id; otherwise (empty
        // or a TVDB id) resolve the series by name.
        let numeric = tmdb_id
            .map(str::trim)
            .filter(|value| value.parse::<i64>().is_ok())
            .map(str::to_string);
        let id = match numeric {
            Some(id) => id,
            None => match self.resolve_series_id(name).await? {
                Some(id) => id,
                None => return Ok(None),
            },
        };
        let value: serde_json::Value = self
            .get_json(
                &format!("https://api.themoviedb.org/3/tv/{id}"),
                &[("language", self.language.as_str())],
            )
            .await?;
        Ok(Some(value))
    }

    /// Main cast of a TV series (up to 8 members) as `(tmdb_person_id, name)`.
    pub async fn series_cast(&self, tmdb_id: &str) -> Result<Vec<(i64, String)>> {
        if self.key.is_none() {
            return Ok(Vec::new());
        }
        let Ok(id) = tmdb_id.trim().parse::<i64>() else {
            return Ok(Vec::new());
        };
        #[derive(Deserialize)]
        struct Credits {
            #[serde(default)]
            cast: Vec<CastEntry>,
        }
        #[derive(Deserialize)]
        struct CastEntry {
            id: i64,
            name: Option<String>,
            order: Option<i64>,
        }
        let response: Credits = self
            .get_json(
                &format!("https://api.themoviedb.org/3/tv/{id}/aggregate_credits"),
                &[("language", self.language.as_str())],
            )
            .await?;
        let mut cast = response.cast;
        cast.sort_by_key(|member| member.order.unwrap_or(i64::MAX));
        Ok(cast
            .into_iter()
            .filter_map(|member| member.name.map(|name| (member.id, name)))
            .take(8)
            .collect())
    }

    pub async fn episode_title(
        &self,
        tmdb_id: &str,
        season: i64,
        episode: i64,
    ) -> Result<Option<String>> {
        if self.key.is_none() {
            return Ok(None);
        }
        let id = tmdb_id.trim().parse::<i64>().ok();
        let Some(id) = id else {
            return Ok(None);
        };
        let cache_key = format!("episode:{id}:{season}:{episode}");
        if let Some(value) = self.cache.lock().unwrap().get(&cache_key).cloned() {
            return Ok(value);
        }
        let result = self
            .get_json::<EpisodeResult>(
                &format!("https://api.themoviedb.org/3/tv/{id}/season/{season}/episode/{episode}"),
                &[("language", self.language.as_str())],
            )
            .await?
            .name
            .filter(|name| !name.trim().is_empty());
        self.cache.lock().unwrap().insert(cache_key, result.clone());
        Ok(result)
    }

    pub async fn season_counts(&self, tmdb_id: &str) -> Result<HashMap<i64, i64>> {
        if self.key.is_none() {
            return Ok(HashMap::new());
        }
        let id = tmdb_id.trim().parse::<i64>().ok();
        let Some(id) = id else {
            return Ok(HashMap::new());
        };
        let details = self
            .get_json::<SeriesDetails>(
                &format!("https://api.themoviedb.org/3/tv/{id}"),
                &[("language", self.language.as_str())],
            )
            .await?;
        Ok(details
            .seasons
            .into_iter()
            .filter_map(|season| Some((season.season_number?, season.episode_count?)))
            .collect())
    }

    pub async fn next_episode(&self, tmdb_id: &str) -> Result<Option<TmdbEpisode>> {
        if self.key.is_none() {
            return Ok(None);
        }
        let id = tmdb_id.trim().parse::<i64>().ok();
        let Some(id) = id else {
            return Ok(None);
        };
        Ok(self
            .get_json::<SeriesDetails>(
                &format!("https://api.themoviedb.org/3/tv/{id}"),
                &[("language", self.language.as_str())],
            )
            .await?
            .next_episode_to_air)
    }
}

fn category_path(kind: &str, category: &str) -> Option<&'static str> {
    let movie = kind.eq_ignore_ascii_case("movie");
    match (movie, category) {
        (true, "top_rated") => Some("movie/top_rated"),
        (false, "top_rated") => Some("tv/top_rated"),
        (true, "now_playing") => Some("movie/now_playing"),
        (false, "now_playing") => Some("tv/on_the_air"),
        (true, "upcoming") => Some("movie/upcoming"),
        (false, "upcoming") => Some("tv/airing_today"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::category_path;

    #[test]
    fn maps_curated_categories_to_the_correct_media_endpoints() {
        assert_eq!(category_path("movie", "top_rated"), Some("movie/top_rated"));
        assert_eq!(category_path("series", "top_rated"), Some("tv/top_rated"));
        assert_eq!(
            category_path("movie", "now_playing"),
            Some("movie/now_playing")
        );
        assert_eq!(
            category_path("series", "now_playing"),
            Some("tv/on_the_air")
        );
        assert_eq!(category_path("movie", "upcoming"), Some("movie/upcoming"));
        assert_eq!(category_path("series", "upcoming"), Some("tv/airing_today"));
        assert_eq!(category_path("movie", "unknown"), None);
    }
}
