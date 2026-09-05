//! The federation outbox (M7, D11, D18).
//!
//! Events queue here and drain when a peer is reachable. Offline is the normal case, so the
//! table is allowed to be large and nothing in it is precious: every row is derived from
//! `episodes` and from the concept nodes, and can be derived again. What *is* precious is the
//! identifier on each row — a resend must reuse it, or a peer without deduplication of its own
//! (D18 assumes exactly that peer) fills up with copies.
//!
//! Three things live in `meta` beside the table:
//!
//! - `sync.origin` — a stable identifier for this index, minted once. It is part of every event
//!   id, so two machines that both remember the same sentence do not collide on the peer.
//! - `sync.lamport` — the Lamport clock. Every enqueued event takes the next tick; an ack that
//!   carries a larger clock from the peer moves it forward (D11).
//! - `sync.episode_cursor` — the last episode id that has been turned into an event. The worker
//!   harvests from here; `replay` resets it to zero and the same events, with the same ids, are
//!   produced again. That is what makes "replaying the entire outbox twice changes nothing"
//!   a property of the identifiers rather than of the peer.
//!
//! **Attempts count rejections, not silence.** A peer that is unreachable is not evidence
//! against an event; a peer that looked at it and refused it is. The pending index only hides a
//! row after a hundred refusals, and an unreachable peer never increments the counter — a week
//! offline against a one-second interval would otherwise dead-letter everything in two minutes.

use crate::error::{Result, StoreError};
use crate::Store;
use cortex_core::ids::EpisodeId;
use rusqlite::{params, OptionalExtension};

const META_ORIGIN: &str = "sync.origin";
const META_LAMPORT: &str = "sync.lamport";
const META_CURSOR: &str = "sync.episode_cursor";
const META_ONTOLOGY: &str = "sync.ontology";

/// Rows with at least this many refusals are dead-lettered: they stay in the table for a human
/// to look at, and they leave the pending set. Mirrors the partial index in the schema.
pub const DEAD_AFTER_ATTEMPTS: i64 = 100;

/// One event waiting to be sent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutboxRow {
    pub id: i64,
    pub event_id: String,
    pub lamport: i64,
    pub payload: serde_json::Value,
    pub created_at: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
}

/// What is known about one peer.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SyncState {
    pub peer: String,
    pub last_acked_lamport: i64,
    pub last_sync_at: Option<i64>,
}

/// A concept node as the harvester sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConceptForSync {
    pub id: i64,
    pub label: String,
    pub created_at: i64,
    /// Sorted by path, so the same membership always hashes the same.
    pub members: Vec<String>,
}

/// Counts for `sync.status` and `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct OutboxCounts {
    /// Waiting for a peer.
    pub pending: i64,
    /// Refused too often to keep trying. Kept for inspection.
    pub dead: i64,
    /// Confirmed by the peer and still on record.
    pub acked: i64,
    /// The last episode id that has been turned into an event.
    pub episode_cursor: i64,
    /// Episodes the harvester has not looked at yet.
    pub episodes_behind: i64,
    pub lamport: i64,
}

