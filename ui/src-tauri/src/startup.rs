//! The cold-start timer.
//!
//! M4 §1 asks for "cold start to interactive < 300 ms, measured with a real timer in the real
//! app". A benchmark that starts the app in a loop would measure a warm page cache and a warm
//! compositor, so this measures the app itself, every run, and reports the number to whoever
//! asks — including the frontend, which shows it in the command palette.
//!
//! The clock starts as early as `run()` can start it, which is after the process is already
//! mapped and the dynamic loader has finished. That is not a fudge: the milestone's concern is
//! the part this project controls, and it is stated here rather than hidden.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct Startup {
    began: Instant,
    setup_ms: Arc<AtomicU64>,
    connected_ms: Arc<AtomicU64>,
    interactive_ms: Arc<AtomicU64>,
}

/// "Not yet", stored in an atomic that has to hold a duration otherwise.
const UNSET: u64 = u64::MAX;

/// What the timer knows. `None` means it has not happened yet.
///
/// `Option` rather than a zero sentinel: a start fast enough to round to zero milliseconds is
/// the *best* case, and reporting it as "never became interactive" was the first bug this
/// module's tests found.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// Milliseconds from process start to Tauri handing control to this application. This is
    /// the toolkit's own startup and nothing in this repository can shorten it.
    pub setup_ms: Option<u64>,
    /// Milliseconds from process start to a usable connection to the daemon.
    pub connected_ms: Option<u64>,
    /// Milliseconds to the first frame a person could act on.
    pub interactive_ms: Option<u64>,
    /// The budget this is measured against, so the frontend does not hard-code it.
    pub budget_ms: u64,
    pub within_budget: bool,
}

/// M4's benchmark table: GUI cold start → interactive.
pub const BUDGET_MS: u64 = 300;

impl Startup {
    pub fn begin() -> Self {
        Self {
            began: Instant::now(),
            setup_ms: Arc::new(AtomicU64::new(UNSET)),
            connected_ms: Arc::new(AtomicU64::new(UNSET)),
            interactive_ms: Arc::new(AtomicU64::new(UNSET)),
        }
    }

    /// When Tauri handed control back to this application.
    ///
    /// Everything before it is the toolkit starting: GTK, the webview process, the plugin
    /// registry. Knowing where that line falls is the difference between "the cold start is too
    /// slow" and "the cold start is too slow and there is nothing in our code to fix".
    pub fn mark_setup(&self) -> u64 {
        let elapsed = self.began.elapsed().as_millis() as u64;
        self.setup_ms.store(elapsed, Ordering::Relaxed);
        elapsed
    }

    pub fn mark_connected(&self) -> u64 {
        let elapsed = self.began.elapsed().as_millis() as u64;
        self.connected_ms.store(elapsed, Ordering::Relaxed);
        elapsed
    }

    /// Idempotent: the frontend may re-render, but there is only one first frame.
    pub fn mark_interactive(&self) -> u64 {
        let elapsed = self.began.elapsed().as_millis() as u64;
        let _ = self
            .interactive_ms
            .compare_exchange(UNSET, elapsed, Ordering::Relaxed, Ordering::Relaxed);
        self.interactive_ms.load(Ordering::Relaxed)
    }

    pub fn report(&self) -> Report {
        let read = |value: &AtomicU64| match value.load(Ordering::Relaxed) {
            UNSET => None,
            ms => Some(ms),
        };
        let interactive = read(&self.interactive_ms);
        Report {
            setup_ms: read(&self.setup_ms),
            connected_ms: read(&self.connected_ms),
            interactive_ms: interactive,
            budget_ms: BUDGET_MS,
            // A run that has not reached interactive yet is not "within budget" — it has no
            // number at all, and reporting `true` for it would make the gate meaningless.
            within_budget: interactive.is_some_and(|ms| ms <= BUDGET_MS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_that_never_became_interactive_is_not_within_budget() {
        let startup = Startup::begin();
        let report = startup.report();
        assert_eq!(report.interactive_ms, None);
        assert!(!report.within_budget, "an unfinished run is not a fast one");
    }

    #[test]
    fn a_start_too_fast_to_measure_is_still_a_start() {
        // The case that broke the first version: `mark_interactive` returning zero milliseconds
        // is the best possible outcome, and a zero sentinel read it as "never happened".
        let startup = Startup::begin();
        assert_eq!(startup.mark_interactive(), 0);
        assert_eq!(startup.report().interactive_ms, Some(0));
        assert!(startup.report().within_budget);
    }

    #[test]
    fn the_first_interactive_mark_is_the_one_that_counts() {
        let startup = Startup::begin();
        let first = startup.mark_interactive();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let second = startup.mark_interactive();
        assert_eq!(first, second, "a re-render must not restate the cold start");
    }

    #[test]
    fn the_budget_is_the_one_the_milestone_names() {
        assert_eq!(BUDGET_MS, 300);
        let startup = Startup::begin();
        startup.mark_interactive();
        assert!(startup.report().within_budget);
    }
}
