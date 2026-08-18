//! The mandatory dry run.
//!
//! A gated tool called without a plan id does not mutate anything: it returns a plan. Calling
//! again with that id applies it, once. This is the "mandatory dry-run that returns a plan
//! which must be confirmed" of rule R6, and the confirmation is the second call.
//!
//! Two properties make it worth more than a confirmation flag:
//!
//! - **The plan is checked against the filesystem when it is applied.** Each step records what
//!   was at the target when the plan was made; if that changed, the plan is stale and refused.
//!   An approval given for "create a file here" cannot silently become "overwrite the file
//!   somebody put there in the meantime".
//! - **Plans expire.** An agent that stockpiles approvals and uses them an hour later is
//!   acting on a filesystem it has not looked at.
//!
//! Plans live in memory only. A plan that survived a restart would describe a filesystem the
//! daemon has not seen since, which is exactly the situation the staleness check exists to
//! catch — so there is nothing to gain by persisting them and a false sense of continuity to
//! lose.

use crate::policy::{Decision, Refusal};
use cortex_core::tools::{Plan, PlanStep};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// How long a plan stays valid.
pub const DEFAULT_TTL_SECS: i64 = 120;

/// A plan plus the bookkeeping the wire format does not carry.
#[derive(Debug, Clone)]
pub struct StoredPlan {
    pub plan: Plan,
    pub actor: String,
    pub created: i64,
    /// One fingerprint per step, in step order.
    pub fingerprints: Vec<Fingerprint>,
}

/// What was at a path when the plan was made. Size and mtime rather than a hash: reading every
/// target twice to plan a move would make planning cost what the move costs, and the question
/// being asked is "did this change since I looked", which mtime answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fingerprint {
    Absent,
    File { size: u64, mtime: i64 },
    Dir,
    Other,
}

impl Fingerprint {
    pub fn of(path: &Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Err(_) => Fingerprint::Absent,
            Ok(md) if md.is_dir() => Fingerprint::Dir,
            Ok(md) if md.is_file() => Fingerprint::File {
                size: md.len(),
                mtime: mtime_of(&md),
            },
            Ok(_) => Fingerprint::Other,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Fingerprint::Absent => "nothing".into(),
            Fingerprint::File { size, .. } => format!("a file of {size} bytes"),
            Fingerprint::Dir => "a directory".into(),
            Fingerprint::Other => "something that is neither a file nor a directory".into(),
        }
    }
}

pub fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The daemon's open plans.
pub struct PlanBook {
    plans: Mutex<HashMap<String, StoredPlan>>,
    counter: AtomicU64,
    ttl_secs: i64,
}

impl Default for PlanBook {
    fn default() -> Self {
        Self::new(DEFAULT_TTL_SECS)
    }
}

