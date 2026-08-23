//! Which tier a node belongs in — the mechanism by which the system forgets.
//!
//! Two asymmetries here are deliberate and both come from what a mistake costs.
//!
//! **Promotion is immediate, demotion is one step per pass.** Touching a file puts it in the
//! Working Set at once, because a memory that took a week to notice what you are doing would
//! be useless. Falling out happens a tier at a time, so a single quiet fortnight cannot send
//! a document from Working to Cold — recovering from over-eager forgetting costs a re-index
//! and, worse, teaches the owner not to trust the tiering.
//!
//! **Pins are immune.** Not "weighted heavily": immune. A pinned node is never demoted by this
//! function at all, whatever its salience says.

use cortex_core::model::Tier;
use serde::{Deserialize, Serialize};

/// Where the tier boundaries sit, in salience units.
///
/// The defaults are stated against [`crate::salience::Weights::default`], whose ordinary
/// ceiling is 4.8. They are thresholds on a bounded scale rather than quantiles on purpose:
/// a quantile-based tiering always demotes something, even on an index where everything is
/// genuinely in use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TierPolicy {
    /// At or above this, and touched inside the working window: Working Set.
    pub working: f64,
    /// At or above this: short-term memory, fully indexed.
    pub stm: f64,
    /// At or above this: long-term memory, still locally searchable.
    pub ltm: f64,
    /// How recently a node must have been touched to be in the Working Set at all, in seconds.
    pub working_window_secs: i64,
    /// A node this new is left in STM regardless of score: it has not had a chance to earn one.
    pub grace_secs: i64,
}

impl Default for TierPolicy {
    fn default() -> Self {
        Self {
            working: 1.6,
            stm: 0.55,
            ltm: 0.15,
            working_window_secs: 4 * 60 * 60,
            grace_secs: 7 * 86_400,
        }
    }
}

/// One step down from `tier`, or `tier` itself if it is already the bottom.
fn one_step_down(tier: Tier) -> Tier {
    match tier {
        Tier::Working => Tier::Stm,
        Tier::Stm => Tier::Ltm,
        Tier::Ltm | Tier::Cold => Tier::Cold,
    }
}

/// Where a node should be after this consolidation pass.
///
/// `last_access` is Unix seconds, `0` when no episode has ever named the node.
pub fn assign_tier(
    current: Tier,
    salience: f64,
    pinned: bool,
    last_access: i64,
    first_seen: i64,
    now: i64,
    policy: &TierPolicy,
) -> Tier {
    // A pin is not a large number. It is an exemption.
    if pinned {
        return if is_working(salience, last_access, now, policy) {
            Tier::Working
        } else {
            current.min(Tier::Stm)
        };
    }

    // Too new to judge. Demoting a file the watcher indexed an hour ago would be forgetting
    // something before ever having had it.
    if now - first_seen < policy.grace_secs {
        return current.min(Tier::Stm);
    }

    let deserved = if is_working(salience, last_access, now, policy) {
        Tier::Working
    } else if salience >= policy.stm {
        Tier::Stm
    } else if salience >= policy.ltm {
        Tier::Ltm
    } else {
        Tier::Cold
    };

    // `Tier` orders Working < Stm < Ltm < Cold, so "better" is "less".
    if deserved < current {
        deserved
    } else if deserved > current {
        one_step_down(current)
    } else {
        current
    }
}

fn is_working(salience: f64, last_access: i64, now: i64, policy: &TierPolicy) -> bool {
    salience >= policy.working && last_access > 0 && now - last_access <= policy.working_window_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    const OLD_ENOUGH: i64 = NOW - 400 * 86_400;

    fn policy() -> TierPolicy {
        TierPolicy::default()
    }

    #[test]
    fn a_touched_and_salient_node_is_in_the_working_set_at_once() {
        let t = assign_tier(Tier::Cold, 3.0, false, NOW - 60, OLD_ENOUGH, NOW, &policy());
        assert_eq!(t, Tier::Working);
    }

    #[test]
    fn a_forgotten_node_falls_one_tier_per_pass_not_to_the_bottom() {
        let p = policy();
        let mut tier = Tier::Working;
        let mut steps = 0;
        while tier != Tier::Cold {
            tier = assign_tier(tier, 0.0, false, 0, OLD_ENOUGH, NOW, &p);
            steps += 1;
            assert!(steps < 10, "tiering did not converge");
        }
        assert_eq!(steps, 3, "Working → Stm → Ltm → Cold, one pass each");
    }

    #[test]
    fn a_pinned_node_is_never_demoted_however_dead_it_looks() {
        let p = policy();
        let mut tier = Tier::Stm;
        for _ in 0..50 {
            tier = assign_tier(tier, 0.0, true, 0, OLD_ENOUGH, NOW, &p);
        }
        assert_eq!(tier, Tier::Stm);
    }

    #[test]
    fn a_pinned_node_still_rises_into_the_working_set() {
        let t = assign_tier(Tier::Ltm, 11.0, true, NOW - 5, OLD_ENOUGH, NOW, &policy());
        assert_eq!(t, Tier::Working);
    }

    #[test]
    fn a_node_indexed_yesterday_is_left_alone() {
        let t = assign_tier(Tier::Stm, 0.0, false, 0, NOW - 86_400, NOW, &policy());
        assert_eq!(t, Tier::Stm);
    }

    #[test]
    fn salience_without_a_recent_touch_is_not_the_working_set() {
        let t = assign_tier(Tier::Stm, 4.0, false, NOW - 5 * 86_400, OLD_ENOUGH, NOW, &policy());
        assert_eq!(t, Tier::Stm, "high salience, but not what the human is doing now");
    }

    #[test]
    fn tiering_is_idempotent_once_it_has_settled() {
        let p = policy();
        let settled = assign_tier(Tier::Ltm, 0.3, false, NOW - 90 * 86_400, OLD_ENOUGH, NOW, &p);
        assert_eq!(settled, Tier::Ltm);
        assert_eq!(
            assign_tier(settled, 0.3, false, NOW - 90 * 86_400, OLD_ENOUGH, NOW, &p),
            Tier::Ltm
        );
    }
}
