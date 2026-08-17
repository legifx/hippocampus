//! Read and write operations against the store.
//!
//! Scope for M0: enough to index paths, record episodes and answer a path query. Content
//! extraction, fusion and ranking arrive in M2 — this module is expected to grow, not to be
//! replaced.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::{EpisodeId, NodeId, SessionId};
use cortex_core::model::{Action, Actor, Episode, Node, NodeKind, Tier};
use rusqlite::{params, OptionalExtension, Row};
use std::path::{Path, PathBuf};

/// Everything needed to record a node the first time it is seen, or to refresh it.
#[derive(Debug, Clone)]
pub struct NodeUpsert {
    pub kind: NodeKind,
    pub path: PathBuf,
    pub parent_id: Option<NodeId>,
    pub mime: Option<String>,
    pub size: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub inode: i64,
    pub dev: i64,
}

/// What the index believes about one path. The comparison unit for reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedNode {
    pub id: NodeId,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub size: i64,
    pub mtime: i64,
    pub inode: i64,
    pub dev: i64,
}

/// Aggregate counts, for `cortex status`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub files: i64,
    pub dirs: i64,
    pub chunks: i64,
    pub episodes: i64,
    pub embeddings: i64,
    pub outbox_pending: i64,
}

/// A batch of writes sharing one transaction.
///
/// SQLite commits every statement separately unless told otherwise, and each commit is a
/// durability barrier. Indexing a hundred thousand files one autocommitted statement at a time
/// is dominated entirely by that barrier — it was the difference between four minutes and a few
/// seconds on the reference corpus. Anything that writes more than a handful of rows should go
/// through here.
pub struct Batch<'a> {
    tx: &'a rusqlite::Transaction<'a>,
}

impl Batch<'_> {
    pub fn upsert_node(&self, node: &NodeUpsert, now: i64) -> Result<NodeId> {
        upsert_node_on(self.tx, node, now)
    }

    pub fn append_episode(
        &self,
        ts: i64,
        actor: &Actor,
        action: Action,
        node_id: Option<NodeId>,
        session_id: Option<&SessionId>,
        metadata: &serde_json::Value,
    ) -> Result<EpisodeId> {
        append_episode_on(self.tx, ts, actor, action, node_id, session_id, metadata)
    }

    pub fn remove_path(&self, path: &Path) -> Result<usize> {
        remove_path_on(self.tx, path)
    }

    pub fn rename_node(&self, from: &Path, to: &Path) -> Result<usize> {
        rename_node_on(self.tx, from, to)
    }
}

