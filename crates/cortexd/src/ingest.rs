//! The ingest loop: watcher, coalescer, index maintenance and reconciliation.
//!
//! Runs on its own thread rather than on the async runtime. The work is blocking by nature —
//! `read(2)` on an inotify descriptor, `stat(2)` per entry, synchronous SQLite writes — and
//! wrapping it in futures would buy nothing except a way to stall the runtime that serves
//! queries.
//!
//! ## Waking up
//!
//! The loop polls rather than blocking on the descriptor, because it has to wake for two
//! independent reasons: an event arriving, and a coalescing window expiring. It sleeps until
//! whichever comes first, and when nothing is buffered it sleeps for [`IDLE_POLL`]. That is a
//! handful of wakeups per second doing nothing, comfortably inside the idle CPU budget, and it
//! avoids the second thread and channel that blocking reads would require.

use cortex_core::config::Config;
use cortex_core::denylist;
use cortex_core::model::{Action, Actor, NodeKind};
use cortex_ingest::event::Change;
use cortex_ingest::reconcile::{reconcile, ReconcileReport};
use cortex_ingest::watcher::{WatchHealth, Watcher};
use cortex_ingest::Coalescer;
use cortex_query::PathIndex;
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long to sleep when nothing at all is buffered.
const IDLE_POLL: Duration = Duration::from_millis(250);

/// Never sleep longer than this even with a distant deadline, so a stop request is acted on
/// promptly rather than after the coalescing window.
const MAX_SLEEP: Duration = Duration::from_millis(250);

/// Shared state the ingest loop publishes for the rest of the daemon to read.
#[derive(Debug, Default)]
pub struct IngestState {
    pub watch_health: Mutex<Option<WatchHealth>>,
    pub last_reconcile: Mutex<Option<ReconcileReport>>,
    /// Set by the kill switch. The loop keeps running but stops touching the index, so
    /// resuming does not mean rebuilding.
    pub paused: AtomicBool,
    /// Raised to ask for a reconcile at the next opportunity.
    resync_requested: AtomicBool,
    stop: AtomicBool,
}

