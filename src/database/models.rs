use std::path::PathBuf;

/// One learned directory. Timestamps are Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub visits: u64,
    pub last_visited: i64,
    pub first_seen: i64,
    /// Whether the directory contained `.git` when last recorded.
    pub is_git: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub directories: u64,
    pub total_visits: u64,
    pub oldest: Option<i64>,
}