impl Store {
    /// Run many writes inside one transaction.
    ///
    /// The closure sees a [`Batch`]; returning `Err` rolls everything back, which is the
    /// behaviour a partially-applied index repair should have.
    pub fn batch<T>(&self, f: impl FnOnce(&Batch<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let result = {
            let batch = Batch { tx: &tx };
            f(&batch)?
        };
        tx.commit()?;
        Ok(result)
    }

    /// Insert or refresh a node, returning its id.
    ///
    /// Identity is the path. Move detection by `(dev, inode)` belongs to the watcher in M1 and
    /// is deliberately not attempted here — a manual index pass has no way to distinguish a
    /// move from a copy-then-delete.
    pub fn upsert_node(&self, node: &NodeUpsert, now: i64) -> Result<NodeId> {
        upsert_node_on(&self.conn(), node, now)
    }

    pub fn node_by_id(&self, id: NodeId) -> Result<Option<Node>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, kind, path, parent_id, mime, size, mtime, ctime, inode, dev,
                    content_hash, tier, salience, pinned, zone_id, first_seen, last_seen
             FROM nodes WHERE id = ?1",
            [id.0],
            row_to_node,
        )
        .optional()
        .map_err(StoreError::from)?
        .transpose()
    }

    pub fn node_by_path(&self, path: &Path) -> Result<Option<Node>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, kind, path, parent_id, mime, size, mtime, ctime, inode, dev,
                    content_hash, tier, salience, pinned, zone_id, first_seen, last_seen
             FROM nodes WHERE path = ?1",
            [path_to_str(path)?],
            row_to_node,
        )
        .optional()
        .map_err(StoreError::from)?
        .transpose()
    }

    /// Append an episode. Episodes are never updated, so this is the only way in.
    pub fn append_episode(
        &self,
        ts: i64,
        actor: &Actor,
        action: Action,
        node_id: Option<NodeId>,
        session_id: Option<&SessionId>,
        metadata: &serde_json::Value,
    ) -> Result<EpisodeId> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, session_id, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts,
                actor.as_str(),
                action.as_str(),
                node_id.map(i64::from),
                session_id.map(|s| s.0.as_str()),
                serde_json::to_string(metadata)?,
            ],
        )?;
        Ok(EpisodeId(conn.last_insert_rowid()))
    }

    /// Most recent episodes first.
    pub fn recent_episodes(&self, limit: usize) -> Result<Vec<Episode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, action, node_id, session_id, metadata_json
             FROM episodes ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_episode)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .collect()
    }

    /// A node's identity as the index currently records it. Used by reconciliation.
    pub fn nodes_under(&self, prefix: &Path) -> Result<Vec<IndexedNode>> {
        let conn = self.conn();
        let prefix = path_to_str(prefix)?;
        let mut stmt = conn.prepare(
            "SELECT id, path, kind, size, mtime, inode, dev
             FROM nodes
             WHERE path IS NOT NULL AND (path = ?1 OR path LIKE ?1 || '/%')
             ORDER BY path",
        )?;
        let rows = stmt.query_map([prefix], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (id, path, kind, size, mtime, inode, dev) = row?;
            out.push(IndexedNode {
                id: NodeId(id),
                path: PathBuf::from(path),
                kind: NodeKind::parse(&kind)
                    .ok_or_else(|| StoreError::corrupt(format!("unknown node kind: {kind}")))?,
                size,
                mtime,
                inode,
                dev,
            });
        }
        Ok(out)
    }

    /// Find a node by the identity the kernel gives it.
    ///
    /// This is how a move is recognised when only the reconcile scan sees it: the path changed
    /// but `(dev, inode)` did not, so it is the same file and its history should follow it.
    pub fn node_by_identity(&self, dev: i64, inode: i64) -> Result<Option<NodeId>> {
        if inode == 0 {
            return Ok(None);
        }
        let conn = self.conn();
        conn.query_row(
            "SELECT id FROM nodes WHERE dev = ?1 AND inode = ?2 LIMIT 1",
            [dev, inode],
            |r| r.get::<_, i64>(0).map(NodeId),
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Move a node to a new path, keeping its identity.
    ///
    /// Not a delete plus an insert: the node id is what edges, chunks, embeddings and episodes
    /// point at. Re-creating it would sever every one of those and force a re-embed, which is
    /// exactly the cost move detection exists to avoid.
    ///
    /// Descendants follow, because renaming a directory renames everything beneath it.
    pub fn rename_node(&self, from: &Path, to: &Path) -> Result<usize> {
        rename_node_on(&self.conn(), from, to)
    }

    /// Remove a node and everything beneath it.
    ///
    /// Episodes deliberately survive — they are the ground truth the index is rebuilt from,
    /// and an episode about a file that no longer exists is the entire point of an episodic
    /// memory.
    pub fn remove_path(&self, path: &Path) -> Result<usize> {
        remove_path_on(&self.conn(), path)
    }

    /// Set a node's parent, linking it into the directory graph.
    pub fn set_parent(&self, child: NodeId, parent: Option<NodeId>) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
            params![child.0, parent.map(i64::from)],
        )?;
        Ok(())
    }

    /// Every indexed path with its node id, for building the in-memory path index at startup.
    pub fn all_paths_with_ids(&self) -> Result<Vec<(NodeId, String)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, path FROM nodes WHERE path IS NOT NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((NodeId(r.get(0)?), r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Every indexed path, sorted.
    ///
    /// This is the backbone of the canonical dump used to prove that `--rebuild-index`
    /// reproduces the same index (hard rule R10), so it is a real API rather than a test helper.
    pub fn all_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT path FROM nodes WHERE path IS NOT NULL ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn stats(&self) -> Result<Stats> {
        let conn = self.conn();
        let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
        Ok(Stats {
            files: one("SELECT count(*) FROM nodes WHERE kind = 'file'")?,
            dirs: one("SELECT count(*) FROM nodes WHERE kind = 'dir'")?,
            chunks: one("SELECT count(*) FROM chunks")?,
            episodes: one("SELECT count(*) FROM episodes")?,
            embeddings: one("SELECT count(*) FROM embeddings")?,
            outbox_pending: one("SELECT count(*) FROM sync_outbox")?,
        })
    }

    /// Reclaim space and rebuild indexes. Blocking and occasionally slow — never on a hot path.
    pub fn vacuum(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute_batch("VACUUM; ANALYZE;")?;
        Ok(())
    }
}


// ---------------------------------------------------------------------------
// Operations expressed against a bare connection.
//
// `Transaction` derefs to `Connection`, so each of these works identically inside a batch and
// on its own. Having one implementation rather than two is the point: a batched path that
// slowly drifts from the unbatched one is a bug waiting for a busy day.
// ---------------------------------------------------------------------------

fn upsert_node_on(conn: &rusqlite::Connection, node: &NodeUpsert, now: i64) -> Result<NodeId> {
    let path = path_to_str(&node.path)?;

    conn.execute(
        "INSERT INTO nodes
             (kind, path, parent_id, mime, size, mtime, ctime, inode, dev, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         -- The unique index on `path` is partial (it excludes pathless concept and
         -- annotation nodes), so the conflict target has to repeat its WHERE clause.
         ON CONFLICT(path) WHERE path IS NOT NULL DO UPDATE SET
             kind      = excluded.kind,
             parent_id = excluded.parent_id,
             mime      = excluded.mime,
             size      = excluded.size,
             mtime     = excluded.mtime,
             ctime     = excluded.ctime,
             inode     = excluded.inode,
             dev       = excluded.dev,
             last_seen = excluded.last_seen",
        params![
            node.kind.as_str(),
            path,
            node.parent_id.map(i64::from),
            node.mime,
            node.size,
            node.mtime,
            node.ctime,
            node.inode,
            node.dev,
            now,
        ],
    )?;

    let id: i64 = conn.query_row("SELECT id FROM nodes WHERE path = ?1", [path], |r| r.get(0))?;
    Ok(NodeId(id))
}

fn append_episode_on(
    conn: &rusqlite::Connection,
    ts: i64,
    actor: &Actor,
    action: Action,
    node_id: Option<NodeId>,
    session_id: Option<&SessionId>,
    metadata: &serde_json::Value,
) -> Result<EpisodeId> {
    conn.execute(
        "INSERT INTO episodes (ts, actor, action, node_id, session_id, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            ts,
            actor.as_str(),
            action.as_str(),
            node_id.map(i64::from),
            session_id.map(|s| s.0.as_str()),
            serde_json::to_string(metadata)?,
        ],
    )?;
    Ok(EpisodeId(conn.last_insert_rowid()))
}

fn remove_path_on(conn: &rusqlite::Connection, path: &Path) -> Result<usize> {
    let path = path_to_str(path)?;
    Ok(conn.execute(
        "DELETE FROM nodes WHERE path = ?1 OR path LIKE ?1 || '/%'",
        [path],
    )?)
}

fn rename_node_on(conn: &rusqlite::Connection, from: &Path, to: &Path) -> Result<usize> {
    let from = path_to_str(from)?;
    let to = path_to_str(to)?;

    // Refuse to do anything if the source is not there. Without this guard the destination
    // clean-up below would delete a perfectly good row on behalf of a source that does not
    // exist — which is exactly what a stale caller does after an earlier rename already
    // moved the subtree.
    let source_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM nodes WHERE path = ?1)",
        [from],
        |r| r.get::<_, i64>(0),
    )? == 1;
    if !source_exists {
        return Ok(0);
    }

    // The destination may already exist if the move overwrote something. Its row has to go
    // first, or the unique index on `path` rejects the rename.
    conn.execute("DELETE FROM nodes WHERE path = ?1", [to])?;

    let moved = conn.execute("UPDATE nodes SET path = ?2 WHERE path = ?1", [from, to])?;
    let descendants = conn.execute(
        "UPDATE nodes SET path = ?2 || substr(path, length(?1) + 1)
         WHERE path LIKE ?1 || '/%'",
        [from, to],
    )?;

    Ok(moved + descendants)
}

