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
use cortex_agent::fs_tools::{Surface, ToolError};
use cortex_agent::{Auditor, KillSwitch, PlanBook, Policy};
use cortex_ingest::throttle;
use cortex_query::search;
use cortex_query::PathIndex;
use cortex_rpc::protocol::SearchUpdateParams;
use cortex_rpc::protocol::{SyncNowResult, SyncReplayResult, SyncStatusResult};
use cortex_rpc::protocol::{
    method, ActivityPingResult, AgentSetEnabledParams, AgentStatusResult, AuditRecentParams,
    RecallParams, RecallResult, RecalledAssociation,
    AuditRecentResult, IdentifyParams, IdentifyResult, IndexSetPausedParams, IndexSetPausedResult, EpisodeAppendParams, EpisodesRecentParams, EpisodesRecentResult,
    CanonicalDumpResult, ExtractRequeueResult, ExtractStatusResult, IndexPathParams,
    IndexPathResult, ModelReloadResult,
    ModelStatusResult, NodeGetParams, NodeGetResult, PingResult, ReconcileParams, ReconcileResult,
    ReconcileSummary, Request, Response, RpcError, SearchExplainParams, SearchExplainResult,
    RebuildParams, RebuildResult, SearchParams, SearchResult, StatusResult, WatchStatusResult,
    ZoneInfo, ZoneSetColorParams, ZonesListResult,
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
    /// What agents may do. Derived from configuration once, at startup — a policy that can be
    /// changed by a message is a policy an agent can argue with.
    pub policy: Policy,
    /// The kill switch (R7). Runtime state, persisted, and the only part of the policy a
    /// running daemon can change.
    pub kill: Arc<KillSwitch>,
    /// Transfers currently running, so they can be cancelled by id.
    pub operations: crate::operations::Operations,
    /// Dry-run plans awaiting confirmation. In memory: a plan that outlived a restart would
    /// describe a filesystem nobody has looked at since.
    pub plans: PlanBook,
    /// The XDG trash. Everything this daemon deletes lands where the rest of the desktop
    /// looks for it, which is what makes it safe to stop using another file manager (D45).
    pub trash: cortex_agent::trash::Trash,
    pub database_path: PathBuf,
    pub started: Instant,
    /// Signalled when a client calls `shutdown`.
    pub shutdown: Arc<Notify>,
    /// The federation worker's handle (M7). Always present; usually off.
    pub federation: Arc<crate::sync_stage::Federation>,
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
        kill: Arc<KillSwitch>,
        trash: cortex_agent::trash::Trash,
        database_path: PathBuf,
        federation: Arc<crate::sync_stage::Federation>,
    ) -> Self {
        let policy = Policy::new(&config.agent, config.index.extra_deny.clone());
        Self {
            store,
            index,
            ingest,
            activity,
            extract_stats,
            embed_stats,
            semantics,
            config,
            policy,
            kill,
            operations: crate::operations::Operations::new(),
            plans: PlanBook::default(),
            trash,
            database_path,
            federation,
            started: Instant::now(),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Is the Agent Surface serving? Configuration and the kill switch must both say yes.
    pub fn agent_surface_enabled(&self) -> bool {
        self.policy.configured_enabled() && self.kill.agent_surface_enabled()
    }

    fn agent_status(&self) -> cortex_store::error::Result<AgentStatusResult> {
        Ok(AgentStatusResult {
            enabled: self.agent_surface_enabled(),
            configured_enabled: self.policy.configured_enabled(),
            indexing_paused: self.kill.indexing_paused(),
            require_dry_run: self.policy.require_dry_run(),
            write_allowlist: self
                .policy
                .write_allowlist()
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            undo_depth: self.store.undo_depth()?,
            audit_entries: self.store.audit_count()?,
        })
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
            runtime_id: status.runtime_id,
            runtime_is_cuda: status.runtime_is_cuda,
            model_dir: status.model_dir,
            runtime_path: status.runtime_path,
            download_bytes: cortex_embed::registry::model_by_id(&self.config.embedding.model)
                .map(|m| m.total_bytes())
                .unwrap_or(0),
            vectors_in_index: status.vectors,
            memory_vectors_in_index: status.memory_vectors,
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

/// Send a response, and if it cannot be sent because it is too large, send an error saying so.
///
/// The transport is newline-delimited JSON with a line limit. A reply over that limit used to be
/// dropped — the concurrent path discarded the write error, so the caller waited for an answer
/// that was never coming. Both processes sat at zero per cent CPU, which is what a deadlock
/// looks like and is a considerably worse failure than an error. `brain.graph` on a large index
/// is the request that found it.
async fn respond<W>(
    writer: &Arc<tokio::sync::Mutex<W>>,
    id: serde_json::Value,
    response: Response,
) -> Result<(), transport::TransportError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let value = serde_json::to_value(&response).expect("a response serialises");
    let mut guard = writer.lock().await;
    match transport::write_message(&mut *guard, &value).await {
        Err(transport::TransportError::LineTooLong) => {
            let bytes = serde_json::to_vec(&value).map(|v| v.len()).unwrap_or(0);
            let fallback = Response::err(
                id,
                RpcError::new(
                    RpcError::RESULT_TOO_LARGE,
                    format!(
                        "the answer is {bytes} bytes and the protocol's line limit is {} — \
                         ask for less, with a smaller `limit`",
                        transport::MAX_LINE_BYTES
                    ),
                ),
            );
            transport::write_message(
                &mut *guard,
                &serde_json::to_value(&fallback).expect("an error serialises"),
            )
            .await
        }
        other => other,
    }
}

async fn handle_connection(
    stream: UnixStream,
    daemon: Arc<Daemon>,
) -> Result<(), transport::TransportError> {
    // Split, so that a task spawned to finish a search can write its answer while this loop is
    // still reading the next request. Without the split, a reranked result could only be sent by
    // blocking the connection until it was ready — which is precisely what "first results are
    // never blocked" (D7) is supposed to prevent.
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let writer = Arc::new(tokio::sync::Mutex::new(writer));

    // Who is on the other end. Connection-scoped rather than per-request: an agent names
    // itself once, and every episode and audit entry after that carries the name. A connection
    // that never says anything is a human at a terminal, which is the safe assumption because
    // it is the one that grants nothing.
    /// How many reads one connection may have in flight.
    ///
    /// Generous for what a window does — a listing, a preview and the brain's neighbours for
    /// one row is three — and small enough that a client cannot spend the daemon's memory by
    /// asking faster than it can answer.
    const CONCURRENT_READS: usize = 8;
    let concurrency = Arc::new(tokio::sync::Semaphore::new(CONCURRENT_READS));

    let notifier_writer = writer.clone();
    let mut ctx = ConnCtx {
        session: None,
        actor: cortex_core::model::Actor::Human,
        notify: Arc::new(move |method: &str, params: serde_json::Value| {
            let message = serde_json::json!({
                "jsonrpc": "2.0", "method": method, "params": params
            });
            let writer = notifier_writer.clone();
            // Spawned rather than awaited: this is called from a blocking transfer thread,
            // which has no runtime to await on and must not stop copying to send a message.
            tokio::spawn(async move {
                let mut guard = writer.lock().await;
                let _ = transport::write_message(&mut *guard, &message).await;
            });
        }),
    };

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
                let mut guard = writer.lock().await;
                let _ = transport::write_message(&mut *guard, &serde_json::to_value(&response).unwrap())
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
                let mut guard = writer.lock().await;
                transport::write_message(&mut *guard, &serde_json::to_value(&response).unwrap())
                    .await?;
                continue;
            }
        };

        let id = request.id.clone();
        let method = request.method.clone();

        // A read is answered on its own task, so that a slow one does not hold up the ones
        // behind it.
        //
        // This loop used to await every request in turn, and the journal shows what that did to
        // a person scrolling a result list: a `search`, a `brain.neighbors` and an `fs.read` all
        // reporting **152 ms** and all finishing within 0.4 ms of each other. None of them was
        // slow. They were in a queue, and the queue was this line.
        //
        // Only the methods in `is_concurrent` — an allow-list, so anything new is serialised
        // until somebody has thought about it. Writes keep the connection's order because their
        // order means something: an undo undoes the previous thing.
        //
        // `ctx` is cloned rather than shared. The only thing that mutates it is `identify`,
        // which is not on the list and therefore still runs here, in order, before anything a
        // clone could observe.
        if id.is_some() && method::is_concurrent(&method) {
            let daemon = daemon.clone();
            let writer = writer.clone();
            let mut ctx = ctx.clone();
            let params = request.params;
            let method = method.clone();
            let id = id.clone();

            // Bounded, so that a client cannot turn a burst of requests into an unbounded pile
            // of tasks. Waiting here is the same back-pressure the old loop had, applied at a
            // depth where it stops being visible.
            let permit = concurrency.clone().acquire_owned().await;

            tokio::spawn(async move {
                let mut refinable: Option<(search::Outcome, Refinement)> = None;
                let outcome = dispatch(&daemon, &method, params, &mut refinable, &mut ctx).await;
                let Some(id) = id else { return };

                let response = match outcome {
                    Ok(result) => Response::ok(id.clone(), result),
                    Err(error) => Response::err(id.clone(), error),
                };
                let _ = respond(&writer, serde_json::to_value(&id).unwrap_or_default(), response)
                    .await;

                if let Some((outcome, shape)) = refinable {
                    spawn_rerank(daemon.clone(), writer.clone(), id, outcome, shape);
                }
                drop(permit);
            });
            continue;
        }

// A search keeps its outcome here so the reranker can reuse it. Re-running the search in
        // the background task would be simpler and would double every query's retrieval work.
        let mut refinable: Option<(search::Outcome, Refinement)> = None;
        let outcome = dispatch(&daemon, &method, request.params, &mut refinable, &mut ctx).await;

        // A request without an id is a notification: the caller expects no answer, and sending
        // one would desynchronise its stream.
        let Some(id) = id else { continue };

        let response = match outcome {
            Ok(result) => Response::ok(id.clone(), result),
            Err(error) => Response::err(id.clone(), error),
        };
        respond(&writer, serde_json::to_value(&id).unwrap_or_default(), response).await?;

        // The answer is out. Anything better than it arrives separately.
        if let Some((outcome, shape)) = refinable {
            spawn_rerank(daemon.clone(), writer.clone(), id, outcome, shape);
        }
    }
}

