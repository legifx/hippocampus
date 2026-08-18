//! The extraction stage: replay at rest.
//!
//! This is where files stop being paths and become searchable content. It is also the most
//! expensive thing the daemon does, so its design is dominated by two constraints that have
//! nothing to do with extraction itself.
//!
//! **It must be resumable.** State lives in `nodes.extract_state`, not in this loop. A daemon
//! killed halfway through a first index run picks up where it stopped; nothing recomputes what
//! is already done, and nothing has to be held in memory between runs.
//!
//! **It must be invisible.** Work happens in bounded slices with a computed pause between them
//! (`cortex_ingest::throttle`), at a `nice` level, on its own thread. A first index run that
//! makes the machine feel slow is how this project loses its user in week one.
//!
//! Everything it decides about a file lives in `cortex_extract::pipeline`. This module owns the
//! queue, the budget and the transaction — not the rules.
//!
//! It writes no episodes. Episodes record what happened to a *file*; being extracted is
//! something that happened to the *index*, and a per-file episode would add a hundred thousand
//! rows saying "the system did its job" to an append-only table on every first run. What the
//! stage is doing is observable through `cortex extract` instead.

use crate::activity::Activity;
use crate::ingest::IngestState;
use cortex_core::config::Config;
use cortex_core::model::ExtractState;
use cortex_extract::pipeline::{self, Outcome};
use cortex_ingest::throttle;
use cortex_store::chunks::{ChunkWrite, PendingNode};
use cortex_store::Store;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Files claimed from the queue in one go.
///
/// Small on purpose. Every file in a batch holds its extracted chunk text in memory until the
/// batch is written, and the daemon has a 250 MB budget for everything it does.
const BATCH_FILES: usize = 16;

/// Accumulated chunk text that forces a write even when the batch is not full.
///
/// The bound that actually protects the memory budget: sixteen ordinary files are nothing,
/// sixteen two-megabyte documents are not.
const MAX_PENDING_TEXT: usize = 8 * 1024 * 1024;

/// First wait when the queue turns up empty.
const QUEUE_EMPTY_POLL: Duration = Duration::from_secs(1);

/// Longest wait between checks of an empty queue.
///
/// The stage does not rely on this to notice work: the watcher signals it, and the signal is
/// checked while it sleeps. This is the backstop for anything that changes the queue without
/// going through the watcher — and it has to be long, because asking the database every second
/// whether it has anything to do measured at 0.65 % of one core on a 100 000-file index, which
/// is more than the daemon's entire idle budget.
const MAX_QUEUE_POLL: Duration = Duration::from_secs(30);

/// Never sleep longer than this in one go, so that stopping stays prompt.
const SLEEP_GRANULARITY: Duration = Duration::from_millis(250);

/// Counters the rest of the daemon can read. Cheap enough to update per file.
#[derive(Debug, Default)]
pub struct ExtractStats {
    pub files_extracted: AtomicU64,
    pub files_unchanged: AtomicU64,
    pub files_skipped: AtomicU64,
    pub files_failed: AtomicU64,
    pub chunks_written: AtomicU64,
    /// Chunks that survived a re-extraction untouched, and therefore will never be re-embedded.
    /// The number that says whether the content hash is earning its place.
    pub chunks_reused: AtomicU64,
    pub chunks_deleted: AtomicU64,
    pub bytes_read: AtomicU64,
    /// Microseconds, not milliseconds.
    ///
    /// A batch of small files costs a few hundred microseconds, and accumulating that in
    /// milliseconds truncates every single batch to zero — the counter then reads "this stage
    /// has never done any work" after extracting a hundred thousand files, which is exactly
    /// what it did read before this was fixed.
    pub busy_us: AtomicU64,
}

pub struct Extraction {
    store: Arc<Store>,
    state: Arc<IngestState>,
    activity: Arc<Activity>,
    stats: Arc<ExtractStats>,
    config: Config,
    settings: pipeline::Settings,
}

