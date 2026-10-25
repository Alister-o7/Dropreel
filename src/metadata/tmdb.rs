//! Optional metadata lookup on The Movie Database (TMDb).

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

const API: &str = "https://api.themoviedb.org/3";
/// Posters 780 pixels wide: sharp on high-DPI screens without TMDb's multi-megabyte originals.
const IMAGES: &str = "https://image.tmdb.org/t/p/w780";

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TmdbMovie {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub poster_path: Option<String>,
}

impl TmdbMovie {
    pub fn year(&self) -> Option<i32> {
        self.release_date.as_deref()?.get(..4)?.parse().ok()
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<TmdbMovie>,
}

pub struct TmdbClient {
    http: reqwest::blocking::Client,
    key: String,
}

impl TmdbClient {
    pub fn new(key: &str) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("DropReel/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            key: key.trim().to_string(),
        })
    }

    /// v4 "API read access tokens" are JWTs sent as a bearer token; v3 keys go in the query.
    fn get(&self, url: &str) -> reqwest::blocking::RequestBuilder {
        let req = self.http.get(url);
        if self.key.starts_with("eyJ") {
            req.bearer_auth(&self.key)
        } else {
            req.query(&[("api_key", self.key.as_str())])
        }
    }

    pub fn search(&self, query: &str, year: Option<i32>) -> Result<Vec<TmdbMovie>, String> {
        let mut req = self
            .get(&format!("{API}/search/movie"))
            .query(&[("query", query), ("include_adult", "false")]);
        if let Some(year) = year {
            req = req.query(&[("year", year.to_string())]);
        }
        let resp = req
            .send()
            .map_err(|e| format!("TMDb request failed: {e}"))?;
        match resp.status().as_u16() {
            200 => {}
            401 => return Err("TMDb rejected the API key. Check it in Settings.".into()),
            code => return Err(format!("TMDb returned HTTP {code}")),
        }
        let body: SearchResponse = resp
            .json()
            .map_err(|e| format!("unexpected TMDb response: {e}"))?;
        Ok(body.results)
    }

    /// Searches with the year first, then without it if nothing matched.
    pub fn find_best(&self, title: &str, year: Option<i32>) -> Result<Option<TmdbMovie>, String> {
        let mut results = self.search(title, year)?;
        if results.is_empty() && year.is_some() {
            results = self.search(title, None)?;
        }
        Ok(best_match(results, year))
    }

    /// The current poster of a movie TMDb already matched.
    pub fn poster_path(&self, tmdb_id: i64) -> Result<Option<String>, String> {
        let resp = self
            .get(&format!("{API}/movie/{tmdb_id}"))
            .send()
            .map_err(|e| format!("TMDb request failed: {e}"))?;
        match resp.status().as_u16() {
            200 => {}
            401 => return Err("TMDb rejected the API key. Check it in Settings.".into()),
            code => return Err(format!("TMDb returned HTTP {code}")),
        }
        let movie: TmdbMovie = resp
            .json()
            .map_err(|e| format!("unexpected TMDb response: {e}"))?;
        Ok(movie.poster_path)
    }

    pub fn download_poster(&self, poster_path: &str, out: &Path) -> Result<(), String> {
        let resp = self
            .http
            .get(format!("{IMAGES}{poster_path}"))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("poster download failed: {e}"))?;
        let bytes = resp
            .bytes()
            .map_err(|e| format!("poster download failed: {e}"))?;
        std::fs::write(out, &bytes).map_err(|e| format!("could not save poster: {e}"))
    }
}

/// Prefers a result released in `year`, then TMDb's own ranking.
pub fn best_match(results: Vec<TmdbMovie>, year: Option<i32>) -> Option<TmdbMovie> {
    if let Some(y) = year
        && let Some(hit) = results.iter().find(|m| m.year() == Some(y))
    {
        return Some(hit.clone());
    }
    results.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(id: i64, date: &str) -> TmdbMovie {
        TmdbMovie {
            id,
            title: format!("m{id}"),
            release_date: Some(date.into()),
            overview: None,
            poster_path: None,
        }
    }

    #[test]
    fn best_match_prefers_matching_year() {
        let results = vec![movie(1, "2005-01-01"), movie(2, "2019-06-01")];
        assert_eq!(best_match(results.clone(), Some(2019)).unwrap().id, 2);
        assert_eq!(best_match(results.clone(), Some(1990)).unwrap().id, 1);
        assert_eq!(best_match(results, None).unwrap().id, 1);
        assert!(best_match(vec![], None).is_none());
    }

    #[test]
    fn year_is_taken_from_release_date() {
        assert_eq!(movie(1, "1999-03-31").year(), Some(1999));
        assert_eq!(movie(1, "").year(), None);
    }
}
