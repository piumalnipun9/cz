//! Rendering.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::app::App;
use crate::search::merge::Source;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const PREVIEW_MIN_WIDTH: u16 = 100;
const PREVIEW_MAX_ENTRIES: usize = 200;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.width < 24 || area.height < 7 {
        f.render_widget(Paragraph::new("cz: terminal too small"), area);
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " cz ",
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [search, scope, sep1, body, sep2, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    draw_search(f, app, search);
    draw_scope(f, app, scope);
    let rule = Paragraph::new("─".repeat(inner.width as usize)).style(dim());
    f.render_widget(rule.clone(), sep1);
    f.render_widget(rule, sep2);

    if app.show_preview && body.width >= PREVIEW_MIN_WIDTH {
        let [list, preview] =
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                .areas(body);
        draw_results(f, app, list);
        draw_preview(f, app, preview);
    } else {
        draw_results(f, app, body);
    }

    let keys = [("↑↓/Tab", "Navigate"), ("Enter", "Jump"), ("Esc", "Cancel")];
    let mut spans = Vec::new();
    for (k, v) in keys {
        spans.push(Span::styled(
            k,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {v}  ")));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), footer);
}

fn draw_search(f: &mut Frame, app: &App, area: Rect) {
    let label = "Search: ";
    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.input.as_str()),
    ]);
    // Keep the cursor visible for long queries by scrolling horizontally.
    let avail = area.width.saturating_sub(label.len() as u16 + 1) as usize;
    let scroll = app.cursor.saturating_sub(avail);
    f.render_widget(Paragraph::new(line).scroll((0, scroll as u16)), area);
    let x = area.x + (label.len() + app.cursor - scroll) as u16;
    f.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), area.y));
}

fn draw_scope(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.scanning {
        format!(
            "{} scanning {} entries",
            SPINNER[app.tick % SPINNER.len()],
            app.scanned
        )
    } else {
        let mut s = match &app.summary {
            Some(sum) if sum.files > 0 => format!("{} dirs, {} files", sum.dirs, sum.files),
            _ => format!("{} dirs", app.scanned),
        };
        if let Some(sum) = &app.summary {
            if sum.errors > 0 {
                s.push_str(&format!(", {} unreadable", sum.errors));
            }
            if sum.truncated {
                s.push_str(", limit reached");
            }
        }
        s
    };
    let status = format!("{status} · {} matches", app.engine.total_matches());
    let [left, right] = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(status.chars().count() as u16 + 1),
    ])
    .areas(area);
    let line = Line::from(vec![
        Span::styled(
            "Scope: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.roots.join(", ")),
        Span::styled(" + history", dim()),
    ]);
    f.render_widget(Paragraph::new(line), left);
    f.render_widget(
        Paragraph::new(Span::styled(status, dim())).right_aligned(),
        right,
    );
}

fn badge(src: Source) -> Span<'static> {
    let (text, color) = match src {
        Source::History => ("hist ", Color::Green),
        Source::Filesystem => ("fs   ", Color::Blue),
        Source::Both => ("both ", Color::Magenta),
    };
    Span::styled(text, Style::default().fg(color))
}

fn draw_results(f: &mut Frame, app: &mut App, area: Rect) {
    let height = area.height as usize;
    app.page = height.saturating_sub(1).max(1);
    let n = app.engine.results().len();
    if n == 0 {
        let msg = if app.scanning {
            "Searching…".to_string()
        } else {
            "No matches in history or the scope above.\nWiden the search explicitly with -H, -r or -p PATH.".to_string()
        };
        f.render_widget(Paragraph::new(msg).style(dim()), area);
        return;
    }
    if app.selected < app.offset {
        app.offset = app.selected;
    } else if app.selected >= app.offset + height {
        app.offset = app.selected + 1 - height;
    }
    app.offset = app.offset.min(n.saturating_sub(height));

    let width = area.width as usize;
    let mut lines = Vec::with_capacity(height);
    for row in app.offset..(app.offset + height).min(n) {
        let r = app.engine.results()[row];
        let selected = row == app.selected;
        let hl = app.engine.highlight(r.idx);
        let c = app.engine.set.get(r.idx);
        let mut spans = vec![
            if selected {
                Span::styled(
                    "❯ ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("  ")
            },
            if c.history.is_none() && !c.path.is_dir() {
                Span::styled("file ", Style::default().fg(Color::Yellow))
            } else {
                badge(c.source())
            },
        ];
        let suffix = if app.show_scores {
            let visits = c
                .history
                .as_ref()
                .map_or(String::new(), |h| format!(" ×{}", h.visits));
            format!(" {:.3} (m {:.2}{visits})", r.score, r.quality)
        } else {
            String::new()
        };
        let avail = width.saturating_sub(7 + suffix.chars().count());
        spans.extend(path_spans(&c.display, &hl, avail, selected));
        if !suffix.is_empty() {
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            spans.push(Span::raw(
                " ".repeat(width.saturating_sub(used + suffix.chars().count())),
            ));
            spans.push(Span::styled(suffix, dim()));
        }
        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::default().bg(Color::Rgb(40, 44, 52)));
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Splits `display` into spans with matched chars highlighted, truncating
/// from the left (keeping the basename visible) to fit `avail` columns.
fn path_spans(display: &str, hl: &[u32], avail: usize, selected: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = display.chars().collect();
    let mut spans = Vec::new();
    let start = if chars.len() > avail && avail > 1 {
        spans.push(Span::styled("…", dim()));
        chars.len() - (avail - 1)
    } else {
        0
    };
    let base = if selected {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let hit = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut cur = String::new();
    let mut cur_hit = false;
    for (i, ch) in chars.iter().enumerate().skip(start) {
        let is_hit = hl.binary_search(&(i as u32)).is_ok();
        if is_hit != cur_hit && !cur.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut cur),
                if cur_hit { hit } else { base },
            ));
        }
        cur_hit = is_hit;
        cur.push(*ch);
    }
    if !cur.is_empty() {
        spans.push(Span::styled(cur, if cur_hit { hit } else { base }));
    }
    spans
}

fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(dim())
        .title(Span::styled(" contents ", dim()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(path) = app.current_path() else {
        return;
    };
    if app.preview.as_ref().is_none_or(|(p, _)| *p != path) {
        app.preview = Some((path.clone(), list_dir(&path)));
    }
    let entries = &app.preview.as_ref().unwrap().1;
    let lines: Vec<Line> = entries
        .iter()
        .take(inner.height as usize)
        .map(|e| {
            if e.ends_with('/') {
                Line::styled(format!(" {e}"), Style::default().fg(Color::Blue))
            } else {
                Line::raw(format!(" {e}"))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn list_dir(path: &std::path::Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(path) else {
        return vec!["(unreadable)".into()];
    };
    let mut v: Vec<(bool, String)> = rd
        .flatten()
        .take(PREVIEW_MAX_ENTRIES)
        .map(|e| {
            let dir = e.file_type().is_ok_and(|t| t.is_dir());
            let mut name = e.file_name().to_string_lossy().into_owned();
            if dir {
                name.push('/');
            }
            (!dir, name)
        })
        .collect();
    v.sort();
    if v.is_empty() {
        return vec!["(empty)".into()];
    }
    v.into_iter().map(|(_, n)| n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_tail_and_highlights() {
        let s = path_spans("/aaaa/bbbb/cnn", &[11, 12, 13], 6, false);
        let text: String = s.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "…b/cnn");
        assert_eq!(s.last().unwrap().content, "cnn");
    }
}
