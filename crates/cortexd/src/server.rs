//! The JSON-RPC server: accept loop and method dispatch.

use crate::indexer;
use crate::ingest::IngestState;
use cortex_core::config::Config;
use cortex_core::ids::{NodeId, SessionId};
use cortex_core::model::{Action, Actor, NodeKind, ScoreBreakdown, SearchHit, Tier};
use cortex_query::PathIndex;
use cortex_rpc::protocol::{
    method, EpisodeAppendParams, EpisodesRecentParams, EpisodesRecentResult, IndexPathParams,
    IndexPathResult, NodeGetParams, NodeGetResult, PingResult, ReconcileParams, ReconcileResult,
    ReconcileSummary, Request, Response, RpcError, SearchParams, SearchResult, StatusResult,
    WatchStatusResult, ZoneInfo, ZoneSetColorParams, ZonesListResult,
};
use cortex_rpc::transport;
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Daemon {
    pub store: Arc<Store>,
    pub index: Arc<RwLock<PathIndex>>,
    pub ingest: Arc<IngestState>,
    pub config: Config,
    pub database_path: PathBuf,
    pub started: Instant,
    /// Signalled when a client calls `shutdown`.
    pub shutdown: Arc<Notify>,
}

impl Daemon {
    pub fn new(
        store: Arc<Store>,
        index: Arc<RwLock<PathIndex>>,
        ingest: Arc<IngestState>,
        config: Config,
        database_path: PathBuf,
    ) -> Self {
        Self {
            store,
            index,
            ingest,
            config,
            database_path,
            started: Instant::now(),
            shutdown: Arc::new(Notify::new()),
        }
    }

    fn watch_status(&self) -> WatchStatusResult {
        let health = self.ingest.watch_health.lock().unwrap().clone();
        let last = self.ingest.last_reconcile.lock().unwrap().map(|r| ReconcileSummary {
            added: r.added,
            updated: r.updated,
            removed: r.removed,
            renamed: r.renamed,
            unchanged: r.unchanged,
            skipped: r.skipped,
        });

        match health {
            Some(health) => WatchStatusResult {
                active: true,
                watched_dirs: health.watched_dirs,
                budget: health.budget,
                degraded: health.degraded,
                unwatched: health.unwatched,
                overflows: health.overflows,
                paused: self.ingest.is_paused(),
                roots: self
                    .config
                    .index
                    .metadata_roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                last_reconcile: last,
            },
            None => WatchStatusResult {
                active: false,
                watched_dirs: 0,
                budget: self.config.index.watch_budget,
                degraded: !self.config.index.metadata_roots.is_empty(),
                unwatched: 0,
                overflows: 0,
                paused: self.ingest.is_paused(),
                roots: self
                    .config
                    .index
                    .metadata_roots
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                last_reconcile: last,
            },
        }
    }

    /// Build search hits from the in-memory path index.
    ///
    /// The database is consulted only for the handful of rows that survived ranking. Reading
    /// tier and salience for a hundred thousand candidates in order to display fifty of them
    /// would undo everything the index is for.
    fn search_paths(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let hits = self.index.read().unwrap().search(query, limit);

        hits.into_iter()
            .enumerate()
            .map(|(rank, hit)| {
                let node = self.store.node_by_id(hit.node_id).ok().flatten();
                SearchHit {
                    node_id: hit.node_id,
                    path: PathBuf::from(&hit.path),
                    kind: node.as_ref().map(|n| n.kind).unwrap_or(NodeKind::File),
                    tier: node.as_ref().map(|n| n.tier).unwrap_or(Tier::Stm),
                    zone_id: node.as_ref().and_then(|n| n.zone_id),
                    snippet: None,
                    score: ScoreBreakdown {
                        path_rank: Some(rank as u32 + 1),
                        lexical_rank: None,
                        vector_rank: None,
                        fused: hit.score as f64,
                        salience_boost: 1.0,
                        rerank: None,
                        final_score: hit.score as f64,
                    },
                }
            })
            .collect()
    }
}

/// Bind the Unix socket.
///
/// A socket left behind by a crashed daemon would make `bind` fail with `EADDRINUSE` forever,
/// so a stale one is removed. This is safe because the instance lock has already been acquired
/// by the time we get here — if another daemon were alive, we would not have reached this point.
pub fn bind(socket_path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    restrict_socket(socket_path)?;
    Ok(listener)
}

