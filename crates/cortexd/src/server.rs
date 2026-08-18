//! The JSON-RPC server: accept loop and method dispatch.

use crate::activity::Activity;
use crate::embed_stage::EmbedStats;
use crate::extract_stage::ExtractStats;
use crate::semantics::Semantics;
use crate::indexer;
use crate::ingest::IngestState;
use cortex_core::config::Config;
use cortex_core::ids::{NodeId, SessionId};
use cortex_core::model::{Action, Actor, QueryMode};
use cortex_ingest::throttle;
use cortex_query::search;
use cortex_query::PathIndex;
use cortex_rpc::protocol::{
    method, ActivityPingResult, EpisodeAppendParams, EpisodesRecentParams, EpisodesRecentResult,
    ExtractRequeueResult, ExtractStatusResult, IndexPathParams, IndexPathResult, ModelReloadResult,
    ModelStatusResult, NodeGetParams, NodeGetResult, PingResult, ReconcileParams, ReconcileResult,
    ReconcileSummary, Request, Response, RpcError, SearchExplainParams, SearchExplainResult,
    SearchParams, SearchResult, StatusResult, WatchStatusResult, ZoneInfo, ZoneSetColorParams,
    ZonesListResult,
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
    /// When a human was last known to be present. Drives the extraction stage's CPU share.
    pub activity: Arc<Activity>,
    pub extract_stats: Arc<ExtractStats>,
    pub embed_stats: Arc<EmbedStats>,
    /// The embedding model, if there is one. Behind a lock because it can arrive while the
    /// daemon is running — `cortex model fetch` does not need a restart.
    pub semantics: Arc<Semantics>,
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
        activity: Arc<Activity>,
        extract_stats: Arc<ExtractStats>,
        embed_stats: Arc<EmbedStats>,
        semantics: Arc<Semantics>,
        config: Config,
        database_path: PathBuf,
    ) -> Self {
        Self {
            store,
            index,
            ingest,
            activity,
            extract_stats,
            embed_stats,
            semantics,
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

    /// Run a query across every arm.
    ///
    /// The read lock on the path index is held for the length of the search. That is deliberate:
    /// a rebuild swapping the index mid-query would mix ranks from two different corpora, and
    /// the lock is uncontended except during a reconcile.
    fn run_search(&self, params: &SearchParams) -> search::Outcome {
        let index = self.index.read().unwrap();
        // Resolved per request rather than held: the model can appear, and a search started a
        // moment later should use it.
        let semantic = self.semantics.get();
        search::run(search::Params {
            store: &self.store,
            index: &index,
            semantic: semantic.as_ref().map(|loaded| search::Semantic {
                embedder: loaded.embedder.as_ref(),
                vectors: loaded.vectors.as_ref(),
            }),
            requested_mode: params.mode,
            raw_query: &params.query,
            limit: params.limit.unwrap_or(self.config.search.default_limit),
            // Typing is the default assumption: results should appear while a word is still
            // half written. A caller that wants exact matching says so.
            prefix: params.prefix.unwrap_or(true),
            candidates_per_arm: self.config.search.candidates_per_arm,
            rrf_k: self.config.search.rrf_k,
            max_salience_boost: self.config.search.max_salience_boost,
        })
    }

    fn model_status(&self) -> Result<ModelStatusResult, RpcError> {
        use std::sync::atomic::Ordering;
        let status = self.semantics.status();
        let stats = &self.embed_stats;

        let chunks_total = self.store.extract_progress().map_err(internal)?.chunks;
        let chunks_embedded = match self.semantics.get() {
            Some(loaded) => {
                let model = loaded.embedder.model();
                self.store
                    .embedding_count(cortex_store::embeddings::ModelRef {
                        id: &model.id,
                        version: &model.version,
                        dim: model.dim,
                    })
                    .map_err(internal)?
            }
            None => 0,
        };

        Ok(ModelStatusResult {
            enabled: status.enabled,
            model: status.model,
            loaded: status.loaded,
            backend: status.backend,
            dim: status.dim,
            note: status.note,
            model_present: status.model_present,
            runtime_present: status.runtime_present,
            model_dir: status.model_dir,
            runtime_path: status.runtime_path,
            download_bytes: cortex_embed::registry::model_by_id(&self.config.embedding.model)
                .map(|m| m.total_bytes())
                .unwrap_or(0),
            vectors_in_index: status.vectors,
            chunks_total,
            chunks_embedded,
            chunks_pending: (chunks_total - chunks_embedded).max(0),
            caught_up: stats.caught_up.load(Ordering::Relaxed),
            embedded_this_run: stats.chunks_embedded.load(Ordering::Relaxed),
            stale: stats.chunks_stale.load(Ordering::Relaxed),
            failures: stats.failures.load(Ordering::Relaxed),
            busy_ms: stats.busy_us.load(Ordering::Relaxed) / 1000,
        })
    }

    fn extract_status(&self) -> Result<ExtractStatusResult, RpcError> {
        let progress = self.store.extract_progress().map_err(internal)?;
        let idle = self.activity.idle_for();
        let active = idle < std::time::Duration::from_secs(self.config.resources.idle_after_secs);
        let stats = &self.extract_stats;
        use std::sync::atomic::Ordering;

        Ok(ExtractStatusResult {
            enabled: !self.config.index.content_roots.is_empty(),
            content_roots: self
                .config
                .index
                .content_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            pending: progress.pending,
            done: progress.done,
            skipped: progress.skipped,
            failed: progress.failed,
            chunks: progress.chunks,
            files_extracted: stats.files_extracted.load(Ordering::Relaxed),
            files_unchanged: stats.files_unchanged.load(Ordering::Relaxed),
            chunks_written: stats.chunks_written.load(Ordering::Relaxed),
            chunks_reused: stats.chunks_reused.load(Ordering::Relaxed),
            chunks_deleted: stats.chunks_deleted.load(Ordering::Relaxed),
            busy_ms: stats.busy_us.load(Ordering::Relaxed) / 1000,
            idle_secs: idle.as_secs(),
            user_active: active,
            cpu_share: throttle::share_for(
                active,
                self.config.resources.cpu_share_active,
                self.config.resources.cpu_share_idle,
            ),
            paused: self.ingest.is_paused(),
        })
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
            let started = Instant::now();

            // A search arriving over the socket is evidence that somebody typed it. That is the
            // only activity signal a headless daemon can trust, and it is a good one.
            daemon.activity.ping();

            let outcome = daemon.run_search(&params);

            // Searching is itself an episode. Without this, "what was I looking for on
            // Tuesday" has nothing to answer from.
            let _ = daemon.store.append_episode(
                indexer::unix_now(),
                &Actor::Human,
                Action::Search,
                None,
                None,
                &serde_json::json!({
                    "query": outcome.query,
                    "mode": outcome.mode,
                    "hits": outcome.hits.len(),
                }),
            );

            json(SearchResult {
                hits: outcome.hits,
                mode: outcome.mode,
                elapsed_ms: started.elapsed().as_millis() as u64,
                // The cross-encoder is the only thing that would arrive later, and it belongs
                // to M2's second half.
                partial: false,
            })
        }

        method::SEARCH_EXPLAIN => {
            let params: SearchExplainParams = parse(params)?;
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

            // Explaining runs the same search rather than a reconstruction of it. A second
            // implementation would eventually explain something the search does not do.
            let outcome = daemon.run_search(&SearchParams {
                query: params.query.clone(),
                mode: params.mode,
                // The whole list is needed: a node ranked fortieth still has a rank, and
                // "not in the top ten" is not an explanation.
                limit: Some(daemon.config.search.candidates_per_arm),
                prefix: params.prefix,
            });

            let (rank, score) = match &node {
                Some(node) => search::explain(&outcome, node.id),
                None => (None, None),
            };

            let chunks = match &node {
                Some(node) => daemon
                    .store
                    .chunks_for_node(node.id)
                    .map_err(internal)?
                    .len(),
                None => 0,
            };
            let extract = match params.path.as_deref() {
                Some(path) => daemon
                    .store
                    .extract_note(Path::new(path))
                    .map_err(internal)?,
                None => match &node {
                    Some(node) => daemon
                        .store
                        .extract_note(&node.path)
                        .map_err(internal)?,
                    None => None,
                },
            };

            let reasons = explain_reasons(
                &outcome,
                node.as_ref(),
                rank,
                chunks,
                extract.as_ref().map(|(state, note)| (*state, note.clone())),
            );

            json(SearchExplainResult {
                query: outcome.query.clone(),
                mode: outcome.mode,
                expression: outcome.expression.clone(),
                node,
                rank,
                score,
                path_candidates: outcome.path_candidates,
                lexical_candidates: outcome.lexical_candidates,
                vector_candidates: outcome.vector_candidates,
                vector_note: outcome.vector_note.clone(),
                extract_state: extract.as_ref().map(|(state, _)| state.as_str().to_string()),
                extract_note: extract.and_then(|(_, note)| note),
                chunks,
                reasons,
            })
        }

        method::EXTRACT_STATUS => json(daemon.extract_status()?),

        method::MODEL_STATUS => json(daemon.model_status()?),

        method::MODEL_RELOAD => {
            // Called after `cortex model fetch`. Loading is cheap enough to do inline — it is
            // opening a file and building a session, not inference — and doing it here means a
            // freshly downloaded model works without restarting the daemon.
            let outcome = daemon.semantics.load();
            // The embedding stage waits on a long timer when it has no model; wake it so the
            // corpus starts being embedded now rather than in a minute.
            daemon.ingest.signal_work();
            match outcome {
                Ok(backend) => json(ModelReloadResult {
                    loaded: true,
                    backend: Some(backend),
                    note: None,
                }),
                Err(note) => json(ModelReloadResult {
                    loaded: false,
                    backend: None,
                    note: Some(note),
                }),
            }
        }

        method::EXTRACT_REQUEUE => {
            let requeued = daemon.store.requeue_extraction().map_err(internal)?;
            // The stage may be in its long idle sleep; it changed nothing on the filesystem, so
            // nothing else would ever tell it.
            daemon.ingest.signal_work();
            json(ExtractRequeueResult { requeued })
        }

        method::ACTIVITY_PING => {
            daemon.activity.ping();
            json(ActivityPingResult {
                idle_secs: daemon.activity.idle_for().as_secs(),
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

/// Turn a search outcome into sentences a person can act on.
///
/// The ordering matters: the first thing said should be the thing that is wrong. "Not indexed"
/// makes every other observation irrelevant, and "extraction skipped it" explains an empty
/// content arm better than any score ever could.
fn explain_reasons(
    outcome: &search::Outcome,
    node: Option<&cortex_core::model::Node>,
    rank: Option<u32>,
    chunks: usize,
    extract: Option<(cortex_core::model::ExtractState, Option<String>)>,
) -> Vec<String> {
    use cortex_core::model::ExtractState;

    let mut reasons = Vec::new();

    let Some(node) = node else {
        reasons.push("this path is not in the index at all".to_string());
        return reasons;
    };

    match rank {
        Some(rank) => reasons.push(format!("ranked {rank} for this query")),
        None => reasons.push("did not appear in the results for this query".to_string()),
    }

    if outcome.mode == QueryMode::Locate {
        reasons.push(
            "the query was answered from paths alone; prefix it with `recall:` to search content"
                .to_string(),
        );
    } else if outcome.expression.is_none() {
        reasons.push("the query had no searchable words, so the content arm did not run".to_string());
    }

    if let Some(note) = &outcome.vector_note {
        reasons.push(format!("semantic search did not run: {note}"));
    }

    // A directory is never a candidate for extraction, so its state stays `pending` for ever.
    // Reporting that as "not reached yet" would send somebody looking for a stalled queue.
    if node.kind == cortex_core::model::NodeKind::Dir {
        reasons.push("a directory has no content of its own; only its name is searchable".to_string());
        return reasons;
    }

    match extract {
        Some((ExtractState::Pending, _)) => {
            reasons.push("content extraction has not reached this file yet".to_string())
        }
        Some((ExtractState::Skipped, note)) => reasons.push(format!(
            "content was not read: {}",
            note.unwrap_or_else(|| "no reason recorded".into())
        )),
        Some((ExtractState::Failed, note)) => reasons.push(format!(
            "content extraction failed: {}",
            note.unwrap_or_else(|| "no reason recorded".into())
        )),
        Some((ExtractState::Done, note)) => {
            if chunks == 0 {
                reasons.push("extracted, but produced no chunks — the file has no text".to_string());
            }
            if let Some(note) = note {
                reasons.push(note);
            }
        }
        None => {}
    }

    if chunks > 0 {
        reasons.push(format!("{chunks} chunk(s) in the lexical index"));
    }
    if node.salience == 0.0 {
        reasons.push("salience is zero, so no boost was applied (the salience engine is M5)".to_string());
    }

    reasons
}
