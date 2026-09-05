//! How bytes reach a peer. The protocol does not care (D17).

use crate::protocol::{SyncAck, SyncBatch};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The peer could not be reached at all. Not evidence against any event: the batch is
    /// retried whole, later, and no attempt is counted (D18).
    #[error("peer unreachable: {0}")]
    Unreachable(String),

    /// The peer answered and refused the whole batch. Every event in it counts one attempt.
    #[error("peer refused the batch: {0}")]
    Refused(String),

    /// The peer answered something that is not an acknowledgement.
    #[error("peer answered nonsense: {0}")]
    Malformed(String),
}

/// One way of delivering a batch. Synchronous: the worker runs on its own thread and a
/// transport that blocks blocks only itself.
pub trait Transport {
    /// A name for `sync.status`. Never an address.
    fn name(&self) -> String;

    fn push(&mut self, batch: &SyncBatch) -> Result<SyncAck, TransportError>;
}

/// A transport that remembers everything and can be told how to answer. The test peer for
/// the unit tests; the integration tests use the real binary.
#[derive(Default)]
pub struct Recording {
    pub batches: Vec<SyncBatch>,
    /// Ids seen, in arrival order, duplicates included — so a test can prove there were none.
    pub arrivals: Vec<String>,
    pub down: bool,
    pub reject_containing: Option<String>,
    pub quiet_for_ms: Option<u64>,
    pub lamport: Option<i64>,
}

impl Transport for Recording {
    fn name(&self) -> String {
        "recording".into()
    }

    fn push(&mut self, batch: &SyncBatch) -> Result<SyncAck, TransportError> {
        if self.down {
            return Err(TransportError::Unreachable("down for the test".into()));
        }
        self.batches.push(batch.clone());
        let mut ack = SyncAck {
            quiet_for_ms: self.quiet_for_ms,
            lamport: self.lamport,
            ..SyncAck::default()
        };
        for event in &batch.events {
            self.arrivals.push(event.event_id.clone());
            match &self.reject_containing {
                Some(needle) if event.payload.to_string().contains(needle.as_str()) => {
                    ack.rejected.push(crate::protocol::Rejected {
                        event_id: event.event_id.clone(),
                        reason: format!("contains {needle:?}"),
                    })
                }
                _ => ack.accepted.push(event.event_id.clone()),
            }
        }
        Ok(ack)
    }
}
