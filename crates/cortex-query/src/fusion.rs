//! Combining several rankings into one.
//!
//! Reciprocal Rank Fusion adds `1 / (k + rank)` per arm. Its virtue is that it needs no
//! calibration: a BM25 score, a cosine similarity and a fuzzy path score are numbers on three
//! unrelated scales, and any attempt to weight them directly is a constant that has to be
//! re-tuned whenever any arm changes. Ranks are comparable by construction.
//!
//! `k` damps the top of each list. With `k = 60` — the value from the original paper — being
//! first in one arm is worth `1/61` and being third in two arms is worth `2/63`, so agreement
//! between arms beats a single confident opinion. That is the behaviour worth having: the path
//! index and the content index are wrong in different ways.
//!
//! Salience is applied afterwards as a bounded multiplier, never as another arm. A file that
//! matters is not a file that matches, and letting importance enter the fusion would let it
//! outvote relevance instead of merely ordering equals.

use cortex_core::ids::NodeId;
use std::collections::HashMap;

/// Which retrieval arm produced a ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arm {
    /// Trigram path matching, in memory.
    Path,
    /// BM25 over chunk text.
    Lexical,
    /// Vector similarity. Populated from M2's second half onwards.
    Vector,
}

/// One arm's opinion: nodes, best first, without scores.
///
/// Scores are deliberately dropped at this boundary. Fusion works on ranks, and keeping the
/// raw scores around invites someone to mix the two.
#[derive(Debug, Clone)]
pub struct Ranking {
    pub arm: Arm,
    pub nodes: Vec<NodeId>,
}

/// A node after fusion, with enough detail to explain the result.
#[derive(Debug, Clone, PartialEq)]
pub struct Fused {
    pub node_id: NodeId,
    pub path_rank: Option<u32>,
    pub lexical_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub fused: f64,
}

/// Fuse rankings into one ordering, best first.
///
/// Ties are broken by node id so that the same corpus and the same query always produce the
/// same order. Without that, two results with identical scores swap places between runs and
/// every test that looks at ordering becomes flaky for no reason.
pub fn fuse(rankings: &[Ranking], k: f64) -> Vec<Fused> {
    let mut by_node: HashMap<NodeId, Fused> = HashMap::new();

    for ranking in rankings {
        for (index, node) in ranking.nodes.iter().enumerate() {
            let rank = index as u32 + 1;
            let entry = by_node.entry(*node).or_insert(Fused {
                node_id: *node,
                path_rank: None,
                lexical_rank: None,
                vector_rank: None,
                fused: 0.0,
            });

            // A node appearing twice in one arm's list would otherwise be counted twice. Keep
            // the better rank and score it once.
            let slot = match ranking.arm {
                Arm::Path => &mut entry.path_rank,
                Arm::Lexical => &mut entry.lexical_rank,
                Arm::Vector => &mut entry.vector_rank,
            };
            if slot.is_some() {
                continue;
            }
            *slot = Some(rank);
            entry.fused += 1.0 / (k + rank as f64);
        }
    }

    let mut fused: Vec<Fused> = by_node.into_values().collect();
    fused.sort_by(|a, b| {
        b.fused
            .partial_cmp(&a.fused)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.node_id.0.cmp(&b.node_id.0))
    });
    fused
}

/// The multiplier salience is allowed to apply to a fused score.
///
/// Bounded by `max_boost` because an unbounded importance signal turns the search into a list
/// of favourites: the top result stops depending on the query, and the user stops believing the
/// ranking. A salience of zero — every node until M5 implements the salience engine — leaves
/// the score untouched.
pub fn salience_boost(salience: f64, max_boost: f64) -> f64 {
    let max_boost = max_boost.max(1.0);
    1.0 + (max_boost - 1.0) * salience.clamp(0.0, 1.0)
}

