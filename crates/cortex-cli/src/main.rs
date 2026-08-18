//! `cortex` — the command line client.
//!
//! A first-class client, not a debugging aid: it speaks the same JSON-RPC as the GUI and the
//! MCP server, and anything it cannot do is missing from the protocol rather than from here.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cortex_core::paths::Paths;
use cortex_rpc::client::Client;
use cortex_rpc::protocol::{
    method, EpisodesRecentParams, EpisodesRecentResult, ExtractRequeueResult, ExtractStatusResult,
    IndexPathParams, IndexPathResult, NodeGetParams, NodeGetResult, PingResult, ReconcileParams,
    ReconcileResult, SearchExplainParams, SearchExplainResult, SearchParams, SearchResult,
    StatusResult, WatchStatusResult, ZoneSetColorParams, ZonesListResult,
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
        /// Match the last word exactly instead of as a prefix. What a goldset run wants.
        #[arg(long)]
        exact: bool,
        /// Search paths only, skipping the content index.
        #[arg(long)]
        paths_only: bool,
    },
    /// Say why a file ranked where it did for a query — or why it did not appear.
    Explain {
        /// The query, exactly as it would be searched.
        query: String,
        /// The file to explain.
        target: std::path::PathBuf,
    },
    /// Show how far content extraction has got.
    Extract {
        #[command(subcommand)]
        command: Option<ExtractCommand>,
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
    /// Show what the watcher is doing, and whether it is keeping up.
    Watch {
        #[command(subcommand)]
        command: WatchCommand,
    },
    /// List the spatial zones and their colours.
    Zones {
        /// Show every zone. By default only the largest are listed.
        #[arg(short, long)]
        all: bool,
        #[command(subcommand)]
        command: Option<ZoneCommand>,
    },
    /// Compare the index against the filesystem and repair any drift.
    Reconcile {
        /// Reconcile only this path. Defaults to every configured root.
        path: Option<std::path::PathBuf>,
    },
    /// Ask the daemon to stop.
    Shutdown,
}

#[derive(Subcommand, Debug)]
enum ExtractCommand {
    /// Queue every file for extraction again.
    ///
    /// For when the *decision* changed rather than the files: a new content root makes
    /// previously skipped files eligible, and nothing about them changed to trigger a re-read.
    /// Unchanged content is recognised by its hash, so this costs a re-read, not a re-index.
    Requeue,
}

#[derive(Subcommand, Debug)]
enum ZoneCommand {
    /// Pick a zone's colour by hand. It then survives regeneration.
    SetColor {
        id: i64,
        /// Hue in degrees, 0-359.
        hue: u16,
    },
}

#[derive(Subcommand, Debug)]
enum WatchCommand {
    /// Watch coverage, degradation and the last reconciliation.
    Status,
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
                println!(
                    "path index {} paths, {:.1} MiB",
                    result.indexed_paths,
                    result.index_memory_bytes as f64 / (1024.0 * 1024.0)
                );
                if let Some(watcher) = &result.watcher {
                    println!(
                        "watcher    {}",
                        if !watcher.active {
                            "not running".to_string()
                        } else if watcher.degraded {
                            format!("DEGRADED, {} unwatched", watcher.unwatched)
                        } else {
                            format!("{} directories", watcher.watched_dirs)
                        }
                    );
                }
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

