//! Filesystem events, before and after coalescing.

use std::path::PathBuf;

/// A raw event as the kernel reported it.
///
/// Deliberately close to what `inotify` delivers. Interpretation belongs in the coalescer,
/// where it can be tested without a filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    pub path: PathBuf,
    pub kind: RawKind,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Created,
    /// Content changed and the writer closed the file. This, not `IN_MODIFY`, is the signal
    /// that a file is worth re-reading: editors write in several bursts per save.
    Written,
    /// Metadata changed — permissions, ownership, timestamps.
    AttributeChanged,
    Deleted,
    /// The source half of a rename. Paired with `MovedTo` by its cookie.
    MovedFrom(u32),
    /// The destination half of a rename.
    MovedTo(u32),
    /// The kernel dropped events because our queue overflowed. The index may now be wrong
    /// anywhere, so the only correct response is a full reconcile.
    Overflow,
}

impl RawKind {
    /// Events of the same class collapse into one another during the coalescing window.
    ///
    /// Creation and writing share a class on purpose: a new file that is immediately written
    /// is one thing happening, not two, and the ingest pipeline treats both as "look at this
    /// path".
    pub fn class(self) -> EventClass {
        match self {
            RawKind::Created | RawKind::Written => EventClass::Upsert,
            RawKind::AttributeChanged => EventClass::Attributes,
            RawKind::Deleted => EventClass::Delete,
            RawKind::MovedFrom(_) | RawKind::MovedTo(_) => EventClass::Move,
            RawKind::Overflow => EventClass::Overflow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventClass {
    Upsert,
    Attributes,
    Delete,
    Move,
    Overflow,
}

/// What the ingest pipeline should actually do, after coalescing and move pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Index or re-index this path.
    Upsert { path: PathBuf, is_dir: bool },
    /// Refresh metadata only; the content did not change.
    Touch { path: PathBuf },
    /// The path is gone.
    Remove { path: PathBuf },
    /// A rename, identified exactly by a matched cookie pair. Not a delete plus a create:
    /// preserving identity keeps the node's history, its salience and its embeddings.
    Rename { from: PathBuf, to: PathBuf },
    /// Events were lost. Everything below this root must be re-examined.
    ResyncRequired,
}
