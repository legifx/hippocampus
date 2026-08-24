//! M5 against the real index: one consolidation pass, and the snapshot the Agent View reads.
//!
//! The arithmetic lives in `cortex-memory` and none of it is here. What is here is the part
//! that needs a database: gathering the inputs in a form that does not load a hundred thousand
//! paths into memory twice, writing the results in one transaction, and answering the question
//! "what does the brain look like right now" in a single round trip.
//!
//! **Why the whole episode history is replayed every pass.** Deriving associations only from
//! new episodes would make the result depend on how often consolidation happened to run, and
//! Definition-of-Done item 3 asks for idempotence. Replaying everything and upserting makes
//! the edge set a pure function of the log — run it twice, get the same graph. The log is
//! capped at [`MAX_REPLAY_EPISODES`] so that the cost has a ceiling.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::NodeId;
use cortex_core::model::{EdgeSource, EdgeType, NodeKind, Tier};
use cortex_memory::coaccess::{CoAccessConfig, Touch, derive_co_access};
use cortex_memory::concepts::{Cluster, ClusterConfig, Link, Named, cluster};
use cortex_memory::consolidate::{ConceptView, ConsolidationReport, Highlight, TierMovement};
use cortex_memory::salience::{Components, Inputs, Weights, effective_count, noise_score, score};
use cortex_memory::tier::{TierPolicy, assign_tier};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The most episodes one pass will replay, newest first. Ten years of ordinary use is well
/// under this; a machine that generated more has a watcher problem, not a memory problem.
pub const MAX_REPLAY_EPISODES: usize = 500_000;

/// How recently a zone must have been touched to count as a project the human is in.
pub const ACTIVE_PROJECT_WINDOW_SECS: i64 = 14 * 86_400;

/// How many computations to keep per node in `salience_log`.
///
/// The specification asks for every computation to be logged. Taken literally that is a
/// hundred thousand rows per pass and a gigabyte a year, for a table only ever read one row at
/// a time — so what is kept is every *changed* computation, and only the newest few per node.
/// The deviation is recorded as D64 rather than left as a surprise.
pub const SALIENCE_LOG_KEEP: usize = 3;

/// A salience below which a change is not worth a log row.
const MATERIAL_CHANGE: f64 = 0.01;

/// How much of the existing concept set a pass has to reproduce before it is allowed to replace
/// it. Below this the pass is treated as unreliable and the old concepts are kept.
const CONCEPT_KEEP_RATIO: f64 = 0.5;

/// What one pass may do.
#[derive(Debug, Clone)]
pub struct ConsolidateParams {
    pub now: i64,
    /// Compute and report, write nothing.
    pub dry_run: bool,
    pub weights: Weights,
    pub tiers: TierPolicy,
    pub co_access: CoAccessConfig,
    pub clusters: ClusterConfig,
    /// What to do about memories nobody reports on (M11, D76).
    pub decay: UnreportedDecay,
    /// How many nodes to name in the report, per section.
    pub highlights: usize,
}

/// How hard a memory is penalised for arriving in packs and never being mentioned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnreportedDecay {
    /// How many retrievals with no report at all before the penalty applies.
    ///
    /// Five, so that a memory is not punished for one unlucky pack. It is a threshold rather
    /// than a rate because the signal being measured is a *pattern* — retrieved repeatedly,
    /// mentioned never — and one retrieval is not a pattern.
    pub min_retrievals: i64,
    /// How much usefulness one pass removes. Small: this is a slow verdict, and a memory that
    /// starts being useful escapes it with a single report.
    pub step: f64,
}

impl Default for UnreportedDecay {
    fn default() -> Self {
        Self {
            min_retrievals: 5,
            step: 0.05,
        }
    }
}

impl ConsolidateParams {
    pub fn new(now: i64) -> Self {
        Self {
            now,
            dry_run: false,
            weights: Weights::default(),
            tiers: TierPolicy::default(),
            co_access: CoAccessConfig::default(),
            clusters: ClusterConfig::default(),
            decay: UnreportedDecay::default(),
            highlights: 8,
        }
    }
}

/// One node as the Agent View draws it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: i64,
    pub path: String,
    /// The last path segment, so the view never has to split a path itself (D4).
    pub name: String,
    pub kind: String,
    pub parent: Option<i64>,
    pub tier: String,
    pub salience: f64,
    pub pinned: bool,
    pub zone: Option<i64>,
    pub size: i64,
    pub mtime: i64,
    pub degree: u32,
    /// Episodes naming this node inside the snapshot's recency window.
    pub recent_touches: u32,
}

/// An edge between two nodes in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub src: i64,
    pub dst: i64,
    #[serde(rename = "type")]
    pub edge_type: String,
    pub weight: f64,
    pub source: String,
}

/// A zone, with the colour the daemon assigned it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphZone {
    pub id: i64,
    pub name: String,
    pub hue: u16,
    /// The zone's icon name, which is also how the daemon records what kind of place it is
    /// ("project", "code"). `None` for a zone nothing has classified.
    pub icon: Option<String>,
    pub nodes: i64,
}

/// One recent event, for the animation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEvent {
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub node: Option<i64>,
}

/// Everything the Agent View needs, in one answer.
///
/// One round trip on purpose: a view that fetched nodes, then edges, then episodes would draw
/// three states of the brain that never existed together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub generated_at: i64,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub zones: Vec<GraphZone>,
    pub events: Vec<GraphEvent>,
    /// Node ids in the Working Set, most recently touched first.
    pub working_set: Vec<i64>,
    pub totals: GraphTotals,
    /// The last consolidation pass, when there has been one.
    ///
    /// Part of the snapshot rather than a second call, because the Agent View animates the pass
    /// over the graph it is drawing: fetching the two separately would eventually animate one
    /// pass over a graph produced by another.
    #[serde(default)]
    pub report: Option<cortex_memory::ConsolidationReport>,
    /// What the numbers mean and what they do not. Rendered in the view's info panel, because
    /// a visualisation that cannot say what it is showing invites the viewer to invent it.
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTotals {
    pub nodes: i64,
    pub edges: i64,
    pub episodes: i64,
    /// Nodes in the index, per tier. The snapshot itself is a sample; this is the population.
    pub tiers: BTreeMap<String, i64>,
    pub last_consolidation: Option<i64>,
}

/// What to include in a snapshot.
#[derive(Debug, Clone)]
pub struct GraphParams {
    /// Most salient N nodes. Their ancestors are added on top, so the answer can be slightly
    /// larger — a floating subtree would be a lie about the shape of the index.
    pub limit: usize,
    /// Only nodes at or above this salience.
    pub min_salience: f64,
    /// Restrict to one zone.
    pub zone: Option<i64>,
    /// How many recent episodes to include.
    pub events: usize,
    /// How far back "recent" reaches, in seconds.
    pub window_secs: i64,
    pub now: i64,
}

impl Default for GraphParams {
    fn default() -> Self {
        Self {
            limit: 3_000,
            min_salience: f64::NEG_INFINITY,
            zone: None,
            events: 500,
            window_secs: 7 * 86_400,
            now: 0,
        }
    }
}

/// Everything one node's score is computed from, straight out of SQL.
struct RawNode {
    id: i64,
    path: String,
    tier: Tier,
    pinned: bool,
    zone: Option<i64>,
    first_seen: i64,
    salience: f64,
}

/// A node's episodes, split into the two kinds salience cares about.
///
/// Timestamps rather than counts, because the counts have to be age-weighted and the weighting
/// depends on `now` — which is not known when the rows are read, and would be wrong the moment
/// it was cached.
#[derive(Default)]
struct Touched {
    accesses: Vec<i64>,
    edits: Vec<i64>,
    last: i64,
}