        Command::Search { query, limit, exact, paths_only } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                bail!("nothing to search for");
            }
            let result: SearchResult = client
                .call(
                    method::SEARCH,
                    SearchParams {
                        query,
                        mode: if paths_only {
                            cortex_core::model::QueryMode::Locate
                        } else {
                            Default::default()
                        },
                        limit,
                        prefix: Some(!exact),
                    },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else if result.hits.is_empty() {
                println!(
                    "no results ({} ms, {} mode)",
                    result.elapsed_ms,
                    mode_name(result.mode)
                );
            } else {
                for hit in &result.hits {
                    println!("{}", hit.path.display());
                    // The snippet is what makes a content hit legible: without it, a file that
                    // matched on a word inside it looks like a result out of nowhere.
                    if let Some(snippet) = &hit.snippet {
                        println!("    {snippet}");
                    }
                }
                println!(
                    "\n{} result(s) in {} ms, {} mode{}",
                    result.hits.len(),
                    result.elapsed_ms,
                    mode_name(result.mode),
                    if result.partial { ", refining…" } else { "" }
                );
            }
        }

        Command::Explain { query, target } => {
            let path = target
                .canonicalize()
                .unwrap_or(target)
                .display()
                .to_string();
            let result: SearchExplainResult = client
                .call(
                    method::SEARCH_EXPLAIN,
                    SearchExplainParams {
                        query,
                        mode: Default::default(),
                        path: Some(path.clone()),
                        id: None,
                        prefix: None,
                    },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else {
                println!("{path}");
                println!("  query      {:?} ({} mode)", result.query, mode_name(result.mode));
                if let Some(expression) = &result.expression {
                    println!("  lexical    {expression}");
                }
                println!(
                    "  candidates {} from paths, {} from content",
                    result.path_candidates, result.lexical_candidates
                );
                if let Some(score) = &result.score {
                    println!(
                        "  ranks      path {}, lexical {}, vector {}",
                        rank_or_dash(score.path_rank),
                        rank_or_dash(score.lexical_rank),
                        rank_or_dash(score.vector_rank)
                    );
                    println!(
                        "  score      fused {:.6} × salience {:.2} = {:.6}",
                        score.fused, score.salience_boost, score.final_score
                    );
                }
                println!();
                for reason in &result.reasons {
                    println!("  · {reason}");
                }
            }
        }

        Command::Extract { command } => match command {
            Some(ExtractCommand::Requeue) => {
                let result: ExtractRequeueResult = client
                    .call(method::EXTRACT_REQUEUE, serde_json::json!({}))
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    println!("{} file(s) queued for extraction again", result.requeued);
                }
            }
            None => {
                let result: ExtractStatusResult = client
                    .call(method::EXTRACT_STATUS, serde_json::json!({}))
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    print_extract_status(&result);
                }
            }
        },

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
                        println!(
                            "  parent   {}",
                            node.parent_id
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| "none (top of the indexed tree)".into())
                        );
                        println!(
                            "  zone     {}",
                            node.zone_id
                                .map(|z| z.to_string())
                                .unwrap_or_else(|| "none".into())
                        );
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

        Command::Watch { command } => match command {
            WatchCommand::Status => {
                let result: WatchStatusResult =
                    client.call(method::WATCH_STATUS, serde_json::json!({})).await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    print_watch_status(&result);
                }
            }
        },

        Command::Zones { all, command } => match command {
            Some(ZoneCommand::SetColor { id, hue }) => {
                if hue >= 360 {
                    bail!("hue must be between 0 and 359");
                }
                let _: serde_json::Value = client
                    .call(method::ZONE_SET_COLOR, ZoneSetColorParams { id, hue })
                    .await?;
                println!("zone {id} set to hue {hue}");
            }
            None => {
                let result: ZonesListResult =
                    client.call(method::ZONES_LIST, serde_json::json!({})).await?;
                if args.json {
                    // Agents and scripts always get everything; truncation is a courtesy to a
                    // human reading a terminal, not a property of the data.
                    print_json(&result)?;
                } else if result.zones.is_empty() {
                    println!("no zones — configure index.metadata_roots and reconcile");
                } else {
                    let mut zones = result.zones.clone();
                    zones.sort_by(|a, b| b.node_count.cmp(&a.node_count));
                    let total = zones.len();
                    let shown = if all { total } else { total.min(20) };

                    println!("{:>5}  {:>5}  {:<10} {:>9}  root", "id", "hue", "icon", "nodes");
                    for zone in zones.iter().take(shown) {
                        println!(
                            "{:>5}  {:>4}{}  {:<10} {:>9}  {}",
                            zone.id,
                            zone.color_hue,
                            if zone.user_override { "*" } else { " " },
                            zone.icon.as_deref().unwrap_or("-"),
                            zone.node_count,
                            zone.root_path,
                        );
                    }
                    if shown < total {
                        println!("\n… and {} more. Pass --all to see them.", total - shown);
                    }
                    if zones.iter().any(|z| z.user_override) {
                        println!("\n* colour chosen by hand; regeneration leaves it alone");
                    }
                }
            }
        },

        Command::Reconcile { path } => {
            let path = match path {
                Some(p) => Some(
                    p.canonicalize()
                        .with_context(|| format!("no such path: {}", p.display()))?
                        .display()
                        .to_string(),
                ),
                None => None,
            };

            let result: ReconcileResult =
                client.call(method::RECONCILE, ReconcileParams { path }).await?;
            if args.json {
                print_json(&result)?;
            } else {
                let s = &result.summary;
                if s.added == 0 && s.updated == 0 && s.removed == 0 && s.renamed == 0 {
                    println!(
                        "index is in sync ({} entries checked, {} ms)",
                        s.unchanged, result.elapsed_ms
                    );
                } else {
                    println!(
                        "repaired drift: +{} added, {} updated, -{} removed, {} renamed \
                         ({} unchanged, {} ms)",
                        s.added, s.updated, s.removed, s.renamed, s.unchanged, result.elapsed_ms
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

fn print_extract_status(status: &ExtractStatusResult) {
    if !status.enabled {
        // Rule R3 is opt-in, so an empty content root list is the default rather than a fault.
        // Saying so plainly is the difference between "off" and "broken".
        println!("extraction is off — no index.content_roots configured, so nothing is read");
        println!("metadata indexing is unaffected; paths remain searchable");
        return;
    }

    for root in &status.content_roots {
        println!("root       {root}");
    }
    println!(
        "queue      {} pending, {} done, {} skipped, {} failed",
        status.pending, status.done, status.skipped, status.failed
    );
    println!("chunks     {} in the lexical index", status.chunks);
    println!(
        "this run   {} extracted, {} unchanged, {} chunks written, {} reused, {} deleted",
        status.files_extracted,
        status.files_unchanged,
        status.chunks_written,
        status.chunks_reused,
        status.chunks_deleted
    );
    println!(
        "budget     {:.0}% of one core ({}), busy {:.1}s so far",
        status.cpu_share * 100.0,
        if status.user_active { "user active" } else { "idle" },
        status.busy_ms as f64 / 1000.0
    );
    if status.paused {
        println!("state      PAUSED");
    }
}

fn mode_name(mode: cortex_core::model::QueryMode) -> &'static str {
    use cortex_core::model::QueryMode;
    match mode {
        QueryMode::Auto => "auto",
        QueryMode::Locate => "locate",
        QueryMode::Recall => "recall",
        QueryMode::Temporal => "temporal",
        QueryMode::Structural => "structural",
        QueryMode::Deep => "deep",
    }
}

fn rank_or_dash(rank: Option<u32>) -> String {
    rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into())
}

fn print_watch_status(status: &WatchStatusResult) {
    if !status.active {
        println!("watcher   not running — the index relies on periodic reconciliation alone");
    } else if status.degraded {
        // Degradation must be loud. An unwatched subtree looks exactly like a subtree where
        // nothing ever changes, and that is the failure mode users never diagnose themselves.
        println!(
            "watcher   DEGRADED — {} of {} watches used, {} directories unwatched",
            status.watched_dirs, status.budget, status.unwatched
        );
        println!("          unwatched paths update only on reconcile");
    } else {
        println!(
            "watcher   healthy — {} of {} watches used",
            status.watched_dirs, status.budget
        );
    }

    if status.paused {
        println!("indexing  PAUSED");
    }
    if status.overflows > 0 {
        println!(
            "overflows {} — the kernel dropped events that many times; each triggered a reconcile",
            status.overflows
        );
    }

    for root in &status.roots {
        println!("root      {root}");
    }

    match &status.last_reconcile {
        None => println!("reconcile has not run yet"),
        Some(r) => println!(
            "reconcile +{} added, {} updated, -{} removed, {} renamed, {} unchanged, {} skipped",
            r.added, r.updated, r.removed, r.renamed, r.unchanged, r.skipped
        ),
    }
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
