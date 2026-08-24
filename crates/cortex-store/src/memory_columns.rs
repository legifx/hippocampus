//! What the brain knows about the rows of a listing, in one query.
//!
//! M10 step 2 puts the memory on every row — salience, tier, degree, when it was last touched,
//! whether it is pinned, which zone it is in. §5 of `docs/paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md`
//! is unusually specific about how *not* to do that:
//!
//! > The memory columns come from the same query, as a join, not as a second round trip per row.
//! > A per-row `brain.explain` would be ten thousand calls and is the obvious way to fail this.
//!
//! So: one statement per view. For a directory that is a scan of `nodes` by `parent_id`; for a
//! set of search hits it is the same statement with an id list. Both answer with everything the
//! renderer needs and nothing it does not.
//!
//! **What is *not* here is as deliberate.** Degree is counted from `edges` in the same statement
//! rather than read from a column the consolidation pass leaves behind. The paper suggests
//! precomputing it; the measurement in `benches` says the count costs a fraction of the budget on
//! ten thousand rows, and a stored count is a second copy of a fact that goes stale between
//! passes. If that measurement changes, the column is the answer and D86 says so.

use crate::error::Result;
use crate::Store;
use cortex_core::ids::{NodeId, ZoneId};
use cortex_core::model::Tier;
use rusqlite::OptionalExtension;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the index knows about one row, beyond what `stat` says.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryColumns {
    pub node_id: NodeId,
    pub tier: Tier,
    pub salience: f64,
    pub pinned: bool,
    pub zone_id: Option<ZoneId>,
    /// How many edges touch this node, in either direction. What "well connected" means, before
    /// salience turns it into a number between nothing and one.
    pub degree: u32,
    /// When something last happened to it, from the episode log. `None` when nothing ever has —
    /// which is different from "long ago" and is shown as different.
    pub last_episode: Option<i64>,
}

/// The columns for a set of paths, keyed by path.
///
/// By path rather than by `parent_id`: a view knows exactly which rows it is showing, and the
/// parent link is filled in by a pass of its own — a file the index has just seen has a path
/// before it has a parent, and a listing that waited for the link would show a directory with
/// no memory in it for as long as that took. Found by exactly that: a freshly reconciled
/// directory came back with every column empty.
///
/// Chunked, because SQLite's variable limit is 999 by default and a large directory is longer
/// than that. Twelve statements for ten thousand rows is still one question about the listing
/// rather than one per row (§5).
pub fn for_paths(store: &Store, paths: &[PathBuf]) -> Result<HashMap<PathBuf, MemoryColumns>> {
    let conn = store.conn();
    let mut out = HashMap::new();

    for chunk in paths.chunks(900) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT n.id, n.path, n.tier, n.salience, n.pinned, n.zone_id,
                    (SELECT COUNT(*) FROM edges e WHERE e.src = n.id)
                  + (SELECT COUNT(*) FROM edges e WHERE e.dst = n.id) AS degree,
                    (SELECT MAX(ep.ts) FROM episodes ep WHERE ep.node_id = n.id) AS last_episode
             FROM nodes n
             WHERE n.path IN ({placeholders})"
        );
        let mut statement = conn.prepare_cached(&sql)?;
        let values: Vec<String> = chunk
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let rows = statement.query_map(rusqlite::params_from_iter(values), row_to_columns)?;
        for row in rows {
            let (path, columns) = row?;
            out.insert(path, columns);
        }
    }
    Ok(out)
}

/// The same columns for a set of nodes that are already known by id — a page of search hits.
///
/// Chunked, because SQLite's variable limit is 999 by default and a page of results can be
/// longer than that. Chunking rather than a temporary table: three statements for two thousand
/// hits is cheaper than creating, filling and dropping a table, and it cannot leave one behind.
pub fn for_nodes(store: &Store, ids: &[NodeId]) -> Result<HashMap<NodeId, MemoryColumns>> {
    let conn = store.conn();
    let mut out = HashMap::new();

    for chunk in ids.chunks(900) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT n.id, n.path, n.tier, n.salience, n.pinned, n.zone_id,
                    (SELECT COUNT(*) FROM edges e WHERE e.src = n.id)
                  + (SELECT COUNT(*) FROM edges e WHERE e.dst = n.id) AS degree,
                    (SELECT MAX(ep.ts) FROM episodes ep WHERE ep.node_id = n.id) AS last_episode
             FROM nodes n
             WHERE n.id IN ({placeholders})"
        );
        let mut statement = conn.prepare_cached(&sql)?;
        let values: Vec<i64> = chunk.iter().map(|id| id.0).collect();
        let rows = statement.query_map(rusqlite::params_from_iter(values), row_to_columns)?;
        for row in rows {
            let (_, columns) = row?;
            out.insert(columns.node_id, columns);
        }
    }
    Ok(out)
}

/// A row built from what the index knows, for the sources that have no directory behind them.
///
/// A concept, the working set, a tier and a slice of time all answer the same way: *these
/// nodes*. So they all go through here — one query that turns a set of ids into rows, with the
/// filesystem facts the index already stores beside the memory ones. The alternative was a row
/// builder per source, which is four more places for "what a row is" to drift.
///
/// **No `stat` happens here.** The size and the modification time are the index's, which is what
/// a view of things you touched last week is *about*; a listing that went to the disk for each
/// of fifty rows scattered over a home directory would be fifty seeks to tell somebody something
/// they can already see.
#[derive(Debug, Clone)]
pub struct IndexRow {
    pub node_id: NodeId,
    pub path: PathBuf,
    pub kind: String,
    pub size: i64,
    pub mtime: i64,
    pub columns: MemoryColumns,
}

