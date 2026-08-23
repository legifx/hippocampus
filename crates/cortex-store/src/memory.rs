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
use cortex_memory::consolidate::{ConsolidationReport, Highlight, TierMovement};
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

/// What one pass may do.
#[derive(Debug, Clone)]
pub struct ConsolidateParams {
    pub now: i64,
    /// Compute and report, write nothing.
    pub dry_run: bool,
    pub weights: Weights,
    pub tiers: TierPolicy,
    pub co_access: CoAccessConfig,
    /// How many nodes to name in the report, per section.
    pub highlights: usize,
}

impl ConsolidateParams {
    pub fn new(now: i64) -> Self {
        Self {
            now,
            dry_run: false,
            weights: Weights::default(),
            tiers: TierPolicy::default(),
            co_access: CoAccessConfig::default(),
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

        // 3 — Score and tier.
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

        // 4 — Write.
        if !p.dry_run {
            self.apply_salience(&updates, &nodes, p.now)?;
            self.prune_salience_log()?;
            self.meta_set("consolidated_through", &p.now.to_string())?;
        }

        // 5 — Report.
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
            not_done: vec![
                "semantic abstraction: one concept node per cluster needs an embedding model \
                 and is M5 step 2"
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
        let mut stmt = conn.prepare(
            "SELECT id, path, kind, parent_id, tier, salience, pinned, zone_id, size, mtime
             FROM nodes
             WHERE path IS NOT NULL AND kind IN ('file', 'dir') AND salience >= ?1
               AND (?2 IS NULL OR zone_id = ?2)
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
                name: leaf(&path).to_string(),
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
                        "SELECT id, path, kind, parent_id, tier, salience, pinned, zone_id, size, mtime
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
                        name: leaf(&path).to_string(),
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

    #[test]
    fn a_percentile_of_nothing_is_still_divisible() {
        assert_eq!(percentile_degree(&HashMap::new(), 0.95), 1);
    }
}
