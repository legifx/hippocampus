//! What a node is worth remembering, and why.
//!
//! The formula is the one in `docs/01-BUILD-PROMPT.md` §M5, with one constraint added that the
//! specification implies but does not state: **every component is bounded before it is
//! weighted.** Without that, a file opened ten thousand times by a build script outranks
//! everything a human ever cared about, and the weights stop being comparable to each other —
//! `w1` would mean "per log-access" while `w5` meant "once". Bounded components make the
//! weights a budget: the numbers in [`Weights`] are the most each signal may ever contribute.

use serde::{Deserialize, Serialize};

/// Seconds in a day. Salience is a slow quantity; nothing here is measured in minutes.
pub const DAY: f64 = 86_400.0;

/// How many effective accesses count as "a lot". The access component reaches 1.0 here and
/// cannot pass it, so a runaway script cannot buy rank.
const ACCESS_REFERENCE: f64 = 50.0;

/// How many effective edits make a node "actively worked on". Three, weighted by age.
const EDIT_REFERENCE: f64 = 3.0;

/// How much each signal may contribute at most.
///
/// These are a budget, not a scale: because every component is bounded to `[0, 1]` before
/// weighting, the weight *is* the maximum contribution. The defaults are deliberately
/// conservative except for the pin, which is meant to be unanswerable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weights {
    /// w1 — how often it has been touched at all.
    pub access: f64,
    /// w2 — how recently. The largest of the ordinary signals, because a memory that ignores
    /// recency is an archive.
    pub recency: f64,
    /// w3 — how often it is *changed*, which is a different claim from being read.
    pub edits: f64,
    /// w4 — how connected it is to the rest of the graph.
    pub centrality: f64,
    /// w5 — an explicit pin. Dominant on purpose: a pin must beat every inference the system
    /// can make, or it is advice rather than an instruction.
    pub pin: f64,
    /// w6 — being inside a project the human is currently working in.
    pub project: f64,
    /// w7 — build output, caches, download debris. Subtracted.
    pub noise: f64,
    /// w8 — what a memory has *earned*, from outcome reports (M11, D76).
    ///
    /// Zero for every file, because a file cannot be reported useful; this is the one component
    /// that exists for the memory layer, and it is here rather than in a scoring engine of its
    /// own because a second engine is exactly what D74 refuses. Signed on the way in: a memory
    /// reported as noise carries a negative usefulness and this term subtracts.
    ///
    /// Smaller than `recency` on purpose. A memory that has proved useful should outrank one
    /// that has not, and it should not outrank *being about what is happening right now* — the
    /// point of a context pack is the task in front of the agent, not a hall of fame.
    pub usefulness: f64,
    /// Half-life of the recency component, in days.
    pub recency_half_life_days: f64,
    /// Half-life of stored salience between consolidations, in days. This is the λ in
    /// `salience(t) = base · e^(−λ·Δt) + Σ(reinforcements)`.
    pub decay_half_life_days: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            access: 1.0,
            recency: 1.5,
            edits: 1.0,
            centrality: 0.8,
            pin: 10.0,
            project: 0.5,
            noise: 2.0,
            usefulness: 1.0,
            recency_half_life_days: 14.0,
            decay_half_life_days: 30.0,
        }
    }
}

impl Weights {
    /// The most an unpinned node can score. Used to normalise for display, never for ranking.
    pub fn ordinary_ceiling(&self) -> f64 {
        self.access + self.recency + self.edits + self.centrality + self.project
    }
}

/// Everything the score is computed from. All of it comes from `episodes`, `edges` and the
/// node's own row — nothing here requires reading a file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    /// Episodes of any kind naming this node, each weighted by its age — see
    /// [`effective_count`]. A raw count would never decay, and a node that was opened once in
    /// 2019 would keep a floor of salience for ever; the first version of this did exactly
    /// that, and nothing in a thirty-day simulation ever reached the Cold tier.
    pub access_count: f64,
    /// The same, over the episodes that changed it: `create`, `modify`, `move`.
    pub edit_count: f64,
    /// Unix seconds of the most recent episode naming it. `0` for a node no episode mentions.
    pub last_access: i64,
    /// Unix seconds of the first episode naming it, or of first indexing.
    pub first_seen: i64,
    /// Evaluation time, Unix seconds.
    pub now: i64,
    /// Number of edges at this node, both directions.
    pub degree: u32,
    /// The degree that counts as well-connected in this index — the 95th percentile, computed
    /// once per consolidation. Never zero when passed in; guarded here anyway.
    pub degree_reference: u32,
    pub pinned: bool,
    /// Inside a zone the human has touched recently.
    pub in_active_project: bool,
    /// `0.0` for ordinary content, `1.0` for certain debris. See [`noise_score`].
    pub noise: f64,
    /// What a memory has earned from outcome reports (M11). Zero for a file, always.
    ///
    /// Signed and unbounded on the way in — it is a running sum of weighted reports — and
    /// squashed into `[-1, 1]` by [`score`], because an unbounded term would let a memory with
    /// forty helpful reports outrank a pin.
    pub usefulness: f64,
}

