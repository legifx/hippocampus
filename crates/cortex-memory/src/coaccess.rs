//! Association by co-occurrence in time.
//!
//! This is the edge type no content model produces. A test and the module it covers may share
//! not one word; a design note and the code it argued about may be in different languages.
//! What they share is that they were open together. `episodes` records that, so the graph can
//! know it.
//!
//! The whole difficulty is telling a person apart from a machine. A human editing a feature
//! touches five or ten files in a quarter of an hour. A build touches four thousand in twenty
//! seconds, and if those became edges the graph would be a single hairball in which everything
//! is related to everything and no traversal means anything. So: **a burst that touches more
//! than [`CoAccessConfig::max_burst_nodes`] nodes is discarded whole.** Not truncated —
//! truncating keeps an arbitrary slice of a meaningless event and makes it look meaningful.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One episode, reduced to what association needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    pub ts: i64,
    pub node_id: i64,
    /// Index into the caller's session table. `None` for episodes with no session.
    pub session: Option<u64>,
}

/// A group of touches treated as one act.
#[derive(Debug, Clone, PartialEq)]
pub struct Burst {
    pub start: i64,
    pub end: i64,
    pub nodes: Vec<i64>,
    /// True when the burst was thrown away for being too large to be a person.
    pub discarded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoAccessConfig {
    /// Two touches further apart than this, with no session to join them, are separate acts.
    pub gap_secs: i64,
    /// A burst touching more nodes than this is machine activity and is discarded.
    pub max_burst_nodes: usize,
    /// How many separate bursts must contain a pair before it becomes an edge. One
    /// co-occurrence is a coincidence.
    pub min_bursts: u32,
    /// Saturation constant for the weight: a pair seen this many times scores about 0.63.
    pub weight_scale: f64,
    /// Never emit more than this many edges from one pass, strongest first.
    pub max_edges: usize,
}

impl Default for CoAccessConfig {
    fn default() -> Self {
        Self {
            gap_secs: 300,
            max_burst_nodes: 12,
            min_bursts: 2,
            weight_scale: 3.0,
            max_edges: 20_000,
        }
    }
}

/// A `co_access` edge the caller should write.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DerivedEdge {
    /// Always the smaller id, because the relation is symmetric and storing it twice would
    /// make the graph's degree counts wrong.
    pub src: i64,
    pub dst: i64,
    pub weight: f64,
    /// How many distinct bursts contained both. Kept so a human can ask why.
    pub bursts: u32,
}