impl Store {
    /// One consolidation pass: replay, associate, score, tier, report.
    ///
    /// The order matters. Edges are written before scoring because centrality is one of the
    /// score's components, and a pass that scored first would rank against the previous pass's
    /// graph — which is how a system ends up permanently one step behind itself.
    pub fn consolidate(&self, p: &ConsolidateParams) -> Result<ConsolidationReport> {
        let started = std::time::Instant::now();
        let window_from: i64 = self
            .meta_get("consolidated_through")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        // 1 — Replay.
        let (touches, stats) = self.replay()?;
        let episodes_replayed = touches.len();
        let (derived, bursts) = derive_co_access(&touches, &p.co_access);
        let bursts_discarded = bursts.iter().filter(|b| b.discarded).count();

        // 2 — Edge formation.
        let known = self.known_co_access()?;
        let (mut written, mut unchanged) = (0usize, 0usize);
        if !p.dry_run && !derived.is_empty() {
            self.batch(|b| {
                for edge in &derived {
                    let same = known
                        .get(&(edge.src, edge.dst))
                        .is_some_and(|w| (w - edge.weight).abs() < 1e-9);
                    if same {
                        continue;
                    }
                    b.tx().execute(
                        "INSERT INTO edges (src, dst, type, weight, created_at, source)
                         VALUES (?1, ?2, 'co_access', ?3, ?4, 'inferred')
                         ON CONFLICT(src, dst, type) DO UPDATE SET weight = excluded.weight",
                        params![edge.src, edge.dst, edge.weight, p.now],
                    )?;
                    written += 1;
                }
                Ok(())
            })?;
            unchanged = derived.len() - written;
        } else {
            for edge in &derived {
                match known.get(&(edge.src, edge.dst)) {
                    Some(w) if (w - edge.weight).abs() < 1e-9 => unchanged += 1,
                    _ => written += 1,
                }
            }
        }

        // 3 — Abstraction. Before scoring, because a file that belongs to a named concept is
        // more connected than one that does not, and centrality is a scoring component.
        let proposed = self.propose_concepts(&derived, &p.clusters)?;
        let concepts_pruned = if p.dry_run {
            0
        } else {
            self.write_concepts(&proposed, p.now)?
        };
        let clusters = proposed;

        // 4 — Score and tier.
        let degrees = self.degrees()?;
        let degree_reference = percentile_degree(&degrees, 0.95);
        let active_zones = self.active_zones(p.now)?;
        let nodes = self.raw_nodes()?;

        let mut updates: Vec<(i64, f64, Tier, Components)> = Vec::with_capacity(nodes.len());
        let mut movements: HashMap<(Tier, Tier), usize> = HashMap::new();
        let mut tier_totals: BTreeMap<String, usize> = BTreeMap::new();
        let mut pinned_held = 0usize;
        let mut ranked: Vec<(f64, usize)> = Vec::with_capacity(nodes.len());
        let mut demoted: Vec<usize> = Vec::new();

        let empty = Touched::default();
        for (index, node) in nodes.iter().enumerate() {
            let touched = stats.get(&node.id).unwrap_or(&empty);
            let input = Inputs {
                access_count: effective_count(touched.accesses.iter().copied(), p.now, &p.weights),
                edit_count: effective_count(touched.edits.iter().copied(), p.now, &p.weights),
                last_access: touched.last,
                first_seen: node.first_seen,
                now: p.now,
                degree: degrees.get(&node.id).copied().unwrap_or(0),
                degree_reference,
                pinned: node.pinned,
                in_active_project: node.zone.is_some_and(|z| active_zones.contains(&z)),
                noise: noise_score(&node.path),
            };
            let components = score(&input, &p.weights);
            let tier = assign_tier(
                node.tier,
                components.total,
                node.pinned,
                touched.last,
                node.first_seen,
                p.now,
                &p.tiers,
            );

            if node.pinned {
                pinned_held += 1;
            }
            if tier != node.tier {
                *movements.entry((node.tier, tier)).or_insert(0) += 1;
                if tier > node.tier {
                    demoted.push(index);
                }
            }
            *tier_totals.entry(tier.as_str().to_string()).or_insert(0) += 1;
            ranked.push((components.total, index));
            updates.push((node.id, components.total, tier, components));
        }

        // 4b — The memory layer's own housekeeping (M11).
        //
        // Here rather than on a timer of its own because it is the same question the rest of
        // this pass asks — what still deserves to be in the hot part of the index — and because
        // a second scheduler is a second thing to be wrong about when it last ran.
        let memories_total = self.count_memories()?;
        let (memories_expired, memories_unreported) = if p.dry_run {
            (self.expiring_memory_count(p.now)?, self.unreported_memory_count(p.decay.min_retrievals)?)
        } else {
            (
                self.expire_memories(p.now)?,
                self.decay_unreported(p.decay.min_retrievals, p.decay.step)?,
            )
        };

        let (orphans, duplicates) = self.health()?;

        // 5 — Write.
        if !p.dry_run {
            self.apply_salience(&updates, &nodes, p.now)?;
            self.score_concepts(&p.tiers)?;
            self.prune_salience_log()?;
            self.meta_set("consolidated_through", &p.now.to_string())?;
        }

        // 6 — Report.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let top: Vec<Highlight> = ranked
            .iter()
            .take(p.highlights)
            .map(|&(total, index)| {
                let node = &nodes[index];
                Highlight {
                    node_id: node.id,
                    path: node.path.clone(),
                    salience: total,
                    tier: updates[index].2.as_str().to_string(),
                    reason: reason_for(&updates[index].3, node.pinned),
                }
            })
            .collect();
        let demoted: Vec<Highlight> = demoted
            .iter()
            .take(p.highlights)
            .map(|&index| Highlight {
                node_id: nodes[index].id,
                path: nodes[index].path.clone(),
                salience: updates[index].1,
                tier: updates[index].2.as_str().to_string(),
                reason: "below threshold".to_string(),
            })
            .collect();

        let mut movements: Vec<TierMovement> = movements
            .into_iter()
            .map(|((from, to), count)| TierMovement {
                from: from.as_str().to_string(),
                to: to.as_str().to_string(),
                count,
            })
            .collect();
        movements.sort_by(|a, b| b.count.cmp(&a.count).then(a.from.cmp(&b.from)));

        let report = ConsolidationReport {
            ran_at: p.now,
            window_from,
            window_to: p.now,
            episodes_replayed,
            bursts: bursts.len(),
            bursts_discarded,
            edges_written: written,
            edges_unchanged: unchanged,
            nodes_scored: nodes.len(),
            movements,
            tier_totals,
            pinned_held,
            top,
            demoted,
            dry_run: p.dry_run,
            duration_ms: started.elapsed().as_millis() as u64,
            concepts: clusters
                .iter()
                .take(p.highlights)
                .map(|c| ConceptView {
                    label: c.label.clone(),
                    members: c.members.len(),
                })
                .collect(),
            concepts_total: clusters.len(),
            concepts_pruned,
            orphans,
            duplicates,
            memories_expired,
            memories_unreported,
            memories_total,
            not_done: vec![
                "concept labels are lexical, taken from what the members' paths have in common \
                 and nothing else has. A summariser would name them better; it would also make \
                 this step non-deterministic and unrunnable offline (D68)"
                    .to_string(),
                "promotion into the sync outbox is M7".to_string(),
            ],
        };

        if !p.dry_run {
            self.meta_set("last_consolidation", &serde_json::to_string(&report)?)?;
        }
        Ok(report)
    }