impl PlanBook {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
            ttl_secs,
        }
    }

    pub fn ttl_secs(&self) -> i64 {
        self.ttl_secs
    }

    /// Register a plan and return it with its id filled in.
    ///
    /// Ids are a counter, not a secret. The socket is `0600` and a plan is bound to the actor
    /// that made it; an unguessable id would defend against an attacker who is already inside
    /// the only trust boundary there is.
    pub fn register(&self, mut plan: Plan, actor: &str, fingerprints: Vec<Fingerprint>, now: i64) -> Plan {
        let id = format!("plan-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        plan.id = id.clone();
        plan.expires_in_secs = self.ttl_secs;

        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        plans.retain(|_, stored| now - stored.created < self.ttl_secs);
        plans.insert(
            id,
            StoredPlan {
                plan: plan.clone(),
                actor: actor.to_string(),
                created: now,
                fingerprints,
            },
        );
        plan
    }

    /// Consume a plan. It is removed whether or not it turns out to be applicable, so a stale
    /// plan cannot be retried until the filesystem happens to match it again.
    pub fn take(&self, id: &str, actor: &str, now: i64) -> Decision<StoredPlan> {
        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        let stored = plans.remove(id).ok_or(Refusal::UnknownPlan)?;

        if stored.actor != actor {
            // Not "wrong actor": that would tell one caller that another caller's plan exists.
            return Err(Refusal::UnknownPlan);
        }
        if now - stored.created >= self.ttl_secs {
            return Err(Refusal::PlanStale {
                detail: format!("the plan expired {} seconds ago", now - stored.created - self.ttl_secs),
            });
        }
        Ok(stored)
    }

    /// Check a taken plan against the filesystem as it is now.
    pub fn verify(stored: &StoredPlan) -> Decision<()> {
        for (step, expected) in stored.plan.steps.iter().zip(&stored.fingerprints) {
            let target = step.from.as_ref().unwrap_or(&step.path);
            let actual = Fingerprint::of(target);
            if &actual != expected {
                return Err(Refusal::PlanStale {
                    detail: format!(
                        "{} held {} when the plan was made and holds {} now",
                        target.display(),
                        expected.describe(),
                        actual.describe()
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn open_plans(&self) -> usize {
        self.plans.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

/// Build the sentences a human reads before approving.
pub fn summarize(steps: &[PlanStep]) -> Vec<String> {
    cortex_core::tools::describe_steps(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::tools::{PlanAction, PlanStep};

    fn plan_for(path: &Path) -> Plan {
        Plan {
            id: String::new(),
            tool: "fs.write".into(),
            steps: vec![PlanStep {
                action: PlanAction::Create,
                path: path.to_path_buf(),
                from: None,
                replaces: None,
            }],
            expires_in_secs: 0,
            summary: vec![],
        }
    }

    #[test]
    fn a_plan_can_only_be_used_once() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let book = PlanBook::new(60);

        let plan = book.register(plan_for(&target), "agent:t", vec![Fingerprint::Absent], 100);
        assert!(book.take(&plan.id, "agent:t", 100).is_ok());
        assert_eq!(book.take(&plan.id, "agent:t", 100).unwrap_err(), Refusal::UnknownPlan);
    }

    #[test]
    fn another_actors_plan_is_simply_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let book = PlanBook::new(60);
        let plan = book.register(plan_for(&target), "agent:one", vec![Fingerprint::Absent], 0);
        assert_eq!(book.take(&plan.id, "agent:two", 0).unwrap_err(), Refusal::UnknownPlan);
    }

    #[test]
    fn plans_expire() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let book = PlanBook::new(60);
        let plan = book.register(plan_for(&target), "agent:t", vec![Fingerprint::Absent], 0);
        assert!(matches!(
            book.take(&plan.id, "agent:t", 61),
            Err(Refusal::PlanStale { .. })
        ));
    }

    #[test]
    fn a_target_that_appeared_in_the_meantime_makes_the_plan_stale() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let book = PlanBook::new(60);
        let plan = book.register(plan_for(&target), "agent:t", vec![Fingerprint::Absent], 0);

        // Somebody else got there first.
        std::fs::write(&target, "not yours").unwrap();

        let stored = book.take(&plan.id, "agent:t", 1).unwrap();
        assert!(matches!(
            PlanBook::verify(&stored),
            Err(Refusal::PlanStale { .. })
        ));
    }

    #[test]
    fn an_unchanged_target_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, "hello").unwrap();
        let book = PlanBook::new(60);
        let plan = book.register(plan_for(&target), "agent:t", vec![Fingerprint::of(&target)], 0);
        let stored = book.take(&plan.id, "agent:t", 1).unwrap();
        assert_eq!(PlanBook::verify(&stored), Ok(()));
    }

    #[test]
    fn registering_purges_what_has_expired() {
        let dir = tempfile::tempdir().unwrap();
        let book = PlanBook::new(60);
        book.register(plan_for(&dir.path().join("a")), "agent:t", vec![Fingerprint::Absent], 0);
        assert_eq!(book.open_plans(), 1);
        book.register(plan_for(&dir.path().join("b")), "agent:t", vec![Fingerprint::Absent], 500);
        assert_eq!(book.open_plans(), 1, "the first one is long gone");
    }
}
