//! The counting behind the start screen.
//!
//! One call, one connection, one point in time. Every number here is a `count(*)` or a `sum()`
//! over a table the daemon owns, and every derived value the window displays — shares,
//! coverage, refusal rates — is computed here rather than there (D4: the frontend paints, the
//! daemon counts).
//!
//! Two rules this module inherits and must not break:
//!
//! * **Nothing that needs its own connection while this one is held.** `last_consolidation`
//!   and `agent_sessions` each take a connection of their own, so they run first. A store with
//!   one connection — every in-memory store, and the first one every test gets — deadlocks
//!   rather than waits, and does it silently (see `Store::graph_snapshot`).
//! * **A breakdown keeps the order its reader needs, not the order SQL produced.** Two refreshes
//!   of the same screen have to be comparable bar for bar; a `GROUP BY` whose row order depends
//!   on the page cache is not.

use crate::error::Result;
use crate::Store;
use cortex_core::overview::{
    AgentFacts, ConsolidationFacts, DaemonFacts, IndexFacts, MemoryFacts, Overview, SessionFacts,
    Slice, ToolFacts,
};
use rusqlite::Connection;

/// How many sessions, tools and edge types the start screen lists. Enough to recognise what has
/// been running, few enough that the answer stays one screen.
const SESSIONS_SHOWN: usize = 6;
const TOOLS_SHOWN: usize = 8;
const ACTORS_SHOWN: usize = 6;
const EDGE_TYPES_SHOWN: usize = 5;
const SCOPES_SHOWN: usize = 6;

impl Store {
    /// Everything the start screen shows, as of `now`, for a window of `window_secs`.
    ///
    /// `daemon` comes back empty: only the daemon knows its own uptime and whether the Agent
    /// Surface is open, and it fills that block in after this returns.
    pub fn overview(&self, now: i64, window_secs: i64) -> Result<Overview> {
        // Before the connection is taken — see the module comment.
        let report = self.last_consolidation()?;
        let sessions = self.agent_sessions(SESSIONS_SHOWN)?;

        let conn = self.conn();
        let since = now - window_secs;

        let consolidation = report.map(|r| {
            let total: i64 = r.tier_totals.values().map(|v| *v as i64).sum();
            ConsolidationFacts {
                ran_at: r.ran_at,
                // A report from the future is a clock change, not a scheduled pass: say zero
                // rather than a negative age the window would have to render as „-4 min".
                ago_secs: (now - r.ran_at).max(0),
                dry_run: r.dry_run,
                episodes_replayed: r.episodes_replayed,
                bursts: r.bursts,
                bursts_discarded: r.bursts_discarded,
                edges_written: r.edges_written,
                movements: r.moved(),
                pinned_held: r.pinned_held,
                concepts_total: r.concepts_total,
                concepts_pruned: r.concepts_pruned,
                orphans: r.orphans,
                duplicates: r.duplicates,
                memories_expired: r.memories_expired,
                memories_total: r.memories_total,
                duration_ms: r.duration_ms,
                tier_totals: r
                    .tier_totals
                    .iter()
                    .map(|(name, count)| Slice::new(name.as_str(), *count as i64, total))
                    .collect(),
            }
        });

        let mut overview = Overview {
            generated_at: now,
            window_secs,
            window_label: Overview::window_label(window_secs),
            headline: String::new(),
            basis: String::new(),
            index: index_facts(&conn, since)?,
            memory: memory_facts(&conn, now)?,
            agents: agent_facts(&conn, since, sessions)?,
            consolidation,
            daemon: DaemonFacts::default(),
        };
        overview.compose();
        Ok(overview)
    }
}

