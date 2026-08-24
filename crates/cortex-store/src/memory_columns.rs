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
