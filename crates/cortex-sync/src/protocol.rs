//! The wire format. One batch up, one acknowledgement down.
//!
//! Everything here is plain JSON with stable field names; a peer in any language can consume
//! it with nothing but a JSON parser. The version is carried on every batch so a peer can
//! refuse a shape it does not understand rather than guess.

use serde::{Deserialize, Serialize};

/// Bumped when a field changes meaning. Adding an optional field does not bump it.
pub const PROTOCOL_VERSION: u32 = 1;

/// The JSON-RPC method a plugin transport is called with. One request per batch.
pub const METHOD_PUSH: &str = "sync.push";

/// One thing that happened here, in a form the peer can consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEvent {
    /// Stable across resends, restarts and index rebuilds. 32 hex characters.
    pub event_id: String,
    /// The Lamport tick this event was queued at. Strictly increasing per origin; the peer
    /// may order by it and must not expect gaps to be filled.
    pub lamport: i64,
    /// Which index produced it. Random, minted once; names no machine.
    pub origin: String,
    /// When the thing happened, seconds since the epoch — not when it was sent.
    pub ts: i64,
    /// `episode`, `concept`. A peer ignores kinds it does not know.
    pub kind: String,
    /// Kind-specific. See `docs/08-FEDERATION.md`.
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncBatch {
    pub protocol: u32,
    pub origin: String,
    /// The sender's clock at the time of sending. Always at least the highest `lamport` in
    /// `events`.
    pub lamport: i64,
    /// Seconds since the epoch, on the sender.
    pub sent_at: i64,
    /// In Lamport order.
    pub events: Vec<SyncEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejected {
    pub event_id: String,
    pub reason: String,
}

/// What the peer says about a batch.
///
/// Every event in the batch is expected to be in exactly one of `accepted` and `rejected`. An
/// event in neither is treated as not delivered and sent again — silence is not consent.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncAck {
    pub accepted: Vec<String>,
    pub rejected: Vec<Rejected>,
    /// The peer's own clock, if it keeps one. The sender's clock moves past it (D11).
    pub lamport: Option<i64>,
    /// Backpressure (D18): the peer asks for silence for this long. Honoured as a floor on the
    /// gap before the next batch. A peer in its nightly batch job says so here rather than
    /// dropping events.
    pub quiet_for_ms: Option<u64>,
    /// The street down: the peer's taxonomy, whole, whenever it changed. The Core stores the
    /// latest and shows it; interpreting it is the Extension's job.
    pub ontology: Option<Ontology>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ontology {
    pub version: i64,
    #[serde(default)]
    pub tags: Vec<OntologyTag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OntologyTag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An event identifier: the origin and a per-kind identity, hashed. Never the content, so a
/// resend after the payload gained a field is still the same event (D18).
pub fn event_id(origin: &str, identity: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(origin.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.as_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_depends_on_origin_and_identity_and_nothing_else() {
        assert_eq!(event_id("o", "episode:1"), event_id("o", "episode:1"));
        assert_ne!(event_id("o", "episode:1"), event_id("p", "episode:1"));
        assert_ne!(event_id("o", "episode:1"), event_id("o", "episode:2"));
        assert_eq!(event_id("o", "x").len(), 32);
    }

    #[test]
    fn an_ack_with_nothing_in_it_is_valid_and_means_nothing_was_delivered() {
        let ack: SyncAck = serde_json::from_str("{}").unwrap();
        assert!(ack.accepted.is_empty() && ack.rejected.is_empty());
        assert_eq!(ack.quiet_for_ms, None);
    }

    #[test]
    fn a_batch_round_trips_with_stable_field_names() {
        let batch = SyncBatch {
            protocol: PROTOCOL_VERSION,
            origin: "abc".into(),
            lamport: 7,
            sent_at: 1,
            events: vec![SyncEvent {
                event_id: "e".into(),
                lamport: 7,
                origin: "abc".into(),
                ts: 0,
                kind: "episode".into(),
                payload: serde_json::json!({ "action": "open" }),
            }],
        };
        let text = serde_json::to_string(&batch).unwrap();
        for field in ["\"protocol\"", "\"event_id\"", "\"lamport\"", "\"payload\""] {
            assert!(text.contains(field), "{text}");
        }
        assert_eq!(serde_json::from_str::<SyncBatch>(&text).unwrap(), batch);
    }
}
