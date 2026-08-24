//! The memory layer: what an agent knows that is not in a file.
//!
//! Everything here obeys one rule, and it is rule **R10**. A memory is not derivable from the
//! filesystem — the filesystem has never heard of it — so the authority is the `remember`
//! episode, and the `nodes` row, the `memories` row, the chunk and the vector are all a
//! *projection* of it. [`Store::replay_memories`] rebuilds the projection from the log. A
//! deletion is an assertion for the same reason: without a `forget_memory` episode the memory
//! comes back the first time the index is rebuilt, and the delete button was a lie (D78).
//!
//! The second rule is scope, and it lives in exactly one place: [`ReaderScope::visible_scopes`]
//! produces the `IN (...)` list, and every read here binds it. A query in this file that builds
//! its own scope predicate is a scope leak waiting to happen.
//!
//! What is deliberately *not* here: salience, decay, tiering, consolidation. A memory is a node
//! (D74), so it gets all of those from the machinery that already exists. If something in this
//! file starts to look like a second ranking engine, the design has gone wrong.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::{ChunkId, NodeId};
use cortex_core::memory::{Memory, MemoryKind, MemoryScope, PackArm, ReaderScope, Reporter};
use cortex_core::model::{Actor, EdgeSource, EdgeType};
use rusqlite::{params, params_from_iter, OptionalExtension};
use std::path::PathBuf;

/// The longest memory that will be accepted.
///
/// A memory is a sentence or a paragraph. Something longer is a document, and a document belongs
/// on the filesystem where the extractor can chunk it properly — this layer stores one chunk per
/// memory on purpose, and a memory that needed several would be silently truncated by the
/// embedder instead of being refused here.
///
/// **1 400, not 4 000, and the number comes from the model rather than from taste.** The
/// embedding model takes 512 tokens; at the upward-biased 3.5 characters per token this system
/// budgets with, 1 400 characters is 400 tokens — comfortably inside it with room for the
/// tokeniser being less generous than the estimate on code or on German. A 4 000-character
/// memory would be accepted, stored whole, searched lexically in full, and embedded from its
/// first third only, so its vector would describe the beginning of something and its text would
/// say something else. Nothing would fail.
pub const MAX_MEMORY_CHARS: usize = 1_400;

/// How much of a memory is kept as the chunk preview.
const PREVIEW_CHARS: usize = 240;

/// What a caller asks to be remembered.
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub text: String,
    /// The node this is about, with the path it had. The path is not redundant: a rebuild
    /// renumbers every node, and an assertion that can only name its subject by a number is an
    /// assertion a rebuild cannot replay.
    pub subject: Option<(NodeId, String)>,
    pub confidence: f64,
    pub session_id: Option<String>,
    /// Only `session:` scope sets one. Everything else is forgotten by the decay engine.
    pub expires_at: Option<i64>,
    /// Memories this one replaces, by node id.
    pub supersedes: Vec<NodeId>,
}

impl NewMemory {
    pub fn new(scope: MemoryScope, kind: MemoryKind, text: impl Into<String>) -> Self {
        Self {
            scope,
            kind,
            text: text.into(),
            subject: None,
            confidence: 0.5,
            session_id: None,
            expires_at: None,
            supersedes: Vec::new(),
        }
    }
}

/// What one call to [`Store::remember`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Remembered {
    pub node_id: NodeId,
    pub chunk_id: ChunkId,
    /// True when the exact text already existed in the same scope and kind, and this call
    /// returned the existing memory instead of writing a second one.
    ///
    /// Deduplicating on exact text is safe and worth it: an agent re-running a task writes the
    /// same memory again, and two identical rows cost two slots in every pack forever. Anything
    /// less than exact is a judgement, and judgement belongs to the caller — see
    /// `near_duplicates`, which reports and does not act.
    pub deduplicated: bool,
    /// The memories this call superseded, as it was asked to.
    pub superseded: Vec<i64>,
}

/// One memory that matched a lexical query.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    pub node_id: NodeId,
    pub chunk_id: ChunkId,
    pub score: f64,
}

/// What to list.
#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    /// `None` means every scope, and is for the Inspector and the CLI — never for an agent.
    pub scopes: Option<Vec<String>>,
    pub kinds: Vec<MemoryKind>,
    /// Superseded memories are hidden unless this is set.
    pub include_superseded: bool,
    /// Expired session memories are hidden unless this is set. `now` decides.
    pub include_expired: bool,
    pub subject: Option<NodeId>,
    pub session_id: Option<String>,
    pub limit: usize,
    pub now: i64,
}

impl MemoryFilter {
    /// Everything a reader may see, live, newest first.
    pub fn visible_to(reader: &ReaderScope, now: i64) -> Self {
        Self {
            scopes: Some(reader.visible_scopes()),
            limit: 200,
            now,
            ..Default::default()
        }
    }

    /// Every memory in the index, including the superseded. The Inspector's view, and the
    /// human's — see D78 for why that is a different thing from an agent's.
    pub fn everything(now: i64) -> Self {
        Self {
            include_superseded: true,
            include_expired: true,
            limit: 1_000,
            now,
            ..Default::default()
        }
    }
}

impl Store {
    /// Write a memory: the episode, the node, the row, the chunk, and any supersession.
    ///
    /// One transaction, because a memory whose episode committed and whose row did not is a
    /// memory that appears out of nowhere on the next rebuild — and the reverse is a memory
    /// that vanishes on it.
    pub fn remember(&self, new: &NewMemory, actor: &Actor, now: i64) -> Result<Remembered> {
        let text = new.text.trim();
        if text.is_empty() {
            return Err(StoreError::corrupt("a memory with no text remembers nothing"));
        }
        if text.chars().count() > MAX_MEMORY_CHARS {
            return Err(StoreError::corrupt(format!(
                "a memory of {} characters is a document; the limit is {MAX_MEMORY_CHARS}, \
                 which is what fits in the embedding model's 512 tokens. Write the document to \
                 a file and remember one sentence about it.",
                text.chars().count()
            )));
        }
        if !(0.0..=1.0).contains(&new.confidence) {
            return Err(StoreError::corrupt(format!(
                "confidence {} is not a probability",
                new.confidence
            )));
        }

        let scope = new.scope.as_str();

        // Exact duplicate in the same scope and kind: hand back what is already there.
        if let Some(existing) = self.exact_memory(&scope, new.kind, text)? {
            return Ok(Remembered {
                node_id: existing.0,
                chunk_id: existing.1,
                deduplicated: true,
                superseded: Vec::new(),
            });
        }

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        tx.execute(
            "INSERT INTO nodes (kind, path, size, mtime, ctime, inode, dev,
                                first_seen, last_seen, tier, extract_state)
             VALUES ('memory', NULL, ?2, ?1, ?1, 0, 0, ?1, ?1, 'working', 'done')",
            params![now, text.len() as i64],
        )?;
        let node = NodeId(tx.last_insert_rowid());