impl Inputs {
    /// A node nothing is known about beyond its existence.
    pub fn unseen(now: i64) -> Self {
        Self {
            access_count: 0.0,
            edit_count: 0.0,
            last_access: 0,
            first_seen: now,
            now,
            degree: 0,
            degree_reference: 1,
            pinned: false,
            in_active_project: false,
            noise: 0.0,
            usefulness: 0.0,
        }
    }
}

/// The score, broken into the parts it was made of.
///
/// `total` is the sum of the other fields and there is a test that says so. This is what
/// `salience_log.components_json` stores and what `brain.explain` prints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Components {
    pub access: f64,
    pub recency: f64,
    pub edits: f64,
    pub centrality: f64,
    pub pin: f64,
    pub project: f64,
    /// Negative or zero. Stored signed so that the fields sum to `total` without the reader
    /// having to remember which one is subtracted.
    pub noise: f64,
    /// Signed: positive for a memory that has been reported useful, negative for one reported
    /// as noise or retrieved repeatedly without ever being mentioned (M11, D76). Always zero
    /// for a file.
    #[serde(default)]
    pub usefulness: f64,
    pub total: f64,
}

impl Components {
    /// The sum of the parts, recomputed. Used by the test that guards the invariant, and by
    /// anything that wants to check a stored breakdown it did not compute itself.
    pub fn sum(&self) -> f64 {
        self.access
            + self.recency
            + self.edits
            + self.centrality
            + self.pin
            + self.project
            + self.noise
            + self.usefulness
    }
}

/// Half-life to the λ of `e^(−λ·t)`.
fn lambda_from_half_life(days: f64) -> f64 {
    if days <= 0.0 {
        return f64::INFINITY;
    }
    std::f64::consts::LN_2 / (days * DAY)
}

/// Score one node.
///
/// Bounded components, then weights. A pinned node also keeps its ordinary components: a pin
/// says "never forget this", not "stop measuring it", and the components still explain where
/// the rest of the score came from.
pub fn score(input: &Inputs, w: &Weights) -> Components {
    let access_unit = if input.access_count <= 0.0 {
        0.0
    } else {
        (input.access_count.ln_1p() / ACCESS_REFERENCE.ln_1p()).min(1.0)
    };

    let recency_unit = if input.last_access <= 0 {
        0.0
    } else {
        let elapsed = (input.now - input.last_access).max(0) as f64;
        (-lambda_from_half_life(w.recency_half_life_days) * elapsed).exp()
    };

    // Effective edits, squashed into `[0, 1)`. The count is already age-weighted, so this does
    // not divide by a lifetime as well: doing both would apply decay twice and make an old file
    // that is still being worked on score below a new one that is not.
    let edit_unit = 1.0 - (-input.edit_count / EDIT_REFERENCE).exp();

    let centrality_unit = if input.degree == 0 {
        0.0
    } else {
        (f64::from(input.degree) / f64::from(input.degree_reference.max(1))).min(1.0)
    };

    let noise_unit = input.noise.clamp(0.0, 1.0);

    // Squashed rather than clamped, so that the difference between one report and two still
    // registers while forty and forty-one do not. `tanh` at this scale reaches 0.76 at one
    // human-confirmed report and 0.96 at two.
    let usefulness_unit = (input.usefulness / USEFULNESS_REFERENCE).tanh();

    let components = Components {
        access: w.access * access_unit,
        recency: w.recency * recency_unit,
        edits: w.edits * edit_unit,
        centrality: w.centrality * centrality_unit,
        pin: if input.pinned { w.pin } else { 0.0 },
        project: if input.in_active_project { w.project } else { 0.0 },
        noise: -w.noise * noise_unit,
        usefulness: w.usefulness * usefulness_unit,
        total: 0.0,
    };
    Components {
        total: components.sum(),
        ..components
    }
}

