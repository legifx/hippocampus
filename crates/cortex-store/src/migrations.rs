//! Forward-only schema migrations.
//!
//! There is no down-migration and there will not be one. A rollback path for an index that is
//! reconstructible from the filesystem is dead weight: if a migration goes wrong, the correct
//! recovery is to delete the index and rebuild it, which is a supported operation anyway.

use crate::error::{Result, StoreError};
use rusqlite::Connection;

/// One migration. `sql` is applied inside a transaction together with the version bump.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial",
    sql: include_str!("schema/0001_initial.sql"),
}];

/// The schema version this build expects.
pub fn target_version() -> i64 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

/// Read the schema version recorded in the database. A fresh database reports 0.
pub fn current_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Bring the database up to `target_version()`.
///
/// Refuses to touch a database written by a newer build. Silently downgrading a schema is a
/// good way to lose data that the older code does not know about.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    let target = target_version();

    if current > target {
        return Err(StoreError::SchemaFromFuture {
            found: current,
            supported: target,
        });
    }
    if current == target {
        return Ok(());
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applying migration"
        );
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        // `PRAGMA user_version` does not accept a bound parameter.
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn migrating_a_fresh_database_reaches_the_target_version() {
        let conn = fresh();
        assert_eq!(current_version(&conn).unwrap(), target_version());
    }

    #[test]
    fn migration_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).expect("re-running migrations must be a no-op");
        assert_eq!(current_version(&conn).unwrap(), target_version());
    }

    #[test]
    fn every_expected_table_exists() {
        let conn = fresh();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view')")
            .unwrap();
        let found: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        for table in [
            "nodes",
            "edges",
            "episodes",
            "chunks",
            "chunks_fts",
            "embeddings",
            "zones",
            "sync_outbox",
            "sync_state",
            "salience_log",
            "undo_journal",
            "audit_log",
            "meta",
        ] {
            assert!(found.contains(&table.to_string()), "missing table: {table}");
        }
    }

    #[test]
    fn a_database_from_a_newer_build_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", target_version() + 5)
            .unwrap();
        let err = migrate(&mut conn).unwrap_err();
        assert!(
            matches!(err, StoreError::SchemaFromFuture { .. }),
            "expected a refusal, got {err:?}"
        );
    }

    #[test]
    fn a_failed_migration_leaves_no_partial_schema() {
        // Each migration runs inside a transaction, so a syntax error part-way through must
        // roll the whole thing back rather than leaving half a schema behind.
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        let outcome = tx.execute_batch("CREATE TABLE ok (a); THIS IS NOT SQL;");
        assert!(outcome.is_err());
        drop(tx); // implicit rollback

        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'ok'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "a rolled-back migration must leave nothing behind");
    }

    #[test]
    fn node_check_constraints_reject_nonsense() {
        let conn = fresh();
        // An invalid tier must be refused by the database, not merely by application code —
        // the database is the last line of defence when a future writer has a bug.
        let outcome = conn.execute(
            "INSERT INTO nodes (kind, path, tier, first_seen, last_seen)
             VALUES ('file', '/x', 'lukewarm', 0, 0)",
            [],
        );
        assert!(outcome.is_err(), "an unknown tier must be rejected");
    }

    #[test]
    fn paths_are_unique_but_pathless_nodes_are_not_constrained() {
        let conn = fresh();
        conn.execute(
            "INSERT INTO nodes (kind, path, first_seen, last_seen) VALUES ('file', '/a', 0, 0)",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO nodes (kind, path, first_seen, last_seen) VALUES ('file', '/a', 0, 0)",
                [],
            )
            .is_err(),
            "the same path must not be indexed twice"
        );

        // Concepts and annotations have no path, and there may be any number of them.
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO nodes (kind, path, first_seen, last_seen)
                 VALUES ('concept', NULL, 0, 0)",
                [],
            )
            .unwrap();
        }
    }

    #[test]
    fn episodes_survive_deletion_of_their_node() {
        // Episodes are the ground truth. An episode about a file that no longer exists is the
        // whole point of an episodic memory, so it must not cascade away.
        let conn = fresh();
        conn.execute(
            "INSERT INTO nodes (id, kind, path, first_seen, last_seen)
             VALUES (1, 'file', '/gone', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO episodes (ts, actor, action, node_id) VALUES (0, 'human', 'open', 1)",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = 1", []).unwrap();

        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1, "deleting a node must not delete its history");
    }

    #[test]
    fn chunks_cascade_when_their_node_goes_away() {
        // Chunks, unlike episodes, are derived data and must not outlive their source.
        let conn = fresh();
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
        conn.execute(
            "INSERT INTO nodes (id, kind, path, first_seen, last_seen)
             VALUES (1, 'file', '/f', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (node_id, ordinal, byte_start, byte_end, hash)
             VALUES (1, 0, 0, 10, 'h')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = 1", []).unwrap();

        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn the_lamport_clock_starts_at_zero() {
        let conn = fresh();
        let value: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'lamport'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(value, "0");
    }
}
