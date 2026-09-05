//! The worker: harvest, then drain, one tick at a time.
//!
//! A tick is a pure function of the store, the clock and the transport, so the rate limit and
//! the backoff are tested with a synthetic clock rather than with `sleep`. The thread that
//! drives it in the daemon is thirty lines at the bottom of this file.
//!
//! Order of a tick:
//!
//! 1. **Harvest.** Episodes after the cursor become events (`derive::from_episode`); concept
//!    nodes become events by identity. Both are `INSERT OR IGNORE` by event id, so harvesting
//!    twice enqueues nothing twice. The cursor moves only after the enqueue succeeded.
//! 2. **Wait, if told to.** After a batch, the configured interval; after a failure, a doubling
//!    backoff; after a peer asked for quiet, at least what it asked for.
//! 3. **Send** the oldest `batch_size` pending events, in clock order, and act on the answer:
//!    accepted rows leave, rejected rows count an attempt, silence about a row means it is
//!    sent again next time.

use crate::derive;
use crate::protocol::{SyncBatch, PROTOCOL_VERSION};
use crate::transport::{Transport, TransportError};
use cortex_core::config::FederationConfig;
use cortex_core::ids::NodeId;
use cortex_store::Store;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many episodes one tick turns into events at most. Bounds the memory of a first sync
/// over a large log; the rest is picked up on the next tick, which is a second later.
pub const HARVEST_PER_TICK: usize = 5_000;

/// How long an acknowledged episode event stays on record before it is pruned. A week: long
/// enough to answer "did Tuesday reach the peer", short enough that the outbox stays a cache.
pub const ACKED_KEPT_SECS: i64 = 7 * 24 * 3600;

/// What one tick did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TickReport {
    pub at_ms: i64,
    pub harvested: usize,
    pub sent: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub outcome: TickOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum TickOutcome {
    /// Nothing pending.
    Idle,
    /// A batch went out and was answered.
    Sent,
    /// Too soon after the last batch, the last failure, or the peer's request for quiet.
    Waiting { until_ms: i64 },
    /// The peer could not be reached. No attempt was counted.
    Unreachable(String),
    /// The peer answered and refused the whole batch. Every event counted an attempt.
    Refused(String),
    /// Off in the configuration.
    Disabled,
}

/// What `sync.status` shows. Everything a human needs to answer "is it syncing, and if not,
/// why not" — and no address.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub transport: String,
    pub peer: String,
    pub origin: String,
    pub pending: i64,
    pub dead: i64,
    pub acked: i64,
    pub episodes_behind: i64,
    pub lamport: i64,
    pub last_acked_lamport: i64,
    pub last_sync_at: Option<i64>,
    pub batches_sent: u64,
    pub events_accepted: u64,
    pub events_rejected: u64,
    pub backoff_ms: u64,
    pub not_before_ms: i64,
    pub last_error: Option<String>,
    pub last_tick: Option<TickReport>,
    pub ontology_version: Option<i64>,
}

/// Levers the daemon pulls from the RPC thread: stop, send now, start over.
#[derive(Default)]
pub struct SyncControl {
    pub stop: AtomicBool,
    pub kick: AtomicBool,
    pub replay: AtomicBool,
}

pub struct Worker {
    store: Arc<Store>,
    transport: Box<dyn Transport + Send>,
    config: FederationConfig,
    origin: String,
    status: Arc<Mutex<SyncStatus>>,
    backoff_ms: u64,
    not_before_ms: i64,
}