        // The assertion. Written with the subject's *path*, not only its id.
        let metadata = serde_json::json!({
            "assertion": "remember",
            "scope": scope,
            "kind": new.kind.as_str(),
            "text": text,
            "confidence": new.confidence,
            "subject": new.subject.as_ref().map(|(id, _)| id.0),
            "subject_path": new.subject.as_ref().map(|(_, path)| path.clone()),
            "expires_at": new.expires_at,
            // Supersession targets are named by text-and-scope for the same reason: after a
            // rebuild the id in this episode points at nothing, or worse, at something else.
            "supersedes": self.supersede_keys_on(&tx, &new.supersedes)?,
        });
        tx.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, session_id, metadata_json)
             VALUES (?1, ?2, 'remember', ?3, ?4, ?5)",
            params![
                now,
                actor.as_str(),
                node.0,
                new.session_id,
                metadata.to_string()
            ],
        )?;
        let episode = tx.last_insert_rowid();

        let chunk = insert_memory_projection(
            &tx,
            node,
            new,
            text,
            &scope,
            episode,
            &actor.as_str(),
            now,
        )?;

        let mut superseded = Vec::new();
        for old in &new.supersedes {
            if old.0 == node.0 {
                continue;
            }
            let changed = tx.execute(
                "UPDATE memories SET superseded_by = ?2 WHERE node_id = ?1",
                params![old.0, node.0],
            )?;
            if changed > 0 {
                tx.execute(
                    "INSERT INTO edges (src, dst, type, weight, created_at, source)
                     VALUES (?1, ?2, 'supersedes', 1.0, ?3, 'explicit')
                     ON CONFLICT(src, dst, type) DO NOTHING",
                    params![node.0, old.0, now],
                )?;
                superseded.push(old.0);
            }
        }

        tx.commit()?;
        Ok(Remembered {
            node_id: node,
            chunk_id: chunk,
            deduplicated: false,
            superseded,
        })
    }

    /// Stable identifiers for supersession targets, for the episode metadata.
    fn supersede_keys_on(
        &self,
        conn: &rusqlite::Connection,
        targets: &[NodeId],
    ) -> Result<serde_json::Value> {
        memory_keys_on(conn, targets)
    }

    fn exact_memory(
        &self,
        scope: &str,
        kind: MemoryKind,
        text: &str,
    ) -> Result<Option<(NodeId, ChunkId)>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT m.node_id, c.id
                 FROM memories m JOIN chunks c ON c.node_id = m.node_id AND c.ordinal = 0
                 WHERE m.scope = ?1 AND m.kind = ?2 AND m.text = ?3
                   AND m.superseded_by IS NULL
                 LIMIT 1",
                params![scope, kind.as_str(), text],
                |r| Ok((NodeId(r.get(0)?), ChunkId(r.get(1)?))),
            )
            .optional()?)
    }

    /// One memory, by node id. `None` when the node is not a memory.
    pub fn memory(&self, node: NodeId) -> Result<Option<Memory>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                &format!("{MEMORY_SELECT} WHERE m.node_id = ?1"),
                params![node.0],
                row_to_memory,
            )
            .optional()?)
    }

    /// The memory a chunk belongs to, for turning a vector hit back into a memory.
    pub fn memory_for_chunk(&self, chunk: ChunkId) -> Result<Option<NodeId>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT m.node_id FROM memories m
                 JOIN chunks c ON c.node_id = m.node_id
                 WHERE c.id = ?1",
                params![chunk.0],
                |r| Ok(NodeId(r.get(0)?)),
            )
            .optional()?)
    }

    /// List memories. The scope filter is the caller's, and it is not optional for an agent.
    pub fn list_memories(&self, filter: &MemoryFilter) -> Result<Vec<Memory>> {
        let conn = self.conn();
        let mut sql = String::from(MEMORY_SELECT);
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut clauses: Vec<String> = Vec::new();

        if let Some(scopes) = &filter.scopes {
            if scopes.is_empty() {
                return Ok(Vec::new());
            }
            let holes = vec!["?"; scopes.len()].join(", ");
            clauses.push(format!("m.scope IN ({holes})"));
            for scope in scopes {
                binds.push(Box::new(scope.clone()));
            }
        }
        if !filter.kinds.is_empty() {
            let holes = vec!["?"; filter.kinds.len()].join(", ");
            clauses.push(format!("m.kind IN ({holes})"));
            for kind in &filter.kinds {
                binds.push(Box::new(kind.as_str().to_string()));
            }
        }
        if !filter.include_superseded {
            clauses.push("m.superseded_by IS NULL".into());
        }
        if !filter.include_expired {
            clauses.push("(m.expires_at IS NULL OR m.expires_at > ?)".into());
            binds.push(Box::new(filter.now));
        }
        if let Some(subject) = filter.subject {
            clauses.push("m.subject_id = ?".into());
            binds.push(Box::new(subject.0));
        }
        if let Some(session) = &filter.session_id {
            clauses.push("m.session_id = ?".into());
            binds.push(Box::new(session.clone()));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY n.pinned DESC, n.salience DESC, m.ts DESC, m.node_id DESC LIMIT ?");
        binds.push(Box::new(filter.limit.max(1) as i64));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(binds.iter().map(|b| b.as_ref())), row_to_memory)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The lexical arm over memory text.
    ///
    /// Separate from [`Store::search_chunks`] rather than a flag on it, and the reason is the
    /// `path IS NOT NULL` in that query: file search is file search, and a memory has no path.
    /// Sharing the query would mean a predicate that means two different things depending on an
    /// argument, which is how the scope filter eventually gets forgotten.
    pub fn search_memories(
        &self,
        match_expr: &str,
        scopes: &[String],
        now: i64,
        limit: usize,
    ) -> Result<Vec<MemoryHit>> {
        self.search_memories_with(match_expr, scopes, now, limit, false)
    }

    /// The same, with the option of reaching superseded memories.
    ///
    /// A separate entry point rather than a fifth positional `bool` on the common one: the
    /// caller that wants history is rare and says so, and the caller that does not cannot get
    /// it by accident.
    pub fn search_memories_with(
        &self,
        match_expr: &str,
        scopes: &[String],
        now: i64,
        limit: usize,
        include_superseded: bool,
    ) -> Result<Vec<MemoryHit>> {
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn();
        let holes = vec!["?"; scopes.len()].join(", ");
        let supersession = if include_superseded {
            ""
        } else {
            "AND m.superseded_by IS NULL"
        };
        let sql = format!(
            "SELECT m.node_id, c.id, bm25(chunks_fts)
             FROM chunks_fts
             JOIN chunks   c ON c.id = chunks_fts.rowid
             JOIN memories m ON m.node_id = c.node_id
             WHERE chunks_fts MATCH ?
               AND m.scope IN ({holes})
               {supersession}
               AND (m.expires_at IS NULL OR m.expires_at > ?)
             ORDER BY bm25(chunks_fts)
             LIMIT ?"
        );
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(match_expr.to_string())];
        for scope in scopes {
            binds.push(Box::new(scope.clone()));
        }
        binds.push(Box::new(now));
        binds.push(Box::new(limit.max(1) as i64));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params_from_iter(binds.iter().map(|b| b.as_ref())),
            |r| {
                let bm25: f64 = r.get(2)?;
                Ok(MemoryHit {
                    node_id: NodeId(r.get(0)?),
                    chunk_id: ChunkId(r.get(1)?),
                    // FTS5 returns a negative score where more negative is better; flipped
                    // here, once, exactly as the file arm does.
                    score: -bm25,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Memories whose text is close enough to `text` to be worth a human's attention.
    ///
    /// Reports; does not act. Deciding that two statements contradict each other is a language
    /// model's job and there is no language model in this daemon (D68), so this returns
    /// candidates and the caller — an agent, or a person in the Inspector — decides.
    pub fn near_duplicate_memories(
        &self,
        scope: &MemoryScope,
        kind: MemoryKind,
        text: &str,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{MEMORY_SELECT} WHERE m.scope = ?1 AND m.kind = ?2 AND m.superseded_by IS NULL
             ORDER BY m.ts DESC LIMIT 500"
        ))?;
        let rows = stmt.query_map(params![scope.as_str(), kind.as_str()], row_to_memory)?;
        let candidates: Vec<Memory> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        let mut scored: Vec<(f64, Memory)> = candidates
            .into_iter()
            .map(|m| (trigram_similarity(text, &m.text), m))
            .filter(|(score, _)| *score >= NEAR_DUPLICATE_THRESHOLD)
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.node_id.cmp(&b.1.node_id)));
        Ok(scored.into_iter().take(limit).map(|(_, m)| m).collect())
    }

    /// Mark one memory as replaced by another, without writing a new memory.
    ///
    /// The Inspector's path, and the human's: a memory that is simply wrong is superseded by
    /// the one that is right, and both stay in the log.
    pub fn supersede_memory(
        &self,
        old: NodeId,
        new: NodeId,
        actor: &Actor,
        now: i64,
    ) -> Result<bool> {
        if old == new {
            return Ok(false);
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let keys: Option<(String, String, String)> = tx
            .query_row(
                "SELECT scope, kind, text FROM memories WHERE node_id = ?1",
                params![old.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((scope, kind, text)) = keys else {
            return Ok(false);
        };
        let replacement: Option<(String, String, String)> = tx
            .query_row(
                "SELECT scope, kind, text FROM memories WHERE node_id = ?1",
                params![new.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((new_scope, new_kind, new_text)) = replacement else {
            return Ok(false);
        };

        tx.execute(
            "UPDATE memories SET superseded_by = ?2 WHERE node_id = ?1",
            params![old.0, new.0],
        )?;
        tx.execute(
            "INSERT INTO edges (src, dst, type, weight, created_at, source)
             VALUES (?1, ?2, 'supersedes', 1.0, ?3, 'explicit')
             ON CONFLICT(src, dst, type) DO NOTHING",
            params![new.0, old.0, now],
        )?;
        let metadata = serde_json::json!({
            "assertion": "supersede",
            "old": { "scope": scope, "kind": kind, "text": text },
            "new": { "scope": new_scope, "kind": new_kind, "text": new_text },
        });
        tx.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, metadata_json)
             VALUES (?1, ?2, 'supersede', ?3, ?4)",
            params![now, actor.as_str(), old.0, metadata.to_string()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Delete a memory, and record the deletion as an assertion.
    ///
    /// The episode is the point. Deleting the rows alone works until the next `rebuild`, which
    /// replays the `remember` assertion and puts the memory back — so a delete that is not
    /// itself in the log is a delete that undoes itself (D78).
    pub fn forget_memory(&self, node: NodeId, actor: &Actor, now: i64) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let keys: Option<(String, String, String)> = tx
            .query_row(
                "SELECT scope, kind, text FROM memories WHERE node_id = ?1",
                params![node.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((scope, kind, text)) = keys else {
            return Ok(false);
        };

        let metadata = serde_json::json!({
            "assertion": "forget_memory",
            "scope": scope, "kind": kind, "text": text,
        });
        tx.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, metadata_json)
             VALUES (?1, ?2, 'forget_memory', ?3, ?4)",
            params![now, actor.as_str(), node.0, metadata.to_string()],
        )?;

        // The chunk cascades with the node; the FTS row does not, because a contentless index
        // has no foreign key to cascade through.
        let chunks: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM chunks WHERE node_id = ?1")?;
            let rows = stmt.query_map(params![node.0], |r| r.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for chunk in &chunks {
            tx.execute("DELETE FROM chunks_fts WHERE rowid = ?1", params![chunk])?;
        }
        tx.execute("DELETE FROM nodes WHERE id = ?1", params![node.0])?;
        tx.commit()?;
        Ok(true)
    }

    /// Remove expired session memories. Returns how many went.
    ///
    /// Only `session:` scope expires on a clock. Everything else is the decay engine's
    /// business, which is the difference between scratch and forgetting.
    pub fn expire_memories(&self, now: i64) -> Result<usize> {
        let expired: Vec<i64> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT node_id FROM memories WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            )?;
            let rows = stmt.query_map(params![now], |r| r.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let count = expired.len();
        for node in expired {
            // Expiry is the system forgetting scratch it promised to forget, not a deletion
            // somebody has to be able to audit. It still writes the assertion, because a
            // rebuild that replays `remember` without it would resurrect every session memory
            // this daemon has ever held.
            self.forget_memory(NodeId(node), &Actor::System, now)?;
        }
        Ok(count)
    }

    /// How many memories the index holds, live and superseded alike.
    pub fn count_memories(&self) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn.query_row("SELECT count(*) FROM memories", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// What [`Store::expire_memories`] would remove, without removing it. For a dry run.
    pub fn expiring_memory_count(&self, now: i64) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM memories WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            params![now],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// What [`Store::decay_unreported`] would penalise, without penalising it.
    pub fn unreported_memory_count(&self, min_retrievals: i64) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM memories
             WHERE retrieved_count >= ?1 AND helped_count = 0 AND noise_count = 0
               AND superseded_by IS NULL",
            params![min_retrievals],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// Note that these memories were placed into a pack. Not an outcome — a retrieval.
    pub fn note_retrieved(&self, nodes: &[NodeId]) -> Result<()> {
        if nodes.is_empty() {
            return Ok(());
        }
        let conn = self.conn();
        let holes = vec!["?"; nodes.len()].join(", ");
        conn.execute(
            &format!(
                "UPDATE memories SET retrieved_count = retrieved_count + 1
                 WHERE node_id IN ({holes})"
            ),
            params_from_iter(nodes.iter().map(|n| n.0)),
        )?;
        Ok(())
    }

    /// Apply an outcome report to the memories a pack contained.
    ///
    /// The weighting is [`Reporter::weight`] and the reason is D76: an agent reporting on its
    /// own success is an unreliable narrator, so it is discounted rather than ignored. Counts
    /// are kept separately and never netted — a memory that is useful in one context and noise
    /// in another needs splitting, and a net of zero is exactly how that stays invisible.
    pub fn report_outcome(
        &self,
        pack_id: &str,
        helped: &[NodeId],
        noise: &[NodeId],
        reporter: Reporter,
        actor: &Actor,
        now: i64,
    ) -> Result<OutcomeApplied> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let known: bool = tx
            .query_row(
                "SELECT 1 FROM context_packs WHERE id = ?1",
                params![pack_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !known {
            return Err(StoreError::corrupt(format!(
                "no context pack {pack_id}; an outcome has to name the pack it is about"
            )));
        }

        let mut applied = OutcomeApplied::default();
        // What this report took back, so a replay can take it back too. A human correcting an
        // agent reverses the agent's contribution, and an episode that recorded only the new
        // verdict would rebuild into a state where both had been counted.
        let mut reversals: Vec<serde_json::Value> = Vec::new();

        for (nodes, verdict) in [(helped, Verdict::Helped), (noise, Verdict::Noise)] {
            for node in nodes {
                // What this pack already holds for this item, and who said it. The column is
                // `<reporter>:<verdict>` rather than a bare verdict, because "has this been
                // reported" and "who reported it" are different questions and the second one is
                // the one that decides whether a new report is allowed to change anything.
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT reported FROM context_pack_items
                         WHERE pack_id = ?1 AND node_id = ?2",
                        params![pack_id, node.0],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();

                let Some(previous) = existing.as_deref().map(parse_report) else {
                    // Not in the pack at all. A client bug; counted rather than accepted,
                    // because silently applying it would let a caller move any score it liked.
                    if existing.is_none()
                        && tx
                            .query_row(
                                "SELECT 1 FROM context_pack_items
                                 WHERE pack_id = ?1 AND node_id = ?2",
                                params![pack_id, node.0],
                                |_| Ok(true),
                            )
                            .optional()?
                            .is_none()
                    {
                        applied.ignored += 1;
                        continue;
                    }
                    // In the pack and never reported on: apply.
                    apply_report(&tx, pack_id, *node, reporter, verdict, None)?;
                    applied.count(verdict);
                    continue;
                };

                match previous {
                    // The same class of reporter reporting twice. A client that retries a
                    // failed call must not double its own vote.
                    Some((was, _)) if was == reporter => {
                        applied.ignored += 1;
                    }
                    // A human correcting an agent. **This is the case a first version got
                    // wrong**, and a live run found it: the guard against double-counting also
                    // silenced the human, which is the exact opposite of the weighting D76
                    // asks for. The agent's contribution is reversed and the human's applied,
                    // so the human's verdict is what stands whether they agree or not.
                    Some((Reporter::Agent, was_verdict)) if reporter == Reporter::Human => {
                        apply_report(&tx, pack_id, *node, reporter, verdict, Some(was_verdict))?;
                        let mut key = memory_keys_on(&tx, std::slice::from_ref(node))?;
                        if let Some(entry) = key.as_array_mut().and_then(|a| a.first_mut()) {
                            entry["verdict"] = serde_json::json!(was_verdict.as_str());
                            entry["reporter"] = serde_json::json!(Reporter::Agent.as_str());
                            reversals.push(entry.clone());
                        }
                        applied.count(verdict);
                    }
                    // An agent contradicting a human. Ignored: the human already looked.
                    Some((Reporter::Human, _)) => {
                        applied.ignored += 1;
                    }
                    Some(_) => {
                        applied.ignored += 1;
                    }
                    None => {
                        apply_report(&tx, pack_id, *node, reporter, verdict, None)?;
                        applied.count(verdict);
                    }
                }
            }
        }

        // The episode names the memories by what they *say*, not by their ids — the same rule
        // every other assertion here follows, and for the same reason: a rebuild renumbers, and
        // an id in an old episode points at nothing or, worse, at something else.
        //
        // This is what makes `usefulness` survive a rebuild. It is derived, and everything
        // derived has to be reconstructible from the log or R10 is a slogan: without this an
        // agent's whole record of what has been useful would silently reset the first time
        // somebody ran `cortex rebuild`, and nothing would have failed.
        // Memories that were useful *for the same task* belong together, and this is the only
        // moment anything knows that. Not "appeared in the same pack" — that is the ranking's
        // opinion about them, and linking on it would let the graph confirm whatever the
        // ranking already believed, which is the loop D76 is about. Being reported helpful
        // together is a judgement from outside it.
        //
        // The memory layer's answer to co-access, and sparser than co-access on purpose: a
        // pack shows a dozen things and a report names two or three.
        link_recalled_together(&tx, helped, now)?;

        let metadata = serde_json::json!({
            "assertion": "outcome",
            "pack": pack_id,
            "reporter": reporter.as_str(),
            "helped": memory_keys_on(&tx, helped)?,
            "noise": memory_keys_on(&tx, noise)?,
            "reversed": reversals,
        });
        tx.execute(
            "INSERT INTO episodes (ts, actor, action, node_id, metadata_json)
             VALUES (?1, ?2, 'outcome', NULL, ?3)",
            params![now, actor.as_str(), metadata.to_string()],
        )?;
        tx.commit()?;
        Ok(applied)
    }

    /// Lower the usefulness of memories that keep being retrieved and never reported.
    ///
    /// This is the half of D76 that is easy to leave out, and leaving it out is what makes a
    /// memory ranking self-confirming: a memory in every pack and in nobody's report is costing
    /// tokens for nothing, and no recency-or-frequency component will ever say so.
    ///
    /// Returns how many were penalised.
    pub fn decay_unreported(&self, min_retrievals: i64, step: f64) -> Result<usize> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE memories
             SET usefulness = usefulness - ?2
             WHERE retrieved_count >= ?1
               AND helped_count = 0
               AND noise_count = 0
               AND superseded_by IS NULL",
            params![min_retrievals, step],
        )?;
        Ok(changed)
    }
}

/// Name memories by what they say, for an episode that has to outlive their ids.
fn memory_keys_on(
    conn: &rusqlite::Connection,
    targets: &[NodeId],
) -> Result<serde_json::Value> {
    let mut out = Vec::new();
    for target in targets {
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT scope, kind, text FROM memories WHERE node_id = ?1",
                params![target.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((scope, kind, text)) = row {
            out.push(serde_json::json!({
                "scope": scope, "kind": kind, "text": text,
            }));
        }
    }
    Ok(serde_json::Value::Array(out))
}

/// Which way a report went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Helped,
    Noise,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Helped => "helped",
            Verdict::Noise => "noise",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "helped" => Verdict::Helped,
            "noise" => Verdict::Noise,
            _ => return None,
        })
    }

    /// The change to `usefulness` one report of this kind makes, before the reporter's weight.
    fn delta(self) -> f64 {
        match self {
            Verdict::Helped => 1.0,
            Verdict::Noise => -NOISE_PENALTY,
        }
    }

    fn column(self) -> &'static str {
        match self {
            Verdict::Helped => "helped_count",
            Verdict::Noise => "noise_count",
        }
    }
}

/// Read back a `<reporter>:<verdict>` mark. `None` for a row that has never been reported on.
fn parse_report(stored: &str) -> Option<(Reporter, Verdict)> {
    let (reporter, verdict) = stored.split_once(':')?;
    Some((Reporter::parse(reporter)?, Verdict::parse(verdict)?))
}

/// Apply one report, reversing `undo` first when a human is correcting an agent.
fn apply_report(
    conn: &rusqlite::Connection,
    pack_id: &str,
    node: NodeId,
    reporter: Reporter,
    verdict: Verdict,
    undo: Option<Verdict>,
) -> Result<()> {
    if let Some(previous) = undo {
        // Reverse exactly what the agent's report did: its weighted delta and its count. The
        // weight is the agent's, not the caller's, or a correction would leave a residue whose
        // size depended on who happened to make it.
        let column = previous.column();
        conn.execute(
            &format!(
                "UPDATE memories
                 SET usefulness = usefulness - ?2,
                     {column} = max(0, {column} - 1)
                 WHERE node_id = ?1"
            ),
            params![node.0, previous.delta() * Reporter::Agent.weight()],
        )?;
    }

    let column = verdict.column();
    conn.execute(
        &format!(
            "UPDATE memories
             SET usefulness = usefulness + ?2,
                 {column} = {column} + 1,
                 human_confirmed = max(human_confirmed, ?3)
             WHERE node_id = ?1"
        ),
        params![
            node.0,
            verdict.delta() * reporter.weight(),
            (reporter == Reporter::Human && verdict == Verdict::Helped) as i64
        ],
    )?;
    conn.execute(
        "UPDATE context_pack_items SET reported = ?3 WHERE pack_id = ?1 AND node_id = ?2",
        params![
            pack_id,
            node.0,
            format!("{}:{}", reporter.as_str(), verdict.as_str())
        ],
    )?;
    Ok(())
}

/// What an outcome report actually changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutcomeApplied {
    pub helped: usize,
    pub noise: usize,
    /// Named by the report but not in the pack, or already settled by somebody whose word
    /// outranks the caller's. Counted rather than refused, so that one stale id does not lose
    /// the rest of a report.
    pub ignored: usize,
}

