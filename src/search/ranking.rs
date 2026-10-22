//! Ranking model.
//!
//! ```text
//! score = quality × (match_weight + history_boost × (frequency_weight × F + recency_weight × R))
//!
//! F = min(1, ln(1 + visits) / ln(1 + frequency_saturation))   — diminishing returns
//! R = 0.5 ^ (days_since_last_visit / recency_half_life_days)   — exponential decay
//! ```
//!
//! `quality` (0..=1) comes from [`super::fuzzy`]; directories never visited
//! have F = R = 0. Because history *multiplies* the match quality instead of
//! being added to it, a poor match cannot be rescued by heavy use: with the
//! default weights the best possible history multiplier is 1.5, so a history
//! entry needs quality > 1/1.5 ≈ 0.67 to outrank an exact, never-visited
//! match. Among comparable matches, frequently and recently visited
//! directories win.

use crate::config::RankingConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    pub frequency: f64,
    pub recency: f64,
}

pub fn history_signals(visits: u64, last_visited: i64, now: i64, cfg: &RankingConfig) -> Signals {
    let sat = (cfg.frequency_saturation.max(1) as f64).ln_1p();
    let frequency = ((visits as f64).ln_1p() / sat).min(1.0);
    let age_days = (now - last_visited).max(0) as f64 / 86_400.0;
    let half_life = cfg.recency_half_life_days.max(f64::MIN_POSITIVE);
    let recency = 0.5f64.powf(age_days / half_life);
    Signals { frequency, recency }
}

pub fn score(quality: f64, signals: Option<Signals>, cfg: &RankingConfig) -> f64 {
    let history = signals.map_or(0.0, |s| {
        cfg.frequency_weight * s.frequency + cfg.recency_weight * s.recency
    });
    quality * (cfg.match_weight + cfg.history_boost * history)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn signals_are_bounded_and_monotonic() {
        let c = RankingConfig::default();
        let s1 = history_signals(1, 0, 0, &c);
        let s10 = history_signals(10, 0, 0, &c);
        let s_huge = history_signals(1_000_000, 0, 0, &c);
        assert!(s1.frequency < s10.frequency && s_huge.frequency == 1.0);
        assert_eq!(s1.recency, 1.0);
        let week = history_signals(1, 0, 7 * DAY, &c);
        assert!((week.recency - 0.5).abs() < 1e-9);
    }

    #[test]
    fn weak_history_does_not_bury_exact_match() {
        let c = RankingConfig::default();
        let maxed = Some(Signals {
            frequency: 1.0,
            recency: 1.0,
        });
        let exact_new = score(1.0, None, &c);
        let weak_hist = score(0.6, maxed, &c);
        assert!(exact_new > weak_hist, "{exact_new} vs {weak_hist}");
    }

    #[test]
    fn history_breaks_ties_between_equal_matches() {
        let c = RankingConfig::default();
        let visited = Some(history_signals(3, 0, DAY, &c));
        assert!(score(0.9, visited, &c) > score(0.9, None, &c));
    }

    #[test]
    fn recent_beats_stale_with_equal_visits() {
        let c = RankingConfig::default();
        let now = 100 * DAY;
        let recent = Some(history_signals(5, now - DAY, now, &c));
        let stale = Some(history_signals(5, now - 60 * DAY, now, &c));
        assert!(score(0.8, recent, &c) > score(0.8, stale, &c));
    }

    #[test]
    fn weights_are_respected() {
        let c = RankingConfig {
            recency_weight: 0.0,
            frequency_weight: 0.0,
            ..Default::default()
        };
        let s = Some(Signals {
            frequency: 1.0,
            recency: 1.0,
        });
        assert_eq!(score(0.5, s, &c), score(0.5, None, &c));
    }
}
