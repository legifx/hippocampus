//! Chunk storage and the lexical index.
//!
//! ## Why replacing chunks is not "delete then insert"
//!
//! An embedding is keyed by chunk id and cascades when the chunk row goes away. Deleting every
//! chunk of a file and inserting fresh rows would therefore throw away every embedding for that
//! file on every save — which is precisely the cost the per-chunk hash exists to avoid, and the
//! build prompt calls it the single largest performance lever in the whole ingest pipeline.
//!
//! So [`replace_chunks`] diffs. A chunk whose hash already exists for this node keeps its row,
//! its id and its embedding, and is merely repositioned. Matching is by hash rather than by
//! ordinal because inserting a paragraph at the top of a document shifts every ordinal while
//! changing no text at all — the case where reuse matters most.
//!
//! ## Why the ordinals are parked first
//!
//! `chunks` has a unique index on `(node_id, ordinal)`. Rewriting the surviving rows in place
//! collides with rows that have not been rewritten yet, so survivors are first moved to
//! negative ordinals — a space no real chunk occupies — and then given their final positions.
//!
//! ## The FTS index
//!
//! `chunks_fts` is contentless (decision D16): it tokenises text for BM25 and keeps no copy.
//! It is not a table that can be joined to for text, only matched against, and its `rowid` is
//! the chunk id. Every write here must keep the two in step, because nothing else will.

use crate::error::{Result, StoreError};
use crate::queries::path_to_str;
use crate::Store;
use cortex_core::ids::{ChunkId, NodeId};
use cortex_core::model::ExtractState;
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One chunk on its way into the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkWrite {
    pub ordinal: i64,
    /// Byte offsets into the *extracted text* of the file, not into the file itself. For plain
    /// text the two coincide except for a byte order mark; for anything with a real extractor
    /// they do not, which is why a snippet is produced by re-extracting rather than by seeking.
    pub byte_start: i64,
    pub byte_end: i64,
    /// BLAKE3 of the chunk text. Identity for reuse.
    pub hash: String,
    /// Bounded excerpt for display. Never the chunk itself (D16).
    pub preview: Option<String>,
    /// Tokenised into `chunks_fts` and then dropped. This is the only place chunk text is
    /// handled at all, and it is never written to a column.
    pub text: String,
}

/// What one call to [`replace_chunks`] actually did.
///
/// `reused` is the number that matters: it is the count of chunks that will not be re-embedded,
/// and the assertion behind "re-indexing an unchanged corpus re-embeds zero chunks".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkDelta {
    pub inserted: usize,
    pub reused: usize,
    pub deleted: usize,
}

impl ChunkDelta {
    pub fn is_noop(&self) -> bool {
        self.inserted == 0 && self.deleted == 0
    }
}

/// A chunk as stored. No text — by design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChunk {
    pub id: ChunkId,
    pub node_id: NodeId,
    pub ordinal: i64,
    pub byte_start: i64,
    pub byte_end: i64,
    pub hash: String,
    pub preview: Option<String>,
}

/// A file the extraction stage still has to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNode {
    pub id: NodeId,
    pub path: PathBuf,
    pub size: i64,
    /// What the index believes the extracted text hashed to last time. `None` means never
    /// extracted; equality means the file can be skipped without re-chunking it.
    pub content_hash: Option<String>,
    /// Chunks currently stored for this node.
    ///
    /// Carried because the hash alone is not enough to decide that there is nothing to do. A
    /// rebuilt index has every hash and no chunks — that is exactly what `--rebuild-index`
    /// produces — and trusting the hash there would leave the corpus permanently unsearchable.
    pub chunk_count: i64,
}

/// One chunk that matched a lexical query.
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalHit {
    pub chunk_id: ChunkId,
    pub node_id: NodeId,
    pub path: PathBuf,
    pub ordinal: i64,
    pub byte_start: i64,
    pub byte_end: i64,
    /// The hash the chunk had when it was indexed. Carried so a snippet can prove the file
    /// still says what the index thinks it says before quoting it.
    pub hash: String,
    pub preview: Option<String>,
    /// Relevance, higher is better.
    ///
    /// SQLite's `bm25()` returns a *negative* score with more negative meaning a better match.
    /// It is negated exactly here, once, so that no downstream comparison has to remember the
    /// sign — getting it wrong inverts the ranking and nothing crashes.
    pub score: f64,
}

