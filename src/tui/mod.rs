//! Interactive terminal session. Draws on stderr (stdout may be redirected)
//! and always restores the terminal, including on panic.

pub mod app;
pub mod events;
pub mod ui;

use std::io::{self, IsTerminal, Stderr};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use crate::search::filesystem::{ScanEvent, ScanHandle};
use app::{App, Outcome};

const FRAME: Duration = Duration::from_millis(60);
const INGEST_BUDGET: Duration = Duration::from_millis(25);

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = TerminalGuard;
        execute!(io::stderr(), EnterAlternateScreen)?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Ok(guard)
    }
}

fn restore() {
    let _ = execute!(io::stderr(), LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

pub fn run(mut app: App, scan: Option<ScanHandle>) -> Result<Outcome> {
    if !io::stderr().is_terminal() {
        bail!("interactive mode needs a terminal on stderr; use --list or --first");
    }
    let _guard = TerminalGuard::enter()?;
    let mut term: Terminal<CrosstermBackend<Stderr>> =
        Terminal::new(CrosstermBackend::new(io::stderr()))?;
    let mut scan = scan;

    loop {
        if let Some(h) = &scan {
            let started = Instant::now();
            let mut batch = Vec::new();
            loop {
                match h.rx.try_recv() {
                    Ok(ScanEvent::Batch(b)) => batch.extend(b),
                    Ok(ScanEvent::Done(s)) => {
                        app.scan_done(s);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        app.scanning = false;
                        break;
                    }
                }
                if started.elapsed() > INGEST_BUDGET {
                    break;
                }
            }
            if !batch.is_empty() {
                app.add_scanned(batch);
            }
            if !app.scanning {
                scan = None;
            }
        }

        term.draw(|f| ui::draw(f, &mut app))?;
        app.tick += 1;

        if event::poll(FRAME)? {
            // Coalesce bursts (fast typing, paste) into one re-rank.
            loop {
                if let Event::Key(k) = event::read()?
                    && let Some(out) = app.handle(events::map(k))
                {
                    return Ok(out);
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}
