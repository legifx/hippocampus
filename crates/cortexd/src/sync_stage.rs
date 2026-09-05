//! Federation in the daemon (M7): one thread, one worker, and a handle the RPC side reads.
//!
//! Off by default (R5). When it is on, the transport is built from the configuration alone —
//! a directory, or a plugin declared by name — and the daemon never learns more than that
//! about the peer. A declaration that cannot be read is reported in `sync.status`, not fatal:
//! the file manager does not stop working because the bridge is misconfigured (D10).

use cortex_core::config::{Config, FederationTransport};
use cortex_core::paths::Paths;
use cortex_core::plugins::PluginDecl;
use cortex_store::Store;
use cortex_sync::{SyncControl, SyncStatus, Transport, Worker};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub struct Federation {
    pub status: Arc<Mutex<SyncStatus>>,
    /// `None` when there is no worker to control: federation is off, or could not start.
    pub control: Option<Arc<SyncControl>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Federation {
    /// Build the transport and start the worker, or explain in the status why not.
    pub fn start(store: Arc<Store>, config: &Config, paths: &Paths) -> Self {
        let fed = &config.federation;
        let origin = store.sync_origin().unwrap_or_default();
        let lamport = store.lamport().unwrap_or(0);
        let off = |reason: Option<String>| {
            let status = SyncStatus {
                enabled: fed.enabled,
                transport: "none".into(),
                peer: fed.peer.clone(),
                origin: origin.clone(),
                pending: 0,
                dead: 0,
                acked: 0,
                episodes_behind: 0,
                lamport,
                last_acked_lamport: 0,
                last_sync_at: None,
                batches_sent: 0,
                events_accepted: 0,
                events_rejected: 0,
                backoff_ms: 0,
                not_before_ms: 0,
                last_error: reason,
                last_tick: None,
                ontology_version: None,
            };
            Federation {
                status: Arc::new(Mutex::new(status)),
                control: None,
                thread: Mutex::new(None),
            }
        };

        if !fed.enabled {
            return off(None);
        }

        let transport: Box<dyn Transport + Send> = match fed.transport {
            FederationTransport::None => {
                return off(Some("federation.transport is none".into()));
            }
            FederationTransport::FileDrop => match &fed.drop_dir {
                Some(dir) => Box::new(cortex_sync::filedrop::FileDrop::new(dir)),
                None => return off(Some("federation.drop_dir is not set".into())),
            },
            FederationTransport::Plugin => {
                let Some(name) = &fed.plugin else {
                    return off(Some("federation.plugin is not set".into()));
                };
                match PluginDecl::load(&paths.plugins_dir(), name) {
                    Ok(decl) => Box::new(cortex_sync::plugin::Plugin::new(name.clone(), decl)),
                    Err(e) => {
                        tracing::warn!(error = %e, "federation plugin declaration unreadable");
                        return off(Some(format!("plugin {name:?}: {e}")));
                    }
                }
            }
        };

        let worker = match Worker::new(store, transport, fed.clone()) {
            Ok(worker) => worker,
            Err(e) => {
                tracing::warn!(error = %e, "federation could not start");
                return off(Some(e.to_string()));
            }
        };
        let status = worker.status();
        let control = Arc::new(SyncControl::default());
        let thread = {
            let control = control.clone();
            std::thread::Builder::new()
                .name("cortex-sync".into())
                .spawn(move || worker.run(control))
        };
        match thread {
            Ok(handle) => {
                tracing::info!(
                    transport = %status.lock().map(|s| s.transport.clone()).unwrap_or_default(),
                    peer = %fed.peer,
                    "federation worker started"
                );
                Federation {
                    status,
                    control: Some(control),
                    thread: Mutex::new(Some(handle)),
                }
            }
            Err(e) => off(Some(format!("could not spawn the sync thread: {e}"))),
        }
    }

    pub fn snapshot(&self) -> SyncStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Send now, regardless of the interval or the backoff. False when there is no worker.
    pub fn kick(&self) -> bool {
        match &self.control {
            Some(control) => {
                control.kick.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Start the whole log over, with the same identifiers. False when there is no worker.
    pub fn replay(&self) -> bool {
        match &self.control {
            Some(control) => {
                control.replay.store(true, Ordering::Relaxed);
                control.kick.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Stop the worker and wait for it. Idempotent.
    pub fn stop(&self) {
        if let Some(control) = &self.control {
            control.stop.store(true, Ordering::Relaxed);
        }
        if let Ok(mut slot) = self.thread.lock() {
            if let Some(handle) = slot.take() {
                if handle.join().is_err() {
                    tracing::warn!("sync thread did not stop cleanly");
                }
            }
        }
    }
}

/// One sentence for a human, from the numbers.
pub fn describe(s: &SyncStatus) -> String {
    if !s.enabled {
        return "federation is off; nothing leaves this machine".to_string();
    }
    if s.transport == "none" {
        return format!(
            "federation is on but has no working transport: {}",
            s.last_error.as_deref().unwrap_or("no reason recorded")
        );
    }
    let mut parts = vec![format!(
        "{} event{} waiting, {} confirmed, {} refused for good",
        s.pending,
        if s.pending == 1 { "" } else { "s" },
        s.acked,
        s.dead
    )];
    if s.episodes_behind > 0 {
        parts.push(format!("{} episodes not yet harvested", s.episodes_behind));
    }
    match (&s.last_error, s.backoff_ms) {
        (Some(err), backoff) if backoff > 0 => {
            parts.push(format!("peer unreachable ({err}); trying again in {} s", backoff / 1000))
        }
        (Some(err), _) => parts.push(format!("last error: {err}")),
        (None, _) => {}
    }
    if let Some(at) = s.last_sync_at {
        parts.push(format!("peer last reached at {at}"));
    } else {
        parts.push("peer never reached".to_string());
    }
    parts.join("; ")
}