fn index_facts(conn: &Connection, since: i64) -> Result<IndexFacts> {
    let kinds = grouped(conn, "SELECT kind, count(*) FROM nodes GROUP BY kind", &[])?;
    let of_kind = |want: &str| -> i64 {
        kinds
            .iter()
            .find(|(name, _)| name == want)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };
    let files = of_kind("file");
    let dirs = of_kind("dir");

    let tiers = grouped(
        conn,
        "SELECT tier, count(*) FROM nodes WHERE kind IN ('file','dir') GROUP BY tier",
        &[],
    )?;
    let extract = grouped(
        conn,
        "SELECT extract_state, count(*) FROM nodes WHERE kind = 'file' GROUP BY extract_state",
        &[],
    )?;
    let edges = grouped(
        conn,
        "SELECT type, count(*) FROM edges GROUP BY type ORDER BY count(*) DESC, type ASC",
        &[],
    )?;
    let actors = grouped(
        conn,
        "SELECT actor, count(*) FROM episodes WHERE ts >= ?1 GROUP BY actor \
         ORDER BY count(*) DESC, actor ASC",
        &[&since],
    )?;

    let chunks = count(conn, "SELECT count(*) FROM chunks", &[])?;
    let embeddings = count(conn, "SELECT count(*) FROM embeddings", &[])?;
    let edge_total: i64 = edges.iter().map(|(_, n)| *n).sum();

    Ok(IndexFacts {
        files,
        dirs,
        chunks,
        embeddings,
        // Vectors per chunk, capped: after a rebuild the index can briefly hold a vector the
        // table no longer lists (D107), and „102 % der Chunks eingebettet" is nonsense on a
        // screen.
        vector_coverage: cortex_core::overview::share(embeddings.min(chunks), chunks),
        episodes: count(conn, "SELECT count(*) FROM episodes", &[])?,
        episodes_window: count(
            conn,
            "SELECT count(*) FROM episodes WHERE ts >= ?1",
            &[&since],
        )?,
        concepts: of_kind("concept"),
        annotations: of_kind("annotation"),
        edges: edge_total,
        zones: count(conn, "SELECT count(*) FROM zones", &[])?,
        pinned: count(conn, "SELECT count(*) FROM nodes WHERE pinned = 1", &[])?,
        duplicates: count(
            conn,
            "SELECT coalesce(sum(c - 1), 0) FROM (
                 SELECT count(*) AS c FROM nodes
                 WHERE kind = 'file' AND content_hash IS NOT NULL
                 GROUP BY content_hash HAVING c > 1
             )",
            &[],
        )?,
        vectors_to_drop: count(conn, "SELECT count(*) FROM dead_chunks", &[])?,
        tiers: breakdown(&tiers, &["working", "stm", "ltm", "cold"], files + dirs),
        extract: breakdown(&extract, &["pending", "done", "skipped", "failed"], files),
        edge_types: breakdown(&edges, &[], edge_total)
            .into_iter()
            .take(EDGE_TYPES_SHOWN)
            .collect(),
        actors_window: breakdown(&actors, &[], actors.iter().map(|(_, n)| *n).sum())
            .into_iter()
            .take(ACTORS_SHOWN)
            .collect(),
    })
}

fn memory_facts(conn: &Connection, now: i64) -> Result<MemoryFacts> {
    let kinds = grouped(conn, "SELECT kind, count(*) FROM memories GROUP BY kind", &[])?;
    let scopes = grouped(
        conn,
        "SELECT scope, count(*) FROM memories GROUP BY scope ORDER BY count(*) DESC, scope ASC",
        &[],
    )?;
    let total = count(conn, "SELECT count(*) FROM memories", &[])?;

    Ok(MemoryFacts {
        total,
        human_confirmed: count(conn, "SELECT count(*) FROM memories WHERE human_confirmed = 1", &[])?,
        superseded: count(
            conn,
            "SELECT count(*) FROM memories WHERE superseded_by IS NOT NULL",
            &[],
        )?,
        pinned: count(
            conn,
            "SELECT count(*) FROM memories m JOIN nodes n ON n.id = m.node_id \
             WHERE n.pinned = 1",
            &[],
        )?,
        expiring: count(
            conn,
            "SELECT count(*) FROM memories WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            &[&now],
        )?,
        helped: count(conn, "SELECT coalesce(sum(helped_count), 0) FROM memories", &[])?,
        noise: count(conn, "SELECT coalesce(sum(noise_count), 0) FROM memories", &[])?,
        retrieved: count(conn, "SELECT coalesce(sum(retrieved_count), 0) FROM memories", &[])?,
        never_retrieved: count(
            conn,
            "SELECT count(*) FROM memories WHERE retrieved_count = 0",
            &[],
        )?,
        kinds: breakdown(
            &kinds,
            &[
                "fact",
                "preference",
                "procedure",
                "decision",
                "failure",
                "observation",
            ],
            total,
        ),
        scopes: breakdown(&scopes, &[], total)
            .into_iter()
            .take(SCOPES_SHOWN)
            .collect(),
    })
}