impl OutcomeApplied {
    fn count(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Helped => self.helped += 1,
            Verdict::Noise => self.noise += 1,
        }
    }
}

/// Associate memories that one report called useful together.
///
/// Shared by the live call and the replay, because two implementations of "what a report means"
/// would give a rebuilt index a different graph from the one it replaced — and nothing would
/// notice, since the canonical dump does not walk edges.
fn link_recalled_together(
    conn: &rusqlite::Connection,
    helped: &[NodeId],
    now: i64,
) -> Result<()> {
    if helped.len() < 2 || helped.len() > MAX_RECALLED_TOGETHER {
        return Ok(());
    }
    for (index, first) in helped.iter().enumerate() {
        for second in helped.iter().skip(index + 1) {
            if first == second {
                continue;
            }
            // Lowest id first, so one pair is one edge however the report ordered it.
            let (src, dst) = if first.0 <= second.0 {
                (first.0, second.0)
            } else {
                (second.0, first.0)
            };
            conn.execute(
                "INSERT INTO edges (src, dst, type, weight, created_at, source)
                 VALUES (?1, ?2, 'recalled_with', 1.0, ?3, 'observed')
                 ON CONFLICT(src, dst, type)
                     DO UPDATE SET weight = min(edges.weight + 1.0, ?4)",
                params![src, dst, now, MAX_RECALLED_WEIGHT],
            )?;
        }
    }
    Ok(())
}