/// The most an ordinary node can score from a single fresh touch and nothing else.
///
/// One access, this instant, no edit, no edges, no active project. **This is the number every
/// tier threshold has to be compared against**, and comparing thresholds against reachable
/// maxima — rather than only testing individual cases — is the only way to find a class of bug
/// that a formula's own unit tests cannot: the formula is correct, and the system is
/// nevertheless dead.
///
/// The lesson is not ours. A second-brain daemon in daily use set its demotion threshold to
/// exactly the weight of its recency term, which is precisely the score an untouched note
/// reaches — so every unused note sat on the edge by construction and its short memory ran
/// empty at one note out of eighty. Nothing was miscalculated. The two numbers had simply never
/// been held up against each other.
pub fn single_touch_ceiling(w: &Weights) -> f64 {
    let access = 1.0_f64.ln_1p() / ACCESS_REFERENCE.ln_1p();
    w.access * access + w.recency
}

/// The scale at which earned usefulness saturates.
///
/// One human-confirmed helpful report is `+1.0`, so this reaching 0.76 of the weight at a single
/// report is deliberate: the first person to say a memory was useful should move it noticeably,
/// and the fortieth should not move it at all.
const USEFULNESS_REFERENCE: f64 = 1.0;

/// `salience(t) = base · e^(−λ·Δt) + Σ(reinforcements)` — the specification's decay law.
///
/// Applied once per gap between reinforcements by [`effective_count`], which is where the law
/// actually does its work. Pinned nodes never reach it, because a pin that decayed would be a
/// pin with an expiry date.
pub fn decay(base: f64, elapsed_secs: i64, reinforcements: f64, w: &Weights) -> f64 {
    let elapsed = elapsed_secs.max(0) as f64;
    base * (-lambda_from_half_life(w.decay_half_life_days) * elapsed).exp() + reinforcements
}

/// Fold a node's episodes into a count that forgets.
///
/// Each episode is one reinforcement of size 1, and the running total decays between them by
/// the law above. So a file touched fifty times last year and never since scores close to
/// nothing, while one touched three times this week scores close to three — which is what
/// "how much is this being used" has to mean in a memory that forgets.
///
/// `timestamps` must be ascending. Anything after `now` is ignored rather than treated as
/// negative elapsed time: a filesystem with a clock skew should not be able to invent salience.
pub fn effective_count(timestamps: impl IntoIterator<Item = i64>, now: i64, w: &Weights) -> f64 {
    let mut total = 0.0;
    let mut last: Option<i64> = None;
    for ts in timestamps {
        if ts > now {
            continue;
        }
        total = match last {
            Some(previous) => decay(total, ts - previous, 1.0, w),
            None => 1.0,
        };
        last = Some(ts);
    }
    match last {
        Some(previous) => decay(total, now - previous, 0.0, w),
        None => 0.0,
    }
}

/// How much of a path looks like something no human will ever ask for.
///
/// Deliberately a score rather than a flag, and deliberately about the *path* rather than the
/// content: this runs over a hundred thousand nodes in a consolidation pass and must not open
/// anything. The strongest signal wins rather than accumulating, so a vendored directory
/// inside a cache does not score 2.0.
pub fn noise_score(path: &str) -> f64 {
    /// Segments that mean "this was produced, not written".
    const GENERATED: [&str; 14] = [
        "node_modules", "target", "build", "dist", ".next", "__pycache__", ".venv", "venv",
        ".pytest_cache", ".ruff_cache", ".mypy_cache", ".gradle", "vendor", "site-packages",
    ];
    /// Segments that mean "a program put this here for itself".
    const CACHE: [&str; 6] = [".cache", ".npm", ".cargo", ".rustup", "Trash", ".thumbnails"];
    /// Segments that mean "this is a copy of something else".
    ///
    /// Found the expensive way, in somebody else's working system: a knowledge base whose search
    /// was `grep -r` over the whole tree returned three copies of the same note — from three
    /// generations of backup — above the canonical one, because nothing ranked and nothing knew
    /// that a backup is not a document. A copy answering for its original is worse than no
    /// answer: it is stale, it looks authoritative, and there are several of it.
    const COPIES: [&str; 8] = [
        "backup", "backups", ".backup", "snapshot", "snapshots", ".snapshots", "old", "archive",
    ];

    let mut worst: f64 = 0.0;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if GENERATED.contains(&segment) {
            worst = worst.max(1.0);
        } else if CACHE.contains(&segment) {
            worst = worst.max(0.9);
        } else if COPIES.contains(&segment.to_lowercase().as_str()) {
            worst = worst.max(0.85);
        }
    }

    // Compiled output and lock files: real files a human occasionally wants, so penalised but
    // not written off.
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some((_, ext)) = name.rsplit_once('.') {
        if matches!(ext, "o" | "a" | "so" | "rlib" | "pyc" | "class" | "d" | "rmeta") {
            worst = worst.max(0.8);
        }
    }
    if matches!(name, "package-lock.json" | "Cargo.lock" | "uv.lock" | "yarn.lock" | "pnpm-lock.yaml") {
        worst = worst.max(0.5);
    }
    worst
}

