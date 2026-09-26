//! Backups of the store, taken while the daemon runs.
//!
//! R2 says the index is a cache: everything in it can be rebuilt from the filesystem and the
//! append-only `episodes` table. That makes two backups worth having, and they are different in
//! kind rather than in size:
//!
//! - **light** — the tables nothing else can reproduce: the episode log, the store's identity and
//!   sync origin (`meta`), the audit log, the undo journal and the handful of small tables a
//!   person made by hand (zones they recoloured, notes, sessions, packs and their outcomes). A few
//!   megabytes, taken in well under a second, so it can run at every boot and every shutdown.
//!   Restoring it means a rebuild: the files are re-read and the episodes replayed (R10).
//! - **full** — `VACUUM INTO`: a consistent, compacted copy of the whole database, vectors and
//!   all, restorable by copying it back. Gigabytes, and seconds to write, so it is taken rarely.
//!
//! Both are ordinary SQLite files, written next to their final name and renamed into place, so a
//! backup directory never holds a half-written file under a finished name.

use std::path::{Path, PathBuf};

use crate::error::{Result, StoreError};
use crate::Store;

/// What a light backup keeps. Everything else is derived and comes back through a rebuild.
pub const LIGHT_TABLES: &[&str] = &[
    "episodes",
    "meta",
    "audit_log",
    "undo_journal",
    "annotations",
    "zones",
    "memories",
    "agent_sessions",
    "context_packs",
    "context_pack_items",
    "sync_state",
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Light,
    Full,
}

impl BackupKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackupKind::Light => "light",
            BackupKind::Full => "full",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupReport {
    pub path: PathBuf,
    pub kind: BackupKind,
    pub bytes: u64,
    /// Rows per table, for a light backup; empty for a full one.
    pub rows: Vec<(String, i64)>,
    /// Older backups of the same kind removed to honour `keep`.
    pub pruned: Vec<PathBuf>,
}

/// `cortex-<kind>-<YYYYmmdd-HHMMSS>[-<label>].db`
pub fn file_name(kind: &BackupKind, stamp: &str, label: Option<&str>) -> String {
    let label: String = label
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(24)
        .collect();
    if label.is_empty() {
        format!("cortex-{}-{stamp}.db", kind.as_str())
    } else {
        format!("cortex-{}-{stamp}-{label}.db", kind.as_str())
    }
}

/// Backups of one kind in a directory, oldest first. The name sorts by time.
pub fn existing(dir: &Path, kind: &BackupKind) -> Vec<PathBuf> {
    let prefix = format!("cortex-{}-", kind.as_str());
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".db"))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

impl Store {
    /// Write a backup of `kind` into `dir` and keep only the newest `keep` of that kind.
    pub fn backup(
        &self,
        dir: &Path,
        kind: BackupKind,
        stamp: &str,
        label: Option<&str>,
        keep: usize,
    ) -> Result<BackupReport> {
        std::fs::create_dir_all(dir)?;
        let final_path = dir.join(file_name(&kind, stamp, label));
        let partial = dir.join(format!(
            ".{}.partial",
            final_path.file_name().and_then(|n| n.to_str()).unwrap_or("backup")
        ));
        let _ = std::fs::remove_file(&partial);
        let target = partial
            .to_str()
            .ok_or_else(|| StoreError::corrupt("backup path is not valid UTF-8"))?
            .to_string();

        let mut rows = Vec::new();
        {
            let conn = self.conn();
            match kind {
                BackupKind::Full => {
                    conn.execute("VACUUM INTO ?1", [&target])?;
                }
                BackupKind::Light => {
                    // The copied tables keep their REFERENCES clauses, and the tables they point
                    // at (`nodes`) are derived and deliberately left out; with enforcement on,
                    // the first insert would fail looking for them. Off for the copy, and put
                    // back however it ends.
                    let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
                    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
                    conn.execute("ATTACH DATABASE ?1 AS backup", [&target])?;
                    let copied = (|| -> Result<Vec<(String, i64)>> {
                        let mut rows = Vec::new();
                        for table in LIGHT_TABLES {
                            let sql: Option<String> = conn
                                .query_row(
                                    "SELECT sql FROM main.sqlite_master WHERE type = 'table' AND name = ?1",
                                    [table],
                                    |r| r.get(0),
                                )
                                .ok();
                            // A table an older schema does not have yet is simply not in the
                            // backup; a restore into that schema would not want it either.
                            let Some(sql) = sql else { continue };
                            let create = rewrite_create(&sql, table).ok_or_else(|| {
                                StoreError::corrupt(format!("unexpected schema for {table}"))
                            })?;
                            conn.execute_batch(&create)?;
                            let n = conn.execute(
                                &format!("INSERT INTO backup.\"{table}\" SELECT * FROM main.\"{table}\""),
                                [],
                            )?;
                            rows.push((table.to_string(), n as i64));
                        }
                        Ok(rows)
                    })();
                    let detached = conn.execute("DETACH DATABASE backup", []);
                    let _ = conn.execute_batch(&format!("PRAGMA foreign_keys = {fk}"));
                    rows = copied?;
                    detached?;
                }
            }
        }
        std::fs::rename(&partial, &final_path)?;
        let bytes = std::fs::metadata(&final_path)?.len();

        let mut pruned = Vec::new();
        let all = existing(dir, &kind);
        if keep > 0 && all.len() > keep {
            for old in &all[..all.len() - keep] {
                if old != &final_path && std::fs::remove_file(old).is_ok() {
                    pruned.push(old.clone());
                }
            }
        }

        Ok(BackupReport {
            path: final_path,
            kind,
            bytes,
            rows,
            pruned,
        })
    }
}

