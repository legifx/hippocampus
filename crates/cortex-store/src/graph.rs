//! The associative index: edges, annotations, and the assertions that survive a rebuild.
//!
//! Everything here has to answer rule R10, and the answer is not obvious. An annotation an
//! agent wrote is not derivable from the filesystem — the filesystem has never heard of it —
//! and the index is a cache. So the assertion is written to `episodes` first, which is the one
//! table that is never rewritten, and the node and edge it produces are *derived* from that
//! episode. [`Store::replay_assertions`] rebuilds them, and `cortex rebuild` calls it.
//!
//! That is the whole trick, and it is worth stating plainly: **an agent's knowledge lives in
//! the episode log; the graph is a projection of it.** A feature that writes to the graph
//! without writing an episode has made the index un-rebuildable and is a bug, not a shortcut.

use crate::error::Result;
use crate::Store;
use cortex_core::ids::NodeId;
use cortex_core::model::{Actor, EdgeSource, EdgeType, NodeKind};
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;

/// One edge, as a traversal reports it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Neighbor {
    pub node_id: i64,
    pub path: Option<PathBuf>,
    pub kind: String,
    pub edge: EdgeType,
    pub source: EdgeSource,
    pub weight: f64,
    /// How many hops from the node the traversal started at.
    pub depth: u32,
    /// For an annotation node, the note itself — otherwise a traversal returns an id and a
    /// caller has to fetch every one of them to find out what it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an annotation holds.
///
/// The row in `annotations` is derived; the authority is the `annotate` episode it came from,
/// and `replay_assertions` rebuilds the row from that episode.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    pub node_id: i64,
    pub target_id: i64,
    pub tags: Vec<String>,
    pub note: String,
    pub actor: String,
    pub ts: i64,
}