#[cfg(test)]
mod tests {

    /// Adding the memory layer's component did not move a single file's score.
    ///
    /// `usefulness` is zero for everything that is not a memory — a file cannot be reported
    /// useful — so every file's total is what it was before `w8` existed. Worth an assertion
    /// rather than an argument, because the recorded goldset numbers depend on file ranking
    /// being stable, and "it should be zero" is how a ranking quietly moves.
    #[test]
    fn a_file_scores_exactly_what_it_scored_before_the_memory_component_existed() {
        let w = Weights::default();
        let mut input = Inputs {
            access_count: 4.0,
            edit_count: 2.0,
            last_access: 1_000_000,
            first_seen: 900_000,
            now: 1_000_600,
            degree: 12,
            degree_reference: 20,
            pinned: false,
            in_active_project: true,
            noise: 0.0,
            usefulness: 0.0,
        };
        let before = score(&input, &w);
        assert_eq!(before.usefulness, 0.0);
        assert_eq!(before.total, before.sum());

        // The same node with the component's weight set to anything at all, still zero, because
        // the *input* is zero and a file's input always is.
        let heavier = Weights {
            usefulness: 100.0,
            ..w
        };
        assert_eq!(score(&input, &heavier).total, before.total);

        // And a memory with the same history and one earned report scores strictly higher.
        input.usefulness = 1.0;
        assert!(score(&input, &w).total > before.total);
    }

    /// Every component reaches the total.
    ///
    /// `Components::sum()` lists its fields by hand, so adding a component and forgetting the
    /// one line that adds it up produces a score that is silently missing a term — which is
    /// precisely what happened when `usefulness` arrived: the field was set, printed and
    /// stored, and contributed nothing. Nothing failed, and two memories with different
    /// histories scored identically to sixteen decimal places.
    ///
    /// Distinct powers of two, so that a missing field is not only detected but *named* by the
    /// difference.
    #[test]
    fn every_component_is_part_of_the_total() {
        let parts = Components {
            access: 1.0,
            recency: 2.0,
            edits: 4.0,
            centrality: 8.0,
            pin: 16.0,
            project: 32.0,
            noise: 64.0,
            usefulness: 128.0,
            total: 0.0,
        };
        let missing = 255.0 - parts.sum();
        assert_eq!(
            missing, 0.0,
            "a component worth {missing} is set but never added up — the powers of two say \
             which: 1 access, 2 recency, 4 edits, 8 centrality, 16 pin, 32 project, 64 noise, \
             128 usefulness"
        );
    }


    use super::*;

    fn at(now: i64) -> Inputs {
        Inputs::unseen(now)
    }

    #[test]
    fn components_sum_to_the_total() {
        let w = Weights::default();
        let mut input = at(1_000_000);
        input.access_count = 7.0;
        input.edit_count = 3.0;
        input.last_access = 1_000_000 - 3 * DAY as i64;
        input.first_seen = 1_000_000 - 40 * DAY as i64;
        input.degree = 4;
        input.degree_reference = 10;
        input.in_active_project = true;
        input.noise = 0.25;

        let c = score(&input, &w);
        assert!((c.total - c.sum()).abs() < 1e-12, "{c:?}");
    }

    #[test]
    fn a_pin_beats_everything_the_system_can_infer() {
        let w = Weights::default();
        let now = 2_000_000;

        let mut best_possible = at(now);
        best_possible.access_count = 100_000.0;
        best_possible.edit_count = 100_000.0;
        best_possible.last_access = now;
        best_possible.first_seen = now - DAY as i64;
        best_possible.degree = 1_000;
        best_possible.degree_reference = 10;
        best_possible.in_active_project = true;

        let mut pinned_and_forgotten = at(now);
        pinned_and_forgotten.pinned = true;
        pinned_and_forgotten.last_access = now - 3650 * DAY as i64;
        pinned_and_forgotten.noise = 1.0;

        assert!(
            pinned_and_forgotten.pinned_score(&w) > score(&best_possible, &w).total,
            "a pin has to be unanswerable"
        );
    }