impl Worker {
    pub fn new(
        store: Arc<Store>,
        transport: Box<dyn Transport + Send>,
        config: FederationConfig,
    ) -> cortex_store::error::Result<Self> {
        let origin = store.sync_origin()?;
        let state = store.sync_state(&config.peer)?;
        let status = SyncStatus {
            enabled: config.enabled,
            transport: transport.name(),
            peer: config.peer.clone(),
            origin: origin.clone(),
            pending: 0,
            dead: 0,
            acked: 0,
            episodes_behind: 0,
            lamport: store.lamport()?,
            last_acked_lamport: state.last_acked_lamport,
            last_sync_at: state.last_sync_at,
            batches_sent: 0,
            events_accepted: 0,
            events_rejected: 0,
            backoff_ms: 0,
            not_before_ms: 0,
            last_error: None,
            last_tick: None,
            ontology_version: store
                .sync_ontology()?
                .and_then(|o| o.get("version").and_then(|v| v.as_i64())),
        };
        Ok(Self {
            store,
            transport,
            config,
            origin,
            status: Arc::new(Mutex::new(status)),
            backoff_ms: 0,
            not_before_ms: 0,
        })
    }

    /// A handle the RPC side reads. Updated at the end of every tick.
    pub fn status(&self) -> Arc<Mutex<SyncStatus>> {
        self.status.clone()
    }

    /// Turn what happened since the cursor into queued events. Returns how many were queued.
    pub fn harvest(&self, now_secs: i64) -> cortex_store::error::Result<usize> {
        let mut queued = 0;
        let mut cursor = self.store.outbox_cursor()?;
        let mut budget = HARVEST_PER_TICK;
        while budget > 0 {
            let chunk = self.store.episodes_after(cursor, budget.min(1_000))?;
            if chunk.is_empty() {
                break;
            }
            budget = budget.saturating_sub(chunk.len());
            for episode in &chunk {
                cursor = episode.id.0;
                let path = match episode.node_id {
                    Some(id) => self.store.node_by_id(NodeId(id.0))?.map(|n| n.path),
                    None => None,
                };
                let path = path.as_ref().map(|p| p.to_string_lossy().into_owned());
                if let Some(event) =
                    derive::from_episode(episode, path.as_deref(), &self.origin, &self.config)
                {
                    let payload = serde_json::json!({
                        "ts": event.ts,
                        "kind": event.kind,
                        "payload": event.payload,
                    });
                    if self
                        .store
                        .outbox_enqueue(&event.event_id, &event.kind, &payload, now_secs)?
                        .is_some()
                    {
                        queued += 1;
                    }
                }
            }
            self.store.set_outbox_cursor(cursor)?;
        }

        for concept in self.store.concepts_for_sync()? {
            let event = derive::from_concept(&concept, &self.origin);
            let payload = serde_json::json!({
                "ts": event.ts,
                "kind": event.kind,
                "payload": event.payload,
            });
            if self
                        .store
                        .outbox_enqueue(&event.event_id, &event.kind, &payload, now_secs)?
                        .is_some()
                    {
                queued += 1;
            }
        }
        Ok(queued)
    }

    /// One tick, at `now_ms` on the caller's clock.
    pub fn tick(&mut self, now_ms: i64) -> TickReport {
        let report = self.tick_inner(now_ms);
        self.publish(&report);
        report
    }