fn agent_facts(
    conn: &Connection,
    since: i64,
    sessions: Vec<cortex_core::memory::AgentSession>,
) -> Result<AgentFacts> {
    let (calls_window, refusals_window) = pair(
        conn,
        "SELECT count(*), coalesce(sum(allowed = 0), 0) FROM audit_log WHERE ts >= ?1",
        &[&since],
    )?;
    let (calls_total, refusals_total) = pair(
        conn,
        "SELECT count(*), coalesce(sum(allowed = 0), 0) FROM audit_log",
        &[],
    )?;

    let tools = {
        let mut stmt = conn.prepare(
            "SELECT tool, count(*), coalesce(sum(allowed = 0), 0) FROM audit_log
             WHERE ts >= ?1 GROUP BY tool ORDER BY count(*) DESC, tool ASC",
        )?;
        let rows = stmt.query_map(&[&since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let sessions = sessions
        .into_iter()
        .map(|s| SessionFacts {
            refusal_share: cortex_core::overview::share(s.refusals, s.calls),
            id: s.id,
            name: s.name,
            task: s.task,
            started_at: s.started_at,
            ended_at: s.ended_at,
            calls: s.calls,
            refusals: s.refusals,
            memories: s.memories,
        })
        .collect();

    let (packs_window, packs_total) = pair(
        conn,
        "SELECT coalesce(sum(created_at >= ?1), 0), count(*) FROM context_packs",
        &[&since],
    )?;
    let (pack_items, pack_helped, pack_noise) = trio(
        conn,
        "SELECT count(*), coalesce(sum(reported = 'helped'), 0),
                coalesce(sum(reported = 'noise'), 0)
         FROM context_pack_items",
        &[],
    )?;

    Ok(AgentFacts {
        sessions_total: count(conn, "SELECT count(*) FROM agent_sessions", &[])?,
        sessions_open: count(
            conn,
            "SELECT count(*) FROM agent_sessions WHERE ended_at IS NULL",
            &[],
        )?,
        sessions_window: count(
            conn,
            "SELECT count(*) FROM agent_sessions WHERE started_at >= ?1",
            &[&since],
        )?,
        sessions,
        calls_window,
        refusals_window,
        refusal_share: cortex_core::overview::share(refusals_window, calls_window),
        calls_total,
        refusals_total,
        tools: tools
            .into_iter()
            .take(TOOLS_SHOWN)
            .map(|(tool, calls, refusals)| ToolFacts {
                tool,
                calls,
                refusals,
                share: cortex_core::overview::share(calls, calls_window),
            })
            .collect(),
        packs_window,
        packs_total,
        pack_items,
        // Outcomes arrive after the pack did, and a pack from yesterday can be reported on
        // today — so these two are cumulative by design, and the window does not apply.
        pack_helped,
        pack_noise,
    })
}

fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<i64> {
    Ok(conn.query_row(sql, params, |r| r.get(0))?)
}

fn pair(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<(i64, i64)> {
    Ok(conn.query_row(sql, params, |r| Ok((r.get(0)?, r.get(1)?)))?)
}

fn trio(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<(i64, i64, i64)> {
    Ok(conn.query_row(sql, params, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?)
}

fn grouped(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Grouped counts in the order the caller names, each with its share of `total`.
///
/// Anything the caller did not name is appended rather than dropped: the vocabularies this is
/// used for (tiers, extract states, memory kinds) are `CHECK` constraints that grow, and a new
/// value appearing in the data must appear on the screen instead of vanishing from a total it
/// is still counted in.
fn breakdown(rows: &[(String, i64)], order: &[&str], total: i64) -> Vec<Slice> {
    let mut out: Vec<Slice> = Vec::with_capacity(rows.len());
    for name in order {
        for (found, count) in rows {
            if found == name {
                out.push(Slice::new(*name, *count, total));
            }
        }
    }
    for (name, count) in rows {
        if !order.contains(&name.as_str()) {
            out.push(Slice::new(name.as_str(), *count, total));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::ChunkWrite;
    use crate::memory::ConsolidateParams;
    use crate::memories::NewMemory;
    use crate::queries::NodeUpsert;
    use cortex_core::ids::NodeId;
    use cortex_core::memory::{MemoryKind, MemoryScope};
    use cortex_core::model::{Actor, ExtractState, NodeKind};
    use std::path::PathBuf;

    const NOW: i64 = 1_700_000_000;
    const DAY: i64 = 86_400;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn file(store: &Store, path: &str) -> NodeId {
        store
            .upsert_node(
                &NodeUpsert {
                    kind: NodeKind::File,
                    path: PathBuf::from(path),
                    parent_id: None,
                    mime: None,
                    size: 0,
                    mtime: 0,
                    ctime: 0,
                    inode: 0,
                    dev: 0,
                    zone_id: None,
                },
                NOW,
            )
            .unwrap()
    }

    fn extracted(store: &Store, path: &str, hash: &str) -> NodeId {
        let node = file(store, path);
        store
            .batch(|b| b.set_extraction_result(node, Some(hash), None, ExtractState::Done, None))
            .unwrap();
        node
    }

    fn chunk(text: &str) -> ChunkWrite {
        ChunkWrite {
            ordinal: 0,
            byte_start: 0,
            byte_end: text.len() as i64,
            hash: "chunkhash".to_string(),
            preview: Some(text.to_string()),
            text: text.to_string(),
        }
    }

    #[test]
    fn an_empty_index_reports_zeroes_and_still_says_what_it_is() {
        let store = store();
        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.index.files, 0);
        assert_eq!(o.index.vector_coverage, 0.0);
        assert_eq!(o.agents.calls_window, 0);
        assert_eq!(o.agents.refusal_share, 0.0, "no calls is not a 0 % refusal rate");
        assert!(o.headline.contains("Kein Agent"), "{}", o.headline);
        assert!(o.basis.contains("keine Schätzung"), "{}", o.basis);
        assert!(o.consolidation.is_none());
        assert!(o.daemon.version.is_empty(), "the store fills no daemon facts");
    }

    #[test]
    fn the_index_block_counts_what_is_there_and_in_what_state() {
        let store = store();
        file(&store, "/a.md");
        extracted(&store, "/b.md", "h2");
        let third = file(&store, "/c.md");
        store
            .set_extract_state(third, ExtractState::Skipped, Some("kein Text"))
            .unwrap();

        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.index.files, 3);

        // The order is the reader's and is stable between refreshes, so two screenshots of the
        // same screen can be compared.
        let states: Vec<(&str, i64)> = o
            .index
            .extract
            .iter()
            .map(|s| (s.name.as_str(), s.count))
            .collect();
        assert_eq!(states, vec![("pending", 1), ("done", 1), ("skipped", 1)]);

        let done = o.index.extract.iter().find(|s| s.name == "done").unwrap();
        assert!((done.share - 1.0 / 3.0).abs() < 1e-9, "{done:?}");

        // New files land in `stm`; nothing has consolidated them yet.
        assert_eq!(o.index.tiers.len(), 1);
        assert_eq!(o.index.tiers[0].name, "stm");
        assert_eq!(o.index.tiers[0].count, 3);
    }

    #[test]
    fn a_copy_of_a_file_is_counted_as_one_duplicate() {
        let store = store();
        extracted(&store, "/a.md", "same");
        extracted(&store, "/copy-of-a.md", "same");
        extracted(&store, "/b.md", "other");
        // A pending file has no hash yet and must not be invented as a duplicate of another
        // pending file.
        file(&store, "/c.md");

        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.index.duplicates, 1);
    }

    #[test]
    fn vectors_waiting_to_be_dropped_are_reported_rather_than_hidden() {
        let store = store();
        let node = file(&store, "/a.md");
        store.replace_chunks(node, &[chunk("some text")]).unwrap();
        assert_eq!(store.overview(NOW, DAY).unwrap().index.chunks, 1);

        store.delete_chunks(node).unwrap();
        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.index.chunks, 0);
        // The vector of the deleted chunk is still in the ANN index until the next idle pass
        // (D107). The start screen has to be able to say so, because that number was once
        // behind a full rebuild every five minutes.
        assert!(o.index.vectors_to_drop >= 1, "{:?}", o.index);
    }

    #[test]
    fn agent_behaviour_comes_from_the_audit_log_and_the_window_is_a_time_cut() {
        let store = store();
        let call = |ts: i64, tool: &str, allowed: bool| {
            store
                .append_audit(ts, "agent:claude", tool, &serde_json::json!({}), Some("ok"), allowed)
                .unwrap()
        };
        call(NOW - 60, "fs.list", true);
        call(NOW - 30, "fs.write", false);
        call(NOW - 2 * DAY, "brain.context", true);

        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.agents.calls_window, 2);
        assert_eq!(o.agents.refusals_window, 1);
        assert!((o.agents.refusal_share - 0.5).abs() < 1e-9);
        assert_eq!(o.agents.calls_total, 3, "the total is not the window");
        assert_eq!(o.agents.refusals_total, 1);

        // Largest first, and the share is of the window rather than of the list.
        assert_eq!(o.agents.tools[0].tool, "fs.list");
        assert!((o.agents.tools[0].share - 0.5).abs() < 1e-9);
        assert_eq!(o.agents.tools[1].tool, "fs.write");
        assert_eq!(o.agents.tools[1].refusals, 1);

        assert!(o.headline.contains("2 Aufrufe"), "{}", o.headline);
        assert!(o.headline.contains("1 abgelehnt (50,0 %)"), "{}", o.headline);
        assert!(o.headline.contains("meistgenutzt: fs.list"), "{}", o.headline);
    }

    #[test]
    fn a_session_says_when_it_started_whether_it_is_open_and_what_it_cost() {
        let store = store();
        store
            .begin_agent_session("s-2", "claude", Some("aufräumen"), NOW - 100)
            .unwrap();
        store
            .begin_agent_session("s-1", "codex", None, NOW - 2 * DAY)
            .unwrap();
        store.end_agent_session("s-1", NOW - 2 * DAY + 60).unwrap();
        store.note_session_call("s-2", true).unwrap();
        store.note_session_call("s-2", false).unwrap();

        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.agents.sessions_total, 2);
        assert_eq!(o.agents.sessions_open, 1);
        assert_eq!(o.agents.sessions_window, 1, "only one started inside the window");
        assert_eq!(o.agents.sessions[0].id, "s-2", "newest first");
        assert_eq!(o.agents.sessions[0].task.as_deref(), Some("aufräumen"));
        assert!((o.agents.sessions[0].refusal_share - 0.5).abs() < 1e-9);
        assert!(o.agents.sessions[1].ended_at.is_some());
    }

    #[test]
    fn memories_are_counted_by_kind_and_scope_and_are_never_netted() {
        let store = store();
        let agent = Actor::Agent("claude".to_string());
        store
            .remember(
                &NewMemory::new(
                    MemoryScope::Agent("claude".into()),
                    MemoryKind::Fact,
                    "die Referenzkorpora werden erzeugt, nie eingecheckt",
                ),
                &agent,
                NOW,
            )
            .unwrap();
        store
            .remember(
                &NewMemory::new(
                    MemoryScope::Session("s-1".into()),
                    MemoryKind::Observation,
                    "dieser Ordner wird bei jedem Build neu erzeugt",
                ),
                &agent,
                NOW,
            )
            .unwrap();

        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.memory.total, 2);
        assert_eq!(o.memory.helped, 0);
        assert_eq!(o.memory.noise, 0);
        assert_eq!(o.memory.never_retrieved, 2);
        assert_eq!(o.memory.superseded, 0);
        let kinds: Vec<&str> = o.memory.kinds.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(kinds, vec!["fact", "observation"], "in the reader's order");
        let scopes: Vec<&str> = o.memory.scopes.iter().map(|s| s.name.as_str()).collect();
        assert!(scopes.contains(&"agent:claude"), "{scopes:?}");
        assert!(scopes.contains(&"session:s-1"), "{scopes:?}");
    }

    #[test]
    fn a_consolidation_report_becomes_numbers_the_screen_can_print() {
        let store = store();
        let report = store.consolidate(&ConsolidateParams::new(NOW)).unwrap();

        let o = store.overview(NOW, DAY).unwrap();
        let c = o.consolidation.expect("a pass was just recorded");
        assert_eq!(c.ran_at, report.ran_at);
        assert_eq!(c.ago_secs, 0);
        assert_eq!(c.concepts_total, report.concepts_total);
        assert_eq!(c.duplicates, report.duplicates);
        assert!(!c.dry_run);
        // And the basis stops claiming there has never been one.
        assert!(!o.basis.contains("noch keine Konsolidierung"), "{}", o.basis);
    }

    #[test]
    fn a_report_from_the_future_is_reported_as_just_now_rather_than_as_negative() {
        let store = store();
        store.consolidate(&ConsolidateParams::new(NOW + 600)).unwrap();
        let o = store.overview(NOW, DAY).unwrap();
        assert_eq!(o.consolidation.unwrap().ago_secs, 0);
    }
}
