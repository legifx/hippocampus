//! The kill switch (hard rule R7).
//!
//! Two independent switches, because they answer different fears: *stop looking at my files*
//! (indexing) and *stop letting agents act* (the Agent Surface). A user who wants one rarely
//! wants the other.
//!
//! Both survive a restart, which is the whole point — a kill switch that a crash re-arms is
//! not a kill switch. They are stored in `meta` rather than in the config file because they
//! are runtime state, and rewriting a user's configuration file behind their back to record a
//! temporary pause is worse than the problem it solves.
//!
//! Configuration and runtime state are not symmetrical: `agent.enabled = false` in the config
//! is a hard off that no RPC can lift, while `true` merely permits the runtime state to decide.
//! An operator who disabled the surface in a file must not find it back on because something
//! called `agent.set_enabled` (D32).

use cortex_store::Store;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `meta` keys. Changing either of these silently resets a user's switch to its default, so
/// they are constants rather than literals at the call sites.
pub const META_AGENT_SURFACE: &str = "agent_surface_enabled";
pub const META_INDEXING_PAUSED: &str = "indexing_paused";

/// The persisted pair of switches. Reads are lock-free; a write goes to SQLite first and to
/// memory afterwards, so a failed write cannot leave the daemon believing it is safer — or
/// less safe — than the record on disk.
pub struct KillSwitch {
    store: Arc<Store>,
    agent_surface: AtomicBool,
    indexing_paused: AtomicBool,
}

impl KillSwitch {
    /// Load persisted state, defaulting to `config_default` for the surface and to running for
    /// indexing. An unreadable or nonsensical value is treated as absent.
    pub fn load(store: Arc<Store>, config_default: bool) -> cortex_store::error::Result<Self> {
        let surface = read_bool(&store, META_AGENT_SURFACE)?.unwrap_or(config_default);
        let paused = read_bool(&store, META_INDEXING_PAUSED)?.unwrap_or(false);
        Ok(Self {
            store,
            agent_surface: AtomicBool::new(surface),
            indexing_paused: AtomicBool::new(paused),
        })
    }

    /// State only, with nothing behind it. Tests and one-shot tools.
    pub fn in_memory(store: Arc<Store>, surface: bool, paused: bool) -> Self {
        Self {
            store,
            agent_surface: AtomicBool::new(surface),
            indexing_paused: AtomicBool::new(paused),
        }
    }

    pub fn agent_surface_enabled(&self) -> bool {
        self.agent_surface.load(Ordering::Relaxed)
    }

    pub fn indexing_paused(&self) -> bool {
        self.indexing_paused.load(Ordering::Relaxed)
    }

    /// Returns the state actually in force afterwards.
    pub fn set_agent_surface(&self, enabled: bool) -> cortex_store::error::Result<bool> {
        self.store
            .meta_set(META_AGENT_SURFACE, if enabled { "1" } else { "0" })?;
        self.agent_surface.store(enabled, Ordering::Relaxed);
        Ok(enabled)
    }

    pub fn set_indexing_paused(&self, paused: bool) -> cortex_store::error::Result<bool> {
        self.store
            .meta_set(META_INDEXING_PAUSED, if paused { "1" } else { "0" })?;
        self.indexing_paused.store(paused, Ordering::Relaxed);
        Ok(paused)
    }
}

fn read_bool(store: &Store, key: &str) -> cortex_store::error::Result<Option<bool>> {
    Ok(store.meta_get(key)?.and_then(|v| match v.as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Arc<Store> {
        Arc::new(Store::open_in_memory().unwrap())
    }

    #[test]
    fn defaults_come_from_configuration_when_nothing_is_persisted() {
        let s = store();
        assert!(KillSwitch::load(s.clone(), true).unwrap().agent_surface_enabled());
        assert!(!KillSwitch::load(s, false).unwrap().agent_surface_enabled());
    }

    #[test]
    fn state_survives_a_reload() {
        let s = store();
        let ks = KillSwitch::load(s.clone(), true).unwrap();
        ks.set_agent_surface(false).unwrap();
        ks.set_indexing_paused(true).unwrap();

        // A fresh daemon over the same database — the config still says `true`, and must lose.
        let reloaded = KillSwitch::load(s, true).unwrap();
        assert!(!reloaded.agent_surface_enabled());
        assert!(reloaded.indexing_paused());
    }

    #[test]
    fn the_two_switches_are_independent() {
        let s = store();
        let ks = KillSwitch::load(s, true).unwrap();
        ks.set_indexing_paused(true).unwrap();
        assert!(ks.agent_surface_enabled(), "pausing indexing does not disarm agents");
        ks.set_agent_surface(false).unwrap();
        ks.set_indexing_paused(false).unwrap();
        assert!(!ks.agent_surface_enabled());
    }

    #[test]
    fn a_corrupt_value_falls_back_to_the_default() {
        let s = store();
        s.meta_set(META_AGENT_SURFACE, "perhaps").unwrap();
        assert!(!KillSwitch::load(s, false).unwrap().agent_surface_enabled());
    }
}
