//! Retrieval for the Cortex daemon.
//!
//! The arms are independent and none of them knows about the others: the path index matches
//! paths, `lexical` prepares queries for the content index in `cortex-store`, and `fusion`
//! combines whatever rankings it is handed. Adding the vector arm in M2's second half is
//! therefore an addition here, not a rewrite — it hands `fusion` one more `Ranking`.
//!
//! The ranking modules — `fuzzy`, `lexical`, `mode`, `fusion`, `path_index` — touch neither SQL
//! nor the filesystem, and are testable without either. [`search`] is the one place that
//! composes them against a real store, and it lives here rather than in the daemon so that the
//! benchmark measures the code that ships instead of a second copy of it.

pub mod fusion;
pub mod fuzzy;
pub mod lexical;
pub mod mode;
pub mod path_index;
pub mod search;

pub use fusion::{fuse, Arm, Fused, Ranking};
pub use path_index::{PathHit, PathIndex};