/// The largest `helped` list that produces association edges.
///
/// A report that names eight memories as useful is a report; one that names thirty is a client
/// ticking every box, and linking all four hundred and thirty-five pairs from it would bury the
/// ones that mean something. Above this the report still counts — every memory still earns its
/// usefulness — and only the edges are skipped.
const MAX_RECALLED_TOGETHER: usize = 8;

/// The most weight repeated co-usefulness can accumulate on one edge.
///
/// Bounded for the same reason salience is: an unbounded edge weight turns a traversal into a
/// list of favourites, and two memories that have been useful together twenty times are not
/// twenty times more related than two that have managed it four times.
const MAX_RECALLED_WEIGHT: f64 = 5.0;

/// How long a context pack's record is kept.
///
/// Thirty days. An outcome is reported minutes after the pack in practice, and the record
/// exists only so that the report can be attributed; keeping every pack for ever would make the
/// largest table in this index a log of things an agent was once shown.
pub const PACK_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// How much harder a noise report bites than a helpful one helps.
///
/// Above one on purpose. A memory that wasted a slot is more informative than one that filled
/// it acceptably: the pack was going to contain *something*, and "this was the wrong something"
/// is the only signal in the loop that can move a memory down.
const NOISE_PENALTY: f64 = 1.5;

/// Trigram overlap above which two memories are worth showing to a human as possibly the same.
///
/// Deliberately lexical. The embedding is a better similarity and it is also asynchronous — the
/// vector for a memory written a moment ago does not exist yet, and a supersession hint that
/// only works for old memories is a hint that never fires when it matters.
const NEAR_DUPLICATE_THRESHOLD: f64 = 0.55;

const MEMORY_SELECT: &str = "
    SELECT m.node_id, m.scope, m.kind, m.text, m.subject_id, s.path,
           m.confidence, m.usefulness, m.helped_count, m.noise_count, m.retrieved_count,
           m.human_confirmed, m.expires_at, m.superseded_by, m.session_id, m.actor, m.ts,
           n.salience, n.pinned
    FROM memories m
    JOIN nodes n ON n.id = m.node_id
    LEFT JOIN nodes s ON s.id = m.subject_id";

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let scope: String = row.get(1)?;
    let kind: String = row.get(2)?;
    Ok(Memory {
        node_id: row.get(0)?,
        // A scope that does not parse is a database written by something else. Falling back to
        // Global would make it visible to everybody, so it falls back to a session nobody has.
        scope: MemoryScope::parse(&scope)
            .unwrap_or_else(|| MemoryScope::Session(format!("unparseable:{scope}"))),
        kind: MemoryKind::parse(&kind).unwrap_or(MemoryKind::Observation),
        text: row.get(3)?,
        subject_id: row.get(4)?,
        subject_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
        confidence: row.get(6)?,
        usefulness: row.get(7)?,
        helped_count: row.get(8)?,
        noise_count: row.get(9)?,
        retrieved_count: row.get(10)?,
        human_confirmed: row.get::<_, i64>(11)? != 0,
        expires_at: row.get(12)?,
        superseded_by: row.get(13)?,
        session_id: row.get(14)?,
        actor: row.get(15)?,
        ts: row.get(16)?,
        salience: row.get(17)?,
        pinned: row.get::<_, i64>(18)? != 0,
    })
}

/// Write everything that is derived from a `remember` assertion: the row, the chunk, the FTS
/// entry and the subject edge.
///
/// Shared by [`Store::remember`] and the replay, because two implementations of "what a
/// remember episode means" is how a rebuild ends up producing a different index from the one it
/// replaced.
#[allow(clippy::too_many_arguments)]
fn insert_memory_projection(
    conn: &rusqlite::Connection,
    node: NodeId,
    new: &NewMemory,
    text: &str,
    scope: &str,
    episode: i64,
    actor: &str,
    now: i64,
) -> Result<ChunkId> {
    conn.execute(
        "INSERT INTO memories (node_id, scope, kind, text, subject_id, confidence,
                               expires_at, session_id, episode_id, actor, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            node.0,
            scope,
            new.kind.as_str(),
            text,
            new.subject.as_ref().map(|(id, _)| id.0),
            new.confidence,
            new.expires_at,
            new.session_id,
            episode,
            actor,
            now
        ],
    )?;

    let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
    let preview: String = text.chars().take(PREVIEW_CHARS).collect();
    conn.execute(
        "INSERT INTO chunks (node_id, ordinal, byte_start, byte_end, hash, text_preview)
         VALUES (?1, 0, 0, ?2, ?3, ?4)",
        params![node.0, text.len() as i64, hash, preview],
    )?;
    let chunk = ChunkId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO chunks_fts (rowid, text) VALUES (?1, ?2)",
        params![chunk.0, text],
    )?;

    if let Some((subject, _)) = &new.subject {
        conn.execute(
            "INSERT INTO edges (src, dst, type, weight, created_at, source)
             VALUES (?1, ?2, ?3, 1.0, ?4, ?5)
             ON CONFLICT(src, dst, type) DO NOTHING",
            params![
                node.0,
                subject.0,
                EdgeType::Annotates.as_str(),
                now,
                EdgeSource::Explicit.as_str()
            ],
        )?;
    }

    Ok(chunk)
}


// ---------------------------------------------------------------------------
// Context packs, and the sessions they belong to
// ---------------------------------------------------------------------------

/// One item as a pack recorded it. Enough to attribute an outcome reported ten minutes later,
/// by which time the ranking has moved — not least because the pack itself moved it.
#[derive(Debug, Clone, PartialEq)]
pub struct PackItemRecord {
    pub node_id: NodeId,
    pub rank: usize,
    pub arm: PackArm,
    pub score: f64,
}

pub use cortex_core::memory::AgentSession;

impl Store {
    /// Record a pack and its items, in one transaction.
    pub fn record_pack(
        &self,
        pack_id: &str,
        session_id: Option<&str>,
        task: &str,
        budget_tokens: usize,
        used_tokens: usize,
        items: &[PackItemRecord],
        now: i64,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO context_packs (id, session_id, task, budget_tokens, used_tokens,
                                        created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                pack_id,
                session_id,
                task,
                budget_tokens as i64,
                used_tokens as i64,
                now
            ],
        )?;
        for item in items {
            tx.execute(
                "INSERT INTO context_pack_items (pack_id, node_id, rank, arm, score)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(pack_id, node_id) DO NOTHING",
                params![
                    pack_id,
                    item.node_id.0,
                    item.rank as i64,
                    item.arm.as_str(),
                    item.score
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Drop packs older than `before`, and the items with them.
    ///
    /// Packs are bookkeeping, not memory: keeping every one forever would make the largest
    /// table in the index a log of things an agent was once shown. An outcome for a pack this
    /// removed is ignored rather than mis-attributed, which is the right failure.
    pub fn prune_packs(&self, before: i64) -> Result<usize> {
        let conn = self.conn();
        Ok(conn.execute(
            "DELETE FROM context_packs WHERE created_at < ?1",
            params![before],
        )?)
    }

    pub fn begin_agent_session(
        &self,
        id: &str,
        name: &str,
        task: Option<&str>,
        now: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_sessions (id, name, task, started_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO NOTHING",
            params![id, name, task, now],
        )?;
        Ok(())
    }

    /// Close a session. Idempotent: closing a closed session keeps the first end time, because
    /// a client that reconnects and tidies up should not rewrite when the work finished.
    pub fn end_agent_session(&self, id: &str, now: i64) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE agent_sessions SET ended_at = ?2 WHERE id = ?1 AND ended_at IS NULL",
            params![id, now],
        )?;
        Ok(changed > 0)
    }

    /// Count one call against a session. Cheap enough to do on every call, which is the point:
    /// a counter that is only updated sometimes is a counter nobody can reason about.
    pub fn note_session_call(&self, id: &str, allowed: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_sessions
             SET calls = calls + 1, refusals = refusals + ?2
             WHERE id = ?1",
            params![id, (!allowed) as i64],
        )?;
        Ok(())
    }

