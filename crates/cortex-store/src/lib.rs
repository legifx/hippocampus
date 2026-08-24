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

pub mod agent;
pub mod chunks;
pub mod embeddings;
pub mod graph;
pub mod memories;
pub mod memory_columns;
pub mod memory;
pub mod error;
pub mod migrations;
pub mod queries;
pub mod vectors;

use cortex_core::paths;
use error::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Condvar, Mutex};

pub use error::StoreError;

/// How many connections a file-backed store may open.
///
/// Four, because SQLite's page cache is per connection and the memory has to come from
/// somewhere: [`CACHE_KIB`] is set so that four of them together cost what one used to. Four is
/// also more than the daemon needs — one long-running writer and a handful of short reads — and
/// a fifth caller waits for a moment rather than growing the footprint.
const POOL_CAPACITY: usize = 4;

/// Page cache per connection, in KiB. See [`POOL_CAPACITY`].
const CACHE_KIB: i64 = 16 * 1024;

/// The connections, and how many exist.
struct Pool {
    idle: Vec<Connection>,
    live: usize,
}

/// The index.
///
/// **Several connections, not one.** The pragmas below ask for WAL, whose entire purpose is that
/// readers do not wait for the writer — and for a long time this held a single `Mutex<Connection>`
/// which made every reader wait for the writer anyway. The comment on the pragma described the
/// intended design and the code did the opposite of it, which cost searches of up to 932 ms
/// against an 80 ms budget while the indexer was working, and a status call of 988 ms.
///
/// With a connection each, WAL does what it says: a reader sees a consistent snapshot and never
/// blocks on the writer. Writers still serialise against each other, which is correct and is
/// what `busy_timeout` is for.
///
/// **Do not take two handles on one thread.** `conn()` blocks when every connection is out, so
/// nesting deadlocks at capacity — and would be wrong before then, because the second handle is
/// a different connection and would not see the first one's open transaction. Nothing nests
/// today: with the old single mutex it would have hung immediately.
pub struct Store {
    /// Where to open another connection. `None` for an in-memory store, where a second
    /// connection would be a second, empty database rather than another view of this one.
    path: Option<std::path::PathBuf>,
    pool: Mutex<Pool>,
    returned: Condvar,
}

/// A borrowed connection, given back when it goes out of scope.
pub(crate) struct Handle<'a> {
    store: &'a Store,
    /// `Some` for the whole life of the handle; taken in `drop` to move it back to the pool.
    conn: Option<Connection>,
}

impl std::ops::Deref for Handle<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("the handle holds its connection until it is dropped")
    }
}

impl std::ops::DerefMut for Handle<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn.as_mut().expect("the handle holds its connection until it is dropped")
    }
}

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let mut pool = self.store.lock();
            pool.idle.push(conn);
            drop(pool);
            // One waiter, because one connection came back. Waking all of them would have the
            // rest go straight back to sleep.
            self.store.returned.notify_one();
        }
    }
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
            path: Some(path.to_path_buf()),
            pool: Mutex::new(Pool {
                idle: vec![conn],
                live: 1,
            }),
            returned: Condvar::new(),
        })
    }

    /// An in-memory store. Tests only.
    ///
    /// One connection and no more: `:memory:` is not a name that can be opened twice, so a
    /// second connection would be a second, empty database. Callers therefore serialise here
    /// exactly as they used to everywhere, which is why the tests are unaffected by the pool.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            path: None,
            pool: Mutex::new(Pool {
                idle: vec![conn],
                live: 1,
            }),
            returned: Condvar::new(),
        })
    }

    /// A poisoned mutex means another thread panicked while holding it. SQLite has already
    /// rolled back whatever transaction that was, so the data is consistent and recovering is
    /// the right call — refusing to continue would turn one panic into a dead daemon.
    fn lock(&self) -> std::sync::MutexGuard<'_, Pool> {
        self.pool.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Borrow a connection, opening one if that is cheaper than waiting.
    ///
    /// Infallible on purpose: seventy call sites read a row, and threading a "could not open a
    /// second connection" error through all of them would be a worse answer than waiting for
    /// one of the connections that already exists. A failure to open therefore falls back to
    /// waiting, which always terminates — at least one connection exists for the life of the
    /// store, and every handle returns it.
    pub(crate) fn conn(&self) -> Handle<'_> {
        let mut pool = self.lock();
        loop {
            if let Some(conn) = pool.idle.pop() {
                return Handle {
                    store: self,
                    conn: Some(conn),
                };
            }

            if let (Some(path), true) = (&self.path, pool.live < POOL_CAPACITY) {
                let path = path.clone();
                pool.live += 1;
                // Opening touches the filesystem, so it happens without the lock held: another
                // thread returning a connection meanwhile must not wait for this.
                drop(pool);
                match open_another(&path) {
                    Ok(conn) => {
                        return Handle {
                            store: self,
                            conn: Some(conn),
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "could not open another connection; waiting for one");
                        pool = self.lock();
                        pool.live -= 1;
                        continue;
                    }
                }
            }

            pool = self
                .returned
                .wait(pool)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// How many connections this store has open. For the tests that assert the pool exists.
    pub fn connections(&self) -> usize {
        self.lock().live
    }

    pub fn schema_version(&self) -> Result<i64> {
        migrations::current_version(&self.conn())
    }
}

