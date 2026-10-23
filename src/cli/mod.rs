//! Command-line parsing and search-scope resolution.
//!
//! Scope flags are independent and *additive*: `-r`, `-H` and every `-p PATH`
//! each contribute one root, and the union is searched. With no scope flag the
//! current directory is the only root. Roots nested inside another root are
//! dropped (they would only produce duplicates), so combining flags can never
//! search more than the user named explicitly.

use anyhow::{Result, bail};
use clap::{ArgGroup, Parser, ValueEnum};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::paths;

#[derive(Debug, Parser)]
#[command(
    name = "cz",
    version,
    about = "Jump to directories by partial name, using learned history and scoped filesystem search",
    after_help = "Scope flags combine: `cz -H -p /srv q` searches ~ and /srv.\n\
                  History is always searched globally, regardless of scope.\n\
                  Use `--` before a query that starts with '-'.",
    group(ArgGroup::new("action").args(["history", "clear_history", "stats", "init", "record", "prune"]))
)]
pub struct Cli {
    /// Search from the filesystem root (/), skipping virtual filesystems
    #[arg(short = 'r', long = "root")]
    pub root: bool,

    /// Search from the current user's home directory
    #[arg(short = 'H', long = "home")]
    pub home: bool,

    /// Search below PATH (repeatable)
    #[arg(short = 'p', long = "path", value_name = "PATH", num_args = 1, action = clap::ArgAction::Append)]
    pub paths: Vec<OsString>,

    /// Also search network and FUSE mounts (sshfs, NFS, ...), skipped by default
    #[arg(short = 'm', long)]
    pub mounts: bool,

    /// Print visit history, most relevant first
    #[arg(long)]
    pub history: bool,

    /// Delete all visit history
    #[arg(long)]
    pub clear_history: bool,

    /// Show history statistics
    #[arg(long)]
    pub stats: bool,

    /// Remove history entries whose directories no longer exist
    #[arg(long)]
    pub prune: bool,

    /// Print the shell integration script
    #[arg(long, value_name = "SHELL")]
    pub init: Option<Shell>,

    /// Record a visit to DIR (called by the shell function after a successful cd)
    #[arg(long, value_name = "DIR")]
    pub record: Option<OsString>,

    /// Always open the picker and include files (used by the `czi` function)
    #[arg(long, hide = true)]
    pub pick: bool,

    /// Print ranked matches instead of opening the interactive UI
    #[arg(long)]
    pub list: bool,

    /// Pick the best match without opening the interactive UI
    #[arg(long)]
    pub first: bool,

    /// Skip confirmation prompts
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Write the selected path to FILE instead of stdout (used by the shell function)
    #[arg(long, value_name = "FILE", hide = true)]
    pub output_file: Option<PathBuf>,

    /// Search query; multiple words are matched independently
    #[arg(value_name = "QUERY")]
    pub query: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
}

impl Cli {
    pub fn query_string(&self) -> String {
        self.query.join(" ")
    }

    pub fn has_scope_flags(&self) -> bool {
        self.root || self.home || !self.paths.is_empty()
    }
}

/// Resolves scope flags to a validated, deduplicated list of absolute roots.
pub fn resolve_roots(cli: &Cli, cwd: &Path, home: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if cli.root {
        roots.push(PathBuf::from("/"));
    }
    if cli.home {
        match home {
            Some(h) => roots.push(paths::normalize(h, cwd)),
            None => bail!("-H/--home: cannot determine the home directory ($HOME is unset)"),
        }
    }
    for raw in &cli.paths {
        if raw.is_empty() {
            bail!("-p/--path: empty path");
        }
        let p = paths::normalize(&paths::expand(raw), cwd);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => roots.push(p),
            Ok(_) => bail!("-p {}: not a directory", p.display()),
            Err(e) => bail!("-p {}: {}", p.display(), e),
        }
    }
    if roots.is_empty() {
        roots.push(paths::normalize(cwd, cwd));
    }
    Ok(dedup_roots(roots))
}

/// Removes duplicate roots and roots nested inside another root, keeping
/// the order in which the remaining roots were given.
pub fn dedup_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (i, r) in roots.iter().enumerate() {
        let covered = roots
            .iter()
            .enumerate()
            .any(|(j, other)| (other != r && paths::is_within(r, other)) || (other == r && j < i));
        if !covered {
            out.push(r.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("cz").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn positional_query_and_repeated_paths() {
        let c = parse(&["-p", "/a b", "-p", "/c", "CNN", "frontend"]);
        assert_eq!(c.paths, vec![OsString::from("/a b"), OsString::from("/c")]);
        assert_eq!(c.query_string(), "CNN frontend");
    }

    #[test]
    fn flags_after_query_and_long_forms() {
        let c = parse(&["CNN", "--home"]);
        assert!(c.home);
        assert_eq!(c.query_string(), "CNN");
        let c = parse(&["--root", "--path", "/x", "q"]);
        assert!(c.root);
        assert_eq!(c.paths.len(), 1);
    }

    #[test]
    fn dash_dash_allows_dash_queries() {
        let c = parse(&["--", "-weird"]);
        assert_eq!(c.query_string(), "-weird");
    }

    #[test]
    fn short_h_is_help_capital_h_is_home() {
        let err = Cli::try_parse_from(["cz", "-h"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
        assert!(parse(&["-H"]).home);
    }

    #[test]
    fn actions_conflict() {
        assert!(Cli::try_parse_from(["cz", "--history", "--stats"]).is_err());
    }

    #[test]
    fn dedup_drops_nested_and_duplicate_roots() {
        let r = dedup_roots(vec![
            "/a/b".into(),
            "/a".into(),
            "/c".into(),
            "/a".into(),
            "/c/d".into(),
        ]);
        assert_eq!(r, vec![PathBuf::from("/a"), PathBuf::from("/c")]);
        let r = dedup_roots(vec!["/home/u".into(), "/".into()]);
        assert_eq!(r, vec![PathBuf::from("/")]);
    }

    #[test]
    fn default_root_is_cwd_and_bad_paths_error() {
        let tmp = tempfile::tempdir().unwrap();
        let c = parse(&["q"]);
        assert_eq!(
            resolve_roots(&c, tmp.path(), None).unwrap(),
            vec![tmp.path().to_path_buf()]
        );
        let c = parse(&["-p", "/definitely/not/here", "q"]);
        assert!(resolve_roots(&c, tmp.path(), None).is_err());
        let c = parse(&["-p", "sub/..", "q"]);
        assert_eq!(
            resolve_roots(&c, tmp.path(), None).unwrap(),
            vec![tmp.path().to_path_buf()]
        );
    }
}
