//! Identifier newtypes.
//!
//! These are newtypes rather than bare integers on purpose: a `NodeId` and an `EpisodeId` are
//! both `i64` in SQLite, and mixing them up is the kind of bug that produces plausible-looking
//! wrong answers rather than a crash.

use std::fmt;

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                Self(v)
            }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> Self {
                v.0
            }
        }
    };
}

id_type!(NodeId, "Identifies a node: a file, directory, chunk, concept or annotation.");
id_type!(EpisodeId, "Identifies an episode. Episodes are append-only and never reused.");
id_type!(ChunkId, "Identifies a chunk of extracted content.");
id_type!(ZoneId, "Identifies a spatial zone (a major root of the tree).");

/// A session groups episodes that belong to one continuous stretch of work.
///
/// Sessions are opaque strings rather than integers because they are also produced by clients
/// (an agent names its own session) and must survive a daemon restart.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
