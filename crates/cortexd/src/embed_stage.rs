//! The embedding stage: replay, more expensive still.
//!
//! ## Why this is not part of extraction
//!
//! Extraction has the chunk text in hand, so embedding there would save a file read. It is still
//! the wrong place, and the numbers say why: on the reference corpus, extracting 100 000 files
//! costs **26 seconds**, embedding the 122 137 chunks they produce costs **hours** on a CPU.
//! Sharing one loop would hold the cheap, high-value work behind the expensive one — lexical
//! search would arrive hours late instead of in half a minute.
//!
//! So the two are separate stages over the same queue discipline. Extraction drains first and
//! makes everything findable by word; embedding follows and makes it findable by meaning. The
//! price is re-reading each chunk from its file, which is a few hundred microseconds against
//! tens of milliseconds of inference — under one per cent of the cost of the thing it decouples.
//!
//! Re-reading is also not optional in general: a model that arrives *after* the content was
//! indexed has no other way to see it, and that is the normal case, because the model is a
//! deliberate download rather than part of the install.
//!
//! ## What it guarantees
//!
//! - A chunk whose text no longer matches its stored hash is skipped rather than embedded. The
//!   vector would describe text the index does not have.
//! - Vectors go into `embeddings` (authoritative, `f32`) and into the usearch index (derived,
//!   `f16`) in that order, so a crash between the two loses only a cache.
//! - Everything runs under the same duty cycle and kill switch as extraction.

use crate::activity::Activity;
use crate::ingest::IngestState;
use crate::semantics::{Loaded, Semantics};
use cortex_core::config::Config;
use cortex_embed::Input;
use cortex_extract::snippet::{self, Location};
use cortex_ingest::throttle;
use cortex_store::embeddings::{ChunkToEmbed, EmbeddingWrite, ModelRef};
use cortex_store::Store;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Chunks claimed from the queue at once. A multiple of the batch size, so a claim feeds several
/// inference calls without holding much text in memory.
const CLAIM: usize = 64;

/// How often the usearch index is written out while a backlog drains.
///
/// The index is derived, so losing it costs a rebuild rather than data — but a rebuild of a
/// hundred thousand vectors is minutes, and saving is cheap. Every few thousand vectors is the
/// balance.
const SAVE_EVERY: usize = 4096;

const QUEUE_EMPTY_POLL: Duration = Duration::from_secs(2);
const MAX_QUEUE_POLL: Duration = Duration::from_secs(60);
const SLEEP_GRANULARITY: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub struct EmbedStats {
    pub chunks_embedded: AtomicU64,
    /// Chunks whose text no longer matches the hash the index holds for them.
    pub chunks_stale: AtomicU64,
    pub failures: AtomicU64,
    pub busy_us: AtomicU64,
    /// Set once the backlog has drained at least once, so `cortex status` can distinguish
    /// "still working" from "nothing to do".
    pub caught_up: std::sync::atomic::AtomicBool,
}

pub struct Embedding {
    store: Arc<Store>,
    /// The model, if there is one *yet*. Consulted per pass rather than captured, because
    /// `cortex model fetch` can make one appear while this thread is already running.
    semantics: Arc<Semantics>,
    state: Arc<IngestState>,
    activity: Arc<Activity>,
    stats: Arc<EmbedStats>,
    config: Config,
}

/// One pass's view of the model: the embedder, its index, and the identity both are stored
/// under. Bundled so a reload between two batches cannot write vectors from one model under the
/// name of another.
struct Session {
    loaded: Loaded,
    model: cortex_embed::ModelId,
}

impl Session {
    fn reference(&self) -> ModelRef<'_> {
        ModelRef {
            id: self.model.id.as_str(),
            version: self.model.version.as_str(),
            dim: self.model.dim,
        }
    }
}