/// One file's outcome, held until the batch is written.
struct Prepared {
    node: cortex_core::ids::NodeId,
    state: ExtractState,
    note: Option<String>,
    hash: Option<String>,
    mime: Option<String>,
    /// `None` means "leave the stored chunks alone" — the content did not change.
    chunks: Option<Vec<ChunkWrite>>,
}

impl Extraction {
    pub fn new(
        store: Arc<Store>,
        state: Arc<IngestState>,
        activity: Arc<Activity>,
        stats: Arc<ExtractStats>,
        config: Config,
    ) -> Self {
        let settings = pipeline::Settings {
            content_roots: config.index.content_roots.clone(),
            extra_deny: config.index.extra_deny.clone(),
            ..pipeline::Settings::default()
        };
        Self {
            store,
            state,
            activity,
            stats,
            config,
            settings,
        }
    }

    /// Run until stopped. Intended to own a thread.
    pub fn run(self) {
        if self.settings.content_roots.is_empty() {
            // Rule R3: without a content root there is nothing this stage may legally read, and
            // that is the default. Say so once, clearly, rather than looping over a queue that
            // can never drain — a silent no-op here reads as a bug in the search.
            tracing::info!(
                "no content_roots configured; extraction is off and the index stays metadata-only"
            );
            return;
        }

        apply_nice(self.config.resources.nice_level);
        tracing::info!(
            roots = ?self.settings.content_roots,
            nice = self.config.resources.nice_level,
            "extraction stage started"
        );

        let mut idle_wait = QUEUE_EMPTY_POLL;
        let mut seen_generation = 0u64;
        while !self.state.stop_requested() {
            if self.state.is_paused() {
                self.sleep(QUEUE_EMPTY_POLL);
                continue;
            }

            // Read before the queue is, so a file created during the read is acted on next time
            // round rather than being lost between the two.
            seen_generation = self.state.work_generation();

            // Timing starts *before* the queue query, because that query is work too. Measuring
            // only the extraction meant the duty cycle throttled a fraction of what the stage
            // actually costs the machine — reading the queue and committing the batch together
            // dominate a batch of small files.
            let slice_started = Instant::now();
            match self.next_batch() {
                Ok(batch) if batch.is_empty() => {
                    self.wait_for_work(idle_wait, seen_generation);
                    idle_wait = (idle_wait * 2).min(MAX_QUEUE_POLL);
                }
                Ok(batch) => {
                    idle_wait = QUEUE_EMPTY_POLL;
                    self.run_batch(batch, slice_started);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not read the extraction queue");
                    self.sleep(QUEUE_EMPTY_POLL);
                }
            }
        }

        tracing::info!("extraction stage stopped");
    }

    /// Files the watcher just saw first, then the backlog.
    ///
    /// The two queues answer different questions. The priority queue is "what did a person just
    /// touch", and it is short and almost always empty. The database queue is "what is left",
    /// and during a first index run it holds everything. Draining only the second would make a
    /// file saved during that run wait for all of it.
    fn next_batch(&self) -> cortex_store::error::Result<Vec<PendingNode>> {
        let urgent = self.state.take_prioritised(BATCH_FILES);
        if !urgent.is_empty() {
            let batch = self.store.pending_by_ids(&urgent)?;
            if !batch.is_empty() {
                return Ok(batch);
            }
            // Everything in the queue had already been extracted by a normal pass. Fall through
            // rather than returning empty, or the stage would idle with work outstanding.
        }
        self.store.pending_extraction(BATCH_FILES)
    }

    fn run_batch(&self, batch: Vec<PendingNode>, started: Instant) {
        let mut prepared: Vec<Prepared> = Vec::with_capacity(batch.len());
        let mut pending_text = 0usize;
        let mut slice_started = started;

        for file in batch {
            if self.state.stop_requested() || self.state.is_paused() {
                break;
            }

            let outcome = self.prepare(&file);
            pending_text += outcome
                .chunks
                .as_ref()
                .map(|chunks| chunks.iter().map(|c| c.text.len()).sum::<usize>())
                .unwrap_or(0);
            prepared.push(outcome);

            if pending_text >= MAX_PENDING_TEXT || slice_started.elapsed() >= throttle::SLICE {
                self.flush(&mut prepared);
                pending_text = 0;
                // The commit is inside the slice: it is the single most expensive thing a batch
                // of small files does, and a pause computed without it is not a duty cycle.
                self.rest(slice_started.elapsed());
                slice_started = Instant::now();
            }
        }

        self.flush(&mut prepared);
        self.rest(slice_started.elapsed());
    }

