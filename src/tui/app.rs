//! UI state, independent of the terminal so it can be unit tested.

use std::path::PathBuf;

use super::events::Action;
use crate::search::Engine;
use crate::search::filesystem::ScanSummary;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Selected(PathBuf),
    Cancelled,
    Interrupted,
}

pub struct App {
    pub engine: Engine,
    pub input: String,
    /// Cursor position in chars.
    pub cursor: usize,
    pub selected: usize,
    pub offset: usize,
    pub roots: Vec<String>,
    pub scanning: bool,
    pub scanned: usize,
    pub summary: Option<ScanSummary>,
    pub tick: usize,
    pub show_scores: bool,
    pub show_preview: bool,
    pub page: usize,
    pub preview: Option<(PathBuf, Vec<String>)>,
}

impl App {
    pub fn new(engine: Engine, query: &str, roots: Vec<String>, scanning: bool) -> Self {
        let mut app = Self {
            engine,
            input: query.to_string(),
            cursor: query.chars().count(),
            selected: 0,
            offset: 0,
            roots,
            scanning,
            scanned: 0,
            summary: None,
            tick: 0,
            show_scores: false,
            show_preview: false,
            page: 10,
            preview: None,
        };
        app.engine.set_query(query);
        app
    }

    pub fn add_scanned(&mut self, batch: Vec<PathBuf>) {
        self.scanned += batch.len();
        let keep = self.current_path();
        self.engine.add_fs_paths(batch);
        // Keep the highlighted entry stable while results stream in.
        if let Some(p) = keep
            && let Some(i) = self
                .engine
                .results()
                .iter()
                .position(|r| self.engine.set.get(r.idx).path == p)
        {
            self.selected = i;
        }
        self.clamp();
    }

    pub fn scan_done(&mut self, s: ScanSummary) {
        self.scanning = false;
        self.summary = Some(s);
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        let r = self.engine.results().get(self.selected)?;
        Some(self.engine.set.get(r.idx).path.clone())
    }

    fn byte_pos(&self, char_pos: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_pos)
            .map_or(self.input.len(), |(b, _)| b)
    }

    fn clamp(&mut self) {
        let n = self.engine.results().len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }

    /// Applies an action; returns `Some` when the session should end.
    pub fn handle(&mut self, action: Action) -> Option<Outcome> {
        let before = self.input.clone();
        let len = self.input.chars().count();
        match action {
            Action::Insert(c) => {
                let b = self.byte_pos(self.cursor);
                self.input.insert(b, c);
                self.cursor += 1;
            }
            Action::Backspace if self.cursor > 0 => {
                let b = self.byte_pos(self.cursor - 1);
                self.input.remove(b);
                self.cursor -= 1;
            }
            Action::Delete if self.cursor < len => {
                let b = self.byte_pos(self.cursor);
                self.input.remove(b);
            }
            Action::Left => self.cursor = self.cursor.saturating_sub(1),
            Action::Right => self.cursor = (self.cursor + 1).min(len),
            Action::Home => self.cursor = 0,
            Action::End => self.cursor = len,
            Action::ClearLine => {
                self.input.clear();
                self.cursor = 0;
            }
            Action::DeleteWord => {
                let chars: Vec<char> = self.input.chars().collect();
                let mut start = self.cursor;
                while start > 0 && chars[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && chars[start - 1] != ' ' && chars[start - 1] != '/' {
                    start -= 1;
                }
                let (a, b) = (self.byte_pos(start), self.byte_pos(self.cursor));
                self.input.replace_range(a..b, "");
                self.cursor = start;
            }
            Action::Up => self.selected = self.selected.saturating_sub(1),
            Action::Down => self.selected += 1,
            Action::PageUp => self.selected = self.selected.saturating_sub(self.page.max(1)),
            Action::PageDown => self.selected += self.page.max(1),
            Action::Accept => return self.current_path().map(Outcome::Selected),
            Action::Cancel => return Some(Outcome::Cancelled),
            Action::Quit => return Some(Outcome::Interrupted),
            _ => {}
        }
        if self.input != before {
            self.engine.set_query(&self.input);
            self.selected = 0;
            self.offset = 0;
        }
        self.clamp();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RankingConfig;

    fn app() -> App {
        let mut e = Engine::new(None, &[], RankingConfig::default(), 0, 100);
        e.add_fs_paths(["/x/alpha".into(), "/x/beta".into(), "/x/alps".into()]);
        App::new(e, "", vec!["/x".into()], false)
    }

    #[test]
    fn typing_filters_and_accept_returns_path() {
        let mut a = app();
        for c in "beta".chars() {
            assert!(a.handle(Action::Insert(c)).is_none());
        }
        assert_eq!(
            a.handle(Action::Accept),
            Some(Outcome::Selected("/x/beta".into()))
        );
    }

    #[test]
    fn navigation_is_clamped_and_cancel_works() {
        let mut a = app();
        a.handle(Action::PageDown);
        assert_eq!(a.selected, 2);
        a.handle(Action::Up);
        a.handle(Action::Up);
        a.handle(Action::Up);
        assert_eq!(a.selected, 0);
        assert_eq!(a.handle(Action::Cancel), Some(Outcome::Cancelled));
        assert_eq!(a.handle(Action::Quit), Some(Outcome::Interrupted));
    }

    #[test]
    fn editing_with_unicode_and_words() {
        let mut a = app();
        for c in "née proj/x".chars() {
            a.handle(Action::Insert(c));
        }
        a.handle(Action::DeleteWord);
        assert_eq!(a.input, "née proj/");
        a.handle(Action::Home);
        a.handle(Action::Right);
        a.handle(Action::Delete);
        assert_eq!(a.input, "ne proj/");
        a.handle(Action::ClearLine);
        assert_eq!(a.input, "");
    }

    #[test]
    fn no_results_accept_does_nothing() {
        let mut a = app();
        for c in "zzzz".chars() {
            a.handle(Action::Insert(c));
        }
        assert_eq!(a.handle(Action::Accept), None);
    }
}
