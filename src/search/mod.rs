//! Search engine: merges history and filesystem candidates and keeps a ranked
//! result list up to date as the query changes and scan batches arrive.

pub mod filesystem;
pub mod fuzzy;
pub mod history;
pub mod merge;
pub mod ranking;

use std::cmp::Ordering;
use std::path::PathBuf;

use crate::config::{JumpConfig, RankingConfig};
use crate::database::HistoryEntry;
use fuzzy::QueryMatcher;
use merge::CandidateSet;

#[derive(Debug, Clone, Copy)]
pub struct Ranked {
    pub idx: usize,
    pub quality: f64,
    pub score: f64,
}

pub struct Engine {
    pub set: CandidateSet,
    cfg: RankingConfig,
    now: i64,
    limit: usize,
    query: String,
    matcher: QueryMatcher,
    /// (candidate index, quality) for every candidate matching `query`.
    matched: Vec<(usize, f64)>,
    results: Vec<Ranked>,
}

impl Engine {
    pub fn new(
        home: Option<PathBuf>,
        history: &[HistoryEntry],
        cfg: RankingConfig,
        now: i64,
        limit: usize,
    ) -> Self {
        let mut set = CandidateSet::new(home);
        for e in history {
            set.add_history(e, now, &cfg);
        }
        let mut engine = Self {
            set,
            cfg,
            now,
            limit: limit.max(1),
            query: String::new(),
            matcher: QueryMatcher::new(""),
            matched: Vec::new(),
            results: Vec::new(),
        };
        engine.rematch_all();
        engine
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn results(&self) -> &[Ranked] {
        &self.results
    }

    pub fn total_matches(&self) -> usize {
        self.matched.len()
    }

    /// Updates the query. When the new query only appends to the old one, the
    /// match set can only shrink, so only previous matches are re-scored.
    pub fn set_query(&mut self, q: &str) {
        if q == self.query {
            return;
        }
        let narrowing = !self.query.trim().is_empty()
            && q.starts_with(&self.query)
            && q.contains('/') == self.query.contains('/');
        self.query = q.to_string();
        self.matcher = QueryMatcher::new(q);
        if narrowing {
            let prev = std::mem::take(&mut self.matched);
            for (i, _) in prev {
                let c = self.set.get(i);
                if let Some(qual) = self.matcher.quality(&c.display, c.base_start) {
                    self.matched.push((i, qual));
                }
            }
            self.rerank();
        } else {
            self.rematch_all();
        }
    }

    /// Adds discovered directories and matches only the new ones.
    pub fn add_fs_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut changed = false;
        for p in paths {
            if let Some(i) = self.set.add_fs(p) {
                let c = self.set.get(i);
                if let Some(q) = self.matcher.quality(&c.display, c.base_start) {
                    self.matched.push((i, q));
                }
            }
            changed = true;
        }
        if changed {
            self.rerank();
        }
    }

    pub fn highlight(&mut self, idx: usize) -> Vec<u32> {
        let c = self.set.get(idx);
        self.matcher.indices(&c.display, c.base_start)
    }

    fn rematch_all(&mut self) {
        self.matched.clear();
        for i in 0..self.set.len() {
            let c = self.set.get(i);
            if let Some(q) = self.matcher.quality(&c.display, c.base_start) {
                self.matched.push((i, q));
            }
        }
        self.rerank();
    }

    fn rerank(&mut self) {
        let mut all: Vec<Ranked> = self
            .matched
            .iter()
            .map(|&(idx, quality)| Ranked {
                idx,
                quality,
                score: ranking::score(quality, self.set.get(idx).signals(), &self.cfg),
            })
            .collect();
        let set = &self.set;
        let cmp = |a: &Ranked, b: &Ranked| compare(set, a, b);
        if all.len() > self.limit {
            all.select_nth_unstable_by(self.limit - 1, cmp);
            all.truncate(self.limit);
        }
        all.sort_unstable_by(cmp);
        self.results = all;
    }
}

