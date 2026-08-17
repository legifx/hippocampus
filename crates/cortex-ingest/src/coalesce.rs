//! Event coalescing and rename pairing.
//!
//! Editors do not save a file once. They write a temporary file, rename it over the target,
//! adjust its permissions and touch it again — three to five events for what the user
//! experienced as pressing Ctrl+S. Feeding all of them into the ingest pipeline means doing
//! the expensive work several times for one edit.
//!
//! This module is deliberately free of any filesystem or clock access: time is a parameter.
//! That makes every rule here testable exactly, rather than approximately with sleeps —
//! and coalescing bugs are precisely the kind that only appear under timing you cannot
//! reproduce on demand.

use crate::event::{Change, EventClass, RawEvent, RawKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A rename whose other half has not arrived yet.
#[derive(Debug, Clone)]
struct PendingMove {
    path: PathBuf,
    is_dir: bool,
    seen_at: Instant,
}

#[derive(Debug, Clone)]
struct Pending {
    event: RawEvent,
    /// When this entry may be emitted. Refreshed on every further event of the same class,
    /// so a file being written continuously does not get re-indexed mid-write.
    ready_at: Instant,
    /// When the first event of this burst arrived. Bounds how long refreshing can defer
    /// emission — see `max_defer`.
    first_seen: Instant,
}

pub struct Coalescer {
    window: Duration,
    max_defer: Duration,
    move_window: Duration,
    pending: HashMap<(PathBuf, EventClass), Pending>,
    moves: HashMap<u32, PendingMove>,
    /// Destination of a paired rename, held until its source event leaves the window.
    rename_targets: HashMap<PathBuf, PathBuf>,
    resync_requested: bool,
}

impl Coalescer {
    /// `window` is the quiet period a path must have before its event is emitted.
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            // A file being appended to continuously — a log, a download in progress — would
            // otherwise refresh its deadline forever and never be indexed at all. After this
            // long, emit regardless and let the next burst produce another event.
            max_defer: window * 20,
            // A rename's two halves are emitted by the kernel back to back. If the other half
            // has not arrived within this, it is not coming: the file was moved across a
            // boundary we do not watch.
            move_window: window,
            pending: HashMap::new(),
            moves: HashMap::new(),
            rename_targets: HashMap::new(),
            resync_requested: false,
        }
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len() + self.moves.len()
    }

    /// Take one raw event in.
    pub fn push(&mut self, event: RawEvent, now: Instant) {
        match event.kind {
            RawKind::Overflow => {
                // Nothing that is buffered can be trusted once the kernel has dropped events.
                self.resync_requested = true;
                self.pending.clear();
                self.moves.clear();
                self.rename_targets.clear();
                return;
            }

            RawKind::MovedFrom(cookie) => {
                self.moves.insert(
                    cookie,
                    PendingMove {
                        path: event.path,
                        is_dir: event.is_dir,
                        seen_at: now,
                    },
                );
                return;
            }

            RawKind::MovedTo(cookie) => {
                if let Some(from) = self.moves.remove(&cookie) {
                    // Both halves seen: this is a rename, and identity is preserved.
                    self.insert_pending(
                        RawEvent {
                            path: from.path,
                            kind: RawKind::MovedFrom(cookie),
                            is_dir: from.is_dir,
                        },
                        now,
                        Some(event.path),
                    );
                    return;
                }
                // Only the destination was seen, so the file arrived from outside the watched
                // tree. That is an ordinary creation as far as we are concerned.
                self.insert_pending(
                    RawEvent {
                        path: event.path,
                        kind: RawKind::Created,
                        is_dir: event.is_dir,
                    },
                    now,
                    None,
                );
                return;
            }

            RawKind::Deleted => {
                // A delete supersedes anything buffered for the same path: re-indexing a file
                // that has since been removed is wasted work at best and a phantom entry at
                // worst.
                self.pending
                    .retain(|(path, _), _| path != &event.path);
            }

            _ => {}
        }

        self.insert_pending(event, now, None);
    }

    fn insert_pending(&mut self, event: RawEvent, now: Instant, rename_target: Option<PathBuf>) {
        let class = event.kind.class();
        let key = (event.path.clone(), class);

        // A rename's destination rides along in the map value so that pairing survives the
        // coalescing window.
        if let Some(target) = rename_target {
            self.rename_targets.insert(event.path.clone(), target);
        }

        match self.pending.get_mut(&key) {
            Some(existing) => {
                existing.event = event;
                existing.ready_at = now + self.window;
                // Never defer past the hard ceiling, however busy the path is.
                let ceiling = existing.first_seen + self.max_defer;
                if existing.ready_at > ceiling {
                    existing.ready_at = ceiling;
                }
            }
            None => {
                self.pending.insert(
                    key,
                    Pending {
                        event,
                        ready_at: now + self.window,
                        first_seen: now,
                    },
                );
            }
        }
    }

    /// Emit everything whose quiet period has elapsed.
    pub fn drain_ready(&mut self, now: Instant) -> Vec<Change> {
        let mut out = Vec::new();

        if self.resync_requested {
            self.resync_requested = false;
            out.push(Change::ResyncRequired);
            return out;
        }

        // A half-rename that never found its partner means the file left the watched tree.
        let expired: Vec<u32> = self
            .moves
            .iter()
            .filter(|(_, m)| now.duration_since(m.seen_at) >= self.move_window)
            .map(|(cookie, _)| *cookie)
            .collect();
        for cookie in expired {
            if let Some(m) = self.moves.remove(&cookie) {
                out.push(Change::Remove { path: m.path });
            }
        }

        let ready: Vec<(PathBuf, EventClass)> = self
            .pending
            .iter()
            .filter(|(_, p)| p.ready_at <= now)
            .map(|(key, _)| key.clone())
            .collect();

        for key in ready {
            let Some(pending) = self.pending.remove(&key) else {
                continue;
            };
            let event = pending.event;
            match event.kind {
                RawKind::Created | RawKind::Written => out.push(Change::Upsert {
                    path: event.path,
                    is_dir: event.is_dir,
                }),
                RawKind::AttributeChanged => out.push(Change::Touch { path: event.path }),
                RawKind::Deleted => out.push(Change::Remove { path: event.path }),
                RawKind::MovedFrom(_) => match self.rename_targets.remove(&event.path) {
                    Some(to) => out.push(Change::Rename {
                        from: event.path,
                        to,
                    }),
                    // The pairing was lost somewhere; treating it as a removal is the safe
                    // reading, and the reconcile scan will restore the destination.
                    None => out.push(Change::Remove { path: event.path }),
                },
                RawKind::MovedTo(_) | RawKind::Overflow => {}
            }
        }

        out
    }

    /// When the caller should next wake up, if anything is buffered.
    ///
    /// Lets the event loop sleep exactly as long as it needs to instead of polling.
    pub fn next_deadline(&self) -> Option<Instant> {
        let pending = self.pending.values().map(|p| p.ready_at);
        let moves = self.moves.values().map(|m| m.seen_at + self.move_window);
        pending.chain(moves).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, kind: RawKind) -> RawEvent {
        RawEvent {
            path: PathBuf::from(path),
            kind,
            is_dir: false,
        }
    }

    fn dir(path: &str, kind: RawKind) -> RawEvent {
        RawEvent {
            path: PathBuf::from(path),
            kind,
            is_dir: true,
        }
    }

    fn coalescer() -> (Coalescer, Instant) {
        (Coalescer::new(Duration::from_millis(500)), Instant::now())
    }

    #[test]
    fn an_event_is_not_emitted_before_its_window_elapses() {
        let (mut c, t0) = coalescer();
        c.push(file("/a.txt", RawKind::Written), t0);

        assert!(c.drain_ready(t0).is_empty());
        assert!(c.drain_ready(t0 + Duration::from_millis(499)).is_empty());
        assert_eq!(c.drain_ready(t0 + Duration::from_millis(500)).len(), 1);
    }

    #[test]
    fn a_burst_of_writes_becomes_one_change() {
        // The case this module exists for: one Ctrl+S, five kernel events.
        let (mut c, t0) = coalescer();
        for i in 0..5 {
            c.push(file("/a.txt", RawKind::Written), t0 + Duration::from_millis(i * 20));
        }

        let changes = c.drain_ready(t0 + Duration::from_millis(700));
        assert_eq!(changes.len(), 1, "a save burst must not produce five re-indexes");
        assert_eq!(
            changes[0],
            Change::Upsert {
                path: PathBuf::from("/a.txt"),
                is_dir: false
            }
        );
    }

    #[test]
    fn each_further_event_defers_emission() {
        let (mut c, t0) = coalescer();
        c.push(file("/a.txt", RawKind::Written), t0);
        // Still being written at 400 ms, so the deadline moves to 900 ms.
        c.push(file("/a.txt", RawKind::Written), t0 + Duration::from_millis(400));

        assert!(
            c.drain_ready(t0 + Duration::from_millis(600)).is_empty(),
            "a file still being written must not be indexed mid-write"
        );
        assert_eq!(c.drain_ready(t0 + Duration::from_millis(900)).len(), 1);
    }

    #[test]
    fn continuous_writing_cannot_defer_forever() {
        // A log file or an in-progress download refreshes its deadline on every append. Without
        // a ceiling it would never be indexed at all.
        let (mut c, t0) = coalescer();
        let mut now = t0;
        for _ in 0..100 {
            c.push(file("/growing.log", RawKind::Written), now);
            now += Duration::from_millis(100);
        }

        let changes = c.drain_ready(now);
        assert_eq!(changes.len(), 1, "a continuously written file must eventually be emitted");
    }

    #[test]
    fn different_paths_do_not_coalesce_into_each_other() {
        let (mut c, t0) = coalescer();
        c.push(file("/a.txt", RawKind::Written), t0);
        c.push(file("/b.txt", RawKind::Written), t0);

        assert_eq!(c.drain_ready(t0 + Duration::from_millis(600)).len(), 2);
    }

    #[test]
    fn different_classes_on_one_path_are_kept_apart() {
        // A permission change and a content change are different facts about the same file.
        let (mut c, t0) = coalescer();
        c.push(file("/a.txt", RawKind::Written), t0);
        c.push(file("/a.txt", RawKind::AttributeChanged), t0);

        let mut changes = c.drain_ready(t0 + Duration::from_millis(600));
        changes.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn a_rename_is_paired_by_its_cookie() {
        let (mut c, t0) = coalescer();
        c.push(file("/old.txt", RawKind::MovedFrom(42)), t0);
        c.push(file("/new.txt", RawKind::MovedTo(42)), t0);

        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(
            changes,
            vec![Change::Rename {
                from: PathBuf::from("/old.txt"),
                to: PathBuf::from("/new.txt"),
            }],
            "a rename must preserve identity, not become a delete plus a create"
        );
    }

    #[test]
    fn concurrent_renames_do_not_cross_wires() {
        // Two renames in flight at once must pair by cookie, not by arrival order.
        let (mut c, t0) = coalescer();
        c.push(file("/a-old", RawKind::MovedFrom(1)), t0);
        c.push(file("/b-old", RawKind::MovedFrom(2)), t0);
        c.push(file("/b-new", RawKind::MovedTo(2)), t0);
        c.push(file("/a-new", RawKind::MovedTo(1)), t0);

        let mut changes = c.drain_ready(t0 + Duration::from_millis(600));
        changes.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(
            changes,
            vec![
                Change::Rename {
                    from: PathBuf::from("/a-old"),
                    to: PathBuf::from("/a-new")
                },
                Change::Rename {
                    from: PathBuf::from("/b-old"),
                    to: PathBuf::from("/b-new")
                },
            ]
        );
    }

    #[test]
    fn a_move_out_of_the_watched_tree_becomes_a_removal() {
        let (mut c, t0) = coalescer();
        c.push(file("/watched/gone.txt", RawKind::MovedFrom(7)), t0);

        // The partner never arrives, because the destination is not watched.
        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(
            changes,
            vec![Change::Remove {
                path: PathBuf::from("/watched/gone.txt")
            }]
        );
    }

    #[test]
    fn a_move_into_the_watched_tree_becomes_a_creation() {
        let (mut c, t0) = coalescer();
        c.push(file("/watched/arrived.txt", RawKind::MovedTo(9)), t0);

        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(
            changes,
            vec![Change::Upsert {
                path: PathBuf::from("/watched/arrived.txt"),
                is_dir: false
            }]
        );
    }

    #[test]
    fn a_delete_supersedes_a_buffered_write() {
        // Re-indexing a file that has since been deleted is wasted work and leaves a phantom.
        let (mut c, t0) = coalescer();
        c.push(file("/tmp.txt", RawKind::Written), t0);
        c.push(file("/tmp.txt", RawKind::Deleted), t0 + Duration::from_millis(50));

        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(
            changes,
            vec![Change::Remove {
                path: PathBuf::from("/tmp.txt")
            }]
        );
    }

    #[test]
    fn an_overflow_discards_everything_and_demands_a_resync() {
        // Once the kernel drops events, nothing buffered can be trusted and the index may be
        // wrong anywhere. A full reconcile is the only correct answer.
        let (mut c, t0) = coalescer();
        c.push(file("/a.txt", RawKind::Written), t0);
        c.push(file("/b.txt", RawKind::Written), t0);
        c.push(
            RawEvent {
                path: PathBuf::new(),
                kind: RawKind::Overflow,
                is_dir: false,
            },
            t0,
        );

        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(changes, vec![Change::ResyncRequired]);
        assert_eq!(c.pending_count(), 0);
    }

    #[test]
    fn directory_events_keep_their_directory_flag() {
        let (mut c, t0) = coalescer();
        c.push(dir("/newdir", RawKind::Created), t0);

        let changes = c.drain_ready(t0 + Duration::from_millis(600));
        assert_eq!(
            changes,
            vec![Change::Upsert {
                path: PathBuf::from("/newdir"),
                is_dir: true
            }],
            "the watcher needs this flag to know whether to place a new watch"
        );
    }

    #[test]
    fn the_next_deadline_lets_the_caller_sleep_exactly_long_enough() {
        let (mut c, t0) = coalescer();
        assert!(c.next_deadline().is_none(), "nothing buffered, nothing to wait for");

        c.push(file("/a.txt", RawKind::Written), t0);
        assert_eq!(c.next_deadline(), Some(t0 + Duration::from_millis(500)));

        c.drain_ready(t0 + Duration::from_millis(600));
        assert!(c.next_deadline().is_none());
    }

    #[test]
    fn draining_an_empty_coalescer_is_harmless() {
        let (mut c, t0) = coalescer();
        assert!(c.drain_ready(t0).is_empty());
        assert_eq!(c.pending_count(), 0);
    }
}