impl Embedding {
    pub fn new(
        store: Arc<Store>,
        semantics: Arc<Semantics>,
        state: Arc<IngestState>,
        activity: Arc<Activity>,
        stats: Arc<EmbedStats>,
        config: Config,
    ) -> Self {
        Self {
            store,
            semantics,
            state,
            activity,
            stats,
            config,
        }
    }

    fn session(&self) -> Option<Session> {
        let loaded = self.semantics.get()?;
        let model = loaded.embedder.model();
        Some(Session { loaded, model })
    }

    pub fn run(self) {
        let mut idle_wait = QUEUE_EMPTY_POLL;
        let mut since_save = 0usize;
        let mut announced = false;
        // The chunk table as it was when the queue last came back empty. While it is unchanged
        // there is provably nothing to embed, and asking again costs a scan of every chunk.
        let mut settled_at: Option<(i64, i64)> = None;

        while !self.state.stop_requested() {
            if self.state.is_paused() {
                self.sleep(QUEUE_EMPTY_POLL);
                continue;
            }

            // No model is a normal state, not an error: the download is a deliberate user
            // action. Wait quietly for one to appear rather than logging every few seconds.
            let Some(session) = self.session() else {
                announced = false;
                self.sleep(MAX_QUEUE_POLL);
                continue;
            };
            if !announced {
                announced = true;
                tracing::info!(
                    model = %session.model.id,
                    version = %session.model.version,
                    dim = session.model.dim,
                    backend = %session.loaded.embedder.backend(),
                    "embedding stage running"
                );
            }

            // Cheap first: two indexed aggregates. Only when they differ from the last settled
            // state is the expensive queue query worth running.
            let watermark = self.store.chunk_watermark().ok();
            if settled_at.is_some() && watermark == settled_at {
                self.sleep(idle_wait);
                idle_wait = (idle_wait * 2).min(MAX_QUEUE_POLL);
                continue;
            }

            let started = Instant::now();
            let batch = match self.store.chunks_without_embedding(session.reference(), CLAIM) {
                Ok(batch) => batch,
                Err(e) => {
                    tracing::warn!(error = %e, "could not read the embedding queue");
                    self.sleep(QUEUE_EMPTY_POLL);
                    continue;
                }
            };

            if batch.is_empty() {
                settled_at = watermark;
                // Nothing left to compute does not mean everything is findable. The ANN index
                // keys by chunk id while the vectors key by content, so re-deriving chunks —
                // which is what `--rebuild-index` does — leaves the table complete and the index
                // empty. Checking here is what makes that self-healing, and it is the same path
                // that rebuilds the index when its file is deleted (Definition of Done §8).
                self.reconcile_index(&session);
                if !self.stats.caught_up.swap(true, Ordering::Relaxed) {
                    // Nothing left to add, so the index goes back to being a mapping: its pages
                    // become file-backed and the daemon stops holding a hundred megabytes of
                    // heap it will never write to again.
                    if let Err(e) = session.loaded.vectors.demote() {
                        tracing::warn!(error = %e, "could not map the vector index");
                    }
                    tracing::info!(
                        vectors = session.loaded.vectors.len(),
                        "embedding caught up with the corpus"
                    );
                }
                self.sleep(idle_wait);
                idle_wait = (idle_wait * 2).min(MAX_QUEUE_POLL);
                continue;
            }
            idle_wait = QUEUE_EMPTY_POLL;
            settled_at = None;

            let embedded = self.run_batch(&session, batch, started);
            if embedded == 0 {
                // The claim was not empty but nothing came of it: every chunk in it was stale,
                // or inference failed. Either way, claiming the same rows again at full speed
                // would be a busy loop against a queue that only extraction can clear.
                self.sleep(idle_wait);
                idle_wait = (idle_wait * 2).min(MAX_QUEUE_POLL);
            }
            since_save += embedded;
            if since_save >= SAVE_EVERY {
                self.save_index(&session);
                since_save = 0;
            }
        }

        // Whatever was learned since the last save is worth keeping; the alternative is
        // recomputing it from the table on the next start for no reason.
        if let Some(session) = self.session() {
            self.save_index(&session);
        }
        tracing::info!("embedding stage stopped");
    }

