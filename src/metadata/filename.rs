//! Guessing a movie's title and year from its file name.
//!
//! `Movie.Name.2019.1080p.BluRay.x264-GRP.mkv` → ("Movie Name", Some(2019))

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

    #[test]
    fn degenerate_input_still_gives_a_title() {
        assert_eq!(parse("1080p"), t("1080p", None));
        assert_eq!(parse(""), t("", None));
    }
}
