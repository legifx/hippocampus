//! Domain types, configuration and filesystem locations for the Cortex daemon.
//!
//! This crate deliberately has no I/O beyond reading configuration and creating the
//! directories the daemon owns. It must stay dependency-light: everything else in the
//! workspace depends on it.

pub mod config;
pub mod denylist;
pub mod error;
pub mod ids;
pub mod model;
pub mod paths;

pub use error::{Error, Result};