    /// Run the pipeline over one file. No database writes — those are batched.
    fn prepare(&self, file: &PendingNode) -> Prepared {
        // The hash is only trustworthy while the chunks it describes still exist. After a
        // rebuild they do not, and believing the hash would skip the file forever.
        let previous = file
            .content_hash
            .as_deref()
            .filter(|_| file.chunk_count > 0);

        let outcome = pipeline::process(&file.path, file.size as u64, previous, &self.settings);
        let note = outcome.note();

        match outcome {
            Outcome::Extracted {
                content_hash,
                mime,
                chunks,
                ..
            } => {
                self.stats.files_extracted.fetch_add(1, Ordering::Relaxed);
                self.stats.bytes_read.fetch_add(
                    chunks.last().map(|c| c.byte_end as u64).unwrap_or(0),
                    Ordering::Relaxed,
                );
                let writes = chunks
                    .into_iter()
                    .map(|c| ChunkWrite {
                        ordinal: c.ordinal as i64,
                        byte_start: c.byte_start as i64,
                        byte_end: c.byte_end as i64,
                        preview: Some(c.preview(self.settings.preview_chars)),
                        hash: c.hash,
                        text: c.text,
                    })
                    .collect();
                Prepared {
                    node: file.id,
                    state: ExtractState::Done,
                    note,
                    hash: Some(content_hash),
                    mime,
                    chunks: Some(writes),
                }
            }

            Outcome::Unchanged { content_hash } => {
                self.stats.files_unchanged.fetch_add(1, Ordering::Relaxed);
                Prepared {
                    node: file.id,
                    state: ExtractState::Done,
                    note,
                    hash: Some(content_hash),
                    mime: None,
                    chunks: None,
                }
            }

            Outcome::Skipped { mime, .. } => {
                self.stats.files_skipped.fetch_add(1, Ordering::Relaxed);
                Prepared {
                    node: file.id,
                    state: ExtractState::Skipped,
                    note,
                    // A file that may not be read may not keep a hash of its content either.
                    hash: None,
                    mime,
                    // It may have been readable before: dropping to metadata-only has to take
                    // the old content out of the index with it.
                    chunks: Some(Vec::new()),
                }
            }

            Outcome::Failed { .. } => {
                self.stats.files_failed.fetch_add(1, Ordering::Relaxed);
                Prepared {
                    node: file.id,
                    state: ExtractState::Failed,
                    note,
                    hash: None,
                    mime: None,
                    chunks: None,
                }
            }
        }
    }

    /// Write a batch of results in one transaction.
    ///
    /// One commit for many files rather than one per file: a document's chunks and its state
    /// belong to the same fact, and SQLite's durability barrier is what dominates otherwise.
    /// A failure rolls the whole batch back, which is safe precisely because every file in it
    /// stays `pending` and will be retried.
    fn flush(&self, prepared: &mut Vec<Prepared>) {
        if prepared.is_empty() {
            return;
        }

        let batch: Vec<Prepared> = std::mem::take(prepared);
        let result = self.store.batch(|tx| {
            for file in &batch {
                if let Some(chunks) = &file.chunks {
                    let delta = tx.replace_chunks(file.node, chunks)?;
                    self.stats
                        .chunks_written
                        .fetch_add(delta.inserted as u64, Ordering::Relaxed);
                    self.stats
                        .chunks_reused
                        .fetch_add(delta.reused as u64, Ordering::Relaxed);
                    self.stats
                        .chunks_deleted
                        .fetch_add(delta.deleted as u64, Ordering::Relaxed);
                }
                tx.set_extraction_result(
                    file.node,
                    file.hash.as_deref(),
                    file.mime.as_deref(),
                    file.state,
                    file.note.as_deref(),
                )?;
            }
            Ok(())
        });

        if let Err(e) = result {
            // The files stay pending, so this is a retry rather than a loss. It is still worth
            // shouting about: a persistent failure here means extraction never converges.
            tracing::warn!(error = %e, files = batch.len(), "could not write an extraction batch");
        }
    }

