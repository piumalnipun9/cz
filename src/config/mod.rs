//! TOML configuration with defaults for every field, so cz works without a file.
//!
//! Lookup order: `$CZ_CONFIG`, `$XDG_CONFIG_HOME/cz/config.toml`,
//! `~/.config/cz/config.toml`. Search roots are deliberately *not* configurable
//! here; they come only from CLI flags.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub search: SearchConfig,
    pub ranking: RankingConfig,
    pub jump: JumpConfig,
    pub ui: UiConfig,
}

/// When a search jumps straight to the best match instead of opening the picker.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JumpConfig {
    /// Jump without the picker when the best match is clearly ahead.
    pub auto: bool,
    /// The best score must be at least `margin` × the runner-up's score.
    pub margin: f64,
    /// The best match's quality (0..=1, see search::fuzzy) must reach this.
    pub min_quality: f64,
    /// How long to wait for the filesystem scan before falling back to the picker.
    pub wait_ms: u64,
}

impl Default for JumpConfig {
    fn default() -> Self {
        Self {
            auto: true,
            margin: 1.25,
            min_quality: 0.6,
            wait_ms: 400,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    pub include_hidden: bool,
    /// Maximum depth below each root (1 = direct children only).
    pub max_depth: usize,
    /// Maximum number of ranked results shown / printed.
    pub max_results: usize,
    pub follow_symlinks: bool,
    /// Hard cap on directories collected per invocation (protects memory on `-r`).
    pub max_scan_entries: usize,
    pub exclusions: Exclusions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Exclusions {
    /// Directory *names* that are neither listed nor descended into.
    pub directories: Vec<String>,
    /// Absolute paths never descended into (virtual filesystems). A path given
    /// explicitly as a root is still searched.
    pub paths: Vec<PathBuf>,
    /// Mount points of these filesystem types are never entered (virtual,
    /// network and FUSE filesystems). A trailing `*` matches a prefix.
    pub filesystem_types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RankingConfig {
    pub match_weight: f64,
    pub frequency_weight: f64,
    pub recency_weight: f64,
    /// Maximum fraction a single history signal can add on top of the match score.
    pub history_boost: f64,
    /// Visits at which the frequency signal saturates at 1.0.
    pub frequency_saturation: u32,
    /// Recency signal halves every this many days.
    pub recency_half_life_days: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub show_scores: bool,
    /// Show a subdirectory preview pane when the terminal is wide enough.
    pub show_preview: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            include_hidden: true,
            max_depth: 15,
            max_results: 100,
            follow_symlinks: false,
            max_scan_entries: 2_000_000,
            exclusions: Exclusions::default(),
        }
    }
}

impl Default for Exclusions {
    fn default() -> Self {
        let dirs = [
            ".git",
            ".hg",
            ".svn",
            "node_modules",
            "target",
            ".cache",
            ".venv",
            "venv",
            "__pycache__",
            ".tox",
            ".mypy_cache",
            ".pytest_cache",
        ];
        Self {
            directories: dirs.iter().map(|s| s.to_string()).collect(),
            paths: ["/proc", "/sys", "/dev", "/run"]
                .iter()
                .map(PathBuf::from)
                .collect(),
            filesystem_types: [
                "proc",
                "sysfs",
                "devtmpfs",
                "devpts",
                "cgroup",
                "cgroup2",
                "debugfs",
                "tracefs",
                "securityfs",
                "pstore",
                "bpf",
                "configfs",
                "mqueue",
                "hugetlbfs",
                "binfmt_misc",
                "fusectl",
                "efivarfs",
                "nsfs",
                "autofs",
                "squashfs",
                "nfs",
                "nfs4",
                "cifs",
                "smb3",
                "smbfs",
                "sshfs",
                "fuse.*",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            match_weight: 1.0,
            frequency_weight: 1.0,
            recency_weight: 1.0,
            history_boost: 0.25,
            frequency_saturation: 100,
            recency_half_life_days: 7.0,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_scores: false,
            show_preview: true,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CZ_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("cz").join("config.toml"))
}

impl Config {
    /// Loads the config file if present; a missing file yields defaults, a
    /// malformed one is an error (silently ignoring typos would be confusing).
    pub fn load() -> Result<Self> {
        match config_path() {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading config {}", p.display()))?;
                Self::parse(&text).with_context(|| format!("parsing config {}", p.display()))
            }
            _ => Ok(Self::default()),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_defaults() {
        let c =
            Config::parse("[search]\nmax_depth = 3\n[ranking]\nrecency_weight = 0.5\n").unwrap();
        assert_eq!(c.search.max_depth, 3);
        assert!(c.search.include_hidden);
        assert_eq!(c.ranking.recency_weight, 0.5);
        assert_eq!(c.ranking.match_weight, 1.0);
    }

    #[test]
    fn spec_example_parses() {
        let c = Config::parse(
            r#"
[search]
include_hidden = true
max_depth = 15
max_results = 100
[search.exclusions]
directories = [".git", "node_modules"]
[ranking]
match_weight = 1.0
frequency_weight = 1.0
recency_weight = 1.0
[ui]
show_scores = false
show_preview = true
"#,
        )
        .unwrap();
        assert_eq!(c.search.exclusions.directories.len(), 2);
        assert!(!c.search.exclusions.paths.is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("[search]\nmax_dpeth = 3\n").is_err());
    }
}
