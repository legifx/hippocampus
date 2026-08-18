//! The storage layer: one SQLite file holding everything authoritative.
//!
//! Concurrency: the store wraps a single connection behind a mutex. That is correct but it
//! serialises readers against the ingest writer, which is not what WAL mode is for. Replacing
//! it with a reader pool plus a single writer is queued for M2, where content extraction makes
//! the writer busy enough for the contention to matter. Do not build a second concurrency model
//! on top of this one; replace it.
//!
//! Bulk writes must go through [`Store::batch`]. SQLite makes every statement its own durable
//! transaction otherwise, and that barrier — not the work — dominates a large index run.

pub mod chunks;
pub mod embeddings;
pub mod error;
pub mod migrations;
pub mod queries;
pub mod vectors;

use cortex_core::paths;
use error::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub use error::StoreError;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if necessary) the database at `path` and migrate it to the current schema.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        migrations::migrate(&mut conn)?;

        // SQLite creates the database and its sidecar files itself, so the mode is tightened
        // afterwards rather than at creation. Rule R4.
        paths::restrict_file(path)?;
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            paths::restrict_file(Path::new(&sidecar))?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory store. Tests only.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Borrow the connection.
    ///
    /// A poisoned mutex means another thread panicked mid-transaction. SQLite has already
    /// rolled that transaction back, so the data is consistent and recovering the guard is the
    /// right call — refusing to continue would turn one panic into a dead daemon.
    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn schema_version(&self) -> Result<i64> {
        migrations::current_version(&self.conn())
    }
}

/// Pragmas applied to every connection.
fn configure(conn: &Connection) -> Result<()> {
    // WAL lets readers run while a writer is active, which is the entire access pattern here:
    // one ingest writer, many short query readers.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // NORMAL is the right trade in WAL mode: a power loss can cost the last transactions, and
    // the last transactions are re-derivable from the filesystem. Correctness of the *file*
    // is still guaranteed.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // Without this, a concurrent writer produces SQLITE_BUSY instead of waiting.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // Negative values are a size in KiB. 64 MiB of page cache is well inside the daemon's
    // memory budget and removes most of the read I/O for a corpus of this size.
    conn.pragma_update(None, "cache_size", -65536)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_creates_a_migrated_database() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("cortex.db")).unwrap();
        assert_eq!(store.schema_version().unwrap(), migrations::target_version());
    }

    #[test]
    fn opening_twice_reuses_the_existing_database() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.db");
        drop(Store::open(&path).unwrap());
        let store = Store::open(&path).expect("reopening must work");
        assert_eq!(store.schema_version().unwrap(), migrations::target_version());
    }

    #[test]
    fn opening_creates_missing_parent_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("deep").join("nested").join("cortex.db");
        Store::open(&path).unwrap();
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_database_file_is_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.db");
        let _store = Store::open(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "database has mode {mode:o}, expected 600");
    }

    #[test]
    fn wal_and_foreign_keys_are_actually_enabled() {
        // These pragmas are load-bearing and silently ignorable, so assert rather than assume.
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1, "foreign keys must be on");
    }

    #[test]
    fn wal_mode_is_set_on_a_file_backed_database() {
        // In-memory databases cannot use WAL, so this has to be checked on a real file.
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("cortex.db")).unwrap();
        let conn = store.conn();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }
}