    /// Sessions, newest first.
    pub fn agent_sessions(&self, limit: usize) -> Result<Vec<AgentSession>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.task, s.started_at, s.ended_at, s.calls, s.refusals,
                    (SELECT count(*) FROM memories m WHERE m.session_id = s.id)
             FROM agent_sessions s
             ORDER BY s.started_at DESC, s.id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.max(1) as i64], |r| {
            Ok(AgentSession {
                id: r.get(0)?,
                name: r.get(1)?,
                task: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
                calls: r.get(5)?,
                refusals: r.get(6)?,
                memories: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Rebuild the memory layer from the episode log.
    ///
    /// This is R10 for everything an agent ever learned, and it is the reason `remember` writes
    /// an episode before it writes anything else. Replay order is episode order, so a
    /// supersession that came after the memory it replaced is applied after it, and a
    /// `forget_memory` at the end removes what it removed.
    ///
    /// Targets are resolved by (scope, kind, text) rather than by node id, because a rebuild
    /// renumbers every node and an id in an old episode points at nothing — or, worse, at
    /// something else.
    ///
    /// Returns (replayed, skipped).
    pub fn replay_memories(&self) -> Result<(usize, usize)> {
        let episodes: Vec<(i64, i64, String, String, String, Option<String>)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT id, ts, actor, action, metadata_json, session_id FROM episodes
                 WHERE action IN ('remember', 'supersede', 'forget_memory', 'outcome')
                 ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut replayed = 0;
        let mut skipped = 0;

        for (episode_id, ts, actor, action, metadata, session_id) in episodes {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&metadata) else {
                skipped += 1;
                continue;
            };
            match action.as_str() {
                "remember" => {
                    let (Some(scope), Some(kind), Some(text)) = (
                        value.get("scope").and_then(|v| v.as_str()),
                        value
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .and_then(MemoryKind::parse),
                        value.get("text").and_then(|v| v.as_str()),
                    ) else {
                        skipped += 1;
                        continue;
                    };
                    let Some(parsed_scope) = MemoryScope::parse(scope) else {
                        skipped += 1;
                        continue;
                    };
                    // A subject that no longer exists loses the edge, not the memory. The note
                    // may still be the most useful thing in the index.
                    let subject = value
                        .get("subject_path")
                        .and_then(|v| v.as_str())
                        .and_then(|path| {
                            self.node_by_path(std::path::Path::new(path))
                                .ok()
                                .flatten()
                                .map(|n| (n.id, path.to_string()))
                        });

                    let new = NewMemory {
                        scope: parsed_scope,
                        kind,
                        text: text.to_string(),
                        subject,
                        confidence: value
                            .get("confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.5),
                        session_id: session_id.clone(),
                        expires_at: value.get("expires_at").and_then(|v| v.as_i64()),
                        supersedes: Vec::new(),
                    };

                    let conn = self.conn();
                    conn.execute(
                        "INSERT INTO nodes (kind, path, size, mtime, ctime, inode, dev,
                                            first_seen, last_seen, tier, extract_state)
                         VALUES ('memory', NULL, ?2, ?1, ?1, 0, 0, ?1, ?1, 'working', 'done')",
                        params![ts, text.len() as i64],
                    )?;
                    let node = NodeId(conn.last_insert_rowid());
                    insert_memory_projection(
                        &conn, node, &new, text, scope, episode_id, &actor, ts,
                    )?;
                    drop(conn);

                    // Supersessions the write recorded, resolved by content.
                    if let Some(targets) = value.get("supersedes").and_then(|v| v.as_array()) {
                        for target in targets {
                            if let Some(old) = self.memory_by_content(target)? {
                                self.apply_supersession(old, node, ts)?;
                            }
                        }
                    }
                    replayed += 1;
                }
                "supersede" => {
                    let (Some(old), Some(new)) = (
                        self.memory_by_content(value.get("old").unwrap_or(&serde_json::Value::Null))?,
                        self.memory_by_content(value.get("new").unwrap_or(&serde_json::Value::Null))?,
                    ) else {
                        skipped += 1;
                        continue;
                    };
                    self.apply_supersession(old, new, ts)?;
                    replayed += 1;
                }
                "forget_memory" => {
                    let Some(node) = self.memory_by_content(&value)? else {
                        // Already gone, which is what a replay of a delete over an index that
                        // never had it looks like. Not an error.
                        skipped += 1;
                        continue;
                    };
                    let conn = self.conn();
                    let chunks: Vec<i64> = {
                        let mut stmt = conn.prepare("SELECT id FROM chunks WHERE node_id = ?1")?;
                        let rows = stmt.query_map(params![node.0], |r| r.get::<_, i64>(0))?;
                        rows.collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    for chunk in chunks {
                        conn.execute("DELETE FROM chunks_fts WHERE rowid = ?1", params![chunk])?;
                    }
                    conn.execute("DELETE FROM nodes WHERE id = ?1", params![node.0])?;
                    replayed += 1;
                }
                "outcome" => {
                    // What a memory has earned is derived, and this is where it is derived
                    // *from*. Applied in episode order, so a human's correction of an agent
                    // lands after the agent's report exactly as it did the first time.
                    let reporter = value
                        .get("reporter")
                        .and_then(|v| v.as_str())
                        .and_then(Reporter::parse)
                        .unwrap_or(Reporter::Agent);
                    let mut applied_any = false;
                    let mut helped_nodes: Vec<NodeId> = Vec::new();
                    // Reversals first, in the same order the live call applied them.
                    if let Some(entries) = value.get("reversed").and_then(|v| v.as_array()) {
                        for entry in entries {
                            let (Some(node), Some(verdict), Some(was)) = (
                                self.memory_by_content(entry)?,
                                entry
                                    .get("verdict")
                                    .and_then(|v| v.as_str())
                                    .and_then(Verdict::parse),
                                entry
                                    .get("reporter")
                                    .and_then(|v| v.as_str())
                                    .and_then(Reporter::parse),
                            ) else {
                                continue;
                            };
                            let conn = self.conn();
                            let column = verdict.column();
                            conn.execute(
                                &format!(
                                    "UPDATE memories
                                     SET usefulness = usefulness - ?2,
                                         {column} = max(0, {column} - 1)
                                     WHERE node_id = ?1"
                                ),
                                params![node.0, verdict.delta() * was.weight()],
                            )?;
                            applied_any = true;
                        }
                    }
                    for (field, verdict) in
                        [("helped", Verdict::Helped), ("noise", Verdict::Noise)]
                    {
                        let Some(entries) = value.get(field).and_then(|v| v.as_array()) else {
                            continue;
                        };
                        for entry in entries {
                            let Some(node) = self.memory_by_content(entry)? else {
                                continue;
                            };
                            if verdict == Verdict::Helped {
                                helped_nodes.push(node);
                            }
                            let conn = self.conn();
                            // The pack itself is gone — packs are bookkeeping and are pruned —
                            // so the score is applied directly rather than through the pack
                            // guard. That guard exists to stop a *client* voting twice; a
                            // replay of the log cannot, because the log holds each report once.
                            let column = verdict.column();
                            conn.execute(
                                &format!(
                                    "UPDATE memories
                                     SET usefulness = usefulness + ?2,
                                         {column} = {column} + 1,
                                         human_confirmed = max(human_confirmed, ?3)
                                     WHERE node_id = ?1"
                                ),
                                params![
                                    node.0,
                                    verdict.delta() * reporter.weight(),
                                    (reporter == Reporter::Human && verdict == Verdict::Helped)
                                        as i64
                                ],
                            )?;
                            applied_any = true;
                        }
                    }
                    // The associations this report produced, rebuilt from the same function the
                    // live call uses. A rebuilt index with a different graph from the one it
                    // replaced would pass the canonical dump, which does not walk edges.
                    link_recalled_together(&self.conn(), &helped_nodes, ts)?;
                    if applied_any {
                        replayed += 1;
                    } else {
                        skipped += 1;
                    }
                }
                _ => skipped += 1,
            }
        }

        Ok((replayed, skipped))
    }

    /// Find a memory by what it says rather than by its id, which a rebuild has changed.
    ///
    /// The newest match wins when a scope and kind hold the same text twice, which cannot
    /// happen through `remember` — it deduplicates — but can happen after a supersession, and
    /// picking the newest is the only choice that makes a replay converge.
    fn memory_by_content(&self, key: &serde_json::Value) -> Result<Option<NodeId>> {
        let (Some(scope), Some(kind), Some(text)) = (
            key.get("scope").and_then(|v| v.as_str()),
            key.get("kind").and_then(|v| v.as_str()),
            key.get("text").and_then(|v| v.as_str()),
        ) else {
            return Ok(None);
        };
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT node_id FROM memories
                 WHERE scope = ?1 AND kind = ?2 AND text = ?3
                 ORDER BY node_id DESC LIMIT 1",
                params![scope, kind, text],
                |r| Ok(NodeId(r.get(0)?)),
            )
            .optional()?)
    }

    fn apply_supersession(&self, old: NodeId, new: NodeId, now: i64) -> Result<()> {
        if old == new {
            return Ok(());
        }
        let conn = self.conn();
        conn.execute(
            "UPDATE memories SET superseded_by = ?2 WHERE node_id = ?1",
            params![old.0, new.0],
        )?;
        conn.execute(
            "INSERT INTO edges (src, dst, type, weight, created_at, source)
             VALUES (?1, ?2, 'supersedes', 1.0, ?3, 'explicit')
             ON CONFLICT(src, dst, type) DO NOTHING",
            params![new.0, old.0, now],
        )?;
        Ok(())
    }
}