/// Decides whether a search is unambiguous enough to jump without the picker.
///
/// Jump to the best result only if its match quality reaches
/// `min_quality` and it is the only result, or an exact name match while the
/// runner-up is not, or scores at least `margin` × the runner-up. Because history multiplies the score (up to
/// 1.5× by default), a directory picked once or twice recently pulls ahead
/// of equally named ones and starts jumping directly; as recency decays the
/// picker returns.
///
/// `unseen_max` is used while the scan is still running: the best score any
/// not-yet-discovered directory could reach. Such directories have no history
/// (every history entry is loaded up front), so it is `match_weight × 1.0`.
/// Only the margin rule can then apply, against that bound; this lets a
/// clear history winner jump instantly instead of waiting for a slow scan.
pub fn auto_jump(results: &[Ranked], cfg: &JumpConfig, unseen_max: Option<f64>) -> Option<usize> {
    let best = results.first()?;
    if best.quality < cfg.min_quality {
        return None;
    }
    let second = results.get(1);
    if unseen_max.is_none() {
        match second {
            None => return Some(best.idx),
            Some(s) if fuzzy::is_exact(best.quality) && !fuzzy::is_exact(s.quality) => {
                return Some(best.idx);
            }
            _ => {}
        }
    }
    let rival = second
        .map_or(0.0, |s| s.score)
        .max(unseen_max.unwrap_or(0.0));
    (rival > 0.0 && best.score >= cfg.margin * rival).then_some(best.idx)
}

/// Upper bound on the score of a directory the scan has not reported yet.
pub fn unseen_bound(cfg: &RankingConfig) -> f64 {
    ranking::score(1.0, None, cfg)
}

