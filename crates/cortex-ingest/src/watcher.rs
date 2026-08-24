//! The `inotify` watcher.
//!
//! One watch per directory, managed by us. See decision **D20** for why this rather than
//! `fanotify`: mount-wide `fanotify` requires `CAP_SYS_ADMIN`, and asking for near-root
//! privileges to read a home directory is a worse trade than the feature is worth.
//!
//! Two properties matter more than throughput here:
//!
//! - **The watch set is bounded.** Exhausting `max_user_watches` must degrade visibly, in
//!   `cortex status`, rather than silently leaving parts of the tree unwatched.
//! - **Loss is expected.** Queue overflow is reported upwards as a resync request, not
//!   swallowed. The reconcile scan is the safety net, and it is not optional.

use crate::event::{RawEvent, RawKind};
use cortex_core::denylist;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("could not initialise inotify: {0}")]
    Init(#[source] std::io::Error),

    #[error("could not watch {path}: {source}")]
    Add {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not read events: {0}")]
    Read(#[source] std::io::Error),
}

/// What the watcher is allowed to notice.
///
/// Two choices here are load-bearing.
///
/// **`IN_CLOSE_WRITE` rather than `IN_MODIFY`.** `IN_MODIFY` fires on every `write(2)`, so a
/// large file being copied produces thousands of events describing one arrival. `IN_CLOSE_WRITE`
/// fires once, when the writer is done.
///
/// **`IN_MOVE_SELF` is deliberately absent.** Renaming a watched directory produces both the
/// parent's `IN_MOVED_FROM`/`IN_MOVED_TO` pair *and* an `IN_MOVE_SELF` on the directory's own
/// watch. The pair describes the move exactly; `IN_MOVE_SELF` says only "something happened to
/// me" with no destination. Subscribing to both meant the self-event was read as a deletion,
/// and a deletion supersedes a pending rename in the coalescer — so renaming a directory
/// deleted its entire subtree from the index instead of moving it. A move that only
/// `IN_MOVE_SELF` could have caught (the parent is not watched) is left to reconciliation,
/// which is what reconciliation is for.
fn watch_mask() -> WatchMask {
    WatchMask::CREATE
        | WatchMask::CLOSE_WRITE
        | WatchMask::DELETE
        | WatchMask::DELETE_SELF
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::ATTRIB
        | WatchMask::EXCL_UNLINK
}

/// How the watcher is currently coping. Surfaced by `cortex status`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WatchHealth {
    pub watched_dirs: usize,
    pub budget: usize,
    /// True once the budget was hit and some subtree is therefore unwatched.
    pub degraded: bool,
    /// Directories we wanted to watch but could not.
    pub unwatched: usize,
    /// How many times the kernel queue overflowed since start.
    pub overflows: u64,
}

pub struct Watcher {
    inotify: Inotify,
    watches: Watches,
    by_descriptor: HashMap<WatchDescriptor, PathBuf>,
    by_path: HashMap<PathBuf, WatchDescriptor>,
    budget: usize,
    unwatched: usize,
    overflows: u64,
    buffer: Vec<u8>,
}

impl Watcher {
    /// `budget` caps how many directories may be watched at once.
    ///
    /// The cap exists so that a runaway tree degrades in a way we can report, instead of
    /// failing with `ENOSPC` somewhere deep in a walk and leaving the operator guessing.
    pub fn new(budget: usize) -> Result<Self, WatchError> {
        let inotify = Inotify::init().map_err(WatchError::Init)?;
        let watches = inotify.watches();
        Ok(Self {
            inotify,
            watches,
            by_descriptor: HashMap::new(),
            by_path: HashMap::new(),
            budget,
            unwatched: 0,
            overflows: 0,
            // 4 KiB is enough for a handful of events; 64 KiB keeps a `git checkout` from
            // overflowing the queue between reads, which is the realistic burst.
            buffer: vec![0; 64 * 1024],
        })
    }

    pub fn health(&self) -> WatchHealth {
        WatchHealth {
            watched_dirs: self.by_descriptor.len(),
            budget: self.budget,
            degraded: self.unwatched > 0,
            unwatched: self.unwatched,
            overflows: self.overflows,
        }
    }

    pub fn is_watching(&self, path: &Path) -> bool {
        self.by_path.contains_key(path)
    }

    /// Watch a single directory.
    pub fn watch_dir(&mut self, path: &Path) -> Result<bool, WatchError> {
        if self.by_path.contains_key(path) {
            return Ok(true);
        }
        if self.by_descriptor.len() >= self.budget {
            self.unwatched += 1;
            return Ok(false);
        }

        match self.watches.add(path, watch_mask()) {
            Ok(descriptor) => {
                self.by_descriptor.insert(descriptor.clone(), path.to_path_buf());
                self.by_path.insert(path.to_path_buf(), descriptor);
                Ok(true)
            }
            // A directory that vanished between the walk and the watch is ordinary, not an
            // error worth failing a whole tree over.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) if e.raw_os_error() == Some(28) => {
                // ENOSPC: the kernel's own watch limit, below our budget. Same degradation.
                self.unwatched += 1;
                tracing::warn!(
                    path = %path.display(),
                    "inotify watch limit reached; raise fs.inotify.max_user_watches"
                );
                Ok(false)
            }
            Err(source) => Err(WatchError::Add {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Watch a directory and everything beneath it, skipping what the deny-list excludes.
    ///
    /// Skipping is what makes the watch count affordable: a source tree with `node_modules`,
    /// `.git` and `target` contains an order of magnitude more directories than the part
    /// anybody wants to find.
    /// **The walk stops at a network mount**, for the same reason the reconcile walk does: a
    /// share is somebody else's disk, and an inotify watch on a FUSE mount watches nothing
    /// anyway — the kernel has no events to give for a change that happened on another machine.
    pub fn watch_tree(&mut self, root: &Path) -> Result<usize, WatchError> {
        let barriers = crate::reconcile::network_barriers(root);
        let mut added = 0;
        let mut queue = vec![root.to_path_buf()];

        while let Some(dir) = queue.pop() {
            if self.watch_dir(&dir)? {
                added += 1;
            }

            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name();
                if denylist::should_skip_subtree(&name.to_string_lossy()) {
                    continue;
                }
                let path = entry.path();
                if barriers.contains(&path) {
                    continue;
                }
                queue.push(path);
            }
        }

        Ok(added)
    }

    pub fn unwatch(&mut self, path: &Path) {
        if let Some(descriptor) = self.by_path.remove(path) {
            self.by_descriptor.remove(&descriptor);
            // The watch may already be gone if the directory was deleted; the kernel removes
            // it and sends IN_IGNORED, so a failure here is expected and uninteresting.
            let _ = self.watches.remove(descriptor);
        }
    }

    /// Read whatever the kernel has for us.
    ///
    /// Non-blocking: returns an empty vector when there is nothing. The caller decides how to
    /// wait, which keeps this usable from both a thread and an async runtime.
    pub fn read_events(&mut self) -> Result<Vec<RawEvent>, WatchError> {
        let events = match self.inotify.read_events(&mut self.buffer) {
            Ok(events) => events,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(Vec::new()),
            Err(e) => return Err(WatchError::Read(e)),
        };

        let mut out = Vec::new();
        let mut to_forget = Vec::new();

        for event in events {
            if event.mask.contains(EventMask::Q_OVERFLOW) {
                self.overflows += 1;
                tracing::warn!("inotify queue overflowed; a full reconcile is required");
                out.push(RawEvent {
                    path: PathBuf::new(),
                    kind: RawKind::Overflow,
                    is_dir: false,
                });
                continue;
            }

            // The kernel has stopped watching this descriptor, so our bookkeeping must follow.
            // Leaking descriptors here would slowly consume the watch budget for directories
            // that no longer exist.
            if event.mask.contains(EventMask::IGNORED) {
                to_forget.push(event.wd.clone());
                continue;
            }

            let Some(dir) = self.by_descriptor.get(&event.wd) else {
                continue;
            };
            let path = match event.name {
                Some(name) => dir.join(name),
                None => dir.clone(),
            };
            let is_dir = event.mask.contains(EventMask::ISDIR);

            let kind = if event.mask.contains(EventMask::CREATE) {
                RawKind::Created
            } else if event.mask.contains(EventMask::CLOSE_WRITE) {
                RawKind::Written
            } else if event.mask.contains(EventMask::DELETE)
                || event.mask.contains(EventMask::DELETE_SELF)
            {
                RawKind::Deleted
            } else if event.mask.contains(EventMask::MOVED_FROM) {
                RawKind::MovedFrom(event.cookie)
            } else if event.mask.contains(EventMask::MOVED_TO) {
                RawKind::MovedTo(event.cookie)
            } else if event.mask.contains(EventMask::ATTRIB) {
                RawKind::AttributeChanged
            } else {
                continue;
            };

            out.push(RawEvent { path, kind, is_dir });
        }

        for descriptor in to_forget {
            if let Some(path) = self.by_descriptor.remove(&descriptor) {
                self.by_path.remove(&path);
            }
        }

        Ok(out)
    }

    /// The raw file descriptor, for callers that want to poll it rather than spin.
    #[cfg(unix)]
    pub fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        self.inotify.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Poll until `f` returns something or the deadline passes.
    ///
    /// Filesystem notifications are inherently asynchronous, so a test either polls with a
    /// timeout or is flaky. Polling with a generous ceiling is the honest option — the tests
    /// finish in milliseconds in practice and only approach the ceiling when something is wrong.
    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(value) = f() {
                return Some(value);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    fn collect_until(watcher: &mut Watcher, predicate: impl Fn(&[RawEvent]) -> bool) -> Vec<RawEvent> {
        let mut seen = Vec::new();
        wait_for(|| {
            seen.extend(watcher.read_events().unwrap());
            predicate(&seen).then_some(())
        });
        seen
    }

    #[test]
    fn a_new_file_is_noticed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();

        std::fs::write(tmp.path().join("new.txt"), b"hello").unwrap();

        let events = collect_until(&mut watcher, |e| {
            e.iter().any(|e| e.kind == RawKind::Created)
        });
        assert!(
            events.iter().any(|e| e.path.ends_with("new.txt")),
            "creation was not reported: {events:?}"
        );
    }

    #[test]
    fn a_write_is_reported_once_the_writer_closes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.txt");
        std::fs::write(&path, b"first").unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();
        let _ = watcher.read_events();

        std::fs::write(&path, b"second").unwrap();

        let events = collect_until(&mut watcher, |e| {
            e.iter().any(|e| e.kind == RawKind::Written)
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == RawKind::Written && e.path.ends_with("a.txt")),
            "write was not reported: {events:?}"
        );
    }

    #[test]
    fn a_deletion_is_noticed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("doomed.txt");
        std::fs::write(&path, b"x").unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();
        let _ = watcher.read_events();

        std::fs::remove_file(&path).unwrap();

        let events = collect_until(&mut watcher, |e| {
            e.iter().any(|e| e.kind == RawKind::Deleted)
        });
        assert!(
            events.iter().any(|e| e.path.ends_with("doomed.txt")),
            "deletion was not reported: {events:?}"
        );
    }

    #[test]
    fn a_rename_arrives_as_a_cookie_pair() {
        // This is what makes move detection exact rather than inferred.
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("before.txt");
        let to = tmp.path().join("after.txt");
        std::fs::write(&from, b"x").unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();
        let _ = watcher.read_events();

        std::fs::rename(&from, &to).unwrap();

        let events = collect_until(&mut watcher, |e| {
            e.iter().any(|e| matches!(e.kind, RawKind::MovedFrom(_)))
                && e.iter().any(|e| matches!(e.kind, RawKind::MovedTo(_)))
        });

        let moved_from = events
            .iter()
            .find_map(|e| match e.kind {
                RawKind::MovedFrom(c) => Some(c),
                _ => None,
            })
            .expect("no MovedFrom");
        let moved_to = events
            .iter()
            .find_map(|e| match e.kind {
                RawKind::MovedTo(c) => Some(c),
                _ => None,
            })
            .expect("no MovedTo");

        assert_eq!(moved_from, moved_to, "the two halves must share a cookie");
    }

    #[test]
    fn a_new_directory_is_flagged_as_one() {
        // The caller needs this to know whether to place a watch on it.
        let tmp = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();

        std::fs::create_dir(tmp.path().join("sub")).unwrap();

        let events = collect_until(&mut watcher, |e| e.iter().any(|e| e.is_dir));
        assert!(
            events.iter().any(|e| e.is_dir && e.path.ends_with("sub")),
            "directory creation was not flagged: {events:?}"
        );
    }

    #[test]
    fn watching_a_tree_skips_dependency_and_build_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_tree(root).unwrap();

        assert!(watcher.is_watching(root));
        assert!(watcher.is_watching(&root.join("src")));
        assert!(watcher.is_watching(&root.join("src/deep")));

        for skipped in ["node_modules", "target", ".git"] {
            assert!(
                !watcher.is_watching(&root.join(skipped)),
                "{skipped} should not consume a watch"
            );
        }
    }

    #[test]
    fn the_watch_budget_is_enforced_and_reported() {
        // Exhaustion must be visible rather than silent: an unwatched subtree looks exactly
        // like a subtree where nothing ever changes.
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::create_dir_all(tmp.path().join(format!("dir{i}"))).unwrap();
        }

        let mut watcher = Watcher::new(5).unwrap();
        watcher.watch_tree(tmp.path()).unwrap();

        let health = watcher.health();
        assert_eq!(health.watched_dirs, 5, "the budget must be a hard cap");
        assert!(health.degraded, "exceeding the budget must be reported");
        assert!(health.unwatched > 0);
    }

    #[test]
    fn watching_the_same_directory_twice_costs_one_watch() {
        let tmp = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(1000).unwrap();

        assert!(watcher.watch_dir(tmp.path()).unwrap());
        assert!(watcher.watch_dir(tmp.path()).unwrap());
        assert_eq!(watcher.health().watched_dirs, 1);
    }

    #[test]
    fn watching_a_missing_directory_is_not_an_error() {
        // A directory that vanishes between the walk and the watch is ordinary during a
        // `git checkout` and must not abort the surrounding tree walk.
        let mut watcher = Watcher::new(1000).unwrap();
        assert!(!watcher.watch_dir(Path::new("/definitely/not/here")).unwrap());
        assert_eq!(watcher.health().watched_dirs, 0);
    }

    #[test]
    fn unwatching_releases_the_watch() {
        let tmp = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();
        assert_eq!(watcher.health().watched_dirs, 1);

        watcher.unwatch(tmp.path());
        assert_eq!(watcher.health().watched_dirs, 0);
        assert!(!watcher.is_watching(tmp.path()));
    }

    #[test]
    fn reading_with_nothing_pending_returns_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_dir(tmp.path()).unwrap();

        let started = Instant::now();
        let events = watcher.read_events().unwrap();
        assert!(events.is_empty());
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "read_events must not block"
        );
    }

    #[test]
    fn renaming_a_watched_directory_reports_a_move_and_never_a_deletion() {
        // Regression. The watched directory also has its own watch, and subscribing to
        // IN_MOVE_SELF meant a rename produced a spurious deletion alongside the real move
        // pair. The coalescer lets a deletion supersede a pending rename, so the net effect
        // was that renaming a directory erased its whole subtree from the index.
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("before");
        let to = tmp.path().join("after");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("inside.txt"), b"x").unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_tree(tmp.path()).unwrap();
        let _ = watcher.read_events();

        std::fs::rename(&from, &to).unwrap();

        let events = collect_until(&mut watcher, |e| {
            e.iter().any(|e| matches!(e.kind, RawKind::MovedTo(_)))
        });

        assert!(
            events.iter().any(|e| matches!(e.kind, RawKind::MovedFrom(_))),
            "the move pair is missing: {events:?}"
        );
        assert!(
            !events.iter().any(|e| e.kind == RawKind::Deleted),
            "a rename must never look like a deletion: {events:?}"
        );
    }

    #[test]
    fn a_deleted_directory_stops_consuming_its_watch() {
        // The kernel sends IN_IGNORED; if we did not act on it, the budget would slowly fill
        // with directories that no longer exist.
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("temporary");
        std::fs::create_dir(&sub).unwrap();

        let mut watcher = Watcher::new(1000).unwrap();
        watcher.watch_tree(tmp.path()).unwrap();
        assert_eq!(watcher.health().watched_dirs, 2);

        std::fs::remove_dir(&sub).unwrap();

        let cleaned = wait_for(|| {
            let _ = watcher.read_events().unwrap();
            (watcher.health().watched_dirs == 1).then_some(())
        });
        assert!(
            cleaned.is_some(),
            "the watch for a deleted directory was not released"
        );
    }
}