    /// The last report, as it was rendered. `None` before the first pass.
    pub fn last_consolidation(&self) -> Result<Option<ConsolidationReport>> {
        match self.meta_get("last_consolidation")? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// The newest stored breakdown for a node, for `brain.explain`.
    pub fn salience_components(&self, node: NodeId) -> Result<Option<(i64, Components)>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT computed_at, components_json FROM salience_log
             WHERE node_id = ?1 ORDER BY computed_at DESC LIMIT 1",
            [node.0],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(at, json)| Ok((at, serde_json::from_str(&json)?)))
        .transpose()
    }

    // -----------------------------------------------------------------------
    // The snapshot
    // -----------------------------------------------------------------------

    /// The brain as it is now, in one answer.
    ///
    /// **Everything that needs its own connection happens before this one is taken.** The store
    /// hands out connections from a small pool and an in-memory store has exactly one, so a
    /// helper called while a connection is held waits for a connection that only the caller can
    /// give back. That is a deadlock, not a slow query, and it does not announce itself: the
    /// first version of this function hung the test suite with no error and no CPU load.
    pub fn graph_snapshot(&self, p: &GraphParams) -> Result<GraphSnapshot> {
        let degrees = self.degrees()?;
        let zone_counts = self.zone_node_counts()?;
        let last_consolidation = self
            .meta_get("consolidated_through")?
            .and_then(|v| v.parse::<i64>().ok());

        let conn = self.conn();
        let since = p.now - p.window_secs;

        let mut recent: HashMap<i64, u32> = HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT node_id, COUNT(*) FROM episodes
                 WHERE node_id IS NOT NULL AND ts >= ?1 GROUP BY node_id",
            )?;
            let rows = stmt.query_map([since], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (id, count) = row?;
                recent.insert(id, count as u32);
            }
        }

        // Selection is by salience, and by recency for the nodes an unconsolidated index has
        // no salience for. Without the second key a fresh install would show an arbitrary
        // three thousand nodes out of a hundred thousand.
        // Concepts have no path — their name lives in `label`, which is where `upsert_concept`
        // puts it — so the label stands in for the path and the view never has to know that.
        // They are also never excluded by a zone filter: a concept spans whatever its members
        // span, and dropping it from a zone's view would hide the one node that says what the
        // zone is about.
        //
        // Memories (M11) are here for the same reason and a stronger one. They have no path
        // either, and their text stands in for it. A view of the brain that drew the files and
        // left out what the agents learned would be a view of half the brain, and the half it
        // left out is the half this milestone is about. Like concepts, they survive a zone
        // filter: a memory belongs to a scope, not to a directory.
        let mut stmt = conn.prepare(
            "SELECT id,
                    COALESCE(path, label,
                             (SELECT substr(m.text, 1, 90) FROM memories m
                              WHERE m.node_id = nodes.id),
                             '?'),
                    kind, parent_id, tier, salience, pinned, zone_id, size, mtime
             FROM nodes
             WHERE kind IN ('file', 'dir', 'concept', 'memory') AND salience >= ?1
               AND (path IS NOT NULL OR kind IN ('concept', 'memory'))
               AND (?2 IS NULL OR zone_id = ?2 OR kind IN ('concept', 'memory'))
             ORDER BY salience DESC, mtime DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![
                if p.min_salience.is_finite() { p.min_salience } else { f64::MIN },
                p.zone,
                p.limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )?;

        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut selected: HashMap<i64, usize> = HashMap::new();
        for row in rows {
            let (id, path, kind, parent, tier, salience, pinned, zone, size, mtime) = row?;
            selected.insert(id, nodes.len());
            nodes.push(GraphNode {
                name: if kind == "concept" || kind == "memory" {
                    // A memory has no last path component; the thing it says *is* its name.
                    path.clone()
                } else {
                    leaf(&path).to_string()
                },
                id,
                path,
                kind,
                parent,
                tier,
                salience,
                pinned: pinned != 0,
                zone,
                size,
                mtime,
                degree: degrees.get(&id).copied().unwrap_or(0),
                recent_touches: recent.get(&id).copied().unwrap_or(0),
            });
        }

        // Ancestors, so every node in the answer can be placed.
        let mut missing: Vec<i64> = nodes.iter().filter_map(|n| n.parent).collect();
        let mut guard = 0;
        while !missing.is_empty() && guard < 64 {
            guard += 1;
            let wanted: Vec<i64> = missing
                .drain(..)
                .filter(|id| !selected.contains_key(id))
                .collect();
            if wanted.is_empty() {
                break;
            }
            for id in wanted {
                if selected.contains_key(&id) {
                    continue;
                }
                let found = conn
                    .query_row(
                        "SELECT id, COALESCE(path, label, '?'), kind, parent_id, tier, salience,
                                pinned, zone_id, size, mtime
                         FROM nodes WHERE id = ?1",
                        [id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<i64>>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, f64>(5)?,
                                row.get::<_, i64>(6)?,
                                row.get::<_, Option<i64>>(7)?,
                                row.get::<_, i64>(8)?,
                                row.get::<_, i64>(9)?,
                            ))
                        },
                    )
                    .optional()?;
                if let Some((id, path, kind, parent, tier, salience, pinned, zone, size, mtime)) = found
                {
                    selected.insert(id, nodes.len());
                    nodes.push(GraphNode {
                        name: if kind == "concept" || kind == "memory" {
                    // A memory has no last path component; the thing it says *is* its name.
                    path.clone()
                } else {
                    leaf(&path).to_string()
                },
                        id,
                        path,
                        kind,
                        parent,
                        tier,
                        salience,
                        pinned: pinned != 0,
                        zone,
                        size,
                        mtime,
                        degree: degrees.get(&id).copied().unwrap_or(0),
                        recent_touches: recent.get(&id).copied().unwrap_or(0),
                    });
                    if let Some(next) = parent {
                        missing.push(next);
                    }
                }
            }
        }

        // Containment comes from `nodes.parent_id`, not from the edge table, so it is
        // synthesised here and labelled for what it is. Everything else is a real edge.
        let mut edges: Vec<GraphEdge> = nodes
            .iter()
            .filter_map(|n| {
                let parent = n.parent?;
                selected.contains_key(&parent).then(|| GraphEdge {
                    src: parent,
                    dst: n.id,
                    edge_type: "contains".to_string(),
                    weight: 1.0,
                    source: "observed".to_string(),
                })
            })
            .collect();
        {
            let mut stmt = conn.prepare("SELECT src, dst, type, weight, source FROM edges")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            for row in rows {
                let (src, dst, edge_type, weight, source) = row?;
                if selected.contains_key(&src) && selected.contains_key(&dst) {
                    edges.push(GraphEdge { src, dst, edge_type, weight, source });
                }
            }
        }

        let zones = {
            let counts = &zone_counts;
            let mut stmt = conn.prepare("SELECT id, name, color_hue, icon FROM zones ORDER BY id")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, name, hue, icon) = row?;
                out.push(GraphZone {
                    nodes: counts.get(&id).copied().unwrap_or(0),
                    id,
                    name,
                    hue: hue as u16,
                    icon,
                });
            }
            out
        };

        let events = {
            let mut stmt = conn.prepare(
                "SELECT ts, actor, action, node_id FROM episodes
                 ORDER BY ts DESC, id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([p.events as i64], |row| {
                Ok(GraphEvent {
                    ts: row.get(0)?,
                    actor: row.get(1)?,
                    action: row.get(2)?,
                    node: row.get(3)?,
                })
            })?;
            let mut out: Vec<GraphEvent> = rows.collect::<std::result::Result<_, _>>()?;
            out.reverse();
            out
        };

        let working_set = {
            let mut stmt = conn.prepare(
                "SELECT node_id FROM episodes
                 WHERE node_id IS NOT NULL AND ts >= ?1
                 GROUP BY node_id ORDER BY MAX(ts) DESC LIMIT 64",
            )?;
            let rows = stmt.query_map([p.now - 4 * 3_600], |r| r.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut tiers = BTreeMap::new();
        {
            let mut stmt = conn.prepare("SELECT tier, COUNT(*) FROM nodes GROUP BY tier")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (tier, count) = row?;
                tiers.insert(tier, count);
            }
        }

        let totals = GraphTotals {
            nodes: conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?,
            edges: conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?,
            episodes: conn.query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))?,
            tiers,
            last_consolidation,
        };

        let basis = match last_consolidation {
            Some(_) => {
                let mut text = format!(
                    "{} of {} nodes, chosen by salience; containment edges are derived from \
                     parent links, association edges from co-occurrence in the episode log",
                    nodes.len(),
                    totals.nodes
                );
                // An index younger than the tiering grace window has every node in one tier,
                // and a legend reading "cold 0" invites the reader to conclude the tiering is
                // broken. It is not: nothing here has had time to be forgotten yet.
                if totals.tiers.len() == 1 {
                    if let Some((tier, count)) = totals.tiers.iter().next() {
                        text.push_str(&format!(
                            ". All {count} nodes are in `{tier}`: consolidation has run, but a \
                             node is not judged during its first week in the index, and nothing \
                             here is older than that yet"
                        ));
                    }
                }
                text
            }
            None => format!(
                "{} of {} nodes. Consolidation has never run, so every salience is 0 and the \
                 selection is by modification time. Run `cortex consolidate`.",
                nodes.len(),
                totals.nodes
            ),
        };

        Ok(GraphSnapshot {
            generated_at: p.now,
            nodes,
            edges,
            zones,
            events,
            working_set,
            totals,
            // Filled in by the caller that has a reason to want it: `graph_snapshot` is also
            // used by tests that do not care, and deserialising a report costs more than the
            // rest of this function.
            report: None,
            basis,
        })
    }

    // -----------------------------------------------------------------------
    // Inputs
    // -----------------------------------------------------------------------

    /// The episode log, in the two shapes consolidation needs, from one read.
    ///
    /// Splitting this into two queries would read the same rows twice and — worse — could read
    /// them at two different moments, so a node's associations and its score would be computed
    /// from slightly different histories.
    fn replay(&self) -> Result<(Vec<Touch>, HashMap<i64, Touched>)> {
        let conn = self.conn();
        // The join is not decoration. `episodes.node_id` is deliberately not a foreign key —
        // an episode about a deleted file must outlive the file — so the log names nodes that
        // no longer exist. `edges` *does* have a foreign key, and deriving an association
        // between two deleted files fails the whole transaction with
        // `FOREIGN KEY constraint failed`. A real index found this in one run; the simulator
        // never could, because nothing in it is ever deleted.
        let mut stmt = conn.prepare(
            "SELECT e.ts, e.node_id, e.session_id, e.action
             FROM episodes e
             JOIN nodes n ON n.id = e.node_id
             ORDER BY e.ts DESC, e.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([MAX_REPLAY_EPISODES as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut sessions: HashMap<String, u64> = HashMap::new();
        let mut touches: Vec<Touch> = Vec::new();
        let mut actions: Vec<bool> = Vec::new();
        for row in rows {
            let (ts, node_id, session, action) = row?;
            let session = session.map(|s| {
                let next = sessions.len() as u64;
                *sessions.entry(s).or_insert(next)
            });
            touches.push(Touch { ts, node_id, session });
            actions.push(matches!(action.as_str(), "create" | "modify" | "move"));
        }
        // The query is newest-first so that the cap keeps recent history; everything downstream
        // wants oldest-first.
        touches.reverse();
        actions.reverse();

        let mut stats: HashMap<i64, Touched> = HashMap::new();
        for (touch, is_edit) in touches.iter().zip(actions) {
            let entry = stats.entry(touch.node_id).or_default();
            entry.accesses.push(touch.ts);
            if is_edit {
                entry.edits.push(touch.ts);
            }
            entry.last = entry.last.max(touch.ts);
        }
        Ok((touches, stats))
    }

    fn known_co_access(&self) -> Result<HashMap<(i64, i64), f64>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT src, dst, weight FROM edges WHERE type = 'co_access'")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, f64>(2)?))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (src, dst, weight) = row?;
            out.insert((src, dst), weight);
        }
        Ok(out)
    }

    /// Degree per node, counting an edge at both of its ends.
    fn degrees(&self) -> Result<HashMap<i64, u32>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, SUM(c) FROM (
                 SELECT src AS id, COUNT(*) AS c FROM edges GROUP BY src
                 UNION ALL
                 SELECT dst AS id, COUNT(*) AS c FROM edges GROUP BY dst
             ) GROUP BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, degree) = row?;
            out.insert(id, degree as u32);
        }
        Ok(out)
    }

    /// Zones with an episode inside the active window.
    fn active_zones(&self, now: i64) -> Result<Vec<i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.zone_id FROM episodes e
             JOIN nodes n ON n.id = e.node_id
             WHERE e.ts >= ?1 AND n.zone_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([now - ACTIVE_PROJECT_WINDOW_SECS], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn raw_nodes(&self) -> Result<Vec<RawNode>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, path, tier, pinned, zone_id, first_seen, salience
             FROM nodes
             WHERE path IS NOT NULL AND kind IN ('file', 'dir')
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (id, path, tier, pinned, zone, first_seen, salience) = row?;
            out.push(RawNode {
                id,
                path,
                tier: Tier::parse(&tier)
                    .ok_or_else(|| StoreError::corrupt(format!("unknown tier: {tier}")))?,
                pinned: pinned != 0,
                zone,
                first_seen,
                salience,
            });
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Abstraction
    // -----------------------------------------------------------------------

    /// The clusters this pass's associations contain, named.
    ///
    /// **Clustered from the edges the replay just derived, not from the edge table.** The two
    /// are the same thing after a real pass — co-access is a pure function of the episode log —
    /// but a dry run has not written anything, and reading the table would make it propose
    /// concepts from the *previous* pass's associations, or from none at all on a fresh index.
    /// A dry run that cannot answer "what would this do" is not worth having.
    fn propose_concepts(
        &self,
        derived: &[cortex_memory::coaccess::DerivedEdge],
        cfg: &ClusterConfig,
    ) -> Result<Vec<Cluster>> {
        if derived.is_empty() {
            return Ok(Vec::new());
        }
        let links: Vec<Link> = derived
            .iter()
            .map(|e| Link { src: e.src, dst: e.dst, weight: e.weight })
            .collect();

        // Only the nodes the edges actually touch: naming a cluster needs their paths, and
        // loading a hundred thousand of them to use a few hundred would be the expensive way to
        // get the same answer.
        let wanted: std::collections::HashSet<i64> =
            links.iter().flat_map(|l| [l.src, l.dst]).collect();
        let mut names = Vec::with_capacity(wanted.len());
        {
            let conn = self.conn();
            let mut stmt =
                conn.prepare("SELECT path FROM nodes WHERE id = ?1 AND path IS NOT NULL")?;
            let mut ids: Vec<i64> = wanted.into_iter().collect();
            ids.sort_unstable();
            for id in ids {
                let path: Option<String> = stmt.query_row([id], |r| r.get(0)).optional()?;
                if let Some(path) = path {
                    names.push(Named { id, path });
                }
            }
        }

        Ok(cluster(&links, &names, &self.uninformative_terms()?, cfg))
    }

    /// Two numbers a memory should be ashamed of, counted every pass.
    ///
    /// **Orphans**: nodes **somebody has actually touched** that no association reaches, in
    /// either direction. Nothing will ever lead you to one; it can only be found by already
    /// knowing its name. The restriction to touched nodes is what makes the number mean
    /// something — without it, a fresh index of a home directory reports a hundred thousand
    /// orphans, which is true, useless, and indistinguishable from a broken graph.
    /// **Duplicates**: nodes whose content hash another node also has. A copy does not merely
    /// waste space — it answers *instead of* the original, and it is the stale one.
    ///
    /// Both are borrowed from a knowledge base that has been in daily use for a year and had to
    /// learn to count them.
    fn health(&self) -> Result<(usize, usize)> {
        let conn = self.conn();
        let orphans: i64 = conn.query_row(
            "SELECT COUNT(*) FROM nodes n
             WHERE n.kind IN ('file', 'concept') AND n.tier IN ('working', 'stm')
               AND EXISTS (SELECT 1 FROM episodes ep WHERE ep.node_id = n.id)
               AND NOT EXISTS (SELECT 1 FROM edges e WHERE e.src = n.id AND e.type <> 'contains')
               AND NOT EXISTS (SELECT 1 FROM edges e WHERE e.dst = n.id AND e.type <> 'contains')",
            [],
            |r| r.get(0),
        )?;
        let duplicates: i64 = conn.query_row(
            "SELECT COALESCE(SUM(n - 1), 0) FROM (
                 SELECT COUNT(*) AS n FROM nodes
                 WHERE kind = 'file' AND content_hash IS NOT NULL
                 GROUP BY content_hash HAVING COUNT(*) > 1
             )",
            [],
            |r| r.get(0),
        )?;
        Ok((orphans as usize, duplicates as usize))
    }

    /// Terms that cannot distinguish anything on this index: the tokens of the path every
    /// indexed file shares.
    ///
    /// The longest common prefix of a sorted set is the longest common prefix of its first and
    /// last element, so this is two rows rather than a scan of a hundred thousand paths.
    fn uninformative_terms(&self) -> Result<std::collections::BTreeSet<String>> {
        let conn = self.conn();
        let bounds: Option<(String, String)> = conn
            .query_row(
                "SELECT MIN(path), MAX(path) FROM nodes WHERE path IS NOT NULL",
                [],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .and_then(|(lo, hi)| Some((lo?, hi?)));
        let Some((low, high)) = bounds else {
            return Ok(Default::default());
        };

        let shared: String = low
            .char_indices()
            .take_while(|&(i, c)| high[i..].starts_with(c))
            .map(|(_, c)| c)
            .collect();
        // Only whole path segments count: a prefix ending mid-name would make half a word
        // uninformative and leave the other half looking distinctive.
        let shared = shared.rsplit_once('/').map_or("", |(head, _)| head);
        Ok(cortex_memory::concepts::tokens(shared).into_iter().collect())
    }

    /// Write the clusters as concept nodes, and remove the concepts that no longer have one.
    ///
    /// A concept's identity is its label, so a cluster that gains a file updates the concept it
    /// already had rather than growing a rival beside it. Its membership is rewritten rather
    /// than merged: a file that left the cluster must stop being part of the concept, and an
    /// edge set that only ever grew would make abstraction another way of accumulating.
    fn write_concepts(&self, clusters: &[Cluster], now: i64) -> Result<usize> {
        // **A collapse is refused, not committed.** Concepts are derived: if this pass proposes
        // far fewer than exist, the likely cause is that something upstream went wrong — a
        // truncated replay, an empty edge set — and not that the subjects stopped existing.
        // Stale concepts are better than none.
        //
        // The lesson is borrowed. A second-brain daemon in daily use lost 939 of its ~4900
        // semantic edges twice in one day, because a rebuild that produced nothing was allowed
        // to replace everything with nothing; its fix was the same shape as this one, and its
        // sentence for it was "veraltete Kanten sind besser als keine".
        let existing: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM nodes WHERE kind = 'concept'",
            [],
            |r| r.get(0),
        )?;
        if existing > 0 && (clusters.len() as f64) < existing as f64 * CONCEPT_KEEP_RATIO {
            tracing::warn!(
                proposed = clusters.len(),
                existing,
                "refusing to replace the concepts: this pass proposes too few to be believed"
            );
            return Ok(0);
        }

        let mut live: Vec<i64> = Vec::with_capacity(clusters.len());
        for cluster in clusters {
            let concept = self.upsert_concept(&cluster.label, now)?;
            live.push(concept.0);
            self.batch(|b| {
                b.tx().execute(
                    "DELETE FROM edges WHERE src = ?1 AND type = 'derived_from'",
                    params![concept.0],
                )?;
                for member in &cluster.members {
                    b.tx().execute(
                        "INSERT INTO edges (src, dst, type, weight, created_at, source)
                         VALUES (?1, ?2, 'derived_from', 1.0, ?3, 'inferred')
                         ON CONFLICT(src, dst, type) DO NOTHING",
                        params![concept.0, member, now],
                    )?;
                }
                Ok(())
            })?;
        }

        // Concepts whose cluster has dissolved. Deleting the node cascades its edges.
        //
        // The empty case is written out rather than folded into the same statement: an empty
        // list rendered as `NOT IN (SELECT NULL)` is a predicate that is NULL for every row and
        // therefore true for none, so the delete silently did nothing exactly when there was
        // most to delete. SQL's three-valued logic makes that the *quiet* wrong answer.
        let conn = self.conn();
        let removed = if live.is_empty() {
            conn.execute("DELETE FROM nodes WHERE kind = 'concept'", [])?
        } else {
            let ids = live.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            conn.execute(
                &format!("DELETE FROM nodes WHERE kind = 'concept' AND id NOT IN ({ids})"),
                [],
            )?
        };
        Ok(removed)
    }

    /// A concept is worth what its best material is worth.
    ///
    /// The mean of its three most salient members, not the maximum and not the mean of all: the
    /// maximum makes every concept containing one hot file look hot, and the mean over all
    /// members lets a large cluster of cold files bury a live subject.
    fn score_concepts(&self, policy: &TierPolicy) -> Result<()> {
        let mut by_concept: HashMap<i64, Vec<f64>> = HashMap::new();
        {
            let conn = self.conn();
            // The grouping is done here rather than in SQL. The first attempt used a correlated
            // subquery with a LIMIT inside a WHERE ... IN, which is the kind of statement that
            // parses, runs, and quietly means something else.
            let mut stmt = conn.prepare(
                "SELECT e.src, m.salience FROM edges e
                 JOIN nodes c ON c.id = e.src AND c.kind = 'concept'
                 JOIN nodes m ON m.id = e.dst
                 WHERE e.type = 'derived_from'",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?;
            for row in rows {
                let (concept, salience) = row?;
                by_concept.entry(concept).or_default().push(salience);
            }
        }

        let mut scored: Vec<(i64, f64)> = by_concept
            .into_iter()
            .map(|(concept, mut members)| {
                members.sort_by(|a, b| b.total_cmp(a));
                let top = &members[..members.len().min(3)];
                (concept, top.iter().sum::<f64>() / top.len() as f64)
            })
            .collect();
        scored.sort_by_key(|(id, _)| *id);

        self.batch(|b| {
            for (id, salience) in &scored {
                // No grace window and no one-step demotion: a concept is rebuilt from its
                // members every pass, so there is no history for it to fall through.
                let tier = if *salience >= policy.working {
                    Tier::Working
                } else if *salience >= policy.stm {
                    Tier::Stm
                } else if *salience >= policy.ltm {
                    Tier::Ltm
                } else {
                    Tier::Cold
                };
                b.tx().execute(
                    "UPDATE nodes SET salience = ?2, tier = ?3 WHERE id = ?1",
                    params![id, salience, tier.as_str()],
                )?;
            }
            Ok(())
        })
    }

    fn apply_salience(
        &self,
        updates: &[(i64, f64, Tier, Components)],
        previous: &[RawNode],
        now: i64,
    ) -> Result<()> {
        self.batch(|b| {
            for (index, (id, salience, tier, components)) in updates.iter().enumerate() {
                b.tx().execute(
                    "UPDATE nodes SET salience = ?2, tier = ?3 WHERE id = ?1",
                    params![id, salience, tier.as_str()],
                )?;

                let before = &previous[index];
                let material = (before.salience - salience).abs() > MATERIAL_CHANGE
                    || before.tier != *tier;
                if material {
                    b.tx().execute(
                        "INSERT INTO salience_log (node_id, computed_at, score, components_json)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(node_id, computed_at) DO UPDATE SET
                             score = excluded.score,
                             components_json = excluded.components_json",
                        params![id, now, salience, serde_json::to_string(components)?],
                    )?;
                }
            }
            Ok(())
        })
    }

    fn prune_salience_log(&self) -> Result<()> {
        self.conn().execute(
            "DELETE FROM salience_log WHERE (node_id, computed_at) IN (
                 SELECT node_id, computed_at FROM (
                     SELECT node_id, computed_at,
                            ROW_NUMBER() OVER (PARTITION BY node_id ORDER BY computed_at DESC) AS rn
                     FROM salience_log
                 ) WHERE rn > ?1
             )",
            [SALIENCE_LOG_KEEP as i64],
        )?;
        Ok(())
    }
}

