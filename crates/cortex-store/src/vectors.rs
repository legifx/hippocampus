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
//! ## Mapped until something writes
//!
//! The index is opened with `view` — memory-mapped, read-only — because that is what the build
//! prompt specifies and because of what it costs otherwise. Measured on the reference corpus:
//! 122 137 vectors is a 104 MiB file and about 145 MiB of heap when loaded, which took the
//! daemon's idle resident memory to 307 MiB against a 250 MB budget. Mapped, those pages are
//! file-backed: they are only resident once touched, and the kernel can drop them under
//! pressure instead of pushing the daemon into swap.
//!
//! A mapped index cannot be written to, so the first mutation promotes it: the mapping is
//! dropped, the file is loaded, and writes continue on the heap. [`VectorIndex::demote`] goes
//! back the other way once there is nothing left to add, which is the steady state — a corpus is
//! embedded once and searched for months.
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
    /// True while the index is a read-only mapping of the file.
    mapped: RwLock<bool>,
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
            // The header is read first, and it is not an optimisation. `view` maps the file and
            // trusts what it finds there: pointed at a truncated or foreign file it does not
            // return an error, it **segfaults**, which was discovered by the test that writes
            // rubbish into the path. Reading the metadata is a plain file read that fails
            // cleanly, so nothing is mapped until the header has been agreed with.
            let usable = match Index::metadata(&path.to_string_lossy()) {
                Ok(metadata) => {
                    let matches = metadata.dimensions as usize == dim;
                    if !matches {
                        tracing::warn!(
                            found = metadata.dimensions,
                            expected = dim,
                            "vector index belongs to another model; rebuilding from the store"
                        );
                    }
                    matches
                }
                Err(e) => {
                    // A corrupt cache is not a corrupt database. Say so and start again.
                    tracing::warn!(error = %e, "vector index header is unreadable; rebuilding from the store");
                    false
                }
            };

            if usable {
            match index.view(&path.to_string_lossy()) {
                Ok(()) if index.dimensions() == dim => {
                    tracing::info!(
                        vectors = index.size(),
                        path = %path.display(),
                        "vector index mapped"
                    );
                    return Ok(Self {
                        index: RwLock::new(index),
                        mapped: RwLock::new(true),
                        path: path.to_path_buf(),
                        dim,
                    });
                }
                Ok(()) => tracing::warn!(
                    found = index.dimensions(),
                    expected = dim,
                    "vector index belongs to another model; rebuilding from the store"
                ),
                Err(e) => tracing::warn!(
                    error = %e,
                    "vector index could not be mapped; rebuilding from the store"
                ),
            }
            }
        }

        Ok(Self {
            index: RwLock::new(new_index(dim)?),
            mapped: RwLock::new(false),
            path: path.to_path_buf(),
            dim,
        })
    }

    /// Whether the index is currently a mapping rather than a heap copy.
    pub fn is_mapped(&self) -> bool {
        *self.mapped.read().unwrap()
    }

    /// Make the index writable, loading it from disk if it is currently mapped.
    ///
    /// Called by every mutation. Idempotent, and cheap once it has happened.
    fn promote(&self) -> Result<()> {
        let mut mapped = self.mapped.write().unwrap();
        if !*mapped {
            return Ok(());
        }

        let fresh = new_index(self.dim)?;
        if self.path.exists() && Index::metadata(&self.path.to_string_lossy()).is_ok() {
            fresh.load(&self.path.to_string_lossy()).map_err(vector_error)?;
        }
        *self.index.write().unwrap() = fresh;
        *mapped = false;
        tracing::debug!("vector index loaded for writing");
        Ok(())
    }

    /// Save, then go back to being a mapping.
    ///
    /// The steady state of a corpus that has been embedded: everything is searched, nothing is
    /// added. Giving the pages back to the page cache is the difference between the daemon
    /// sitting inside its memory budget and sitting above it.
    pub fn demote(&self) -> Result<()> {
        if self.is_mapped() {
            return Ok(());
        }
        self.save()?;

        let mut mapped = self.mapped.write().unwrap();
        let fresh = new_index(self.dim)?;
        fresh
            .view(&self.path.to_string_lossy())
            .map_err(vector_error)?;
        *self.index.write().unwrap() = fresh;
        *mapped = true;
        tracing::debug!("vector index mapped again");
        Ok(())
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

        self.promote()?;
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
            let key = chunk.0 as u64;
            // Adding a key that is already there is an error in usearch's wrapper, not a
            // replacement. "Here is the vector for this chunk" has to be idempotent: a
            // re-embedding pass, a model that was re-run, or an index file left over from an
            // earlier run all produce exactly this, and none of them is a failure.
            if index.contains(key) {
                index.remove(key).map_err(vector_error)?;
            }
            index.add(key, vector).map_err(vector_error)?;
        }
        Ok(())
    }

    /// Remove a chunk's vector. Absent keys are not an error — the caller is deleting.
    pub fn remove(&self, chunk: ChunkId) -> Result<()> {
        self.remove_many(std::iter::once(chunk)).map(|_| ())
    }

    /// Remove several keys under one lock. A key the index does not hold is not an error: the
    /// caller is clearing a note about chunks that *may* still be in here, and most of the
    /// time — after a rebuild, or for a chunk that was never embedded — they are not.
    pub fn remove_many(&self, chunks: impl IntoIterator<Item = ChunkId>) -> Result<usize> {
        self.promote()?;
        let index = self.index.write().unwrap();
        let mut removed = 0;
        for chunk in chunks {
            let key = chunk.0 as u64;
            if index.contains(key) {
                index.remove(key).map_err(vector_error)?;
                removed += 1;
            }
        }
        Ok(removed)
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
        *self.mapped.write().unwrap() = false;
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
    fn re_adding_a_key_does_not_grow_the_count() {
        // A chunk whose file changed is embedded again under the same id: remove, then add.
        // The count the reconcile compares against the table must not grow by one for every
        // such re-embedding, or the index "drifts" by exactly the number of files edited since
        // the last check and is rebuilt on every idle pass. (Seen live on 2026-09-06: expected
        // 49257, present 49258, a twelve-second rebuild every five minutes, for ever.)
        let tmp = tempfile::tempdir().unwrap();
        let index = VectorIndex::open(&tmp.path().join("vectors.usearch"), 4).unwrap();
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        index.add(ChunkId(2), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();
        assert_eq!(index.len(), 2);
        index.add(ChunkId(1), &unit([0.0, 0.0, 1.0, 0.0])).unwrap();
        assert_eq!(index.len(), 2, "re-adding key 1 must replace it, not count twice");
        index.remove(ChunkId(2)).unwrap();
        assert_eq!(index.len(), 1, "a removed key is not counted");
    }

    #[test]
    fn a_removed_key_stays_uncounted_after_save_and_reopen() {
        // The live path: the index is saved, mapped read-only while idle, and loaded back for
        // writing when a file changes. A tombstone that is not counted in memory but is counted
        // after a round trip through the file is the off-by-one the reconcile saw.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        {
            let index = VectorIndex::open(&path, 4).unwrap();
            index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
            index.add(ChunkId(2), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();
            index.add(ChunkId(1), &unit([0.0, 0.0, 1.0, 0.0])).unwrap();
            index.remove(ChunkId(2)).unwrap();
            assert_eq!(index.len(), 1);
            index.save().unwrap();
        }
        let reopened = VectorIndex::open(&path, 4).unwrap();
        assert_eq!(reopened.len(), 1, "the file must not resurrect the removed key in the count");
        reopened.add(ChunkId(3), &unit([0.0, 0.0, 0.0, 1.0])).unwrap();
        assert_eq!(reopened.len(), 2, "and not after promotion to a writable index either");
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
    fn a_corrupt_file_is_a_rebuild_rather_than_a_crash() {
        // The index is a cache, so a bad one must cost a rebuild rather than a start. It very
        // nearly cost the process instead: `view` maps the file and trusts its header, and
        // pointing it at this fixture segfaulted rather than returning an error. The header is
        // now read with a plain file read first.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        std::fs::write(&path, b"this is not an hnsw graph").unwrap();

        let index = VectorIndex::open(&path, 4).unwrap();
        assert_eq!(index.len(), 0);
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn an_existing_index_is_mapped_rather_than_loaded() {
        // The whole reason for the mode: 122 137 vectors are 104 MiB on disk and about 145 MiB
        // on the heap, and the daemon's entire memory budget is 250 MB.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        {
            let index = VectorIndex::open(&path, 4).unwrap();
            index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
            assert!(!index.is_mapped(), "a fresh index is writable");
            index.save().unwrap();
        }

        let reopened = VectorIndex::open(&path, 4).unwrap();
        assert!(reopened.is_mapped());
        assert_eq!(reopened.len(), 1);
        assert_eq!(
            reopened.search(&unit([1.0, 0.0, 0.0, 0.0]), 1).unwrap()[0].chunk_id,
            ChunkId(1)
        );
    }

    #[test]
    fn writing_to_a_mapped_index_promotes_it_and_keeps_what_was_there() {
        // A mapped index is read-only. Adding to it has to load the file first, and losing the
        // existing vectors in the process would be a silent, corpus-wide regression.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        {
            let index = VectorIndex::open(&path, 4).unwrap();
            index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
            index.save().unwrap();
        }

        let index = VectorIndex::open(&path, 4).unwrap();
        assert!(index.is_mapped());
        index.add(ChunkId(2), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();

        assert!(!index.is_mapped(), "an add must have promoted it");
        assert_eq!(index.len(), 2, "the mapped vectors must survive the promotion");
        assert_eq!(
            index.search(&unit([1.0, 0.0, 0.0, 0.0]), 1).unwrap()[0].chunk_id,
            ChunkId(1)
        );
    }

    #[test]
    fn demoting_saves_and_maps_again() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vectors.usearch");
        let index = VectorIndex::open(&path, 4).unwrap();
        index.add(ChunkId(3), &unit([0.0, 0.0, 1.0, 0.0])).unwrap();
        assert!(!index.is_mapped());

        index.demote().unwrap();
        assert!(index.is_mapped());
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.search(&unit([0.0, 0.0, 1.0, 0.0]), 1).unwrap()[0].chunk_id,
            ChunkId(3)
        );

        // And it is on disk, which is what "demote" promised.
        let reopened = VectorIndex::open(&path, 4).unwrap();
        assert_eq!(reopened.len(), 1);
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
    fn adding_the_same_chunk_twice_replaces_rather_than_failing() {
        // Found by the goldset runner against a leftover index file: usearch's wrapper refuses a
        // duplicate key outright. Re-embedding a chunk is an ordinary thing to do — after a model
        // change, or over an index that survived from an earlier run — and it must overwrite.
        let (_tmp, index) = index();
        index.add(ChunkId(1), &unit([1.0, 0.0, 0.0, 0.0])).unwrap();
        index.add(ChunkId(1), &unit([0.0, 1.0, 0.0, 0.0])).unwrap();

        assert_eq!(index.len(), 1, "the chunk must appear once, not twice");
        let hits = index.search(&unit([0.0, 1.0, 0.0, 0.0]), 1).unwrap();
        assert_eq!(hits[0].chunk_id, ChunkId(1));
        assert!(
            hits[0].similarity > 0.9,
            "the second vector must be the one that stuck, got {}",
            hits[0].similarity
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
