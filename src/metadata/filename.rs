//! Guessing a movie's title and year from its file name, and spotting episodes of shows.
//!
//! `Movie.Name.2019.1080p.BluRay.x264-GRP.mkv` → ("Movie Name", Some(2019))
//! `Show.Name.S01E02.Pilot.1080p.mkv` → "Show Name", season 1, episode 2, "Pilot"

use std::path::Path;

use chrono::Datelike as _;

/// Tokens that mark the start of release junk (resolution, source, codec, …).
const JUNK: &[&str] = &[
    "480p",
    "576p",
    "720p",
    "1080p",
    "1080i",
    "2160p",
    "4320p",
    "4k",
    "8k",
    "uhd",
    "bluray",
    "blu-ray",
    "bdrip",
    "brrip",
    "bdremux",
    "remux",
    "webrip",
    "web-dl",
    "webdl",
    "web",
    "hdrip",
    "dvdrip",
    "dvdscr",
    "dvd",
    "hdtv",
    "hdcam",
    "x264",
    "x265",
    "h264",
    "h265",
    "h.264",
    "h.265",
    "hevc",
    "avc",
    "xvid",
    "divx",
    "10bit",
    "8bit",
    "hdr",
    "hdr10",
    "hdr10+",
    "dv",
    "atmos",
    "truehd",
    "dts",
    "dts-hd",
    "ac3",
    "eac3",
    "aac",
    "flac",
    "5.1",
    "7.1",
    "proper",
    "repack",
    "extended",
    "unrated",
    "remastered",
    "imax",
    "internal",
    "limited",
    "multi",
    "subbed",
    "dubbed",
    "directors",
    "director's",
    "theatrical",
];

/// Junk words that can also be ordinary title words ("Charlotte's Web"). They only end the
/// title when they come after the year.
const WEAK_JUNK: &[&str] = &["web", "dvd", "dv", "multi", "limited", "internal", "proper"];

pub fn parse_title_year(stem: &str) -> (String, Option<i32>) {
    parse_with_max_year(stem, chrono::Local::now().year() + 1)
}

fn core(token: &str) -> &str {
    token.trim_matches(|c: char| "()[]{}".contains(c))
}

fn as_year(token: &str, max_year: i32) -> Option<i32> {
    let c = core(token);
    if c.len() != 4 || !c.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = c.parse().ok()?;
    (1900..=max_year).contains(&year).then_some(year)
}

fn is_junk(token: &str, strong_only: bool) -> bool {
    let lower = core(token).to_lowercase();
    // Release group suffix like "x264-GROUP" or audio like "DDP5.1".
    let head = lower.split('-').next().unwrap_or(&lower);
    let matches = |s: &str| JUNK.contains(&s) && !(strong_only && WEAK_JUNK.contains(&s));
    matches(&lower)
        || matches(head)
        || lower.starts_with("ddp")
        || lower.starts_with("dd5")
        || is_bracket_tag(token)
}

fn is_bracket_tag(token: &str) -> bool {
    token.starts_with('[') && token.ends_with(']') && as_year(token, 9999).is_none()
}