/// Rerank a search that has already been answered, and send the better order as a notification.
///
/// Everything about this is deliberately after the fact: the reranker is a second model, it reads
/// each candidate together with the query, and it takes long enough that waiting for it would
/// undo the point of having fast retrieval at all (D7).
/// Which shape a reordering is sent in.
///
/// The same reranker serves `search` and `view.open`, and their clients read different things:
/// one wants hits, the other wants rows and should never have to know what a hit is. So the
/// answer's shape is decided where the request was, and carried here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refinement {
    Search,
    View,
}

fn spawn_rerank(
    daemon: Arc<Daemon>,
    writer: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<UnixStream>>>,
    id: serde_json::Value,
    outcome: search::Outcome,
    shape: Refinement,
) {
    let Some(reranker) = daemon.semantics.reranker() else {
        return;
    };

    tokio::spawn(async move {
        let started = Instant::now();
        let top_n = daemon.config.search.rerank_top_n;

        // Inference is blocking and long by this crate's standards; it does not belong on the
        // runtime that serves every other client.
        let reranked = tokio::task::spawn_blocking(move || {
            let model = reranker.model().id.clone();
            match search::rerank(&outcome, reranker.as_ref(), top_n, 200) {
                Ok(hits) => Some((hits, model)),
                Err(e) => {
                    tracing::warn!(error = %e, "reranking failed; the fused order stands");
                    None
                }
            }
        })
        .await;

        let Ok(Some((hits, model))) = reranked else {
            return;
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let notification = match shape {
            Refinement::Search => serde_json::json!({
                "jsonrpc": "2.0",
                "method": method::SEARCH_UPDATE,
                "params": SearchUpdateParams {
                    request_id: id,
                    hits,
                    elapsed_ms,
                    model,
                },
            }),
            Refinement::View => serde_json::json!({
                "jsonrpc": "2.0",
                "method": method::VIEW_UPDATE,
                "params": cortex_core::view::ViewUpdate {
                    request_id: id.as_i64().unwrap_or(0),
                    rows: hits.into_iter().map(cortex_core::view::ViewRow::from_hit).collect(),
                    elapsed_ms,
                    model: Some(model),
                },
            }),
        };

        let mut guard = writer.lock().await;
        // The client may have gone; that is not an error worth logging loudly.
        let _ = transport::write_message(&mut *guard, &notification).await;
    });
}

/// State that belongs to one connection rather than to the daemon.
#[derive(Clone)]
pub struct ConnCtx {
    pub actor: cortex_core::model::Actor,
    /// The agent session this connection is working under, once it has opened one.
    ///
    /// Set by `agent.session.begin` and by `identify`, so that everything after it is
    /// attributable to *this task* rather than to the agent's name in general — which is the
    /// whole reason sessions exist (M11 §7). Without it `agent_sessions.calls` stays at zero
    /// for every session ever opened, which is what it did until somebody looked at the panel
    /// that prints it.
    pub session: Option<String>,
    /// How a spawned operation reaches the client that started it.
    ///
    /// A transfer's progress belongs to the connection that asked for it, not to every
    /// connection: two file manager windows copying different things must not each see the
    /// other's bar.
    pub notify: crate::operations::Notifier,
}

async fn dispatch(
    daemon: &Arc<Daemon>,
    method: &str,
    params: Option<serde_json::Value>,
    refinable: &mut Option<(search::Outcome, Refinement)>,
    ctx: &mut ConnCtx,
) -> Result<serde_json::Value, RpcError> {
    let answer = dispatch_inner(daemon, method, params, refinable, ctx).await;

    // Count it against the session, if this connection is working under one. Here rather than
    // in each handler for the obvious reason and one less obvious: a refusal never reaches a
    // handler at all, and a refusal is the interesting half of the number.
    //
    // A counter that is only updated sometimes is a counter nobody can reason about, and this
    // one was updated never — `note_session_call` existed, was tested, and was called by
    // nothing, so every session in the Inspector reported zero calls for ever.
    if let Some(session) = ctx.session.as_deref() {
        if let Err(e) = daemon.store.note_session_call(session, answer.is_ok()) {
            tracing::warn!(error = %e, "could not count a call against its session");
        }
    }
    answer
}

async fn dispatch_inner(
    daemon: &Arc<Daemon>,
    method: &str,
    params: Option<serde_json::Value>,
    refinable: &mut Option<(search::Outcome, Refinement)>,
    ctx: &mut ConnCtx,
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
                agent_surface_enabled: daemon.agent_surface_enabled(),
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

            let refining = params.rerank.unwrap_or(daemon.config.search.rerank)
                && daemon.semantics.reranker().is_some()
                && !outcome.hits.is_empty();

            let result = SearchResult {
                hits: outcome.hits.clone(),
                mode: outcome.mode,
                elapsed_ms: started.elapsed().as_millis() as u64,
                // True means a better order is on its way as a `search.update` notification.
                // A client that does not wait for it still has a complete answer.
                partial: refining,
            };
            if refining {
                *refinable = Some((outcome, Refinement::Search));
            }
            json(result)
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
                // Explaining shows the ranking the search produced, not one a second model
                // would produce afterwards.
                rerank: Some(false),
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

            // A memory is a node, so it arrives here like any other — and explaining it as a
            // file that failed to rank would be true and useless.
            let memory = match &node {
                Some(node) if node.kind == cortex_core::model::NodeKind::Memory => {
                    daemon.store.memory(node.id).map_err(internal)?
                }
                _ => None,
            };

            let reasons = match &memory {
                Some(memory) => explain_memory(memory),
                None => explain_reasons(
                    &outcome,
                    node.as_ref(),
                    rank,
                    chunks,
                    extract.as_ref().map(|(state, note)| (*state, note.clone())),
                ),
            };

            json(SearchExplainResult {
                memory,
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

        method::CANONICAL_DUMP => {
            let dump = daemon.store.canonical_dump().map_err(internal)?;
            json(CanonicalDumpResult {
                lines: dump.lines().count(),
                dump,
            })
        }

        method::REBUILD_INDEX => {
            let params: RebuildParams = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(RebuildParams { drop_embeddings: false });

            let started = Instant::now();
            let roots = daemon.config.index.metadata_roots.clone();
            if roots.is_empty() {
                return Err(RpcError::invalid_params(
                    "no metadata_roots configured; there would be nothing to rebuild from",
                ));
            }

            // Paused for the whole operation. The alternative is the watcher inserting rows into
            // a table that is being emptied, which is a race with no upside.
            let was_paused = daemon.ingest.is_paused();
            daemon.ingest.set_paused(true);

            let store = daemon.store.clone();
            let extra_deny = daemon.config.index.extra_deny.clone();
            let drop_embeddings = params.drop_embeddings;
            let outcome = tokio::task::spawn_blocking(move || {
                store.clear_derived(!drop_embeddings)?;
                let mut indexed = 0u64;
                for root in &roots {
                    let report = cortex_ingest::reconcile::reconcile(
                        &store,
                        root,
                        &extra_deny,
                        indexer::unix_now(),
                    );
                    indexed += report.added;
                }
                store.link_parents()?;
                store.assign_zones()?;
                // Explicit assertions — annotations, links, pins — are not derivable from the
                // filesystem, so a rebuild that stopped here would silently drop everything an
                // agent ever recorded. They are replayed from the episode log, which is the
                // only table a rebuild does not touch (R10).
                let (replayed, skipped) = store.replay_assertions()?;
                // And the memory layer, which is the larger half of what an agent ever
                // recorded and the half with columns to lose. Replayed after the annotations
                // because a memory may name a file as its subject, and the node has to exist
                // for the edge to be written.
                let (memories, memories_skipped) = store.replay_memories()?;
                let (replayed, skipped) = (replayed + memories, skipped + memories_skipped);
                if skipped > 0 {
                    tracing::warn!(
                        replayed,
                        skipped,
                        "some assertions name files that no longer exist and were dropped"
                    );
                }
                tracing::info!(memories, "memory layer replayed from the episode log");
                store.prune_orphan_annotations()?;
                cortex_store::error::Result::Ok((indexed, replayed, skipped))
            })
            .await
            .map_err(|e| RpcError::internal(format!("rebuild task failed: {e}")))?
            .map_err(internal)?;
            let (outcome, assertions_replayed, assertions_dropped) = outcome;

            rebuild_index(daemon).map_err(internal)?;

            // The vector index is *not* refilled here, and the first attempt at this was the bug
            // that made the point: at this moment the chunks do not exist yet, so there is
            // nothing to point at and the refill silently wrote an empty index. The embedding
            // stage does it once extraction has caught up, where the chunks are back.
            let embeddings_kept = daemon.semantics.get().map_or(0, |loaded| {
                let model = loaded.embedder.model();
                daemon
                    .store
                    .embedding_count(cortex_store::embeddings::ModelRef {
                        id: &model.id,
                        version: &model.version,
                        dim: model.dim,
                    })
                    .unwrap_or(0)
            });

            daemon.ingest.set_paused(was_paused);
            daemon.ingest.signal_work();
            let progress = daemon.store.extract_progress().map_err(internal)?;

            json(RebuildResult {
                indexed: outcome,
                assertions_replayed,
                assertions_dropped,
                extraction_pending: progress.pending,
                embeddings_kept,
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }

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

        // Why this row is where it is, and what has happened to it (M10 step 4).
        //
        // One call for the sidebar's two questions. They are asked about the same file at the
        // same moment — a person looks at a row and wants to know why it is ranked where it is
        // *and* when they last touched it — and two round trips for one glance is the pattern
        // the context panel was already paying for annotations and memories separately.
        method::NODE_WHY => {
            let params: NodeGetParams = parse(params)?;
            let path = params
                .path
                .as_deref()
                .ok_or_else(|| RpcError::invalid_params("`path` is required"))?;
            let node = daemon
                .store
                .node_by_path(Path::new(path))
                .map_err(internal)?
                .ok_or_else(|| {
                    RpcError::invalid_params(&format!(
                        "{path} is not in the index, so there is nothing to explain yet"
                    ))
                })?;

            // The components the consolidation pass computed and stored. `None` means no pass
            // has scored this node yet, which is a real answer — a fresh index has no
            // judgements in it — and is said rather than shown as a row of zeroes.
            let scored = daemon
                .store
                .salience_components(node.id)
                .map_err(internal)?;
            let episodes = daemon
                .store
                .episodes_for(node.id, 20)
                .map_err(internal)?;
            let columns = cortex_store::memory_columns::for_path(&daemon.store, Path::new(path))
                .map_err(internal)?;
            // The notes somebody attached to the file. Here rather than through
            // `brain.neighbors`, which the panel used to call for them and which answers a
            // different question — the graph around a node is a *place* now (M10 §4.5).
            let annotations = daemon
                .store
                .annotations_for(node.id)
                .map_err(internal)?;

            json(serde_json::json!({
                "path": path,
                "annotations": annotations,
                "node_id": node.id.0,
                "tier": node.tier.as_str(),
                "salience": node.salience,
                "pinned": node.pinned,
                "degree": columns.as_ref().map(|c| c.degree),
                "last_episode": columns.as_ref().and_then(|c| c.last_episode),
                "computed_at": scored.as_ref().map(|(at, _)| *at),
                "components": scored.as_ref().map(|(_, c)| c),
                "episodes": episodes,
            }))
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

        method::AGENT_STATUS => json(daemon.agent_status().map_err(internal)?),

        method::AGENT_SET_ENABLED => {
            let params: AgentSetEnabledParams = parse(params)?;
            // Configuration wins: turning the surface back on over RPC when the config file
            // switched it off would make the file a suggestion (D32).
            if params.enabled && !daemon.config.agent.enabled {
                return Err(RpcError::new(
                    RpcError::AGENT_SURFACE_DISABLED,
                    "the Agent Surface is disabled in the configuration file; change \
                     agent.enabled there and restart",
                ));
            }
            daemon.kill.set_agent_surface(params.enabled).map_err(internal)?;
            tracing::warn!(enabled = params.enabled, "agent surface switched");
            json(daemon.agent_status().map_err(internal)?)
        }

        method::INDEX_SET_PAUSED => {
            let params: IndexSetPausedParams = parse(params)?;
            // Both, and in this order: the persisted record first, so a crash between the two
            // leaves a daemon that comes back paused rather than one that comes back indexing.
            daemon.kill.set_indexing_paused(params.paused).map_err(internal)?;
            daemon.ingest.set_paused(params.paused);
            if !params.paused {
                daemon.ingest.signal_work();
            }
            tracing::warn!(paused = params.paused, "indexing switched");
            json(IndexSetPausedResult { paused: params.paused })
        }

        method::AUDIT_RECENT => {
            let params: AuditRecentParams = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(AuditRecentParams {
                    limit: None,
                    actor: None,
                    refused_only: false,
                    after: None,
                });
            let entries = daemon
                .store
                .recent_audit(
                    params.limit.unwrap_or(50).min(1000),
                    params.actor.as_deref(),
                    params.refused_only,
                    params.after,
                )
                .map_err(internal)?;
            json(AuditRecentResult {
                entries,
                total: daemon.store.audit_count().map_err(internal)?,
            })
        }

        method::IDENTIFY => {
            let params: IdentifyParams = parse(params)?;
            ctx.actor = cortex_agent::actor_from_client(Some(&params.actor));
            if let Some(session) = params.session.as_deref() {
                ctx.session = Some(session.to_string());
            }
            tracing::info!(actor = %ctx.actor.as_str(), "connection identified");
            json(IdentifyResult {
                actor: ctx.actor.as_str(),
                agent_surface_enabled: daemon.agent_surface_enabled(),
            })
        }

        // The agent tools. Every one of them is the same three lines — build the surface for
        // this actor, call the one implementation, translate its refusal — because the tool
        // logic lives in `cortex-agent` and this is only the socket's way in to it. The CLI
        // and the MCP server reach the same code through the same methods.
        method::FS_PLACES => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let mut answer = surface(daemon, &ctx.actor, &auditor)
                .places(&params)
                .map_err(tool_error)?;

            // The places the index made, beside the ones the filesystem has (M10 §4.3). Built
            // here rather than in `cortex_agent::places`, which reads files and knows nothing
            // about a store — and a concept is a row exactly like a folder is: a name, and
            // somewhere Enter takes you.
            let mut memory_places = vec![
                cortex_core::tools::Place {
                    path: std::path::PathBuf::new(),
                    name: "Woran ich gerade arbeite".to_string(),
                    kind: cortex_core::tools::PlaceKind::View,
                    capacity: None,
                    device: None,
                    volume: None,
                    source: Some(cortex_core::view::Source::WorkingSet),
                },
            ];

            // The concepts the last consolidation pass named, biggest first. Only the ones with
            // members: an empty concept is a row that opens onto nothing.
            if let Ok(concepts) = cortex_store::memory_columns::concepts(&daemon.store, 8) {
                for (id, label, members) in concepts {
                    if members == 0 {
                        continue;
                    }
                    memory_places.push(cortex_core::tools::Place {
                        path: std::path::PathBuf::new(),
                        name: label.clone(),
                        kind: cortex_core::tools::PlaceKind::View,
                        capacity: None,
                        // What the sidebar shows under the name: how many files are in it, so
                        // nobody has to open three concepts to find the one worth opening.
                        device: Some(format!("{members} Dateien")),
                        volume: None,
                        source: Some(cortex_core::view::Source::Concept { id: id.0, label }),
                    });
                }
            }

            // And the tier that is a subject rather than a brightness. Offered only when there
            // is something in it: a row called "Vergessen" that opens onto an empty list would
            // teach somebody that the word means nothing.
            if cortex_store::memory_columns::tier_total(
                &daemon.store,
                cortex_core::model::Tier::Cold,
            )
            .unwrap_or(0)
                > 0
            {
                memory_places.push(cortex_core::tools::Place {
                    path: std::path::PathBuf::new(),
                    name: "Vergessen".to_string(),
                    kind: cortex_core::tools::PlaceKind::View,
                    capacity: None,
                    device: Some("nur noch Metadaten".to_string()),
                    volume: None,
                    source: Some(cortex_core::view::Source::Tier {
                        tier: "cold".to_string(),
                    }),
                });
            }

            // Before "System", which is where everything technical lives: these are places
            // somebody goes to, and the filesystem root is the one they go to last.
            let at = answer
                .sections
                .iter()
                .position(|s| s.title == "System")
                .unwrap_or(answer.sections.len());
            answer.sections.insert(
                at,
                cortex_core::tools::PlaceSection {
                    title: "Gedächtnis".to_string(),
                    places: memory_places,
                },
            );
            json(answer)
        }

        method::FS_LIST => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).list(&params).map_err(tool_error)?)
        }

        method::APPS_FOR_PATH => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).apps_for_path(&params).map_err(tool_error)?)
        }

        method::FS_MOUNT => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).mount(&params).map_err(tool_error)?)
        }

        method::FS_UNMOUNT => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).unmount(&params).map_err(tool_error)?)
        }

        method::APPS_SET_DEFAULT => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .apps_set_default(&params)
                    .map_err(tool_error)?,
            )
        }

        method::FS_READ => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).read(&params).map_err(tool_error)?)
        }

        method::FS_STAT => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).stat(&params).map_err(tool_error)?)
        }

        method::FS_WRITE => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).write(&params).map_err(tool_error)?)
        }

        method::FS_MOVE => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let result = surface(daemon, &ctx.actor, &auditor).move_path(&params).map_err(tool_error)?;
            // A move changes paths the in-memory index answers from, and it changed them
            // outside the watcher's knowledge if the target is not under a watched root.
            rebuild_index(daemon).map_err(internal)?;
            json(result)
        }

        method::FS_TRASH => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let result = surface(daemon, &ctx.actor, &auditor).trash(&params).map_err(tool_error)?;
            rebuild_index(daemon).map_err(internal)?;
            json(result)
        }

        method::UNDO => {
            let params = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(cortex_core::tools::UndoParams {
                    last: None,
                    plan: None,
                    dry_run: false,
                });
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let result = surface(daemon, &ctx.actor, &auditor).undo(&params).map_err(tool_error)?;
            if !result.undone.is_empty() {
                // Undo puts files back under paths the index stopped believing in.
                daemon.ingest.signal_work();
            }
            json(result)
        }

        // Memory. Same shape as the fs tools: build the surface for this actor, call the one
        // implementation in `cortex-agent`, translate the refusal.
        method::BRAIN_ANNOTATE => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).annotate(&params).map_err(tool_error)?)
        }

        method::BRAIN_PIN | method::BRAIN_FORGET => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let pinned = method == method::BRAIN_PIN;
            json(surface(daemon, &ctx.actor, &auditor).pin(&params, pinned).map_err(tool_error)?)
        }

        method::BRAIN_LINK => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).link(&params).map_err(tool_error)?)
        }

        method::BRAIN_NEIGHBORS => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).neighbors(&params).map_err(tool_error)?)
        }

        method::BRAIN_TIMELINE => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).timeline(&params).map_err(tool_error)?)
        }

        method::BRAIN_WORKING_SET => {
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let idle = Some(daemon.activity.idle_for().as_secs());
            json(surface(daemon, &ctx.actor, &auditor).working_set(idle).map_err(tool_error)?)
        }

        method::BRAIN_GRAPH => {
            let params = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or_default();
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).graph(&params).map_err(tool_error)?)
        }

        method::BRAIN_CONSOLIDATE => {
            let params = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or_default();
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).consolidate(&params).map_err(tool_error)?)
        }

        method::BRAIN_REPORT => {
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).report().map_err(tool_error)?)
        }

        method::BRAIN_DIFF => {
            let params = params
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| RpcError::invalid_params(e.to_string()))?
                .unwrap_or(cortex_core::tools::DiffParams { since: None, limit: None });
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(surface(daemon, &ctx.actor, &auditor).diff(&params).map_err(tool_error)?)
        }

        // ---- The agent brain (M11) --------------------------------------------------------

        method::BRAIN_REMEMBER => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .remember(&params)
                    .map_err(tool_error)?,
            )
        }

        method::BRAIN_SUPERSEDE => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .supersede(&params)
                    .map_err(tool_error)?,
            )
        }

        method::BRAIN_FORGET_MEMORY => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .forget_memory(&params)
                    .map_err(tool_error)?,
            )
        }

        method::BRAIN_MEMORIES => {
            let params = optional(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .memories(&params)
                    .map_err(tool_error)?,
            )
        }

        method::BRAIN_OUTCOME => {
            let params = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .outcome(&params)
                    .map_err(tool_error)?,
            )
        }

        // The one M11 tool that is assembled here rather than in `cortex-agent`, because it
        // needs the embedder and the memory collection — both of which the daemon owns and the
        // surface layer deliberately does not. The gate and the audit entry still go through
        // the surface, so a pack cannot be built while the kill switch is off.
        method::BRAIN_CONTEXT => {
            let params: cortex_core::tools::ContextParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let surface = surface(daemon, &ctx.actor, &auditor);
            surface.gate("brain.context").map_err(tool_error)?;
            daemon.activity.ping();

            let reader = surface.reader_scope(
                params.session_id.as_deref(),
                &params.project_paths,
            );
            let kinds: Vec<cortex_core::memory::MemoryKind> = params
                .kinds
                .iter()
                .filter_map(|k| cortex_core::memory::MemoryKind::parse(k))
                .collect();

            let loaded = daemon.semantics.get();
            let model = loaded.as_ref().map(|l| l.embedder.model());
            let semantic = match (loaded.as_ref(), model.as_ref()) {
                (Some(loaded), Some(model)) => Some(cortex_query::context::Semantic {
                    embedder: loaded.embedder.as_ref(),
                    memory_vectors: loaded.memory_vectors.as_ref(),
                    model: cortex_store::embeddings::ModelRef {
                        id: &model.id,
                        version: &model.version,
                        dim: model.dim,
                    },
                }),
                _ => None,
            };

            let now = indexer::unix_now();
            let pack_id = pack_id(now, &ctx.actor, &params.task);
            let pack = cortex_query::context::build(
                cortex_query::context::ContextParams {
                    store: &daemon.store,
                    semantic,
                    reader: &reader,
                    task: &params.task,
                    budget_tokens: params.budget_tokens.unwrap_or(DEFAULT_CONTEXT_BUDGET),
                    kinds: &kinds,
                    include_files: params.include_files.unwrap_or(true),
                    include_history: params.include_history.unwrap_or(false),
                    now,
                    candidates: cortex_query::context::DEFAULT_CANDIDATES,
                    mmr_lambda: cortex_query::context::DEFAULT_LAMBDA,
                    rrf_k: daemon.config.search.rrf_k,
                    chars_per_token: params
                        .chars_per_token
                        .unwrap_or(cortex_core::tokens::CHARS_PER_TOKEN),
                },
                &pack_id,
            );

            // The record is what makes an outcome attributable later. A pack that could not be
            // recorded is still a usable pack — it just cannot be reported on — so this warns
            // rather than failing the call.
            //
            // An empty pack is not recorded at all. There is nothing to report an outcome
            // against, and a store that kept one row per question nobody could answer would
            // fill up fastest on the fresh install where the memory is empty and every call
            // returns nothing.
            if !pack.items.is_empty() {
                if let Err(e) = daemon.store.record_pack(
                    &pack.pack_id,
                    params.session_id.as_deref(),
                    &params.task,
                    pack.budget_tokens,
                    pack.used_tokens,
                    &cortex_query::context::records(&pack),
                    now,
                ) {
                    tracing::warn!(error = %e, "could not record the context pack");
                }
                if let Err(e) = daemon.store.note_retrieved(&pack.memory_ids()) {
                    tracing::warn!(error = %e, "could not count a retrieval");
                }
            }

            auditor.allowed(
                now,
                "brain.context",
                &serde_json::to_value(&params).unwrap_or_default(),
                &format!(
                    "{} items, {} of {} tokens, {} dropped",
                    pack.items.len(),
                    pack.used_tokens,
                    pack.budget_tokens,
                    pack.dropped
                ),
            );
            json(pack)
        }

        method::AGENT_SESSION_BEGIN => {
            let params: cortex_core::tools::SessionBeginParams = optional(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let generated = session_id(indexer::unix_now(), &ctx.actor);
            let opened = surface(daemon, &ctx.actor, &auditor)
                .session_begin(&params, &generated)
                .map_err(tool_error)?;
            // Everything this connection does from here on belongs to this session, which is
            // what makes "what did this task do" answerable rather than "what did this agent
            // do, some time in the last year".
            ctx.session = Some(opened.session_id.clone());
            json(opened)
        }

        method::AGENT_SESSION_END => {
            let params: cortex_core::tools::SessionEndParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let ended = surface(daemon, &ctx.actor, &auditor)
                .session_end(&params)
                .map_err(tool_error)?;
            if ctx.session.as_deref() == Some(params.session_id.as_str()) {
                ctx.session = None;
            }
            json(ended)
        }

        method::AGENT_SESSIONS => {
            let params = optional(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            json(
                surface(daemon, &ctx.actor, &auditor)
                    .sessions(&params)
                    .map_err(tool_error)?,
            )
        }

        method::SYNC_STATUS => {
            let s = daemon.federation.snapshot();
            let counts = daemon.store.outbox_counts().map_err(internal)?;
            let basis = crate::sync_stage::describe(&s);
            json(SyncStatusResult {
                enabled: s.enabled,
                transport: s.transport,
                peer: s.peer,
                origin: s.origin,
                pending: counts.pending,
                dead: counts.dead,
                acked: counts.acked,
                episodes_behind: counts.episodes_behind,
                lamport: counts.lamport,
                last_acked_lamport: s.last_acked_lamport,
                last_sync_at: s.last_sync_at,
                batches_sent: s.batches_sent,
                events_accepted: s.events_accepted,
                events_rejected: s.events_rejected,
                backoff_ms: s.backoff_ms,
                not_before_ms: s.not_before_ms,
                last_error: s.last_error,
                last_tick: s.last_tick.and_then(|t| serde_json::to_value(t).ok()),
                ontology_version: s.ontology_version,
                basis,
            })
        }

        method::SYNC_NOW => {
            let kicked = daemon.federation.kick();
            json(SyncNowResult {
                kicked,
                basis: if kicked {
                    "the worker will send on its next step, interval and backoff set aside".into()
                } else {
                    "there is no worker to kick; see sync.status for why".into()
                },
            })
        }

        method::SYNC_REPLAY => {
            let replaying = daemon.federation.replay();
            json(SyncReplayResult {
                replaying,
                basis: if replaying {
                    "the outbox is cleared and every episode is harvested again with the same \
                     identifiers; a peer that keeps them will see nothing new"
                        .into()
                } else {
                    "there is no worker to replay through; see sync.status for why".into()
                },
            })
        }

        // `brain.search` and `brain.explain` are the same operations as `search` and
        // `search.explain`, reached by the name an agent looks for. Aliases rather than
        // reimplementations — a second search would be a second set of results to explain.
        method::BRAIN_SEARCH => {
            Box::pin(dispatch(daemon, method::SEARCH, params, refinable, ctx)).await
        }

        method::BRAIN_EXPLAIN => {
            Box::pin(dispatch(daemon, method::SEARCH_EXPLAIN, params, refinable, ctx)).await
        }

        method::BRAIN_RECALL => {
            let params: RecallParams = parse(params)?;
            let started = Instant::now();
            daemon.activity.ping();

            // Pattern completion, not retrieval: search finds the way in, and the associative
            // graph supplies what belongs with it. A cue that names one document should bring
            // back the note somebody wrote on it and the file it was derived from, neither of
            // which contains the words in the cue.
            let outcome = daemon.run_search(&SearchParams {
                query: params.cue.clone(),
                mode: cortex_core::model::QueryMode::Auto,
                limit: params.limit.or(Some(10)),
                prefix: None,
                rerank: Some(false),
            });

            let depth = params.depth.unwrap_or(1).min(3);
            let mut associations = Vec::new();
            let mut seen: std::collections::HashSet<i64> =
                outcome.hits.iter().map(|h| h.node_id.0).collect();

            for hit in &outcome.hits {
                let neighbors = daemon
                    .store
                    .neighbors(hit.node_id, &[], depth, 20)
                    .map_err(internal)?;
                for neighbor in neighbors {
                    if !seen.insert(neighbor.node_id) {
                        continue;
                    }
                    associations.push(RecalledAssociation {
                        node_id: neighbor.node_id,
                        path: neighbor.path.map(|p| p.display().to_string()),
                        via: hit.path.display().to_string(),
                        edge: neighbor.edge.as_str().to_string(),
                        depth: neighbor.depth,
                        note: neighbor.note,
                    });
                }
            }

            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let recaller = surface(daemon, &ctx.actor, &auditor);
            if let Err(e) = recaller.gate("brain.recall") {
                return Err(tool_error(e));
            }

            // The cue against the memory layer as well. Pattern completion that could only
            // reach what an agent learned when the learning happened to hang off a file the
            // file search also found would be reliable for the easy half of the question and
            // silent for the other.
            //
            // The task form of the expression rather than the search form: a cue is a phrase,
            // and a conjunction of every word in it matches nothing over a store of thousands.
            let mut memories = Vec::new();
            if let Some(expression) = cortex_query::lexical::task_expression(&params.cue) {
                let reader = recaller.reader_scope(None, &[]);
                let now = indexer::unix_now();
                match daemon.store.search_memories(
                    &expression,
                    &reader.visible_scopes(),
                    now,
                    params.limit.unwrap_or(10),
                ) {
                    Ok(hits) => {
                        for hit in hits {
                            if let Ok(Some(memory)) = daemon.store.memory(hit.node_id) {
                                memories.push(memory);
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "the memory arm of recall failed"),
                }
            }

            auditor.allowed(
                indexer::unix_now(),
                "brain.recall",
                &serde_json::json!({ "cue": params.cue, "depth": depth }),
                &format!(
                    "{} hits, {} associations, {} memories",
                    outcome.hits.len(),
                    associations.len(),
                    memories.len()
                ),
            );

            json(RecallResult {
                hits: outcome.hits,
                associations,
                memories,
                mode: outcome.mode,
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }

        method::FS_MKDIR => {
            let params: cortex_core::tools::FsMkdirParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let made = surface(daemon, &ctx.actor, &auditor)
                .mkdir(&params.path)
                .map_err(tool_error)?;
            daemon.ingest.signal_work();
            json(serde_json::json!({ "path": made }))
        }

        method::FS_RENAME => {
            let params: cortex_core::tools::FsRenameParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
            let renamed = surface(daemon, &ctx.actor, &auditor)
                .rename(&params.path, &params.name)
                .map_err(tool_error)?;
            rebuild_index(daemon).map_err(internal)?;
            json(serde_json::json!({ "path": renamed }))
        }

        method::OPERATION_CANCEL => {
            let params: cortex_core::tools::OperationCancelParams = parse(params)?;
            json(serde_json::json!({ "cancelled": daemon.operations.cancel(params.operation) }))
        }

        method::FS_TRANSFER => {
            let params: cortex_core::tools::FsTransferParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);

            // Everything that can be refused is refused here, before an id exists. An operation
            // that starts and then immediately fails on policy grounds would make the caller
            // wait for a notification to learn what a call could have told it.
            let plan = surface(daemon, &ctx.actor, &auditor)
                .plan_transfer(&params.sources, &params.destination, params.mode)
                .map_err(tool_error)?;

            // A dry run answers what would happen and moves nothing. It is how a client learns
            // which names already exist, so it can ask before anything is touched rather than
            // reporting a failure afterwards.
            if params.dry_run {
                return json(cortex_core::tools::TransferAnswer::Planned {
                    total_files: plan.total.files,
                    total_bytes: plan.total.bytes,
                    conflicts: plan.conflicts,
                });
            }

            let (sources, destination, total) = (plan.sources, plan.destination, plan.total);
            let (operation, handle) = daemon.operations.start();
            let started = Instant::now();
            let answer = cortex_core::tools::TransferAnswer::Started {
                operation,
                total_files: total.files,
                total_bytes: total.bytes,
            };

            let daemon = daemon.clone();
            let actor = ctx.actor.clone();
            let notify = ctx.notify.clone();
            let conflict = params.conflict;
            let mode = params.mode;

            // On the blocking pool: this is `read` and `write` syscalls for as long as it
            // takes, and running it on the async runtime would stop the daemon answering
            // anything else for the duration.
            tokio::task::spawn_blocking(move || {
                let auditor = Auditor::new(daemon.store.clone(), &actor);
                let surface = surface(&daemon, &actor, &auditor);
                let mut throttle = crate::operations::Throttle::new(
                    crate::operations::PROGRESS_INTERVAL,
                );

                let outcome = surface.run_transfer(
                    &sources,
                    &destination,
                    mode,
                    conflict,
                    total,
                    &handle,
                    |progress| {
                        if throttle.should_send(Instant::now()) {
                            notify(
                                method::TRANSFER_PROGRESS,
                                serde_json::to_value(crate::operations::to_wire(
                                    operation, progress,
                                ))
                                .unwrap_or_default(),
                            );
                        }
                    },
                );

                daemon.operations.finish(operation);
                // The index believes in paths that have moved. A transfer of five files is one
                // rebuild rather than five.
                let _ = rebuild_index(&daemon);
                daemon.ingest.signal_work();

                notify(
                    method::TRANSFER_DONE,
                    serde_json::to_value(cortex_core::tools::TransferDone {
                        operation,
                        moved: outcome.moved,
                        failed: outcome.failed,
                        cancelled: outcome.cancelled,
                        undo_ids: outcome.undo_ids,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    })
                    .unwrap_or_default(),
                );
            });

            json(answer)
        }

        method::VIEW_OPEN => {
            use cortex_core::view::{self, Order, Source, View, ViewRow};
            let params: cortex_rpc::protocol::ViewOpenParams = parse(params)?;
            let started = Instant::now();
            let origin = params.source.clone();

            match params.source {
                // A directory is one query among several, and this is the one that has to keep
                // proving it: the same `Surface::list` answers it, with the same deny-list, the
                // same ordering and the same audit entry as `fs.list`. Nothing here is a second
                // implementation of listing a directory.
                Source::Dir { path } => {
                    let order = params.order.unwrap_or(Order::Name);
                    let sort = match order {
                        Order::Name => cortex_core::tools::SortBy::Name,
                        Order::Size => cortex_core::tools::SortBy::Size,
                        Order::Modified => cortex_core::tools::SortBy::Modified,
                        // Salience is not something a directory has; the listing comes back by
                        // name and is reordered below, once the numbers are attached. Asking the
                        // filesystem layer for an order it cannot know would be inventing one.
                        Order::Salience | Order::None => cortex_core::tools::SortBy::Name,
                    };
                    let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);
                    let listed = surface(daemon, &ctx.actor, &auditor)
                        .list(&cortex_core::tools::FsListParams {
                            path,
                            glob: None,
                            depth: Some(1),
                            limit: params.limit,
                            sort,
                            reverse: params.reverse,
                        })
                        .map_err(tool_error)?;

                    let hidden = listed.hidden as u64;
                    let truncated = listed.truncated;
                    let mut rows: Vec<ViewRow> =
                        listed.entries.into_iter().map(ViewRow::from_entry).collect();

                    // The memory columns, in one statement for the whole listing rather than one
                    // per row (§5). Measured before it was built: on ten thousand rows the join
                    // costs a sixth of the per-row lookups the listing already makes — see
                    // `cortex-store/tests/listing_cost.rs`.
                    let shown: Vec<std::path::PathBuf> =
                        rows.iter().map(|r| r.path.clone()).collect();
                    let columns =
                        cortex_store::memory_columns::for_paths(&daemon.store, &shown)
                            .unwrap_or_default();
                    let mut known = 0u64;
                    for row in &mut rows {
                        if let Some(c) = columns.get(&row.path) {
                            known += 1;
                            cortex_core::view::attach_memory(
                                row,
                                c.node_id,
                                c.tier,
                                c.salience,
                                c.pinned,
                                c.zone_id,
                                c.degree,
                                c.last_episode,
                            );
                        }
                    }

                    // Ordering is the daemon's for every source (D4). Salience is the one order
                    // the filesystem cannot provide, so it is applied here — on stored numbers,
                    // with the name as the tie-break so two listings never disagree.
                    if order == Order::Salience {
                        rows.sort_by(|a, b| {
                            let (x, y) = (a.salience.unwrap_or(0.0), b.salience.unwrap_or(0.0));
                            y.partial_cmp(&x)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.path.cmp(&b.path))
                        });
                        if params.reverse {
                            rows.reverse();
                        }
                    }

                    let total = rows.len() as u64;

                    json(View {
                        basis: view::basis_for_dir(
                            rows.len(),
                            total,
                            hidden,
                            truncated,
                            order,
                            known,
                        ),
                        rows,
                        origin: Source::Dir {
                            path: listed.path,
                        },
                        order,
                        total,
                        truncated,
                        hidden,
                        refining: false,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        cursor: None,
                    })
                }

                Source::Query { text } => {
                    // Everything `search` does, because it *is* `search`: the same activity
                    // ping, the same episode — "what was I looking for on Tuesday" must not
                    // depend on which client asked — and the same reranking, delivered in the
                    // shape this caller reads.
                    daemon.activity.ping();
                    let search_params = SearchParams {
                        query: text.clone(),
                        mode: Default::default(),
                        limit: params.limit,
                        prefix: None,
                        rerank: params.rerank,
                    };
                    let outcome = daemon.run_search(&search_params);

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
                            "via": "view.open",
                        }),
                    );

                    let refining = params.rerank.unwrap_or(daemon.config.search.rerank)
                        && daemon.semantics.reranker().is_some()
                        && !outcome.hits.is_empty();

                    let mode = format!("{:?}", outcome.mode).to_lowercase();
                    let mut rows: Vec<ViewRow> = outcome
                        .hits
                        .iter()
                        .cloned()
                        .map(ViewRow::from_hit)
                        .collect();

                    // A hit already carries its tier and its zone; salience, degree, when it was
                    // last touched and whether somebody pinned it come from the same statement a
                    // directory uses, keyed by id instead of by parent.
                    let ids: Vec<cortex_core::ids::NodeId> =
                        rows.iter().filter_map(|r| r.node_id).collect();
                    let columns =
                        cortex_store::memory_columns::for_nodes(&daemon.store, &ids)
                            .unwrap_or_default();
                    for row in &mut rows {
                        if let Some(c) = row.node_id.and_then(|id| columns.get(&id)) {
                            cortex_core::view::attach_memory(
                                row,
                                c.node_id,
                                c.tier,
                                c.salience,
                                c.pinned,
                                c.zone_id,
                                c.degree,
                                c.last_episode,
                            );
                        }
                    }
                    let total = rows.len() as u64;
                    let answer = View {
                        basis: view::basis_for_query(&text, &mode, total, refining),
                        rows,
                        origin,
                        // Relevance, kept. Re-sorting a ranked list by name would throw away the
                        // answer and leave the question.
                        order: Order::None,
                        total,
                        truncated: false,
                        hidden: 0,
                        refining,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        cursor: None,
                    };
                    if refining {
                        *refinable = Some((outcome, Refinement::View));
                    }
                    json(answer)
                }

                // Everything that is neither a directory nor a search: a concept, the working
                // set, the graph around a node, a slice of time, a tier. They differ only in
                // *which* nodes and *in what order* — the rest is one path through `rows_for`,
                // which is what keeps "what a row is" from drifting five ways.
                ref other => {
                    let limit = params.limit.unwrap_or(500).min(5000);
                    let now = indexer::unix_now();
                    let store = &daemon.store;

                    let (ids, total): (Vec<cortex_core::ids::NodeId>, u64) = match other {
                        Source::Concept { id, .. } => {
                            let members = store
                                .neighbors(
                                    cortex_core::ids::NodeId(*id),
                                    &[cortex_core::model::EdgeType::DerivedFrom],
                                    1,
                                    limit,
                                )
                                .map_err(internal)?;
                            let ids: Vec<_> = members
                                .iter()
                                .filter(|n| n.path.is_some())
                                .map(|n| cortex_core::ids::NodeId(n.node_id))
                                .collect();
                            let total = ids.len() as u64;
                            (ids, total)
                        }
                        Source::WorkingSet => {
                            let touched = store
                                .recently_touched(
                                    now - cortex_agent::brain_tools::WORKING_SET_WINDOW_SECS,
                                    limit,
                                )
                                .map_err(internal)?;
                            let ids: Vec<_> = touched
                                .iter()
                                .filter(|t| t.path.is_some())
                                .map(|t| cortex_core::ids::NodeId(t.node_id))
                                .collect();
                            let total = ids.len() as u64;
                            (ids, total)
                        }
                        Source::Neighbours { path, edges } => {
                            let node = store
                                .node_by_path(path)
                                .map_err(internal)?
                                .ok_or_else(|| {
                                    RpcError::invalid_params(&format!(
                                        "{} is not in the index, so it has no neighbours yet",
                                        path.display()
                                    ))
                                })?;
                            let types: Vec<cortex_core::model::EdgeType> = edges
                                .iter()
                                .filter_map(|t| cortex_core::model::EdgeType::parse(t))
                                .collect();
                            let found = store
                                .neighbors(node.id, &types, 1, limit)
                                .map_err(internal)?;
                            let ids: Vec<_> = found
                                .iter()
                                .filter(|n| n.path.is_some())
                                .map(|n| cortex_core::ids::NodeId(n.node_id))
                                .collect();
                            let total = ids.len() as u64;
                            (ids, total)
                        }
                        Source::Timeline { from, to } => {
                            let episodes = store
                                .episodes_in_range(Some(*from), Some(*to), None, None, limit * 4)
                                .map_err(internal)?;
                            // One row per file, in the order it was last touched inside the
                            // window: a timeline of episodes would list the same file eleven
                            // times for eleven saves, which is a log rather than a place.
                            let mut seen = std::collections::HashSet::new();
                            let mut ids = Vec::new();
                            for episode in &episodes {
                                if let Some(node) = episode.node_id {
                                    if seen.insert(node.0) {
                                        ids.push(node);
                                    }
                                }
                            }
                            ids.truncate(limit);
                            let total = ids.len() as u64;
                            (ids, total)
                        }
                        Source::Tier { tier } => {
                            let parsed = cortex_core::model::Tier::parse(tier).ok_or_else(|| {
                                RpcError::invalid_params(&format!("no such tier: {tier}"))
                            })?;
                            let ids = cortex_store::memory_columns::in_tier(store, parsed, limit)
                                .map_err(internal)?;
                            let total = cortex_store::memory_columns::tier_total(store, parsed)
                                .map_err(internal)?;
                            (ids, total)
                        }
                        Source::Dir { .. } | Source::Query { .. } => unreachable!("handled above"),
                    };

                    let indexed = cortex_store::memory_columns::rows_for(store, &ids)
                        .map_err(internal)?;
                    let rows: Vec<ViewRow> = indexed
                        .into_iter()
                        .map(|row| {
                            let mut out = ViewRow {
                                path: row.path,
                                kind: if row.kind == "dir" {
                                    cortex_core::tools::FsKind::Dir
                                } else {
                                    cortex_core::tools::FsKind::File
                                },
                                size: row.size.max(0) as u64,
                                mtime: row.mtime,
                                indexed: true,
                                node_id: None,
                                zone_id: None,
                                tier: None,
                                snippet: None,
                                score: None,
                                salience: None,
                                degree: None,
                                last_episode: None,
                                pinned: false,
                                mode: None,
                                owner: None,
                                group: None,
                                target: None,
                                target_kind: None,
                                broken: false,
                            };
                            let c = row.columns;
                            cortex_core::view::attach_memory(
                                &mut out,
                                c.node_id,
                                c.tier,
                                c.salience,
                                c.pinned,
                                c.zone_id,
                                c.degree,
                                c.last_episode,
                            );
                            out
                        })
                        .collect();

                    let shown = rows.len() as u64;
                    json(View {
                        basis: view::basis_for(&origin, shown, total),
                        rows,
                        origin,
                        // The order is the source's own — most recently touched, most connected,
                        // most salient — and it is already applied. Re-sorting would throw away
                        // the thing that makes the list a place rather than a bag.
                        order: Order::None,
                        total,
                        truncated: shown < total,
                        hidden: 0,
                        refining: false,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        cursor: None,
                    })
                }
            }
        }

        method::FS_EXTRACT => {
            let params: cortex_core::tools::FsExtractParams = parse(params)?;
            let auditor = Auditor::new(daemon.store.clone(), &ctx.actor);

            // Everything that can be refused is refused before an operation id exists, for the
            // same reason as a transfer: an operation that starts and then fails on policy
            // grounds makes the caller wait for a notification to learn what the call could
            // have told it.
            let plan = surface(daemon, &ctx.actor, &auditor)
                .plan_extract(&params.archive, params.destination.as_deref(), params.conflict)
                .map_err(tool_error)?;

            if params.dry_run {
                return json(cortex_core::tools::ExtractAnswer::Planned {
                    format: plan.format,
                    destination: plan.destination,
                    total_files: plan.total.files,
                    total_bytes: plan.total.bytes,
                    entries: plan.entries,
                    truncated: plan.truncated,
                    occupied: plan.occupied,
                });
            }

            let (archive, destination, format, total) =
                (plan.archive, plan.destination, plan.format, plan.total);
            let (operation, handle) = daemon.operations.start();
            let started = Instant::now();
            let answer = cortex_core::tools::ExtractAnswer::Started {
                operation,
                destination: destination.clone(),
                total_files: total.files,
                total_bytes: total.bytes,
            };

            let daemon = daemon.clone();
            let actor = ctx.actor.clone();
            let notify = ctx.notify.clone();

            tokio::task::spawn_blocking(move || {
                let auditor = Auditor::new(daemon.store.clone(), &actor);
                let surface = surface(&daemon, &actor, &auditor);
                let mut throttle =
                    crate::operations::Throttle::new(crate::operations::PROGRESS_INTERVAL);

                let outcome = surface.run_extract(
                    &archive,
                    &destination,
                    format,
                    total,
                    &handle,
                    |progress| {
                        if throttle.should_send(Instant::now()) {
                            notify(
                                method::TRANSFER_PROGRESS,
                                serde_json::to_value(crate::operations::to_wire(
                                    operation, progress,
                                ))
                                .unwrap_or_default(),
                            );
                        }
                    },
                );

                daemon.operations.finish(operation);
                // A directory of new files the index has never seen: one rebuild, then the
                // ingest pass that reads them.
                let _ = rebuild_index(&daemon);
                daemon.ingest.signal_work();

                // The same notification a transfer ends on, because to whoever is watching the
                // progress bar it is the same operation ending. `moved` carries the one path
                // that matters — where it landed.
                notify(
                    method::TRANSFER_DONE,
                    serde_json::to_value(cortex_core::tools::TransferDone {
                        operation,
                        moved: if outcome.cancelled {
                            Vec::new()
                        } else {
                            vec![outcome.destination.clone()]
                        },
                        failed: outcome.refused,
                        cancelled: outcome.cancelled,
                        undo_ids: outcome.undo_id.into_iter().collect(),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    })
                    .unwrap_or_default(),
                );
            });

            json(answer)
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

/// Build the tool surface for one call.
///
/// Cheap: everything in it is borrowed or an `Arc` clone. It is built per call rather than held
/// on the daemon so that the kill switch is read at the moment of the call — a surface cached
/// at connection time would keep serving an agent after the switch was thrown.
fn surface<'a>(
    daemon: &'a Arc<Daemon>,
    actor: &cortex_core::model::Actor,
    auditor: &'a Auditor,
) -> Surface<'a> {
    Surface {
        policy: &daemon.policy,
        enabled: daemon.kill.agent_surface_enabled(),
        store: &daemon.store,
        auditor,
        plans: &daemon.plans,
        trash: &daemon.trash,
        actor: actor.clone(),
        now: indexer::unix_now(),
    }
}

