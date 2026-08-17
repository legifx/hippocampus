//! The Cortex daemon.
//!
//! Owns the index and serves every client over one Unix socket. See `docs/04-ARCHITECTURE.md`.

mod indexer;
mod lock;
mod server;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_core::config::Config;
use cortex_core::paths::Paths;
use cortex_store::Store;
use std::sync::Arc;

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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let daemon = Arc::new(server::Daemon::new(store, config, paths.database()));
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
        tracing::info!("cortexd stopped");
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
