//! The in-memory view of the library: movies, shows, tags, and the filtered/sorted list on
//! screen.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::library::{Db, Movie, MovieId, Show, ShowId, Tag, TagId};

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

/// The sidebar's built-in collections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shelf {
    #[default]
    All,
    Movies,
    Shows,
    ContinueWatching,
    Unwatched,
    Watched,
    Missing,
}

impl Shelf {
    /// The sidebar's name; "All movies" only while the library has no shows.
    pub fn label(self, has_shows: bool) -> &'static str {
        match self {
            Self::All if has_shows => "All",
            Self::All => "All movies",
            Self::Movies => "Movies",
            Self::Shows => "Shows",
            Self::ContinueWatching => "Continue watching",
            Self::Unwatched => "Unwatched",
            Self::Watched => "Watched",
            Self::Missing => "Missing files",
        }
    }

    pub fn contains(self, movie: &Movie, missing: bool) -> bool {
        match self {
            Self::All | Self::Movies => true,
            Self::Shows => false,
            Self::ContinueWatching => movie.resume_pos.is_some() && !movie.watched,
            Self::Unwatched => !movie.watched,
            Self::Watched => movie.watched,
            Self::Missing => missing,
        }
    }

    pub fn contains_show(self, lib: &Library, show: &ShowEntry) -> bool {
        let mut episodes = lib.episodes(show);
        match self {
            Self::All | Self::Shows => true,
            Self::Movies => false,
            Self::ContinueWatching => lib.show_in_progress(show),
            Self::Unwatched => episodes.any(|m| !m.watched),
            Self::Watched => episodes.all(|m| m.watched),
            Self::Missing => episodes.any(|m| lib.missing.contains(&m.id)),
        }
    }
}

/// One item in the grid or list: a movie, or a show standing for all its episodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Index into [`Library::movies`].
    Movie(usize),
    /// Index into [`Library::shows`].
    Show(usize),
}

/// A show with its episodes.
#[derive(Clone, Debug)]
pub struct ShowEntry {
    pub show: Show,
    /// Indexes into [`Library::movies`], in season and episode order.
    pub episodes: Vec<usize>,
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
    pub shows: Vec<ShowEntry>,
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
        let shows = group_shows(db.all_shows()?, &movies);
        Ok(Self {
            movies,
            shows,
            missing,
            tags: db.all_tags()?,
            movie_tags: db.movie_tags()?,
        })
    }

    pub fn get(&self, id: MovieId) -> Option<&Movie> {
        self.movies.iter().find(|m| m.id == id)
    }

    pub fn show(&self, id: ShowId) -> Option<&ShowEntry> {
        self.shows.iter().find(|s| s.show.id == id)
    }

    pub fn episodes<'a>(&'a self, show: &'a ShowEntry) -> impl Iterator<Item = &'a Movie> + 'a {
        show.episodes.iter().map(|&i| &self.movies[i])
    }

    /// Started but not finished: an episode part-way through, or some episodes watched and
    /// some not.
    pub fn show_in_progress(&self, show: &ShowEntry) -> bool {
        let mut watched = false;
        let mut unwatched = false;
        for m in self.episodes(show) {
            if m.resume_pos.is_some() && !m.watched {
                return true;
            }
            watched |= m.watched;
            unwatched |= !m.watched;
        }
        watched && unwatched
    }

    /// The episode to play next, and where to resume it: the one last left part-way, else the
    /// one after the last watched, else the first.
    pub fn next_episode<'a>(&'a self, show: &'a ShowEntry) -> Option<(&'a Movie, Option<f64>)> {
        let episodes: Vec<&Movie> = self.episodes(show).collect();
        let last = episodes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.last_played.is_some() || m.watched)
            .max_by_key(|(i, m)| (m.last_played.unwrap_or(0), *i));
        let Some((at, last)) = last else {
            return episodes.first().map(|m| (*m, None));
        };
        if !last.watched {
            return Some((last, last.resume_pos));
        }
        episodes[at + 1..]
            .iter()
            .chain(&episodes[..at])
            .find(|m| !m.watched)
            .or(episodes.get(at + 1))
            .or(episodes.first())
            .map(|m| (*m, m.resume_pos.filter(|_| !m.watched)))
    }

    /// The episode after `movie` in its show, if any.
    pub fn episode_after(&self, movie: &Movie) -> Option<&Movie> {
        let show = self.show(movie.show_id?)?;
        let at = show
            .episodes
            .iter()
            .position(|&i| self.movies[i].id == movie.id)?;
        show.episodes.get(at + 1).map(|&i| &self.movies[i])
    }

    pub fn has_shows(&self) -> bool {
        !self.shows.is_empty()
    }

    /// Whether any episode of the show has the tag.
    pub fn show_has_tag(&self, show: &ShowEntry, tag: TagId) -> bool {
        self.episodes(show).any(|m| self.has_tag(m.id, tag))
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

    /// Movies and shows on a shelf; a show counts once.
    pub fn shelf_count(&self, shelf: Shelf) -> usize {
        let movies = self
            .movies
            .iter()
            .filter(|m| m.show_id.is_none() && shelf.contains(m, self.missing.contains(&m.id)))
            .count();
        let shows = self
            .shows
            .iter()
            .filter(|s| shelf.contains_show(self, s))
            .count();
        movies + shows
    }

    /// Movies and episodes with the tag.
    pub fn tag_count(&self, tag: TagId) -> usize {
        self.movie_tags
            .values()
            .filter(|t| t.contains(&tag))
            .count()
    }

    /// Display title of an entry.
    pub fn entry_title(&self, entry: Entry) -> &str {
        match entry {
            Entry::Movie(i) => &self.movies[i].title,
            Entry::Show(i) => &self.shows[i].show.title,
        }
    }
}