/// Where a chunk is, without any opinion about how it was found.
///
/// The vector index knows chunk ids and nothing else; this is how they become something a result
/// list can show. Deliberately the same fields the lexical arm carries, so both arms hand the
/// snippet path identical information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLocation {
    pub chunk_id: ChunkId,
    pub node_id: NodeId,
    pub path: PathBuf,
    pub ordinal: i64,
    pub byte_start: i64,
    pub byte_end: i64,
    pub hash: String,
    pub preview: Option<String>,
}

/// How much of the index has been through extraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtractProgress {
    pub pending: i64,
    pub done: i64,
    pub skipped: i64,
    pub failed: i64,
    pub chunks: i64,
}

impl crate::queries::Batch<'_> {
    pub fn replace_chunks(&self, node: NodeId, chunks: &[ChunkWrite]) -> Result<ChunkDelta> {
        replace_chunks_on(self.tx(), node, chunks)
    }

    pub fn delete_chunks(&self, node: NodeId) -> Result<usize> {
        delete_chunks_on(self.tx(), node)
    }

    pub fn set_extract_state(
        &self,
        node: NodeId,
        state: ExtractState,
        note: Option<&str>,
    ) -> Result<()> {
        set_extract_state_on(self.tx(), node, state, note)
    }

    /// Record the whole result of one extraction: what the content hashed to, what it turned
    /// out to be, and how far the stage got. One statement because the three are one fact — a
    /// state that disagrees with the hash beside it is how a file becomes permanently stale.
    pub fn set_extraction_result(
        &self,
        node: NodeId,
        hash: Option<&str>,
        mime: Option<&str>,
        state: ExtractState,
        note: Option<&str>,
    ) -> Result<()> {
        set_extraction_result_on(self.tx(), node, hash, mime, state, note)
    }
}

impl Store {
    /// Replace a node's chunks, keeping every chunk whose text has not changed.
    pub fn replace_chunks(&self, node: NodeId, chunks: &[ChunkWrite]) -> Result<ChunkDelta> {
        self.batch(|b| b.replace_chunks(node, chunks))
    }

    pub fn delete_chunks(&self, node: NodeId) -> Result<usize> {
        self.batch(|b| b.delete_chunks(node))
    }

    pub fn set_extract_state(
        &self,
        node: NodeId,
        state: ExtractState,
        note: Option<&str>,
    ) -> Result<()> {
        set_extract_state_on(&self.conn(), node, state, note)
    }