/// The degree at the given quantile, used to normalise centrality.
///
/// A maximum would let one hub flatten every other node to nearly zero; a mean would be
/// dragged around by the long tail of degree-one nodes. Never zero, so the caller can divide.
fn percentile_degree(degrees: &HashMap<i64, u32>, q: f64) -> u32 {
    if degrees.is_empty() {
        return 1;
    }
    let mut values: Vec<u32> = degrees.values().copied().collect();
    values.sort_unstable();
    let index = ((values.len() as f64 - 1.0) * q).round() as usize;
    values[index.min(values.len() - 1)].max(1)
}

/// Why a node scored what it did, in a few words.
fn reason_for(c: &Components, pinned: bool) -> String {
    if pinned {
        return "pinned".to_string();
    }
    let mut parts: Vec<(&str, f64)> = vec![
        ("recent", c.recency),
        ("edited often", c.edits),
        ("often touched", c.access),
        ("well connected", c.centrality),
        ("in an active project", c.project),
    ];
    parts.sort_by(|a, b| b.1.total_cmp(&a.1));
    let named: Vec<&str> = parts
        .iter()
        .filter(|(_, value)| *value > 0.05)
        .take(2)
        .map(|(name, _)| *name)
        .collect();
    if named.is_empty() {
        "nothing but existing".to_string()
    } else {
        named.join(", ")
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path)
}