    fn tick_inner(&mut self, now_ms: i64) -> TickReport {
        let mut report = TickReport {
            at_ms: now_ms,
            harvested: 0,
            sent: 0,
            accepted: 0,
            rejected: 0,
            outcome: TickOutcome::Idle,
        };
        if !self.config.enabled {
            report.outcome = TickOutcome::Disabled;
            return report;
        }
        let now_secs = now_ms.div_euclid(1000);

        match self.harvest(now_secs) {
            Ok(n) => report.harvested = n,
            Err(e) => {
                tracing::warn!(error = %e, "could not harvest events for the outbox");
                self.note_error(e.to_string());
            }
        }

        if now_ms < self.not_before_ms {
            report.outcome = TickOutcome::Waiting {
                until_ms: self.not_before_ms,
            };
            return report;
        }

        let pending = match self.store.outbox_pending(self.config.batch_size) {
            Ok(rows) => rows,
            Err(e) => {
                self.note_error(e.to_string());
                return report;
            }
        };
        if pending.is_empty() {
            return report;
        }

        let batch = SyncBatch {
            protocol: PROTOCOL_VERSION,
            origin: self.origin.clone(),
            lamport: self.store.lamport().unwrap_or(0),
            sent_at: now_secs,
            events: pending
                .iter()
                .map(|row| crate::protocol::SyncEvent {
                    event_id: row.event_id.clone(),
                    lamport: row.lamport,
                    origin: self.origin.clone(),
                    ts: row.payload.get("ts").and_then(|v| v.as_i64()).unwrap_or(row.created_at),
                    kind: row
                        .payload
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("episode")
                        .to_string(),
                    payload: row
                        .payload
                        .get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
                .collect(),
        };
        report.sent = batch.events.len();

        match self.transport.push(&batch) {
            Ok(ack) => {
                let interval = self.config.batch_interval_ms as i64;
                let quiet = ack.quiet_for_ms.unwrap_or(0) as i64;
                self.not_before_ms = now_ms + interval.max(quiet);
                self.backoff_ms = 0;

                let accepted: Vec<String> = ack
                    .accepted
                    .iter()
                    .filter(|id| batch.events.iter().any(|e| &e.event_id == *id))
                    .cloned()
                    .collect();
                let rejected: Vec<(String, String)> = ack
                    .rejected
                    .iter()
                    .filter(|r| batch.events.iter().any(|e| e.event_id == r.event_id))
                    .map(|r| (r.event_id.clone(), r.reason.clone()))
                    .collect();
                report.accepted = accepted.len();
                report.rejected = rejected.len();

                if let Err(e) = self.store.outbox_ack(&self.config.peer, &accepted, now_secs) {
                    self.note_error(e.to_string());
                }
                if let Err(e) = self.store.outbox_prune_acked(now_secs - ACKED_KEPT_SECS) {
                    self.note_error(e.to_string());
                }
                if let Err(e) = self.store.outbox_reject(&rejected) {
                    self.note_error(e.to_string());
                }
                if accepted.is_empty() {
                    let _ = self.store.sync_touch(&self.config.peer, now_secs);
                }
                if let Some(seen) = ack.lamport {
                    let _ = self.store.lamport_observe(seen);
                }
                if let Some(ontology) = &ack.ontology {
                    match serde_json::to_value(ontology) {
                        Ok(value) => {
                            let _ = self.store.set_sync_ontology(&value);
                            if let Ok(mut s) = self.status.lock() {
                                s.ontology_version = Some(ontology.version);
                            }
                        }
                        Err(e) => self.note_error(e.to_string()),
                    }
                }
                if let Ok(mut s) = self.status.lock() {
                    s.batches_sent += 1;
                    s.events_accepted += accepted.len() as u64;
                    s.events_rejected += rejected.len() as u64;
                    s.last_error = None;
                }
                report.outcome = TickOutcome::Sent;
            }
            Err(TransportError::Refused(reason)) => {
                let all: Vec<(String, String)> = batch
                    .events
                    .iter()
                    .map(|e| (e.event_id.clone(), reason.clone()))
                    .collect();
                report.rejected = all.len();
                if let Err(e) = self.store.outbox_reject(&all) {
                    self.note_error(e.to_string());
                }
                let _ = self.store.sync_touch(&self.config.peer, now_secs);
                self.not_before_ms = now_ms + self.config.batch_interval_ms as i64;
                self.note_error(format!("refused: {reason}"));
                report.outcome = TickOutcome::Refused(reason);
            }
            Err(e @ (TransportError::Unreachable(_) | TransportError::Malformed(_))) => {
                // Doubling from the interval up to the ceiling. The first failure waits one
                // interval, not the ceiling: a peer that blinked is back in a second.
                self.backoff_ms = if self.backoff_ms == 0 {
                    self.config.batch_interval_ms
                } else {
                    (self.backoff_ms * 2).min(self.config.max_backoff_ms)
                };
                self.not_before_ms = now_ms + self.backoff_ms as i64;
                let message = e.to_string();
                self.note_error(message.clone());
                report.outcome = TickOutcome::Unreachable(message);
            }
        }
        report
    }

    fn note_error(&self, message: String) {
        if let Ok(mut s) = self.status.lock() {
            s.last_error = Some(message);
        }
    }

    fn publish(&self, report: &TickReport) {
        // The counts are a scan of the outbox, and the outbox can hold three days of events.
        // They change only when this worker changes them — nothing else writes the table —
        // so a tick that harvested nothing and sent nothing leaves them as they were. That
        // keeps a waiting tick, which is most ticks, at one cheap query.
        let changed = report.harvested > 0 || report.sent > 0 || report.rejected > 0;
        let never_counted = self.status.lock().map(|s| s.last_tick.is_none()).unwrap_or(true);
        let counts = if changed || never_counted {
            self.store.outbox_counts().ok()
        } else {
            None
        };
        let state = if changed {
            self.store.sync_state(&self.config.peer).ok()
        } else {
            None
        };
        if let Ok(mut s) = self.status.lock() {
            if let Some(c) = counts {
                s.pending = c.pending;
                s.dead = c.dead;
                s.acked = c.acked;
                s.episodes_behind = c.episodes_behind;
                s.lamport = c.lamport;
            }
            if let Some(st) = state {
                s.last_acked_lamport = st.last_acked_lamport;
                s.last_sync_at = st.last_sync_at;
            }
            s.backoff_ms = self.backoff_ms;
            s.not_before_ms = self.not_before_ms;
            s.last_tick = Some(report.clone());
        }
    }

    /// Drive ticks on the wall clock until told to stop. `kick` sends at once regardless of
    /// the interval; `replay` starts the whole log over (with the same identifiers).
    pub fn run(mut self, control: Arc<SyncControl>) {
        let interval = std::time::Duration::from_millis(self.config.batch_interval_ms.max(50));
        while !control.stop.load(Ordering::Relaxed) {
            if control.replay.swap(false, Ordering::Relaxed) {
                match self.store.outbox_reset() {
                    Ok(()) => tracing::info!("replaying the outbox from the first episode"),
                    Err(e) => self.note_error(e.to_string()),
                }
                self.not_before_ms = 0;
                self.backoff_ms = 0;
            }
            if control.kick.swap(false, Ordering::Relaxed) {
                self.not_before_ms = 0;
                self.backoff_ms = 0;
            }
            let report = self.tick(wall_ms());
            if let TickOutcome::Unreachable(reason) = &report.outcome {
                tracing::info!(reason, backoff_ms = self.backoff_ms, "peer unreachable");
            }
            // Sleep in small steps so a stop or a kick is answered within a tenth of a second.
            let mut slept = std::time::Duration::ZERO;
            while slept < interval && !control.stop.load(Ordering::Relaxed) {
                if control.kick.load(Ordering::Relaxed) || control.replay.load(Ordering::Relaxed) {
                    break;
                }
                let step = std::time::Duration::from_millis(100).min(interval - slept);
                std::thread::sleep(step);
                slept += step;
            }
        }
    }
}

fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Recording;
    use cortex_core::config::FederationTransport;
    use cortex_core::model::{Action, Actor};
    use std::sync::{Arc, Mutex};

    /// A recording transport the test can still look at after the worker took it.
    struct Shared(Arc<Mutex<Recording>>);
    impl Transport for Shared {
        fn name(&self) -> String {
            "shared".into()
        }
        fn push(&mut self, batch: &SyncBatch) -> Result<crate::protocol::SyncAck, TransportError> {
            self.0.lock().unwrap().push(batch)
        }
    }

    fn config() -> FederationConfig {
        FederationConfig {
            enabled: true,
            transport: FederationTransport::FileDrop,
            batch_size: 10,
            batch_interval_ms: 1_000,
            max_backoff_ms: 8_000,
            ..FederationConfig::default()
        }
    }

    fn worker(config: FederationConfig) -> (Worker, Arc<Store>, Arc<Mutex<Recording>>) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let peer = Arc::new(Mutex::new(Recording::default()));
        let w = Worker::new(store.clone(), Box::new(Shared(peer.clone())), config).unwrap();
        (w, store, peer)
    }