    pub fn chunks_for_node(&self, node: NodeId) -> Result<Vec<StoredChunk>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, node_id, ordinal, byte_start, byte_end, hash, text_preview
             FROM chunks WHERE node_id = ?1 ORDER BY ordinal",
        )?;
        let rows = stmt
            .query_map([node.0], |r| {
                Ok(StoredChunk {
                    id: ChunkId(r.get(0)?),
                    node_id: NodeId(r.get(1)?),
                    ordinal: r.get(2)?,
                    byte_start: r.get(3)?,
                    byte_end: r.get(4)?,
                    hash: r.get(5)?,
                    preview: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The next files awaiting extraction, in index order.
    ///
    /// **Not newest first**, although that was the first design and it is the more attractive
    /// one: a file the user just touched is the one they are most likely to search for. It was
    /// measured and abandoned. `ORDER BY mtime DESC` cannot use the partial index on
    /// `extract_state`, so every batch read the entire backlog, evaluated the chunk count for
    /// each row and sorted the result: **11 ms per batch with 50 000 files pending, against
    /// 6 µs in index order.** The queue query would have cost more than the extraction it feeds,
    /// and the cost would have grown with the size of the backlog — worst exactly when the
    /// backlog is worst.
    ///
    /// Index order means the partial index yields pending rows directly and `LIMIT` stops after
    /// the first few. The queue drains in the order files were discovered, which is a weaker
    /// guarantee than recency and the only one that is free.
    pub fn pending_extraction(&self, limit: usize) -> Result<Vec<PendingNode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, content_hash,
                    -- Evaluated for the selected rows only. With no ORDER BY there is nothing
                    -- to materialise first, so this runs `limit` times rather than once per
                    -- pending file.
                    (SELECT count(*) FROM chunks WHERE chunks.node_id = nodes.id)
             FROM nodes
             WHERE extract_state = 'pending' AND kind = 'file' AND path IS NOT NULL
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok(PendingNode {
                    id: NodeId(r.get(0)?),
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    size: r.get(2)?,
                    content_hash: r.get(3)?,
                    chunk_count: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The subset of these nodes that is still waiting for extraction.
    ///
    /// For the priority path: the watcher knows which files a person just touched, and those
    /// must not wait behind a first-index backlog. Ids that are already done, or were never
    /// files, are simply absent from the result rather than being an error — the caller is
    /// working from a queue that may be stale by the time it is read.
    pub fn pending_by_ids(&self, ids: &[NodeId]) -> Result<Vec<PendingNode>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Built rather than bound as one parameter because SQLite has no array type. The list
        // is bounded by the caller's batch size and every element is an integer, so there is
        // nothing here a string can smuggle in.
        let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, path, size, content_hash,
                    (SELECT count(*) FROM chunks WHERE chunks.node_id = nodes.id)
             FROM nodes
             WHERE id IN ({placeholders})
               AND extract_state = 'pending' AND kind = 'file' AND path IS NOT NULL"
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter().map(|id| id.0)), |r| {
                Ok(PendingNode {
                    id: NodeId(r.get(0)?),
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    size: r.get(2)?,
                    content_hash: r.get(3)?,
                    chunk_count: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Salience for a set of nodes, in one query.
    ///
    /// Ranking needs this for every candidate, not only for the ones that end up displayed: a
    /// boost that is applied after truncating the list cannot promote anything into it, which
    /// makes it decoration rather than ranking. One query for the candidate set is the cheap
    /// way to have it both ways. Nodes that no longer exist are absent rather than zero, so a
    /// caller can tell "not salient" from "not there".
    pub fn salience_of(&self, ids: &[NodeId]) -> Result<HashMap<NodeId, f64>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
        let conn = self.conn();
        let mut stmt =
            conn.prepare(&format!("SELECT id, salience FROM nodes WHERE id IN ({placeholders})"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter().map(|id| id.0)), |r| {
                Ok((NodeId(r.get(0)?), r.get(1)?))
            })?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(rows)
    }

    /// Resolve chunk ids to their locations, for the vector arm.
    ///
    /// Ids that no longer exist are absent rather than an error: the vector index is a cache and
    /// may briefly know about a chunk the database has already deleted.
    pub fn chunk_locations(&self, ids: &[ChunkId]) -> Result<Vec<ChunkLocation>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT c.id, c.node_id, n.path, c.ordinal, c.byte_start, c.byte_end, c.hash,
                    c.text_preview
             FROM chunks c JOIN nodes n ON n.id = c.node_id
             WHERE c.id IN ({placeholders}) AND n.path IS NOT NULL"
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter().map(|id| id.0)), |r| {
                Ok(ChunkLocation {
                    chunk_id: ChunkId(r.get(0)?),
                    node_id: NodeId(r.get(1)?),
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    ordinal: r.get(3)?,
                    byte_start: r.get(4)?,
                    byte_end: r.get(5)?,
                    hash: r.get(6)?,
                    preview: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A cheap fingerprint of the chunk table: how many rows, and the highest id.
    ///
    /// The embedding stage needs to ask "is there anything to do" every so often, and the honest
    /// version of that question — a left join looking for chunks without a vector — scans every
    /// chunk in the index. Measured on the reference corpus that was 0.84 % of one core spent
    /// discovering that there was nothing to do, against a whole-daemon idle budget of 0.5 %.
    ///
    /// Both numbers come from indexes and are effectively free. Together they change whenever a
    /// chunk is added, removed or re-derived, which is exactly when the answer to the real
    /// question can have changed.
    pub fn chunk_watermark(&self) -> Result<(i64, i64)> {
        let conn = self.conn();
        Ok(conn.query_row(
            "SELECT count(*), coalesce(max(id), 0) FROM chunks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    pub fn extract_progress(&self) -> Result<ExtractProgress> {
        let conn = self.conn();
        let mut progress = ExtractProgress::default();
        let mut stmt = conn.prepare(
            "SELECT extract_state, count(*) FROM nodes WHERE kind = 'file' GROUP BY extract_state",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let state: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            match ExtractState::parse(&state) {
                Some(ExtractState::Pending) => progress.pending = count,
                Some(ExtractState::Done) => progress.done = count,
                Some(ExtractState::Skipped) => progress.skipped = count,
                Some(ExtractState::Failed) => progress.failed = count,
                None => return Err(StoreError::corrupt(format!("unknown extract state: {state}"))),
            }
        }
        progress.chunks = conn.query_row("SELECT count(*) FROM chunks", [], |r| r.get(0))?;
        Ok(progress)
    }

    /// Queue everything for extraction again.
    ///
    /// Needed whenever the *decision* changes rather than the file: adding a content root makes
    /// files that were skipped for being outside one eligible, and nothing about those files
    /// changed to trigger a re-read. Chunks are kept, because the hash comparison will find
    /// them unchanged and reuse them; this costs a re-read, not a re-embed.
    pub fn requeue_extraction(&self) -> Result<usize> {
        let conn = self.conn();
        Ok(conn.execute(
            "UPDATE nodes SET extract_state = 'pending', extract_note = NULL
             WHERE kind = 'file' AND extract_state != 'pending'",
            [],
        )?)
    }

    /// Rank chunks against a prepared FTS5 match expression.
    ///
    /// The expression is built by `cortex_query::lexical`, never by string-pasting user input:
    /// a bare apostrophe or a stray `NEAR` is a syntax error to FTS5, and a search that fails
    /// on a quote mark is a search nobody trusts.
    pub fn search_chunks(&self, match_expr: &str, limit: usize) -> Result<Vec<LexicalHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.node_id, n.path, c.ordinal, c.byte_start, c.byte_end,
                    c.hash, c.text_preview, bm25(chunks_fts)
             FROM chunks_fts
             JOIN chunks c ON c.id = chunks_fts.rowid
             JOIN nodes  n ON n.id = c.node_id
             WHERE chunks_fts MATCH ?1 AND n.path IS NOT NULL
             ORDER BY bm25(chunks_fts)
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![match_expr, limit as i64], |r| {
                let bm25: f64 = r.get(8)?;
                Ok(LexicalHit {
                    chunk_id: ChunkId(r.get(0)?),
                    node_id: NodeId(r.get(1)?),
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    ordinal: r.get(3)?,
                    byte_start: r.get(4)?,
                    byte_end: r.get(5)?,
                    hash: r.get(6)?,
                    preview: r.get(7)?,
                    score: -bm25,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The node behind a path, plus what extraction thinks of it. For `cortex explain`.
    pub fn extract_note(&self, path: &Path) -> Result<Option<(ExtractState, Option<String>)>> {
        let conn = self.conn();
        let path = path_to_str(path)?;
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT extract_state, extract_note FROM nodes WHERE path = ?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((state, note)) => {
                let state = ExtractState::parse(&state)
                    .ok_or_else(|| StoreError::corrupt(format!("unknown extract state: {state}")))?;
                Ok(Some((state, note)))
            }
        }
    }
}

fn set_extract_state_on(
    conn: &rusqlite::Connection,
    node: NodeId,
    state: ExtractState,
    note: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE nodes SET extract_state = ?2, extract_note = ?3 WHERE id = ?1",
        params![node.0, state.as_str(), note],
    )?;
    Ok(())
}

fn set_extraction_result_on(
    conn: &rusqlite::Connection,
    node: NodeId,
    hash: Option<&str>,
    mime: Option<&str>,
    state: ExtractState,
    note: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE nodes
         SET content_hash = coalesce(?2, content_hash),
             mime = coalesce(?3, mime),
             extract_state = ?4,
             extract_note = ?5
         WHERE id = ?1",
        params![node.0, hash, mime, state.as_str(), note],
    )?;
    Ok(())
}

fn delete_chunks_on(conn: &rusqlite::Connection, node: NodeId) -> Result<usize> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM chunks WHERE node_id = ?1")?;
        let ids = stmt
            .query_map([node.0], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        ids
    };
    for id in &ids {
        conn.execute("DELETE FROM chunks_fts WHERE rowid = ?1", [id])?;
        conn.execute("INSERT OR IGNORE INTO dead_chunks (chunk_id) VALUES (?1)", [id])?;
    }
    conn.execute("DELETE FROM chunks WHERE node_id = ?1", [node.0])?;
    Ok(ids.len())
}

fn replace_chunks_on(
    conn: &rusqlite::Connection,
    node: NodeId,
    chunks: &[ChunkWrite],
) -> Result<ChunkDelta> {
    let existing: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, hash FROM chunks WHERE node_id = ?1")?;
        let rows = stmt
            .query_map([node.0], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    // Several chunks of one file can legitimately share a hash — a repeated boilerplate
    // paragraph, or two empty sections — so the map holds every candidate id per hash and they
    // are consumed one at a time.
    let mut by_hash: HashMap<&str, Vec<i64>> = HashMap::new();
    for (id, hash) in &existing {
        by_hash.entry(hash.as_str()).or_default().push(*id);
    }

    let mut delta = ChunkDelta::default();
    let mut reused_ids: Vec<Option<i64>> = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let id = by_hash
            .get_mut(chunk.hash.as_str())
            .and_then(|candidates| candidates.pop());
        if id.is_some() {
            delta.reused += 1;
        }
        reused_ids.push(id);
    }

    // Whatever was not claimed is gone from the file.
    for (_, leftovers) in by_hash.iter() {
        for id in leftovers {
            conn.execute("DELETE FROM chunks_fts WHERE rowid = ?1", [id])?;
            conn.execute("DELETE FROM chunks WHERE id = ?1", [id])?;
            // The vector index keys by chunk id and cannot be reached from here; the
            // embedding stage removes the vector from this note (0009 migration).
            conn.execute("INSERT OR IGNORE INTO dead_chunks (chunk_id) VALUES (?1)", [id])?;
            delta.deleted += 1;
        }
    }

    // Park the survivors below zero so that assigning final ordinals cannot collide with an
    // ordinal that has not been rewritten yet.
    for id in reused_ids.iter().flatten() {
        conn.execute(
            "UPDATE chunks SET ordinal = ?2 WHERE id = ?1",
            params![id, -(id + 1)],
        )?;
    }

    for (chunk, reused) in chunks.iter().zip(reused_ids.iter()) {
        match reused {
            // Same text, so the FTS row is already correct and must not be rewritten — that
            // would be the one operation in this file that costs tokenisation for nothing.
            Some(id) => {
                conn.execute(
                    "UPDATE chunks SET ordinal = ?2, byte_start = ?3, byte_end = ?4,
                                       text_preview = ?5
                     WHERE id = ?1",
                    params![id, chunk.ordinal, chunk.byte_start, chunk.byte_end, chunk.preview],
                )?;
            }
            None => {
                conn.execute(
                    "INSERT INTO chunks (node_id, ordinal, byte_start, byte_end, hash, text_preview)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        node.0,
                        chunk.ordinal,
                        chunk.byte_start,
                        chunk.byte_end,
                        chunk.hash,
                        chunk.preview
                    ],
                )?;
                let id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO chunks_fts (rowid, text) VALUES (?1, ?2)",
                    params![id, chunk.text],
                )?;
                delta.inserted += 1;
            }
        }
    }

    Ok(delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::NodeUpsert;
    use cortex_core::model::NodeKind;

    fn store_with_file(name: &str) -> (Store, NodeId) {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::File,
                    path: PathBuf::from("/tmp/corpus").join(name),
                    parent_id: None,
                    mime: None,
                    size: 10,
                    mtime: 100,
                    ctime: 100,
                    inode: 1,
                    dev: 1,
                    zone_id: None,
                },
                1000,
            )
            .unwrap();
        (store, id)
    }

    fn chunk(ordinal: i64, text: &str) -> ChunkWrite {
        ChunkWrite {
            ordinal,
            byte_start: ordinal * 100,
            byte_end: ordinal * 100 + text.len() as i64,
            hash: blake3_hex(text),
            preview: Some(text.chars().take(20).collect()),
            text: text.to_string(),
        }
    }

    /// The chunker hashes with BLAKE3; the store only cares that equal text means equal hash.
    fn blake3_hex(text: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    #[test]
    fn writing_chunks_makes_them_findable_by_content() {
        let (store, node) = store_with_file("notes.md");
        let delta = store
            .replace_chunks(node, &[chunk(0, "kamerawinkel und blickachse"), chunk(1, "budget")])
            .unwrap();
        assert_eq!(delta.inserted, 2);

        let hits = store.search_chunks("kamerawinkel", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, node);
        assert_eq!(hits[0].ordinal, 0);
    }

    #[test]
    fn bm25_is_reported_as_higher_is_better() {
        // The whole point of negating in one place: if this ever flips, every ranking in the
        // system inverts silently and nothing fails except relevance.
        let (store, node) = store_with_file("a.md");
        store
            .replace_chunks(
                node,
                &[
                    chunk(0, "aurora aurora aurora aurora"),
                    chunk(1, "aurora mentioned once among many other unrelated words here"),
                ],
            )
            .unwrap();

        let hits = store.search_chunks("aurora", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(
            hits[0].score > hits[1].score,
            "expected descending relevance, got {:?}",
            hits.iter().map(|h| h.score).collect::<Vec<_>>()
        );
        assert!(hits[0].score > 0.0, "relevance must be positive, got {}", hits[0].score);
        assert_eq!(hits[0].ordinal, 0, "the denser chunk must rank first");
    }

    #[test]
    fn an_unchanged_file_reuses_every_chunk_row() {
        // This is the assertion behind "re-indexing an unchanged corpus re-embeds zero chunks".
        let (store, node) = store_with_file("stable.md");
        let chunks = vec![chunk(0, "first section"), chunk(1, "second section")];
        store.replace_chunks(node, &chunks).unwrap();
        let before: Vec<ChunkId> = store
            .chunks_for_node(node)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();

        let delta = store.replace_chunks(node, &chunks).unwrap();
        assert_eq!(delta, ChunkDelta { inserted: 0, reused: 2, deleted: 0 });

        let after: Vec<ChunkId> = store
            .chunks_for_node(node)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(before, after, "chunk ids must survive, or embeddings die with them");
    }

    #[test]
    fn inserting_a_paragraph_at_the_top_reuses_the_paragraphs_below_it() {
        // Matching by ordinal would report every chunk as changed here. Matching by hash is the
        // difference between one embedding call and a whole document's worth.
        let (store, node) = store_with_file("shifted.md");
        store
            .replace_chunks(node, &[chunk(0, "alpha"), chunk(1, "beta"), chunk(2, "gamma")])
            .unwrap();

        let delta = store
            .replace_chunks(
                node,
                &[chunk(0, "new opening"), chunk(1, "alpha"), chunk(2, "beta"), chunk(3, "gamma")],
            )
            .unwrap();

        assert_eq!(delta.reused, 3);
        assert_eq!(delta.inserted, 1);
        assert_eq!(delta.deleted, 0);

        let stored = store.chunks_for_node(node).unwrap();
        let ordinals: Vec<i64> = stored.iter().map(|c| c.ordinal).collect();
        assert_eq!(ordinals, vec![0, 1, 2, 3], "ordinals must be dense and in order");
    }

    #[test]
    fn a_removed_chunk_leaves_the_lexical_index() {
        // Contentless FTS could not delete before migration 0002. If that ever regresses, a
        // deleted paragraph stays searchable forever and the index quietly lies.
        let (store, node) = store_with_file("shrinking.md");
        store
            .replace_chunks(node, &[chunk(0, "keeper"), chunk(1, "vanishing kamerawinkel")])
            .unwrap();
        assert_eq!(store.search_chunks("kamerawinkel", 10).unwrap().len(), 1);

        let delta = store.replace_chunks(node, &[chunk(0, "keeper")]).unwrap();
        assert_eq!(delta.deleted, 1);
        assert!(
            store.search_chunks("kamerawinkel", 10).unwrap().is_empty(),
            "the removed text is still in the lexical index"
        );
    }

    #[test]
    fn deleting_a_node_takes_its_chunks_and_their_index_entries() {
        let (store, node) = store_with_file("doomed.md");
        store.replace_chunks(node, &[chunk(0, "ephemeral")]).unwrap();
        store.delete_chunks(node).unwrap();
        assert!(store.chunks_for_node(node).unwrap().is_empty());
        assert!(store.search_chunks("ephemeral", 10).unwrap().is_empty());
    }

    #[test]
    fn repeated_text_within_one_file_is_reused_once_per_occurrence() {
        let (store, node) = store_with_file("repetitive.md");
        store
            .replace_chunks(node, &[chunk(0, "same"), chunk(1, "same"), chunk(2, "same")])
            .unwrap();

        let delta = store
            .replace_chunks(node, &[chunk(0, "same"), chunk(1, "same")])
            .unwrap();
        assert_eq!(delta, ChunkDelta { inserted: 0, reused: 2, deleted: 1 });
    }

    #[test]
    fn extraction_state_moves_through_its_stages_and_is_countable() {
        let (store, node) = store_with_file("tracked.md");
        let progress = store.extract_progress().unwrap();
        assert_eq!(progress.pending, 1, "a freshly indexed file starts pending");

        store.set_extract_state(node, ExtractState::Done, None).unwrap();
        assert_eq!(store.extract_progress().unwrap().done, 1);
        assert!(store.pending_extraction(10).unwrap().is_empty());

        store
            .set_extract_state(node, ExtractState::Skipped, Some("outside the content roots"))
            .unwrap();
        let (state, note) = store
            .extract_note(&PathBuf::from("/tmp/corpus/tracked.md"))
            .unwrap()
            .unwrap();
        assert_eq!(state, ExtractState::Skipped);
        assert_eq!(note.as_deref(), Some("outside the content roots"));
    }

    #[test]
    fn a_hash_without_chunks_is_reported_so_a_rebuilt_index_is_not_trusted() {
        // `--rebuild-index` drops chunks and keeps hashes. A stage that compared hashes alone
        // would decide there was nothing to do and leave the corpus unsearchable for good.
        let (store, node) = store_with_file("rebuilt.md");
        store.replace_chunks(node, &[chunk(0, "some content")]).unwrap();
        store
            .batch(|b| b.set_extraction_result(node, Some("deadbeef"), None, ExtractState::Done, None))
            .unwrap();
        store.requeue_extraction().unwrap();
        assert_eq!(store.pending_extraction(10).unwrap()[0].chunk_count, 1);

        store.delete_chunks(node).unwrap();
        let pending = store.pending_extraction(10).unwrap();
        assert_eq!(pending[0].content_hash.as_deref(), Some("deadbeef"));
        assert_eq!(pending[0].chunk_count, 0, "the caller must be able to see this");
    }

    #[test]
    fn asking_for_specific_nodes_returns_only_those_still_pending() {
        let (store, node) = store_with_file("urgent.md");
        let missing = NodeId(9999);

        let pending = store.pending_by_ids(&[node, missing]).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, node);

        store.set_extract_state(node, ExtractState::Done, None).unwrap();
        assert!(
            store.pending_by_ids(&[node]).unwrap().is_empty(),
            "an id that is no longer pending must drop out rather than be re-extracted"
        );
        assert!(store.pending_by_ids(&[]).unwrap().is_empty());
    }

    #[test]
    fn requeueing_brings_skipped_files_back() {
        // Adding a content root changes the decision, not the file, so nothing else would ever
        // retry it.
        let (store, node) = store_with_file("later.md");
        store
            .set_extract_state(node, ExtractState::Skipped, Some("outside the content roots"))
            .unwrap();
        assert!(store.pending_extraction(10).unwrap().is_empty());

        assert_eq!(store.requeue_extraction().unwrap(), 1);
        assert_eq!(store.pending_extraction(10).unwrap().len(), 1);
    }

    #[test]
    fn a_changed_file_becomes_pending_again_but_a_touched_one_does_not() {
        let (store, node) = store_with_file("edited.md");
        store.set_extract_state(node, ExtractState::Done, None).unwrap();

        let mut refresh = NodeUpsert {
            kind: NodeKind::File,
            path: PathBuf::from("/tmp/corpus/edited.md"),
            parent_id: None,
            mime: None,
            size: 10,
            mtime: 100,
            ctime: 100,
            inode: 1,
            dev: 1,
            zone_id: None,
        };
        store.upsert_node(&refresh, 2000).unwrap();
        assert!(
            store.pending_extraction(10).unwrap().is_empty(),
            "re-seeing an unchanged file must not queue it again"
        );

        refresh.size = 4096;
        store.upsert_node(&refresh, 3000).unwrap();
        let pending = store.pending_extraction(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, node);
    }

    #[test]
    fn chunk_ids_resolve_to_locations_and_missing_ones_drop_out() {
        let (store, node) = store_with_file("located.md");
        store
            .replace_chunks(node, &[chunk(0, "erster Abschnitt"), chunk(1, "zweiter Abschnitt")])
            .unwrap();
        let ids: Vec<ChunkId> = store
            .chunks_for_node(node)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();

        let mut located = store
            .chunk_locations(&[ids[0], ids[1], ChunkId(9999)])
            .unwrap();
        located.sort_by_key(|c| c.ordinal);
        assert_eq!(located.len(), 2, "an id the vector index still holds must simply drop out");
        assert_eq!(located[0].node_id, node);
        assert!(located[0].path.ends_with("located.md"));
        assert!(!located[0].hash.is_empty());
    }

    #[test]
    fn clearing_derived_data_keeps_episodes_and_vectors() {
        // Rule R10 in one test: what survives a rebuild is the append-only log and the vectors
        // that are keyed by content. Everything else comes back from the filesystem.
        let (store, node) = store_with_file("wird-neu-gebaut.md");
        store.replace_chunks(node, &[chunk(0, "inhalt der wiederkommt")]).unwrap();
        store
            .append_episode(
                1,
                &cortex_core::model::Actor::Human,
                cortex_core::model::Action::Open,
                Some(node),
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        let stored = store.chunks_for_node(node).unwrap();
        store
            .upsert_embeddings(
                crate::embeddings::ModelRef { id: "m", version: "1", dim: 2 },
                &[crate::embeddings::EmbeddingWrite {
                    chunk_id: stored[0].id,
                    chunk_hash: stored[0].hash.clone(),
                    vector: vec![1.0, 0.0],
                }],
                1,
            )
            .unwrap();

        store.clear_derived(true).unwrap();

        assert_eq!(store.stats().unwrap().files, 0, "nodes are derived and must go");
        assert_eq!(store.stats().unwrap().chunks, 0);
        assert!(
            store.search_chunks("\"wiederkommt\"", 10).unwrap().is_empty(),
            "the lexical index is contentless and does not cascade — it must be cleared explicitly"
        );
        assert_eq!(store.recent_episodes(10).unwrap().len(), 1, "episodes are never derived");
        assert_eq!(
            store
                .embedding_count(crate::embeddings::ModelRef { id: "m", version: "1", dim: 2 })
                .unwrap(),
            1,
            "vectors are keyed by content and must survive a rebuild"
        );
    }

    #[test]
    fn clearing_can_also_drop_the_vectors_when_asked() {
        let (store, node) = store_with_file("ganz-weg.md");
        store.replace_chunks(node, &[chunk(0, "inhalt")]).unwrap();
        let stored = store.chunks_for_node(node).unwrap();
        let model = crate::embeddings::ModelRef { id: "m", version: "1", dim: 2 };
        store
            .upsert_embeddings(
                model,
                &[crate::embeddings::EmbeddingWrite {
                    chunk_id: stored[0].id,
                    chunk_hash: stored[0].hash.clone(),
                    vector: vec![1.0, 0.0],
                }],
                1,
            )
            .unwrap();

        store.clear_derived(false).unwrap();
        assert_eq!(store.embedding_count(model).unwrap(), 0);
    }

    #[test]
    fn the_canonical_dump_ignores_everything_a_rebuild_is_allowed_to_change() {
        // Ids and "when did we first see this" differ after a rebuild by construction. If the
        // dump included them, the rule R10 test could never pass and would be quietly dropped.
        let store = Store::open_in_memory().unwrap();
        let insert = |path: &str, seen: i64| {
            store
                .upsert_node(
                    &NodeUpsert {
                        kind: NodeKind::File,
                        path: PathBuf::from(path),
                        parent_id: None,
                        mime: None,
                        size: 42,
                        mtime: 7,
                        ctime: 7,
                        inode: 1,
                        dev: 1,
                        zone_id: None,
                    },
                    seen,
                )
                .unwrap()
        };

        let first = insert("/tmp/corpus/b.md", 1000);
        insert("/tmp/corpus/a.md", 2000);
        store.replace_chunks(first, &[chunk(0, "eins"), chunk(1, "zwei")]).unwrap();
        let before = store.canonical_dump().unwrap();

        // Same corpus, different ids, different discovery times, different insertion order.
        store.clear_derived(true).unwrap();
        insert("/tmp/corpus/a.md", 9000);
        let rebuilt = insert("/tmp/corpus/b.md", 9000);
        store.replace_chunks(rebuilt, &[chunk(0, "eins"), chunk(1, "zwei")]).unwrap();

        assert_eq!(before, store.canonical_dump().unwrap());
        assert!(before.starts_with("/tmp/corpus/a.md"), "the dump must be sorted by path");
    }

    #[test]
    fn the_canonical_dump_notices_a_change_in_chunk_boundaries() {
        // The point of putting chunk hashes in it: a rebuild that produced different boundaries
        // would otherwise pass a comparison of paths and stats.
        let (store, node) = store_with_file("grenzen.md");
        store.replace_chunks(node, &[chunk(0, "eins"), chunk(1, "zwei")]).unwrap();
        let before = store.canonical_dump().unwrap();

        store.replace_chunks(node, &[chunk(0, "eins zwei")]).unwrap();
        assert_ne!(before, store.canonical_dump().unwrap());
    }

    #[test]
    fn a_malformed_query_is_an_error_rather_than_a_panic() {
        // The daemon builds match expressions with `cortex_query::lexical`, but the store is
        // still the last line of defence against a caller that does not.
        let (store, _) = store_with_file("x.md");
        assert!(store.search_chunks("\"unbalanced", 10).is_err());
    }
}