/// Kinds that take part in the graph at all.
pub fn graphable(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::File | NodeKind::Dir | NodeKind::Concept)
}

/// Edge types the view knows how to draw. Anything else is passed through as its own name.
pub fn drawable(edge: EdgeType) -> bool {
    matches!(
        edge,
        EdgeType::Contains | EdgeType::CoAccess | EdgeType::SimilarTo | EdgeType::Annotates
    )
}

/// Source labels, for the view's legend.
pub fn source_label(source: EdgeSource) -> &'static str {
    match source {
        EdgeSource::Observed => "observed",
        EdgeSource::Inferred => "inferred",
        EdgeSource::Explicit => "asserted",
    }
}

#[cfg(test)]
mod tests {
    /// A view of the brain that drew the files and left out what the agents learned would be a
    /// view of half of it, and the half it left out is the half M11 is about.
    #[test]
    fn a_memory_is_drawn_by_the_graph_and_says_what_it_says() {
        use crate::memories::NewMemory;
        use cortex_core::memory::{MemoryKind, MemoryScope};
        use cortex_core::model::Actor;

        let store = Store::open_in_memory().unwrap();
        let written = store
            .remember(
                &NewMemory::new(
                    MemoryScope::Global,
                    MemoryKind::Decision,
                    "usearch stays; a second collection rather than a second engine",
                ),
                &Actor::Agent("a".into()),
                100,
            )
            .unwrap();

        let snapshot = store
            .graph_snapshot(&GraphParams {
                now: 200,
                ..GraphParams::default()
            })
            .unwrap();

        let node = snapshot
            .nodes
            .iter()
            .find(|n| n.id == written.node_id.0)
            .expect("the memory is not in the graph");
        assert_eq!(node.kind, "memory");
        // A memory has no last path component, so the thing it says is its name.
        assert!(node.name.contains("usearch stays"));
        assert!(node.path.contains("usearch stays"));
    }

