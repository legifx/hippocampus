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

/// How many just-touched files may be waiting for priority extraction.
///
/// Bounded because it is a courtesy queue, not a work queue: everything in it is also in the
/// database's own queue and will be extracted either way. Dropping the oldest entry when a burst
/// overflows costs a little latency on one file and nothing else.
const PRIORITY_QUEUE: usize = 512;

/// Shared state the ingest loop publishes for the rest of the daemon to read.
#[derive(Debug)]
pub struct IngestState {
    pub watch_health: Mutex<Option<WatchHealth>>,
    pub last_reconcile: Mutex<Option<ReconcileReport>>,
    /// Files a human just created or edited, for the extraction stage to take first.
    ///
    /// Without this, a file saved during a first index run waits behind the whole backlog —
    /// measured at minutes on the reference corpus — because the queue is drained in index
    /// order (see `Store::pending_extraction` for why it is not drained by recency).
    priority: Mutex<std::collections::VecDeque<cortex_core::ids::NodeId>>,
    /// Raised whenever something may have become extractable.
    ///
    /// The extraction stage backs off to a long sleep when its queue is empty, and this is what
    /// wakes it. Polling instead cost 0.65 % of one core doing nothing — over the whole daemon's
    /// idle budget, spent asking a question whose answer only changes when the watcher already
    /// knows it has.
    work_available: AtomicBool,
    /// Set by the kill switch. The loop keeps running but stops touching the index, so
    /// resuming does not mean rebuilding.
    pub paused: AtomicBool,
    /// Raised to ask for a reconcile at the next opportunity.
    resync_requested: AtomicBool,
    stop: AtomicBool,
}

impl Default for IngestState {
    fn default() -> Self {
        Self {
            watch_health: Mutex::new(None),
            last_reconcile: Mutex::new(None),
            priority: Mutex::new(std::collections::VecDeque::new()),
            // Signalled at construction: a daemon starting against an index full of pending
            // files must not wait for a change that may never come.
            work_available: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            resync_requested: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }
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

    /// Shared by every background stage, so that one stop request ends all of them.
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Note that a file was just touched, so extraction reaches it before the backlog.
    pub fn prioritise(&self, node: cortex_core::ids::NodeId) {
        {
            let mut queue = self.priority.lock().unwrap();
            if queue.len() >= PRIORITY_QUEUE {
                queue.pop_front();
            }
            queue.push_back(node);
        }
        self.signal_work();
    }

    /// Tell the extraction stage that there may be something to do.
    pub fn signal_work(&self) {
        self.work_available.store(true, Ordering::Relaxed);
    }

    /// Whether a signal is outstanding.
    pub fn work_signalled(&self) -> bool {
        self.work_available.load(Ordering::Relaxed)
    }

    /// Consume the signal.
    ///
    /// Cleared *before* the queue is read, never after: a signal arriving while the query runs
    /// must survive it. Clearing afterwards would drop exactly the file that was created during
    /// the query, and it would look like the watcher had missed it.
    pub fn clear_work_signal(&self) {
        self.work_available.store(false, Ordering::Relaxed);
    }

    /// Take up to `limit` files off the priority queue.
    pub fn take_prioritised(&self, limit: usize) -> Vec<cortex_core::ids::NodeId> {
        let mut queue = self.priority.lock().unwrap();
        let take = limit.min(queue.len());
        queue.drain(..take).collect()
    }
}

pub struct Ingest {
    pub store: Arc<Store>,
    pub index: Arc<RwLock<PathIndex>>,
    pub state: Arc<IngestState>,
    pub config: Config,
    /// The current zone set, kept in memory so that assigning a zone to a newly seen path is a
    /// comparison against a handful of prefixes rather than a database round trip. Bulk
    /// assignment still exists, but it is for when the *set* changes, not for every new file.
    zones: RwLock<Vec<cortex_core::zones::Zone>>,
}

impl Ingest {
    pub fn new(
        store: Arc<Store>,
        index: Arc<RwLock<PathIndex>>,
        state: Arc<IngestState>,
        config: Config,
    ) -> Self {
        Self {
            store,
            index,
            state,
            config,
            zones: RwLock::new(Vec::new()),
        }
    }
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
        let mut refreshed = false;
        for root in &roots {
            refreshed |= self.run_reconcile(root);
        }

        // If no reconcile had anything to do, the structure refresh still has to run once. An
        // index built by an earlier version has no zones and no directory graph, and a clean
        // reconcile would never trigger the pass that creates them.
        if !refreshed {
            self.refresh_structure();
        }

        let mut coalescer = Coalescer::new(Duration::from_millis(self.config.index.debounce_ms));
        let reconcile_interval = Duration::from_secs(self.config.index.reconcile_interval_secs);
        let mut next_reconcile = Instant::now() + reconcile_interval;

        while !self.state.stop_requested() {
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
        while !self.state.stop_requested() {
            for root in roots {
                self.run_reconcile(root);
            }
            let deadline = Instant::now() + interval;
            while Instant::now() < deadline && !self.state.stop_requested() {
                std::thread::sleep(MAX_SLEEP);
            }
        }
    }

