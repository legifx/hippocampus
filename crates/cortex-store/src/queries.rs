//! Read and write operations against the store.
//!
//! Scope for M0: enough to index paths, record episodes and answer a path query. Content
//! extraction, fusion and ranking arrive in M2 — this module is expected to grow, not to be
//! replaced.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::{EpisodeId, NodeId, SessionId};
use cortex_core::model::{Action, Actor, Episode, Node, NodeKind, ScoreBreakdown, SearchHit, Tier};
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

impl Store {
    /// Insert or refresh a node, returning its id.
    ///
    /// Identity is the path. Move detection by `(dev, inode)` belongs to the watcher in M1 and
    /// is deliberately not attempted here — a manual index pass has no way to distinguish a
    /// move from a copy-then-delete.
    pub fn upsert_node(&self, node: &NodeUpsert, now: i64) -> Result<NodeId> {
        let conn = self.conn();
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

    /// M0 path search: a case-insensitive substring match over indexed paths.
    ///
    /// This is intentionally the naive implementation. M1 replaces it with an in-memory trigram
    /// index that meets the 10 ms budget; this version exists so that the end-to-end path from
    /// CLI through RPC to storage is exercised and tested before the fast path is written.
    pub fn search_paths(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, path, tier, zone_id, salience
             FROM nodes
             WHERE path IS NOT NULL AND path LIKE '%' || ?1 || '%' ESCAPE '\\'
             ORDER BY salience DESC, mtime DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![escape_like(query), limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, f64>(5)?,
            ))
        })?;

        let mut hits = Vec::new();
        for (rank, row) in rows.enumerate() {
            let (id, kind, path, tier, zone_id, salience) = row?;
            hits.push(SearchHit {
                node_id: NodeId(id),
                path: PathBuf::from(path),
                kind: NodeKind::parse(&kind)
                    .ok_or_else(|| StoreError::corrupt(format!("unknown node kind: {kind}")))?,
                tier: Tier::parse(&tier)
                    .ok_or_else(|| StoreError::corrupt(format!("unknown tier: {tier}")))?,
                zone_id: zone_id.map(cortex_core::ids::ZoneId),
                snippet: None,
                score: ScoreBreakdown {
                    path_rank: Some(rank as u32 + 1),
                    lexical_rank: None,
                    vector_rank: None,
                    fused: 1.0 / (rank as f64 + 1.0),
                    salience_boost: 1.0,
                    rerank: None,
                    final_score: 1.0 / (rank as f64 + 1.0) + salience,
                },
            });
        }
        Ok(hits)
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

/// Escape the wildcards `LIKE` would otherwise interpret.
///
/// Without this, searching for `_` or `%` silently matches everything, which reads as a bug in
/// the search rather than as a quirk of SQL.
fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
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
    fn search_finds_a_substring_of_the_path() {
        let store = store_with(&["/home/user/report-foo.pdf", "/home/user/other.txt"]);
        let hits = store.search_paths("foo", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, PathBuf::from("/home/user/report-foo.pdf"));
    }

    #[test]
    fn search_respects_the_limit() {
        let paths: Vec<String> = (0..20).map(|i| format!("/x/foo{i}.txt")).collect();
        let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let store = store_with(&refs);
        assert_eq!(store.search_paths("foo", 5).unwrap().len(), 5);
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_everything() {
        let store = store_with(&["/a", "/b"]);
        assert!(store.search_paths("", 10).unwrap().is_empty());
        assert!(store.search_paths("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn like_wildcards_in_a_query_are_treated_as_literal_characters() {
        // Typing `%` must not match every file in the index.
        let store = store_with(&["/a/100%done.txt", "/a/other.txt", "/a/b_c.txt", "/a/bxc.txt"]);

        let percent = store.search_paths("%", 10).unwrap();
        assert_eq!(percent.len(), 1, "`%` must match only the literal percent sign");
        assert_eq!(percent[0].path, PathBuf::from("/a/100%done.txt"));

        let underscore = store.search_paths("b_c", 10).unwrap();
        assert_eq!(underscore.len(), 1, "`_` must not act as a single-character wildcard");
        assert_eq!(underscore[0].path, PathBuf::from("/a/b_c.txt"));
    }

    #[test]
    fn search_results_carry_an_explainable_score() {
        // Every result must be able to say where it came from — rule from the build prompt.
        let store = store_with(&["/x/foo.txt"]);
        let hit = &store.search_paths("foo", 10).unwrap()[0];
        assert_eq!(hit.score.path_rank, Some(1));
        assert!(hit.score.fused > 0.0);
        assert!(hit.score.rerank.is_none(), "nothing has reranked yet at M0");
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