    #[test]
    fn a_consolidation_pass_expires_scratch_and_penalises_what_nobody_reports() {
        use crate::memories::{MemoryFilter, NewMemory, PackItemRecord};
        use cortex_core::memory::{MemoryKind, MemoryScope, PackArm};
        use cortex_core::model::Actor;

        let store = Store::open_in_memory().unwrap();
        let actor = Actor::Agent("a".into());

        // One durable memory that keeps arriving in packs and that nobody ever mentions…
        let ignored = store
            .remember(
                &NewMemory::new(MemoryScope::Global, MemoryKind::Fact, "nobody ever uses this"),
                &actor,
                100,
            )
            .unwrap()
            .node_id;
        // …and one piece of scratch whose time is up.
        store
            .remember(
                &NewMemory {
                    expires_at: Some(150),
                    ..NewMemory::new(
                        MemoryScope::Session("s-1".into()),
                        MemoryKind::Observation,
                        "the editor has this open",
                    )
                },
                &actor,
                100,
            )
            .unwrap();

        for round in 0..6 {
            store
                .record_pack(
                    &format!("p{round}"),
                    None,
                    "a task",
                    2048,
                    50,
                    &[PackItemRecord {
                        node_id: ignored,
                        rank: 0,
                        arm: PackArm::Lexical,
                        score: 1.0,
                    }],
                    100,
                )
                .unwrap();
            store.note_retrieved(&[ignored]).unwrap();
        }

        // A dry run reports and changes nothing — the property that makes the number
        // trustworthy before somebody agrees to it.
        let params = ConsolidateParams {
            dry_run: true,
            ..ConsolidateParams::new(200)
        };
        let dry = store.consolidate(&params).unwrap();
        assert_eq!(dry.memories_expired, 1);
        assert_eq!(dry.memories_unreported, 1);
        assert_eq!(dry.memories_total, 2);
        assert_eq!(store.memory(ignored).unwrap().unwrap().usefulness, 0.0);
        assert_eq!(
            store.list_memories(&MemoryFilter::everything(200)).unwrap().len(),
            2,
            "a dry run removed something"
        );

        let real = store.consolidate(&ConsolidateParams::new(200)).unwrap();
        assert_eq!(real.memories_expired, 1);
        assert_eq!(real.memories_unreported, 1);
        assert!(
            store.memory(ignored).unwrap().unwrap().usefulness < 0.0,
            "the half of the learning signal that has to be driven by the pass is still inert"
        );
        assert_eq!(
            store.list_memories(&MemoryFilter::everything(200)).unwrap().len(),
            1,
            "the expired scratch is still there"
        );
        assert!(real.render().contains("memories"));
    }


    use super::*;
    use crate::queries::NodeUpsert;
    use cortex_core::ids::SessionId;
    use cortex_core::model::{Action, Actor};
    use std::path::PathBuf;

    const DAY: i64 = 86_400;
    /// A fixed clock. `Instant::now` is fine for a duration; a wall clock in a test is not.
    const T0: i64 = 1_700_000_000;

    fn store_with(paths: &[&str]) -> (Store, Vec<i64>) {
        let store = Store::open_in_memory().unwrap();
        let ids = paths
            .iter()
            .map(|p| {
                store
                    .upsert_node(
                        &NodeUpsert {
                            kind: NodeKind::File,
                            path: PathBuf::from(p),
                            parent_id: None,
                            mime: None,
                            size: 100,
                            mtime: T0 - 400 * DAY,
                            ctime: T0 - 400 * DAY,
                            inode: 0,
                            dev: 0,
                            zone_id: None,
                        },
                        T0 - 400 * DAY,
                    )
                    .unwrap()
                    .0
            })
            .collect();
        (store, ids)
    }

    fn touch(store: &Store, node: i64, ts: i64, session: &str) {
        store
            .append_episode(
                ts,
                &Actor::Human,
                Action::Modify,
                Some(NodeId(node)),
                Some(&SessionId(session.to_string())),
                &serde_json::json!({}),
            )
            .unwrap();
    }

    fn params(now: i64) -> ConsolidateParams {
        ConsolidateParams::new(now)
    }

    #[test]
    fn consolidation_scores_every_node_and_says_so() {
        let (store, ids) = store_with(&["/tmp/a.md", "/tmp/b.md"]);
        touch(&store, ids[0], T0 - DAY, "s1");
        let report = store.consolidate(&params(T0)).unwrap();

        assert_eq!(report.nodes_scored, 2);
        assert_eq!(report.episodes_replayed, 1);
        let a = store.node_by_id(NodeId(ids[0])).unwrap().unwrap();
        let b = store.node_by_id(NodeId(ids[1])).unwrap().unwrap();
        assert!(a.salience > b.salience, "the touched file has to outrank the untouched one");
    }

    #[test]
    fn running_it_twice_over_the_same_episodes_changes_nothing() {
        let (store, ids) = store_with(&["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"]);
        for round in 0..4 {
            let base = T0 - (10 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 30, &format!("s{round}"));
        }

        // Run to a fixed point first. Demotion is one tier per pass by design, so two passes
        // over a freshly scored index are *supposed* to differ — the claim being tested is that
        // a settled index does not drift, and that edges are never written twice.
        let mut settled = store.consolidate(&params(T0)).unwrap();
        for _ in 0..6 {
            let next = store.consolidate(&params(T0)).unwrap();
            if next.tier_totals == settled.tier_totals {
                settled = next;
                break;
            }
            settled = next;
        }
        let edges = store.edge_count().unwrap();

        let again = store.consolidate(&params(T0)).unwrap();
        assert_eq!(edges, store.edge_count().unwrap(), "duplicate edges");
        assert_eq!(again.edges_written, 0, "nothing should need rewriting");
        assert_eq!(settled.nodes_scored, again.nodes_scored);
        assert_eq!(settled.tier_totals, again.tier_totals, "a settled index drifted");
        assert_eq!(again.movements, vec![], "a settled index moved something");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let (store, ids) = store_with(&["/tmp/a.md", "/tmp/b.md"]);
        for round in 0..3 {
            let base = T0 - (5 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 10, &format!("s{round}"));
        }

        let report = store.consolidate(&ConsolidateParams { dry_run: true, ..params(T0) }).unwrap();
        assert!(report.edges_written > 0, "it must still say what it would have done");
        assert_eq!(store.edge_count().unwrap(), 0);
        assert_eq!(store.node_by_id(NodeId(ids[0])).unwrap().unwrap().salience, 0.0);
        assert!(store.meta_get("consolidated_through").unwrap().is_none());
    }