    /// Returns whether the structure refresh ran, so a caller can avoid repeating it.
    fn run_reconcile(&self, root: &Path) -> bool {
        if self.state.is_paused() {
            return false;
        }
        let started = Instant::now();
        let report = reconcile(&self.store, root, &self.config.index.extra_deny, unix_now());

        let refreshed = !report.is_clean();
        if refreshed {
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
            self.refresh_structure();
            // Added or updated files are pending extraction now, and the stage may be asleep.
            self.state.signal_work();
        }

        *self.state.last_reconcile.lock().unwrap() = Some(report);
        refreshed
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

    /// Re-derive the parts of the index that are computed from the whole tree rather than from
    /// one path: the directory graph and the zone assignment.
    ///
    /// Both are cheap in bulk and expensive per row, so they run after a change rather than
    /// during one. Both are also idempotent, so running them more often than strictly necessary
    /// costs a little time and can never produce a wrong answer.
    pub fn refresh_structure(&self) {
        match self.store.link_parents() {
            Ok(0) => {}
            Ok(n) => tracing::debug!(linked = n, "directory graph updated"),
            Err(e) => tracing::warn!(error = %e, "could not link parents"),
        }
        self.refresh_zones(true);
    }

    /// Re-detect zones and, if the set actually changed, re-assign every node.
    ///
    /// The bulk assignment costs one pass over the index *per zone*, so doing it whenever a
    /// directory appears would make creating a handful of folders take longer than the initial
    /// index did. It runs when the set of zones changes — which is rare — and incremental
    /// inserts cover everything in between by consulting the cached list.
    fn refresh_zones(&self, force_assign: bool) {
        let detected = cortex_ingest::zone_detect::detect(
            &self.config.index.metadata_roots,
            self.config.index.zone_budget,
        );

        let previous: Vec<std::path::PathBuf> = self
            .zones
            .read()
            .unwrap()
            .iter()
            .map(|z| z.root_path.clone())
            .collect();
        let roots: Vec<std::path::PathBuf> = detected.iter().map(|z| z.root_path.clone()).collect();
        let set_changed = previous != roots;

        for zone in &detected {
            if let Err(e) = self.store.upsert_zone(zone) {
                tracing::warn!(error = %e, root = %zone.root_path.display(), "could not record zone");
            }
        }
        // A directory that stopped being a project, or was deleted, must stop being a zone —
        // otherwise the colour legend slowly fills with places that no longer exist.
        if let Err(e) = self.store.prune_zones(&roots) {
            tracing::warn!(error = %e, "could not prune zones");
        }

        // Re-read so the cache carries the database ids that incremental inserts need.
        match self.store.zones() {
            Ok(stored) => *self.zones.write().unwrap() = stored,
            Err(e) => tracing::warn!(error = %e, "could not read zones back"),
        }

        if set_changed || force_assign {
            match self.store.assign_zones() {
                Ok(n) => tracing::debug!(zones = detected.len(), assigned = n, "zones assigned"),
                Err(e) => tracing::warn!(error = %e, "could not assign zones"),
            }
        }
    }

    /// The zone a path belongs to, from the cached set.
    fn zone_for(&self, path: &Path) -> Option<cortex_core::ids::ZoneId> {
        let zones = self.zones.read().unwrap();
        cortex_core::zones::zone_for(path, &zones).and_then(|z| z.id)
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

                // Re-detect zones only when this path could plausibly have created one: a
                // directory with a conventional name, or a file that is a project marker.
                // Anything else leaves the zone set untouched, and re-running detection for it
                // would cost a full pass over the index for nothing.
                let name = file_name_of(path);
                let might_be_a_zone = if *is_dir {
                    cortex_ingest::zone_detect::is_conventional_area(&name)
                        || self.config.index.metadata_roots.iter().any(|r| path.parent() == Some(r))
                } else {
                    cortex_ingest::zone_detect::is_project_marker(&name)
                };
                if might_be_a_zone {
                    self.refresh_zones(false);
                }
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
                        self.refresh_structure();
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
                        self.refresh_structure();
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

        // One lookup, on a path that is almost certainly already in SQLite's page cache. Doing
        // it here keeps the directory graph correct between reconciles rather than only after
        // one.
        let parent_id = path
            .parent()
            .and_then(|parent| self.store.node_by_path(parent).ok().flatten())
            .map(|node| node.id);

        let record = NodeUpsert {
            kind: if is_dir { NodeKind::Dir } else { NodeKind::File },
            path: path.to_path_buf(),
            parent_id,
            zone_id: self.zone_for(path),
            mime: None,
            size: metadata.len() as i64,
            mtime: unix_seconds(metadata.modified().ok()),
            ctime: unix_seconds(metadata.created().ok()),
            inode: inode_of(&metadata),
            dev: device_of(&metadata),
        };

        match self.store.upsert_node(&record, unix_now()) {
            Ok(id) => {
                if !is_dir {
                    // A file that just changed is the one somebody is most likely to search for
                    // in the next minute. Directories have no content to extract.
                    self.state.prioritise(id);
                }
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
