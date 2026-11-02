//! Vectors, and the bookkeeping that makes changing the model a background job.
//!
//! Every embedding is stored against the model that produced it — id *and* version. That is
//! what turns "we changed the embedding model" from a rebuild into a migration: the old vectors
//! keep answering queries while the new ones are computed, and the switch happens when the new
//! set is complete. A table keyed only by chunk id would force a corpus-wide outage instead.
//!
//! ## Keyed by the text, not by the row
//!
//! The primary key is the chunk *hash* (migration 0003). An embedding is a pure function of the
//! text and the model, so tying it to a row id meant losing every vector whenever the rows were
//! re-derived — which is exactly what `--rebuild-index` does, and it turned a supported
//! operation into hours of inference nobody would spend. It also meant embedding the same
//! boilerplate once per file that contained it.
//!
//! ## What is stored, and what is not
//!
//! The vector here is `f32`, full precision, and it is the authoritative copy. The usearch index
//! beside it is `f16` and is *derived* — rebuilding it must never need the model again, which is
//! an explicit Definition-of-Done item for M2, and it can only hold if the precision that was
//! quantised away is still on disk somewhere.
//!
//! Chunk text is still not stored (D16). Embedding a chunk therefore needs either the text in
//! hand — which the extraction stage has, so it embeds as it goes — or a re-read of the file
//! through the stored offsets, which is what the backfill path does when a model arrives after
//! the content was already indexed.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::{ChunkId, NodeId};
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;

/// Which model an operation is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRef<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub dim: usize,
}

/// One vector on its way into the store.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingWrite {
    /// Which chunk this came from. Not the key — it is what the vector index needs in order to
    /// point back at a place in a file.
    pub chunk_id: ChunkId,
    /// The hash of the text that was embedded. This *is* the key.
    pub chunk_hash: String,
    pub vector: Vec<f32>,
}

/// A chunk that still needs a vector, with everything needed to read its text back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkToEmbed {
    pub chunk_id: ChunkId,
    pub node_id: NodeId,
    pub path: PathBuf,
    pub byte_start: i64,
    pub byte_end: i64,
    /// What the chunk hashed to when it was indexed. The re-read is refused if it disagrees,
    /// because embedding text the index does not describe would be worse than embedding nothing.
    pub hash: String,
}

/// A memory chunk waiting for a vector. Carries its text, because there is no file to re-read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryChunkToEmbed {
    pub chunk_id: ChunkId,
    pub node_id: NodeId,
    pub text: String,
    pub hash: String,
}

