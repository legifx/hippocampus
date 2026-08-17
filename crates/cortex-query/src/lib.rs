//! Retrieval for the Cortex daemon.
//!
//! M1 scope: the `locate` mode — the in-memory path index and its fuzzy scorer. Lexical and
//! vector retrieval, reciprocal rank fusion and reranking arrive in M2 and slot in alongside
//! this, not on top of it.

pub mod fuzzy;
pub mod path_index;

pub use path_index::{PathHit, PathIndex};