impl IngestState {
    pub fn request_reconcile(&self) {
        self.resync_requested.store(true, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Used by the kill switch (hard rule R7), which lands with the Agent Surface in M3.
    #[allow(dead_code)]
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
}

pub struct Ingest {
    pub store: Arc<Store>,
    pub index: Arc<RwLock<PathIndex>>,
    pub state: Arc<IngestState>,
    pub config: Config,
}

impl Ingest {
    /// Run until stopped. Intended to own a thread.
    pub fn run(self) {
        let roots: Vec<PathBuf> = self.config.index.metadata_roots.clone();
        if roots.is_empty() {
            tracing::info!("no metadata roots configured; the watcher has nothing to do");
        }

        let mut watcher = match Watcher::new(self.config.index.watch_budget) {
            Ok(watcher) => watcher,
            Err(e) => {
                // Losing the watcher is a degradation, not a reason to take the daemon down:
                // search still works, and periodic reconciliation still keeps the index honest.
                tracing::error!(error = %e, "could not start the watcher; falling back to periodic reconciliation only");
                self.reconcile_loop_without_watcher(&roots);
                return;
            }
        };

        for root in &roots {
            match watcher.watch_tree(root) {
                Ok(added) => tracing::info!(root = %root.display(), watches = added, "watching"),
                Err(e) => tracing::warn!(root = %root.display(), error = %e, "could not watch"),
            }
        }
        self.publish_health(&watcher);

        // An initial reconcile brings the index up to date with everything that changed while
        // the daemon was not running. Without it, the first minutes after every start serve a
        // stale index.
        for root in &roots {
            self.run_reconcile(root);
        }

        let mut coalescer = Coalescer::new(Duration::from_millis(self.config.index.debounce_ms));
        let reconcile_interval = Duration::from_secs(self.config.index.reconcile_interval_secs);
        let mut next_reconcile = Instant::now() + reconcile_interval;

        while !self.state.stop.load(Ordering::Relaxed) {
            let now = Instant::now();

            match watcher.read_events() {
                Ok(events) => {
                    for event in events {
                        coalescer.push(event, now);
                    }
                }
                Err(e) => tracing::warn!(error = %e, "watcher read failed"),
            }

            for change in coalescer.drain_ready(Instant::now()) {
                self.apply(&change, &mut watcher);
            }

            let due = self.state.resync_requested.swap(false, Ordering::Relaxed)
                || Instant::now() >= next_reconcile;
            if due {
                for root in &roots {
                    self.run_reconcile(root);
                }
                self.publish_health(&watcher);
                next_reconcile = Instant::now() + reconcile_interval;
            }

            self.maybe_compact();

            let sleep = coalescer
                .next_deadline()
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(IDLE_POLL)
                .min(MAX_SLEEP);
            if !sleep.is_zero() {
                std::thread::sleep(sleep);
            }
        }

        tracing::info!("ingest loop stopped");
    }

    /// Degraded mode: no watcher, so reconciliation is the only source of truth.
    fn reconcile_loop_without_watcher(&self, roots: &[PathBuf]) {
        let interval = Duration::from_secs(self.config.index.reconcile_interval_secs);
        while !self.state.stop.load(Ordering::Relaxed) {
            for root in roots {
                self.run_reconcile(root);
            }
            let deadline = Instant::now() + interval;
            while Instant::now() < deadline && !self.state.stop.load(Ordering::Relaxed) {
                std::thread::sleep(MAX_SLEEP);
            }
        }
    }

    fn run_reconcile(&self, root: &Path) {
        if self.state.is_paused() {
            return;
        }
        let started = Instant::now();
        let report = reconcile(&self.store, root, &self.config.index.extra_deny, unix_now());

        if !report.is_clean() {
            tracing::info!(
                root = %root.display(),
                added = report.added,
                updated = report.updated,
                removed = report.removed,
                renamed = report.renamed,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "reconciled drift"
            );
            // Whole subtrees may have moved, so patching the in-memory index entry by entry
            // would be more error-prone than rebuilding it from what the database now says.
            self.rebuild_index();
        }

        *self.state.last_reconcile.lock().unwrap() = Some(report);
    }

    fn publish_health(&self, watcher: &Watcher) {
        let health = watcher.health();
        if health.degraded {
            tracing::warn!(
                watched = health.watched_dirs,
                unwatched = health.unwatched,
                "watch budget exhausted; part of the tree is unwatched and relies on reconciliation"
            );
        }
        *self.state.watch_health.lock().unwrap() = Some(health);
    }

    fn maybe_compact(&self) {
        let needs = self.index.read().unwrap().needs_compaction();
        if needs {
            self.index.write().unwrap().compact();
        }
    }

    /// Rebuild the in-memory path index from the database.
    pub fn rebuild_index(&self) {
        match self.store.all_paths_with_ids() {
            Ok(paths) => {
                let fresh = PathIndex::build(paths);
                *self.index.write().unwrap() = fresh;
            }
            Err(e) => tracing::error!(error = %e, "could not rebuild the path index"),
        }
    }

    fn apply(&self, change: &Change, watcher: &mut Watcher) {
        if self.state.is_paused() {
            return;
        }

        match change {
            Change::Upsert { path, is_dir } => {
                // Rule R1 applies to every route into the index, not only to the walker.
                if !denylist::classify(path, &self.config.index.extra_deny).may_index() {
                    return;
                }
                if *is_dir && !denylist::should_skip_subtree(&file_name_of(path)) {
                    // A new directory needs its own watch, or everything created inside it
                    // afterwards is invisible until the next reconcile.
                    if let Err(e) = watcher.watch_tree(path) {
                        tracing::warn!(path = %path.display(), error = %e, "could not watch new directory");
                    }
                }
                self.index_path(path, *is_dir);
            }

            Change::Touch { path } => {
                self.index_path(path, path.is_dir());
            }

            Change::Remove { path } => {
                watcher.unwatch(path);
                match self.store.remove_path(path) {
                    Ok(0) => {}
                    Ok(_) => {
                        let _ = self.store.append_episode(
                            unix_now(),
                            &Actor::System,
                            Action::Delete,
                            None,
                            None,
                            &serde_json::json!({ "source": "watcher", "path": path.display().to_string() }),
                        );
                        // A directory removal takes an unknown number of descendants with it,
                        // so the in-memory index is rebuilt rather than guessed at.
                        self.rebuild_index();
                    }
                    Err(e) => tracing::warn!(error = %e, path = %path.display(), "could not remove"),
                }
            }

            Change::Rename { from, to } => {
                watcher.unwatch(from);
                if to.is_dir() {
                    let _ = watcher.watch_tree(to);
                }
                match self.store.rename_node(from, to) {
                    Ok(0) => {
                        // We never knew the source — treat the destination as new.
                        self.index_path(to, to.is_dir());
                    }
                    Ok(_) => {
                        let _ = self.store.append_episode(
                            unix_now(),
                            &Actor::System,
                            Action::Move,
                            None,
                            None,
                            &serde_json::json!({
                                "source": "watcher",
                                "from": from.display().to_string(),
                                "to": to.display().to_string(),
                            }),
                        );
                        self.rebuild_index();
                    }
                    Err(e) => tracing::warn!(error = %e, "could not record a rename"),
                }
            }

            Change::ResyncRequired => {
                tracing::warn!("events were lost; requesting a full reconcile");
                self.state.request_reconcile();
            }
        }
    }

    fn index_path(&self, path: &Path, is_dir: bool) {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            // Gone again between the event and now. The removal event is either already
            // buffered or the reconcile scan will catch it.
            return;
        };

        let record = NodeUpsert {
            kind: if is_dir { NodeKind::Dir } else { NodeKind::File },
            path: path.to_path_buf(),
            parent_id: None,
            mime: None,
            size: metadata.len() as i64,
            mtime: unix_seconds(metadata.modified().ok()),
            ctime: unix_seconds(metadata.created().ok()),
            inode: inode_of(&metadata),
            dev: device_of(&metadata),
        };

        match self.store.upsert_node(&record, unix_now()) {
            Ok(id) => {
                let _ = self.store.append_episode(
                    unix_now(),
                    &Actor::System,
                    Action::Modify,
                    Some(id),
                    None,
                    &serde_json::json!({ "source": "watcher" }),
                );
                if let Some(text) = path.to_str() {
                    self.index.write().unwrap().insert(id, text);
                }
            }
            Err(e) => tracing::debug!(error = %e, path = %path.display(), "could not index"),
        }
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn unix_now() -> i64 {
    unix_seconds(Some(std::time::SystemTime::now()))
}

fn unix_seconds(time: Option<std::time::SystemTime>) -> i64 {
    time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn inode_of(metadata: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino() as i64
}

#[cfg(unix)]
fn device_of(metadata: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.dev() as i64
}

#[cfg(not(unix))]
fn inode_of(_metadata: &std::fs::Metadata) -> i64 {
    0
}

#[cfg(not(unix))]
fn device_of(_metadata: &std::fs::Metadata) -> i64 {
    0
}