/// Score descending; ties prefer history, then shallower/shorter paths, then
/// alphabetical order, so the ordering is fully deterministic.
fn compare(set: &CandidateSet, a: &Ranked, b: &Ranked) -> Ordering {
    let (ca, cb) = (set.get(a.idx), set.get(b.idx));
    b.score
        .total_cmp(&a.score)
        .then_with(|| cb.history.is_some().cmp(&ca.history.is_some()))
        .then_with(|| ca.display.len().cmp(&cb.display.len()))
        .then_with(|| ca.display.cmp(&cb.display))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, visits: u64, last: i64) -> HistoryEntry {
        HistoryEntry {
            path: path.into(),
            visits,
            last_visited: last,
            first_seen: 0,
            is_git: false,
        }
    }

    fn top(e: &Engine) -> Vec<String> {
        e.results()
            .iter()
            .map(|r| e.set.get(r.idx).display.clone())
            .collect()
    }

    #[test]
    fn merges_dedups_and_ranks_history_first_on_equal_match() {
        let hist = [entry("/u/CNN-frontend", 10, 1000)];
        let mut e = Engine::new(None, &hist, RankingConfig::default(), 1000, 100);
        e.set_query("CNN-frontend");
        e.add_fs_paths([
            "/w/CNN-frontend".into(),
            "/u/CNN-frontend".into(),
            "/w/other".into(),
        ]);
        assert_eq!(top(&e), vec!["/u/CNN-frontend", "/w/CNN-frontend"]);
        assert_eq!(e.set.len(), 3);
    }

    #[test]
    fn exact_filesystem_match_beats_weak_history_match() {
        let hist = [entry("/h/c-n-n-x-frontend-thing", 500, 1000)];
        let mut e = Engine::new(None, &hist, RankingConfig::default(), 1000, 100);
        e.add_fs_paths(["/w/cnnfrontend".into()]);
        e.set_query("cnnfrontend");
        assert_eq!(top(&e)[0], "/w/cnnfrontend");
    }

    #[test]
    fn narrowing_and_widening_agree_with_fresh_match() {
        let mut e = Engine::new(None, &[], RankingConfig::default(), 0, 100);
        e.add_fs_paths(["/a/alpha".into(), "/a/alps".into(), "/a/beta".into()]);
        e.set_query("al");
        assert_eq!(e.total_matches(), 2);
        e.set_query("alph");
        assert_eq!(top(&e), vec!["/a/alpha"]);
        e.set_query("b");
        assert_eq!(top(&e), vec!["/a/beta"]);
        e.set_query("");
        assert_eq!(e.total_matches(), 3);
    }

    #[test]
    fn empty_query_orders_by_frecency() {
        let hist = [entry("/a", 1, 0), entry("/b", 50, 1000)];
        let e = Engine::new(None, &hist, RankingConfig::default(), 1000, 100);
        assert_eq!(top(&e), vec!["/b", "/a"]);
    }

    fn jump(hist: &[HistoryEntry], fs: &[&str], q: &str) -> Option<String> {
        let mut e = Engine::new(None, hist, RankingConfig::default(), 1000, 100);
        e.add_fs_paths(fs.iter().map(PathBuf::from));
        e.set_query(q);
        auto_jump(e.results(), &JumpConfig::default(), None).map(|i| e.set.get(i).display.clone())
    }

    #[test]
    fn auto_jump_policy() {
        // Sole good match: jump.
        assert_eq!(
            jump(&[], &["/a/kicad", "/a/other"], "kicad").as_deref(),
            Some("/a/kicad")
        );
        // Exact name clearly ahead of a prefix match: jump.
        assert_eq!(
            jump(&[], &["/a/kicad", "/a/kicad-libs-old-stuff"], "kicad").as_deref(),
            Some("/a/kicad")
        );
        // Two identical names, no history: ambiguous, show picker.
        assert_eq!(jump(&[], &["/a/kicad", "/b/kicad"], "kicad"), None);
        // Weak sole match: show picker.
        assert_eq!(jump(&[], &["/a/k-x-i-y-c-z-a-w-d"], "kicad"), None);
        // Nothing: no jump.
        assert_eq!(jump(&[], &["/a/x"], "kicad"), None);
    }

    #[test]
    fn picking_once_or_twice_makes_it_jump() {
        let day = 86_400;
        let fs = ["/a/kicad", "/b/kicad"];
        let picked = |visits, last| [entry("/b/kicad", visits, last)];
        assert_eq!(
            jump(&picked(1, 1000), &fs, "kicad").as_deref(),
            Some("/b/kicad")
        );
        assert_eq!(
            jump(&picked(2, 1000), &fs, "kicad").as_deref(),
            Some("/b/kicad")
        );
        // Long unused and rarely visited: ambiguous again.
        assert_eq!(jump(&picked(1, 1000 - 30 * day), &fs, "kicad"), None);
    }

    #[test]
    fn history_winner_jumps_before_scan_finishes() {
        let cfg = RankingConfig::default();
        let bound = Some(unseen_bound(&cfg));
        // Picked twice just now: beats anything the scan could still find.
        let mut e = Engine::new(None, &[entry("/a/zed", 2, 1000)], cfg.clone(), 1000, 100);
        e.set_query("zed");
        assert!(auto_jump(e.results(), &JumpConfig::default(), bound).is_some());
        // A sole exact match without history must wait: an unseen one could tie.
        let mut e = Engine::new(None, &[], cfg, 1000, 100);
        e.add_fs_paths(["/a/zed".into()]);
        e.set_query("zed");
        assert!(auto_jump(e.results(), &JumpConfig::default(), bound).is_none());
        assert!(auto_jump(e.results(), &JumpConfig::default(), None).is_some());
    }

    #[test]
    fn limit_keeps_best() {
        let mut e = Engine::new(None, &[], RankingConfig::default(), 0, 2);
        e.add_fs_paths((0..50).map(|i| PathBuf::from(format!("/d/x{i:02}"))));
        e.add_fs_paths(["/d/x".into()]);
        e.set_query("x");
        assert_eq!(e.results().len(), 2);
        assert_eq!(top(&e)[0], "/d/x");
    }
}
