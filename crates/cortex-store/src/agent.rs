//! The two tables the Agent Surface writes to: the audit log (R8) and the undo journal (R6).
//!
//! Both are deliberately separate from `episodes`. An episode says *what happened to the
//! filesystem*; an audit entry says *who asked, and whether they were allowed*. A refused call
//! changes nothing and therefore has no episode, which is exactly the call one most wants to
//! see afterwards — so it needs a log of its own.
//!
//! Neither table is reconstructible from the filesystem, and that is allowed under R10 only
//! because neither is an index: dropping them loses history, not function. `clear_derived`
//! leaves both alone for that reason.

use crate::error::{Result, StoreError};
use crate::Store;
use rusqlite::{params, OptionalExtension};

pub use cortex_core::model::{AuditEntry, UndoEntry};

impl Store {
    /// Record one call to the Agent Surface. Returns the entry id.
    ///
    /// This is called on the refusal path too, and must therefore never itself refuse: a
    /// failure to write the audit row is propagated to the caller, because a surface that
    /// cannot be audited must not serve.
    pub fn append_audit(
        &self,
        ts: i64,
        actor: &str,
        tool: &str,
        args: &serde_json::Value,
        result_summary: Option<&str>,
        allowed: bool,
    ) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO audit_log (ts, actor, tool, args_json, result_summary, allowed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts,
                actor,
                tool,
                serde_json::to_string(args)?,
                result_summary,
                allowed as i64
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The most recent audit entries, newest first.
    ///
    /// `actor` and `refused_only` narrow the answer in SQL rather than in the caller, because
    /// "show me every refusal" over a long-running index must not have to read the whole log.
    pub fn recent_audit(
        &self,
        limit: usize,
        actor: Option<&str>,
        refused_only: bool,
    ) -> Result<Vec<AuditEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, tool, args_json, result_summary, allowed
             FROM audit_log
             WHERE (?1 IS NULL OR actor = ?1)
               AND (?2 = 0 OR allowed = 0)
             ORDER BY ts DESC, id DESC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![actor, refused_only as i64, limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (id, ts, actor, tool, args_json, result_summary, allowed) = row?;
            out.push(AuditEntry {
                id,
                ts,
                actor,
                tool,
                // A corrupt args column must not hide the entry: the fact that the call
                // happened matters more than reading back its arguments.
                args: serde_json::from_str(&args_json)
                    .unwrap_or(serde_json::Value::String(args_json)),
                result_summary,
                allowed: allowed != 0,
            });
        }
        Ok(out)
    }

    pub fn audit_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))?)
    }

    /// Write a mutation and its inverse to the journal. Call this *before* touching the
    /// filesystem, so that a crash halfway leaves an undoable record rather than a mystery.
    pub fn push_undo(
        &self,
        ts: i64,
        actor: &str,
        operation: &serde_json::Value,
        inverse: &serde_json::Value,
    ) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO undo_journal (ts, actor, operation_json, inverse_json, applied)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![
                ts,
                actor,
                serde_json::to_string(operation)?,
                serde_json::to_string(inverse)?
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The undoable entries, newest first. Entries already undone are excluded.
    pub fn undoable(&self, limit: usize) -> Result<Vec<UndoEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, operation_json, inverse_json, applied, undone_at
             FROM undo_journal
             WHERE applied = 1
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_undo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn undo_entry(&self, id: i64) -> Result<Option<UndoEntry>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT id, ts, actor, operation_json, inverse_json, applied, undone_at
                 FROM undo_journal WHERE id = ?1",
                params![id],
                row_to_undo,
            )
            .optional()?)
    }

    /// Mark an entry as undone. Returns false if it was already undone, which is how a
    /// concurrent second `cortex undo` is prevented from applying the same inverse twice.
    pub fn mark_undone(&self, id: i64, ts: i64) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE undo_journal SET applied = 0, undone_at = ?2 WHERE id = ?1 AND applied = 1",
            params![id, ts],
        )?;
        Ok(changed == 1)
    }

    /// Drop the oldest entries beyond `depth`, counting undone ones too.
    ///
    /// The journal is a safety net with a fixed size, not an archive: an agent that moves ten
    /// thousand files must not be able to grow the index without bound.
    pub fn trim_undo(&self, depth: usize) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM undo_journal WHERE id NOT IN (
                 SELECT id FROM undo_journal ORDER BY id DESC LIMIT ?1
             )",
            params![depth as i64],
        )?)
    }

    pub fn undo_depth(&self) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM undo_journal WHERE applied = 1",
            [],
            |r| r.get(0),
        )?)
    }
}

