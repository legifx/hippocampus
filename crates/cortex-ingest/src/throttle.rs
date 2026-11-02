//! Keeping background work from being felt.
//!
//! Extraction is the "replay" stage in the biological analogy: expensive consolidation that
//! belongs to rest, not to the moment. The build prompt states the constraint plainly — a first
//! index run must never make the machine feel slow — and a first index run over a large home
//! directory is minutes of continuous reading and hashing.
//!
//! The mechanism is a duty cycle rather than a scheduler priority. `nice` alone is not enough:
//! it yields CPU but does nothing about page cache pressure or disk queue depth, and the user
//! notices those first. Working for a bounded slice and then standing back for a computed pause
//! bounds all three, and it does so with arithmetic that can be tested exactly rather than with
//! a measurement that can only be observed.
//!
//! Nothing here reads a clock. The caller passes in how long it worked, which is what makes the
//! rules testable without sleeping — the same reason the coalescer takes its `now` as an
//! argument.

use std::time::Duration;

/// How long a work slice may run before the stage yields.
///
/// Short enough that a pause request is acted on promptly, long enough that the pause
/// arithmetic is not dominated by the cost of stopping and starting.
pub const SLICE: Duration = Duration::from_millis(200);

/// Longest single pause. A larger computed pause is split into several, so that a stop request
/// is never waiting behind one long sleep.
pub const MAX_PAUSE: Duration = Duration::from_secs(5);

/// How long to stand back after working for `worked`, to hold an average CPU share of `share`.
///
/// Working `w` and pausing `p` gives a share of `w / (w + p)`, so `p = w * (1 - share) / share`.
///
/// The share is of **one core**, because the stage is single-threaded. On a sixteen-thread
/// machine a share of 0.15 is therefore about one per cent of the machine, which is the
/// intended magnitude: this work is meant to be invisible, not merely polite. A stage whose
/// slice keeps several cores busy divides its share first, with [`share_per_worker`].
pub fn pause_after(worked: Duration, share: f64) -> Duration {
    // A share at or above one means "no throttling asked for"; anything at or below zero means
    // the work is not allowed to proceed at all, and the caller should be waiting, not working.
    if share >= 1.0 {
        return Duration::ZERO;
    }
    if share <= 0.0 {
        return MAX_PAUSE;
    }

    pause_after_uncapped(worked, share).min(MAX_PAUSE)
}

/// [`pause_after`] without the [`MAX_PAUSE`] cap, for a caller whose sleep wakes for stop requests
/// on its own. A stage whose slice can be longer than [`SLICE`] needs it — one inference call is one
/// slice: capped, a 1.5-second slice at a share of 0.0375 rests 5 s instead of 38 s, and the share
/// actually spent is six times the configured one (D129).
pub fn pause_after_uncapped(worked: Duration, share: f64) -> Duration {
    if share >= 1.0 {
        return Duration::ZERO;
    }
    if share <= 0.0 {
        return MAX_PAUSE;
    }
    worked.mul_f64((1.0 - share) / share)
}

/// The share that applies right now.
///
/// Two numbers rather than one because the difference between them is the entire point: the
/// machine has spare capacity precisely when nobody is using it, and a background stage that
/// ignores that either wastes the idle time or intrudes on the busy time.
pub fn share_for(active: bool, active_share: f64, idle_share: f64) -> f64 {
    if active {
        active_share
    } else {
        idle_share
    }
}

/// The share to pass to [`pause_after`] when a slice keeps `workers` cores busy at once (D22, D129).
///
/// The pause arithmetic measures wall time. An embedder on eight threads costs eight times the CPU
/// of the time it takes, so an undivided share of 0.8 held eight cores 80 % of the time instead of
/// one. Divided, the total is back where the configuration put it: 0.8 over four workers is 0.2 of
/// the wall clock, which is 0.8 of one core. A share of one or more still means "no throttling", and
/// zero workers count as one.
pub fn share_per_worker(share: f64, workers: usize) -> f64 {
    if share >= 1.0 {
        return share;
    }
    share / workers.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_slice_on_four_cores_holds_a_quarter_of_the_share_each() {
        assert_eq!(share_per_worker(0.8, 4), 0.2);
        // 100 ms on four cores is 400 ms of CPU; at 0.8 of one core that needs 400 ms of pause.
        assert_eq!(pause_after(millis(100), share_per_worker(0.8, 4)), millis(400));
    }

    #[test]
    fn a_long_slice_rests_its_whole_pause_when_the_caller_can_wait() {
        let share = share_per_worker(0.15, 4);
        assert_eq!(pause_after(millis(1500), share), MAX_PAUSE);
        let full = pause_after_uncapped(millis(1500), share);
        assert!(full > Duration::from_secs(38) && full < Duration::from_secs(39), "{full:?}");
        assert_eq!(pause_after_uncapped(millis(100), 1.0), Duration::ZERO);
    }

    #[test]
    fn no_throttling_stays_no_throttling_and_zero_workers_count_as_one() {
        assert_eq!(share_per_worker(1.0, 8), 1.0);
        assert_eq!(pause_after(millis(100), share_per_worker(1.0, 8)), Duration::ZERO);
        assert_eq!(share_per_worker(0.15, 0), 0.15);
        assert_eq!(share_per_worker(0.15, 1), 0.15);
    }

    #[test]
    fn half_a_share_pauses_for_as_long_as_it_worked() {
        assert_eq!(pause_after(millis(200), 0.5), millis(200));
    }

    #[test]
    fn the_default_active_share_costs_roughly_six_times_the_work_in_waiting() {
        // 0.15 is the configured default while the user is at the keyboard.
        let pause = pause_after(millis(200), 0.15);
        assert!(
            (pause.as_millis() as i64 - 1133).abs() <= 2,
            "expected about 1133 ms, got {pause:?}"
        );
    }

    #[test]
    fn a_full_share_never_pauses() {
        assert_eq!(pause_after(millis(200), 1.0), Duration::ZERO);
        assert_eq!(pause_after(millis(200), 2.0), Duration::ZERO);
    }

    #[test]
    fn a_zero_share_is_a_full_stop_rather_than_a_division_by_zero() {
        // A misconfigured share of zero must not produce an infinite or NaN duration, which
        // `Duration::mul_f64` would panic on.
        assert_eq!(pause_after(millis(200), 0.0), MAX_PAUSE);
        assert_eq!(pause_after(millis(200), -1.0), MAX_PAUSE);
    }

    #[test]
    fn a_single_pause_is_bounded_so_that_stopping_stays_responsive() {
        assert_eq!(pause_after(Duration::from_secs(60), 0.15), MAX_PAUSE);
    }

    #[test]
    fn zero_work_earns_no_pause() {
        assert_eq!(pause_after(Duration::ZERO, 0.15), Duration::ZERO);
    }

    #[test]
    fn activity_selects_the_stricter_share() {
        assert_eq!(share_for(true, 0.15, 0.80), 0.15);
        assert_eq!(share_for(false, 0.15, 0.80), 0.80);
    }

    #[test]
    fn the_measured_share_matches_the_requested_one() {
        // The arithmetic restated as the property it is meant to have, because the formula is
        // easy to invert by accident and the symptom — a background stage using six times its
        // budget — is not obvious in a profile.
        for share in [0.05, 0.15, 0.5, 0.8, 0.95] {
            let worked = millis(100);
            let pause = pause_after(worked, share);
            let measured = worked.as_secs_f64() / (worked + pause).as_secs_f64();
            assert!(
                (measured - share).abs() < 1e-9,
                "share {share} produced {measured}"
            );
        }
    }
}
