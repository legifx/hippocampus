//! Federation (M7): the outbox protocol and the worker that drains it.
//!
//! Two one-way streets, never a crossing (D11): events go up, an ontology comes down. The
//! Cortex owns idempotency (D18) — every event carries an identifier that is the same on every
//! resend, and a peer that keeps nothing but a set of identifiers can still refuse a duplicate.
//! Vectors stay home (D19): what travels is text and names.
//!
//! **This crate knows no endpoint.** It ships a wire format ([`protocol`]), a [`Transport`]
//! trait, and two transports that need no address: one writes files into a directory
//! ([`filedrop`]), the other hands batches to a declared subprocess ([`plugin`]). Whatever
//! talks to a real peer is the Extension's, behind that subprocess boundary (D9, D17).
//!
//! The wire format is documented in `docs/08-FEDERATION.md`; `hc-test-peer` is the reference
//! peer and the thing the tests assert against.

pub mod derive;
pub mod filedrop;
pub mod plugin;
pub mod protocol;
pub mod transport;
pub mod worker;

pub use protocol::{Ontology, Rejected, SyncAck, SyncBatch, SyncEvent, PROTOCOL_VERSION};
pub use transport::{Transport, TransportError};
pub use worker::{SyncControl, SyncStatus, TickOutcome, TickReport, Worker};