/// Reduce per-chunk hits to per-node hits, keeping the best chunk of each node.
///
/// Retrieval happens over chunks; results are files. A document whose every paragraph mentions
/// the query would otherwise fill the entire result list on its own.
///
/// Returns indices into `hits`, ordered by descending score, so the caller keeps its own richer
/// hit type rather than converting into something this module would have to know about.
pub fn best_per_node<T>(
    hits: &[T],
    node_of: impl Fn(&T) -> NodeId,
    score_of: impl Fn(&T) -> f64,
) -> Vec<usize> {
    let mut best: HashMap<NodeId, usize> = HashMap::new();
    let mut order: Vec<NodeId> = Vec::new();

    for (index, hit) in hits.iter().enumerate() {
        let node = node_of(hit);
        match best.get(&node) {
            // Already have a better chunk of this document.
            Some(existing) if score_of(&hits[*existing]) >= score_of(hit) => {}
            Some(_) => {
                best.insert(node, index);
            }
            None => {
                best.insert(node, index);
                order.push(node);
            }
        }
    }

    let mut selected: Vec<usize> = order.into_iter().map(|node| best[&node]).collect();
    selected.sort_by(|a, b| {
        score_of(&hits[*b])
            .partial_cmp(&score_of(&hits[*a]))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: f64 = 60.0;

    fn ranking(arm: Arm, ids: &[i64]) -> Ranking {
        Ranking {
            arm,
            nodes: ids.iter().copied().map(NodeId).collect(),
        }
    }

    #[test]
    fn agreement_between_two_arms_beats_a_single_first_place() {
        // The property that makes fusion worth having at all.
        // Node 1 is the path index's favourite and nothing else. Node 2 is second there and
        // first in the content index, and that is the one to show.
        let fused = fuse(
            &[
                ranking(Arm::Path, &[1, 2, 3]),
                ranking(Arm::Lexical, &[2, 4, 5]),
            ],
            K,
        );
        assert_eq!(fused[0].node_id, NodeId(2), "the node both arms like must win");
        assert_eq!(fused[0].path_rank, Some(2));
        assert_eq!(fused[0].lexical_rank, Some(1));
        assert_eq!(fused[1].node_id, NodeId(1));
    }

    #[test]
    fn symmetric_disagreement_favours_the_ends_of_the_lists() {
        // Reversed lists look like they should tie, and they nearly do — but 1/(k+1) + 1/(k+3)
        // is fractionally more than twice 1/(k+2), because the reciprocal is convex. So a node
        // that one arm loves and the other dislikes edges out one that both find middling.
        // Recorded because a 0.003 % difference is exactly the kind of thing that gets
        // "corrected" into an equality assertion by someone who assumes symmetry.
        let fused = fuse(
            &[
                ranking(Arm::Path, &[1, 2, 3]),
                ranking(Arm::Lexical, &[3, 2, 1]),
            ],
            K,
        );
        assert_eq!(
            fused.iter().map(|f| f.node_id.0).collect::<Vec<_>>(),
            vec![1, 3, 2],
            "the two extremes must precede the consistently-middling node"
        );
        assert!(fused[0].fused > fused[2].fused);
        assert!((fused[0].fused - fused[2].fused) < 1e-4, "and only barely");
    }

    #[test]
    fn a_node_found_by_one_arm_only_still_appears() {
        let fused = fuse(
            &[ranking(Arm::Path, &[1]), ranking(Arm::Lexical, &[2])],
            K,
        );
        assert_eq!(fused.len(), 2);
        assert!(fused.iter().any(|f| f.node_id == NodeId(2) && f.path_rank.is_none()));
    }

    #[test]
    fn the_order_is_stable_across_runs() {
        // HashMap iteration order changes per process. Anything that returns results in that
        // order is a test that fails one morning for no reason anybody can reproduce.
        let rankings = [
            ranking(Arm::Path, &[7, 3, 9, 1]),
            ranking(Arm::Lexical, &[9, 1, 7, 3]),
        ];
        let first = fuse(&rankings, K);
        for _ in 0..20 {
            assert_eq!(fuse(&rankings, K), first);
        }
    }

    #[test]
    fn ties_are_broken_by_node_id_rather_than_by_chance() {
        let fused = fuse(&[ranking(Arm::Path, &[5]), ranking(Arm::Lexical, &[2])], K);
        assert_eq!(fused[0].node_id, NodeId(2));
        assert_eq!(fused[1].node_id, NodeId(5));
        assert_eq!(fused[0].fused, fused[1].fused);
    }

    #[test]
    fn a_duplicate_inside_one_arm_is_counted_once() {
        let fused = fuse(&[ranking(Arm::Path, &[1, 1, 1])], K);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].path_rank, Some(1));
        assert!((fused[0].fused - 1.0 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn an_empty_arm_contributes_nothing_and_breaks_nothing() {
        let fused = fuse(&[ranking(Arm::Path, &[]), ranking(Arm::Lexical, &[4])], K);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].node_id, NodeId(4));
    }

    #[test]
    fn salience_is_bounded_at_both_ends() {
        assert_eq!(salience_boost(0.0, 1.5), 1.0);
        assert_eq!(salience_boost(1.0, 1.5), 1.5);
        assert_eq!(salience_boost(9.9, 1.5), 1.5, "an out-of-range salience must not run away");
        assert_eq!(salience_boost(-1.0, 1.5), 1.0);
        assert_eq!(salience_boost(1.0, 0.5), 1.0, "a boost below 1 must not demote");
    }

    #[test]
    fn the_best_chunk_of_a_document_represents_it() {
        // Three chunks of node 1, two of node 2. The result is one row per document, ordered by
        // its best chunk — not five rows, four of which are the same file.
        let hits = [(1i64, 0.2), (1, 0.9), (2, 0.5), (1, 0.1), (2, 0.4)];
        let selected = best_per_node(&hits, |h| NodeId(h.0), |h| h.1);
        assert_eq!(selected, vec![1, 2], "expected the best chunk of each node");
        assert_eq!(hits[selected[0]], (1, 0.9));
        assert_eq!(hits[selected[1]], (2, 0.5));
    }

    #[test]
    fn equal_scores_keep_the_order_they_arrived_in() {
        let hits = [(1i64, 0.5), (2, 0.5), (3, 0.5)];
        let selected = best_per_node(&hits, |h| NodeId(h.0), |h| h.1);
        assert_eq!(selected, vec![0, 1, 2]);
    }
}