fn parse_with_max_year(stem: &str, max_year: i32) -> (String, Option<i32>) {
    // Dot-separated names ("Movie.Name.2019") use dots as spaces; names with real spaces keep
    // their dots ("E.T. the Extra-Terrestrial").
    let spaced: String = if stem.contains(' ') {
        stem.replace('_', " ")
    } else {
        stem.replace(['.', '_'], " ")
    };
    let mut tokens: Vec<&str> = spaced.split_whitespace().collect();
    // Leading release-group tags like "[YTS]" are not part of the title.
    let leading_tags = tokens.iter().take_while(|t| is_bracket_tag(t)).count();
    if leading_tags < tokens.len() {
        tokens.drain(..leading_tags);
    }
    if tokens.is_empty() {
        return (stem.trim().to_string(), None);
    }

    // The year is the last year-like token (never the first word, so "1917" or "2001 A Space
    // Odyssey" stay titles) before any strong junk token.
    let first_strong_junk = tokens
        .iter()
        .position(|t| is_junk(t, true))
        .unwrap_or(tokens.len());
    let year_at = (1..first_strong_junk)
        .rev()
        .find(|&i| as_year(tokens[i], max_year).is_some());

    let (title_end, year) = match year_at {
        Some(i) => (i, as_year(tokens[i], max_year)),
        None => {
            let junk_at = tokens
                .iter()
                .position(|t| is_junk(t, false))
                .unwrap_or(tokens.len());
            (if junk_at == 0 { tokens.len() } else { junk_at }, None)
        }
    };

    let title = clean(&tokens[..title_end].join(" "));
    if title.is_empty() {
        (clean(&spaced), year)
    } else {
        (title, year)
    }
}

/// A file recognised as an episode of a show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeName {
    pub show: String,
    pub year: Option<i32>,
    pub season: u32,
    pub episode: u32,
    /// The episode's own title, when the file name has one.
    pub title: Option<String>,
}

/// Recognises `S01E02`, `S01.E02` and `1x02` episode names. The show's name comes from the file
/// name, or from its folder (skipping a "Season 1" folder) when the file name starts with the
/// episode number.
pub fn parse_episode(path: &Path) -> Option<EpisodeName> {
    let stem = path.file_stem()?.to_string_lossy();
    let tokens = tokenize(&stem);
    let (at, len, season, episode) = (0..tokens.len()).find_map(|i| {
        if let Some((s, e)) = episode_marker(tokens[i]) {
            return Some((i, 1, s, e));
        }
        let s = season_marker(tokens[i])?;
        let e = tokens.get(i + 1).and_then(|t| number_after(core(t), 'e'))?;
        Some((i, 2, s, e))
    })?;
    let max_year = chrono::Local::now().year() + 1;

    let (mut show, mut year) = parse_with_max_year(&tokens[..at].join(" "), max_year);
    if show.is_empty() {
        (show, year) = show_from_folders(path, max_year)?;
    }

    // What follows the marker, minus further episode numbers ("S01E01E02", "S01E01-E02").
    let rest: Vec<&str> = tokens[at + len..]
        .iter()
        .copied()
        .skip_while(|t| {
            let c = core(t).trim_start_matches('-');
            c.is_empty() || number_after(c, 'e').is_some()
        })
        .collect();
    let title = match rest.first() {
        Some(first) if !is_junk(first, false) => {
            let (title, _) = parse_with_max_year(&rest.join(" "), max_year);
            Some(title).filter(|t| !t.is_empty())
        }
        _ => None,
    };
    Some(EpisodeName {
        show,
        year,
        season,
        episode,
        title,
    })
}

/// Splits a name into words, treating dots and underscores as spaces when it has no spaces.
fn tokenize(stem: &str) -> Vec<&str> {
    stem.split(|c: char| c.is_whitespace() || c == '_' || (c == '.' && !stem.contains(' ')))
        .filter(|t| !t.is_empty())
        .collect()
}

/// `S01E02` (also `s1e2`, `S01E02E03`, `S01E02-E03`) or `1x02`.
fn episode_marker(token: &str) -> Option<(u32, u32)> {
    let lower = core(token).to_lowercase();
    if let Some(rest) = lower.strip_prefix('s') {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=2).contains(&digits) {
            let season = rest[..digits].parse().ok()?;
            let episode = number_after(&rest[digits..], 'e')?;
            return Some((season, episode));
        }
        return None;
    }
    let (s, e) = lower.split_once('x')?;
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    if (1..=2).contains(&s.len()) && (2..=3).contains(&e.len()) && all_digits(s) && all_digits(e) {
        return Some((s.parse().ok()?, e.parse().ok()?));
    }
    None
}

