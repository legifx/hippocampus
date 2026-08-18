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

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("schema/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "fts_contentless_delete",
        sql: include_str!("schema/0002_fts_contentless_delete.sql"),
    },
];

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
    fn the_lexical_index_can_delete_without_being_given_the_text_back() {
        // Decision D16 keeps no copy of chunk text, so a plain contentless FTS5 table could
        // never remove a row for a file that changed — the index would only ever grow.
        let conn = fresh();
        conn.execute("INSERT INTO chunks_fts(rowid, text) VALUES (1, 'kamerawinkel vergleich')", [])
            .unwrap();

        let hits: i64 = conn
            .query_row("SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'kamerawinkel'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hits, 1);

        conn.execute("DELETE FROM chunks_fts WHERE rowid = 1", []).unwrap();

        let hits: i64 = conn
            .query_row("SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'kamerawinkel'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hits, 0, "a stale row survived deletion");
    }

    #[test]
    fn umlauts_are_folded_so_a_query_without_them_still_matches() {
        // Users type "anderungen" and mean "Änderungen". `remove_diacritics 2` makes that work.
        let conn = fresh();
        conn.execute(
            "INSERT INTO chunks_fts(rowid, text) VALUES (1, 'Änderungen an der Übersicht')",
            [],
        )
        .unwrap();

        for query in ["anderungen", "Anderungen", "ubersicht"] {
            let hits: i64 = conn
                .query_row(
                    "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                    [query],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(hits, 1, "{query:?} should match through diacritic folding");
        }
    }

    #[test]
    fn eszett_is_not_folded_to_ss_and_this_is_a_known_gap() {
        // `remove_diacritics` strips diacritics. ß carries none — it is a distinct letter — so
        // "grosse" does not find "große". Closing this needs either a custom tokenizer or a
        // query-side expansion, and it matters for the German half of the M2 goldset. Asserted
        // here so the limitation is recorded rather than discovered later as a recall mystery.
        let conn = fresh();
        conn.execute("INSERT INTO chunks_fts(rowid, text) VALUES (1, 'größere Datei')", [])
            .unwrap();

        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'grossere'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0, "if this now matches, the gap is closed — update the goldset notes");
    }

    #[test]
    fn extraction_state_defaults_to_pending_and_is_constrained() {
        let conn = fresh();
        conn.execute(
            "INSERT INTO nodes (kind, path, first_seen, last_seen) VALUES ('file', '/x', 0, 0)",
            [],
        )
        .unwrap();

        let state: String = conn
            .query_row("SELECT extract_state FROM nodes WHERE path = '/x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "pending");

        assert!(
            conn.execute("UPDATE nodes SET extract_state = 'maybe' WHERE path = '/x'", [])
                .is_err(),
            "an unknown extraction state must be rejected by the database"
        );
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
