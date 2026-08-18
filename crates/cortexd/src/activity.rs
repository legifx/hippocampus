//! Is somebody using this machine right now?
//!
//! The extraction stage needs the answer, because its whole budget depends on it. The obvious
//! implementations are all wrong for this daemon:
//!
//! - Reading input devices means `/dev/input` access, which is a privilege this daemon must
//!   never need, and it sees keystrokes — including passwords — which it must never see.
//! - Asking the compositor means becoming a Wayland client, and a headless daemon that requires
//!   a session bus to throttle correctly is a daemon that misbehaves on a server.
//! - Inferring activity from filesystem events confuses a background download with a person.
//!
//! So activity is **signalled, not sniffed**: a client that knows a human is present says so.
//! The GUI will ping on interaction; every interactive request already counts, because a search
//! arriving over the socket is proof that somebody typed it.
//!
//! The consequence of nobody signalling is deliberate: a daemon with no interactive client
//! treats the machine as idle and uses the idle budget. That is the correct default for a
//! headless install, and any interactive client corrects it the moment it connects.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

/// The last moment a human was known to be present.
#[derive(Debug)]
pub struct Activity {
    /// Unix milliseconds. Zero means "never", which reads as idle since the epoch.
    last: AtomicI64,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            last: AtomicI64::new(0),
        }
    }
}

impl Activity {
    /// Record that a human just did something.
    pub fn ping(&self) {
        self.record(now_millis());
    }

    /// Record activity at an explicit instant. Tests use this; nothing else should.
    pub fn record(&self, unix_millis: i64) {
        // Never move the marker backwards: two clients pinging out of order must not make the
        // daemon think activity is older than it is.
        self.last.fetch_max(unix_millis, Ordering::Relaxed);
    }

    /// How long since the last signal. Saturates rather than going negative when the clock moves.
    pub fn idle_for(&self) -> Duration {
        self.idle_for_at(now_millis())
    }

    pub fn idle_for_at(&self, now_millis: i64) -> Duration {
        let last = self.last.load(Ordering::Relaxed);
        if last <= 0 {
            // Never signalled. Report a long idle rather than "just now", so a fresh daemon
            // with no GUI does its indexing at the idle budget instead of crawling.
            return Duration::from_secs(u32::MAX as u64);
        }
        Duration::from_millis(now_millis.saturating_sub(last).max(0) as u64)
    }

    pub fn is_active(&self, idle_after: Duration) -> bool {
        self.idle_for() < idle_after
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_nobody_has_spoken_to_counts_as_idle() {
        let activity = Activity::default();
        assert!(!activity.is_active(Duration::from_secs(120)));
    }

    #[test]
    fn a_ping_makes_it_active_until_the_threshold_passes() {
        let activity = Activity::default();
        activity.record(1_000_000);
        assert!(activity.idle_for_at(1_000_000).is_zero());
        assert_eq!(activity.idle_for_at(1_060_000), Duration::from_secs(60));
    }

    #[test]
    fn an_out_of_order_ping_cannot_make_activity_look_older() {
        // Two clients, two clocks, one socket. Taking the later of the two is the only
        // behaviour that cannot be exploited into throttling the daemon by accident.
        let activity = Activity::default();
        activity.record(2_000_000);
        activity.record(1_000_000);
        assert!(activity.idle_for_at(2_000_000).is_zero());
    }

    #[test]
    fn a_clock_that_jumps_backwards_does_not_produce_a_negative_idle_time() {
        let activity = Activity::default();
        activity.record(5_000_000);
        assert_eq!(activity.idle_for_at(4_000_000), Duration::ZERO);
    }
}
