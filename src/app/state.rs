//! The in-memory view of the library: movies, tags, and the filtered/sorted list on screen.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::library::{Db, Movie, MovieId, Tag, TagId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Title,
    Year,
    Size,
    DateAdded,
    LastPlayed,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        Self::Title,
        Self::Year,
        Self::Size,
        Self::DateAdded,
        Self::LastPlayed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Year => "Year",
            Self::Size => "Size",
            Self::DateAdded => "Date added",
            Self::LastPlayed => "Last played",
        }
    }

    /// The direction that feels natural when first picking this key.
    pub fn default_descending(self) -> bool {
        !matches!(self, Self::Title)
    }
}

/// UI preferences remembered between runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub layout: Layout,
    pub sort: SortKey,
    pub descending: bool,
}

#[derive(Default)]
pub struct Library {
    pub movies: Vec<Movie>,
    pub missing: HashSet<MovieId>,
    pub tags: Vec<Tag>,
    pub movie_tags: HashMap<MovieId, BTreeSet<TagId>>,
}

impl Library {
    pub fn load(db: &Db) -> anyhow::Result<Self> {
        let movies = db.all_movies()?;
        let missing = movies
            .iter()
            .filter(|m| !m.path.exists())
            .map(|m| m.id)
            .collect();
        Ok(Self {
            movies,
            missing,
            tags: db.all_tags()?,
            movie_tags: db.movie_tags()?,
        })
    }

    pub fn get(&self, id: MovieId) -> Option<&Movie> {
        self.movies.iter().find(|m| m.id == id)
    }

    pub fn has_tag(&self, movie: MovieId, tag: TagId) -> bool {
        self.movie_tags
            .get(&movie)
            .is_some_and(|t| t.contains(&tag))
    }

    pub fn tag_names(&self, movie: MovieId) -> Vec<&str> {
        self.tags
            .iter()
            .filter(|t| self.has_tag(movie, t.id))
            .map(|t| t.name.as_str())
            .collect()
    }

    pub fn total_size(&self) -> u64 {
        self.movies.iter().map(|m| m.size).sum()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub search: String,
    pub tag: Option<TagId>,
    pub sort: SortKey,
    pub descending: bool,
}

/// Title without a leading article, lowercased: "The Matrix" sorts under M.
pub fn sort_title(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.to_string();
        }
    }
    lower
}

/// Indexes into `lib.movies` that match `filter`, in display order.
pub fn visible_movies(lib: &Library, filter: &Filter) -> Vec<usize> {
    let needle = filter.search.trim().to_lowercase();
    let mut idx: Vec<usize> = lib
        .movies
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            needle.is_empty()
                || m.title.to_lowercase().contains(&needle)
                || m.year.is_some_and(|y| y.to_string() == needle)
        })
        .filter(|(_, m)| filter.tag.is_none_or(|t| lib.has_tag(m.id, t)))
        .map(|(i, _)| i)
        .collect();

    let titles: Vec<String> = lib.movies.iter().map(|m| sort_title(&m.title)).collect();
    idx.sort_by(|&a, &b| {
        let (ma, mb) = (&lib.movies[a], &lib.movies[b]);
        let primary = match filter.sort {
            SortKey::Title => titles[a].cmp(&titles[b]),
            SortKey::Year => ma.year.cmp(&mb.year),
            SortKey::Size => ma.size.cmp(&mb.size),
            SortKey::DateAdded => ma.date_added.cmp(&mb.date_added),
            SortKey::LastPlayed => ma.last_played.cmp(&mb.last_played),
        };
        let primary = if filter.descending {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| titles[a].cmp(&titles[b]))
    });
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn movie(id: MovieId, title: &str, year: i32, size: u64) -> Movie {
        Movie {
            id,
            path: PathBuf::from(format!("/m/{id}.mkv")),
            title: title.into(),
            year: Some(year),
            plot: None,
            size,
            duration: None,
            video_codec: None,
            width: None,
            height: None,
            audio_tracks: vec![],
            subtitle_tracks: vec![],
            fingerprint: String::new(),
            date_added: id,
            last_played: None,
            resume_pos: None,
            watched: false,
            poster: None,
            tmdb_id: None,
        }
    }

    fn lib() -> Library {
        let mut lib = Library {
            movies: vec![
                movie(1, "The Matrix", 1999, 30),
                movie(2, "Alien", 1979, 10),
                movie(3, "Heat", 1995, 20),
            ],
            ..Library::default()
        };
        lib.movie_tags.entry(2).or_default().insert(7);
        lib
    }

    fn titles(lib: &Library, f: &Filter) -> Vec<String> {
        visible_movies(lib, f)
            .into_iter()
            .map(|i| lib.movies[i].title.clone())
            .collect()
    }

    #[test]
    fn title_sort_ignores_leading_articles() {
        let f = Filter::default();
        assert_eq!(titles(&lib(), &f), ["Alien", "Heat", "The Matrix"]);
    }

    #[test]
    fn sorts_by_size_descending() {
        let f = Filter {
            sort: SortKey::Size,
            descending: true,
            ..Filter::default()
        };
        assert_eq!(titles(&lib(), &f), ["The Matrix", "Heat", "Alien"]);
    }

    #[test]
    fn search_and_tag_filters() {
        let lib = lib();
        assert_eq!(
            titles(
                &lib,
                &Filter {
                    search: "MAT".into(),
                    ..Filter::default()
                }
            ),
            ["The Matrix"]
        );
        assert_eq!(
            titles(
                &lib,
                &Filter {
                    search: "1995".into(),
                    ..Filter::default()
                }
            ),
            ["Heat"]
        );
        assert_eq!(
            titles(
                &lib,
                &Filter {
                    tag: Some(7),
                    ..Filter::default()
                }
            ),
            ["Alien"]
        );
    }
}
