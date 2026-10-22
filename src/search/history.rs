//! Loading learned history into the candidate set.

use crate::config::RankingConfig;
use crate::database::HistoryEntry;
use crate::search::ranking;

/// Entries whose directory still exists. Missing directories are kept in the
/// database (they may be on an unmounted drive) but never offered as results;
/// `cz --prune` deletes them.
pub fn existing(entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    entries.into_iter().filter(|e| e.path.is_dir()).collect()
}

/// History sorted by pure frecency (query-independent), most relevant first.
pub fn by_frecency(
    mut entries: Vec<HistoryEntry>,
    now: i64,
    cfg: &RankingConfig,
) -> Vec<(HistoryEntry, f64)> {
    let mut scored: Vec<_> = entries
        .drain(..)
        .map(|e| {
            let s = ranking::score(
                1.0,
                Some(ranking::history_signals(e.visits, e.last_visited, now, cfg)),
                cfg,
            );
            (e, s)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
    scored
}