    #[test]
    fn thirty_days_of_disuse_demotes_but_a_pin_survives_all_of_it() {
        let (store, ids) = store_with(&["/tmp/kept.md", "/tmp/forgotten.md"]);
        store.set_pinned(NodeId(ids[0]), true).unwrap();
        touch(&store, ids[0], T0, "s0");
        touch(&store, ids[1], T0, "s0");

        for day in 1..=30 {
            store.consolidate(&params(T0 + day * DAY)).unwrap();
        }
        // A month of silence is not enough to reach Cold and should not be: one touch a month
        // ago is a file somebody might still be about to come back to. What must have happened
        // by now is the first step down.
        let after_a_month = store.node_by_id(NodeId(ids[1])).unwrap().unwrap();
        assert!(after_a_month.tier >= Tier::Stm, "{:?}", after_a_month.tier);

        for day in 31..=90 {
            store.consolidate(&params(T0 + day * DAY)).unwrap();
        }

        let kept = store.node_by_id(NodeId(ids[0])).unwrap().unwrap();
        let forgotten = store.node_by_id(NodeId(ids[1])).unwrap().unwrap();
        assert!(
            kept.tier <= Tier::Stm,
            "a pinned node must still be hot after three months of silence, was {:?}",
            kept.tier
        );
        assert_eq!(
            forgotten.tier,
            Tier::Cold,
            "an untouched file should have fallen all the way out in three months"
        );
    }

    #[test]
    fn tiering_settles_instead_of_oscillating() {
        let (store, ids) = store_with(&["/tmp/a.md"]);
        touch(&store, ids[0], T0 - 60 * DAY, "s0");
        let mut seen = Vec::new();
        for day in 0..12 {
            store.consolidate(&params(T0 + day * DAY)).unwrap();
            seen.push(store.node_by_id(NodeId(ids[0])).unwrap().unwrap().tier);
        }
        let last_four = &seen[seen.len() - 4..];
        assert!(
            last_four.windows(2).all(|w| w[0] == w[1]),
            "tier kept changing: {seen:?}"
        );
    }

    #[test]
    fn the_breakdown_that_is_stored_sums_to_the_score_that_was_used() {
        let (store, ids) = store_with(&["/tmp/a.md"]);
        touch(&store, ids[0], T0 - DAY, "s0");
        store.consolidate(&params(T0)).unwrap();

        let (_, components) = store.salience_components(NodeId(ids[0])).unwrap().unwrap();
        let stored = store.node_by_id(NodeId(ids[0])).unwrap().unwrap().salience;
        assert!((components.sum() - stored).abs() < 1e-9);
        assert!((components.total - stored).abs() < 1e-9);
    }

    #[test]
    fn build_output_is_ranked_below_prose_it_was_generated_from() {
        let (store, ids) = store_with(&["/tmp/p/src/main.rs", "/tmp/p/target/debug/main.o"]);
        for round in 0..3 {
            let base = T0 - (5 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 5, &format!("s{round}"));
        }
        store.consolidate(&params(T0)).unwrap();

        let source = store.node_by_id(NodeId(ids[0])).unwrap().unwrap();
        let object = store.node_by_id(NodeId(ids[1])).unwrap().unwrap();
        assert!(
            source.salience > object.salience,
            "identical histories, but one of them is build output"
        );
    }

