//! Perception: how the Cortex learns that the filesystem changed.
//!
//! Three layers, deliberately separable:
//!
//! - [`watcher`] turns kernel notifications into [`event::RawEvent`]s and manages the watch set.
//! - [`coalesce`] turns bursts of raw events into the small number of things that actually
//!   happened. Pure logic, no clock, no filesystem.
//! - [`reconcile`] repairs drift, because watcher events *are* lost under load and an index
//!   that silently diverges is worse than no index at all.
//!
//! [`throttle`] sits beside them rather than under them: it is the arithmetic that keeps the
//! expensive stages — extraction today, embedding next — from being felt by the person using
//! the machine.

pub mod coalesce;
pub mod event;
pub mod reconcile;
pub mod throttle;
pub mod zone_detect;

#[cfg(target_os = "linux")]
pub mod watcher;

pub use coalesce::Coalescer;
pub use event::{Change, RawEvent, RawKind};
