use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Parser;

use cz::cli::{self, Cli};
use cz::config::Config;
use cz::database::{self, Database};
use cz::search::filesystem::{self, ScanEvent, ScanOptions};
use cz::search::{self, Engine, history};
use cz::tui::app::{App, Outcome};
use cz::{commands, paths, shell, tui};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("cz: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// The logical working directory: `$PWD` when it names the same directory as
/// `.` (so symlinked paths match what the shell shows), else the physical one.
fn current_dir() -> Result<PathBuf> {
    let physical = std::env::current_dir().context("cannot determine current directory")?;
    if let Some(pwd) = std::env::var_os("PWD").map(PathBuf::from)
        && pwd.is_absolute()
        && let (Ok(a), Ok(b)) = (std::fs::metadata(&pwd), std::fs::metadata("."))
        && a.dev() == b.dev()
        && a.ino() == b.ino()
    {
        return Ok(paths::normalize(&pwd, &physical));
    }
    Ok(physical)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn emit_selection(path: &Path, output_file: Option<&Path>) -> Result<()> {
    let bytes = path.as_os_str().as_bytes();
    match output_file {
        Some(f) => std::fs::write(f, bytes).with_context(|| format!("writing {}", f.display())),
        None => {
            let mut out = std::io::stdout().lock();
            out.write_all(bytes)?;
            out.write_all(b"\n")?;
            Ok(())
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    if let Some(sh) = cli.init {
        let bin = std::env::current_exe().context("locating the cz executable")?;
        print!("{}", shell::integration::script(sh, &bin));
        return Ok(ExitCode::SUCCESS);
    }

    let cfg = Config::load()?;
    let cwd = current_dir()?;
    let home = paths::home_dir();
    let db_path = database::default_path()?;
    let db = Database::open(&db_path)?;
    let now = now();

    if let Some(raw) = &cli.record {
        let p = paths::normalize(&paths::expand(raw), &cwd);
        if !p.is_dir() {
            bail!("--record {}: not a directory", p.display());
        }
        db.record_visit(&p, now)?;
        return Ok(ExitCode::SUCCESS);
    }
    if cli.history {
        commands::print_history(&db, &cfg, now, home.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if cli.stats {
        commands::print_stats(&db, &db_path, &cfg, now, home.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if cli.clear_history {
        let done = commands::clear_history(&db, cli.yes)?;
        return Ok(if done {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    if cli.prune {
        eprintln!("removed {} missing directories", db.prune_missing()?);
        return Ok(ExitCode::SUCCESS);
    }

    let query = cli.query_string();

    // cd semantics first: a single argument naming an existing directory is
    // a destination, not a query (unless -i or a scope flag asks to search).
    if !cli.interactive && !cli.list && !cli.has_scope_flags() && cli.query.len() == 1 {
        let target = paths::normalize(&paths::expand(cli.query[0].as_ref()), &cwd);
        if target.is_dir() {
            emit_selection(&target, cli.output_file.as_deref())?;
            return Ok(ExitCode::SUCCESS);
        }
    }

    let roots = cli::resolve_roots(&cli, &cwd, home.as_deref())?;
    let entries = history::existing(db.entries()?);
    drop(db); // don't hold the database open during the interactive session
    let mut engine = Engine::new(
        home.clone(),
        &entries,
        cfg.ranking.clone(),
        now,
        cfg.search.max_results,
    );
    let opts = ScanOptions::from(&cfg.search);

    if cli.list || cli.first {
        let mut found = Vec::new();
        let summary = filesystem::scan(&roots, &opts, &AtomicBool::new(false), |p| found.push(p));
        if summary.truncated {
            eprintln!(
                "cz: scan stopped after {} directories (search.max_scan_entries)",
                summary.dirs
            );
        }
        engine.add_fs_paths(found);
        engine.set_query(&query);
        let results = engine.results();
        if results.is_empty() {
            eprintln!("cz: no match for {query:?}");
            return Ok(ExitCode::FAILURE);
        }
        if cli.first {
            emit_selection(
                &engine.set.get(results[0].idx).path,
                cli.output_file.as_deref(),
            )?;
        } else {
            let mut out = std::io::stdout().lock();
            for r in results {
                let c = engine.set.get(r.idx);
                if cfg.ui.show_scores {
                    write!(out, "{:.3}\t{:?}\t", r.score, c.source())?;
                }
                out.write_all(c.path.as_os_str().as_bytes())?;
                out.write_all(b"\n")?;
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    let scan = filesystem::spawn(roots.clone(), opts);
    let root_names = roots
        .iter()
        .map(|r| paths::display(r, home.as_deref()))
        .collect();

    // Give the scan a moment; if it finishes and one match is clearly best,
    // jump without the picker. Otherwise the picker opens with what we have.
    let mut scanned = 0;
    let mut summary = None;
    if !cli.interactive && cfg.jump.auto && !query.trim().is_empty() {
        engine.set_query(&query);
        let bound = search::unseen_bound(&cfg.ranking);
        // History alone may already decide it: no need to wait for the scan.
        if let Some(i) = search::auto_jump(engine.results(), &cfg.jump, Some(bound)) {
            emit_selection(&engine.set.get(i).path, cli.output_file.as_deref())?;
            return Ok(ExitCode::SUCCESS);
        }
        let deadline = Instant::now() + Duration::from_millis(cfg.jump.wait_ms);
        let mut found = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match scan.rx.recv_timeout(left) {
                Ok(ScanEvent::Batch(b)) => found.extend(b),
                Ok(ScanEvent::Done(s)) => {
                    summary = Some(s);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    summary = Some(Default::default());
                    break;
                }
            }
        }
        scanned = found.len();
        engine.add_fs_paths(found);
        let unseen = if summary.is_some() { None } else { Some(bound) };
        if let Some(i) = search::auto_jump(engine.results(), &cfg.jump, unseen) {
            emit_selection(&engine.set.get(i).path, cli.output_file.as_deref())?;
            return Ok(ExitCode::SUCCESS);
        }
    }

    let mut app = App::new(engine, &query, root_names, summary.is_none());
    app.scanned = scanned;
    if let Some(s) = summary {
        app.scan_done(s);
    }
    app.show_scores = cfg.ui.show_scores;
    app.show_preview = cfg.ui.show_preview;
    match tui::run(app, Some(scan))? {
        Outcome::Selected(p) => {
            emit_selection(&p, cli.output_file.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Outcome::Cancelled => Ok(ExitCode::FAILURE),
        Outcome::Interrupted => Ok(ExitCode::from(130)),
    }
}