    #[test]
    fn no_component_can_exceed_its_weight() {
        let w = Weights::default();
        let now = 3_000_000;
        let mut absurd = at(now);
        absurd.access_count = 1e12;
        absurd.edit_count = 1e12;
        absurd.last_access = now;
        absurd.first_seen = now - 1;
        absurd.degree = u32::MAX;
        absurd.degree_reference = 1;
        absurd.in_active_project = true;

        let c = score(&absurd, &w);
        assert!(c.access <= w.access + 1e-9);
        assert!(c.recency <= w.recency + 1e-9);
        assert!(c.edits <= w.edits + 1e-9);
        assert!(c.centrality <= w.centrality + 1e-9);
        assert!(c.total <= w.ordinary_ceiling() + 1e-9);
    }

    #[test]
    fn recency_halves_over_the_half_life() {
        let w = Weights::default();
        let now = 4_000_000;
        let mut fresh = at(now);
        fresh.last_access = now;
        let mut old = at(now);
        old.last_access = now - (w.recency_half_life_days * DAY) as i64;

        let ratio = score(&old, &w).recency / score(&fresh, &w).recency;
        assert!((ratio - 0.5).abs() < 1e-6, "ratio was {ratio}");
    }

    #[test]
    fn decay_without_reinforcement_halves_over_its_half_life() {
        let w = Weights::default();
        let after = decay(4.0, (w.decay_half_life_days * DAY) as i64, 0.0, &w);
        assert!((after - 2.0).abs() < 1e-9, "{after}");
    }

    #[test]
    fn reinforcement_is_added_after_decay_not_before() {
        let w = Weights::default();
        let after = decay(4.0, (w.decay_half_life_days * DAY) as i64, 1.0, &w);
        assert!((after - 3.0).abs() < 1e-9, "{after}");
    }


    #[test]
    fn an_access_is_forgotten_like_everything_else() {
        let w = Weights::default();
        let now = 6_000_000_000;
        let long_ago: Vec<i64> = (0..50).map(|i| now - (700 + i) * DAY as i64).collect();
        let recently: Vec<i64> = (0..3).map(|i| now - (2 + i) * DAY as i64).collect();

        let stale = effective_count(long_ago, now, &w);
        let fresh = effective_count(recently, now, &w);
        assert!(stale < 0.1, "fifty accesses two years ago still counted {stale}");
        assert!(fresh > 2.0, "three accesses this week counted only {fresh}");
    }

    #[test]
    fn an_effective_count_never_exceeds_the_number_of_episodes() {
        let w = Weights::default();
        let now = 7_000_000_000;
        let all_at_once: Vec<i64> = vec![now; 10];
        assert!(effective_count(all_at_once, now, &w) <= 10.0 + 1e-9);
    }

    #[test]
    fn an_episode_from_the_future_is_ignored() {
        let w = Weights::default();
        let now = 8_000_000_000;
        assert_eq!(effective_count(vec![now + 10 * DAY as i64], now, &w), 0.0);
    }

    #[test]
    fn generated_output_scores_as_noise_and_prose_does_not() {
        assert_eq!(noise_score("/w/p/node_modules/react/index.js"), 1.0);
        assert_eq!(noise_score("/w/p/target/debug/thing.rlib"), 1.0);
        assert_eq!(noise_score("/w/p/docs/decisions.md"), 0.0);
        assert!(noise_score("/w/p/src/main.o") > 0.5);
        assert!(noise_score("/w/p/Cargo.lock") > 0.0 && noise_score("/w/p/Cargo.lock") < 0.8);
    }

    #[test]
    fn a_copy_of_something_ranks_below_the_thing_it_copies() {
        assert!(noise_score("/w/p/scratch/backups/fm-2026-07-07/notes/atommem.md") > 0.8);
        assert!(noise_score("/w/p/.snapshots/2026-08-01/notes/atommem.md") > 0.8);
        assert_eq!(noise_score("/w/p/notes/atommem.md"), 0.0);
        // `oldnotes` is not a backup directory, and a substring match would say it was.
        assert_eq!(noise_score("/w/p/oldnotes/atommem.md"), 0.0);
    }

    #[test]
    fn a_node_nothing_is_known_about_scores_zero() {
        let c = score(&at(5_000_000), &Weights::default());
        assert_eq!(c.total, 0.0);
    }
}

#[cfg(test)]
impl Inputs {
    fn pinned_score(&self, w: &Weights) -> f64 {
        score(self, w).total
    }
}