fn row_to_undo(row: &rusqlite::Row<'_>) -> rusqlite::Result<UndoEntry> {
    let operation: String = row.get(3)?;
    let inverse: String = row.get(4)?;
    Ok(UndoEntry {
        id: row.get(0)?,
        ts: row.get(1)?,
        actor: row.get(2)?,
        operation: serde_json::from_str(&operation).unwrap_or(serde_json::Value::Null),
        inverse: serde_json::from_str(&inverse).unwrap_or(serde_json::Value::Null),
        applied: row.get::<_, i64>(5)? != 0,
        undone_at: row.get(6)?,
    })
}

/// A journal entry whose inverse could not be read back is unusable; say so rather than
/// applying `null`.
pub fn require_inverse(entry: &UndoEntry) -> Result<&serde_json::Value> {
    if entry.inverse.is_null() {
        return Err(StoreError::corrupt(format!(
            "undo entry {} has no readable inverse",
            entry.id
        )));
    }
    Ok(&entry.inverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn audit_records_refusals_and_reads_them_back() {
        let s = store();
        s.append_audit(10, "agent:test", "fs.read", &serde_json::json!({"path": "/x"}), Some("42 bytes"), true)
            .unwrap();
        s.append_audit(
            11,
            "agent:test",
            "fs.write",
            &serde_json::json!({"path": "/x"}),
            Some("refused: outside allow-list"),
            false,
        )
        .unwrap();

        let all = s.recent_audit(10, None, false).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].tool, "fs.write", "newest first");

        let refused = s.recent_audit(10, None, true).unwrap();
        assert_eq!(refused.len(), 1);
        assert!(!refused[0].allowed);
        assert_eq!(refused[0].args["path"], "/x");

        let other = s.recent_audit(10, Some("agent:other"), false).unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn undo_entry_can_only_be_applied_once() {
        let s = store();
        let id = s
            .push_undo(
                1,
                "agent:test",
                &serde_json::json!({"op": "move", "from": "/a", "to": "/b"}),
                &serde_json::json!({"op": "move", "from": "/b", "to": "/a"}),
            )
            .unwrap();

        assert_eq!(s.undo_depth().unwrap(), 1);
        assert!(s.mark_undone(id, 2).unwrap());
        assert!(!s.mark_undone(id, 3).unwrap(), "second undo must not fire");
        assert_eq!(s.undo_depth().unwrap(), 0);

        let entry = s.undo_entry(id).unwrap().unwrap();
        assert!(!entry.applied);
        assert_eq!(entry.undone_at, Some(2));
    }

    #[test]
    fn undoable_returns_newest_first_and_skips_undone() {
        let s = store();
        let op = serde_json::json!({"op": "trash"});
        let first = s.push_undo(1, "agent:a", &op, &op).unwrap();
        let second = s.push_undo(2, "agent:a", &op, &op).unwrap();

        let entries = s.undoable(10).unwrap();
        assert_eq!(entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![second, first]);

        s.mark_undone(second, 3).unwrap();
        let entries = s.undoable(10).unwrap();
        assert_eq!(entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![first]);
    }

    #[test]
    fn trim_keeps_the_newest_depth_entries() {
        let s = store();
        let op = serde_json::json!({"op": "trash"});
        for ts in 0..10 {
            s.push_undo(ts, "agent:a", &op, &op).unwrap();
        }
        let removed = s.trim_undo(4).unwrap();
        assert_eq!(removed, 6);
        let left = s.undoable(100).unwrap();
        assert_eq!(left.len(), 4);
        assert_eq!(left[0].ts, 9, "the newest survived");
    }
}