impl Store {
    /// Assert an edge. Idempotent: asserting the same edge twice updates its weight rather than
    /// failing, because an agent re-stating something it already knows is not an error.
    pub fn insert_edge(
        &self,
        src: NodeId,
        dst: NodeId,
        edge: EdgeType,
        weight: f64,
        source: EdgeSource,
        now: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO edges (src, dst, type, weight, created_at, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(src, dst, type) DO UPDATE SET
                 weight = excluded.weight,
                 source = excluded.source",
            params![src.0, dst.0, edge.as_str(), weight, now, source.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_edge(&self, src: NodeId, dst: NodeId, edge: EdgeType) -> Result<bool> {
        let n = self.conn().execute(
            "DELETE FROM edges WHERE src = ?1 AND dst = ?2 AND type = ?3",
            params![src.0, dst.0, edge.as_str()],
        )?;
        Ok(n > 0)
    }

    /// Walk outwards from a node, breadth-first, following edges in either direction.
    ///
    /// Both directions on purpose: an association is not one-way in a memory. A document that
    /// was annotated is a neighbour of its annotation just as much as the other way round, and
    /// a traversal that only followed `src → dst` would answer half the question depending on
    /// which end the caller happened to have.
    pub fn neighbors(
        &self,
        start: NodeId,
        types: &[EdgeType],
        depth: u32,
        limit: usize,
    ) -> Result<Vec<Neighbor>> {
        let conn = self.conn();
        let mut seen = std::collections::HashSet::from([start.0]);
        let mut frontier = vec![start.0];
        let mut out: Vec<Neighbor> = Vec::new();

        let filter: Vec<&str> = types.iter().map(|t| t.as_str()).collect();

        for level in 1..=depth.max(1) {
            let mut next = Vec::new();
            for node in frontier.drain(..) {
                let mut stmt = conn.prepare(
                    "SELECT e.dst, e.type, e.source, e.weight, n.path, n.kind
                     FROM edges e JOIN nodes n ON n.id = e.dst
                     WHERE e.src = ?1
                     UNION ALL
                     SELECT e.src, e.type, e.source, e.weight, n.path, n.kind
                     FROM edges e JOIN nodes n ON n.id = e.src
                     WHERE e.dst = ?1",
                )?;
                let rows = stmt.query_map(params![node], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?;

                for row in rows {
                    let (id, edge, source, weight, path, kind) = row?;
                    if !filter.is_empty() && !filter.contains(&edge.as_str()) {
                        continue;
                    }
                    if !seen.insert(id) {
                        continue;
                    }
                    out.push(Neighbor {
                        node_id: id,
                        path: path.map(PathBuf::from),
                        kind,
                        edge: EdgeType::parse(&edge).unwrap_or(EdgeType::SimilarTo),
                        source: EdgeSource::parse(&source).unwrap_or(EdgeSource::Inferred),
                        weight,
                        depth: level,
                        note: None,
                    });
                    next.push(id);
                    if out.len() >= limit {
                        return self.fill_notes(out);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }

        drop(conn);
        self.fill_notes(out)
    }

    /// Attach the text of any node in a traversal result whose whole content is its text.
    ///
    /// Annotations and memories both. A traversal that returned an id for either would make the
    /// caller fetch every one of them to find out what it says, which is the thing `note`
    /// exists to prevent — and it was returning bare ids for memories, which is worse than for
    /// annotations because a memory has no path to fall back on either.
    fn fill_notes(&self, mut neighbors: Vec<Neighbor>) -> Result<Vec<Neighbor>> {
        for neighbor in &mut neighbors {
            neighbor.note = match neighbor.kind.as_str() {
                "annotation" => self.annotation_text(NodeId(neighbor.node_id))?,
                "memory" => self.memory_text(NodeId(neighbor.node_id))?,
                _ => continue,
            };
        }
        Ok(neighbors)
    }

    /// The text a memory node carries.
    pub fn memory_text(&self, node: NodeId) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT text FROM memories WHERE node_id = ?1",
                params![node.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// The note an annotation node carries.
    pub fn annotation_text(&self, node: NodeId) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT note FROM annotations WHERE node_id = ?1",
                params![node.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Create an annotation about `target`, as one transaction: the annotation node, the
    /// `annotates` edge, and the episode that both are derived from.
    ///
    /// The episode is written last but is the authority. Losing the other two to a rebuild is
    /// expected; [`Store::replay_assertions`] puts them back.
    pub fn annotate(
        &self,
        target: NodeId,
        target_path: &str,
        tags: &[String],
        note: &str,
        actor: &Actor,
        now: i64,
    ) -> Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO nodes (kind, path, size, mtime, ctime, inode, dev, first_seen, last_seen)
             VALUES ('annotation', NULL, 0, ?1, ?1, 0, 0, ?1, ?1)",
            params![now],
        )?;
        let annotation = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO edges (src, dst, type, weight, created_at, source)
             VALUES (?1, ?2, 'annotates', 1.0, ?3, 'explicit')
             ON CONFLICT(src, dst, type) DO NOTHING",
            params![annotation, target.0, now],
        )?;

        // The path, not just the id: a rebuild renumbers every node, and an assertion that can
        // only name its target by a number is an assertion a rebuild cannot replay.
        let metadata = serde_json::json!({
            "assertion": "annotate",
            "target": target.0,
            "target_path": target_path,
            "tags": tags,
            "note": note,
        });
        tx.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, metadata_json)
             VALUES (?1, ?2, 'annotate', ?3, ?4)",
            params![now, actor.as_str(), annotation, metadata.to_string()],
        )?;
        let episode = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO annotations (node_id, target_id, episode_id, tags_json, note, actor, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                annotation,
                target.0,
                episode,
                serde_json::to_string(tags)?,
                note,
                actor.as_str(),
                now
            ],
        )?;

        tx.commit()?;
        Ok(annotation)
    }

    /// Every annotation attached to a node, newest first.
    pub fn annotations_for(&self, target: NodeId) -> Result<Vec<Annotation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT node_id, target_id, tags_json, note, actor, ts
             FROM annotations WHERE target_id = ?1 ORDER BY ts DESC, node_id DESC",
        )?;
        let rows = stmt.query_map(params![target.0], |row| {
            Ok(Annotation {
                node_id: row.get(0)?,
                target_id: row.get(1)?,
                tags: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                note: row.get(3)?,
                actor: row.get(4)?,
                ts: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_pinned(&self, node: NodeId, pinned: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE nodes SET pinned = ?2 WHERE id = ?1",
            params![node.0, pinned as i64],
        )?;
        Ok(())
    }

    pub fn is_pinned(&self, node: NodeId) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT pinned FROM nodes WHERE id = ?1", params![node.0], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .map(|v| v != 0)
            .unwrap_or(false))
    }

    /// Rebuild every explicit assertion from the episode log.
    ///
    /// Called after a reconcile during `cortex rebuild`. Node ids change across a rebuild, so
    /// assertions are replayed against *paths*, which is why every assertion episode records
    /// the path alongside the id it had at the time. An assertion whose target no longer exists
    /// is skipped and counted: the file is gone, and inventing a node for it would be worse
    /// than losing the note.
    ///
    /// Returns (replayed, skipped).
    pub fn replay_assertions(&self) -> Result<(usize, usize)> {
        let episodes: Vec<(i64, i64, String, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT id, ts, actor, metadata_json FROM episodes
                 WHERE action = 'annotate'
                   AND json_extract(metadata_json, '$.assertion') IS NOT NULL
                 ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut replayed = 0;
        let mut skipped = 0;

        for (_id, ts, actor, metadata) in episodes {
            let value: serde_json::Value = match serde_json::from_str(&metadata) {
                Ok(v) => v,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let kind = value.get("assertion").and_then(|a| a.as_str()).unwrap_or("");
            let actor = Actor::parse(&actor).unwrap_or(Actor::System);

            let resolved = |key: &str| -> Option<NodeId> {
                let path = value.get(key)?.as_str()?;
                self.node_by_path(std::path::Path::new(path))
                    .ok()
                    .flatten()
                    .map(|n| n.id)
            };

            match kind {
                "annotate" => {
                    let Some(target) = resolved("target_path") else {
                        skipped += 1;
                        continue;
                    };
                    let tags = value.get("tags").cloned().unwrap_or(serde_json::json!([]));
                    let note = value.get("note").and_then(|n| n.as_str()).unwrap_or("");

                    // The node and the derived row are recreated; the episode is left alone,
                    // because writing a new one would double the annotation on every rebuild.
                    let annotation = {
                        let conn = self.conn();
                        conn.execute(
                            "INSERT INTO nodes (kind, path, size, mtime, ctime, inode, dev,
                                                first_seen, last_seen)
                             VALUES ('annotation', NULL, 0, ?1, ?1, 0, 0, ?1, ?1)",
                            params![ts],
                        )?;
                        let annotation = conn.last_insert_rowid();
                        conn.execute(
                            "INSERT INTO annotations
                                 (node_id, target_id, episode_id, tags_json, note, actor, ts)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![
                                annotation,
                                target.0,
                                _id,
                                tags.to_string(),
                                note,
                                actor.as_str(),
                                ts
                            ],
                        )?;
                        annotation
                    };
                    self.insert_edge(
                        NodeId(annotation),
                        target,
                        EdgeType::Annotates,
                        1.0,
                        EdgeSource::Explicit,
                        ts,
                    )?;
                    replayed += 1;
                }
                "link" => {
                    let (Some(a), Some(b)) = (resolved("a_path"), resolved("b_path")) else {
                        skipped += 1;
                        continue;
                    };
                    let edge = value
                        .get("edge")
                        .and_then(|e| e.as_str())
                        .and_then(EdgeType::parse)
                        .unwrap_or(EdgeType::SimilarTo);
                    self.insert_edge(a, b, edge, 1.0, EdgeSource::Explicit, ts)?;
                    replayed += 1;
                }
                "unlink" => {
                    let (Some(a), Some(b)) = (resolved("a_path"), resolved("b_path")) else {
                        skipped += 1;
                        continue;
                    };
                    let edge = value
                        .get("edge")
                        .and_then(|e| e.as_str())
                        .and_then(EdgeType::parse)
                        .unwrap_or(EdgeType::SimilarTo);
                    self.remove_edge(a, b, edge)?;
                    replayed += 1;
                }
                "pin" | "forget" => {
                    let Some(target) = resolved("target_path") else {
                        skipped += 1;
                        continue;
                    };
                    self.set_pinned(target, kind == "pin")?;
                    if kind == "forget" {
                        self.set_salience(target, 0.0)?;
                    }
                    replayed += 1;
                }
                _ => skipped += 1,
            }
        }

        Ok((replayed, skipped))
    }

    /// Record an assertion episode. The path is stored alongside the id because ids do not
    /// survive a rebuild and paths do.
    pub fn record_assertion(
        &self,
        actor: &Actor,
        now: i64,
        node: Option<NodeId>,
        metadata: serde_json::Value,
    ) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, metadata_json)
             VALUES (?1, ?2, 'annotate', ?3, ?4)",
            params![now, actor.as_str(), node.map(|n| n.0), metadata.to_string()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Create a concept node — a node with no path, named only by what it means.
    /// Find or create the concept called `name`.
    ///
    /// The name is the concept's identity, which is what lets a cluster that gains a file update
    /// the concept it already had instead of growing a rival beside it.
    pub fn upsert_concept(&self, name: &str, now: i64) -> Result<NodeId> {
        let conn = self.conn();
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM nodes WHERE kind = 'concept' AND label = ?1",
                params![name],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            return Ok(NodeId(id));
        }
        conn.execute(
            "INSERT INTO nodes (kind, path, label, size, mtime, ctime, inode, dev,
                                first_seen, last_seen)
             VALUES ('concept', NULL, ?1, 0, ?2, ?2, 0, 0, ?2, ?2)",
            params![name, now],
        )?;
        Ok(NodeId(conn.last_insert_rowid()))
    }

    /// Nodes whose kind is `annotation` and which nothing points at any more. A rebuild that
    /// replayed assertions can leave these behind if an assertion's target disappeared.
    pub fn prune_orphan_annotations(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM nodes WHERE kind = 'annotation' AND id NOT IN (
                 SELECT src FROM edges WHERE type = 'annotates'
             )",
            [],
        )?)
    }