pub fn rows_for(store: &Store, ids: &[NodeId]) -> Result<Vec<IndexRow>> {
    let conn = store.conn();
    let mut found: HashMap<NodeId, IndexRow> = HashMap::new();

    for chunk in ids.chunks(900) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT n.id, n.path, n.tier, n.salience, n.pinned, n.zone_id,
                    (SELECT COUNT(*) FROM edges e WHERE e.src = n.id)
                  + (SELECT COUNT(*) FROM edges e WHERE e.dst = n.id) AS degree,
                    (SELECT MAX(ep.ts) FROM episodes ep WHERE ep.node_id = n.id) AS last_episode,
                    n.kind, n.size, n.mtime
             FROM nodes n
             WHERE n.id IN ({placeholders}) AND n.path IS NOT NULL"
        );
        let mut statement = conn.prepare_cached(&sql)?;
        let values: Vec<i64> = chunk.iter().map(|id| id.0).collect();
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            let (path, columns) = row_to_columns(row)?;
            Ok(IndexRow {
                node_id: columns.node_id,
                path,
                kind: row.get(8)?,
                size: row.get(9)?,
                mtime: row.get(10)?,
                columns,
            })
        })?;
        for row in rows {
            let row = row?;
            found.insert(row.node_id, row);
        }
    }

    // In the order the caller asked for. Every one of these sources *is* an ordering — most
    // recently touched, most connected, most salient — and returning them in whatever order
    // SQLite felt like would throw that away and leave the list looking arbitrary.
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = found.remove(id) {
            out.push(row);
        }
    }
    Ok(out)
}

/// The nodes in a tier, most salient first.
///
/// For the one view that is *about* tiers (§4.4): everywhere else a tier is brightness, and here
/// it is the whole question — "what has been forgotten, and is any of it something I want back".
pub fn in_tier(store: &Store, tier: Tier, limit: usize) -> Result<Vec<NodeId>> {
    let conn = store.conn();
    let mut statement = conn.prepare_cached(
        "SELECT id FROM nodes
         WHERE tier = ?1 AND path IS NOT NULL
         ORDER BY salience DESC, path ASC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(
        rusqlite::params![tier.as_str(), limit as i64],
        |row| Ok(NodeId(row.get(0)?)),
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// How many nodes are in a tier. What the cold view's basis line counts before it says anything
/// about what "cold" means.
pub fn tier_total(store: &Store, tier: Tier) -> Result<u64> {
    let conn = store.conn();
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE tier = ?1 AND path IS NOT NULL",
        [tier.as_str()],
        |row| row.get(0),
    )?;
    Ok(total.max(0) as u64)
}

/// The concepts the last consolidation pass named, with how many members each has.
///
/// For the places screen: a concept is somewhere to go, and a list of them with no sizes would
/// make somebody open each one to find out which is worth opening.
pub fn concepts(store: &Store, limit: usize) -> Result<Vec<(NodeId, String, u64)>> {
    let conn = store.conn();
    let mut statement = conn.prepare_cached(
        "SELECT n.id, COALESCE(n.label, '?'),
                (SELECT COUNT(*) FROM edges e WHERE e.src = n.id AND e.type = 'derived_from')
         FROM nodes n
         WHERE n.kind = 'concept'
         ORDER BY 3 DESC, 2 ASC
         LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok((
            NodeId(row.get(0)?),
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?.max(0) as u64,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One node's columns, for a caller that has exactly one — the sidebar asking about a selection.
pub fn for_path(store: &Store, path: &Path) -> Result<Option<MemoryColumns>> {
    let conn = store.conn();
    let mut statement = conn.prepare_cached(
        "SELECT n.id, n.path, n.tier, n.salience, n.pinned, n.zone_id,
                (SELECT COUNT(*) FROM edges e WHERE e.src = n.id)
              + (SELECT COUNT(*) FROM edges e WHERE e.dst = n.id) AS degree,
                (SELECT MAX(ep.ts) FROM episodes ep WHERE ep.node_id = n.id) AS last_episode
         FROM nodes n WHERE n.path = ?1",
    )?;
    let found = statement
        .query_row([path.to_string_lossy().to_string()], row_to_columns)
        .optional()?;
    Ok(found.map(|(_, columns)| columns))
}

fn row_to_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<(PathBuf, MemoryColumns)> {
    let path: String = row.get(1)?;
    let tier: String = row.get(2)?;
    Ok((
        PathBuf::from(path),
        MemoryColumns {
            node_id: NodeId(row.get(0)?),
            tier: Tier::parse(&tier).unwrap_or(Tier::Stm),
            salience: row.get(3)?,
            pinned: row.get::<_, i64>(4)? != 0,
            zone_id: row.get::<_, Option<i64>>(5)?.map(ZoneId),
            degree: row.get::<_, i64>(6)?.max(0) as u32,
            last_episode: row.get::<_, Option<i64>>(7)?,
        },
    ))
}