/// Encode a vector for storage. Little-endian `f32`, no header.
///
/// The dimension is a column, so the blob needs no length prefix; a mismatch between the two is
/// corruption and is detected on read rather than silently reinterpreted.
pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub fn decode_vector(blob: &[u8], dim: usize) -> Result<Vec<f32>> {
    if blob.len() != dim * 4 {
        return Err(StoreError::corrupt(format!(
            "embedding blob is {} bytes, expected {} for {dim} dimensions",
            blob.len(),
            dim * 4
        )));
    }
    Ok(blob
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

impl crate::queries::Batch<'_> {
    /// Write vectors for chunks, replacing any the same model already produced.
    pub fn upsert_embeddings(
        &self,
        model: ModelRef<'_>,
        rows: &[EmbeddingWrite],
        now: i64,
    ) -> Result<usize> {
        let mut written = 0;
        for row in rows {
            if row.vector.len() != model.dim {
                return Err(StoreError::corrupt(format!(
                    "model {} produced {} dimensions, expected {}",
                    model.id,
                    row.vector.len(),
                    model.dim
                )));
            }
            self.tx().execute(
                "INSERT INTO embeddings (chunk_hash, model_id, model_version, dim, vector, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(chunk_hash, model_id, model_version) DO UPDATE SET
                     dim = excluded.dim,
                     vector = excluded.vector,
                     created_at = excluded.created_at",
                params![
                    row.chunk_hash,
                    model.id,
                    model.version,
                    model.dim as i64,
                    encode_vector(&row.vector),
                    now
                ],
            )?;
            written += 1;
        }
        Ok(written)
    }
}

impl Store {
    pub fn upsert_embeddings(
        &self,
        model: ModelRef<'_>,
        rows: &[EmbeddingWrite],
        now: i64,
    ) -> Result<usize> {
        self.batch(|b| b.upsert_embeddings(model, rows, now))
    }

    /// Chunks that were deleted and whose vectors may still be in the ANN index, each with
    /// whether a **live** chunk now carries the same id.
    ///
    /// The flag exists because SQLite hands a deleted row id straight back to the next insert,
    /// so the id noted here can already belong to the replacement chunk — whose vector is the
    /// right one and must stay. The first version of the stage's cleanup removed it, and the
    /// test that re-extracts a file caught it on the first run.
    pub fn dead_chunks(&self, limit: usize) -> Result<Vec<(ChunkId, bool)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT d.chunk_id, c.id IS NOT NULL
             FROM dead_chunks d LEFT JOIN chunks c ON c.id = d.chunk_id
             LIMIT ?1",
        )?;
        let ids = stmt
            .query_map([limit as i64], |r| Ok((ChunkId(r.get(0)?), r.get::<_, bool>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// The vectors for these are gone from the index; forget the note.
    pub fn forget_dead_chunks(&self, chunks: &[ChunkId]) -> Result<()> {
        let conn = self.conn();
        for chunk in chunks {
            conn.execute("DELETE FROM dead_chunks WHERE chunk_id = ?1", [chunk.0])?;
        }
        Ok(())
    }

    pub fn embedding_count(&self, model: ModelRef<'_>) -> Result<i64> {
        let conn = self.conn();
        Ok(conn.query_row(
            "SELECT count(*) FROM embeddings WHERE model_id = ?1 AND model_version = ?2",
            params![model.id, model.version],
            |r| r.get(0),
        )?)
    }

    /// Chunks this model has not embedded yet.
    ///
    /// Ordered by chunk id rather than by anything cleverer, for the same reason the extraction
    /// queue is: a sort over the outstanding set costs more than the work it schedules, and it
    /// grows with the backlog.
    pub fn chunks_without_embedding(
        &self,
        model: ModelRef<'_>,
        limit: usize,
    ) -> Result<Vec<ChunkToEmbed>> {
        self.chunks_without_embedding_after(model, 0, limit)
    }

    /// The same queue, starting after chunk id `after` (D131).
    ///
    /// The embedding stage walks the queue with this cursor instead of asking for the head again
    /// and again. A chunk it cannot read back — its file moved on, or its bytes never round-trip —
    /// stays in the queue until extraction replaces it, and at the head it blocked everything behind
    /// it: on the owner's machine 64 such chunks of one binary file meant zero vectors written, and
    /// each re-claim of the head was a 3-second scan. With the cursor, each sweep scans the table once.
    pub fn chunks_without_embedding_after(
        &self,
        model: ModelRef<'_>,
        after: i64,
        limit: usize,
    ) -> Result<Vec<ChunkToEmbed>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.node_id, n.path, c.byte_start, c.byte_end, c.hash
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             LEFT JOIN embeddings e
                 ON e.chunk_hash = c.hash AND e.model_id = ?1 AND e.model_version = ?2
             WHERE e.chunk_hash IS NULL AND n.path IS NOT NULL AND c.id > ?4
             ORDER BY c.id
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![model.id, model.version, limit as i64, after], |r| {
                Ok(ChunkToEmbed {
                    chunk_id: ChunkId(r.get(0)?),
                    node_id: NodeId(r.get(1)?),
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    byte_start: r.get(3)?,
                    byte_end: r.get(4)?,
                    hash: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// How many chunks have a vector this model produced.
    ///
    /// The number the ANN index must contain. Compared against it to notice when the two have
    /// drifted apart — which happens whenever chunks are re-derived, because the index keys by
    /// chunk id and the vectors key by content.
    pub fn indexable_vector_count(&self, model: ModelRef<'_>) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            // `n.path IS NOT NULL` is what keeps memory chunks out of the file collection's
            // count. Without it, the moment a memory is embedded this number exceeds what the
            // file index holds, the reconcile concludes the index has drifted, and it rebuilds
            // the whole thing on every pass for ever.
            "SELECT count(*) FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN embeddings e ON e.chunk_hash = c.hash
             WHERE e.model_id = ?1 AND e.model_version = ?2 AND n.path IS NOT NULL",
            params![model.id, model.version],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// Every chunk that has a vector, with the vector, handed to `f` one at a time.
    ///
    /// Joined back to `chunks` rather than read straight from `embeddings`, because the vector
    /// index points at chunks: one vector can serve several identical chunks, and every one of
    /// them has to be findable.
    pub fn for_each_indexable_vector(
        &self,
        model: ModelRef<'_>,
        mut f: impl FnMut(ChunkId, &[f32]),
    ) -> Result<usize> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, e.dim, e.vector
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             JOIN embeddings e ON e.chunk_hash = c.hash
             WHERE e.model_id = ?1 AND e.model_version = ?2 AND n.path IS NOT NULL
             ORDER BY c.id",
        )?;
        let mut rows = stmt.query(params![model.id, model.version])?;

        let mut count = 0;
        while let Some(row) = rows.next()? {
            let chunk = ChunkId(row.get(0)?);
            let dim: i64 = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            let vector = decode_vector(&blob, dim as usize)?;
            f(chunk, &vector);
            count += 1;
        }
        Ok(count)
    }

    /// The memory chunks this model has not embedded yet.
    ///
    /// A separate queue from [`Store::chunks_without_embedding`] rather than a flag on it,
    /// because the text comes from a different place: a file chunk is re-read from the file and
    /// its hash verified, and a memory has no file to re-read. The text is in the row.
    pub fn memory_chunks_without_embedding(
        &self,
        model: ModelRef<'_>,
        limit: usize,
    ) -> Result<Vec<MemoryChunkToEmbed>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, m.node_id, m.text, c.hash
             FROM chunks c
             JOIN memories m ON m.node_id = c.node_id
             LEFT JOIN embeddings e
                 ON e.chunk_hash = c.hash AND e.model_id = ?1 AND e.model_version = ?2
             WHERE e.chunk_hash IS NULL
             ORDER BY c.id
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![model.id, model.version, limit as i64], |r| {
                Ok(MemoryChunkToEmbed {
                    chunk_id: ChunkId(r.get(0)?),
                    node_id: NodeId(r.get(1)?),
                    text: r.get(2)?,
                    hash: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// How many memory chunks have a vector this model produced.
    pub fn indexable_memory_vector_count(&self, model: ModelRef<'_>) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM chunks c
             JOIN memories m ON m.node_id = c.node_id
             JOIN embeddings e ON e.chunk_hash = c.hash
             WHERE e.model_id = ?1 AND e.model_version = ?2",
            params![model.id, model.version],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// Every memory vector, streamed. Rebuilds the memory collection without the model.
    pub fn for_each_indexable_memory_vector(
        &self,
        model: ModelRef<'_>,
        mut f: impl FnMut(ChunkId, &[f32]),
    ) -> Result<usize> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, e.dim, e.vector
             FROM chunks c
             JOIN memories m ON m.node_id = c.node_id
             JOIN embeddings e ON e.chunk_hash = c.hash
             WHERE e.model_id = ?1 AND e.model_version = ?2
             ORDER BY c.id",
        )?;
        let mut rows = stmt.query(params![model.id, model.version])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            let chunk = ChunkId(row.get(0)?);
            let dim: i64 = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            let vector = decode_vector(&blob, dim as usize)?;
            f(chunk, &vector);
            count += 1;
        }
        Ok(count)
    }

    /// The vectors for a specific set of chunks, for callers that already have candidates.
    ///
    /// Used by the diversification step, which needs the vectors of the twenty things it is
    /// choosing between and not of the hundred thousand it is not.
    pub fn embeddings_for_chunks(
        &self,
        model: ModelRef<'_>,
        chunks: &[ChunkId],
    ) -> Result<std::collections::HashMap<ChunkId, Vec<f32>>> {
        let mut out = std::collections::HashMap::with_capacity(chunks.len());
        if chunks.is_empty() {
            return Ok(out);
        }
        let conn = self.conn();
        // One statement, re-used. A single `IN (...)` would need the placeholder count baked
        // into the SQL and re-prepared for every distinct candidate count, which is a prepared
        // statement cache that never hits.
        let mut stmt = conn.prepare(
            "SELECT e.dim, e.vector FROM chunks c
             JOIN embeddings e ON e.chunk_hash = c.hash
             WHERE c.id = ?1 AND e.model_id = ?2 AND e.model_version = ?3",
        )?;
        for chunk in chunks {
            let row: Option<(i64, Vec<u8>)> = stmt
                .query_row(params![chunk.0, model.id, model.version], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            if let Some((dim, blob)) = row {
                out.insert(*chunk, decode_vector(&blob, dim as usize)?);
            }
        }
        Ok(out)
    }

    /// Every vector this model produced, handed to `f` one at a time.
    ///
    /// Streamed rather than returned: a hundred thousand chunks at 384 dimensions is 150 MB of
    /// `f32`, and materialising that to hand it to a consumer that writes it straight into an
    /// index would double the peak for no reason.
    pub fn for_each_embedding(
        &self,
        model: ModelRef<'_>,
        mut f: impl FnMut(&str, &[f32]),
    ) -> Result<usize> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT chunk_hash, dim, vector FROM embeddings
             WHERE model_id = ?1 AND model_version = ?2
             ORDER BY chunk_hash",
        )?;
        let mut rows = stmt.query(params![model.id, model.version])?;

        let mut count = 0;
        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            let dim: i64 = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            let vector = decode_vector(&blob, dim as usize)?;
            f(&hash, &vector);
            count += 1;
        }
        Ok(count)
    }

    /// Drop everything a model produced. For a model that has been replaced, or a bad run.
    pub fn delete_embeddings(&self, model: ModelRef<'_>) -> Result<usize> {
        let conn = self.conn();
        Ok(conn.execute(
            "DELETE FROM embeddings WHERE model_id = ?1 AND model_version = ?2",
            params![model.id, model.version],
        )?)
    }

    /// The models that have vectors in this database, with their counts.
    pub fn embedding_models(&self) -> Result<Vec<(String, String, i64, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT model_id, model_version, max(dim), count(*)
             FROM embeddings GROUP BY model_id, model_version ORDER BY count(*) DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::ChunkWrite;
    use crate::queries::NodeUpsert;
    use cortex_core::model::NodeKind;

    const MODEL: ModelRef<'static> = ModelRef {
        id: "test-model",
        version: "1",
        dim: 4,
    };

    fn store_with_chunks(count: usize) -> (Store, Vec<crate::chunks::StoredChunk>) {
        let store = Store::open_in_memory().unwrap();
        let node = store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::File,
                    path: PathBuf::from("/tmp/corpus/notes.md"),
                    parent_id: None,
                    mime: None,
                    size: 10,
                    mtime: 1,
                    ctime: 1,
                    inode: 1,
                    dev: 1,
                    zone_id: None,
                },
                1000,
            )
            .unwrap();

        let writes: Vec<ChunkWrite> = (0..count)
            .map(|i| ChunkWrite {
                ordinal: i as i64,
                byte_start: i as i64 * 100,
                byte_end: i as i64 * 100 + 50,
                hash: format!("hash-{i}"),
                preview: Some(format!("chunk {i}")),
                text: format!("chunk number {i}"),
            })
            .collect();
        store.replace_chunks(node, &writes).unwrap();

        let chunks = store.chunks_for_node(node).unwrap();
        (store, chunks)
    }

    fn write(chunk: &crate::chunks::StoredChunk, value: f32) -> EmbeddingWrite {
        EmbeddingWrite {
            chunk_id: chunk.id,
            chunk_hash: chunk.hash.clone(),
            vector: vec![value, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn a_vector_survives_the_round_trip_exactly() {
        // f32 in, f32 out, bit for bit. Anything lossy here would make the usearch index
        // impossible to rebuild identically, which is a Definition-of-Done item.
        let vector = vec![0.5, -0.25, 1.0e-7, f32::MIN_POSITIVE];
        let blob = encode_vector(&vector);
        assert_eq!(decode_vector(&blob, 4).unwrap(), vector);
    }

    #[test]
    fn a_blob_of_the_wrong_length_is_corruption_rather_than_a_reinterpretation() {
        let blob = encode_vector(&[1.0, 2.0, 3.0]);
        assert!(decode_vector(&blob, 4).is_err());
        assert!(decode_vector(&blob, 2).is_err());
    }

    #[test]
    fn embeddings_are_written_read_back_and_counted() {
        let (store, chunks) = store_with_chunks(3);
        let rows: Vec<EmbeddingWrite> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| write(chunk, i as f32))
            .collect();
        assert_eq!(store.upsert_embeddings(MODEL, &rows, 1).unwrap(), 3);
        assert_eq!(store.embedding_count(MODEL).unwrap(), 3);

        let mut seen = Vec::new();
        store
            .for_each_indexable_vector(MODEL, |id, v| seen.push((id, v.to_vec())))
            .unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].1, vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn the_queue_can_be_walked_past_a_head_that_never_clears() {
        // D131: the stage reads on from where its last claim ended, so chunks it cannot embed
        // (stale, unreadable) no longer block everything behind them.
        let (store, chunks) = store_with_chunks(5);
        let head = store.chunks_without_embedding(MODEL, 2).unwrap();
        assert_eq!(head.len(), 2);
        let last = head.last().unwrap().chunk_id.0;

        let next = store.chunks_without_embedding_after(MODEL, last, 10).unwrap();
        assert_eq!(next.len(), 3);
        assert!(next.iter().all(|c| c.chunk_id.0 > last));
        assert_eq!(next[0].hash, chunks[2].hash);

        let end = next.last().unwrap().chunk_id.0;
        assert!(store.chunks_without_embedding_after(MODEL, end, 10).unwrap().is_empty());
    }

    #[test]
    fn a_second_run_of_the_same_model_replaces_rather_than_duplicates() {
        let (store, chunks) = store_with_chunks(1);
        store
            .upsert_embeddings(MODEL, &[write(&chunks[0], 1.0)], 1)
            .unwrap();
        store
            .upsert_embeddings(MODEL, &[write(&chunks[0], 2.0)], 2)
            .unwrap();

        assert_eq!(store.embedding_count(MODEL).unwrap(), 1);
        let mut seen = Vec::new();
        store
            .for_each_embedding(MODEL, |_, v| seen.push(v[0]))
            .unwrap();
        assert_eq!(seen, vec![2.0]);
    }

    #[test]
    fn two_models_coexist_so_a_model_change_is_a_migration_not_an_outage() {
        // The property that makes changing the embedding model survivable: the old vectors keep
        // answering while the new ones are computed.
        let (store, chunks) = store_with_chunks(2);
        let other = ModelRef { id: "next-model", version: "1", dim: 4 };
        let rows: Vec<EmbeddingWrite> = chunks.iter().map(|c| write(c, 1.0)).collect();

        store.upsert_embeddings(MODEL, &rows, 1).unwrap();
        store.upsert_embeddings(other, &rows[..1], 1).unwrap();

        assert_eq!(store.embedding_count(MODEL).unwrap(), 2);
        assert_eq!(store.embedding_count(other).unwrap(), 1);
        assert_eq!(store.chunks_without_embedding(other, 10).unwrap().len(), 1);
        assert!(store.chunks_without_embedding(MODEL, 10).unwrap().is_empty());

        assert_eq!(store.delete_embeddings(other).unwrap(), 1);
        assert_eq!(store.embedding_count(MODEL).unwrap(), 2, "the other model is untouched");
    }

    #[test]
    fn a_vector_of_the_wrong_dimension_is_refused_at_the_boundary() {
        // A model that quietly returns the wrong shape would otherwise fill the index with
        // vectors that only fail much later, inside the ANN library.
        let (store, chunks) = store_with_chunks(1);
        let bad = EmbeddingWrite {
            chunk_id: chunks[0].id,
            chunk_hash: chunks[0].hash.clone(),
            vector: vec![1.0, 2.0],
        };
        assert!(store.upsert_embeddings(MODEL, &bad_rows(&bad), 1).is_err());
        assert_eq!(store.embedding_count(MODEL).unwrap(), 0);
    }

    fn bad_rows(row: &EmbeddingWrite) -> Vec<EmbeddingWrite> {
        vec![row.clone()]
    }

    #[test]
    fn chunks_needing_work_carry_what_it_takes_to_read_them_back() {
        let (store, chunks) = store_with_chunks(2);
        let pending = store.chunks_without_embedding(MODEL, 10).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].chunk_id, chunks[0].id);
        assert_eq!(pending[0].path, PathBuf::from("/tmp/corpus/notes.md"));
        assert_eq!(pending[0].hash, "hash-0");
        assert_eq!(pending[0].byte_end - pending[0].byte_start, 50);
    }

    #[test]
    fn deleting_a_chunk_leaves_its_vector_for_next_time() {
        let (store, chunks) = store_with_chunks(1);
        store
            .upsert_embeddings(MODEL, &[write(&chunks[0], 1.0)], 1)
            .unwrap();
        let node = store
            .node_by_path(&PathBuf::from("/tmp/corpus/notes.md"))
            .unwrap()
            .unwrap();
        store.delete_chunks(node.id).unwrap();

        // The vector *survives* the chunk now, on purpose: it describes the text, not the row.
        // Re-chunking the same file recovers it without touching the model, which is what makes
        // `--rebuild-index` affordable (migration 0003).
        assert_eq!(store.embedding_count(MODEL).unwrap(), 1);
        assert_eq!(
            store.for_each_indexable_vector(MODEL, |_, _| {}).unwrap(),
            0,
            "but nothing points at it any more, so nothing can be found through it"
        );
    }

    #[test]
    fn identical_text_in_two_files_is_embedded_once() {
        // Boilerplate — a licence header, a shared footer, an exported template — is the same
        // chunk in many files. Keying by hash means the model runs once for all of them, and it
        // is the same property that lets a rebuild keep its vectors.
        let store = Store::open_in_memory().unwrap();
        let mut ids = Vec::new();
        for name in ["a.md", "b.md"] {
            let node = store
                .upsert_node(
                    &NodeUpsert {
                        kind: NodeKind::File,
                        path: PathBuf::from("/tmp/corpus").join(name),
                        parent_id: None,
                        mime: None,
                        size: 10,
                        mtime: 1,
                        ctime: 1,
                        inode: 1,
                        dev: 1,
                        zone_id: None,
                    },
                    1000,
                )
                .unwrap();
            store
                .replace_chunks(
                    node,
                    &[ChunkWrite {
                        ordinal: 0,
                        byte_start: 0,
                        byte_end: 20,
                        hash: "shared-boilerplate".into(),
                        preview: None,
                        text: "the same paragraph".into(),
                    }],
                )
                .unwrap();
            ids.push(store.chunks_for_node(node).unwrap().remove(0));
        }

        assert_eq!(
            store.chunks_without_embedding(MODEL, 10).unwrap().len(),
            2,
            "both chunks start out needing a vector"
        );

        store
            .upsert_embeddings(MODEL, &[write(&ids[0], 1.0)], 1)
            .unwrap();

        assert_eq!(store.embedding_count(MODEL).unwrap(), 1);
        assert!(
            store.chunks_without_embedding(MODEL, 10).unwrap().is_empty(),
            "the second chunk must be covered by the first one's vector"
        );
        assert_eq!(
            store.for_each_indexable_vector(MODEL, |_, _| {}).unwrap(),
            2,
            "and both must still be findable through the index"
        );
    }

    #[test]
    fn re_chunking_a_file_recovers_its_vectors_without_the_model() {
        // Definition of Done §7: `--rebuild-index` re-derives every chunk from the filesystem.
        // With row-keyed embeddings that meant hours of inference; keyed by hash it means none.
        let (store, chunks) = store_with_chunks(2);
        let rows: Vec<EmbeddingWrite> = chunks.iter().map(|c| write(c, 1.0)).collect();
        store.upsert_embeddings(MODEL, &rows, 1).unwrap();

        let node = store
            .node_by_path(&PathBuf::from("/tmp/corpus/notes.md"))
            .unwrap()
            .unwrap();
        let same_again: Vec<ChunkWrite> = chunks
            .iter()
            .map(|c| ChunkWrite {
                ordinal: c.ordinal,
                byte_start: c.byte_start,
                byte_end: c.byte_end,
                hash: c.hash.clone(),
                preview: c.preview.clone(),
                text: format!("text of {}", c.ordinal),
            })
            .collect();

        store.delete_chunks(node.id).unwrap();
        store.replace_chunks(node.id, &same_again).unwrap();

        assert!(
            store.chunks_without_embedding(MODEL, 10).unwrap().is_empty(),
            "a rebuild must not ask the model for anything it has already computed"
        );
    }

    #[test]
    fn the_models_present_can_be_listed() {
        let (store, chunks) = store_with_chunks(1);
        store
            .upsert_embeddings(MODEL, &[write(&chunks[0], 1.0)], 1)
            .unwrap();
        let models = store.embedding_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].0, "test-model");
        assert_eq!(models[0].3, 1);
    }
}
