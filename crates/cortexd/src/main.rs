//! The Cortex daemon.
//!
//! Owns the index and serves every client over one Unix socket. See `docs/04-ARCHITECTURE.md`.

mod activity;
mod embed_stage;
mod extract_stage;
mod indexer;
mod ingest;
mod lock;
mod operations;
mod semantics;
mod server;
mod sync_stage;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_core::config::Config;
use cortex_core::paths::Paths;
use cortex_query::PathIndex;
use cortex_store::Store;
use std::sync::{Arc, RwLock};

#[derive(Parser, Debug)]
#[command(name = "cortexd", version, about = "The Cortex memory daemon")]
struct Args {
    /// Put config, state and runtime files under one root. For tests and throwaway instances.
    #[arg(long, value_name = "DIR")]
    root: Option<std::path::PathBuf>,

    /// Log filter, overriding the configuration file.
    #[arg(long, value_name = "FILTER")]
    log: Option<String>,

    /// Start, print the resolved configuration and exit. Useful for verifying a deployment.
    #[arg(long)]
    check: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let paths = match &args.root {
        Some(root) => Paths::rooted_at(root),
        None => Paths::from_env().context("could not determine where to keep state")?,
    };
    paths.ensure_dirs()?;

    let mut config = Config::load(&paths.config_file())?;
    if let Some(filter) = args.log {
        config.log.filter = filter;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log.filter)),
        )
        .with_writer(std::io::stderr)
        .init();

    if args.check {
        println!("{}", config.to_toml());
        println!("# database: {}", paths.database().display());
        println!("# socket:   {}", paths.socket().display());
        return Ok(());
    }

    // The lock is acquired before anything touches the database. Two daemons against one index
    // produce duplicated episodes and interleaved writes that look like corruption much later.
    let _lock = lock::InstanceLock::acquire(&paths.lock_file())?;

    let store = Arc::new(
        Store::open(&paths.database())
            .with_context(|| format!("could not open {}", paths.database().display()))?,
    );

    // Build the in-memory path index before serving anything. A daemon that accepts queries
    // while its index is still empty answers "no results" to questions it could answer, which
    // is worse than making the client wait the ~150 ms this takes.
    let started = std::time::Instant::now();
    let index = Arc::new(RwLock::new(PathIndex::build(
        store.all_paths_with_ids().context("could not read indexed paths")?,
    )));
    tracing::info!(
        paths = index.read().unwrap().len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "path index ready"
    );

    let ingest_state = Arc::new(ingest::IngestState::default());
    let activity = Arc::new(activity::Activity::default());
    let extract_stats = Arc::new(extract_stage::ExtractStats::default());
    let embed_stats = Arc::new(embed_stage::EmbedStats::default());

    // The model is a deliberate download (rule R5), so not having one is the normal state of a
    // fresh install and must not look like a failure.
    let semantics = Arc::new(semantics::Semantics::new(
        paths.state_dir(),
        config.embedding.clone(),
        config.search.rerank,
    ));
    match semantics.load() {
        Ok(backend) => tracing::info!(backend = %backend, "embedding model ready"),
        Err(note) => tracing::info!(
            reason = %note,
            "no embedding model; path and lexical search are unaffected"
        ),
    }
    // The reranker is a separate download and a separate failure: a machine with one and not the
    // other gets the half it has.
    match semantics.load_reranker() {
        Ok(backend) => tracing::info!(backend = %backend, "reranker ready"),
        Err(note) => tracing::info!(reason = %note, "no reranker; results are shown as retrieved"),
    }

    // The ingest loop gets its own thread: its work is blocking by nature, and running it on
    // the async runtime would let a long walk stall every query.
    let ingest_thread = {
        let ingest = ingest::Ingest::new(
            store.clone(),
            index.clone(),
            ingest_state.clone(),
            config.clone(),
        );
        std::thread::Builder::new()
            .name("cortex-ingest".into())
            .spawn(move || ingest.run())?
    };

    // Extraction gets a thread of its own rather than a slot in the ingest loop. The two have
    // opposite requirements: perception must react within a debounce window, while extraction
    // is throttled to stay invisible. Sharing a thread would make the watcher wait behind a
    // deliberate pause.
    let extract_thread = {
        let stage = extract_stage::Extraction::new(
            store.clone(),
            ingest_state.clone(),
            activity.clone(),
            extract_stats.clone(),
            config.clone(),
        );
        std::thread::Builder::new()
            .name("cortex-extract".into())
            .spawn(move || stage.run())?
    };

    // Its own thread, like extraction, and for the same reason: the work is blocking and
    // throttled. It waits for a model rather than being spawned only when one exists, so
    // `cortex model fetch` followed by `model.reload` needs no restart.
    let embed_thread = {
        let stage = embed_stage::Embedding::new(
            store.clone(),
            semantics.clone(),
            ingest_state.clone(),
            activity.clone(),
            embed_stats.clone(),
            config.clone(),
        );
        std::thread::Builder::new()
            .name("cortex-embed".into())
            .spawn(move || stage.run())?
    };

    // Federation (M7). Off by default; when on, the only thing it knows about the peer is a
    // directory or a plugin name, and it runs on its own thread like every other stage.
    let federation = Arc::new(sync_stage::Federation::start(store.clone(), &config, &paths));
    let federation_for_shutdown = federation.clone();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let ingest_for_shutdown = ingest_state.clone();
    runtime.block_on(async move {
        // The kill switch is read before anything is served: a daemon that comes back from a
        // crash must come back in the state its owner left it in, not in the state its
        // configuration file describes (R7).
        let kill = Arc::new(cortex_agent::KillSwitch::load(store.clone(), config.agent.enabled)?);
        if kill.indexing_paused() {
            ingest_state.set_paused(true);
            tracing::warn!("indexing is paused (persisted); `cortex resume` starts it again");
        }
        if !kill.agent_surface_enabled() {
            tracing::warn!("the Agent Surface is switched off (persisted)");
        }

        let daemon = Arc::new(server::Daemon::new(
            store,
            index,
            ingest_state,
            activity,
            extract_stats,
            embed_stats,
            semantics,
            config,
            kill,
            // The shared trash, not one of our own: a file this daemon deletes must be in the
            // same place a file deleted by anything else is, and restorable by anything else.
            cortex_agent::trash::Trash::home()
                .unwrap_or_else(|_| cortex_agent::trash::Trash::at(paths.trash_dir())),
            paths.database(),
            federation.clone(),
        ));
        let listener = server::bind(&paths.socket())
            .with_context(|| format!("could not bind {}", paths.socket().display()))?;

        tracing::info!(
            version = server::VERSION,
            socket = %paths.socket().display(),
            database = %paths.database().display(),
            "cortexd ready"
        );

        let shutdown = daemon.shutdown.clone();
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
                _ = sigterm.recv() => tracing::info!("terminated"),
            }
            shutdown.notify_waiters();
        });

        server::serve(listener, daemon).await;

        // The socket is ours; leaving it behind would confuse the next start into thinking a
        // daemon is listening.
        let _ = std::fs::remove_file(paths.socket());
        Ok::<(), anyhow::Error>(())
    })?;

    // Let the ingest loop finish its current pass rather than tearing it down mid-write.
    federation_for_shutdown.stop();
    ingest_for_shutdown.stop();
    if let Err(e) = ingest_thread.join() {
        tracing::warn!("ingest thread did not stop cleanly: {e:?}");
    }
    if let Err(e) = extract_thread.join() {
        tracing::warn!("extraction thread did not stop cleanly: {e:?}");
    }
    if let Err(e) = embed_thread.join() {
        tracing::warn!("embedding thread did not stop cleanly: {e:?}");
    }

    tracing::info!("cortexd stopped");
    Ok(())
}
