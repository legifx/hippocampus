//! The approximate-nearest-neighbour index.
//!
//! usearch holds an HNSW graph over `f16` vectors keyed by chunk id. It is **derived data and
//! nothing else**: every vector in it is also in the `embeddings` table at full precision, so
//! deleting the file and restarting rebuilds it without running the model — which is an
//! explicit Definition-of-Done item for M2 and the reason the table stores `f32`.
//!
//! ## Why `f16`
//!
//! Halving the index halves the page cache it occupies, and cosine similarity over unit vectors
//! does not notice: the quantisation error is far below the distance between a relevant result
//! and an irrelevant one. Precision that matters for anything else is still in the table.
//!
//! ## Why the whole thing is behind one lock
//!
//! usearch supports concurrent reads and writes, but the *capacity* is not dynamic — adding
//! beyond it has to grow the index first, and that must not race with a search. One lock, taken
//! for as short a time as possible, is honest and fast enough: a search is microseconds, and an
//! add is only ever done by the one thread that embeds.

use crate::error::{Result, StoreError};
use cortex_core::ids::ChunkId;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Smallest capacity worth reserving, so a fresh index does not grow on every one of its first
/// few hundred inserts.
const MIN_CAPACITY: usize = 1024;

/// One approximate match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorHit {
    pub chunk_id: ChunkId,
    /// Cosine similarity, higher is better.
    ///
    /// usearch reports a *distance* for the cosine metric; it is converted here, once, for the
    /// same reason BM25's sign is flipped in exactly one place.
    pub similarity: f32,
}

pub struct VectorIndex {
    index: RwLock<Index>,
    path: PathBuf,
    dim: usize,
}

