//! When to tell the daemon. Pure, so the rules are tested without a compositor or a clock.

use std::time::{Duration, Instant};

/// How often to ping while somebody is there. Well inside the daemon's `idle_after_secs`
/// (120 s by default), so one lost ping never makes a person at the keyboard look absent.
pub const PING_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct Presence {
    idle: bool,
    last_ping: Option<Instant>,
}

impl Presence {
    /// What the compositor said last. Coming back from idle pings at once: the daemon may have
    /// switched to its idle budget, and the person who just touched the mouse is feeling it.
    pub fn set_idle(&mut self, idle: bool) {
        if self.idle && !idle {
            self.last_ping = None;
        }
        self.idle = idle;
    }

    pub fn due(&self, now: Instant) -> bool {
        !self.idle
            && self
                .last_ping
                .is_none_or(|at| now.saturating_duration_since(at) >= PING_EVERY)
    }

    pub fn pinged(&mut self, now: Instant) {
        self.last_ping = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_pings_at_once() {
        // The notification starts in the "not idle" state, which is how the protocol defines it.
        assert!(Presence::default().due(Instant::now()));
    }

    #[test]
    fn while_somebody_is_there_it_pings_every_thirty_seconds() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.pinged(t0);
        assert!(!p.due(t0 + Duration::from_secs(29)));
        assert!(p.due(t0 + PING_EVERY));
    }

    #[test]
    fn nobody_there_means_no_pings() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.set_idle(true);
        assert!(!p.due(t0));
        assert!(!p.due(t0 + Duration::from_secs(3600)));
    }

    #[test]
    fn coming_back_pings_without_waiting_for_the_interval() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.pinged(t0);
        p.set_idle(true);
        p.set_idle(false);
        assert!(p.due(t0 + Duration::from_secs(5)));
    }
}