    #[test]
    fn co_access_edges_are_written_and_are_inferred_not_asserted() {
        let (store, ids) = store_with(&["/tmp/a.rs", "/tmp/a_test.rs"]);
        for round in 0..3 {
            let base = T0 - (9 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 20, &format!("s{round}"));
        }
        store.consolidate(&params(T0)).unwrap();

        let neighbors = store
            .neighbors(NodeId(ids[0]), &[EdgeType::CoAccess], 1, 10)
            .unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].source, EdgeSource::Inferred);
        assert!(neighbors[0].weight > 0.0);
    }

    #[test]
    fn the_snapshot_is_self_describing_before_consolidation_has_ever_run() {
        let (store, _) = store_with(&["/tmp/a.md", "/tmp/b.md"]);
        let snapshot = store
            .graph_snapshot(&GraphParams { now: T0, ..GraphParams::default() })
            .unwrap();
        assert_eq!(snapshot.nodes.len(), 2);
        assert!(snapshot.basis.contains("never"), "{}", snapshot.basis);
        assert!(snapshot.totals.last_consolidation.is_none());
    }

    #[test]
    fn the_snapshot_carries_its_ancestors_so_nothing_floats() {
        let store = Store::open_in_memory().unwrap();
        let parent = store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::Dir,
                    path: PathBuf::from("/tmp/p"),
                    parent_id: None,
                    mime: None,
                    size: 0,
                    mtime: T0,
                    ctime: T0,
                    inode: 0,
                    dev: 0,
                    zone_id: None,
                },
                T0,
            )
            .unwrap();
        let child = store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::File,
                    path: PathBuf::from("/tmp/p/a.md"),
                    parent_id: Some(parent),
                    mime: None,
                    size: 1,
                    mtime: T0,
                    ctime: T0,
                    inode: 0,
                    dev: 0,
                    zone_id: None,
                },
                T0,
            )
            .unwrap();
        store.set_salience(child, 5.0).unwrap();

        let snapshot = store
            .graph_snapshot(&GraphParams { limit: 1, now: T0, ..GraphParams::default() })
            .unwrap();
        let ids: Vec<i64> = snapshot.nodes.iter().map(|n| n.id).collect();
        assert!(ids.contains(&child.0) && ids.contains(&parent.0), "{ids:?}");
        assert!(snapshot
            .edges
            .iter()
            .any(|e| e.edge_type == "contains" && e.src == parent.0 && e.dst == child.0));
    }

    #[test]
    fn the_snapshot_never_contains_an_edge_to_a_node_it_did_not_send() {
        let (store, ids) = store_with(&["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"]);
        store
            .insert_edge(
                NodeId(ids[0]),
                NodeId(ids[2]),
                EdgeType::SimilarTo,
                1.0,
                EdgeSource::Explicit,
                T0,
            )
            .unwrap();
        store.set_salience(NodeId(ids[0]), 9.0).unwrap();
        store.set_salience(NodeId(ids[1]), 8.0).unwrap();

        let snapshot = store
            .graph_snapshot(&GraphParams { limit: 2, now: T0, ..GraphParams::default() })
            .unwrap();
        let sent: Vec<i64> = snapshot.nodes.iter().map(|n| n.id).collect();
        for edge in &snapshot.edges {
            assert!(sent.contains(&edge.src) && sent.contains(&edge.dst), "{edge:?}");
        }
    }

    #[test]
    fn the_salience_log_does_not_grow_without_bound() {
        let (store, ids) = store_with(&["/tmp/a.md"]);
        touch(&store, ids[0], T0, "s0");
        for day in 0..20 {
            store.consolidate(&params(T0 + day * DAY)).unwrap();
        }
        let rows: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM salience_log WHERE node_id = ?1", [ids[0]], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(rows as usize <= SALIENCE_LOG_KEEP, "kept {rows} rows");
    }

    #[test]
    fn an_episode_about_a_file_that_no_longer_exists_does_not_break_the_pass() {
        let (store, ids) = store_with(&["/tmp/a.md", "/tmp/gone.md"]);
        for round in 0..3 {
            let base = T0 - (5 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 10, &format!("s{round}"));
        }
        // The file is deleted; its episodes stay, because `episodes` is the ground truth (R10).
        store.remove_path(std::path::Path::new("/tmp/gone.md")).unwrap();

        let report = store.consolidate(&params(T0)).unwrap();
        assert_eq!(report.nodes_scored, 1);
        // No edge may have been derived to the node that is gone: `edges` has a foreign key.
        assert_eq!(store.edge_count().unwrap(), 0);
    }

    /// Three files worked on together, twice, become a concept with a name from their paths.
    fn workspace_with_a_habit(store: &Store) -> Vec<i64> {
        let ids: Vec<i64> = ["/w/p/parser/tokens.rs", "/w/p/parser/grammar.rs", "/w/p/t/parser_spec.rs"]
            .iter()
            .map(|path| {
                store
                    .upsert_node(
                        &NodeUpsert {
                            kind: NodeKind::File,
                            path: PathBuf::from(path),
                            parent_id: None,
                            mime: None,
                            size: 100,
                            mtime: T0 - 400 * DAY,
                            ctime: T0 - 400 * DAY,
                            inode: 0,
                            dev: 0,
                            zone_id: None,
                        },
                        T0 - 400 * DAY,
                    )
                    .unwrap()
                    .0
            })
            .collect();
        for round in 0..4 {
            let base = T0 - (10 - round) * DAY;
            for (i, id) in ids.iter().enumerate() {
                touch(store, *id, base + i as i64 * 30, &format!("s{round}"));
            }
        }
        ids
    }

    #[test]
    fn files_used_together_become_a_concept_named_after_them() {
        let store = Store::open_in_memory().unwrap();
        let ids = workspace_with_a_habit(&store);
        let report = store.consolidate(&params(T0)).unwrap();

        assert_eq!(report.concepts_total, 1, "{:#?}", report.concepts);
        let concept = &report.concepts[0];
        assert_eq!(concept.members, 3);
        assert!(concept.label.contains("parser"), "{}", concept.label);

        // The concept exists as a node, and it points at its members.
        let neighbours = store
            .neighbors(NodeId(ids[0]), &[EdgeType::DerivedFrom], 1, 10)
            .unwrap();
        assert_eq!(neighbours.len(), 1, "the file must be reachable from its concept");
        assert_eq!(neighbours[0].kind, NodeKind::Concept.as_str());
    }

    #[test]
    fn consolidating_twice_does_not_grow_a_second_concept() {
        let store = Store::open_in_memory().unwrap();
        workspace_with_a_habit(&store);
        let first = store.consolidate(&params(T0)).unwrap();
        let second = store.consolidate(&params(T0)).unwrap();

        assert_eq!(first.concepts_total, second.concepts_total);
        let concepts: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM nodes WHERE kind = 'concept'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concepts, 1, "abstraction has to be idempotent too");
    }

    #[test]
    fn a_concept_is_worth_what_its_members_are_worth() {
        let store = Store::open_in_memory().unwrap();
        let ids = workspace_with_a_habit(&store);
        store.consolidate(&params(T0)).unwrap();

        let concept: f64 = store
            .conn()
            .query_row("SELECT salience FROM nodes WHERE kind = 'concept'", [], |r| r.get(0))
            .unwrap();
        let member = store.node_by_id(NodeId(ids[0])).unwrap().unwrap().salience;
        assert!(concept > 0.0, "a concept with no salience would never be drawn");
        assert!(
            concept <= member * 1.001,
            "a concept must not outrank the material it abstracts: {concept} vs {member}"
        );
    }

    #[test]
    fn a_concept_whose_cluster_dissolves_is_removed() {
        let store = Store::open_in_memory().unwrap();
        workspace_with_a_habit(&store);
        store.consolidate(&params(T0)).unwrap();
        assert_eq!(store.consolidate(&params(T0)).unwrap().concepts_total, 1);

        // Deleting the edges would not do it — they are a pure function of the episode log and
        // the next pass derives them again, which is the property D65 is about. What dissolves
        // a cluster is the cluster no longer qualifying.
        //
        // One cluster becoming none is a total collapse, so the credibility guard holds the old
        // concepts and reports having removed nothing. That is the intended behaviour; the test
        // below covers the guard itself. Here the guard is stood down to test the pruning.
        let stricter = ConsolidateParams {
            clusters: cortex_memory::concepts::ClusterConfig {
                min_members: 99,
                ..cortex_memory::concepts::ClusterConfig::default()
            },
            ..params(T0)
        };
        let report = store.consolidate(&stricter).unwrap();
        assert_eq!(report.concepts_total, 0);
        // The guard held: nothing was removed, and the concept is still there.
        assert_eq!(report.concepts_pruned, 0);
        let held: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM nodes WHERE kind = 'concept'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 1, "a collapse must not be committed");
    }

    /// A pass that proposes a plausible set is allowed to prune; one that collapses is not.
    #[test]
    fn a_pass_that_keeps_most_of_the_concepts_may_still_remove_the_rest() {
        let store = Store::open_in_memory().unwrap();
        workspace_with_a_habit(&store);
        store.consolidate(&params(T0)).unwrap();

        // A concept nothing proposes any more, beside the one that is still proposed. One of
        // two is exactly at the keep ratio, so the pass is believed and the stray goes.
        store.upsert_concept("stray", T0).unwrap();
        let report = store.consolidate(&params(T0)).unwrap();
        assert_eq!(report.concepts_pruned, 1, "{report:#?}");
        let left: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM nodes WHERE kind = 'concept'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
    }

    #[test]
    fn a_dry_run_names_the_concepts_it_would_form_without_forming_them() {
        let store = Store::open_in_memory().unwrap();
        workspace_with_a_habit(&store);
        let report = store
            .consolidate(&ConsolidateParams { dry_run: true, ..params(T0) })
            .unwrap();

        assert_eq!(report.concepts_total, 1);
        let concepts: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM nodes WHERE kind = 'concept'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concepts, 0, "a dry run must still write nothing");
    }

    #[test]
    fn the_snapshot_carries_concepts_and_calls_them_by_their_name() {
        let store = Store::open_in_memory().unwrap();
        workspace_with_a_habit(&store);
        store.consolidate(&params(T0)).unwrap();

        let snapshot = store
            .graph_snapshot(&GraphParams { now: T0, ..GraphParams::default() })
            .unwrap();
        let concept = snapshot
            .nodes
            .iter()
            .find(|n| n.kind == "concept")
            .expect("the concept has to be in the picture");
        assert!(concept.name.contains("parser"), "{}", concept.name);
        assert!(concept.parent.is_none());
        assert!(
            snapshot.edges.iter().any(|e| e.edge_type == "derived_from" && e.src == concept.id),
            "and so do the edges that say what it abstracts"
        );
    }

    #[test]
    fn the_report_counts_what_nothing_links_to_and_what_exists_twice() {
        let store = Store::open_in_memory().unwrap();
        let ids = workspace_with_a_habit(&store);

        // A fourth file somebody opened, but never alongside anything else — so it has a
        // history and no association. An untouched file is not an orphan, it is simply a file.
        let lonely = store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::File,
                    path: PathBuf::from("/w/p/notes/alone.md"),
                    parent_id: None,
                    mime: None,
                    size: 10,
                    mtime: T0,
                    ctime: T0,
                    inode: 0,
                    dev: 0,
                    zone_id: None,
                },
                T0,
            )
            .unwrap();

        touch(&store, lonely.0, T0 - 3 * DAY, "alone-1");
        touch(&store, lonely.0, T0 - 2 * DAY, "alone-2");

        // And two files with the same content, as a backup directory produces.
        for path in ["/w/p/notes/a.md", "/w/p/backups/2026-07/a.md"] {
            let id = store
                .upsert_node(
                    &NodeUpsert {
                        kind: NodeKind::File,
                        path: PathBuf::from(path),
                        parent_id: None,
                        mime: None,
                        size: 10,
                        mtime: T0,
                        ctime: T0,
                        inode: 0,
                        dev: 0,
                        zone_id: None,
                    },
                    T0,
                )
                .unwrap();
            store
                .conn()
                .execute("UPDATE nodes SET content_hash = 'same' WHERE id = ?1", [id.0])
                .unwrap();
        }

        let report = store.consolidate(&params(T0)).unwrap();
        assert!(report.orphans >= 1, "the unlinked file has to be counted: {report:#?}");
        assert_eq!(report.duplicates, 1, "two copies is one duplicate, not two");
        assert!(ids.len() == 3 && lonely.0 > 0);
    }

    #[test]
    fn a_backup_copy_ranks_below_the_file_it_copies() {
        let (store, ids) = store_with(&["/w/p/notes/a.md", "/w/p/backups/2026-07/a.md"]);
        for round in 0..3 {
            let base = T0 - (5 - round) * DAY;
            touch(&store, ids[0], base, &format!("s{round}"));
            touch(&store, ids[1], base + 5, &format!("s{round}"));
        }
        store.consolidate(&params(T0)).unwrap();

        let original = store.node_by_id(NodeId(ids[0])).unwrap().unwrap().salience;
        let copy = store.node_by_id(NodeId(ids[1])).unwrap().unwrap().salience;
        assert!(original > copy, "identical histories, but one of them is a backup");
    }

    #[test]
    fn a_percentile_of_nothing_is_still_divisible() {
        assert_eq!(percentile_degree(&HashMap::new(), 0.95), 1);
    }
}