/// `CREATE TABLE episodes (…)` → `CREATE TABLE backup."episodes" (…)`.
fn rewrite_create(sql: &str, table: &str) -> Option<String> {
    let open = sql.find('(')?;
    let head = sql[..open].to_ascii_uppercase();
    if !head.trim_start().starts_with("CREATE TABLE") {
        return None;
    }
    Some(format!("CREATE TABLE backup.\"{table}\" {}", &sql[open..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::model::{Action, Actor};

    fn store_with_episodes(n: usize) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cortex.db")).unwrap();
        for i in 0..n {
            store
                .append_episode(1_000 + i as i64, &Actor::Human, Action::Search, None, None, &serde_json::json!({ "q": i }))
                .unwrap();
        }
        (dir, store)
    }

    #[test]
    fn a_light_backup_holds_the_episode_log_and_nothing_derived() {
        let (dir, store) = store_with_episodes(7);
        let out = dir.path().join("backups");
        let report = store.backup(&out, BackupKind::Light, "20260101-000000", Some("boot"), 5).unwrap();
        assert!(report.path.ends_with("cortex-light-20260101-000000-boot.db"));
        assert!(report.rows.iter().any(|(t, n)| t == "episodes" && *n == 7));

        let copy = rusqlite::Connection::open(&report.path).unwrap();
        let episodes: i64 = copy.query_row("SELECT count(*) FROM episodes", [], |r| r.get(0)).unwrap();
        assert_eq!(episodes, 7);
        let has_nodes: i64 = copy
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'nodes'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(has_nodes, 0, "nodes are derived and must not be in a light backup");
        // No partial file is left behind under any name.
        assert!(std::fs::read_dir(&out).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".partial")));
    }

    #[test]
    fn a_full_backup_is_the_whole_database() {
        let (dir, store) = store_with_episodes(3);
        let report = store.backup(&dir.path().join("b"), BackupKind::Full, "20260101-000000", None, 2).unwrap();
        let copy = Store::open(&report.path).unwrap();
        assert_eq!(copy.stats().unwrap().episodes, store.stats().unwrap().episodes);
    }

    #[test]
    fn only_the_newest_are_kept_and_only_of_the_same_kind() {
        let (dir, store) = store_with_episodes(1);
        let out = dir.path().join("b");
        store.backup(&out, BackupKind::Full, "20260101-000000", None, 5).unwrap();
        for stamp in ["20260101-000001", "20260101-000002", "20260101-000003"] {
            store.backup(&out, BackupKind::Light, stamp, None, 2).unwrap();
        }
        let light = existing(&out, &BackupKind::Light);
        assert_eq!(light.len(), 2);
        assert!(light[0].to_string_lossy().contains("000002"));
        assert_eq!(existing(&out, &BackupKind::Full).len(), 1);
    }

    #[test]
    fn a_label_cannot_escape_the_directory() {
        assert_eq!(
            file_name(&BackupKind::Light, "s", Some("../../etc/passwd")),
            "cortex-light-s-etcpasswd.db"
        );
    }
}
