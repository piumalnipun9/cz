//! Non-interactive subcommands: history inspection and maintenance.

use anyhow::Result;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use crate::config::Config;
use crate::database::Database;
use crate::paths;
use crate::search::history;

pub fn ago(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

pub fn print_history(db: &Database, cfg: &Config, now: i64, home: Option<&Path>) -> Result<()> {
    let mut out = io::stdout().lock();
    for (e, score) in history::by_frecency(db.entries()?, now, &cfg.ranking) {
        let missing = if e.path.is_dir() { "" } else { "  (missing)" };
        writeln!(
            out,
            "{score:5.2} {:>6}  {:>9}  {}{missing}",
            e.visits,
            ago(now, e.last_visited),
            paths::display(&e.path, home)
        )?;
    }
    Ok(())
}

pub fn print_stats(
    db: &Database,
    db_path: &Path,
    cfg: &Config,
    now: i64,
    home: Option<&Path>,
) -> Result<()> {
    let s = db.stats()?;
    let entries = db.entries()?;
    let missing = entries.iter().filter(|e| !e.path.is_dir()).count();
    let git = entries.iter().filter(|e| e.is_git).count();
    println!("database:     {}", db_path.display());
    println!(
        "directories:  {} ({} missing, {} git repositories)",
        s.directories, missing, git
    );
    println!("total visits: {}", s.total_visits);
    if let Some(t) = s.oldest {
        println!("tracking for: {}", ago(now, t).trim_end_matches(" ago"));
    }
    let top = history::by_frecency(entries, now, &cfg.ranking);
    if !top.is_empty() {
        println!("\ntop directories:");
        for (e, _) in top.iter().take(10) {
            println!("  {:>5}  {}", e.visits, paths::display(&e.path, home));
        }
    }
    Ok(())
}

pub fn clear_history(db: &Database, yes: bool) -> Result<bool> {
    let n = db.stats()?.directories;
    if !yes && io::stdin().is_terminal() {
        eprint!("Delete history for {n} directories? [y/N] ");
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            eprintln!("aborted");
            return Ok(false);
        }
    }
    let removed = db.clear()?;
    eprintln!("cleared {removed} directories");
    Ok(true)
}