/// A lone season token such as `S01`.
fn season_marker(token: &str) -> Option<u32> {
    let rest = core(token).to_lowercase();
    let rest = rest.strip_prefix('s')?;
    ((1..=2).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit()))
        .then(|| rest.parse().ok())
        .flatten()
}

/// The number after `prefix` at the start of `text` ("e02" → 2), ignoring what follows it.
fn number_after(text: &str, prefix: char) -> Option<u32> {
    let lower = text.to_lowercase();
    let rest = lower.strip_prefix(prefix)?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (1..=3)
        .contains(&digits)
        .then(|| rest[..digits].parse().ok())
        .flatten()
}

/// Whether a folder is just a season of its show: "Season 1", "Season 01", "S01", "Series 2".
fn is_season_folder(name: &str) -> bool {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_')
        .filter(|w| !w.is_empty())
        .collect();
    match words.as_slice() {
        [word, n] => {
            [
                "season",
                "series",
                "staffel",
                "saison",
                "temporada",
                "sezon",
            ]
            .contains(word)
                && n.bytes().all(|b| b.is_ascii_digit())
        }
        [one] => season_marker(one).is_some(),
        _ => false,
    }
}

/// The show's name from the folders around an episode: `Show/Season 1/S01E02.mkv` or
/// `Show.Name.S01.1080p/S01E02.mkv`.
fn show_from_folders(path: &Path, max_year: i32) -> Option<(String, Option<i32>)> {
    let mut folders = path.ancestors().skip(1).filter_map(|p| p.file_name());
    let mut name = folders.next()?.to_string_lossy();
    if is_season_folder(&name) {
        name = folders.next()?.to_string_lossy();
    }
    let tokens = tokenize(&name);
    let end = tokens
        .iter()
        .position(|t| season_marker(t).is_some() || episode_marker(t).is_some())
        .unwrap_or(tokens.len());
    let (show, year) = parse_with_max_year(&tokens[..end].join(" "), max_year);
    (!show.is_empty()).then_some((show, year))
}