    /// Read, embed and store one claim's worth of chunks. Returns how many were embedded.
    fn run_batch(&self, session: &Session, batch: Vec<ChunkToEmbed>, started: Instant) -> usize {
        let mut embedded = 0;
        let mut slice_started = started;

        for group in batch.chunks(self.config.embedding.batch_size) {
            if self.state.stop_requested() || self.state.is_paused() {
                break;
            }

            // Read the text back through the same path a snippet uses, which verifies the hash
            // on the way. A chunk whose file has moved on is left for the extraction stage to
            // re-chunk; embedding it would describe text nobody can find.
            let mut texts = Vec::with_capacity(group.len());
            let mut chunks = Vec::with_capacity(group.len());
            for chunk in group {
                match snippet::chunk_text(Location {
                    path: &chunk.path,
                    byte_start: chunk.byte_start as usize,
                    byte_end: chunk.byte_end as usize,
                    hash: &chunk.hash,
                }) {
                    Some(text) => {
                        texts.push(text);
                        chunks.push(chunk);
                    }
                    None => {
                        self.stats.chunks_stale.fetch_add(1, Ordering::Relaxed);
                        // Nothing takes a chunk off this queue except a vector, and a stale
                        // chunk will never get one — it would describe text the index does not
                        // have. So the *file* is queued for re-extraction, which deletes the
                        // stale chunk and produces ones that exist. Without this the stage
                        // re-reads the same dead chunk on every pass, for ever.
                        self.state.prioritise(chunk.node_id);
                    }
                }
            }
            if chunks.is_empty() {
                // Everything in this group was stale, and each one has been queued for
                // re-extraction. Pause briefly so a corpus of moving files cannot turn this
                // into a busy loop while extraction catches up.
                self.sleep(Duration::from_millis(200));
                continue;
            }

            let inputs: Vec<Input<'_>> = texts.iter().map(|t| Input::Passage(t.as_str())).collect();
            let vectors = match session.loaded.embedder.embed(&inputs) {
                Ok(vectors) => vectors,
                Err(e) => {
                    self.stats.failures.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(error = %e, chunks = chunks.len(), "embedding failed");
                    self.rest(slice_started.elapsed());
                    slice_started = Instant::now();
                    continue;
                }
            };

            let rows: Vec<EmbeddingWrite> = chunks
                .iter()
                .zip(&vectors)
                .map(|(chunk, vector)| EmbeddingWrite {
                    chunk_id: chunk.chunk_id,
                    chunk_hash: chunk.hash.clone(),
                    vector: vector.clone(),
                })
                .collect();

            // The table first: it is authoritative, and the index can always be rebuilt from it.
            match self
                .store
                .upsert_embeddings(session.reference(), &rows, crate::ingest::unix_now())
            {
                Ok(written) => {
                    embedded += written;
                    self.stats
                        .chunks_embedded
                        .fetch_add(written as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    self.stats.failures.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(error = %e, "could not store embeddings");
                    continue;
                }
            }

            if let Err(e) = session
                .loaded
                .vectors
                .add_many(rows.iter().map(|r| (r.chunk_id, r.vector.as_slice())))
            {
                tracing::warn!(error = %e, "could not add vectors to the index");
            }

            self.rest(slice_started.elapsed());
            slice_started = Instant::now();
        }

        embedded
    }

    /// Make the ANN index agree with the table, refilling it if it does not.
    ///
    /// Cheap in the normal case: one count against one number already in memory. Expensive
    /// exactly when it needs to be.
    fn reconcile_index(&self, session: &Session) {
        let expected = match self.store.indexable_vector_count(session.reference()) {
            Ok(count) => count,
            Err(e) => {
                tracing::warn!(error = %e, "could not count indexable vectors");
                return;
            }
        };
        if expected == session.loaded.vectors.len() {
            return;
        }

        tracing::info!(
            expected,
            present = session.loaded.vectors.len(),
            "rebuilding the vector index from the store"
        );
        if let Err(e) = session.loaded.vectors.clear() {
            tracing::warn!(error = %e, "could not clear the vector index");
            return;
        }

        let mut batch: Vec<(cortex_core::ids::ChunkId, Vec<f32>)> = Vec::with_capacity(1024);
        let mut added = 0usize;
        let mut flush = |batch: &mut Vec<(cortex_core::ids::ChunkId, Vec<f32>)>| {
            if batch.is_empty() {
                return;
            }
            if let Err(e) = session
                .loaded
                .vectors
                .add_many(batch.iter().map(|(id, v)| (*id, v.as_slice())))
            {
                tracing::warn!(error = %e, "could not refill the vector index");
            }
            batch.clear();
        };

        let result = self
            .store
            .for_each_indexable_vector(session.reference(), |chunk, vector| {
                batch.push((chunk, vector.to_vec()));
                added += 1;
                if batch.len() == 1024 {
                    flush(&mut batch);
                }
            });
        flush(&mut batch);

        match result {
            Ok(_) => {
                self.save_index(session);
                tracing::info!(vectors = added, "vector index rebuilt without the model");
            }
            Err(e) => tracing::warn!(error = %e, "could not read embeddings back"),
        }
    }

    fn save_index(&self, session: &Session) {
        if let Err(e) = session.loaded.vectors.save() {
            tracing::warn!(error = %e, "could not save the vector index");
        }
    }

    fn rest(&self, worked: Duration) {
        self.stats
            .busy_us
            .fetch_add(worked.as_micros() as u64, Ordering::Relaxed);
        let active = self
            .activity
            .is_active(Duration::from_secs(self.config.resources.idle_after_secs));
        let share = throttle::share_for(
            active,
            self.config.resources.cpu_share_active,
            self.config.resources.cpu_share_idle,
        );
        self.sleep(throttle::pause_after(worked, share));
    }

    fn sleep(&self, total: Duration) {
        let deadline = Instant::now() + total;
        while Instant::now() < deadline && !self.state.stop_requested() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(SLEEP_GRANULARITY));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_embed::hashing::HashingEmbedder;
    use cortex_embed::Embedder as _;
    use cortex_extract::classify::Handling;
    use cortex_store::chunks::ChunkWrite;
    use cortex_store::queries::NodeUpsert;
    use std::path::PathBuf;

    struct Harness {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        store: Arc<Store>,
        semantics: Arc<Semantics>,
        state: Arc<IngestState>,
        stats: Arc<EmbedStats>,
        config: Config,
    }

    impl Harness {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            let mut config = Config::default();
            config.index.content_roots = vec![root.clone()];
            config.resources.cpu_share_idle = 1.0;
            config.resources.cpu_share_active = 1.0;

            // The built-in embedder, through the same path the real one takes: no model files,
            // no download, and every other line of this stage exercised exactly as it ships.
            config.embedding.model = HashingEmbedder::ID.to_string();
            let semantics = Semantics::new(&root, config.embedding.clone());
            semantics.load().expect("the hashing embedder always loads");

            Self {
                _tmp: tmp,
                root,
                store: Arc::new(Store::open_in_memory().unwrap()),
                semantics: Arc::new(semantics),
                state: Arc::new(IngestState::default()),
                stats: Arc::new(EmbedStats::default()),
                config,
            }
        }

        fn stage(&self) -> Embedding {
            Embedding::new(
                self.store.clone(),
                self.semantics.clone(),
                self.state.clone(),
                Arc::new(Activity::default()),
                self.stats.clone(),
                self.config.clone(),
            )
        }

        /// Write a file, index it and chunk it, as extraction would have.
        fn add(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, contents).unwrap();

            let id = self
                .store
                .upsert_node(
                    &NodeUpsert {
                        kind: cortex_core::model::NodeKind::File,
                        path: path.clone(),
                        parent_id: None,
                        mime: None,
                        size: contents.len() as i64,
                        mtime: 1,
                        ctime: 1,
                        inode: 1,
                        dev: 1,
                        zone_id: None,
                    },
                    1000,
                )
                .unwrap();

            let writes: Vec<ChunkWrite> = cortex_extract::chunk::chunk(contents, Handling::PlainText)
                .into_iter()
                .map(|c| ChunkWrite {
                    ordinal: c.ordinal as i64,
                    byte_start: c.byte_start as i64,
                    byte_end: c.byte_end as i64,
                    preview: Some(c.preview(240)),
                    hash: c.hash,
                    text: c.text,
                })
                .collect();
            self.store.replace_chunks(id, &writes).unwrap();
            path
        }

