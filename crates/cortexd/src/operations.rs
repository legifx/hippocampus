//! Work that takes long enough to need a handle.
//!
//! Everything else the daemon does answers in milliseconds and is therefore a call. Copying is
//! not: it can take a minute, and during that minute a person needs to see it moving and be able
//! to stop it. So a transfer answers immediately with an id, reports through notifications, and
//! ends with one final message however it ended.
//!
//! **Progress is throttled here rather than at the source.** The copy reports every chunk,
//! which is thousands of times a second on a fast disk; forwarding all of them would spend more
//! time serialising notifications than moving bytes, and no display can show them. Ten a second
//! is more than a person can read and less than a socket notices.

use cortex_agent::transfer::{Handle, Progress};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often progress reaches the client.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// The transfers currently running, so they can be cancelled by id.
#[derive(Default)]
pub struct Operations {
    next_id: AtomicU64,
    running: Mutex<HashMap<u64, Handle>>,
}

impl Operations {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new operation and return its id and its handle.
    pub fn start(&self) -> (u64, Handle) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = Handle::new();
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, handle.clone());
        (id, handle)
    }

    /// Stop one. Returns false for an id that is not running, which is not an error: a person
    /// who pressed cancel as the last file finished has not done anything wrong.
    pub fn cancel(&self, id: u64) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
        {
            Some(handle) => {
                handle.cancel();
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, id: u64) {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
    }

    pub fn running_count(&self) -> usize {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

/// Rate-limits progress so that a thousand chunks a second become ten messages.
///
/// The first report always passes — a transfer that showed nothing for its first tenth of a
/// second would look like it had not started — and so does the last, which the caller sends
/// explicitly.
pub struct Throttle {
    last: Option<Instant>,
    interval: Duration,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }

    pub fn should_send(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.duration_since(last) < self.interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// Turn a transfer's progress into what the wire carries.
pub fn to_wire(operation: u64, progress: &Progress) -> cortex_core::tools::TransferProgress {
    cortex_core::tools::TransferProgress {
        operation,
        files_done: progress.files_done,
        bytes_done: progress.bytes_done,
        total_files: progress.total.files,
        total_bytes: progress.total.bytes,
        fraction: progress.fraction(),
        current: progress.current.clone(),
    }
}

/// Shared so a spawned transfer can reach the connection that asked for it.
pub type Notifier = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_cancelling_an_unknown_one_is_not_an_error() {
        let operations = Operations::new();
        let (first, _) = operations.start();
        let (second, _) = operations.start();
        assert_ne!(first, second);

        assert!(operations.cancel(first));
        assert!(
            !operations.cancel(9999),
            "a person who pressed cancel as the last file finished has done nothing wrong"
        );
    }

    #[test]
    fn cancelling_reaches_the_handle_the_transfer_is_watching() {
        let operations = Operations::new();
        let (id, handle) = operations.start();
        assert!(!handle.is_cancelled());

        operations.cancel(id);
        assert!(handle.is_cancelled());
    }

    #[test]
    fn a_finished_operation_stops_being_cancellable() {
        let operations = Operations::new();
        let (id, _) = operations.start();
        assert_eq!(operations.running_count(), 1);

        operations.finish(id);
        assert_eq!(operations.running_count(), 0);
        assert!(!operations.cancel(id));
    }

    #[test]
    fn the_first_report_always_gets_through() {
        let mut throttle = Throttle::new(Duration::from_millis(100));
        // A transfer that showed nothing for its first tenth of a second would look stalled.
        assert!(throttle.should_send(Instant::now()));
    }

    #[test]
    fn a_thousand_chunks_a_second_become_a_handful_of_messages() {
        let mut throttle = Throttle::new(Duration::from_millis(100));
        let start = Instant::now();

        let mut sent = 0;
        for chunk in 0..1000 {
            // A thousand chunks spread over one second.
            let now = start + Duration::from_micros(chunk * 1000);
            if throttle.should_send(now) {
                sent += 1;
            }
        }

        assert!(
            (10..=11).contains(&sent),
            "one second at ten per second should be about ten messages, not {sent}"
        );
    }
}