fn clean(s: &str) -> String {
    let s = s.replace("()", "").replace("[]", "");
    s.trim_matches(|c: char| c.is_whitespace() || "-–—:,([{".contains(c))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> (String, Option<i32>) {
        parse_with_max_year(s, 2027)
    }

    fn t(title: &str, year: Option<i32>) -> (String, Option<i32>) {
        (title.to_string(), year)
    }

    #[test]
    fn scene_style_names() {
        assert_eq!(
            parse("Movie.Name.2019.1080p.BluRay.x264-GROUP"),
            t("Movie Name", Some(2019))
        );
        assert_eq!(
            parse("Inception.2010.2160p.UHD.BluRay.REMUX.HDR.HEVC.Atmos-EPSiLON"),
            t("Inception", Some(2010))
        );
        assert_eq!(
            parse("The_Big_Lebowski_1998_720p"),
            t("The Big Lebowski", Some(1998))
        );
    }

    #[test]
    fn plain_names_with_parenthesised_year() {
        assert_eq!(parse("The Matrix (1999)"), t("The Matrix", Some(1999)));
        assert_eq!(parse("Movie [2019] [YTS.MX]"), t("Movie", Some(2019)));
        assert_eq!(
            parse("[YTS] Movie Name (2019)"),
            t("Movie Name", Some(2019))
        );
        assert_eq!(
            parse("E.T. the Extra-Terrestrial (1982)"),
            t("E.T. the Extra-Terrestrial", Some(1982))
        );
        assert_eq!(
            parse("Charlotte's Web (1973)"),
            t("Charlotte's Web", Some(1973))
        );
    }

    #[test]
    fn numeric_titles_are_not_mistaken_for_years() {
        assert_eq!(
            parse("2001.A.Space.Odyssey.1968.2160p.UHD.BluRay"),
            t("2001 A Space Odyssey", Some(1968))
        );
        assert_eq!(
            parse("Blade.Runner.2049.2017.1080p.WEB-DL"),
            t("Blade Runner 2049", Some(2017))
        );
        assert_eq!(parse("1917 (2019) [1080p]"), t("1917", Some(2019)));
        assert_eq!(parse("1917"), t("1917", None));
    }

    #[test]
    fn names_without_a_year() {
        assert_eq!(parse("Inception.1080p.BluRay"), t("Inception", None));
        assert_eq!(parse("some_home_video"), t("some home video", None));
        assert_eq!(
            parse("Alien.1979.Directors.Cut.1080p"),
            t("Alien", Some(1979))
        );
    }

    fn episode(path: &str) -> Option<EpisodeName> {
        parse_episode(Path::new(path))
    }

    fn ep(
        show: &str,
        year: Option<i32>,
        season: u32,
        episode: u32,
        title: Option<&str>,
    ) -> Option<EpisodeName> {
        Some(EpisodeName {
            show: show.into(),
            year,
            season,
            episode,
            title: title.map(Into::into),
        })
    }

    #[test]
    fn scene_style_episodes() {
        assert_eq!(
            episode("D:/TV/Breaking.Bad.S01E02.Cats.in.the.Bag.1080p.BluRay.x264-GRP.mkv"),
            ep("Breaking Bad", None, 1, 2, Some("Cats in the Bag"))
        );
        assert_eq!(
            episode("D:/TV/the.office.us.s02e10.720p.hdtv.mkv"),
            ep("the office us", None, 2, 10, None)
        );
        assert_eq!(
            episode("D:/TV/Doctor.Who.2005.S03E08.mkv"),
            ep("Doctor Who", Some(2005), 3, 8, None)
        );
        assert_eq!(
            episode("D:/TV/Show.S01E01E02.Double.Bill.mkv"),
            ep("Show", None, 1, 1, Some("Double Bill"))
        );
        assert_eq!(
            episode("D:/TV/Show.S01.E05.mkv"),
            ep("Show", None, 1, 5, None)
        );
    }

    #[test]
    fn plain_episodes() {
        assert_eq!(
            episode("D:/TV/Fawlty Towers - 1x03 - The Wedding Party.mkv"),
            ep("Fawlty Towers", None, 1, 3, Some("The Wedding Party"))
        );
        assert_eq!(
            episode("D:/TV/Show Name - S02E11 - A Title (1080p).mkv"),
            ep("Show Name", None, 2, 11, Some("A Title"))
        );
    }

    #[test]
    fn the_show_comes_from_folders_when_the_file_has_no_name() {
        assert_eq!(
            episode("D:/TV/Severance/Season 1/S01E03.mkv"),
            ep("Severance", None, 1, 3, None)
        );
        assert_eq!(
            episode("D:/TV/Severance (2022)/S01/S01E04 - In Perpetuity.mkv"),
            ep("Severance", Some(2022), 1, 4, Some("In Perpetuity"))
        );
        assert_eq!(
            episode("D:/TV/Dark.S02.1080p.WEB/s02e01.mkv"),
            ep("Dark", None, 2, 1, None)
        );
    }

    #[test]
    fn movies_are_not_episodes() {
        for movie in [
            "D:/Movies/Inception.2010.1080p.mkv",
            "D:/Movies/2001.A.Space.Odyssey.1968.mkv",
            "D:/Movies/Se7en.1995.mkv",
            "D:/Movies/S1m0ne.2002.mkv",
            "D:/Movies/The.Matrix.1999.x264.mkv",
            "D:/Movies/Blade.Runner.2049.2017.2160p.mkv",
            "D:/Movies/Movie.Name.2019.1920x1080.mkv",
        ] {
            assert_eq!(episode(movie), None, "{movie}");
        }
    }

    #[test]
    fn degenerate_input_still_gives_a_title() {
        assert_eq!(parse("1080p"), t("1080p", None));
        assert_eq!(parse(""), t("", None));
    }
}