    fn file(path: &str, now: i64) -> cortex_store::queries::NodeUpsert {
        cortex_store::queries::NodeUpsert {
            kind: cortex_core::model::NodeKind::File,
            path: std::path::PathBuf::from(path),
            parent_id: None,
            mime: None,
            size: 1,
            mtime: now,
            ctime: now,
            inode: 0,
            dev: 0,
            zone_id: None,
        }
    }

    fn member(store: &Store, concept: NodeId, node: NodeId, now: i64) {
        store
            .insert_edge(
                concept,
                node,
                cortex_core::model::EdgeType::DerivedFrom,
                1.0,
                cortex_core::model::EdgeSource::Inferred,
                now,
            )
            .unwrap();
    }

    fn remember(store: &Store, ts: i64, text: &str) {
        store
            .append_episode(
                ts,
                &Actor::Agent("t".into()),
                Action::Remember,
                None,
                None,
                &serde_json::json!({ "scope": "global", "kind": "fact", "text": text }),
            )
            .unwrap();
    }

    #[test]
    fn a_disabled_worker_touches_nothing() {
        let (mut w, store, peer) = worker(FederationConfig::default());
        remember(&store, 1, "a");
        assert_eq!(w.tick(0).outcome, TickOutcome::Disabled);
        assert_eq!(store.outbox_counts().unwrap().pending, 0);
        assert!(peer.lock().unwrap().batches.is_empty());
    }