/// Translate a tool failure into the wire error, keeping the refusal's own code.
///
/// The code is what a client branches on — "ask the user to widen the allow-list" is a
/// different reaction from "make a plan first" — so flattening every refusal into one internal
/// error would make the surface unusable from the other side.
fn tool_error(e: ToolError) -> RpcError {
    let code = e.code();
    let mut error = RpcError::new(code, e.to_string());
    if let ToolError::Refused(refusal) = &e {
        error.data = Some(serde_json::json!({ "refusal": refusal.tag() }));
    }
    error
}

fn parse<T: serde::de::DeserializeOwned>(
    params: Option<serde_json::Value>,
) -> Result<T, RpcError> {
    let params = params.ok_or_else(|| RpcError::invalid_params("missing params"))?;
    serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))
}

/// Parse params that a caller may leave out entirely.
fn optional<T: serde::de::DeserializeOwned + Default>(
    params: Option<serde_json::Value>,
) -> Result<T, RpcError> {
    match params {
        None | Some(serde_json::Value::Null) => Ok(T::default()),
        Some(value) => {
            serde_json::from_value(value).map_err(|e| RpcError::invalid_params(e.to_string()))
        }
    }
}

/// How much of a context window a pack fills when the caller does not say.
///
/// Two thousand tokens: about one per cent of a large context window, and enough for a dozen
/// memories with their attribution. A default that filled a tenth of the window would be a
/// default nobody leaves alone.
const DEFAULT_CONTEXT_BUDGET: usize = 2_048;