/// Character-trigram Jaccard similarity, for the supersession hint only.
///
/// Not a retrieval score and never used as one. It exists because it works on text that was
/// written one millisecond ago, which is exactly when the embedding does not exist yet.
fn trigram_similarity(a: &str, b: &str) -> f64 {
    fn trigrams(s: &str) -> std::collections::HashSet<[char; 3]> {
        let chars: Vec<char> = s
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
    }
    let (left, right) = (trigrams(a), trigrams(b));
    if left.is_empty() || right.is_empty() {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    let shared = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    shared / union
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::NodeUpsert;
    use cortex_core::model::NodeKind;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn agent(name: &str) -> Actor {
        Actor::Agent(name.to_string())
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
                0,
            )
            .unwrap()
    }

    fn remember(store: &Store, scope: MemoryScope, kind: MemoryKind, text: &str) -> NodeId {
        store
            .remember(&NewMemory::new(scope, kind, text), &agent("a"), 100)
            .unwrap()
            .node_id
    }

    #[test]
    fn a_memory_is_a_node_with_a_chunk_and_a_row() {
        let store = store();
        let written = store
            .remember(
                &NewMemory::new(
                    MemoryScope::Agent("a".into()),
                    MemoryKind::Fact,
                    "the reference corpus is generated from a seed, never checked in",
                ),
                &agent("a"),
                100,
            )
            .unwrap();
        assert!(!written.deduplicated);

        let memory = store.memory(written.node_id).unwrap().unwrap();
        assert_eq!(memory.kind, MemoryKind::Fact);
        assert_eq!(memory.scope, MemoryScope::Agent("a".into()));
        assert_eq!(memory.usefulness, 0.0, "usefulness is earned, not given");
        // The chunk the write reported is the one the vector arm will find its way back from.
        assert_eq!(
            store.memory_for_chunk(written.chunk_id).unwrap(),
            Some(written.node_id)
        );
    }

    /// A context pack's record is bookkeeping and the consolidation pass takes out the old ones.
    ///
    /// The point of the test is not the SQL. It is that something *calls* it: `decay_unreported`
    /// spent its first hour written, tested and inert, and a table that only ever grows is a
    /// worse version of the same mistake.
    #[test]
    fn the_consolidation_pass_prunes_packs_it_no_longer_needs() {
        use crate::memory::ConsolidateParams;

        let store = store();
        let node = remember(&store, MemoryScope::Global, MemoryKind::Fact, "something");
        let now = 100 * 86_400;
        let old = now - PACK_RETENTION_SECS - 1;

        // Recorded with the test's own clock rather than through `pack_with`, whose fixed
        // timestamp is older than the retention window and would make "recent" a misnomer.
        store
            .record_pack(
                "recent",
                None,
                "just now",
                2048,
                50,
                &[PackItemRecord {
                    node_id: node,
                    rank: 0,
                    arm: PackArm::Lexical,
                    score: 1.0,
                }],
                now,
            )
            .unwrap();
        store
            .record_pack("ancient", None, "long ago", 2048, 50, &[], old)
            .unwrap();

        // A dry run counts and removes nothing. Asserted behaviourally — a pack that is still
        // there is a pack an outcome can be reported against, which is the only thing a pack is
        // for and therefore the only property worth checking.
        store
            .consolidate(&ConsolidateParams {
                dry_run: true,
                ..ConsolidateParams::new(now)
            })
            .unwrap();
        assert!(store
            .report_outcome("ancient", &[node], &[], Reporter::Human, &Actor::Human, now)
            .is_ok());

        store.consolidate(&ConsolidateParams::new(now)).unwrap();
        assert!(
            store
                .report_outcome("recent", &[node], &[], Reporter::Agent, &agent("a"), now)
                .is_ok(),
            "a pack inside the retention window was pruned"
        );
        assert!(
            store
                .report_outcome("ancient", &[node], &[], Reporter::Human, &Actor::Human, now)
                .is_err(),
            "a pack older than the retention window is still there"
        );
    }

    #[test]
    fn a_memory_the_model_could_not_embed_whole_is_refused() {
        // The limit is the embedding model's 512 tokens expressed in characters, not a taste.
        // A memory over it would be stored whole, searched lexically in full, and embedded from
        // its first part only — a vector describing the beginning of something whose text says
        // something else, with nothing failing anywhere.
        let store = store();
        let long = "a".repeat(MAX_MEMORY_CHARS + 1);
        let refused = store.remember(
            &NewMemory::new(MemoryScope::Global, MemoryKind::Fact, long),
            &agent("a"),
            1,
        );
        let message = refused.unwrap_err().to_string();
        assert!(
            message.contains("512 tokens"),
            "the refusal does not say why the limit is where it is: {message}"
        );
    }

    #[test]
    fn an_empty_memory_and_a_document_are_both_refused() {
        let store = store();
        assert!(store
            .remember(
                &NewMemory::new(MemoryScope::Global, MemoryKind::Fact, "   "),
                &agent("a"),
                1
            )
            .is_err());
        let long = "x".repeat(MAX_MEMORY_CHARS + 1);
        assert!(store
            .remember(
                &NewMemory::new(MemoryScope::Global, MemoryKind::Fact, long),
                &agent("a"),
                1
            )
            .is_err());
    }

    #[test]
    fn writing_the_same_memory_twice_returns_the_first_one() {
        let store = store();
        let first = store
            .remember(
                &NewMemory::new(MemoryScope::Global, MemoryKind::Procedure, "run the gate"),
                &agent("a"),
                1,
            )
            .unwrap();
        let second = store
            .remember(
                &NewMemory::new(MemoryScope::Global, MemoryKind::Procedure, "  run the gate "),
                &agent("a"),
                2,
            )
            .unwrap();
        assert_eq!(first.node_id, second.node_id);
        assert!(second.deduplicated);
        // …and the same text in another scope is a different memory, not the same one.
        let elsewhere = store
            .remember(
                &NewMemory::new(
                    MemoryScope::Agent("a".into()),
                    MemoryKind::Procedure,
                    "run the gate",
                ),
                &agent("a"),
                3,
            )
            .unwrap();
        assert_ne!(first.node_id, elsewhere.node_id);
    }

    #[test]
    fn one_agents_memory_is_not_listed_for_another() {
        let store = store();
        remember(&store, MemoryScope::Agent("a".into()), MemoryKind::Fact, "mine");
        remember(&store, MemoryScope::Agent("b".into()), MemoryKind::Fact, "theirs");
        remember(&store, MemoryScope::Global, MemoryKind::Fact, "everyones");

        let seen = store
            .list_memories(&MemoryFilter::visible_to(&ReaderScope::for_agent("a"), 200))
            .unwrap();
        let texts: Vec<&str> = seen.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"mine"));
        assert!(texts.contains(&"everyones"));
        assert!(!texts.contains(&"theirs"), "scope isolation leaked");
    }

    #[test]
    fn the_lexical_arm_over_memories_respects_scope_and_supersession() {
        let store = store();
        let mine = remember(
            &store,
            MemoryScope::Agent("a".into()),
            MemoryKind::Fact,
            "the watcher debounces for three hundred milliseconds",
        );
        remember(
            &store,
            MemoryScope::Agent("b".into()),
            MemoryKind::Fact,
            "the watcher debounces for three hundred milliseconds too",
        );

        let scopes = ReaderScope::for_agent("a").visible_scopes();
        let hits = store.search_memories("watcher", &scopes, 200, 10).unwrap();
        assert_eq!(hits.len(), 1, "another agent's memory was searchable");
        assert_eq!(hits[0].node_id, mine);

        // Supersede it, and it leaves retrieval without leaving the log.
        let replacement = remember(
            &store,
            MemoryScope::Agent("a".into()),
            MemoryKind::Fact,
            "the watcher debounces for five hundred milliseconds",
        );
        store
            .supersede_memory(mine, replacement, &agent("a"), 300)
            .unwrap();
        let hits = store.search_memories("watcher", &scopes, 400, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, replacement);
        assert!(store.memory(mine).unwrap().unwrap().superseded_by.is_some());
    }

    #[test]
    fn a_session_memory_expires_and_nothing_else_does() {
        let store = store();
        let scratch = store
            .remember(
                &NewMemory {
                    expires_at: Some(150),
                    ..NewMemory::new(
                        MemoryScope::Session("s-1".into()),
                        MemoryKind::Observation,
                        "the file is open in the editor",
                    )
                },
                &agent("a"),
                100,
            )
            .unwrap();
        let durable = remember(&store, MemoryScope::Global, MemoryKind::Fact, "durable");

        assert_eq!(store.expire_memories(140).unwrap(), 0, "expired early");
        assert_eq!(store.expire_memories(200).unwrap(), 1);
        assert!(store.memory(scratch.node_id).unwrap().is_none());
        assert!(store.memory(durable).unwrap().is_some());
    }

    #[test]
    fn a_memory_about_a_file_gets_an_edge_to_it() {
        let store = store();
        let target = file(&store, "/tmp/report.md");
        let written = store
            .remember(
                &NewMemory {
                    subject: Some((target, "/tmp/report.md".into())),
                    ..NewMemory::new(
                        MemoryScope::Global,
                        MemoryKind::Observation,
                        "this is output, not input",
                    )
                },
                &agent("a"),
                100,
            )
            .unwrap();
        let neighbours = store
            .neighbors(target, &[EdgeType::Annotates], 1, 10)
            .unwrap();
        assert!(neighbours.iter().any(|n| n.node_id == written.node_id.0));
        let memory = store.memory(written.node_id).unwrap().unwrap();
        assert_eq!(memory.subject_path, Some(PathBuf::from("/tmp/report.md")));
    }

    #[test]
    fn a_near_duplicate_is_reported_and_nothing_is_decided() {
        let store = store();
        let first = remember(
            &store,
            MemoryScope::Global,
            MemoryKind::Fact,
            "the daemon listens on a unix socket in the runtime directory",
        );
        let candidates = store
            .near_duplicate_memories(
                &MemoryScope::Global,
                MemoryKind::Fact,
                "the daemon listens on a unix socket in the runtime dir",
                5,
            )
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].node_id, first.0);
        // Reported, not acted on: nothing was superseded.
        assert!(store.memory(first).unwrap().unwrap().superseded_by.is_none());

        let unrelated = store
            .near_duplicate_memories(
                &MemoryScope::Global,
                MemoryKind::Fact,
                "cross-encoder reranking runs on the graphics card",
                5,
            )
            .unwrap();
        assert!(unrelated.is_empty());
    }

    // -----------------------------------------------------------------------
    // The learning signal
    // -----------------------------------------------------------------------

    fn pack_with(store: &Store, id: &str, nodes: &[NodeId]) {
        let items: Vec<PackItemRecord> = nodes
            .iter()
            .enumerate()
            .map(|(rank, node)| PackItemRecord {
                node_id: *node,
                rank,
                arm: PackArm::Lexical,
                score: 1.0,
            })
            .collect();
        store
            .record_pack(id, Some("s-1"), "a task", 2048, 100, &items, 100)
            .unwrap();
        store.note_retrieved(nodes).unwrap();
    }

    #[test]
    fn a_helpful_report_raises_usefulness_and_a_noise_report_lowers_it_further() {
        let store = store();
        let good = remember(&store, MemoryScope::Global, MemoryKind::Fact, "good");
        let bad = remember(&store, MemoryScope::Global, MemoryKind::Fact, "bad");
        pack_with(&store, "p1", &[good, bad]);

        let applied = store
            .report_outcome("p1", &[good], &[bad], Reporter::Agent, &agent("a"), 200)
            .unwrap();
        assert_eq!(applied.helped, 1);
        assert_eq!(applied.noise, 1);
        assert_eq!(applied.ignored, 0);

        let good = store.memory(good).unwrap().unwrap();
        let bad = store.memory(bad).unwrap().unwrap();
        assert!(good.usefulness > 0.0);
        assert!(bad.usefulness < 0.0);
        assert!(
            bad.usefulness.abs() > good.usefulness,
            "a wasted slot must be more informative than an acceptable one"
        );
        assert_eq!(good.helped_count, 1);
        assert_eq!(bad.noise_count, 1);
        assert_eq!(good.retrieved_count, 1);
    }

    #[test]
    fn a_human_report_outweighs_the_agents_report_on_itself() {
        let store = store();
        let by_agent = remember(&store, MemoryScope::Global, MemoryKind::Fact, "one");
        let by_human = remember(&store, MemoryScope::Global, MemoryKind::Fact, "two");
        pack_with(&store, "p1", &[by_agent, by_human]);

        store
            .report_outcome("p1", &[by_agent], &[], Reporter::Agent, &agent("a"), 200)
            .unwrap();
        store
            .report_outcome("p1", &[by_human], &[], Reporter::Human, &Actor::Human, 201)
            .unwrap();

        let agent_scored = store.memory(by_agent).unwrap().unwrap();
        let human_scored = store.memory(by_human).unwrap().unwrap();
        assert!(human_scored.usefulness > agent_scored.usefulness);
        assert!(human_scored.human_confirmed);
        assert!(!agent_scored.human_confirmed);
    }

    /// The flaw a live run found, and the one a unit test had actually been written *around*.
    ///
    /// `reporting_twice_on_one_item_only_counts_once` guards a client that retries. The first
    /// implementation achieved that by refusing any second report on an item — which also
    /// silenced a human correcting an agent, the exact opposite of the weighting D76 asks for.
    /// Both properties are true now, and they are different properties.
    #[test]
    fn a_human_may_overrule_an_agents_report_and_an_agent_may_not_overrule_a_human() {
        let store = store();
        let contested = remember(&store, MemoryScope::Global, MemoryKind::Fact, "contested");
        let settled = remember(&store, MemoryScope::Global, MemoryKind::Fact, "settled");
        pack_with(&store, "p1", &[contested, settled]);

        // The agent says one memory helped. The human, looking at the same pack, disagrees.
        store
            .report_outcome("p1", &[contested], &[], Reporter::Agent, &agent("a"), 200)
            .unwrap();
        let after_agent = store.memory(contested).unwrap().unwrap();
        assert!(after_agent.usefulness > 0.0);
        assert_eq!(after_agent.helped_count, 1);

        let applied = store
            .report_outcome("p1", &[], &[contested], Reporter::Human, &Actor::Human, 201)
            .unwrap();
        assert_eq!(applied.noise, 1, "the human's correction was ignored");
        let corrected = store.memory(contested).unwrap().unwrap();
        assert!(
            corrected.usefulness < 0.0,
            "the human said noise and the score is still positive: {}",
            corrected.usefulness
        );
        assert_eq!(
            corrected.helped_count, 0,
            "the agent's count survived the correction, so the Inspector would still show it"
        );
        assert_eq!(corrected.noise_count, 1);

        // And the other direction is refused: a human has already looked at `settled`.
        store
            .report_outcome("p1", &[settled], &[], Reporter::Human, &Actor::Human, 202)
            .unwrap();
        let before = store.memory(settled).unwrap().unwrap().usefulness;
        let ignored = store
            .report_outcome("p1", &[], &[settled], Reporter::Agent, &agent("a"), 203)
            .unwrap();
        assert_eq!(ignored.ignored, 1);
        assert_eq!(store.memory(settled).unwrap().unwrap().usefulness, before);
    }

    /// Memories useful for the same task become associated — the memory layer's co-access.
    ///
    /// On being reported helpful *together*, not on appearing in the same pack. Appearing
    /// together is the ranking's own opinion, and building a graph from it would let the graph
    /// confirm whatever the ranking already believed. Being useful together is a judgement from
    /// outside the loop, which is the only kind D76 accepts.
    #[test]
    fn two_memories_useful_for_the_same_task_become_associated() {
        use cortex_core::model::EdgeType;

        let store = store();
        let one = remember(&store, MemoryScope::Global, MemoryKind::Fact, "the first");
        let two = remember(&store, MemoryScope::Global, MemoryKind::Procedure, "the second");
        let bystander = remember(&store, MemoryScope::Global, MemoryKind::Fact, "along for the ride");
        pack_with(&store, "p1", &[one, two, bystander]);

        // All three were shown; two were useful.
        store
            .report_outcome("p1", &[one, two], &[], Reporter::Human, &Actor::Human, 200)
            .unwrap();

        let neighbours = store
            .neighbors(one, &[EdgeType::RecalledWith], 1, 10)
            .unwrap();
        let reached: Vec<i64> = neighbours.iter().map(|n| n.node_id).collect();
        assert!(reached.contains(&two.0), "the two useful ones are not associated");
        assert!(
            !reached.contains(&bystander.0),
            "a memory that was merely shown became associated, which would let the graph \
             confirm whatever the ranking already believed"
        );

        // And the text comes with it, because a traversal that answers with an id is a
        // traversal the caller has to follow up on.
        let edge = neighbours
            .iter()
            .find(|n| n.node_id == two.0)
            .expect("the edge is there");
        assert_eq!(edge.note.as_deref(), Some("the second"));
    }

    #[test]
    fn a_report_that_names_everything_says_nothing_and_builds_no_edges() {
        let store = store();
        let all: Vec<NodeId> = (0..MAX_RECALLED_TOGETHER + 1)
            .map(|index| {
                remember(
                    &store,
                    MemoryScope::Global,
                    MemoryKind::Fact,
                    &format!("memory {index}"),
                )
            })
            .collect();
        pack_with(&store, "p1", &all);
        store
            .report_outcome("p1", &all, &[], Reporter::Human, &Actor::Human, 200)
            .unwrap();

        let edges: i64 = store
            .conn()
            .query_row(
                "SELECT count(*) FROM edges WHERE type = 'recalled_with'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            edges, 0,
            "a client ticking every box produced an association for every pair"
        );
        // The usefulness still lands; only the edges are skipped.
        assert!(store.memory(all[0]).unwrap().unwrap().usefulness > 0.0);
    }

    #[test]
    fn an_outcome_naming_something_the_pack_never_held_is_ignored_not_applied() {
        let store = store();
        let inside = remember(&store, MemoryScope::Global, MemoryKind::Fact, "inside");
        let outside = remember(&store, MemoryScope::Global, MemoryKind::Fact, "outside");
        pack_with(&store, "p1", &[inside]);

        let applied = store
            .report_outcome("p1", &[inside, outside], &[], Reporter::Human, &Actor::Human, 200)
            .unwrap();
        assert_eq!(applied.helped, 1);
        assert_eq!(applied.ignored, 1, "a score was moved by a memory not in the pack");
        assert_eq!(store.memory(outside).unwrap().unwrap().usefulness, 0.0);
    }

    #[test]
    fn reporting_twice_on_one_item_only_counts_once() {
        // Otherwise a client that retries a failed call doubles its own vote.
        let store = store();
        let node = remember(&store, MemoryScope::Global, MemoryKind::Fact, "once");
        pack_with(&store, "p1", &[node]);
        store
            .report_outcome("p1", &[node], &[], Reporter::Human, &Actor::Human, 200)
            .unwrap();
        let second = store
            .report_outcome("p1", &[node], &[], Reporter::Human, &Actor::Human, 201)
            .unwrap();
        assert_eq!(second.helped, 0);
        assert_eq!(second.ignored, 1);
        assert_eq!(store.memory(node).unwrap().unwrap().helped_count, 1);
    }

    #[test]
    fn an_outcome_for_a_pack_nobody_recorded_is_an_error() {
        let store = store();
        let node = remember(&store, MemoryScope::Global, MemoryKind::Fact, "x");
        assert!(store
            .report_outcome("nope", &[node], &[], Reporter::Human, &Actor::Human, 200)
            .is_err());
    }

    /// The counter-intuitive half of D76, and the one that makes the signal a signal.
    #[test]
    fn a_memory_retrieved_often_and_never_reported_falls_behind_one_never_retrieved() {
        let store = store();
        let noisy = remember(&store, MemoryScope::Global, MemoryKind::Fact, "always there");
        let quiet = remember(&store, MemoryScope::Global, MemoryKind::Fact, "never asked for");
        for round in 0..10 {
            pack_with(&store, &format!("p{round}"), &[noisy]);
        }
        assert_eq!(store.decay_unreported(5, 0.05).unwrap(), 1);

        let noisy = store.memory(noisy).unwrap().unwrap();
        let quiet = store.memory(quiet).unwrap().unwrap();
        assert_eq!(noisy.retrieved_count, 10);
        assert!(
            noisy.usefulness < quiet.usefulness,
            "retrieval without use has to be evidence against a memory"
        );
    }

    #[test]
    fn a_memory_that_earned_a_report_is_not_penalised_for_being_retrieved() {
        let store = store();
        let earned = remember(&store, MemoryScope::Global, MemoryKind::Fact, "useful");
        for round in 0..10 {
            pack_with(&store, &format!("p{round}"), &[earned]);
        }
        store
            .report_outcome("p0", &[earned], &[], Reporter::Human, &Actor::Human, 200)
            .unwrap();
        assert_eq!(
            store.decay_unreported(5, 0.05).unwrap(),
            0,
            "a memory with a report is not unreported"
        );
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    #[test]
    fn a_session_counts_its_calls_and_its_refusals_and_its_memories() {
        let store = store();
        store.begin_agent_session("s-1", "claude", Some("tidy"), 100).unwrap();
        store.note_session_call("s-1", true).unwrap();
        store.note_session_call("s-1", true).unwrap();
        store.note_session_call("s-1", false).unwrap();
        store
            .remember(
                &NewMemory {
                    session_id: Some("s-1".into()),
                    ..NewMemory::new(MemoryScope::Global, MemoryKind::Fact, "learned")
                },
                &agent("claude"),
                110,
            )
            .unwrap();

        let only = |store: &Store| {
            store
                .agent_sessions(10)
                .unwrap()
                .into_iter()
                .find(|s| s.id == "s-1")
                .expect("the session is listed")
        };
        let session = only(&store);
        assert_eq!(session.calls, 3);
        assert_eq!(session.refusals, 1);
        assert_eq!(session.memories, 1);
        assert!(session.ended_at.is_none());

        assert!(store.end_agent_session("s-1", 200).unwrap());
        assert!(!store.end_agent_session("s-1", 300).unwrap(), "closed twice");
        assert_eq!(only(&store).ended_at, Some(200));
    }

    // -----------------------------------------------------------------------
    // R10 — the criterion the Definition of Done says will find the bugs
    // -----------------------------------------------------------------------

    /// Everything an agent ever learned, through a full rebuild, compared by canonical dump.
    #[test]
    fn the_whole_memory_layer_survives_a_rebuild() {
        let store = store();
        let subject = file(&store, "/tmp/report.md");

        for (scope, kind, text) in [
            (MemoryScope::Global, MemoryKind::Preference, "German first"),
            (MemoryScope::Agent("a".into()), MemoryKind::Fact, "one fact"),
            (MemoryScope::Agent("b".into()), MemoryKind::Failure, "that did not work"),
            (MemoryScope::Project(1), MemoryKind::Procedure, "run the gate"),
            (MemoryScope::Global, MemoryKind::Decision, "usearch, not a server"),
        ] {
            store
                .remember(&NewMemory::new(scope, kind, text), &agent("a"), 100)
                .unwrap();
        }
        let with_subject = store
            .remember(
                &NewMemory {
                    subject: Some((subject, "/tmp/report.md".into())),
                    confidence: 0.9,
                    ..NewMemory::new(
                        MemoryScope::Global,
                        MemoryKind::Observation,
                        "this is output, not input",
                    )
                },
                &agent("a"),
                101,
            )
            .unwrap();

        // A supersession, so the chain has to survive too.
        let old = remember(&store, MemoryScope::Global, MemoryKind::Fact, "the old number");
        let new = remember(&store, MemoryScope::Global, MemoryKind::Fact, "the new number");
        store.supersede_memory(old, new, &agent("a"), 150).unwrap();

        // A deletion, which must survive as a deletion (D78).
        let deleted = remember(&store, MemoryScope::Global, MemoryKind::Fact, "wrong, delete me");
        assert!(store.forget_memory(deleted, &Actor::Human, 160).unwrap());

        // Usefulness, which is not in the episode log at all.
        pack_with(&store, "p1", &[with_subject.node_id]);
        store
            .report_outcome(
                "p1",
                &[with_subject.node_id],
                &[],
                Reporter::Human,
                &Actor::Human,
                170,
            )
            .unwrap();

        // Two memories reported useful in the same report, so there is an association to lose.
        pack_with(&store, "p-pair", &[old, new]);
        store
            .report_outcome("p-pair", &[old, new], &[], Reporter::Human, &Actor::Human, 171)
            .unwrap();
        let edges_before = store
            .conn()
            .query_row(
                "SELECT count(*) FROM edges WHERE type = 'recalled_with'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert!(edges_before > 0, "the fixture built no association to lose");

        let before = store.canonical_dump().unwrap();
        assert!(before.contains("memory\tglobal\tpreference\tGerman first"));

        store.clear_derived(true).unwrap();
        assert!(store.list_memories(&MemoryFilter::everything(200)).unwrap().is_empty());

        // A rebuild reconciles the filesystem first; the file is recreated here by hand,
        // because this test is about the memory layer rather than about the watcher.
        file(&store, "/tmp/report.md");
        let (replayed, _) = store.replay_memories().unwrap();
        assert!(replayed >= 8);

        let after = store.canonical_dump().unwrap();

        // `retrieved_count` is the one column that legitimately does not come back, and it is
        // worth saying why rather than quietly excluding it: it counts how often a memory was
        // put into a *pack*, packs are bookkeeping that `prune_packs` deletes on purpose, and a
        // number that could be reconstructed from a table this system throws away would be a
        // reason not to throw it away. Everything a memory has *earned* survives, because an
        // outcome report is an episode like every other assertion here.
        //
        // Compared line by line, so a failure names the row rather than the file.
        let strip = |dump: &str| -> Vec<String> {
            dump.lines()
                .map(|line| {
                    if !line.starts_with("memory\t") {
                        return line.to_string();
                    }
                    let mut fields: Vec<&str> = line.split('\t').collect();
                    fields[8] = "-"; // retrieved_count
                    fields.join("\t")
                })
                .collect()
        };
        assert_eq!(strip(&before), strip(&after));

        // And the earned columns are actually non-zero on both sides, or the comparison above
        // would be comparing two rows of zeroes and passing for the wrong reason.
        let earned = |dump: &str| {
            dump.lines()
                .filter(|line| line.starts_with("memory\t"))
                .any(|line| {
                    let fields: Vec<&str> = line.split('\t').collect();
                    fields[5].parse::<f64>().unwrap_or(0.0) != 0.0
                })
        };
        assert!(earned(&before), "the fixture earned nothing to compare");
        assert!(
            earned(&after),
            "usefulness did not survive the rebuild; an agent's whole record of what has been \
             useful would reset on `cortex rebuild`"
        );

        // The associations an outcome produced are edges, and the canonical dump does not walk
        // edges — so they are checked directly. A rebuilt index whose graph differs from the
        // one it replaced would otherwise compare equal and be wrong.
        let associations = |store: &Store| -> i64 {
            store
                .conn()
                .query_row(
                    "SELECT count(*) FROM edges WHERE type = 'recalled_with'",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            associations(&store),
            edges_before,
            "the associations between memories reported useful together did not survive"
        );

        // And the deletion did not come back.
        let texts: Vec<String> = store
            .list_memories(&MemoryFilter::everything(200))
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert!(
            !texts.iter().any(|t| t == "wrong, delete me"),
            "a deleted memory returned on the rebuild; the delete button was a lie"
        );
        assert!(texts.iter().any(|t| t == "the old number"), "history was lost");
    }

    /// The subtle half of replaying outcomes: a correction has to be replayed as a correction.
    ///
    /// An episode that recorded only the human's new verdict would rebuild into a state where
    /// both the agent's report and the human's had been counted — which is not what the live
    /// call produced, and is a state no sequence of real reports could reach.
    #[test]
    fn a_humans_correction_survives_a_rebuild_as_a_correction() {
        let store = store();
        let contested = remember(&store, MemoryScope::Global, MemoryKind::Fact, "contested");
        pack_with(&store, "p1", &[contested]);
        store
            .report_outcome("p1", &[contested], &[], Reporter::Agent, &agent("a"), 200)
            .unwrap();
        store
            .report_outcome("p1", &[], &[contested], Reporter::Human, &Actor::Human, 201)
            .unwrap();

        let live = store.memory(contested).unwrap().unwrap();
        assert!(live.usefulness < 0.0);
        assert_eq!(live.helped_count, 0);
        assert_eq!(live.noise_count, 1);

        store.clear_derived(true).unwrap();
        store.replay_memories().unwrap();

        let rebuilt = store
            .list_memories(&MemoryFilter::everything(300))
            .unwrap()
            .into_iter()
            .find(|m| m.text == "contested")
            .expect("the memory came back");
        assert_eq!(
            (rebuilt.usefulness * 1e6).round(),
            (live.usefulness * 1e6).round(),
            "the rebuild counted the agent's report the human had already overruled"
        );
        assert_eq!(rebuilt.helped_count, 0);
        assert_eq!(rebuilt.noise_count, 1);
    }

    /// Definition of Done §2 at the scale it names.
    ///
    /// The dozen-memory test above is about *shape*: every scope, every kind, a supersession, a
    /// deletion, a correction. This one is about the claim actually being made — a thousand
    /// memories, rebuilt, compared — and about the two things that only show up in numbers: how
    /// long the replay takes, and whether anything in it is quadratic.
    #[test]
    fn a_thousand_memories_survive_a_rebuild() {
        let store = store();
        let actor = agent("a");

        let scopes = [
            MemoryScope::Global,
            MemoryScope::Agent("a".into()),
            MemoryScope::Agent("b".into()),
            MemoryScope::Project(1),
            MemoryScope::Session("s-1".into()),
        ];
        let kinds = MemoryKind::ALL;
        let mut written = Vec::with_capacity(1_000);
        for index in 0..1_000usize {
            let scope = scopes[index % scopes.len()].clone();
            let is_session = matches!(scope, MemoryScope::Session(_));
            let memory = NewMemory {
                // Far in the future, so the session memories are live for this test rather
                // than being expired out from under the comparison.
                expires_at: is_session.then_some(i64::MAX / 2),
                confidence: (index % 100) as f64 / 100.0,
                ..NewMemory::new(
                    scope,
                    kinds[index % kinds.len()],
                    format!("memory number {index}, with something particular to say about it"),
                )
            };
            written.push(store.remember(&memory, &actor, 100 + index as i64).unwrap().node_id);
        }

        // A supersession chain three deep, so the replay has to resolve one memory that is
        // itself superseded.
        store.supersede_memory(written[0], written[1], &actor, 2_000).unwrap();
        store.supersede_memory(written[1], written[2], &actor, 2_001).unwrap();
        // A deletion, which must stay deleted.
        assert!(store.forget_memory(written[3], &Actor::Human, 2_002).unwrap());
        // And something earned, including a correction.
        pack_with(&store, "p1", &[written[4], written[5]]);
        store
            .report_outcome("p1", &[written[4]], &[written[5]], Reporter::Agent, &actor, 2_003)
            .unwrap();
        store
            .report_outcome("p1", &[], &[written[4]], Reporter::Human, &Actor::Human, 2_004)
            .unwrap();

        let before = store.canonical_dump().unwrap();
        assert_eq!(
            before.lines().filter(|l| l.starts_with("memory\t")).count(),
            999,
            "the fixture is not the size this criterion is about"
        );

        store.clear_derived(true).unwrap();
        let started = std::time::Instant::now();
        let (replayed, _) = store.replay_memories().unwrap();
        let elapsed = started.elapsed();
        assert!(replayed >= 1_000);

        let strip = |dump: &str| -> Vec<String> {
            dump.lines()
                .map(|line| {
                    if !line.starts_with("memory\t") {
                        return line.to_string();
                    }
                    let mut fields: Vec<&str> = line.split('\t').collect();
                    fields[8] = "-"; // retrieved_count; see the note in the smaller test
                    fields.join("\t")
                })
                .collect()
        };
        assert_eq!(strip(&before), strip(&store.canonical_dump().unwrap()));

        // Not a gate — the build prompt gives the replay no budget — but recorded, because a
        // replay that is quadratic in the memory count would be invisible at a dozen and
        // ruinous at a hundred thousand. Ten seconds for a thousand would be that signal.
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "replaying 1 000 memories took {elapsed:?}, which is the shape of a quadratic"
        );
    }

    #[test]
    fn replaying_twice_does_not_double_the_memory_layer() {
        let store = store();
        remember(&store, MemoryScope::Global, MemoryKind::Fact, "once only");
        let before = store.canonical_dump().unwrap();

        store.clear_derived(true).unwrap();
        store.replay_memories().unwrap();
        store.clear_derived(true).unwrap();
        store.replay_memories().unwrap();

        assert_eq!(before, store.canonical_dump().unwrap());
    }
}