    #[test]
    fn harvest_twice_enqueues_once_and_the_cursor_moves_only_forward() {
        let (w, store, _) = worker(config());
        remember(&store, 1, "a");
        remember(&store, 2, "b");
        assert_eq!(w.harvest(0).unwrap(), 2);
        assert_eq!(w.harvest(0).unwrap(), 0);
        assert_eq!(store.outbox_cursor().unwrap(), 2);
        store.set_outbox_cursor(0).unwrap();
        assert_eq!(w.harvest(0).unwrap(), 0, "the same episodes are the same events");
    }

    #[test]
    fn batches_are_bounded_spaced_and_in_clock_order() {
        let (mut w, store, peer) = worker(config());
        for i in 0..25 {
            remember(&store, i, &format!("m{i}"));
        }
        let mut now = 0;
        let mut sends = Vec::new();
        for _ in 0..50 {
            let r = w.tick(now);
            match r.outcome {
                TickOutcome::Sent => sends.push((now, r.sent)),
                TickOutcome::Waiting { until_ms } => now = until_ms,
                TickOutcome::Idle => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(sends.iter().map(|s| s.1).collect::<Vec<_>>(), [10, 10, 5]);
        assert_eq!(sends.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 1000, 2000]);
        let peer = peer.lock().unwrap();
        let clocks: Vec<i64> = peer.batches.iter().flat_map(|b| b.events.iter().map(|e| e.lamport)).collect();
        assert!(clocks.windows(2).all(|w| w[0] < w[1]), "{clocks:?}");
        assert_eq!(store.outbox_counts().unwrap().pending, 0);
        assert_eq!(store.sync_state("peer").unwrap().last_acked_lamport, 25);
    }

    #[test]
    fn an_unreachable_peer_costs_no_attempts_and_the_wait_doubles_to_a_ceiling() {
        let (mut w, store, peer) = worker(config());
        remember(&store, 1, "a");
        peer.lock().unwrap().down = true;
        let mut now = 0;
        let mut waits = Vec::new();
        for _ in 0..6 {
            let r = w.tick(now);
            assert!(matches!(r.outcome, TickOutcome::Unreachable(_)), "{r:?}");
            let until = match w.tick(now).outcome {
                TickOutcome::Waiting { until_ms } => until_ms,
                other => panic!("{other:?}"),
            };
            waits.push(until - now);
            now = until;
        }
        assert_eq!(waits, [1000, 2000, 4000, 8000, 8000, 8000]);
        let row = &store.outbox_pending(1).unwrap()[0];
        assert_eq!(row.attempts, 0, "silence is not a refusal");
        peer.lock().unwrap().down = false;
        assert_eq!(w.tick(now).outcome, TickOutcome::Sent);
        assert_eq!(w.tick(now + 1000).outcome, TickOutcome::Idle);
    }

    #[test]
    fn a_rejected_event_counts_an_attempt_and_is_offered_again() {
        let (mut w, store, peer) = worker(config());
        remember(&store, 1, "fine");
        remember(&store, 2, "poison");
        peer.lock().unwrap().reject_containing = Some("poison".into());
        let r = w.tick(0);
        assert_eq!((r.accepted, r.rejected), (1, 1));
        let pending = store.outbox_pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 1);
        assert_eq!(pending[0].last_error.as_deref(), Some("contains \"poison\""));
        let r = w.tick(1000);
        assert_eq!((r.sent, r.rejected), (1, 1));
        assert_eq!(store.outbox_pending(10).unwrap()[0].attempts, 2);
    }