#[cfg(unix)]
fn restrict_socket(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // The socket is the entire control surface of the daemon. Anything looser than 0600 would
    // hand every local account read access to the user's index.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_socket(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Serve until the shutdown signal fires.
pub async fn serve(listener: UnixListener, daemon: Arc<Daemon>) {
    let shutdown = daemon.shutdown.clone();

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("shutdown requested");
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let daemon = daemon.clone();
                        // One task per connection. A slow or misbehaving client must not be
                        // able to stall the others.
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, daemon).await {
                                tracing::debug!(error = %e, "connection ended");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    daemon: Arc<Daemon>,
) -> Result<(), transport::TransportError> {
    let mut reader = BufReader::new(stream);

    loop {
        let value = match transport::read_message(&mut reader).await {
            Ok(Some(value)) => value,
            Ok(None) => return Ok(()),
            Err(e) => {
                // A malformed message is the client's problem, not a reason to drop the
                // connection silently — answer with a parse error and carry on.
                let response = Response::err(
                    serde_json::Value::Null,
                    RpcError::new(RpcError::PARSE_ERROR, e.to_string()),
                );
                let _ = transport::write_message(
                    reader.get_mut(),
                    &serde_json::to_value(&response).unwrap(),
                )
                .await;
                return Err(e);
            }
        };

        let request: Request = match serde_json::from_value(value) {
            Ok(request) => request,
            Err(e) => {
                let response = Response::err(
                    serde_json::Value::Null,
                    RpcError::new(RpcError::INVALID_REQUEST, e.to_string()),
                );
                transport::write_message(
                    reader.get_mut(),
                    &serde_json::to_value(&response).unwrap(),
                )
                .await?;
                continue;
            }
        };

        let id = request.id.clone();
        let outcome = dispatch(&daemon, &request.method, request.params).await;

        // A request without an id is a notification: the caller expects no answer, and sending
        // one would desynchronise its stream.
        let Some(id) = id else { continue };

        let response = match outcome {
            Ok(result) => Response::ok(id, result),
            Err(error) => Response::err(id, error),
        };
        transport::write_message(reader.get_mut(), &serde_json::to_value(&response).unwrap())
            .await?;
    }
}

async fn dispatch(
    daemon: &Arc<Daemon>,
    method: &str,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, RpcError> {
    match method {
        method::PING => json(PingResult {
            pong: true,
            version: VERSION.to_string(),
        }),

        method::STATUS => {
            let stats = daemon.store.stats().map_err(internal)?;
            let schema_version = daemon.store.schema_version().map_err(internal)?;
            json(StatusResult {
                version: VERSION.to_string(),
                schema_version,
                uptime_secs: daemon.started.elapsed().as_secs(),
                database: daemon.database_path.display().to_string(),
                files: stats.files,
                dirs: stats.dirs,
                chunks: stats.chunks,
                episodes: stats.episodes,
                embeddings: stats.embeddings,
                outbox_pending: stats.outbox_pending,
                agent_surface_enabled: daemon.config.agent.enabled,
                indexed_paths: daemon.index.read().unwrap().len(),
                index_memory_bytes: daemon.index.read().unwrap().memory_bytes(),
                watcher: Some(daemon.watch_status()),
                methods: method::ALL.iter().map(|m| m.to_string()).collect(),
            })
        }

        method::SEARCH => {
            let params: SearchParams = parse(params)?;
            let limit = params.limit.unwrap_or(daemon.config.search.default_limit);
            let started = Instant::now();

            let hits = daemon.search_paths(&params.query, limit);

            // Searching is itself an episode. Without this, "what was I looking for on
            // Tuesday" has nothing to answer from.
            let _ = daemon.store.append_episode(
                indexer::unix_now(),
                &Actor::Human,
                Action::Search,
                None,
                None,
                &serde_json::json!({ "query": params.query, "mode": params.mode }),
            );

            json(SearchResult {
                hits,
                elapsed_ms: started.elapsed().as_millis() as u64,
                partial: false,
            })
        }

        method::NODE_GET => {
            let params: NodeGetParams = parse(params)?;
            let node = match (params.id, params.path.as_deref()) {
                (Some(id), _) => daemon.store.node_by_id(id).map_err(internal)?,
                (None, Some(path)) => daemon
                    .store
                    .node_by_path(Path::new(path))
                    .map_err(internal)?,
                (None, None) => {
                    return Err(RpcError::invalid_params("either `id` or `path` is required"))
                }
            };
            json(NodeGetResult { node })
        }

        method::EPISODE_APPEND => {
            let params: EpisodeAppendParams = parse(params)?;
            let actor = Actor::parse(&params.actor).ok_or_else(|| {
                RpcError::invalid_params(format!("unknown actor: {}", params.actor))
            })?;
            let action = Action::parse(&params.action).ok_or_else(|| {
                RpcError::invalid_params(format!("unknown action: {}", params.action))
            })?;

            let id = daemon
                .store
                .append_episode(
                    indexer::unix_now(),
                    &actor,
                    action,
                    params.node_id.map(NodeId::from).map(|id| id),
                    params.session_id.map(SessionId).as_ref(),
                    &params.metadata,
                )
                .map_err(internal)?;
            json(serde_json::json!({ "id": id }))
        }

        method::EPISODES_RECENT => {
            let params: EpisodesRecentParams = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(EpisodesRecentParams { limit: None });
            let episodes = daemon
                .store
                .recent_episodes(params.limit.unwrap_or(50))
                .map_err(internal)?;
            json(EpisodesRecentResult { episodes })
        }

        method::INDEX_PATH => {
            let params: IndexPathParams = parse(params)?;
            let started = Instant::now();
            let store = daemon.store.clone();
            let root = PathBuf::from(&params.path);
            let extra_deny = daemon.config.index.extra_deny.clone();
            let recursive = params.recursive;

            // The walk is synchronous and can run for a long time. Doing it on a blocking
            // thread keeps the async runtime free to answer other clients meanwhile.
            let outcome = tokio::task::spawn_blocking(move || {
                indexer::index_path(&store, &root, recursive, &extra_deny)
            })
            .await
            .map_err(|e| RpcError::internal(format!("index task failed: {e}")))?;

            rebuild_index(daemon).map_err(internal)?;

            json(IndexPathResult {
                indexed: outcome.indexed,
                skipped: outcome.skipped,
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }

        method::WATCH_STATUS => json(daemon.watch_status()),

        method::ZONES_LIST => {
            let zones = daemon.store.zones().map_err(internal)?;
            let counts = daemon.store.zone_node_counts().map_err(internal)?;
            json(ZonesListResult {
                zones: zones
                    .into_iter()
                    .map(|z| {
                        let id = z.id.map(|i| i.0).unwrap_or_default();
                        ZoneInfo {
                            id,
                            root_path: z.root_path.display().to_string(),
                            name: z.name,
                            color_hue: z.color_hue,
                            icon: z.icon,
                            user_override: z.user_override,
                            node_count: counts.get(&id).copied().unwrap_or(0),
                        }
                    })
                    .collect(),
            })
        }

        method::ZONE_SET_COLOR => {
            let params: ZoneSetColorParams = parse(params)?;
            if params.hue >= 360 {
                return Err(RpcError::invalid_params("hue must be between 0 and 359"));
            }
            daemon
                .store
                .set_zone_color(cortex_core::ids::ZoneId(params.id), params.hue)
                .map_err(internal)?;
            json(serde_json::json!({ "ok": true }))
        }

        method::RECONCILE => {
            let params: ReconcileParams = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(ReconcileParams { path: None });

            let roots: Vec<PathBuf> = match params.path {
                Some(path) => vec![PathBuf::from(path)],
                None => daemon.config.index.metadata_roots.clone(),
            };
            if roots.is_empty() {
                return Err(RpcError::invalid_params(
                    "no path given and no metadata_roots configured",
                ));
            }

            let started = Instant::now();
            let store = daemon.store.clone();
            let extra_deny = daemon.config.index.extra_deny.clone();

            // Reconciliation walks the whole tree, so it belongs on a blocking thread. Holding
            // the async runtime for the duration would stall every concurrent query.
            let summary = tokio::task::spawn_blocking(move || {
                let mut total = cortex_ingest::reconcile::ReconcileReport::default();
                for root in &roots {
                    let report = cortex_ingest::reconcile::reconcile(
                        &store,
                        root,
                        &extra_deny,
                        indexer::unix_now(),
                    );
                    total.added += report.added;
                    total.updated += report.updated;
                    total.removed += report.removed;
                    total.renamed += report.renamed;
                    total.unchanged += report.unchanged;
                    total.skipped += report.skipped;
                }
                total
            })
            .await
            .map_err(|e| RpcError::internal(format!("reconcile task failed: {e}")))?;

            rebuild_index(daemon).map_err(internal)?;
            *daemon.ingest.last_reconcile.lock().unwrap() = Some(summary);

            json(ReconcileResult {
                summary: ReconcileSummary {
                    added: summary.added,
                    updated: summary.updated,
                    removed: summary.removed,
                    renamed: summary.renamed,
                    unchanged: summary.unchanged,
                    skipped: summary.skipped,
                },
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }

        method::SHUTDOWN => {
            daemon.ingest.stop();
            daemon.shutdown.notify_waiters();
            json(serde_json::json!({ "stopping": true }))
        }

        unknown => Err(RpcError::method_not_found(unknown)),
    }
}

/// Rebuild the in-memory path index from the database.
///
/// Used after any operation that may have changed many paths at once. Patching entry by entry
/// would be faster but would also be a second, subtly different implementation of what the
/// database already knows — and the two would drift.
fn rebuild_index(daemon: &Arc<Daemon>) -> cortex_store::error::Result<()> {
    let paths = daemon.store.all_paths_with_ids()?;
    *daemon.index.write().unwrap() = PathIndex::build(paths);
    Ok(())
}

fn parse<T: serde::de::DeserializeOwned>(
    params: Option<serde_json::Value>,
) -> Result<T, RpcError> {
    let params = params.ok_or_else(|| RpcError::invalid_params("missing params"))?;
    serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))
}

fn json<T: serde::Serialize>(value: T) -> Result<serde_json::Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::internal(e.to_string()))
}

fn internal<E: std::fmt::Display>(e: E) -> RpcError {
    RpcError::internal(e.to_string())
}