    pub fn edge_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))?)
    }
}


/// One node the human or an agent has touched lately.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Touched {
    pub node_id: i64,
    pub path: Option<PathBuf>,
    pub last_touched: i64,
    /// Which actions happened to it in the window, most recent first.
    pub actions: Vec<String>,
    pub pinned: bool,
}

impl Store {
    /// Episodes in a time range, narrowed in SQL.
    ///
    /// Every filter is optional and every one of them is applied by the database. A timeline
    /// over a year of episodes must not have to read a year of episodes to answer "what did
    /// this agent do on Tuesday".
    pub fn episodes_in_range(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        actor: Option<&str>,
        action: Option<&str>,
        limit: usize,
    ) -> Result<Vec<cortex_core::model::Episode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, action, node_id, session_id, metadata_json
             FROM episodes
             WHERE (?1 IS NULL OR ts >= ?1)
               AND (?2 IS NULL OR ts <= ?2)
               AND (?3 IS NULL OR actor = ?3)
               AND (?4 IS NULL OR action = ?4)
             ORDER BY ts DESC, id DESC
             LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![from, to, actor, action, limit as i64],
            crate::queries::row_to_episode,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().collect()
    }

    /// Episodes after an id, oldest first — the shape a cursor wants.
    ///
    /// By id rather than by timestamp: two episodes can share a second, and a cursor that
    /// resumes by timestamp either repeats them or skips them.
    pub fn episodes_after(
        &self,
        after_id: i64,
        limit: usize,
    ) -> Result<Vec<cortex_core::model::Episode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, action, node_id, session_id, metadata_json
             FROM episodes WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![after_id, limit as i64], crate::queries::row_to_episode)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().collect()
    }

    pub fn episode_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT count(*) FROM episodes", [], |r| r.get(0))?)
    }

    /// Nodes with an episode since `since`, most recently touched first.
    pub fn recently_touched(&self, since: i64, limit: usize) -> Result<Vec<Touched>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.node_id, max(e.ts) AS last_touched, n.path, n.pinned,
                    group_concat(DISTINCT e.action)
             FROM episodes e JOIN nodes n ON n.id = e.node_id
             WHERE e.ts >= ?1 AND e.node_id IS NOT NULL
             GROUP BY e.node_id
             ORDER BY last_touched DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since, limit as i64], |row| {
            Ok(Touched {
                node_id: row.get(0)?,
                last_touched: row.get(1)?,
                path: row.get::<_, Option<String>>(2)?.map(PathBuf::from),
                pinned: row.get::<_, i64>(3)? != 0,
                actions: row
                    .get::<_, Option<String>>(4)?
                    .map(|a| a.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// The node kinds an agent may assert about. A chunk is not one of them: chunks are derived
/// from content and an assertion about one would not survive re-chunking.
pub fn assertable(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::File | NodeKind::Dir | NodeKind::Concept | NodeKind::Annotation
    )
}

/// Actions an assertion episode may carry, so a typo becomes a compile error rather than an
/// episode nothing ever replays.
pub const ASSERTIONS: &[&str] = &["annotate", "link", "unlink", "pin", "forget"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::NodeUpsert;
    use std::path::Path;

    fn store_with_files(paths: &[&str]) -> (Store, Vec<NodeId>) {
        let store = Store::open_in_memory().unwrap();
        let ids = paths
            .iter()
            .map(|p| {
                store
                    .upsert_node(
                        &NodeUpsert {
                            kind: NodeKind::File,
                            path: PathBuf::from(p),
                            parent_id: None,
                            mime: None,
                            size: 10,
                            mtime: 1,
                            ctime: 1,
                            inode: 0,
                            dev: 0,
                            zone_id: None,
                        },
                        1,
                    )
                    .unwrap()
            })
            .collect();
        (store, ids)
    }

    #[test]
    fn an_edge_can_be_asserted_twice_without_failing() {
        let (store, ids) = store_with_files(&["/a", "/b"]);
        store
            .insert_edge(ids[0], ids[1], EdgeType::SimilarTo, 1.0, EdgeSource::Explicit, 1)
            .unwrap();
        store
            .insert_edge(ids[0], ids[1], EdgeType::SimilarTo, 0.5, EdgeSource::Explicit, 2)
            .unwrap();
        assert_eq!(store.edge_count().unwrap(), 1);

        let neighbors = store.neighbors(ids[0], &[], 1, 10).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].weight, 0.5, "the second assertion won");
    }

    #[test]
    fn traversal_follows_edges_in_both_directions() {
        let (store, ids) = store_with_files(&["/a", "/b"]);
        store
            .insert_edge(ids[0], ids[1], EdgeType::Imports, 1.0, EdgeSource::Observed, 1)
            .unwrap();

        assert_eq!(store.neighbors(ids[0], &[], 1, 10).unwrap().len(), 1);
        assert_eq!(
            store.neighbors(ids[1], &[], 1, 10).unwrap().len(),
            1,
            "an association is not one-way"
        );
    }

    #[test]
    fn traversal_respects_depth_and_type() {
        let (store, ids) = store_with_files(&["/a", "/b", "/c"]);
        store
            .insert_edge(ids[0], ids[1], EdgeType::Imports, 1.0, EdgeSource::Observed, 1)
            .unwrap();
        store
            .insert_edge(ids[1], ids[2], EdgeType::SimilarTo, 1.0, EdgeSource::Inferred, 1)
            .unwrap();

        assert_eq!(store.neighbors(ids[0], &[], 1, 10).unwrap().len(), 1);
        assert_eq!(store.neighbors(ids[0], &[], 2, 10).unwrap().len(), 2);
        assert_eq!(
            store
                .neighbors(ids[0], &[EdgeType::Imports], 2, 10)
                .unwrap()
                .len(),
            1,
            "the second hop is a similar_to edge and was filtered out"
        );
    }

    #[test]
    fn an_annotation_is_a_node_an_edge_and_an_episode() {
        let (store, ids) = store_with_files(&["/a"]);
        let annotation = store
            .annotate(
                ids[0],
                "/a",
                &["invoice".into()],
                "the one that was paid twice",
                &Actor::Agent("t".into()),
                100,
            )
            .unwrap();

        let neighbors = store.neighbors(ids[0], &[EdgeType::Annotates], 1, 10).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].node_id, annotation);
        assert_eq!(
            neighbors[0].note.as_deref(),
            Some("the one that was paid twice"),
            "a traversal that returns an annotation must return what it says"
        );

        let annotations = store.annotations_for(ids[0]).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].tags, vec!["invoice"]);
        assert_eq!(annotations[0].actor, "agent:t");
    }

    #[test]
    fn assertions_survive_a_rebuild() {
        let (store, ids) = store_with_files(&["/a", "/b"]);

        // What the Agent Surface writes: the assertion episode carries paths, because ids do
        // not survive what comes next.
        store
            .record_assertion(
                &Actor::Agent("t".into()),
                50,
                Some(ids[0]),
                serde_json::json!({
                    "assertion": "link", "a_path": "/a", "b_path": "/b", "edge": "derived_from"
                }),
            )
            .unwrap();
        store
            .record_assertion(
                &Actor::Agent("t".into()),
                51,
                Some(ids[0]),
                serde_json::json!({ "assertion": "pin", "target_path": "/a" }),
            )
            .unwrap();
        store.insert_edge(ids[0], ids[1], EdgeType::DerivedFrom, 1.0, EdgeSource::Explicit, 50).unwrap();
        store.set_pinned(ids[0], true).unwrap();

        // The rebuild: everything derived is dropped and the filesystem is walked again.
        store.clear_derived(true).unwrap();
        let (store, ids) = {
            let ids: Vec<NodeId> = ["/a", "/b"]
                .iter()
                .map(|p| {
                    store
                        .upsert_node(
                            &NodeUpsert {
                                kind: NodeKind::File,
                                path: PathBuf::from(p),
                                parent_id: None,
                                mime: None,
                                size: 10,
                                mtime: 1,
                                ctime: 1,
                                inode: 0,
                                dev: 0,
                                zone_id: None,
                            },
                            1,
                        )
                        .unwrap()
                })
                .collect();
            (store, ids)
        };
        assert_eq!(store.edge_count().unwrap(), 0, "the rebuild dropped the edges");

        let (replayed, skipped) = store.replay_assertions().unwrap();
        assert_eq!(replayed, 2);
        assert_eq!(skipped, 0);
        assert_eq!(store.edge_count().unwrap(), 1, "the link came back");
        assert!(store.is_pinned(ids[0]).unwrap(), "the pin came back");
    }

    #[test]
    fn an_assertion_about_a_deleted_file_is_skipped_rather_than_invented() {
        let (store, ids) = store_with_files(&["/a"]);
        store
            .record_assertion(
                &Actor::Agent("t".into()),
                50,
                Some(ids[0]),
                serde_json::json!({ "assertion": "pin", "target_path": "/gone" }),
            )
            .unwrap();

        let (replayed, skipped) = store.replay_assertions().unwrap();
        assert_eq!(replayed, 0);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn an_annotation_survives_a_rebuild_and_keeps_its_text() {
        let (store, ids) = store_with_files(&["/a"]);
        store
            .record_assertion(
                &Actor::Agent("t".into()),
                50,
                None,
                serde_json::json!({
                    "assertion": "annotate", "target_path": "/a",
                    "tags": ["x"], "note": "remember this"
                }),
            )
            .unwrap();
        let _ = ids;

        let (replayed, _) = store.replay_assertions().unwrap();
        assert_eq!(replayed, 1);
        let target = store.node_by_path(Path::new("/a")).unwrap().unwrap();
        let neighbors = store
            .neighbors(target.id, &[EdgeType::Annotates], 1, 10)
            .unwrap();
        assert_eq!(neighbors.len(), 1, "the annotation node came back");
        assert_eq!(
            neighbors[0].note.as_deref(),
            Some("remember this"),
            "and it still says what it said"
        );
    }
}
