//! Schema migrations keyed on SQLite's `user_version` pragma. Each entry
//! upgrades from version `i` to `i + 1`; never edit a released migration.

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // v1: paths are stored as raw bytes so non-UTF-8 names round-trip exactly.
    "CREATE TABLE dirs (
        path         BLOB PRIMARY KEY NOT NULL,
        visits       INTEGER NOT NULL,
        last_visited INTEGER NOT NULL,
        first_seen   INTEGER NOT NULL,
        is_git       INTEGER NOT NULL DEFAULT 0
    ) WITHOUT ROWID;",
];

pub fn run(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