/// Puts each show's episodes in order; shows without episodes are left out.
fn group_shows(shows: Vec<Show>, movies: &[Movie]) -> Vec<ShowEntry> {
    let mut by_show: HashMap<ShowId, Vec<usize>> = HashMap::new();
    for (i, m) in movies.iter().enumerate() {
        if let Some(show) = m.show_id {
            by_show.entry(show).or_default().push(i);
        }
    }
    shows
        .into_iter()
        .filter_map(|show| {
            let mut episodes = by_show.remove(&show.id)?;
            episodes.sort_by_key(|&i| (movies[i].season, movies[i].episode, movies[i].id));
            Some(ShowEntry { show, episodes })
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub search: String,
    pub shelf: Shelf,
    pub tag: Option<TagId>,
    pub sort: SortKey,
    pub descending: bool,
}

/// Lowercase without accents, so "amelie" finds "Amélie".
pub fn fold(text: &str) -> String {
    use unicode_normalization::{UnicodeNormalization as _, char::is_combining_mark};
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Title without a leading article or accents, lowercased: "The Matrix" sorts under M.
pub fn sort_title(title: &str) -> String {
    let lower = fold(title.trim());
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.to_string();
        }
    }
    lower
}

/// What sorting compares for an entry.
struct SortFields {
    title: String,
    year: Option<i32>,
    size: u64,
    date_added: i64,
    last_played: Option<i64>,
}

fn sort_fields(lib: &Library, entry: Entry) -> SortFields {
    match entry {
        Entry::Movie(i) => {
            let m = &lib.movies[i];
            SortFields {
                title: sort_title(&m.title),
                year: m.year,
                size: m.size,
                date_added: m.date_added,
                last_played: m.last_played,
            }
        }
        Entry::Show(i) => {
            let s = &lib.shows[i];
            let episodes = || lib.episodes(s);
            SortFields {
                title: sort_title(&s.show.title),
                year: s.show.year,
                size: episodes().map(|m| m.size).sum(),
                date_added: episodes().map(|m| m.date_added).max().unwrap_or(0),
                last_played: episodes().filter_map(|m| m.last_played).max(),
            }
        }
    }
}

/// The movies and shows that match `filter`, in display order. Episodes appear through their
/// show.
pub fn visible_entries(lib: &Library, filter: &Filter) -> Vec<Entry> {
    let needle = fold(filter.search.trim());
    let year_matches = |year: Option<i32>| year.is_some_and(|y| y.to_string() == needle);
    let movies = lib
        .movies
        .iter()
        .enumerate()
        .filter(|(_, m)| m.show_id.is_none())
        .filter(|(_, m)| {
            needle.is_empty() || fold(&m.title).contains(&needle) || year_matches(m.year)
        })
        .filter(|(_, m)| filter.tag.is_none_or(|t| lib.has_tag(m.id, t)))
        .filter(|(_, m)| filter.shelf.contains(m, lib.missing.contains(&m.id)))
        .map(|(i, _)| Entry::Movie(i));
    let shows = lib
        .shows
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            needle.is_empty()
                || fold(&s.show.title).contains(&needle)
                || year_matches(s.show.year)
                || lib.episodes(s).any(|m| fold(&m.title).contains(&needle))
        })
        .filter(|(_, s)| filter.tag.is_none_or(|t| lib.show_has_tag(s, t)))
        .filter(|(_, s)| filter.shelf.contains_show(lib, s))
        .map(|(i, _)| Entry::Show(i));
    let mut entries: Vec<(Entry, SortFields)> = movies
        .chain(shows)
        .map(|e| (e, sort_fields(lib, e)))
        .collect();
    entries.sort_by(|(_, a), (_, b)| {
        let primary = match filter.sort {
            SortKey::Title => a.title.cmp(&b.title),
            SortKey::Year => a.year.cmp(&b.year),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::DateAdded => a.date_added.cmp(&b.date_added),
            SortKey::LastPlayed => a.last_played.cmp(&b.last_played),
        };
        let primary = if filter.descending {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| a.title.cmp(&b.title))
    });
    entries.into_iter().map(|(e, _)| e).collect()
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
            show_id: None,
            season: None,
            episode: None,
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
        visible_entries(lib, f)
            .into_iter()
            .map(|e| lib.entry_title(e).to_string())
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

    /// A library with the three movies and a show with two seasons.
    fn with_show() -> Library {
        let mut lib = lib();
        for (id, season, episode) in [(10, 1, 1), (11, 1, 2), (12, 2, 1)] {
            let mut ep = movie(id, &format!("Episode {episode}"), 2020, 5);
            ep.year = None;
            ep.show_id = Some(1);
            ep.season = Some(season);
            ep.episode = Some(episode);
            lib.movies.push(ep);
        }
        let show = Show {
            id: 1,
            title: "Dark".into(),
            year: Some(2017),
            plot: None,
            poster: None,
            tmdb_id: None,
        };
        lib.shows = group_shows(vec![show], &lib.movies);
        lib
    }

    #[test]
    fn episodes_appear_through_their_show() {
        let lib = with_show();
        assert_eq!(
            titles(&lib, &Filter::default()),
            ["Alien", "Dark", "Heat", "The Matrix"]
        );
        let shelf = |shelf| {
            titles(
                &lib,
                &Filter {
                    shelf,
                    ..Filter::default()
                },
            )
        };
        assert_eq!(shelf(Shelf::Shows), ["Dark"]);
        assert_eq!(shelf(Shelf::Movies), ["Alien", "Heat", "The Matrix"]);
        assert_eq!(lib.shelf_count(Shelf::All), 4);
        // Searching an episode's title finds its show.
        let mut lib = lib;
        lib.movies[4].title = "Lost and Found".into();
        assert_eq!(
            titles(
                &lib,
                &Filter {
                    search: "found".into(),
                    ..Filter::default()
                }
            ),
            ["Dark"]
        );
    }

    #[test]
    fn next_episode_follows_what_was_watched() {
        let mut lib = with_show();
        let next = |lib: &Library| {
            let (m, resume) = lib.next_episode(&lib.shows[0]).unwrap();
            (m.id, resume)
        };
        assert_eq!(next(&lib), (10, None), "starts at the first episode");

        lib.movies[3].watched = true;
        lib.movies[3].last_played = Some(100);
        assert_eq!(
            next(&lib),
            (11, None),
            "then the one after the last watched"
        );
        assert!(lib.show_in_progress(&lib.shows[0]));

        lib.movies[4].last_played = Some(200);
        lib.movies[4].resume_pos = Some(42.0);
        assert_eq!(next(&lib), (11, Some(42.0)), "resumes one left part-way");

        lib.movies[4].watched = true;
        lib.movies[4].resume_pos = None;
        assert_eq!(next(&lib), (12, None), "on into the next season");
        assert_eq!(lib.episode_after(&lib.movies[4]).map(|m| m.id), Some(12));
        assert_eq!(lib.episode_after(&lib.movies[5]).map(|m| m.id), None);

        lib.movies[5].watched = true;
        lib.movies[5].last_played = Some(300);
        assert!(!lib.show_in_progress(&lib.shows[0]));
        assert!(Shelf::Watched.contains_show(&lib, &lib.shows[0]));
        assert_eq!(next(&lib), (10, None), "all watched: from the start");
    }

    #[test]
    fn shelves() {
        let mut lib = lib();
        lib.movies[0].resume_pos = Some(100.0);
        lib.movies[1].watched = true;
        lib.missing.insert(3);
        let shelf = |shelf| {
            titles(
                &lib,
                &Filter {
                    shelf,
                    ..Filter::default()
                },
            )
        };
        assert_eq!(shelf(Shelf::ContinueWatching), ["The Matrix"]);
        assert_eq!(shelf(Shelf::Watched), ["Alien"]);
        assert_eq!(shelf(Shelf::Unwatched), ["Heat", "The Matrix"]);
        assert_eq!(shelf(Shelf::Missing), ["Heat"]);
        assert_eq!(lib.shelf_count(Shelf::All), 3);
    }
}
