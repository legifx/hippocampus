//! `cortex` — the command line client.
//!
//! A first-class client, not a debugging aid: it speaks the same JSON-RPC as the GUI and the
//! MCP server, and anything it cannot do is missing from the protocol rather than from here.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cortex_core::paths::Paths;
use cortex_rpc::client::Client;
use cortex_rpc::protocol::{
    method, EpisodesRecentParams, EpisodesRecentResult, IndexPathParams, IndexPathResult,
    NodeGetParams, NodeGetResult, PingResult, SearchParams, SearchResult, StatusResult,
};

#[derive(Parser, Debug)]
#[command(name = "cortex", version, about = "Talk to the Cortex memory daemon")]
struct Args {
    /// Use an instance rooted at this directory instead of the standard locations.
    #[arg(long, value_name = "DIR", global = true)]
    root: Option<std::path::PathBuf>,

    /// Print raw JSON instead of formatted output. For scripts and for agents.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Check that the daemon is alive.
    Ping,
    /// Show what the daemon knows and how much of it.
    Status,
    /// Search the index.
    Search {
        query: Vec<String>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Index a path. Metadata only — content extraction arrives in M2.
    Index {
        path: std::path::PathBuf,
        /// Walk the whole subtree.
        #[arg(short, long, default_value_t = true)]
        recursive: bool,
    },
    /// Show a single node.
    Node {
        /// A path, or a numeric node id.
        target: String,
    },
    /// Show the most recent episodes.
    Episodes {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Ask the daemon to stop.
    Shutdown,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    let paths = match &args.root {
        Some(root) => Paths::rooted_at(root),
        None => Paths::from_env()?,
    };
    let socket = paths.socket();

    let mut client = Client::connect(&socket).await.with_context(|| {
        format!(
            "no daemon listening on {}. Start it with `cortexd`.",
            socket.display()
        )
    })?;

    match args.command {
        Command::Ping => {
            let result: PingResult = client.call(method::PING, serde_json::json!({})).await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!("pong (cortexd {})", result.version);
            }
        }

        Command::Status => {
            let result: StatusResult = client.call(method::STATUS, serde_json::json!({})).await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!("cortexd {}  schema {}", result.version, result.schema_version);
                println!("database   {}", result.database);
                println!("uptime     {}", format_duration(result.uptime_secs));
                println!(
                    "indexed    {} files, {} directories",
                    result.files, result.dirs
                );
                println!(
                    "memory     {} chunks, {} embeddings, {} episodes",
                    result.chunks, result.embeddings, result.episodes
                );
                println!("outbox     {} pending", result.outbox_pending);
                println!(
                    "agents     {}",
                    if result.agent_surface_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
        }

        Command::Search { query, limit } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                bail!("nothing to search for");
            }
            let result: SearchResult = client
                .call(
                    method::SEARCH,
                    SearchParams {
                        query,
                        mode: Default::default(),
                        limit,
                    },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else if result.hits.is_empty() {
                println!("no results ({} ms)", result.elapsed_ms);
            } else {
                for hit in &result.hits {
                    println!("{}", hit.path.display());
                }
                println!(
                    "\n{} result(s) in {} ms{}",
                    result.hits.len(),
                    result.elapsed_ms,
                    if result.partial { ", refining…" } else { "" }
                );
            }
        }

        Command::Index { path, recursive } => {
            let path = path
                .canonicalize()
                .with_context(|| format!("no such path: {}", path.display()))?;
            let result: IndexPathResult = client
                .call(
                    method::INDEX_PATH,
                    IndexPathParams {
                        path: path.display().to_string(),
                        recursive,
                    },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else {
                println!(
                    "indexed {} entries, skipped {}, in {} ms",
                    result.indexed, result.skipped, result.elapsed_ms
                );
            }
        }

        Command::Node { target } => {
            // A bare number is a node id; anything else is a path. Ambiguity is not a real
            // concern — a file named "42" still resolves once it is given as "./42".
            let params = match target.parse::<i64>() {
                Ok(id) => NodeGetParams {
                    id: Some(cortex_core::ids::NodeId(id)),
                    path: None,
                },
                Err(_) => NodeGetParams {
                    id: None,
                    path: Some(
                        std::path::PathBuf::from(&target)
                            .canonicalize()
                            .unwrap_or_else(|_| target.clone().into())
                            .display()
                            .to_string(),
                    ),
                },
            };

            let result: NodeGetResult = client.call(method::NODE_GET, params).await?;
            match result.node {
                None => {
                    if args.json {
                        print_json(&result)?;
                    } else {
                        println!("not indexed");
                    }
                }
                Some(node) => {
                    if args.json {
                        print_json(&node)?;
                    } else {
                        println!("{}", node.path.display());
                        println!("  id       {}", node.id);
                        println!("  kind     {}", node.kind.as_str());
                        println!("  size     {} bytes", node.size);
                        println!("  tier     {}", node.tier.as_str());
                        println!("  salience {:.3}{}", node.salience, if node.pinned { " (pinned)" } else { "" });
                    }
                }
            }
        }

        Command::Episodes { limit } => {
            let result: EpisodesRecentResult = client
                .call(
                    method::EPISODES_RECENT,
                    EpisodesRecentParams { limit: Some(limit) },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for e in &result.episodes {
                    println!(
                        "{}  {:<12} {:<8} {}",
                        e.ts,
                        e.actor.as_str(),
                        e.action.as_str(),
                        e.node_id.map(|n| n.to_string()).unwrap_or_default()
                    );
                }
            }
        }

        Command::Shutdown => {
            let _: serde_json::Value = client.call(method::SHUTDOWN, serde_json::json!({})).await?;
            println!("stopping");
        }
    }

    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn format_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}
