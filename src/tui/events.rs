//! Key bindings.
//!
//! | Key                     | Action                                      |
//! |-------------------------|---------------------------------------------|
//! | text                    | edit the query (results update live)        |
//! | Up / Shift-Tab / Ctrl-P / Ctrl-K | previous result                    |
//! | Down / Tab / Ctrl-N / Ctrl-J     | next result                        |
//! | PageUp / PageDown       | move by a page                              |
//! | Enter                   | jump to the highlighted directory           |
//! | Esc / Ctrl-C            | cancel; the shell stays where it was        |
//! | Ctrl-U / Ctrl-W         | clear query / delete previous word          |
//! | Left/Right, Home/End, Ctrl-A/Ctrl-E | move the cursor                 |

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    ClearLine,
    DeleteWord,
    Up,
    Down,
    PageUp,
    PageDown,
    Accept,
    Cancel,
    None,
}

pub fn map(key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if ctrl => Action::Cancel,
        KeyCode::Char('p' | 'k') if ctrl => Action::Up,
        KeyCode::Char('n' | 'j') if ctrl => Action::Down,
        KeyCode::Char('u') if ctrl => Action::ClearLine,
        KeyCode::Char('w') if ctrl => Action::DeleteWord,
        KeyCode::Char('h') if ctrl => Action::Backspace,
        KeyCode::Char('a') if ctrl => Action::Home,
        KeyCode::Char('e') if ctrl => Action::End,
        KeyCode::Char(_) if ctrl => Action::None,
        KeyCode::Char(c) => Action::Insert(c),
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete => Action::Delete,
        KeyCode::Left => Action::Left,
        KeyCode::Right => Action::Right,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Up | KeyCode::BackTab => Action::Up,
        KeyCode::Down | KeyCode::Tab => Action::Down,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Enter => Action::Accept,
        KeyCode::Esc => Action::Cancel,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> Action {
        map(KeyEvent::new(code, m))
    }

    #[test]
    fn bindings() {
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Cancel);
        assert_eq!(
            k(KeyCode::Char('C'), KeyModifiers::SHIFT),
            Action::Insert('C')
        );
        assert_eq!(k(KeyCode::Tab, KeyModifiers::NONE), Action::Down);
        assert_eq!(k(KeyCode::BackTab, KeyModifiers::SHIFT), Action::Up);
        assert_eq!(k(KeyCode::Esc, KeyModifiers::NONE), Action::Cancel);
    }
}