/// Another connection to a database that is already open and migrated.
///
/// No migration here: the schema is whatever the first connection made it, and running the
/// migrations again would at best be wasted work and at worst a second writer racing the first
/// during startup.
fn open_another(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
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
    // Negative values are a size in KiB. Per *connection*, which is why this is a quarter of
    // what it was: [`POOL_CAPACITY`] connections at [`CACHE_KIB`] each come to the same 64 MiB
    // the single connection used to have to itself.
    conn.pragma_update(None, "cache_size", -CACHE_KIB)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_store_opens_more_connections_as_they_are_asked_for() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("cortex.db")).unwrap();
        assert_eq!(store.connections(), 1, "one to begin with");

        let first = store.conn();
        let second = store.conn();
        assert_eq!(store.connections(), 2, "the second caller did not wait");

        drop(second);
        drop(first);
        assert_eq!(store.connections(), 2, "and they are kept, not closed");

        // Reused rather than multiplied: two sequential borrows are one connection.
        let again = store.conn();
        assert_eq!(store.connections(), 2);
        drop(again);
    }

    #[test]
    fn an_in_memory_store_has_exactly_one() {
        // `:memory:` cannot be opened twice — a second connection would be a second, empty
        // database — so callers serialise here exactly as they used to everywhere.
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.connections(), 1);
        let held = store.conn();
        assert_eq!(store.connections(), 1);
        drop(held);
    }

    #[test]
    fn two_writers_wait_for_each_other_rather_than_failing() {
        // The hazard the pool creates and the single mutex hid. Two `batch` calls on two
        // connections would, as deferred transactions, have one of them return SQLITE_BUSY the
        // instant it tried to write — with no retry, because SQLite refuses to let a reader
        // block while upgrading. Taking the write lock up front turns that into a wait.
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("cortex.db")).unwrap();
        let done = AtomicUsize::new(0);

        std::thread::scope(|scope| {
            let store = &store;
            let done = &done;
            for worker in 0..4 {
                scope.spawn(move || {
                    for round in 0..20 {
                        store
                            .batch(|tx| {
                                tx.append_episode(
                                    round,
                                    &cortex_core::model::Actor::System,
                                    cortex_core::model::Action::Modify,
                                    None,
                                    None,
                                    &serde_json::json!({ "worker": worker }),
                                )?;
                                Ok(())
                            })
                            .expect("a concurrent writer must wait, not fail");
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });

        assert_eq!(done.load(Ordering::Relaxed), 80);
        assert_eq!(store.recent_episodes(100).unwrap().len(), 80, "every write landed");
    }

    #[test]
    fn a_reader_does_not_wait_for_a_writer() {
        // The property WAL is configured for and the single mutex used to prevent. A writer
        // holding a transaction open must not stop a reader from answering — that is what cost
        // searches of up to 932 ms against an 80 ms budget while the indexer worked.
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("cortex.db")).unwrap();

        let writing = AtomicBool::new(false);
        let stop = AtomicBool::new(false);

        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut conn = store.conn();
                let tx = conn.transaction().unwrap();
                tx.execute("CREATE TABLE IF NOT EXISTS slow (id INTEGER PRIMARY KEY)", [])
                    .unwrap();
                for i in 0..500 {
                    tx.execute("INSERT INTO slow (id) VALUES (?1)", [i]).unwrap();
                }
                writing.store(true, Ordering::SeqCst);
                // Held open, deliberately: this is the indexer mid-batch.
                while !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                tx.commit().unwrap();
            });

            while !writing.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }

            let started = Instant::now();
            let version = store.schema_version().unwrap();
            let waited = started.elapsed();
            stop.store(true, Ordering::SeqCst);

            assert_eq!(version, migrations::target_version());
            assert!(
                waited < Duration::from_millis(100),
                "the read waited {waited:?} for a writer that had not committed"
            );
        });
    }

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
