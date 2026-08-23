//! Salience, decay, tiering and edge formation — M5, and nothing that touches a disk.
//!
//! Everything here is a pure function over values the storage layer hands in. That is not
//! tidiness for its own sake: the milestone's Definition of Done asks for a thirty-day
//! simulation, and a simulation that had to write a real index for every simulated day would
//! be slow enough that nobody would run it. Here a day is a number.
//!
//! The three ideas, in the order they matter:
//!
//! 1. **Salience is a sum of named components, never an opaque number.** Every score carries
//!    its own breakdown and the breakdown sums to the score. A ranking that cannot say why is
//!    a ranking nobody has grounds to trust, and `brain.explain` exists to print it.
//! 2. **Forgetting is demotion, not deletion.** A node below a threshold moves down a tier.
//!    Because the index is reconstructible from the filesystem plus `episodes` (R10), a
//!    demotion is always reversible, which is what makes aggressive forgetting safe.
//! 3. **Association comes from co-occurrence in time.** Two files touched inside the same
//!    burst of activity are related in a way no content similarity measure would notice —
//!    a test and the module it covers, a design note and the code it argued about.
//! 4. **A cluster that keeps recurring earns a name.** Abstraction turns a community of
//!    associated files into one concept node, which is the step from "these are related" to
//!    "this is the parser work" — see [`concepts`].

pub mod coaccess;
pub mod concepts;
pub mod consolidate;
pub mod salience;
pub mod tier;

pub use coaccess::{Burst, CoAccessConfig, DerivedEdge, derive_co_access};
pub use concepts::{Cluster, ClusterConfig, Link, Named, cluster};
pub use consolidate::{ConceptView, ConsolidationReport, TierMovement};
pub use salience::{Components, Inputs, Weights, decay, effective_count, score};
pub use tier::{TierPolicy, assign_tier};