    #[test]
    fn a_peer_asking_for_quiet_gets_it_and_its_clock_is_observed() {
        let (mut w, store, peer) = worker(config());
        remember(&store, 1, "a");
        remember(&store, 2, "b");
        {
            let mut p = peer.lock().unwrap();
            p.quiet_for_ms = Some(60_000);
            p.lamport = Some(500);
        }
        assert_eq!(w.tick(0).outcome, TickOutcome::Sent);
        assert_eq!(
            w.tick(59_999).outcome,
            TickOutcome::Waiting { until_ms: 60_000 }
        );
        assert!(store.lamport().unwrap() >= 500);
        remember(&store, 3, "c");
        assert_eq!(w.tick(60_000).outcome, TickOutcome::Sent);
        let clocks: Vec<i64> = peer.lock().unwrap().batches[1].events.iter().map(|e| e.lamport).collect();
        assert!(clocks[0] > 500, "a later event sorts after the peer's clock: {clocks:?}");
    }

    #[test]
    fn an_ack_naming_an_event_that_was_not_in_the_batch_is_ignored() {
        let (mut w, store, _) = worker(config());
        remember(&store, 1, "a");
        struct Liar;
        impl Transport for Liar {
            fn name(&self) -> String {
                "liar".into()
            }
            fn push(&mut self, _: &SyncBatch) -> Result<crate::protocol::SyncAck, TransportError> {
                Ok(crate::protocol::SyncAck {
                    accepted: vec!["not-yours".into()],
                    ..Default::default()
                })
            }
        }
        w.transport = Box::new(Liar);
        let r = w.tick(0);
        assert_eq!((r.sent, r.accepted), (1, 0));
        assert_eq!(store.outbox_counts().unwrap().pending, 1, "silence about a row means resend");
    }

    #[test]
    fn a_concept_is_sent_once_per_membership() {
        let (mut w, store, peer) = worker(config());
        let now = 10;
        let a = store.upsert_node(&file("/tmp/a", now), now).unwrap();
        let concept = store.upsert_concept("things", now).unwrap();
        member(&store, concept, a, now);
        assert_eq!(w.tick(0).outcome, TickOutcome::Sent);
        let second = w.tick(1000);
        assert_eq!(second.outcome, TickOutcome::Idle, "{:?}", peer.lock().unwrap().batches);
        let b = store.upsert_node(&file("/tmp/b", now), now).unwrap();
        member(&store, concept, b, now);
        assert_eq!(w.tick(2000).outcome, TickOutcome::Sent);
        let p = peer.lock().unwrap();
        assert_eq!(p.batches.len(), 2);
        assert_eq!(p.batches[1].events[0].payload["member_count"], 2);
        assert_eq!(p.batches[1].events[0].kind, "concept");
    }
}
