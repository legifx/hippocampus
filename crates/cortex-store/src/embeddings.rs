//! Vectors, and the bookkeeping that makes changing the model a background job.
//!
//! Every embedding is stored against the model that produced it — id *and* version. That is
//! what turns "we changed the embedding model" from a rebuild into a migration: the old vectors
//! keep answering queries while the new ones are computed, and the switch happens when the new
//! set is complete. A table keyed only by chunk id would force a corpus-wide outage instead.
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
use rusqlite::params;
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
    pub chunk_id: ChunkId,
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
                "INSERT INTO embeddings (chunk_id, model_id, model_version, dim, vector, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(chunk_id, model_id, model_version) DO UPDATE SET
                     dim = excluded.dim,
                     vector = excluded.vector,
                     created_at = excluded.created_at",
                params![
                    row.chunk_id.0,
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
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.node_id, n.path, c.byte_start, c.byte_end, c.hash
             FROM chunks c
             JOIN nodes n ON n.id = c.node_id
             LEFT JOIN embeddings e
                 ON e.chunk_id = c.id AND e.model_id = ?1 AND e.model_version = ?2
             WHERE e.chunk_id IS NULL AND n.path IS NOT NULL
             ORDER BY c.id
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![model.id, model.version, limit as i64], |r| {
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

    /// Every vector this model produced, handed to `f` one at a time.
    ///
    /// Streamed rather than returned: a hundred thousand chunks at 384 dimensions is 150 MB of
    /// `f32`, and materialising that to hand it to a consumer that writes it straight into an
    /// index would double the peak for no reason.
    pub fn for_each_embedding(
        &self,
        model: ModelRef<'_>,
        mut f: impl FnMut(ChunkId, &[f32]),
    ) -> Result<usize> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT chunk_id, dim, vector FROM embeddings
             WHERE model_id = ?1 AND model_version = ?2
             ORDER BY chunk_id",
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

    fn store_with_chunks(count: usize) -> (Store, Vec<ChunkId>) {
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

        let ids = store
            .chunks_for_node(node)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        (store, ids)
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
            .map(|(i, id)| EmbeddingWrite {
                chunk_id: *id,
                vector: vec![i as f32, 0.0, 0.0, 1.0],
            })
            .collect();
        assert_eq!(store.upsert_embeddings(MODEL, &rows, 1).unwrap(), 3);
        assert_eq!(store.embedding_count(MODEL).unwrap(), 3);

        let mut seen = Vec::new();
        store
            .for_each_embedding(MODEL, |id, v| seen.push((id, v.to_vec())))
            .unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].1, vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_second_run_of_the_same_model_replaces_rather_than_duplicates() {
        let (store, chunks) = store_with_chunks(1);
        let write = |value: f32| EmbeddingWrite {
            chunk_id: chunks[0],
            vector: vec![value, 0.0, 0.0, 0.0],
        };
        store.upsert_embeddings(MODEL, &[write(1.0)], 1).unwrap();
        store.upsert_embeddings(MODEL, &[write(2.0)], 2).unwrap();

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
        let rows: Vec<EmbeddingWrite> = chunks
            .iter()
            .map(|id| EmbeddingWrite { chunk_id: *id, vector: vec![1.0, 0.0, 0.0, 0.0] })
            .collect();

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
            chunk_id: chunks[0],
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
        assert_eq!(pending[0].chunk_id, chunks[0]);
        assert_eq!(pending[0].path, PathBuf::from("/tmp/corpus/notes.md"));
        assert_eq!(pending[0].hash, "hash-0");
        assert_eq!(pending[0].byte_end - pending[0].byte_start, 50);
    }

    #[test]
    fn deleting_a_chunk_takes_its_vector_with_it() {
        // Cascade, not cleanup: an embedding for a chunk that no longer exists would be a
        // result pointing at nothing.
        let (store, chunks) = store_with_chunks(1);
        store
            .upsert_embeddings(
                MODEL,
                &[EmbeddingWrite { chunk_id: chunks[0], vector: vec![1.0, 0.0, 0.0, 0.0] }],
                1,
            )
            .unwrap();
        let node = store
            .node_by_path(&PathBuf::from("/tmp/corpus/notes.md"))
            .unwrap()
            .unwrap();
        store.delete_chunks(node.id).unwrap();
        assert_eq!(store.embedding_count(MODEL).unwrap(), 0);
    }

    #[test]
    fn the_models_present_can_be_listed() {
        let (store, chunks) = store_with_chunks(1);
        store
            .upsert_embeddings(
                MODEL,
                &[EmbeddingWrite { chunk_id: chunks[0], vector: vec![1.0, 0.0, 0.0, 0.0] }],
                1,
            )
            .unwrap();
        let models = store.embedding_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].0, "test-model");
        assert_eq!(models[0].3, 1);
    }
}
