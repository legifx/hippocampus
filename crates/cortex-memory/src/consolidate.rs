//! What a consolidation pass did, in a form a human will actually read.
//!
//! Definition-of-Done item 4 for M5 is not "a report exists". It is that somebody can tell
//! from the report what the system did **without reading logs**. That rules out a dump of
//! counters: the interesting facts are what moved down, what moved up, what got connected to
//! what, and — the one every such system hides — what it decided to ignore.
//!
//! The type is pure data and the renderer is a pure function, so the report can be produced by
//! a dry run that wrote nothing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How many nodes went from one tier to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierMovement {
    pub from: String,
    pub to: String,
    pub count: usize,
}

/// A concept the pass formed or refreshed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptView {
    pub label: String,
    pub members: usize,
}

/// A node worth naming in the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub node_id: i64,
    pub path: String,
    pub salience: f64,
    pub tier: String,
    pub reason: String,
}

/// The result of one pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub ran_at: i64,
    /// The episode window that was replayed. `from` is the previous pass's `to`.
    pub window_from: i64,
    pub window_to: i64,
    pub episodes_replayed: usize,
    pub bursts: usize,
    /// Bursts thrown away for being machine activity rather than a person working.
    pub bursts_discarded: usize,
    pub edges_written: usize,
    pub edges_unchanged: usize,
    pub nodes_scored: usize,
    pub movements: Vec<TierMovement>,
    pub tier_totals: BTreeMap<String, usize>,
    pub pinned_held: usize,
    pub top: Vec<Highlight>,
    pub demoted: Vec<Highlight>,
    /// The largest concepts this pass formed or refreshed, for the report.
    #[serde(default)]
    pub concepts: Vec<ConceptView>,
    /// How many concepts exist after this pass.
    #[serde(default)]
    pub concepts_total: usize,
    /// Concepts whose cluster no longer exists, removed by this pass.
    #[serde(default)]
    pub concepts_pruned: usize,
    /// Nodes somebody has touched that no association reaches, in either direction.
    ///
    /// Borrowed from a knowledge base that has been in daily use for a year: its health report
    /// counts exactly this, and the number — 75 of 415 curated notes — is the most informative
    /// thing in the file. An unlinked node is one nothing will ever lead you to.
    #[serde(default)]
    pub orphans: usize,
    /// Nodes whose content is byte-identical to another node's.
    ///
    /// Copies are not merely wasted space: they answer instead of the original, several times
    /// over, and the copy is the stale one.
    #[serde(default)]
    pub duplicates: usize,
    /// True when nothing was written.
    pub dry_run: bool,
    pub duration_ms: u64,
    /// What this pass deliberately did not do. Printed, not hidden — a report that lists only
    /// its successes is how a system's limits become folklore instead of documentation.
    pub not_done: Vec<String>,
}

impl ConsolidationReport {
    pub fn moved(&self) -> usize {
        self.movements.iter().map(|m| m.count).sum()
    }

    /// The report a human reads. Deliberately narrow enough for a terminal.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let head = if self.dry_run { "consolidation (dry run)" } else { "consolidation" };
        out.push_str(&format!("{head} — {} ms\n\n", self.duration_ms));

        out.push_str(&format!(
            "replayed   {} episodes in {} bursts ({} discarded as machine activity)\n",
            self.episodes_replayed, self.bursts, self.bursts_discarded
        ));
        out.push_str(&format!(
            "associated {} co-access edges written, {} already known\n",
            self.edges_written, self.edges_unchanged
        ));
        out.push_str(&format!(
            "abstracted {} concepts from those associations, {} no longer had one\n",
            self.concepts_total, self.concepts_pruned
        ));
        out.push_str(&format!(
            "scored     {} nodes, {} moved tier, {} pinned nodes held\n",
            self.nodes_scored,
            self.moved(),
            self.pinned_held
        ));
        out.push_str(&format!(
            "health     {} touched nodes nothing links to, {} with a byte-identical twin\n\n",
            self.orphans, self.duplicates
        ));

        if !self.tier_totals.is_empty() {
            out.push_str("tiers      ");
            // Explicit order: alphabetical would print cold first and read as a ranking.
            for tier in ["working", "stm", "ltm", "cold"] {
                if let Some(count) = self.tier_totals.get(tier) {
                    out.push_str(&format!("{tier} {count}   "));
                }
            }
            out.push_str("\n\n");
        }

        if !self.movements.is_empty() {
            out.push_str("moved\n");
            for m in &self.movements {
                out.push_str(&format!("  {:>7} → {:<7} {}\n", m.from, m.to, m.count));
            }
            out.push('\n');
        }

        if !self.top.is_empty() {
            out.push_str("most salient\n");
            for h in &self.top {
                out.push_str(&format!("  {:>6.2}  {}  ({})\n", h.salience, h.path, h.reason));
            }
            out.push('\n');
        }

        if !self.concepts.is_empty() {
            out.push_str("concepts\n");
            for concept in &self.concepts {
                out.push_str(&format!("  {:>4} files  {}\n", concept.members, concept.label));
            }
            out.push('\n');
        }

        if !self.demoted.is_empty() {
            out.push_str("forgotten a tier\n");
            for h in &self.demoted {
                out.push_str(&format!("  {:>6.2}  {}  → {}\n", h.salience, h.path, h.tier));
            }
            out.push('\n');
        }

        if !self.not_done.is_empty() {
            out.push_str("not done in this pass\n");
            for item in &self.not_done {
                out.push_str(&format!("  · {item}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> ConsolidationReport {
        ConsolidationReport {
            ran_at: 1_700_000_000,
            window_from: 1_699_000_000,
            window_to: 1_700_000_000,
            episodes_replayed: 900,
            bursts: 40,
            bursts_discarded: 12,
            edges_written: 31,
            edges_unchanged: 4,
            nodes_scored: 1_200,
            movements: vec![TierMovement { from: "stm".into(), to: "ltm".into(), count: 17 }],
            tier_totals: BTreeMap::from([("stm".to_string(), 1_183), ("ltm".to_string(), 17)]),
            pinned_held: 3,
            top: vec![Highlight {
                node_id: 5,
                path: "/tmp/x/notes.md".into(),
                salience: 3.1,
                tier: "working".into(),
                reason: "recent, edited often".into(),
            }],
            demoted: vec![],
            concepts: vec![ConceptView { label: "parser · grammar".into(), members: 6 }],
            concepts_total: 1,
            concepts_pruned: 0,
            orphans: 12,
            duplicates: 3,
            dry_run: false,
            duration_ms: 42,
            not_done: vec!["semantic abstraction of clusters (needs an embedding model)".into()],
        }
    }

    #[test]
    fn the_report_says_what_moved_and_what_was_ignored() {
        let text = report().render();
        assert!(text.contains("900 episodes"));
        assert!(text.contains("12 discarded as machine activity"));
        assert!(text.contains("stm → ltm"));
        assert!(text.contains("parser · grammar"), "the concepts have to be in the report");
        assert!(text.contains("abstracted 1 concepts"));
        assert!(text.contains("12 touched nodes nothing links to"));
        assert!(text.contains("not done in this pass"));
        assert!(text.contains("semantic abstraction"));
    }

    #[test]
    fn a_dry_run_says_so_in_the_first_line() {
        let mut r = report();
        r.dry_run = true;
        assert!(r.render().lines().next().unwrap().contains("dry run"));
    }

    #[test]
    fn moved_counts_every_movement() {
        let mut r = report();
        r.movements.push(TierMovement { from: "ltm".into(), to: "cold".into(), count: 5 });
        assert_eq!(r.moved(), 22);
    }
}