    /// Stand back for as long as the CPU share requires.
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

    /// Sleep in small steps so a stop request is not stuck behind it.
    fn sleep(&self, total: Duration) {
        let deadline = Instant::now() + total;
        while Instant::now() < deadline && !self.state.stop_requested() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(SLEEP_GRANULARITY));
        }
    }

    /// Sleep until the deadline, or until somebody says there is work.
    ///
    /// The signal is what keeps a file saved on an idle machine from waiting half a minute:
    /// the backoff exists to stop the database being asked pointlessly, not to delay work.
    fn wait_for_work(&self, total: Duration, seen_generation: u64) {
        let deadline = Instant::now() + total;
        while Instant::now() < deadline
            && !self.state.stop_requested()
            && self.state.work_generation() == seen_generation
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(SLEEP_GRANULARITY));
        }
    }
}

/// Lower this thread's scheduling priority.
///
/// On Linux `nice` is per-thread and `setpriority(PRIO_PROCESS, 0, …)` applies to the calling
/// thread, which is exactly the granularity wanted here: the extraction thread steps back while
/// the socket server does not.
#[cfg(target_os = "linux")]
fn apply_nice(level: i32) {
    if level == 0 {
        return;
    }
    // Safety: `setpriority` takes no pointers and cannot fail in a way that affects memory. A
    // refusal (raising priority without privilege) is reported and otherwise ignored.
    let result = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, level) };
    if result != 0 {
        tracing::debug!(
            level,
            error = %std::io::Error::last_os_error(),
            "could not set the extraction thread's nice level"
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_nice(_level: i32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::ids::NodeId;
    use cortex_store::queries::NodeUpsert;
    use std::path::{Path, PathBuf};

    /// A stage wired to a temporary corpus, with everything that would sleep set to zero.
    struct Harness {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        store: Arc<Store>,
        state: Arc<IngestState>,
        activity: Arc<Activity>,
        stats: Arc<ExtractStats>,
        config: Config,
    }

    impl Harness {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            let mut config = Config::default();
            config.index.content_roots = vec![root.clone()];
            // The stage must not actually throttle inside a test: the arithmetic has its own
            // tests, and sleeping here would only make the suite slow.
            config.resources.cpu_share_idle = 1.0;
            config.resources.cpu_share_active = 1.0;
            Self {
                _tmp: tmp,
                root,
                store: Arc::new(Store::open_in_memory().unwrap()),
                state: Arc::new(IngestState::default()),
                activity: Arc::new(Activity::default()),
                stats: Arc::new(ExtractStats::default()),
                config,
            }
        }

        fn stage(&self) -> Extraction {
            Extraction::new(
                self.store.clone(),
                self.state.clone(),
                self.activity.clone(),
                self.stats.clone(),
                self.config.clone(),
            )
        }

        /// Write a file and index it as the walker would.
        fn add(&self, name: &str, contents: &str) -> (NodeId, PathBuf) {
            let path = self.root.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, contents).unwrap();
            let id = self.index(&path);
            (id, path)
        }

        fn index(&self, path: &Path) -> NodeId {
            let metadata = std::fs::metadata(path).unwrap();
            self.store
                .upsert_node(
                    &NodeUpsert {
                        kind: cortex_core::model::NodeKind::File,
                        path: path.to_path_buf(),
                        parent_id: None,
                        mime: None,
                        size: metadata.len() as i64,
                        mtime: metadata
                            .modified()
                            .unwrap()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64,
                        ctime: 0,
                        inode: 1,
                        dev: 1,
                        zone_id: None,
                    },
                    1000,
                )
                .unwrap()
        }

        /// Drain the queue, exactly as `run` would but without the thread or the sleeping.
        fn drain(&self) {
            let stage = self.stage();
            loop {
                let batch = self.store.pending_extraction(BATCH_FILES).unwrap();
                if batch.is_empty() {
                    break;
                }
                let mut prepared: Vec<Prepared> =
                    batch.iter().map(|file| stage.prepare(file)).collect();
                stage.flush(&mut prepared);
            }
        }
    }

    #[test]
    fn a_text_file_becomes_searchable_by_its_contents() {
        let harness = Harness::new();
        let (node, _) = harness.add(
            "notes/besprechung.md",
            "# Kamerawinkel\n\nDie Blickachse bleibt konstant über die ganze Sequenz.\n",
        );

        harness.drain();

        let hits = harness.store.search_chunks("\"blickachse\"", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, node);
        assert!(hits[0].preview.is_some(), "a result must be displayable without file access");
    }

    #[test]
    fn re_extracting_an_unchanged_corpus_writes_no_chunks_at_all() {
        // Definition of Done §4 for M2, minus the embedder that does not exist yet: the count
        // asserted here is exactly the count of embedding calls a re-index would make.
        let harness = Harness::new();
        for i in 0..5 {
            harness.add(&format!("doc{i}.md"), &format!("# Titel {i}\n\nInhalt {i}.\n"));
        }
        harness.drain();
        let written = harness.stats.chunks_written.load(Ordering::Relaxed);
        assert_eq!(written, 5);

        harness.store.requeue_extraction().unwrap();
        harness.drain();

        assert_eq!(
            harness.stats.chunks_written.load(Ordering::Relaxed),
            written,
            "a re-index must not write a single new chunk"
        );
        assert_eq!(harness.stats.chunks_deleted.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn editing_one_paragraph_rewrites_one_chunk_and_keeps_the_rest() {
        // The performance lever, stated as a test. Without hash matching every chunk of the
        // document would be rewritten, and later every one of them re-embedded.
        let harness = Harness::new();
        let section = |n: usize| {
            format!(
                "# Abschnitt {n}\n\n{}\n\n",
                "Ein hinreichend langer Absatz über die Blickachse. ".repeat(40)
            )
        };
        let document: String = (0..4).map(section).collect();
        let (node, path) = harness.add("lang.md", &document);
        harness.drain();

        let before = harness.store.chunks_for_node(node).unwrap();
        assert!(before.len() >= 4, "fixture must produce several chunks");
        let baseline_written = harness.stats.chunks_written.load(Ordering::Relaxed);

        let edited = document.replace("Abschnitt 2", "Abschnitt zwei, umbenannt");
        std::fs::write(&path, &edited).unwrap();
        harness.index(&path);
        harness.drain();

        let newly_written = harness.stats.chunks_written.load(Ordering::Relaxed) - baseline_written;
        assert!(
            newly_written <= 2,
            "expected at most the edited chunk and its overlap neighbour, wrote {newly_written}"
        );
        assert!(harness.stats.chunks_reused.load(Ordering::Relaxed) >= 2);
    }

    #[test]
    fn a_file_outside_the_content_roots_is_recorded_as_skipped_and_not_retried() {
        let harness = Harness::new();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("elsewhere.md");
        std::fs::write(&path, "content that stays private\n").unwrap();
        harness.index(&path);

        harness.drain();

        let (state, note) = harness.store.extract_note(&path).unwrap().unwrap();
        assert_eq!(state, ExtractState::Skipped);
        assert_eq!(note.as_deref(), Some("outside every content root (rule R3)"));
        assert!(
            harness.store.pending_extraction(10).unwrap().is_empty(),
            "a skipped file must not come back on the next pass"
        );
    }

    #[test]
    fn a_denied_file_is_never_opened_even_inside_a_content_root() {
        // Rule R1 end to end, through the real stage rather than through the classifier alone.
        let harness = Harness::new();
        let (_, path) = harness.add(".ssh/id_ed25519", "content that must never be indexed\n");
        harness.drain();

        assert!(harness.store.search_chunks("\"content\"", 10).unwrap().is_empty());
        let (state, note) = harness.store.extract_note(&path).unwrap().unwrap();
        assert_eq!(state, ExtractState::Skipped);
        assert_eq!(note.as_deref(), Some("on the deny-list (rule R1)"));
    }

    #[test]
    fn a_file_that_stops_being_readable_loses_its_chunks() {
        // A document moved out of a content root, or renamed to something the deny-list covers,
        // must take its content out of the index with it. Leaving the chunks behind would keep
        // answering searches from text the user has withdrawn — the index would still hold, and
        // return, content that the file no longer offers.
        let harness = Harness::new();
        let (node, path) = harness.add("draft.md", "vertrauliche Notizen zum Budget\n");
        harness.drain();
        assert_eq!(harness.store.chunks_for_node(node).unwrap().len(), 1);

        let secret = harness.root.join("draft.pem");
        std::fs::rename(&path, &secret).unwrap();
        harness.store.rename_node(&path, &secret).unwrap();
        harness.store.requeue_extraction().unwrap();
        harness.drain();

        assert!(harness.store.chunks_for_node(node).unwrap().is_empty());
        assert!(harness.store.search_chunks("\"vertrauliche\"", 10).unwrap().is_empty());
    }

    #[test]
    fn a_binary_file_is_skipped_without_being_treated_as_a_failure() {
        let harness = Harness::new();
        let path = harness.root.join("bild.png");
        std::fs::write(&path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0]).unwrap();
        harness.index(&path);

        harness.drain();

        let (state, _) = harness.store.extract_note(&path).unwrap().unwrap();
        assert_eq!(state, ExtractState::Skipped);
        assert_eq!(harness.stats.files_failed.load(Ordering::Relaxed), 0);
        let node = harness.store.node_by_path(&path).unwrap().unwrap();
        assert_eq!(
            node.mime.as_deref(),
            Some("image/png"),
            "a file that is skipped is still classified"
        );
    }

    #[test]
    fn a_rebuilt_index_re_chunks_even_though_the_hashes_still_match() {
        // Rule R10: dropping the derived tables and starting the daemon must restore full
        // function. The content hash survives that, so the stage has to notice the chunks did
        // not.
        let harness = Harness::new();
        let (node, _) = harness.add("bestand.md", "Inhalt der wiederhergestellt werden muss\n");
        harness.drain();
        assert_eq!(harness.store.chunks_for_node(node).unwrap().len(), 1);

        harness.store.delete_chunks(node).unwrap();
        harness.store.requeue_extraction().unwrap();
        harness.drain();

        assert_eq!(
            harness.store.chunks_for_node(node).unwrap().len(),
            1,
            "the chunks must come back rather than being skipped as unchanged"
        );
        assert!(!harness.store.search_chunks("\"wiederhergestellt\"", 10).unwrap().is_empty());
    }

    #[test]
    fn the_busy_counter_survives_batches_that_take_under_a_millisecond() {
        // It did not, for the length of one whole corpus run: accumulating microseconds as
        // milliseconds truncated every batch of small files to zero, and the stage reported
        // that it had never worked after extracting a hundred thousand files.
        let harness = Harness::new();
        for i in 0..4 {
            harness.add(&format!("winzig{i}.txt"), "kurz\n");
        }

        let stage = harness.stage();
        let batch = harness.store.pending_extraction(BATCH_FILES).unwrap();
        stage.run_batch(batch, Instant::now());

        assert!(
            harness.stats.busy_us.load(Ordering::Relaxed) > 0,
            "a batch that did real work reported none"
        );
    }

    #[test]
    fn a_freshly_touched_file_jumps_the_backlog() {
        // Measured before it was fixed: a file saved during a first index run of the reference
        // corpus waited behind forty-five thousand others, because the queue drains in index
        // order. The watcher knows better and now says so.
        let harness = Harness::new();
        for i in 0..60 {
            harness.add(&format!("backlog/{i}.md"), &format!("Alter Inhalt {i}\n"));
        }
        let (urgent, _) = harness.add("gerade-getippt.md", "Zwiebelkuchen und Federweisser\n");
        harness.state.prioritise(urgent);

        let stage = harness.stage();
        let batch = stage.next_batch().unwrap();
        assert_eq!(
            batch.iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![urgent],
            "the file the watcher just saw must be extracted first"
        );

        stage.run_batch(batch, Instant::now());
        assert!(
            !harness.store.search_chunks("\"zwiebelkuchen\"", 10).unwrap().is_empty(),
            "and it must be searchable immediately afterwards"
        );
    }

    #[test]
    fn a_priority_entry_that_is_already_done_does_not_stall_the_backlog() {
        // The queue is a courtesy, not a source of truth: entries in it may have been extracted
        // by an ordinary pass in the meantime. Returning empty for those would idle the stage
        // while work was outstanding.
        let harness = Harness::new();
        let (done, _) = harness.add("schon-fertig.md", "Inhalt\n");
        harness.add("wartet.md", "Anderer Inhalt\n");
        harness
            .store
            .set_extract_state(done, ExtractState::Done, None)
            .unwrap();
        harness.state.prioritise(done);

        let batch = harness.stage().next_batch().unwrap();
        assert_eq!(batch.len(), 1);
        assert_ne!(batch[0].id, done);
    }

    #[test]
    fn an_idle_stage_waits_for_a_signal_rather_than_asking_the_database() {
        // The daemon's whole idle budget is 0.5 % of one core, and asking a 100 000-file index
        // once a second whether it has work measured at 0.65 % on its own. The wait therefore
        // has to end on a signal, not on a poll.
        let harness = Harness::new();
        let stage = harness.stage();
        let started = Instant::now();
        let generation = harness.state.work_generation();
        stage.wait_for_work(Duration::from_millis(600), generation);
        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "an unsignalled wait must actually wait"
        );

        harness.state.signal_work();
        let started = Instant::now();
        stage.wait_for_work(Duration::from_secs(30), generation);
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "a signalled wait must return at once, not after the backoff"
        );
    }

    #[test]
    fn a_new_file_signals_the_stage_awake() {
        let harness = Harness::new();
        let before = harness.state.work_generation();
        let (node, _) = harness.add("gerade-erst.md", "Inhalt\n");
        harness.state.prioritise(node);
        assert!(
            harness.state.work_generation() > before,
            "prioritising a file must also wake whatever is going to extract it"
        );
    }

    #[test]
    fn one_signal_reaches_both_stages() {
        // A boolean flag belongs to whichever stage looks first, and the other one sleeps through
        // it. That is not hypothetical: a rebuild dropped every vector, the extraction stage
        // consumed the signal, and the embedding stage stayed asleep with 1 721 chunks waiting.
        let harness = Harness::new();
        let extraction_saw = harness.state.work_generation();
        let embedding_saw = harness.state.work_generation();

        harness.state.signal_work();

        assert!(harness.state.work_generation() > extraction_saw);
        assert!(
            harness.state.work_generation() > embedding_saw,
            "the second stage must see the signal too"
        );
    }

    #[test]
    fn extraction_stops_when_the_kill_switch_is_pulled() {
        // Rule R7: pausing must stop the expensive stage, not merely make it quieter.
        let harness = Harness::new();
        for i in 0..40 {
            harness.add(&format!("viele/{i}.md"), &format!("Datei Nummer {i}\n"));
        }
        harness.state.set_paused(true);

        let stage = harness.stage();
        let batch = harness.store.pending_extraction(BATCH_FILES).unwrap();
        stage.run_batch(batch, Instant::now());

        assert_eq!(harness.stats.files_extracted.load(Ordering::Relaxed), 0);
        assert_eq!(
            harness.store.pending_extraction(100).unwrap().len(),
            40,
            "nothing may have been consumed while paused"
        );
    }
}