impl VectorIndex {
    /// Open the index at `path`, or start an empty one.
    ///
    /// A file that does not match the expected dimensions is discarded rather than repaired: it
    /// belongs to a different model, and every vector it holds is in the table anyway.
    pub fn open(path: &Path, dim: usize) -> Result<Self> {
        let index = new_index(dim)?;

        if path.exists() {
            match index.load(&path.to_string_lossy()) {
                Ok(()) if index.dimensions() == dim => {
                    tracing::info!(
                        vectors = index.size(),
                        path = %path.display(),
                        "vector index loaded"
                    );
                }
                Ok(()) => {
                    tracing::warn!(
                        found = index.dimensions(),
                        expected = dim,
                        "vector index belongs to another model; rebuilding from the store"
                    );
                    return Ok(Self {
                        index: RwLock::new(new_index(dim)?),
                        path: path.to_path_buf(),
                        dim,
                    });
                }
                Err(e) => {
                    // A corrupt cache is not a corrupt database. Say so and start again.
                    tracing::warn!(error = %e, "vector index could not be read; rebuilding from the store");
                    return Ok(Self {
                        index: RwLock::new(new_index(dim)?),
                        path: path.to_path_buf(),
                        dim,
                    });
                }
            }
        }

        Ok(Self {
            index: RwLock::new(index),
            path: path.to_path_buf(),
            dim,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.index.read().unwrap().size()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate memory held by the graph, for the resource budget.
    pub fn memory_bytes(&self) -> usize {
        self.index.read().unwrap().memory_usage()
    }

    pub fn add(&self, chunk: ChunkId, vector: &[f32]) -> Result<()> {
        self.add_many(std::iter::once((chunk, vector)))
    }

    /// Add many vectors, growing capacity once rather than per insert.
    pub fn add_many<'a>(
        &self,
        vectors: impl IntoIterator<Item = (ChunkId, &'a [f32])>,
    ) -> Result<()> {
        let vectors: Vec<(ChunkId, &[f32])> = vectors.into_iter().collect();
        if vectors.is_empty() {
            return Ok(());
        }
        for (_, vector) in &vectors {
            if vector.len() != self.dim {
                return Err(StoreError::corrupt(format!(
                    "vector has {} dimensions, index has {}",
                    vector.len(),
                    self.dim
                )));
            }
        }

        let index = self.index.write().unwrap();
        let needed = index.size() + vectors.len();
        if needed > index.capacity() {
            // Doubling, so a first index run does not pay for a reallocation per batch.
            let target = (index.capacity() * 2).max(needed).max(MIN_CAPACITY);
            index.reserve(target).map_err(vector_error)?;
        }
        for (chunk, vector) in vectors {
            // usearch keys are u64 and chunk ids are positive by construction, so this is a
            // reinterpretation rather than a conversion that could collide.
            index.add(chunk.0 as u64, vector).map_err(vector_error)?;
        }
        Ok(())
    }

    /// Remove a chunk's vector. Absent keys are not an error — the caller is deleting.
    pub fn remove(&self, chunk: ChunkId) -> Result<()> {
        let index = self.index.write().unwrap();
        index.remove(chunk.0 as u64).map_err(vector_error)?;
        Ok(())
    }

    pub fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<VectorHit>> {
        if vector.len() != self.dim {
            return Err(StoreError::corrupt(format!(
                "query vector has {} dimensions, index has {}",
                vector.len(),
                self.dim
            )));
        }
        let index = self.index.read().unwrap();
        if index.size() == 0 {
            return Ok(Vec::new());
        }
        let matches = index.search(vector, limit).map_err(vector_error)?;
        Ok(matches
            .keys
            .into_iter()
            .zip(matches.distances)
            .map(|(key, distance)| VectorHit {
                chunk_id: ChunkId(key as i64),
                similarity: 1.0 - distance,
            })
            .collect())
    }

    /// Write the index to disk.
    ///
    /// Through a temporary file and a rename, because a half-written index that still looks
    /// loadable is worse than one that is obviously missing.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("usearch.tmp");
        {
            let index = self.index.read().unwrap();
            index
                .save(&temporary.to_string_lossy())
                .map_err(vector_error)?;
        }
        std::fs::rename(&temporary, &self.path)?;
        cortex_core::paths::restrict_file(&self.path)?;
        Ok(())
    }

    /// Throw everything away. Used before a rebuild.
    pub fn clear(&self) -> Result<()> {
        let mut index = self.index.write().unwrap();
        *index = new_index(self.dim)?;
        Ok(())
    }
}

fn new_index(dim: usize) -> Result<Index> {
    let options = IndexOptions {
        dimensions: dim,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        // Zero means "use the library's default", which is tuned for the recall/latency balance
        // this index wants. Choosing our own numbers without a goldset to measure against would
        // be superstition.
        connectivity: 0,
        expansion_add: 0,
        expansion_search: 0,
        multi: false,
    };
    Index::new(&options).map_err(vector_error)
}

fn vector_error(e: impl std::fmt::Display) -> StoreError {
    StoreError::corrupt(format!("vector index: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(values: [f32; 4]) -> Vec<f32> {
        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        values.iter().map(|v| v / norm).collect()
    }

    fn index() -> (tempfile::TempDir, VectorIndex) {
        let tmp = tempfile::tempdir().unwrap();
        let index = VectorIndex::open(&tmp.path().join("vectors.usearch"), 4).unwrap();
        (tmp, index)
    }

    #[test]
    fn the_nearest_vector_comes_back_first() {
        let (_tmp, index) = index();
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        index.add(ChunkId(2), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();
        index.add(ChunkId(3), &unit([0.9, 0.1, 0.0, 0.0])).unwrap();

        let hits = index.search(&unit([1.0, 0.05, 0.0, 0.0]), 3).unwrap();
        assert_eq!(hits[0].chunk_id, ChunkId(1));
        assert_eq!(hits[1].chunk_id, ChunkId(3));
        assert!(hits[0].similarity > hits[1].similarity);
        assert!(hits[2].similarity < hits[1].similarity);
    }

    #[test]
    fn similarity_is_reported_as_higher_is_better() {
        // usearch returns a distance. If this conversion is ever dropped, every vector ranking
        // inverts and nothing fails.
        let (_tmp, index) = index();
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        let hits = index.search(&unit([1.0, 0.0, 0.0, 0.0]), 1).unwrap();
        assert!(
            (hits[0].similarity - 1.0).abs() < 0.01,
            "an identical vector should have similarity 1, got {}",
            hits[0].similarity
        );
    }

    #[test]
    fn an_empty_index_answers_with_nothing_rather_than_failing() {
        let (_tmp, index) = index();
        assert!(index.search(&unit([1.0, 0.0, 0.0, 0.0]), 10).unwrap().is_empty());
        assert!(index.is_empty());
    }

    #[test]
    fn it_survives_a_save_and_a_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        {
            let index = VectorIndex::open(&path, 4).unwrap();
            index.add(ChunkId(7), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
            index.save().unwrap();
        }

        let reopened = VectorIndex::open(&path, 4).unwrap();
        assert_eq!(reopened.len(), 1);
        let hits = reopened.search(&unit([1.0, 0.0, 0.0, 0.0]), 1).unwrap();
        assert_eq!(hits[0].chunk_id, ChunkId(7));
    }

    #[test]
    fn an_index_from_another_model_is_discarded_rather_than_misread() {
        // Changing the model changes the dimensions. Reading the old file as though it belonged
        // to the new model would return neighbours computed in a space that no longer exists.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        {
            let index = VectorIndex::open(&path, 4).unwrap();
            index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
            index.save().unwrap();
        }

        let other = VectorIndex::open(&path, 8).unwrap();
        assert_eq!(other.len(), 0, "the old index must not be adopted");
        assert_eq!(other.dim(), 8);
    }

    #[test]
    fn a_corrupt_file_is_a_rebuild_rather_than_a_failure() {
        // The index is a cache. A cache that refuses to start is worse than one that rebuilds.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        std::fs::write(&path, b"this is not an hnsw graph").unwrap();

        let index = VectorIndex::open(&path, 4).unwrap();
        assert_eq!(index.len(), 0);
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn a_removed_chunk_stops_being_a_neighbour() {
        let (_tmp, index) = index();
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        index.add(ChunkId(2), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();
        index.remove(ChunkId(1)).unwrap();

        let hits = index.search(&unit([1.0, 0.0, 0.0, 0.0]), 5).unwrap();
        assert!(
            hits.iter().all(|h| h.chunk_id != ChunkId(1)),
            "a removed vector came back: {hits:?}"
        );
    }

    #[test]
    fn a_vector_of_the_wrong_size_is_refused_on_both_paths() {
        let (_tmp, index) = index();
        assert!(index.add(ChunkId(1), &[1.0, 0.0]).is_err());
        assert!(index.search(&[1.0, 0.0], 5).is_err());
    }

    #[test]
    fn capacity_grows_without_the_caller_thinking_about_it() {
        // The first thousand inserts of a first index run must not each trigger a reallocation,
        // and the thousand-and-first must not fail.
        let (_tmp, index) = index();
        let vectors: Vec<(ChunkId, Vec<f32>)> = (1..=2000)
            .map(|i| (ChunkId(i), unit([i as f32, 1.0, 0.0, 0.0])))
            .collect();
        index
            .add_many(vectors.iter().map(|(id, v)| (*id, v.as_slice())))
            .unwrap();
        assert_eq!(index.len(), 2000);
    }
}
