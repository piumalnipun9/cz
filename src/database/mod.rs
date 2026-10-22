//! Persistent visit history in SQLite.
//!
//! Location: `$CZ_DATA_DIR/history.db`, else `$XDG_DATA_HOME/cz/history.db`,
//! else `~/.local/share/cz/history.db`.

mod migrations;
pub mod models;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use models::{HistoryEntry, Stats};

pub struct Database {
    conn: Connection,
}

pub fn default_path() -> Result<PathBuf> {
    let dir = if let Some(d) = std::env::var_os("CZ_DATA_DIR").filter(|v| !v.is_empty()) {
        PathBuf::from(d)
    } else if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(d).join("cz")
    } else {
        crate::paths::home_dir()
            .context("cannot determine data directory ($HOME unset)")?
            .join(".local/share/cz")
    };
    Ok(dir.join("history.db"))
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut conn = Connection::open(path)
            .with_context(|| format!("opening history database {}", path.display()))?;
        // Concurrent shells may record at the same time.
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrations::run(&mut conn).context("migrating history database")?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        migrations::run(&mut conn)?;
        Ok(Self { conn })
    }

    /// Records one successful visit. `path` must already be normalized.
    pub fn record_visit(&self, path: &Path, now: i64) -> Result<()> {
        let is_git = path.join(".git").exists();
        self.conn.execute(
            "INSERT INTO dirs (path, visits, last_visited, first_seen, is_git)
             VALUES (?1, 1, ?2, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET
                visits = visits + 1, last_visited = excluded.last_visited, is_git = excluded.is_git",
            params![path.as_os_str().as_bytes(), now, is_git],
        )?;
        Ok(())
    }

    pub fn entries(&self) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT path, visits, last_visited, first_seen, is_git FROM dirs")?;
        let rows = stmt.query_map([], |r| {
            Ok(HistoryEntry {
                path: PathBuf::from(std::ffi::OsString::from_vec(r.get(0)?)),
                visits: r.get::<_, i64>(1)? as u64,
                last_visited: r.get(2)?,
                first_seen: r.get(3)?,
                is_git: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn remove(&self, path: &Path) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM dirs WHERE path = ?1",
            [path.as_os_str().as_bytes()],
        )? > 0)
    }

    pub fn clear(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM dirs", [])?)
    }

    /// Deletes entries whose directory no longer exists; returns how many.
    pub fn prune_missing(&self) -> Result<usize> {
        let mut n = 0;
        for e in self.entries()? {
            if !e.path.is_dir() && self.remove(&e.path)? {
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn stats(&self) -> Result<Stats> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(visits), 0), MIN(first_seen) FROM dirs",
            [],
            |r| {
                Ok(Stats {
                    directories: r.get::<_, i64>(0)? as u64,
                    total_visits: r.get::<_, i64>(1)? as u64,
                    oldest: r.get(2)?,
                })
            },
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_increments_and_updates_timestamps() {
        let db = Database::open_in_memory().unwrap();
        db.record_visit(Path::new("/a b/c"), 100).unwrap();
        db.record_visit(Path::new("/a b/c"), 200).unwrap();
        db.record_visit(Path::new("/x"), 150).unwrap();
        let mut e = db.entries().unwrap();
        e.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(e[0].path, PathBuf::from("/a b/c"));
        assert_eq!(
            (e[0].visits, e[0].first_seen, e[0].last_visited),
            (2, 100, 200)
        );
        let s = db.stats().unwrap();
        assert_eq!((s.directories, s.total_visits, s.oldest), (2, 3, Some(100)));
        assert_eq!(db.clear().unwrap(), 2);
        assert!(db.entries().unwrap().is_empty());
    }

    #[test]
    fn non_utf8_paths_round_trip() {
        use std::ffi::OsStr;
        let db = Database::open_in_memory().unwrap();
        let p = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff\xfe weird\nname"));
        db.record_visit(&p, 1).unwrap();
        assert_eq!(db.entries().unwrap()[0].path, p);
    }

    #[test]
    fn prune_removes_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open(&tmp.path().join("db/history.db")).unwrap();
        db.record_visit(tmp.path(), 1).unwrap();
        db.record_visit(&tmp.path().join("gone"), 1).unwrap();
        assert_eq!(db.prune_missing().unwrap(), 1);
        assert_eq!(db.entries().unwrap().len(), 1);
    }
}