/// A stable, readable identifier for one pack.
///
/// Derived rather than random because this crate has no random number generator and does not
/// need one: the pair (when, who) is unique in practice, and the hash of the task makes two
/// packs in the same second distinguishable. It is an identifier, not a secret.
fn pack_id(now: i64, actor: &cortex_core::model::Actor, task: &str) -> String {
    let digest = blake3::hash(task.as_bytes()).to_hex();
    format!("pack-{now}-{}-{}", short_actor(actor), &digest[..8])
}

fn session_id(now: i64, actor: &cortex_core::model::Actor) -> String {
    format!("s-{now}-{}", short_actor(actor))
}

fn short_actor(actor: &cortex_core::model::Actor) -> String {
    match actor {
        cortex_core::model::Actor::Agent(name) => name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(12)
            .collect(),
        other => other.as_str(),
    }
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

/// Why a memory is what it is: where it belongs, what it has earned, and what replaced it.
///
/// Deliberately not a ranking explanation. A memory does not compete with files for a place in
/// a result list — it competes with other memories for a place in a context pack, and the pack
/// explains that per item. What this answers is the other question: given that it is there,
/// what is known about it and should it still be believed.
fn explain_memory(memory: &cortex_core::memory::Memory) -> Vec<String> {
    let mut reasons = Vec::new();
    reasons.push(format!(
        "this is a {} memory in {}, written by {}",
        memory.kind, memory.scope, memory.actor
    ));

    if let Some(replaced_by) = memory.superseded_by {
        reasons.push(format!(
            "it has been superseded by memory {replaced_by}, so retrieval no longer returns it — \
             it is kept because the history is the reason supersession exists instead of deletion"
        ));
    }
    if let Some(expires) = memory.expires_at {
        reasons.push(format!(
            "it is session scratch and expires at {expires}; only session scope has a clock, \
             everything else is the decay engine's decision"
        ));
    }

    // The three counts, and what their relationship means. This is the part a person actually
    // wants: whether the thing has ever been worth its place.
    reasons.push(format!(
        "it has been put into {} context pack(s), reported helpful {} time(s) and noise {} \
         time(s); its earned usefulness is {:+.3}",
        memory.retrieved_count, memory.helped_count, memory.noise_count, memory.usefulness
    ));
    if memory.retrieved_count >= 5 && memory.helped_count == 0 && memory.noise_count == 0 {
        reasons.push(
            "it keeps arriving in context and nobody has ever said whether it helped, which is \
             why its usefulness is falling: retrieval without use is evidence against a \
             memory, because the retrieval was chosen by the ranking being evaluated"
                .to_string(),
        );
    }
    if memory.human_confirmed {
        reasons.push("a human has confirmed it, which outweighs any agent's own report".to_string());
    } else if memory.helped_count > 0 {
        reasons.push(
            "the reports on it are an agent's own, which are discounted rather than trusted"
                .to_string(),
        );
    }

    reasons.push(format!(
        "its writer claimed a confidence of {:.2}. That is a claim, not evidence; usefulness \
         is the number that was earned",
        memory.confidence
    ));
    if let Some(path) = &memory.subject_path {
        reasons.push(format!("it is about {}", path.display()));
    }
    if memory.pinned {
        reasons.push("it is pinned, so nothing forgets it".to_string());
    }
    reasons
}


#[cfg(test)]
mod respond_tests {
    use super::*;

    /// The answer that does not fit must come back as an error, not as silence.
    ///
    /// This is the regression test for a hang: a `brain.graph` over a large index produced a
    /// reply above the transport's line limit, the concurrent path discarded the write error,
    /// and the caller waited for ever with both processes idle.
    #[tokio::test]
    async fn a_reply_that_does_not_fit_comes_back_as_an_error() {
        let writer = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let id = serde_json::json!(7);
        let oversized = serde_json::json!({
            "s": "x".repeat(transport::MAX_LINE_BYTES + 1_024)
        });

        respond(&writer, id.clone(), Response::ok(id.clone(), oversized))
            .await
            .expect("the fallback must be sendable even when the answer is not");

        let written = writer.lock().await.clone();
        let line = String::from_utf8(written).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(value["id"], id);
        assert_eq!(value["error"]["code"], RpcError::RESULT_TOO_LARGE);
        assert!(
            value["error"]["message"].as_str().unwrap().contains("ask for less"),
            "the error has to say what to do about it: {value}"
        );
        assert!(line.len() < transport::MAX_LINE_BYTES, "the error itself must fit");
    }

    /// The ordinary case must be untouched: one line, the result, nothing added.
    #[tokio::test]
    async fn an_ordinary_reply_is_written_once_and_unchanged() {
        let writer = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let id = serde_json::json!("abc");
        respond(&writer, id.clone(), Response::ok(id.clone(), serde_json::json!({ "n": 1 })))
            .await
            .unwrap();

        let line = String::from_utf8(writer.lock().await.clone()).unwrap();
        assert_eq!(line.lines().count(), 1);
        let value: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(value["result"]["n"], 1);
        assert!(value.get("error").is_none());
    }
}
