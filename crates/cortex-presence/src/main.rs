//! `cortex-presence` — tells the daemon when somebody is at the machine (D130).
//!
//! The daemon's duty cycle depends on knowing whether a person is using the machine, and the
//! daemon is not allowed to find out by itself (D22): no input devices, no compositor. Before
//! this, only the overlay and interactive requests said so, so a person playing a game or
//! watching a video was "idle" and the index ran at its idle budget under them. This client asks
//! the compositor through `ext-idle-notify-v1` — the standard Wayland protocol, which reports
//! "no input for N seconds" and "input again", and never a keystroke — and turns that into
//! `activity.ping` every 30 seconds for as long as somebody is there.
//!
//! A separate process so the daemon stays headless: on a server this binary is simply not
//! started, and on a desktop whose compositor lacks the protocol it says so once and exits.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use cortex_core::paths::Paths;
use cortex_rpc::client::Client;
use cortex_rpc::protocol::method;
use tokio::sync::watch;

mod presence;
mod wayland;

use presence::Presence;

/// How often the loop looks at the clock. Pings themselves are `presence::PING_EVERY` apart;
/// this only bounds how long a failed ping waits before it is tried again.
const CHECK_EVERY: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(version, about = "Tell the Cortex daemon when somebody is at the machine.")]
struct Args {
    /// Seconds without input after which nobody counts as there.
    #[arg(long, default_value_t = 60)]
    idle_after: u64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let socket = Paths::from_env()?.socket();

    let (tx, rx) = watch::channel(false);
    let watcher = match wayland::connect(Duration::from_secs(args.idle_after), tx) {
        Ok(watcher) => watcher,
        Err(wayland::SetupError::NoCompositor(e)) => return Err(e),
        Err(wayland::SetupError::Unsupported(why)) => {
            // Not a failure to retry: the same compositor will say the same thing next time.
            tracing::warn!("{why}; nothing to signal, the daemon keeps relying on its clients");
            return Ok(());
        }
    };
    std::thread::Builder::new()
        .name("cortex-presence-wayland".into())
        .spawn(move || {
            let why = watcher.run();
            tracing::error!(error = %format!("{why:#}"), "stopped watching the compositor");
        })
        .context("could not start the compositor thread")?;

    tracing::info!(
        socket = %socket.display(),
        idle_after_secs = args.idle_after,
        "telling the daemon while there is input"
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(signal(rx, &socket))
}

async fn signal(mut idle: watch::Receiver<bool>, socket: &Path) -> Result<()> {
    let mut presence = Presence::default();
    let mut client: Option<Client> = None;
    let mut tick = tokio::time::interval(CHECK_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = idle.changed() => {
                // The sender lives in the compositor thread; it is gone only when that thread is.
                changed.context("the compositor connection ended")?;
                let now_idle = *idle.borrow_and_update();
                tracing::debug!(idle = now_idle, "the compositor reports a change");
                presence.set_idle(now_idle);
            }
            _ = tick.tick() => {}
        }
        let now = Instant::now();
        if presence.due(now) && ping(&mut client, socket).await {
            presence.pinged(now);
        }
    }
}

/// One `activity.ping`. A daemon that is restarting or not started yet is normal: the connection
/// is dropped and made again on the next try, and nothing is logged above debug.
async fn ping(client: &mut Option<Client>, socket: &Path) -> bool {
    if client.is_none() {
        match Client::connect(socket).await {
            Ok(c) => *client = Some(c),
            Err(e) => {
                tracing::debug!(error = %e, "daemon not reachable");
                return false;
            }
        }
    }
    let Some(c) = client.as_mut() else {
        return false;
    };
    match c
        .call::<_, serde_json::Value>(method::ACTIVITY_PING, serde_json::json!({}))
        .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(error = %e, "ping failed; reconnecting on the next try");
            *client = None;
            false
        }
    }
}