        /// Drain the queue the way `run` would, without the thread or the waiting.
        fn drain(&self) {
            let stage = self.stage();
            let session = stage.session().expect("the fixture loads an embedder");
            for _ in 0..20 {
                let batch = self
                    .store
                    .chunks_without_embedding(session.reference(), CLAIM)
                    .unwrap();
                if batch.is_empty() {
                    break;
                }
                if stage.run_batch(&session, batch, Instant::now()) == 0 {
                    break;
                }
            }
        }

        fn vectors(&self) -> Arc<cortex_store::vectors::VectorIndex> {
            self.semantics.get().unwrap().vectors
        }

        fn model_ref(&self) -> cortex_store::embeddings::ModelRef<'static> {
            ModelRef {
                id: HashingEmbedder::ID,
                version: HashingEmbedder::VERSION,
                dim: cortex_embed::hashing::DIM,
            }
        }
    }

    #[test]
    fn chunks_become_vectors_in_the_table_and_the_index() {
        let harness = Harness::new();
        harness.add("notizen.txt", "Die Blickachse bleibt über die Sequenz konstant.\n");
        harness.drain();

        assert_eq!(harness.store.embedding_count(harness.model_ref()).unwrap(), 1);
        assert_eq!(harness.vectors().len(), 1);
        assert_eq!(harness.stats.chunks_embedded.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_second_pass_embeds_nothing_new() {
        // The queue is defined by what has no vector *for this model*. If that ever stopped
        // being true, every restart would re-embed the whole corpus.
        let harness = Harness::new();
        for i in 0..5 {
            harness.add(&format!("datei{i}.txt"), &format!("Inhalt Nummer {i}\n"));
        }
        harness.drain();
        let after_first = harness.stats.chunks_embedded.load(Ordering::Relaxed);
        assert_eq!(after_first, 5);

        harness.drain();
        assert_eq!(
            harness.stats.chunks_embedded.load(Ordering::Relaxed),
            after_first,
            "a second pass re-embedded chunks that already had vectors"
        );
    }

    #[test]
    fn a_chunk_whose_file_changed_is_not_embedded() {
        // The vector would describe text that is not in the index. Better to leave it out and
        // let re-extraction produce chunks that exist.
        let harness = Harness::new();
        let path = harness.add("wandelbar.txt", "Der ursprüngliche Inhalt der Datei.\n");
        std::fs::write(&path, "Etwas ganz anderes steht jetzt hier drin.\n").unwrap();

        harness.drain();

        assert_eq!(harness.store.embedding_count(harness.model_ref()).unwrap(), 0);
        assert_eq!(harness.stats.chunks_stale.load(Ordering::Relaxed), 1);
        assert_eq!(harness.vectors().len(), 0);
    }

    #[test]
    fn a_stale_chunk_queues_its_file_for_re_extraction() {
        // Otherwise the stage would claim the same chunk on every pass for ever, and the file
        // would never be repaired.
        let harness = Harness::new();
        let path = harness.add("wandelbar.txt", "Der ursprüngliche Inhalt.\n");
        std::fs::write(&path, "Anderer Inhalt.\n").unwrap();
        harness.state.take_prioritised(100);

        harness.drain();

        assert!(
            !harness.state.take_prioritised(10).is_empty(),
            "the file was not queued for re-extraction"
        );
    }

    #[test]
    fn the_vector_index_finds_the_chunk_it_was_given() {
        // End to end through the real index: embed a document, then look for it with a vector
        // derived from its own words.
        let harness = Harness::new();
        harness.add(
            "kamera.txt",
            "Die Kamerawinkel wurden an drei Montagepositionen verglichen.\n",
        );
        harness.add("budget.txt", "Der Haushalt wurde im Sommer freigegeben.\n");
        harness.drain();

        let query = HashingEmbedder::new()
            .embed(&[Input::Query("kamerawinkel montagepositionen")])
            .unwrap()
            .remove(0);
        let hits = harness.vectors().search(&query, 2).unwrap();

        assert!(!hits.is_empty());
        let best = harness
            .store
            .chunks_without_embedding(
                ModelRef { id: "nobody", version: "0", dim: 4 },
                1000,
            )
            .unwrap()
            .into_iter()
            .find(|c| c.chunk_id == hits[0].chunk_id)
            .expect("the hit must be a chunk that exists");
        assert!(
            best.path.ends_with("kamera.txt"),
            "the nearest vector was {}",
            best.path.display()
        );
    }

    /// An embedder that counts what it was asked to do.
    ///
    /// Definition of Done §4 says re-indexing an unchanged corpus re-embeds zero chunks and asks
    /// for it to be asserted by a call counter. Counting stored vectors would not do: it would
    /// pass just as well if the model ran and produced the same numbers again, which is the
    /// expensive failure this is about.
    #[derive(Default)]
    struct CountingEmbedder {
        inner: HashingEmbedder,
        texts: std::sync::atomic::AtomicUsize,
    }

    impl cortex_embed::Embedder for CountingEmbedder {
        fn model(&self) -> cortex_embed::ModelId {
            self.inner.model()
        }
        fn embed(&self, inputs: &[Input<'_>]) -> cortex_embed::Result<Vec<Vec<f32>>> {
            self.texts.fetch_add(inputs.len(), Ordering::Relaxed);
            self.inner.embed(inputs)
        }
        fn backend(&self) -> String {
            self.inner.backend()
        }
    }

    #[test]
    fn re_indexing_an_unchanged_corpus_asks_the_model_for_nothing() {
        let harness = Harness::new();
        for i in 0..6 {
            harness.add(&format!("doc{i}.txt"), &format!("Inhalt Nummer {i} mit etwas Text.\n"));
        }

        let counter = Arc::new(CountingEmbedder::default());
        let drain_counting = || {
            let stage = Embedding::new(
                harness.store.clone(),
                harness.semantics.clone(),
                harness.state.clone(),
                Arc::new(Activity::default()),
                harness.stats.clone(),
                harness.config.clone(),
            );
            let session = Session {
                loaded: Loaded {
                    embedder: counter.clone(),
                    vectors: harness.semantics.get().unwrap().vectors,
                },
                model: counter.model(),
            };
            for _ in 0..10 {
                let batch = harness
                    .store
                    .chunks_without_embedding(session.reference(), CLAIM)
                    .unwrap();
                if batch.is_empty() {
                    break;
                }
                if stage.run_batch(&session, batch, Instant::now()) == 0 {
                    break;
                }
            }
        };

        drain_counting();
        let first = counter.texts.load(Ordering::Relaxed);
        assert_eq!(first, 6, "the first pass must embed every chunk exactly once");

        // Re-derive the chunks, exactly as `--rebuild-index` does: same text, new rows.
        for node in harness.store.all_paths_with_ids().unwrap() {
            let existing = harness.store.chunks_for_node(node.0).unwrap();
            let again: Vec<cortex_store::chunks::ChunkWrite> = existing
                .iter()
                .map(|c| cortex_store::chunks::ChunkWrite {
                    ordinal: c.ordinal,
                    byte_start: c.byte_start,
                    byte_end: c.byte_end,
                    hash: c.hash.clone(),
                    preview: c.preview.clone(),
                    text: "not read again".into(),
                })
                .collect();
            harness.store.delete_chunks(node.0).unwrap();
            harness.store.replace_chunks(node.0, &again).unwrap();
        }

        drain_counting();
        assert_eq!(
            counter.texts.load(Ordering::Relaxed),
            first,
            "a re-index called the model again"
        );
    }

    #[test]
    fn the_vector_index_is_refilled_when_it_disagrees_with_the_table() {
        // Definition of Done §8, and the bug a live rebuild found: the vectors are keyed by
        // content and survive, the index keys by chunk id and does not. Nothing else notices,
        // because the embedding queue is empty — every hash already has a vector — so semantic
        // search simply stops returning anything.
        let harness = Harness::new();
        harness.add("bleibt.txt", "Inhalt der erhalten bleibt.\n");
        harness.drain();
        assert_eq!(harness.vectors().len(), 1);

        harness.vectors().clear().unwrap();
        assert_eq!(harness.vectors().len(), 0);

        let stage = harness.stage();
        let session = stage.session().unwrap();
        assert!(
            harness
                .store
                .chunks_without_embedding(session.reference(), 10)
                .unwrap()
                .is_empty(),
            "the fixture is wrong: there is still embedding work, so this would not be the case"
        );

        stage.reconcile_index(&session);
        assert_eq!(
            harness.vectors().len(),
            1,
            "the index must be refilled from the table, without the model"
        );
    }

    #[test]
    fn a_matching_index_is_left_alone() {
        // The check runs every time the queue drains, so it has to be cheap and quiet when
        // there is nothing to do.
        let harness = Harness::new();
        harness.add("unveraendert.txt", "Inhalt.\n");
        harness.drain();

        let stage = harness.stage();
        let session = stage.session().unwrap();
        stage.reconcile_index(&session);
        assert_eq!(harness.vectors().len(), 1);
    }

    #[test]
    fn pausing_stops_the_stage_rather_than_slowing_it() {
        let harness = Harness::new();
        for i in 0..40 {
            harness.add(&format!("viele{i}.txt"), &format!("Inhalt {i}\n"));
        }
        harness.state.set_paused(true);

        let stage = harness.stage();
        let session = stage.session().unwrap();
        let batch = harness
            .store
            .chunks_without_embedding(session.reference(), CLAIM)
            .unwrap();
        stage.run_batch(&session, batch, Instant::now());

        assert_eq!(harness.stats.chunks_embedded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_empty_corpus_is_not_an_error() {
        let harness = Harness::new();
        harness.drain();
        assert_eq!(harness.stats.chunks_embedded.load(Ordering::Relaxed), 0);
        assert_eq!(harness.stats.failures.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_missing_file_costs_one_chunk_rather_than_the_batch() {
        let harness = Harness::new();
        let gone = harness.add("verschwindet.txt", "Inhalt der gleich weg ist.\n");
        harness.add("bleibt.txt", "Inhalt der bleibt.\n");
        std::fs::remove_file(&gone).unwrap();

        harness.drain();

        assert_eq!(
            harness.store.embedding_count(harness.model_ref()).unwrap(),
            1,
            "the surviving file must still be embedded"
        );
        assert!(harness.stats.chunks_stale.load(Ordering::Relaxed) >= 1);
        assert!(
            !harness.state.take_prioritised(10).is_empty(),
            "the unreadable file must be queued for re-extraction rather than retried for ever"
        );
    }

}