fn row_to_node(row: &Row<'_>) -> rusqlite::Result<Result<Node>> {
    let kind: String = row.get(1)?;
    let tier: String = row.get(3 + 8)?; // column 11: tier
    let path: Option<String> = row.get(2)?;

    Ok((|| {
        Ok(Node {
            id: NodeId(row.get(0)?),
            kind: NodeKind::parse(&kind)
                .ok_or_else(|| StoreError::corrupt(format!("unknown node kind: {kind}")))?,
            path: PathBuf::from(path.unwrap_or_default()),
            parent_id: row.get::<_, Option<i64>>(3)?.map(NodeId),
            mime: row.get(4)?,
            size: row.get(5)?,
            mtime: row.get(6)?,
            ctime: row.get(7)?,
            inode: row.get(8)?,
            dev: row.get(9)?,
            content_hash: row.get(10)?,
            tier: Tier::parse(&tier)
                .ok_or_else(|| StoreError::corrupt(format!("unknown tier: {tier}")))?,
            salience: row.get(12)?,
            pinned: row.get::<_, i64>(13)? != 0,
            zone_id: row.get::<_, Option<i64>>(14)?.map(cortex_core::ids::ZoneId),
            first_seen: row.get(15)?,
            last_seen: row.get(16)?,
        })
    })())
}