impl Store {
    /// The stable identifier of this index, minted on first use.
    ///
    /// Random rather than derived from the machine: a hostname is a private identifier (R9)
    /// and would leave the machine inside every event id.
    pub fn sync_origin(&self) -> Result<String> {
        if let Some(origin) = self.meta_get(META_ORIGIN)? {
            return Ok(origin);
        }
        let origin = mint_origin();
        // Two callers minting at once must agree; the first write wins and both read it back.
        self.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
            params![META_ORIGIN, origin],
        )?;
        Ok(self
            .meta_get(META_ORIGIN)?
            .ok_or_else(|| StoreError::corrupt("the sync origin vanished after being written"))?)
    }

    /// The Lamport clock as it stands. Zero on a fresh index.
    pub fn lamport(&self) -> Result<i64> {
        Ok(self
            .meta_get(META_LAMPORT)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// Move the clock past a value seen elsewhere — a peer's clock in an ack. Never moves it
    /// backwards.
    pub fn lamport_observe(&self, seen: i64) -> Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = lamport_on(&tx)?;
        let next = current.max(seen);
        set_lamport_on(&tx, next)?;
        tx.commit()?;
        Ok(next)
    }

    /// Queue an event. Returns its Lamport tick, or `None` when an event with this id is already
    /// known — queued, dead or acknowledged. A second enqueue of the same identity is the
    /// harmless duplicate D18 wants, and an acknowledged one is not sent again.
    ///
    /// The tick is taken inside the same transaction as the insert, so two writers cannot hand
    /// out one tick twice and a row is never visible with a clock the counter has not reached.
    pub fn outbox_enqueue(
        &self,
        event_id: &str,
        kind: &str,
        payload: &serde_json::Value,
        now: i64,
    ) -> Result<Option<i64>> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM sync_outbox WHERE event_id = ?1",
                [event_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if exists {
            tx.commit()?;
            return Ok(None);
        }
        let tick = lamport_on(&tx)? + 1;
        set_lamport_on(&tx, tick)?;
        tx.execute(
            "INSERT INTO sync_outbox (event_id, kind, lamport, payload_json, created_at, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![event_id, kind, tick, serde_json::to_string(payload)?, now],
        )?;
        tx.commit()?;
        Ok(Some(tick))
    }

    /// The next `limit` events in clock order, skipping the dead-lettered ones.
    pub fn outbox_pending(&self, limit: usize) -> Result<Vec<OutboxRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, event_id, lamport, payload_json, created_at, attempts, last_error
             FROM sync_outbox WHERE acked_at IS NULL AND attempts < ?1
             ORDER BY lamport ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![DEAD_AFTER_ATTEMPTS, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, event_id, lamport, payload, created_at, attempts, last_error) = row?;
            out.push(OutboxRow {
                id,
                event_id,
                lamport,
                payload: serde_json::from_str(&payload)?,
                created_at,
                attempts,
                last_error,
            });
        }
        Ok(out)
    }

    /// The peer confirmed these. They are marked, not removed — see the 0008 migration — and
    /// the clock of the highest one is recorded against the peer. Returns how many rows were
    /// actually marked.
    pub fn outbox_ack(&self, peer: &str, event_ids: &[String], now: i64) -> Result<usize> {
        if event_ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut removed = 0;
        let mut highest: i64 = 0;
        for id in event_ids {
            let lamport: Option<i64> = tx
                .query_row(
                    "SELECT lamport FROM sync_outbox WHERE event_id = ?1 AND acked_at IS NULL",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(lamport) = lamport {
                highest = highest.max(lamport);
                removed += tx.execute(
                    "UPDATE sync_outbox SET acked_at = ?2, last_error = NULL WHERE event_id = ?1",
                    params![id, now],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO sync_state (peer, last_acked_lamport, last_sync_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(peer) DO UPDATE SET
                last_acked_lamport = max(last_acked_lamport, excluded.last_acked_lamport),
                last_sync_at = excluded.last_sync_at",
            params![peer, highest, now],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// The peer looked at these and refused them. One more attempt each, and the reason kept.
    pub fn outbox_reject(&self, event_ids: &[(String, String)]) -> Result<()> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (id, reason) in event_ids {
            tx.execute(
                "UPDATE sync_outbox SET attempts = attempts + 1, last_error = ?2
                 WHERE event_id = ?1",
                params![id, reason],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Note that the peer was reached, without acknowledging anything — an empty batch, or a
    /// batch that was refused whole.
    pub fn sync_touch(&self, peer: &str, now: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sync_state (peer, last_acked_lamport, last_sync_at)
             VALUES (?1, 0, ?2)
             ON CONFLICT(peer) DO UPDATE SET last_sync_at = excluded.last_sync_at",
            params![peer, now],
        )?;
        Ok(())
    }

    pub fn sync_state(&self, peer: &str) -> Result<SyncState> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT peer, last_acked_lamport, last_sync_at FROM sync_state WHERE peer = ?1",
                [peer],
                |r| {
                    Ok(SyncState {
                        peer: r.get(0)?,
                        last_acked_lamport: r.get(1)?,
                        last_sync_at: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_else(|| SyncState {
                peer: peer.to_string(),
                ..SyncState::default()
            }))
    }

    pub fn outbox_counts(&self) -> Result<OutboxCounts> {
        let conn = self.conn();
        let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
        let pending = one(&format!(
            "SELECT count(*) FROM sync_outbox
             WHERE acked_at IS NULL AND attempts < {DEAD_AFTER_ATTEMPTS}"
        ))?;
        let dead = one(&format!(
            "SELECT count(*) FROM sync_outbox
             WHERE acked_at IS NULL AND attempts >= {DEAD_AFTER_ATTEMPTS}"
        ))?;
        let acked = one("SELECT count(*) FROM sync_outbox WHERE acked_at IS NOT NULL")?;
        drop(conn);
        let cursor = self.outbox_cursor()?;
        let behind = self.count_episodes_after(EpisodeId(cursor))?;
        Ok(OutboxCounts {
            pending,
            dead,
            acked,
            episode_cursor: cursor,
            episodes_behind: behind,
            lamport: self.lamport()?,
        })
    }

    /// Drop acknowledged episode events older than `before`. Concept events stay: they are
    /// harvested by identity, and the row is what stops them being sent again.
    pub fn outbox_prune_acked(&self, before: i64) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM sync_outbox
             WHERE kind = 'episode' AND acked_at IS NOT NULL AND acked_at < ?1",
            [before],
        )?)
    }

    /// The last episode id that has been turned into an event. Zero on a fresh index.
    pub fn outbox_cursor(&self) -> Result<i64> {
        Ok(self
            .meta_get(META_CURSOR)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_outbox_cursor(&self, cursor: i64) -> Result<()> {
        self.meta_set(META_CURSOR, &cursor.to_string())
    }

    /// Forget every queued event and start harvesting from the first episode again. The events
    /// come back with the same ids, which is the whole point of the exercise.
    pub fn outbox_reset(&self) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM sync_outbox", [])?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '0')
             ON CONFLICT(key) DO UPDATE SET value = '0'",
            [META_CURSOR],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn count_episodes_after(&self, after: EpisodeId) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM episodes WHERE id > ?1",
            [after.0],
            |r| r.get(0),
        )?)
    }

    /// Every concept node with the paths of its members, for the harvester.
    ///
    /// A concept's identity on the wire is its label plus its member set: a concept that gained
    /// a member is a new event, an untouched one is not sent again. The Core never sends the
    /// members' content — a concept is a name and a list of places (D19).
    pub fn concepts_for_sync(&self) -> Result<Vec<ConceptForSync>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.label, c.first_seen, m.path
             FROM nodes c
             LEFT JOIN edges e ON e.src = c.id AND e.type = 'derived_from'
             LEFT JOIN nodes m ON m.id = e.dst
             WHERE c.kind = 'concept' AND c.label IS NOT NULL
             ORDER BY c.id, m.path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out: Vec<ConceptForSync> = Vec::new();
        for row in rows {
            let (id, label, first_seen, member) = row?;
            match out.last_mut() {
                Some(last) if last.id == id => {
                    if let Some(m) = member {
                        last.members.push(m);
                    }
                }
                _ => out.push(ConceptForSync {
                    id,
                    label,
                    created_at: first_seen,
                    members: member.into_iter().collect(),
                }),
            }
        }
        Ok(out)
    }

    /// The ontology the peer last sent down, if any. The Core stores it and shows it; what to
    /// do with a taxonomy is the Extension's business (D11).
    pub fn sync_ontology(&self) -> Result<Option<serde_json::Value>> {
        Ok(match self.meta_get(META_ONTOLOGY)? {
            Some(text) => Some(serde_json::from_str(&text)?),
            None => None,
        })
    }

    pub fn set_sync_ontology(&self, ontology: &serde_json::Value) -> Result<()> {
        self.meta_set(META_ONTOLOGY, &serde_json::to_string(ontology)?)
    }
}

fn lamport_on(conn: &rusqlite::Connection) -> Result<i64> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [META_LAMPORT], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

fn set_lamport_on(conn: &rusqlite::Connection, value: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![META_LAMPORT, value.to_string()],
    )?;
    Ok(())
}

/// Thirty-two hex characters that identify this index and nothing else.
///
/// Hashed from the clock, the process id and a counter rather than read from a random device:
/// the daemon has no random-number dependency, and this is minted once per index, so the
/// entropy of a nanosecond clock and a pid is more than enough to keep two machines apart.
fn mint_origin() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::model::{Action, Actor};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn the_origin_is_minted_once_and_names_no_machine() {
        let s = store();
        let a = s.sync_origin().unwrap();
        let b = s.sync_origin().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, store().sync_origin().unwrap(), "two indexes must not share an origin");
    }

    #[test]
    fn every_enqueue_takes_the_next_tick_and_a_duplicate_takes_none() {
        let s = store();
        assert_eq!(s.outbox_enqueue("a", "episode", &serde_json::json!({}), 1).unwrap(), Some(1));
        assert_eq!(s.outbox_enqueue("b", "episode", &serde_json::json!({}), 1).unwrap(), Some(2));
        assert_eq!(s.outbox_enqueue("a", "episode", &serde_json::json!({}), 1).unwrap(), None);
        assert_eq!(s.lamport().unwrap(), 2);
        assert_eq!(s.outbox_counts().unwrap().pending, 2);
    }

    #[test]
    fn the_clock_moves_forward_past_a_peer_and_never_back() {
        let s = store();
        s.outbox_enqueue("a", "episode", &serde_json::json!({}), 1).unwrap();
        assert_eq!(s.lamport_observe(10).unwrap(), 10);
        assert_eq!(s.lamport_observe(3).unwrap(), 10);
        assert_eq!(s.outbox_enqueue("b", "episode", &serde_json::json!({}), 1).unwrap(), Some(11));
    }

    #[test]
    fn pending_is_in_clock_order_and_acks_remove_and_record() {
        let s = store();
        for id in ["x", "y", "z"] {
            s.outbox_enqueue(id, "episode", &serde_json::json!({ "id": id }), 5).unwrap();
        }
        let pending = s.outbox_pending(10).unwrap();
        assert_eq!(
            pending.iter().map(|r| r.event_id.as_str()).collect::<Vec<_>>(),
            ["x", "y", "z"]
        );
        let removed = s
            .outbox_ack("peer", &["x".into(), "z".into(), "never-sent".into()], 99)
            .unwrap();
        assert_eq!(removed, 2);
        assert_eq!(s.outbox_pending(10).unwrap().len(), 1);
        let state = s.sync_state("peer").unwrap();
        assert_eq!(state.last_acked_lamport, 3);
        assert_eq!(state.last_sync_at, Some(99));
        // Acknowledged is remembered: the same identity is not queued a second time, and a
        // second ack of it changes nothing.
        assert_eq!(s.outbox_enqueue("x", "episode", &serde_json::json!({}), 6).unwrap(), None);
        assert_eq!(s.outbox_ack("peer", &["x".into()], 100).unwrap(), 0);
        assert_eq!(s.outbox_counts().unwrap().acked, 2);
    }

    #[test]
    fn pruning_forgets_old_episode_acks_and_keeps_concepts() {
        let s = store();
        s.outbox_enqueue("ep", "episode", &serde_json::json!({}), 1).unwrap();
        s.outbox_enqueue("co", "concept", &serde_json::json!({}), 1).unwrap();
        s.outbox_ack("peer", &["ep".into(), "co".into()], 50).unwrap();
        assert_eq!(s.outbox_prune_acked(40).unwrap(), 0);
        assert_eq!(s.outbox_prune_acked(60).unwrap(), 1);
        assert_eq!(s.outbox_counts().unwrap().acked, 1);
        assert_eq!(s.outbox_enqueue("co", "concept", &serde_json::json!({}), 70).unwrap(), None);
        assert_eq!(s.outbox_enqueue("ep", "episode", &serde_json::json!({}), 70).unwrap(), Some(3));
    }

    #[test]
    fn a_refusal_counts_and_a_hundred_of_them_dead_letter_the_row() {
        let s = store();
        s.outbox_enqueue("bad", "episode", &serde_json::json!({}), 1).unwrap();
        for _ in 0..DEAD_AFTER_ATTEMPTS {
            s.outbox_reject(&[("bad".into(), "no".into())]).unwrap();
        }
        assert!(s.outbox_pending(10).unwrap().is_empty());
        let counts = s.outbox_counts().unwrap();
        assert_eq!((counts.pending, counts.dead), (0, 1));
    }

    #[test]
    fn the_cursor_walks_episodes_oldest_first_and_reset_starts_over() {
        let s = store();
        for i in 0..5 {
            s.append_episode(i, &Actor::Human, Action::Open, None, None, &serde_json::json!({}))
                .unwrap();
        }
        let first = s.episodes_after(0, 2).unwrap();
        assert_eq!(first.iter().map(|e| e.id.0).collect::<Vec<_>>(), [1, 2]);
        s.set_outbox_cursor(2).unwrap();
        assert_eq!(s.count_episodes_after(EpisodeId(2)).unwrap(), 3);
        s.outbox_enqueue("e", "episode", &serde_json::json!({}), 1).unwrap();
        s.outbox_reset().unwrap();
        assert_eq!(s.outbox_cursor().unwrap(), 0);
        assert_eq!(s.outbox_counts().unwrap().pending, 0);
        // The clock is not reset: a replayed event must still sort after everything the peer
        // has already seen from us, or "replay" would reorder history.
        assert_eq!(s.lamport().unwrap(), 1);
    }
}