/// Group touches into bursts, then count which pairs keep turning up together.
///
/// `touches` must be sorted by `ts` ascending. It is the caller's job because the storage
/// layer can do it in the query for free.
pub fn derive_co_access(touches: &[Touch], cfg: &CoAccessConfig) -> (Vec<DerivedEdge>, Vec<Burst>) {
    let bursts = split_into_bursts(touches, cfg);

    let mut pair_counts: HashMap<(i64, i64), u32> = HashMap::new();
    for burst in bursts.iter().filter(|b| !b.discarded) {
        for (i, &a) in burst.nodes.iter().enumerate() {
            for &b in &burst.nodes[i + 1..] {
                let key = if a < b { (a, b) } else { (b, a) };
                *pair_counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    let mut edges: Vec<DerivedEdge> = pair_counts
        .into_iter()
        .filter(|(_, count)| *count >= cfg.min_bursts)
        .map(|((src, dst), bursts)| DerivedEdge {
            src,
            dst,
            weight: 1.0 - (-f64::from(bursts) / cfg.weight_scale.max(0.001)).exp(),
            bursts,
        })
        .collect();

    // Strongest first, then by id so that two runs over the same episodes produce the same
    // list in the same order — idempotence is a Definition-of-Done item, and a HashMap's
    // iteration order is not.
    edges.sort_by(|a, b| {
        b.bursts
            .cmp(&a.bursts)
            .then(a.src.cmp(&b.src))
            .then(a.dst.cmp(&b.dst))
    });
    edges.truncate(cfg.max_edges);
    (edges, bursts)
}

fn split_into_bursts(touches: &[Touch], cfg: &CoAccessConfig) -> Vec<Burst> {
    let mut bursts: Vec<Burst> = Vec::new();
    // Sessions are authoritative and may interleave with each other, so each open session
    // carries its own accumulating burst.
    let mut open: HashMap<Option<u64>, usize> = HashMap::new();

    for touch in touches {
        let key = touch.session;
        let index = match open.get(&key) {
            // A session ends when it stops being mentioned for a whole gap, so that a session
            // id reused across a working day does not glue the morning to the evening.
            Some(&i) if touch.ts - bursts[i].end <= cfg.gap_secs => i,
            _ => {
                bursts.push(Burst {
                    start: touch.ts,
                    end: touch.ts,
                    nodes: Vec::new(),
                    discarded: false,
                });
                let i = bursts.len() - 1;
                open.insert(key, i);
                i
            }
        };

        let burst = &mut bursts[index];
        burst.end = touch.ts;
        if !burst.nodes.contains(&touch.node_id) {
            burst.nodes.push(touch.node_id);
        }
    }

    for burst in &mut bursts {
        if burst.nodes.len() > cfg.max_burst_nodes || burst.nodes.len() < 2 {
            burst.discarded = true;
        }
    }
    bursts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(ts: i64, node_id: i64) -> Touch {
        Touch { ts, node_id, session: None }
    }

    fn in_session(ts: i64, node_id: i64, session: u64) -> Touch {
        Touch { ts, node_id, session: Some(session) }
    }

    #[test]
    fn two_files_touched_together_twice_become_an_edge() {
        let touches = vec![
            touch(1_000, 1),
            touch(1_010, 2),
            touch(90_000, 1),
            touch(90_020, 2),
        ];
        let (edges, _) = derive_co_access(&touches, &CoAccessConfig::default());
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].src, edges[0].dst), (1, 2));
        assert_eq!(edges[0].bursts, 2);
    }

    #[test]
    fn one_co_occurrence_is_a_coincidence_and_not_an_edge() {
        let touches = vec![touch(1_000, 1), touch(1_010, 2)];
        let (edges, _) = derive_co_access(&touches, &CoAccessConfig::default());
        assert!(edges.is_empty());
    }

    #[test]
    fn a_build_touching_everything_produces_no_edges_at_all() {
        let mut touches = Vec::new();
        for round in 0..5 {
            for node in 0..500 {
                touches.push(touch(round * 100_000 + node, node));
            }
        }
        let (edges, bursts) = derive_co_access(&touches, &CoAccessConfig::default());
        assert!(edges.is_empty(), "a build is not an association");
        assert!(bursts.iter().all(|b| b.discarded));
    }

    #[test]
    fn a_gap_ends_a_burst() {
        let cfg = CoAccessConfig::default();
        let touches = vec![touch(0, 1), touch(cfg.gap_secs + 1, 2)];
        let (_, bursts) = derive_co_access(&touches, &cfg);
        assert_eq!(bursts.len(), 2);
    }

    #[test]
    fn a_session_holds_a_burst_together_across_a_gap() {
        let cfg = CoAccessConfig { gap_secs: 10, ..CoAccessConfig::default() };
        let touches = vec![
            in_session(0, 1, 7),
            in_session(5, 2, 7),
            in_session(9, 3, 7),
            in_session(200, 1, 7),
            in_session(205, 2, 7),
            in_session(209, 3, 7),
        ];
        let (edges, bursts) = derive_co_access(&touches, &cfg);
        assert_eq!(bursts.len(), 2, "the long gap splits the session in two");
        assert_eq!(edges.len(), 3, "three pairs among three files, seen twice each");
    }

    #[test]
    fn interleaved_sessions_do_not_contaminate_each_other() {
        let cfg = CoAccessConfig::default();
        let touches = vec![
            in_session(0, 1, 1),
            in_session(1, 10, 2),
            in_session(2, 2, 1),
            in_session(3, 11, 2),
            in_session(100_000, 1, 1),
            in_session(100_001, 10, 2),
            in_session(100_002, 2, 1),
            in_session(100_003, 11, 2),
        ];
        let (edges, _) = derive_co_access(&touches, &cfg);
        let pairs: Vec<(i64, i64)> = edges.iter().map(|e| (e.src, e.dst)).collect();
        assert!(pairs.contains(&(1, 2)));
        assert!(pairs.contains(&(10, 11)));
        assert!(!pairs.contains(&(1, 10)), "different sessions, same clock");
    }

    #[test]
    fn the_same_episodes_always_produce_the_same_list() {
        let touches: Vec<Touch> = (0..40)
            .flat_map(|round| (0..4).map(move |n| touch(round * 10_000 + n, n)))
            .collect();
        let cfg = CoAccessConfig::default();
        let (first, _) = derive_co_access(&touches, &cfg);
        let (second, _) = derive_co_access(&touches, &cfg);
        assert_eq!(first, second);
    }

    #[test]
    fn weight_rises_with_repetition_and_stays_below_one() {
        let cfg = CoAccessConfig::default();
        let mut touches = Vec::new();
        for round in 0..30 {
            touches.push(touch(round * 10_000, 1));
            touches.push(touch(round * 10_000 + 1, 2));
        }
        let (edges, _) = derive_co_access(&touches, &cfg);
        assert_eq!(edges.len(), 1);
        assert!(edges[0].weight > 0.9 && edges[0].weight < 1.0);
    }
}
