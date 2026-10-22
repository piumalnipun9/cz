//! Fuzzy matching on top of `nucleo-matcher`, normalized to a 0..=1 quality.
//!
//! What is matched:
//! * A query without `/` is matched against the directory's *basename* only.
//!   Matching full paths would let a short query hit almost every directory
//!   below a common ancestor (e.g. `pro` would match everything under
//!   `~/Projects`), drowning the directories that are actually named that.
//! * A query containing `/` is matched against the full displayed path
//!   (`~/Projects/CNN-frontend`), so `proj/cnn` disambiguates by parent.
//!
//! Whitespace splits the query into words that must all match (any order).
//! Matching is always case-insensitive.
//!
//! Quality = 0.7 * fuzzy + 0.3 * tier, where
//! * fuzzy = nucleo score / score the query would get matching itself
//!   (the best possible), clamped to 1;
//! * tier rewards literal matches: exact basename 1.0, basename prefix 0.6,
//!   contiguous substring 0.3, scattered fuzzy match 0. In path mode an exact
//!   path suffix gets 1.0 and a substring 0.3.
//!
//! The tier term exists because nucleo scores `CNN-frontend` and
//! `CNN-frontend-old` almost identically for the query `cnn-frontend`; users
//! expect the exact name first.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub const FUZZY_SHARE: f64 = 0.7;
pub const TIER_SHARE: f64 = 0.3;
pub const TIER_EXACT: f64 = 1.0;
pub const TIER_PREFIX: f64 = 0.6;
pub const TIER_SUBSTRING: f64 = 0.3;

/// Only an exact basename (or exact path suffix) can reach quality 1: every
/// other tier caps it at `FUZZY_SHARE + TIER_SHARE * TIER_PREFIX` = 0.88.
pub fn is_exact(quality: f64) -> bool {
    quality >= 1.0 - 1e-9
}

pub struct QueryMatcher {
    pattern: Pattern,
    matcher: Matcher,
    ideal: u32,
    query_lc: String,
    single_word: bool,
    path_mode: bool,
    buf: Vec<char>,
}

impl QueryMatcher {
    pub fn new(query: &str) -> Self {
        let query = query.trim();
        let path_mode = query.contains('/');
        let mut config = Config::DEFAULT;
        if path_mode {
            config.set_match_paths();
        } else {
            config.prefer_prefix = true;
        }
        let mut matcher = Matcher::new(config);
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let ideal = pattern
            .atoms
            .iter()
            .map(|a| a.score(a.needle_text(), &mut matcher).unwrap_or(1) as u32)
            .sum::<u32>()
            .max(1);
        Self {
            single_word: pattern.atoms.len() == 1,
            pattern,
            matcher,
            ideal,
            query_lc: query.to_lowercase(),
            path_mode,
            buf: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.atoms.is_empty()
    }

    pub fn path_mode(&self) -> bool {
        self.path_mode
    }

    /// Match quality in `0..=1`, or `None` if the candidate does not match.
    /// `base_start` is the byte offset of the basename within `display`.
    /// An empty query matches everything with quality 1.
    pub fn quality(&mut self, display: &str, base_start: usize) -> Option<f64> {
        if self.is_empty() {
            return Some(1.0);
        }
        let hay = self.haystack(display, base_start);
        let raw = self
            .pattern
            .score(Utf32Str::new(hay, &mut self.buf), &mut self.matcher)?;
        let fuzzy = (raw as f64 / self.ideal as f64).min(1.0);
        Some(FUZZY_SHARE * fuzzy + TIER_SHARE * self.tier(hay))
    }

    fn tier(&self, hay: &str) -> f64 {
        if !self.single_word {
            return 0.0;
        }
        let hay = hay.to_lowercase();
        if self.path_mode {
            if hay.ends_with(&self.query_lc) {
                TIER_EXACT
            } else if hay.contains(&self.query_lc) {
                TIER_SUBSTRING
            } else {
                0.0
            }
        } else if hay == self.query_lc {
            TIER_EXACT
        } else if hay.starts_with(&self.query_lc) {
            TIER_PREFIX
        } else if hay.contains(&self.query_lc) {
            TIER_SUBSTRING
        } else {
            0.0
        }
    }

    /// Sorted, deduplicated *char* indices into `display` for highlighting.
    pub fn indices(&mut self, display: &str, base_start: usize) -> Vec<u32> {
        if self.is_empty() {
            return Vec::new();
        }
        let hay = self.haystack(display, base_start);
        let offset = if self.path_mode {
            0
        } else {
            display[..base_start].chars().count() as u32
        };
        let mut idx = Vec::new();
        self.pattern.indices(
            Utf32Str::new(hay, &mut self.buf),
            &mut self.matcher,
            &mut idx,
        );
        idx.sort_unstable();
        idx.dedup();
        idx.iter_mut().for_each(|i| *i += offset);
        idx
    }

    fn haystack<'a>(&self, display: &'a str, base_start: usize) -> &'a str {
        if self.path_mode {
            display
        } else {
            &display[base_start..]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(query: &str, display: &str) -> Option<f64> {
        let base = display.rfind('/').map_or(0, |i| i + 1);
        QueryMatcher::new(query).quality(display, base)
    }

    #[test]
    fn exact_beats_prefix_beats_fuzzy() {
        let exact = q("cnn-frontend", "/x/CNN-frontend").unwrap();
        let prefix = q("cnn-frontend", "/x/CNN-frontend-old").unwrap();
        let fuzzy = q("cnnfe", "/x/CNN-frontend").unwrap();
        assert!((exact - 1.0).abs() < 1e-9, "exact = {exact}");
        assert!(exact > prefix && prefix > fuzzy, "{exact} {prefix} {fuzzy}");
    }

    #[test]
    fn basename_only_unless_slash() {
        assert!(q("projects", "/home/u/Projects/app").is_none());
        assert!(q("proj/app", "/home/u/Projects/app").is_some());
    }

    #[test]
    fn words_match_in_any_order() {
        assert!(q("front cnn", "/x/CNN-frontend").is_some());
        assert!(q("front zzz", "/x/CNN-frontend").is_none());
    }

    #[test]
    fn indices_point_into_display() {
        let d = "~/a/CNN";
        let mut m = QueryMatcher::new("cnn");
        assert_eq!(m.indices(d, 4), vec![4, 5, 6]);
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(q("  ", "/anything"), Some(1.0));
    }
}