fn row_to_episode(row: &Row<'_>) -> rusqlite::Result<Result<Episode>> {
    let actor: String = row.get(2)?;
    let action: String = row.get(3)?;
    let metadata: String = row.get(6)?;

    Ok((|| {
        Ok(Episode {
            id: EpisodeId(row.get(0)?),
            ts: row.get(1)?,
            actor: Actor::parse(&actor)
                .ok_or_else(|| StoreError::corrupt(format!("unknown actor: {actor}")))?,
            action: Action::parse(&action)
                .ok_or_else(|| StoreError::corrupt(format!("unknown action: {action}")))?,
            node_id: row.get::<_, Option<i64>>(4)?.map(NodeId),
            session_id: row.get::<_, Option<String>>(5)?.map(SessionId),
            metadata: serde_json::from_str(&metadata)?,
        })
    })())
}

/// Paths that are not valid UTF-8 are rejected rather than lossily converted.
///
/// A lossy conversion would produce an index entry that can never be matched back to the real
/// file, which is worse than declining to index it and saying so.
fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        StoreError::corrupt(format!("path is not valid UTF-8: {}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert(kind: NodeKind, path: &str) -> NodeUpsert {
        NodeUpsert {
            kind,
            path: PathBuf::from(path),
            parent_id: None,
            mime: None,
            size: 0,
            mtime: 0,
            ctime: 0,
            inode: 0,
            dev: 0,
        }
    }

    fn store_with(paths: &[&str]) -> Store {
        let store = Store::open_in_memory().unwrap();
        for p in paths {
            store.upsert_node(&upsert(NodeKind::File, p), 0).unwrap();
        }
        store
    }

    #[test]
    fn a_node_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let mut u = upsert(NodeKind::File, "/home/user/notes.md");
        u.size = 1234;
        u.mtime = 99;
        u.mime = Some("text/markdown".into());

        let id = store.upsert_node(&u, 10).unwrap();
        let node = store.node_by_id(id).unwrap().expect("node must exist");

        assert_eq!(node.path, PathBuf::from("/home/user/notes.md"));
        assert_eq!(node.size, 1234);
        assert_eq!(node.mtime, 99);
        assert_eq!(node.mime.as_deref(), Some("text/markdown"));
        assert_eq!(node.tier, Tier::Stm);
        assert!(!node.pinned);
    }

    #[test]
    fn upserting_the_same_path_updates_rather_than_duplicates() {
        let store = Store::open_in_memory().unwrap();
        let mut u = upsert(NodeKind::File, "/a");
        let first = store.upsert_node(&u, 1).unwrap();

        u.size = 500;
        let second = store.upsert_node(&u, 2).unwrap();

        assert_eq!(first, second, "the same path must keep its node id");
        let node = store.node_by_id(first).unwrap().unwrap();
        assert_eq!(node.size, 500);
        assert_eq!(node.first_seen, 1, "first_seen must not move");
        assert_eq!(node.last_seen, 2, "last_seen must advance");
    }

    #[test]
    fn lookup_by_path_and_by_id_agree() {
        let store = store_with(&["/a/b.txt"]);
        let by_path = store.node_by_path(Path::new("/a/b.txt")).unwrap().unwrap();
        let by_id = store.node_by_id(by_path.id).unwrap().unwrap();
        assert_eq!(by_path.id, by_id.id);
    }

    #[test]
    fn a_missing_node_is_none_rather_than_an_error() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.node_by_id(NodeId(999)).unwrap().is_none());
        assert!(store.node_by_path(Path::new("/nope")).unwrap().is_none());
    }






    #[test]
    fn episodes_append_and_read_back_in_reverse_order() {
        let store = Store::open_in_memory().unwrap();
        let id = store.upsert_node(&upsert(NodeKind::File, "/a"), 0).unwrap();

        store
            .append_episode(1, &Actor::Human, Action::Open, Some(id), None, &serde_json::json!({}))
            .unwrap();
        store
            .append_episode(
                2,
                &Actor::Agent("claude".into()),
                Action::Search,
                None,
                Some(&SessionId("s1".into())),
                &serde_json::json!({ "query": "foo" }),
            )
            .unwrap();

        let episodes = store.recent_episodes(10).unwrap();
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].ts, 2, "most recent first");
        assert_eq!(episodes[0].actor, Actor::Agent("claude".into()));
        assert_eq!(episodes[0].metadata["query"], "foo");
        assert_eq!(episodes[1].actor, Actor::Human);
    }

    #[test]
    fn an_episode_survives_its_node_being_removed() {
        let store = Store::open_in_memory().unwrap();
        let id = store.upsert_node(&upsert(NodeKind::File, "/gone"), 0).unwrap();
        store
            .append_episode(1, &Actor::Human, Action::Open, Some(id), None, &serde_json::json!({}))
            .unwrap();

        store
            .conn()
            .execute("DELETE FROM nodes WHERE id = ?1", [id.0])
            .unwrap();

        assert_eq!(store.recent_episodes(10).unwrap().len(), 1);
    }

    #[test]
    fn stats_count_what_is_there() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_node(&upsert(NodeKind::File, "/a"), 0).unwrap();
        store.upsert_node(&upsert(NodeKind::File, "/b"), 0).unwrap();
        store.upsert_node(&upsert(NodeKind::Dir, "/d"), 0).unwrap();
        store
            .append_episode(1, &Actor::System, Action::Create, None, None, &serde_json::json!({}))
            .unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.files, 2);
        assert_eq!(stats.dirs, 1);
        assert_eq!(stats.episodes, 1);
        assert_eq!(stats.chunks, 0);
    }

    #[test]
    fn nodes_under_returns_a_subtree_and_nothing_else() {
        let store = store_with(&[
            "/home/user/proj",
            "/home/user/proj/src/main.rs",
            "/home/user/project-other/file.rs",
            "/home/user/elsewhere.txt",
        ]);
        let under: Vec<String> = store
            .nodes_under(Path::new("/home/user/proj"))
            .unwrap()
            .into_iter()
            .map(|n| n.path.display().to_string())
            .collect();

        assert_eq!(under, vec!["/home/user/proj", "/home/user/proj/src/main.rs"]);
        assert!(
            !under.iter().any(|p| p.contains("project-other")),
            "a sibling sharing a name prefix must not be swept in"
        );
    }

    #[test]
    fn renaming_a_file_keeps_its_node_id() {
        // The node id is what edges, chunks and embeddings point at. Losing it on a rename
        // would sever all of them and force a re-embed.
        let store = store_with(&["/a/old.txt"]);
        let before = store.node_by_path(Path::new("/a/old.txt")).unwrap().unwrap();

        assert_eq!(store.rename_node(Path::new("/a/old.txt"), Path::new("/a/new.txt")).unwrap(), 1);

        let after = store.node_by_path(Path::new("/a/new.txt")).unwrap().unwrap();
        assert_eq!(before.id, after.id, "a rename must preserve identity");
        assert!(store.node_by_path(Path::new("/a/old.txt")).unwrap().is_none());
    }

    #[test]
    fn renaming_a_directory_carries_its_descendants() {
        let store = store_with(&["/a/old", "/a/old/one.txt", "/a/old/deep/two.txt"]);
        let moved = store.rename_node(Path::new("/a/old"), Path::new("/a/new")).unwrap();
        assert_eq!(moved, 3);

        let paths = store.all_paths().unwrap();
        assert_eq!(paths, vec!["/a/new", "/a/new/deep/two.txt", "/a/new/one.txt"]);
    }

    #[test]
    fn renaming_over_an_existing_path_replaces_it() {
        // `mv a b` where b exists is legal and common; without clearing the destination first,
        // the unique index on `path` would reject the rename.
        let store = store_with(&["/a/source.txt", "/a/target.txt"]);
        store
            .rename_node(Path::new("/a/source.txt"), Path::new("/a/target.txt"))
            .unwrap();

        assert_eq!(store.all_paths().unwrap(), vec!["/a/target.txt"]);
    }

    #[test]
    fn removing_a_path_takes_its_subtree_but_leaves_history() {
        let store = store_with(&["/a/dir", "/a/dir/file.txt", "/a/keep.txt"]);
        let node = store.node_by_path(Path::new("/a/dir/file.txt")).unwrap().unwrap();
        store
            .append_episode(1, &Actor::Human, Action::Open, Some(node.id), None, &serde_json::json!({}))
            .unwrap();

        assert_eq!(store.remove_path(Path::new("/a/dir")).unwrap(), 2);
        assert_eq!(store.all_paths().unwrap(), vec!["/a/keep.txt"]);
        assert_eq!(
            store.recent_episodes(10).unwrap().len(),
            1,
            "removing a node must not erase what happened to it"
        );
    }

    #[test]
    fn a_node_can_be_found_by_its_kernel_identity() {
        let store = Store::open_in_memory().unwrap();
        let mut u = upsert(NodeKind::File, "/a/file.txt");
        u.dev = 42;
        u.inode = 1234;
        let id = store.upsert_node(&u, 0).unwrap();

        assert_eq!(store.node_by_identity(42, 1234).unwrap(), Some(id));
        assert_eq!(store.node_by_identity(42, 9999).unwrap(), None);
    }

    #[test]
    fn identity_lookup_ignores_the_unknown_inode() {
        // Zero is the placeholder for "we did not record one", and it would otherwise match
        // every node that was indexed without stat information.
        let store = store_with(&["/a/one.txt", "/a/two.txt"]);
        assert_eq!(store.node_by_identity(0, 0).unwrap(), None);
    }

    #[test]
    fn parents_can_be_linked() {
        let store = store_with(&["/a", "/a/b.txt"]);
        let parent = store.node_by_path(Path::new("/a")).unwrap().unwrap();
        let child = store.node_by_path(Path::new("/a/b.txt")).unwrap().unwrap();

        store.set_parent(child.id, Some(parent.id)).unwrap();

        let reread = store.node_by_id(child.id).unwrap().unwrap();
        assert_eq!(reread.parent_id, Some(parent.id));
    }

    #[test]
    fn a_non_utf8_path_is_refused_with_a_clear_error() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let store = Store::open_in_memory().unwrap();
            let bad = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe"));
            let mut u = upsert(NodeKind::File, "/placeholder");
            u.path = bad;

            let err = store.upsert_node(&u, 0).unwrap_err();
            assert!(
                matches!(err, StoreError::Corrupt(ref m) if m.contains("UTF-8")),
                "expected a clear corruption error, got {err:?}"
            );
        }
    }
}
