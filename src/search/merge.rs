//! The merged candidate set: one entry per normalized absolute path, carrying
//! whether it came from history, filesystem discovery, or both.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::ranking::{self, Signals};
use crate::config::RankingConfig;
use crate::database::HistoryEntry;
use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    History,
    Filesystem,
    Both,
}

#[derive(Debug, Clone)]
pub struct HistoryInfo {
    pub visits: u64,
    pub last_visited: i64,
    pub is_git: bool,
    pub signals: Signals,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    /// `~`-abbreviated, lossily decoded path used for matching and display.
    pub display: String,
    /// Byte offset of the basename within `display`.
    pub base_start: usize,
    pub history: Option<HistoryInfo>,
    pub in_fs: bool,
}

impl Candidate {
    pub fn source(&self) -> Source {
        match (self.history.is_some(), self.in_fs) {
            (true, true) => Source::Both,
            (true, false) => Source::History,
            _ => Source::Filesystem,
        }
    }

    pub fn signals(&self) -> Option<Signals> {
        self.history.as_ref().map(|h| h.signals)
    }
}

pub struct CandidateSet {
    items: Vec<Candidate>,
    index: HashMap<PathBuf, usize>,
    home: Option<PathBuf>,
}

impl CandidateSet {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            items: Vec::new(),
            index: HashMap::new(),
            home,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> &Candidate {
        &self.items[i]
    }

    pub fn find(&self, path: &Path) -> Option<&Candidate> {
        self.index.get(path).map(|&i| &self.items[i])
    }

    fn insert(&mut self, path: PathBuf) -> (usize, bool) {
        if let Some(&i) = self.index.get(&path) {
            return (i, false);
        }
        let display = paths::display(&path, self.home.as_deref());
        let base_start = match display.rfind('/') {
            Some(i) if display.len() > 1 => i + 1,
            _ => 0,
        };
        let i = self.items.len();
        self.index.insert(path.clone(), i);
        self.items.push(Candidate {
            path,
            display,
            base_start,
            history: None,
            in_fs: false,
        });
        (i, true)
    }

    pub fn add_history(&mut self, e: &HistoryEntry, now: i64, cfg: &RankingConfig) -> usize {
        let (i, _) = self.insert(e.path.clone());
        self.items[i].history = Some(HistoryInfo {
            visits: e.visits,
            last_visited: e.last_visited,
            is_git: e.is_git,
            signals: ranking::history_signals(e.visits, e.last_visited, now, cfg),
        });
        i
    }

    /// Adds a discovered directory; returns its index if it was not known yet.
    pub fn add_fs(&mut self, path: PathBuf) -> Option<usize> {
        let (i, new) = self.insert(path);
        self.items[i].in_fs = true;
        new.then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_and_tracks_sources() {
        let cfg = RankingConfig::default();
        let mut s = CandidateSet::new(Some("/home/u".into()));
        let e = HistoryEntry {
            path: "/home/u/p".into(),
            visits: 2,
            last_visited: 0,
            first_seen: 0,
            is_git: false,
        };
        s.add_history(&e, 0, &cfg);
        assert_eq!(s.add_fs("/home/u/p".into()), None);
        assert!(s.add_fs("/home/u/q".into()).is_some());
        assert_eq!(s.len(), 2);
        assert_eq!(
            s.find(Path::new("/home/u/p")).unwrap().source(),
            Source::Both
        );
        let q = s.find(Path::new("/home/u/q")).unwrap();
        assert_eq!(q.source(), Source::Filesystem);
        assert_eq!(&q.display[q.base_start..], "q");
        assert_eq!(q.display, "~/q");
    }
}
