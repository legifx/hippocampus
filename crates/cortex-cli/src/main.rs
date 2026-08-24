//! `cortex` — the command line client.
//!
//! A first-class client, not a debugging aid: it speaks the same JSON-RPC as the GUI and the
//! MCP server, and anything it cannot do is missing from the protocol rather than from here.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cortex_core::memory::ContextPack;
use cortex_core::paths::Paths;
use cortex_core::tools::{
    AnnotateParams, AnnotateResult, AppsForPathParams, AppsForPathResult, AppsSetDefaultParams,
    AppsSetDefaultResult, DiffParams, DiffResult, FsListParams, FsListResult, FsMountParams,
    FsMountResult, FsUnmountParams, FsUnmountResult,
    FsMoveParams, FsPlacesParams, FsPlacesResult, FsReadParams, FsReadResult, FsStatParams,
    FsStatResult, FsTrashParams,
    FsWriteParams, LinkParams, LinkResult, Mutation, NeighborsParams, NeighborsResult, PinParams,
    PinResult, Target, TimelineParams, TimelineResult, UndoParams, UndoResult, WorkingSetResult,
    WriteMode,
};
use cortex_core::tools::{
    ChangedResult, ContextParams, ForgetMemoryParams, MemoriesParams, MemoriesResult,
    OutcomeParams, OutcomeResult, RememberParams, RememberResult, ScopeRequest, SessionsParams,
    SessionsResult, SupersedeParams,
};
use cortex_rpc::client::Client;
use cortex_rpc::protocol::{
    method, AgentSetEnabledParams, AgentStatusResult, AuditRecentParams, AuditRecentResult,
    EpisodesRecentParams, EpisodesRecentResult, ExtractRequeueResult, ExtractStatusResult,
    IdentifyParams, IdentifyResult, IndexSetPausedParams, IndexSetPausedResult, RecallParams,
    RecallResult,
    CanonicalDumpResult, IndexPathParams, IndexPathResult, ModelReloadResult, ModelStatusResult,
    NodeGetParams, NodeGetResult, PingResult, RebuildParams, RebuildResult, ReconcileParams,
    ReconcileResult, SearchExplainParams, SearchUpdateParams,
    SearchExplainResult, SearchParams, SearchResult, StatusResult, WatchStatusResult,
    ZoneSetColorParams, ZonesListResult,
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

    /// Act as this actor. Everything done on this connection is attributed to it in the
    /// episode log and the audit log, and it is held to the agent policy.
    #[arg(long = "as", value_name = "ACTOR", global = true)]
    actor: Option<String>,

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
        /// Print the first answer and do not wait for the reranked one.
        #[arg(long)]
        no_rerank: bool,
    },
    /// Say why a file ranked where it did for a query — or why it did not appear.
    ///
    /// Also takes a node id, which is the only way to name a memory: a memory has no path,
    /// and explaining one says what it has earned rather than where it ranked.
    Explain {
        /// The query, exactly as it would be searched. Ignored for a memory.
        query: String,
        /// A path, or a numeric node id.
        target: String,
    },
    /// Show how far content extraction has got.
    Extract {
        #[command(subcommand)]
        command: Option<ExtractCommand>,
    },
    /// The embedding model: what it is, whether it is there, and how to get it.
    Model {
        #[command(subcommand)]
        command: Option<ModelCommand>,
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
    /// Throw the index away and derive it again from the filesystem.
    ///
    /// Everything except the episode log is derived, so this is a supported operation rather
    /// than a repair of last resort (rule R10). Vectors are keyed by the content they describe
    /// and survive it, so the model is not asked for anything it has already computed.
    Rebuild {
        /// Drop the vectors too. Costs a full re-embedding pass.
        #[arg(long)]
        drop_embeddings: bool,
        /// Wait until extraction has caught up again, printing progress.
        #[arg(long, default_value_t = true)]
        wait: bool,
    },
    /// Print the canonical dump of the index: one stable line per path.
    ///
    /// Two dumps either side of a rebuild must be identical. That is the test rule R10 asks for,
    /// and `cortex dump > before && cortex rebuild && cortex dump > after && diff` is it.
    Dump,
    /// Compare the index against the filesystem and repair any drift.
    Reconcile {
        /// Reconcile only this path. Defaults to every configured root.
        path: Option<std::path::PathBuf>,
    },
    /// What the memory knows about a task, budgeted for a context window.
    ///
    /// The tool an agent reaches for first, on the command line so that a person can see
    /// exactly what an agent would have been handed.
    Context {
        /// What you are about to do, in a sentence.
        task: Vec<String>,
        #[arg(long, default_value_t = 2048)]
        budget: usize,
        /// Restrict to these kinds. Repeatable.
        #[arg(long = "kind")]
        kinds: Vec<String>,
        /// Include memories that have been superseded.
        #[arg(long)]
        history: bool,
        /// Print the block exactly as an agent would receive it, and nothing else.
        #[arg(long)]
        raw: bool,
    },
    /// Write something into the memory that is not in a file.
    Remember {
        text: String,
        #[arg(long, default_value = "observation")]
        kind: String,
        /// session | agent | project | global.
        ///
        /// The default follows who is speaking: `agent` when `--as` names one, `global`
        /// otherwise. A person typing this has no agent scope to write into; an agent does,
        /// and defaulting *its* writes to global would widen them silently, which is the one
        /// thing the scope rules exist to prevent.
        #[arg(long)]
        scope: Option<String>,
        /// The file this is about.
        #[arg(long)]
        about: Option<std::path::PathBuf>,
        #[arg(long)]
        confidence: Option<f64>,
        /// Memory ids this replaces. Repeatable.
        #[arg(long = "supersedes")]
        supersedes: Vec<i64>,
    },
    /// List memories, with their scope, provenance and how useful they have proved.
    Memories {
        #[command(subcommand)]
        command: Option<MemoryCommand>,
        /// Repeatable.
        #[arg(long = "kind")]
        kinds: Vec<String>,
        #[arg(long)]
        about: Option<std::path::PathBuf>,
        /// Include superseded memories.
        #[arg(long)]
        history: bool,
        /// Every scope in the index, not only the ones this caller can see.
        #[arg(long)]
        all: bool,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// The agent sessions this daemon has seen.
    Sessions {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Attach a note to a file without touching the file.
    Annotate {
        /// A path, or a numeric node id.
        target: String,
        note: String,
        /// Repeatable.
        #[arg(short, long)]
        tag: Vec<String>,
    },
    /// Mark a file as never to be forgotten.
    Pin {
        target: String,
        /// Undo the pin and reset salience instead.
        #[arg(long)]
        forget: bool,
    },
    /// Assert that two nodes belong together.
    Link {
        a: String,
        b: String,
        #[arg(long, default_value = "similar_to")]
        edge: String,
        /// Remove the edge instead.
        #[arg(long)]
        remove: bool,
    },
    /// Show what a node is connected to.
    Neighbors {
        target: String,
        #[arg(short, long, default_value_t = 1)]
        depth: u32,
        /// Only these edge types. Repeatable.
        #[arg(long = "edge")]
        edges: Vec<String>,
    },
    /// A fragment in, its context out — search plus the associations around what it found.
    Recall {
        cue: Vec<String>,
        #[arg(short, long, default_value_t = 1)]
        depth: u32,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Episodes in a time range.
    Timeline {
        /// Unix seconds, or a number of hours back written as e.g. `24h`.
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        action: Option<String>,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// What has been touched lately.
    WorkingSet,
    /// Run a consolidation pass: replay the episode log, form associations, rescore, retier.
    Consolidate {
        /// Compute the whole pass and print the report, but write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// The last consolidation report, without running one.
    Report,
    /// One snapshot of the brain as JSON — nodes, edges, zones, recent episodes.
    ///
    /// This is what the Agent View draws. Written to a file rather than to standard output by
    /// default, because a snapshot of a real index is tens of megabytes and a terminal is not
    /// where it belongs.
    Graph {
        /// Where to write it. `-` writes to standard output.
        #[arg(long, default_value = "graph.json")]
        out: String,
        /// How many nodes, chosen by salience. Ancestors are added on top.
        ///
        /// The daemon caps this at 15 000: beyond that the answer no longer fits the
        /// protocol's line limit. Ask for more and you get 15 000, and the count is printed.
        #[arg(long, default_value_t = 3000)]
        limit: usize,
        /// How many recent episodes to include, for the animation.
        #[arg(long, default_value_t = 500)]
        events: usize,
        /// Only one zone, by id. `cortex zones` lists them.
        #[arg(long)]
        zone: Option<i64>,
    },
    /// What changed since an episode id. For an agent that should not poll.
    Diff {
        #[arg(long)]
        since: Option<i64>,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// The places worth starting from: your folders, the drives, and the filesystem.
    Places,
    /// Mount a drive that is plugged in but not mounted.
    Mount {
        /// The device node, as `cortex places` prints it: `/dev/sdb1`.
        device: std::path::PathBuf,
        /// Let polkit ask for a password. An internal disk needs an administrator, and without
        /// this the attempt is refused rather than left waiting on a dialog.
        #[arg(long)]
        ask: bool,
    },
    /// Unmount a drive, and optionally power it off so it can be unplugged.
    Unmount {
        device: std::path::PathBuf,
        /// Cut the power afterwards. Refused for a disk that cannot be unplugged.
        #[arg(long)]
        eject: bool,
    },
    /// What opens this file: its type, and the applications that claim it.
    ///
    /// The same answer the window shows. When the two disagree, one of them is reading the
    /// association files wrongly, and this is the cheap way to find out which.
    Apps {
        path: std::path::PathBuf,
        /// Every installed application, not only the ones that claim the type.
        #[arg(long)]
        all: bool,
        /// Make this application the default for the file's type, for the whole desktop.
        #[arg(long, value_name = "DESKTOP_ID")]
        set_default: Option<String>,
    },
    /// List a directory, the way an agent sees it.
    Ls {
        path: std::path::PathBuf,
        /// How far to descend.
        #[arg(short, long, default_value_t = 1)]
        depth: u32,
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Print a file's contents through the surface, deny-list and all.
    Cat {
        path: std::path::PathBuf,
        /// A byte range, as `start:end`.
        #[arg(long, value_name = "START:END")]
        range: Option<String>,
    },
    /// What the surface knows about a path, including whether it may be written.
    Stat { path: std::path::PathBuf },
    /// Write a file. Gated: plans first, applies second (rule R6).
    Write {
        path: std::path::PathBuf,
        /// The content. Without it, standard input is read.
        content: Option<String>,
        #[arg(long, value_parser = ["create", "overwrite", "append"], default_value = "create")]
        mode: String,
        /// Apply the plan instead of printing it.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Move a file. Gated.
    Mv {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Move a file to the trash, where `cortex undo` can still reach it. Gated.
    Trash {
        path: std::path::PathBuf,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Reverse recent mutations, newest first.
    Undo {
        /// How many to reverse.
        #[arg(short, long, default_value_t = 1)]
        last: usize,
        /// Reverse everything one plan did.
        #[arg(long)]
        plan: Option<String>,
        /// Say what would be reversed, and reverse nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop watching and indexing. Survives a restart (rule R7).
    ///
    /// The index keeps answering; it just stops learning. `cortex resume` starts it again, and
    /// a reconcile afterwards catches up on everything that changed meanwhile.
    Pause,
    /// Start indexing again after `cortex pause`.
    Resume,
    /// The Agent Surface: whether agents are being served at all (rule R7).
    #[command(name = "agent-surface")]
    AgentSurface {
        #[command(subcommand)]
        command: Option<AgentSurfaceCommand>,
    },
    /// Show what agents have asked for, and what was refused (rule R8).
    Audit {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        /// Only one actor, e.g. `agent:claude`.
        #[arg(long)]
        actor: Option<String>,
        /// Only the refusals.
        #[arg(long)]
        refused: bool,
    },
    /// Ask the daemon to stop.
    Shutdown,
}

#[derive(Subcommand, Debug)]
enum AgentSurfaceCommand {
    /// Serve agent tools.
    On,
    /// Refuse every agent tool, and stop accepting MCP connections.
    Off,
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
enum ModelCommand {
    /// Download the embedding model and the ONNX runtime.
    ///
    /// This is the one command in the project that opens a network connection (rule R5). It
    /// says what it is about to fetch and from where before it does anything.
    Fetch {
        /// Fetch without asking.
        #[arg(short, long)]
        yes: bool,
        /// Only the runtime library, not the model.
        #[arg(long)]
        runtime_only: bool,
        /// Also fetch the cross-encoder that reranks results (488 MB).
        #[arg(long)]
        reranker: bool,
        /// Fetch the CUDA build of the runtime (191 MB instead of 11 MB).
        ///
        /// It needs a CUDA runtime and cuDNN on the machine — a driver alone is not enough. With
        /// them it embeds roughly forty times faster; without them it falls back to the CPU and
        /// says so, so fetching it is never wrong, only sometimes pointless.
        #[arg(long)]
        gpu: bool,
    },
    /// Load a model that was fetched while the daemon was already running.
    Reload,
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
enum MemoryCommand {
    /// Delete a memory. For things that were never true; supersede what merely stopped being.
    Forget { memory_id: i64 },
    /// Mark one memory as replaced by another.
    Supersede { old: i64, new: i64 },
    /// Say which memories of a pack helped. A human's report outweighs an agent's.
    Outcome {
        pack_id: String,
        #[arg(long = "helped")]
        helped: Vec<i64>,
        #[arg(long = "noise")]
        noise: Vec<i64>,
    },
}

#[derive(Subcommand, Debug)]
enum WatchCommand {
    /// Watch coverage, degradation and the last reconciliation.
    Status,
}

/// One line of a memory, for a listing. Memories are prose and some of them are paragraphs.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > 96 {
        let cut: String = line.chars().take(93).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
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

    // Provisioning is a user action against the filesystem, not a daemon operation: it must work
    // before there is a daemon, and it must not tie up the socket for the minutes a download
    // takes. The daemon is told afterwards, if one happens to be listening.
    if let Command::Model {
        command: Some(ModelCommand::Fetch { yes, runtime_only, gpu, reranker }),
    } = &args.command
    {
        return fetch_model(&paths, *yes, *runtime_only, *gpu, *reranker, args.json).await;
    }

    let mut client = Client::connect(&socket).await.with_context(|| {
        format!(
            "no daemon listening on {}. Start it with `cortexd`.",
            socket.display()
        )
    })?;

    // Naming yourself is one round trip and it is what makes the audit log worth reading.
    if let Some(actor) = &args.actor {
        let identified: IdentifyResult = client
            .call(method::IDENTIFY, IdentifyParams { actor: actor.clone(), session: None })
            .await?;
        if !identified.agent_surface_enabled {
            eprintln!("warning: the Agent Surface is switched off; agent tools will refuse");
        }
    }

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

        Command::Search { query, limit, exact, paths_only, no_rerank } => {
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
                        rerank: Some(!no_rerank),
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

            // The first answer is complete; this is the better one arriving afterwards (D7).
            // Waiting is the client's choice, and the timeout means a reranker that is slow or
            // absent costs a moment rather than the result.
            if result.partial && !args.json {
                let update = client
                    .next_notification(
                        method::SEARCH_UPDATE,
                        std::time::Duration::from_millis(2000),
                    )
                    .await?;
                if let Some(params) = update {
                    let update: SearchUpdateParams = serde_json::from_value(params)?;
                    println!("\nreranked by {} in {} ms:", update.model, update.elapsed_ms);
                    for hit in update.hits.iter().take(limit.unwrap_or(10)) {
                        println!(
                            "{}  [{}]",
                            hit.path.display(),
                            hit.score
                                .rerank
                                .map(|r| format!("{r:.2}"))
                                .unwrap_or_else(|| "-".into())
                        );
                    }
                }
            }
        }

        Command::Explain { query, target } => {
            let (path, id) = match parse_target(&target) {
                Target::Node(id) => (None, Some(cortex_core::ids::NodeId(id))),
                Target::Path(path) => (
                    Some(
                        path.canonicalize()
                            .unwrap_or(path)
                            .display()
                            .to_string(),
                    ),
                    None,
                ),
            };
            let result: SearchExplainResult = client
                .call(
                    method::SEARCH_EXPLAIN,
                    SearchExplainParams {
                        query,
                        mode: Default::default(),
                        path: path.clone(),
                        id,
                        prefix: None,
                    },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else if let Some(memory) = &result.memory {
                // A memory does not compete with files for a place in a result list, so the
                // ranking lines below would all be "did not appear" and would say nothing.
                println!("memory {}", memory.node_id);
                println!("  {}", memory.text);
                for reason in &result.reasons {
                    println!("  · {reason}");
                }
            } else {
                let path = path.unwrap_or_else(|| match id {
                    Some(id) => format!("node {id}"),
                    None => "?".to_string(),
                });
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

        Command::Model { command } => match command {
            Some(ModelCommand::Fetch { .. }) => unreachable!("handled before connecting"),
            Some(ModelCommand::Reload) => {
                let result: ModelReloadResult = client
                    .call(method::MODEL_RELOAD, serde_json::json!({}))
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else if result.loaded {
                    println!("loaded: {}", result.backend.as_deref().unwrap_or("unknown backend"));
                } else {
                    println!(
                        "not loaded: {}",
                        result.note.as_deref().unwrap_or("no reason given")
                    );
                }
            }
            None => {
                let result: ModelStatusResult = client
                    .call(method::MODEL_STATUS, serde_json::json!({}))
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    print_model_status(&result);
                }
            }
        },

        Command::Dump => {
            let result: CanonicalDumpResult = client
                .call(method::CANONICAL_DUMP, serde_json::json!({}))
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                print!("{}", result.dump);
            }
        }

        Command::Rebuild { drop_embeddings, wait } => {
            let result: RebuildResult = client
                .call(method::REBUILD_INDEX, RebuildParams { drop_embeddings })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!(
                    "re-derived {} entries in {} ms, {} vectors kept",
                    result.indexed, result.elapsed_ms, result.embeddings_kept
                );
                println!("{} files queued for extraction", result.extraction_pending);
            }

            if wait && !args.json {
                // A rebuild returns as soon as the tree has been walked; extraction and embedding
                // run in the background under their own budgets. Waiting here is what makes
                // "rebuild, then compare the dumps" a single sitting.
                loop {
                    let status: ExtractStatusResult = client
                        .call(method::EXTRACT_STATUS, serde_json::json!({}))
                        .await?;
                    if status.pending == 0 {
                        println!("extraction complete: {} chunks", status.chunks);
                        break;
                    }
                    // Extraction that is switched off never drains its queue, and waiting for
                    // it to is waiting forever. Found by running `rebuild` against an index
                    // with no content root — which is the default configuration, so this is the
                    // first thing a new user would have hit.
                    if !status.enabled {
                        println!(
                            "\r{} file(s) are queued but no content root is configured, so \
                             nothing will extract them   ",
                            status.pending
                        );
                        break;
                    }
                    print!("\r  {} files left to extract   ", status.pending);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }

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

        Command::Context { task, budget, kinds, history, raw } => {
            let task = task.join(" ");
            let pack: ContextPack = client
                .call(
                    method::BRAIN_CONTEXT,
                    ContextParams {
                        task: task.clone(),
                        budget_tokens: Some(budget),
                        kinds,
                        include_history: Some(history),
                        ..Default::default()
                    },
                )
                .await?;
            if args.json {
                print_json(&pack)?;
            } else if raw {
                print!("{}", pack.rendered);
            } else {
                if pack.items.is_empty() {
                    println!("nothing in memory bears on {task:?}");
                }
                print!("{}", pack.rendered);
                println!();
                println!(
                    "{} items · ~{} of {} tokens · {} dropped · pack {}",
                    pack.items.len(),
                    pack.used_tokens,
                    pack.budget_tokens,
                    pack.dropped,
                    pack.pack_id
                );
                for note in &pack.notes {
                    println!("note: {note}");
                }
            }
        }

        Command::Remember { text, kind, scope, about, confidence, supersedes } => {
            let scope = match scope.as_deref() {
                Some("session") => ScopeRequest::Session,
                Some("agent") => ScopeRequest::Agent,
                Some("project") => ScopeRequest::Project,
                Some("global") => ScopeRequest::Global,
                Some(other) => anyhow::bail!(
                    "{other} is not a scope; it is one of session, agent, project, global"
                ),
                // Whoever is speaking gets the narrowest scope that can hold it.
                None if args.actor.is_some() => ScopeRequest::Agent,
                None => ScopeRequest::Global,
            };
            let result: RememberResult = client
                .call(
                    method::BRAIN_REMEMBER,
                    RememberParams {
                        text,
                        kind: Some(kind),
                        scope,
                        session_id: None,
                        about,
                        confidence,
                        supersedes,
                        ttl_secs: None,
                    },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else if result.deduplicated {
                println!(
                    "memory {} in {} already said exactly that",
                    result.memory_id, result.scope
                );
            } else {
                println!("memory {} in {}", result.memory_id, result.scope);
                for id in &result.superseded {
                    println!("  superseded {id}");
                }
                for duplicate in &result.possible_duplicates {
                    println!(
                        "  possibly the same as {}: {}",
                        duplicate.node_id,
                        first_line(&duplicate.text)
                    );
                }
                if !result.possible_duplicates.is_empty() {
                    println!("  (reported, not acted on — supersede them if they are)");
                }
            }
        }

        Command::Memories { command, kinds, about, history, all, limit } => match command {
            Some(MemoryCommand::Forget { memory_id }) => {
                let result: ChangedResult = client
                    .call(method::BRAIN_FORGET_MEMORY, ForgetMemoryParams { memory_id })
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else if result.changed {
                    println!("memory {memory_id} forgotten");
                } else {
                    println!("no memory {memory_id}");
                }
            }
            Some(MemoryCommand::Supersede { old, new }) => {
                let result: ChangedResult = client
                    .call(method::BRAIN_SUPERSEDE, SupersedeParams { old, new })
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else if result.changed {
                    println!("memory {old} superseded by {new}");
                } else {
                    println!("nothing changed; check both ids are memories");
                }
            }
            Some(MemoryCommand::Outcome { pack_id, helped, noise }) => {
                let result: OutcomeResult = client
                    .call(
                        method::BRAIN_OUTCOME,
                        OutcomeParams { pack_id, helped, noise, note: None },
                    )
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    println!(
                        "{} helped, {} noise, {} ignored, as {}",
                        result.helped, result.noise, result.ignored, result.reporter
                    );
                }
            }
            None => {
                let result: MemoriesResult = client
                    .call(
                        method::BRAIN_MEMORIES,
                        MemoriesParams {
                            kinds,
                            about,
                            include_superseded: history,
                            everything: all,
                            limit: Some(limit),
                            ..Default::default()
                        },
                    )
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    println!("{}", result.basis);
                    for memory in &result.memories {
                        let mark = if memory.superseded_by.is_some() {
                            "×"
                        } else if memory.human_confirmed {
                            "✓"
                        } else {
                            " "
                        };
                        println!(
                            "{mark} {:>6}  {:<11} {:<22} {}",
                            memory.node_id,
                            memory.kind.as_str(),
                            memory.scope.as_str(),
                            first_line(&memory.text)
                        );
                        println!(
                            "         by {} · useful {:+.2} · helped {} · noise {} · shown {}",
                            memory.actor,
                            memory.usefulness,
                            memory.helped_count,
                            memory.noise_count,
                            memory.retrieved_count
                        );
                    }
                    if result.memories.is_empty() {
                        println!("(nothing)");
                    }
                }
            }
        },

        Command::Sessions { limit } => {
            let result: SessionsResult = client
                .call(method::AGENT_SESSIONS, SessionsParams { limit: Some(limit) })
                .await?;
            let sessions = &result.sessions;
            if args.json {
                print_json(&result)?;
            } else {
                println!("{}", result.basis);
                for session in sessions {
                    println!(
                        "{:<28} {:<12} {:>5} calls {:>4} refused {:>4} memories  {}",
                        session.id,
                        session.name,
                        session.calls,
                        session.refusals,
                        session.memories,
                        session.task.as_deref().unwrap_or("")
                    );
                }
                if sessions.is_empty() {
                    println!("no agent has opened a session on this daemon");
                }
            }
        }

        Command::Annotate { target, note, tag } => {
            let result: AnnotateResult = client
                .call(
                    method::BRAIN_ANNOTATE,
                    AnnotateParams { target: parse_target(&target), tags: tag, note },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!(
                    "annotation {} on {}",
                    result.annotation_id,
                    result
                        .target_path
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| format!("node {}", result.target_id))
                );
            }
        }

        Command::Pin { target, forget } => {
            let result: PinResult = client
                .call(
                    if forget { method::BRAIN_FORGET } else { method::BRAIN_PIN },
                    PinParams { target: parse_target(&target) },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!("node {} — {}", result.target_id, result.note);
            }
        }

        Command::Link { a, b, edge, remove } => {
            let result: LinkResult = client
                .call(
                    method::BRAIN_LINK,
                    LinkParams {
                        a: parse_target(&a),
                        b: parse_target(&b),
                        edge: Some(edge),
                        remove,
                    },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!(
                    "{} {} --{}--> {}",
                    if result.removed { "unlinked" } else { "linked" },
                    result.a_id,
                    result.edge,
                    result.b_id
                );
            }
        }

        Command::Neighbors { target, depth, edges } => {
            let result: NeighborsResult = client
                .call(
                    method::BRAIN_NEIGHBORS,
                    NeighborsParams {
                        target: parse_target(&target),
                        edge_types: edges,
                        depth: Some(depth),
                        limit: None,
                    },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for annotation in &result.annotations {
                    println!("note   [{}] {}", annotation.tags.join(", "), annotation.note);
                }
                for neighbor in &result.neighbors {
                    println!(
                        "{:<12} {:<10} {}",
                        neighbor.edge.as_str(),
                        format!("depth {}", neighbor.depth),
                        neighbor
                            .note
                            .clone()
                            .or_else(|| neighbor.path.as_ref().map(|p| p.display().to_string()))
                            .unwrap_or_else(|| format!("node {}", neighbor.node_id))
                    );
                }
                if result.neighbors.is_empty() && result.annotations.is_empty() {
                    println!("nothing is connected to this yet");
                }
            }
        }

        Command::Recall { cue, depth, limit } => {
            let cue = cue.join(" ");
            if cue.trim().is_empty() {
                bail!("nothing to recall");
            }
            let result: RecallResult = client
                .call(method::BRAIN_RECALL, RecallParams { cue, depth: Some(depth), limit })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for hit in &result.hits {
                    println!("{}", hit.path.display());
                    if let Some(snippet) = &hit.snippet {
                        println!("    {snippet}");
                    }
                }
                if !result.memories.is_empty() {
                    println!("\nwhat is known about it and is not in a file:");
                    for memory in &result.memories {
                        println!(
                            "  {:<11} {}",
                            memory.kind.as_str(),
                            first_line(&memory.text)
                        );
                        println!("              {} · {}", memory.actor, memory.scope.as_str());
                    }
                }
                if !result.associations.is_empty() {
                    println!("\nbrought back with them:");
                    for a in &result.associations {
                        println!(
                            "  {:<12} {}  (via {})",
                            a.edge,
                            a.note.clone().unwrap_or_else(|| a
                                .path
                                .clone()
                                .unwrap_or_else(|| format!("node {}", a.node_id))),
                            a.via
                        );
                    }
                }
                println!(
                    "\n{} hit(s), {} association(s) in {} ms",
                    result.hits.len(),
                    result.associations.len(),
                    result.elapsed_ms
                );
            }
        }

        Command::Timeline { from, to, actor, action, limit } => {
            let result: TimelineResult = client
                .call(
                    method::BRAIN_TIMELINE,
                    TimelineParams {
                        from: from.as_deref().map(parse_time).transpose()?,
                        to: to.as_deref().map(parse_time).transpose()?,
                        actor,
                        action,
                        limit: Some(limit),
                    },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for e in &result.episodes {
                    println!(
                        "{}  {:<14} {:<8} {}",
                        format_age(e.ts),
                        e.actor.as_str(),
                        e.action.as_str(),
                        e.metadata
                            .get("path")
                            .and_then(|p| p.as_str())
                            .map(str::to_string)
                            .or_else(|| e.node_id.map(|n| format!("node {n}")))
                            .unwrap_or_default()
                    );
                }
                println!("\nshowing {} of {} episodes", result.episodes.len(), result.total);
            }
        }

        Command::WorkingSet => {
            let result: WorkingSetResult = client
                .call(method::BRAIN_WORKING_SET, serde_json::json!({}))
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for entry in &result.nodes {
                    println!(
                        "{}  {:<28} {}{}",
                        format_age(entry.last_touched),
                        entry.actions.join(","),
                        entry
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| format!("node {}", entry.node_id)),
                        if entry.pinned { "  [pinned]" } else { "" }
                    );
                }
                if result.nodes.is_empty() {
                    println!("nothing recent");
                }
                println!("\n{}", result.basis);
            }
        }

        Command::Consolidate { dry_run } => {
            let report: cortex_memory::ConsolidationReport = client
                .call(method::BRAIN_CONSOLIDATE, serde_json::json!({ "dry_run": dry_run }))
                .await?;
            if args.json {
                print_json(&report)?;
            } else {
                print!("{}", report.render());
            }
        }

        Command::Report => {
            let report: Option<cortex_memory::ConsolidationReport> =
                client.call(method::BRAIN_REPORT, serde_json::json!({})).await?;
            match report {
                Some(report) if args.json => print_json(&report)?,
                Some(report) => print!("{}", report.render()),
                None if args.json => print_json(&serde_json::Value::Null)?,
                None => println!("consolidation has never run. `cortex consolidate` starts one."),
            }
        }

        Command::Graph { out, limit, events, zone } => {
            let snapshot: serde_json::Value = client
                .call(
                    method::BRAIN_GRAPH,
                    serde_json::json!({
                        "limit": limit,
                        "events": events,
                        "zone": zone,
                    }),
                )
                .await?;
            let text = serde_json::to_string(&snapshot)?;
            if out == "-" {
                println!("{text}");
            } else {
                std::fs::write(&out, text.as_bytes())?;
                // The path is the user's own and is printed, not logged. A snapshot of a real
                // index contains real paths, so the message says so rather than leaving the
                // reader to find out by opening it.
                let nodes = snapshot["nodes"].as_array().map(Vec::len).unwrap_or(0);
                let edges = snapshot["edges"].as_array().map(Vec::len).unwrap_or(0);
                println!(
                    "wrote {out}: {nodes} nodes, {edges} edges, {} bytes.\n\
                     It contains the paths of the files it describes. Treat it like the index.",
                    text.len()
                );
            }
        }

        Command::Diff { since, limit } => {
            let result: DiffResult = client
                .call(method::BRAIN_DIFF, DiffParams { since, limit })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for e in &result.changes {
                    println!(
                        "{:<8} {:<14} {:<8} {}",
                        e.id,
                        e.actor.as_str(),
                        e.action.as_str(),
                        e.metadata
                            .get("path")
                            .and_then(|p| p.as_str())
                            .unwrap_or_default()
                    );
                }
                println!(
                    "\ncursor {}{}",
                    result.cursor,
                    if result.more { " (more to come)" } else { "" }
                );
            }
        }

        Command::Mount { device, ask } => {
            let result: FsMountResult = client
                .call(method::FS_MOUNT, FsMountParams { device, interactive: ask })
                .await?;
            if args.json {
                print_json(&result)?;
            } else if result.mounted_now {
                println!("{} is at {}", result.device.display(), result.mount_point.display());
            } else {
                println!(
                    "{} was already at {}",
                    result.device.display(),
                    result.mount_point.display()
                );
            }
        }

        Command::Unmount { device, eject } => {
            let result: FsUnmountResult = client
                .call(method::FS_UNMOUNT, FsUnmountParams { device, eject })
                .await?;
            if args.json {
                print_json(&result)?;
            } else if result.ejected {
                println!("{} can be unplugged", result.device.display());
            } else {
                println!("{} is unmounted", result.device.display());
            }
        }

        Command::Apps { path, all, set_default } => {
            if let Some(id) = set_default {
                // The type comes from the file rather than from the caller: a default set for
                // the wrong type looks like it worked and changes nothing anybody notices.
                let found: AppsForPathResult = client
                    .call(
                        method::APPS_FOR_PATH,
                        AppsForPathParams { path: path.clone(), all: false },
                    )
                    .await?;
                let result: AppsSetDefaultResult = client
                    .call(
                        method::APPS_SET_DEFAULT,
                        AppsSetDefaultParams { mime: found.mime, application: id.clone() },
                    )
                    .await?;
                if args.json {
                    print_json(&result)?;
                } else {
                    println!("{} now opens {} ({})", result.application, result.mime, result.file.display());
                }
                return Ok(());
            }

            let result: AppsForPathResult = client
                .call(method::APPS_FOR_PATH, AppsForPathParams { path, all })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!("{}", result.mime);
                if result.applications.is_empty() {
                    println!("  nothing claims this type — try --all");
                }
                for app in &result.applications {
                    // The marker is the whole point of the column: which of these is what a
                    // double click does.
                    println!(
                        "  {} {:<40} {}",
                        if app.is_default { "*" } else { " " },
                        app.name,
                        app.id
                    );
                }
            }
        }

        Command::Places => {
            let result: FsPlacesResult = client.call(method::FS_PLACES, FsPlacesParams {}).await?;
            if args.json {
                print_json(&result)?;
            } else {
                for section in &result.sections {
                    println!("\n{}", section.title);
                    for place in &section.places {
                        // The capacity where there is one, because "which disk was that" is the
                        // question a list of mount points does not answer.
                        let capacity = match place.capacity {
                            Some(c) => format!(
                                "  {} von {} frei",
                                human_bytes(c.free_bytes),
                                human_bytes(c.total_bytes)
                            ),
                            None => String::new(),
                        };
                        // A drive that is not mounted yet has a device where the others have a
                        // directory, and the difference decides what pressing Enter on it means.
                        let state = match &place.volume {
                            Some(v) if !v.mounted => "  (nicht eingehängt)",
                            _ => "",
                        };
                        println!(
                            "  {:<24} {}{}{}",
                            place.name,
                            place.path.display(),
                            capacity,
                            state
                        );
                    }
                }
                println!();
            }
        }

        Command::Ls { path, depth, limit } => {
            let result: FsListResult = client
                .call(
                    method::FS_LIST,
                    FsListParams {
                        path,
                        glob: None,
                        depth: Some(depth),
                        limit,
                        sort: Default::default(),
                        reverse: false,
                    },
                )
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                for entry in &result.entries {
                    // The columns `ls` has, in the order `ls` has them, because this is the
                    // surface a person reaches for when they want to check permissions and any
                    // rearrangement would make them read it twice.
                    let mode = match entry.mode {
                        Some(mode) => cortex_core::tools::format_mode(entry.kind, mode),
                        None => format!("{:<10}", format!("{:?}", entry.kind).to_lowercase()),
                    };
                    let owner = match (&entry.owner, &entry.group) {
                        (Some(user), Some(group)) => format!("{} {}", user.label(), group.label()),
                        _ => String::new(),
                    };
                    let link = match &entry.target {
                        Some(target) => format!(
                            " -> {}{}",
                            target.display(),
                            if entry.broken { " (missing)" } else { "" }
                        ),
                        None => String::new(),
                    };
                    println!(
                        "{mode} {owner:<16} {:>10}  {}{link}{}",
                        entry.size,
                        entry.path.display(),
                        if entry.indexed { "" } else { "  (not indexed)" }
                    );
                }
                let mut notes = Vec::new();
                if result.hidden > 0 {
                    notes.push(format!("{} hidden by the deny-list", result.hidden));
                }
                if result.truncated {
                    notes.push("truncated".to_string());
                }
                println!(
                    "\n{} entr{}{}",
                    result.entries.len(),
                    if result.entries.len() == 1 { "y" } else { "ies" },
                    if notes.is_empty() { String::new() } else { format!(", {}", notes.join(", ")) }
                );
            }
        }

        Command::Cat { path, range } => {
            let byte_range = match &range {
                None => None,
                Some(text) => {
                    let (start, end) = text
                        .split_once(':')
                        .context("a range looks like `start:end`")?;
                    Some((start.parse::<u64>()?, end.parse::<u64>()?))
                }
            };
            let result: FsReadResult = client
                .call(method::FS_READ, FsReadParams { path, byte_range })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                print!("{}", result.content);
                if result.truncated {
                    eprintln!(
                        "\n[truncated: {} of {} bytes]",
                        result.bytes, result.total_bytes
                    );
                }
                if let Some(note) = &result.note {
                    eprintln!("[{note}]");
                }
            }
        }

        Command::Stat { path } => {
            let result: FsStatResult = client
                .call(method::FS_STAT, FsStatParams { path })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                println!("path      {}", result.path.display());
                if !result.exists {
                    println!("state     does not exist");
                } else {
                    println!("kind      {:?}", result.kind);
                    println!("size      {} bytes", result.size);
                }
                println!("readable  {}", yes_no(result.readable));
                println!("writable  {}", yes_no(result.writable));
                println!(
                    "indexed   {}",
                    match result.node_id {
                        Some(id) => format!("yes, node {id}"),
                        None => "no".to_string(),
                    }
                );
                if let Some(mime) = &result.mime {
                    println!("mime      {mime}");
                }
            }
        }

        Command::Write { path, content, mode, yes } => {
            let content = match content {
                Some(text) => text,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let mode = match mode.as_str() {
                "overwrite" => WriteMode::Overwrite,
                "append" => WriteMode::Append,
                _ => WriteMode::Create,
            };
            let make = |plan: Option<String>| FsWriteParams {
                path: path.clone(),
                content: content.clone(),
                mode,
                plan,
            };
            let result = gated(&mut client, method::FS_WRITE, make, yes).await?;
            report_mutation(&result, args.json)?;
        }

        Command::Mv { from, to, yes } => {
            let make = |plan: Option<String>| FsMoveParams {
                from: from.clone(),
                to: to.clone(),
                plan,
            };
            let result = gated(&mut client, method::FS_MOVE, make, yes).await?;
            report_mutation(&result, args.json)?;
        }

        Command::Trash { path, yes } => {
            let make = |plan: Option<String>| FsTrashParams { path: path.clone(), plan };
            let result = gated(&mut client, method::FS_TRASH, make, yes).await?;
            report_mutation(&result, args.json)?;
        }

        Command::Undo { last, plan, dry_run } => {
            let result: UndoResult = client
                .call(method::UNDO, UndoParams { last: Some(last), plan, dry_run })
                .await?;
            if args.json {
                print_json(&result)?;
            } else {
                if result.undone.is_empty() && result.failed.is_empty() {
                    println!("nothing to undo");
                }
                for entry in &result.undone {
                    println!(
                        "{} {}",
                        if result.dry_run { "would undo" } else { "undid    " },
                        entry.description
                    );
                }
                for failure in &result.failed {
                    println!("FAILED    {} — {}", failure.description, failure.reason);
                }
                println!("\n{} operation(s) still undoable", result.remaining);
            }
        }

        Command::Pause | Command::Resume => {
            let paused = matches!(args.command, Command::Pause);
            let result: IndexSetPausedResult = client
                .call(method::INDEX_SET_PAUSED, IndexSetPausedParams { paused })
                .await?;
            if args.json {
                print_json(&result)?;
            } else if result.paused {
                println!("indexing paused — it stays paused across restarts until `cortex resume`");
            } else {
                println!("indexing resumed");
            }
        }

        Command::AgentSurface { command } => {
            let result: AgentStatusResult = match command {
                None => client.call(method::AGENT_STATUS, serde_json::json!({})).await?,
                Some(cmd) => {
                    let enabled = matches!(cmd, AgentSurfaceCommand::On);
                    client
                        .call(method::AGENT_SET_ENABLED, AgentSetEnabledParams { enabled })
                        .await?
                }
            };

            if args.json {
                print_json(&result)?;
            } else {
                println!(
                    "surface     {}",
                    if result.enabled { "ENABLED" } else { "DISABLED" }
                );
                if !result.configured_enabled {
                    println!("            (switched off in the configuration file; RPC cannot lift that)");
                }
                println!("indexing    {}", if result.indexing_paused { "PAUSED" } else { "running" });
                println!("dry run     {}", if result.require_dry_run { "required" } else { "not required" });
                if result.write_allowlist.is_empty() {
                    println!("writable    nothing — agents cannot mutate any file");
                } else {
                    println!("writable    {}", result.write_allowlist.join("\n            "));
                }
                println!("undoable    {} operation(s)", result.undo_depth);
                println!("audit       {} entries", result.audit_entries);
            }
        }

        Command::Audit { limit, actor, refused } => {
            let result: AuditRecentResult = client
                .call(
                    method::AUDIT_RECENT,
                    AuditRecentParams { limit: Some(limit), actor, refused_only: refused },
                )
                .await?;

            if args.json {
                print_json(&result)?;
            } else if result.entries.is_empty() {
                println!("no audit entries{}", if refused { " — nothing has been refused" } else { "" });
            } else {
                for entry in &result.entries {
                    // The refusals are what this log exists for, so they are marked rather than
                    // left to be spotted in a column of identical-looking lines.
                    println!(
                        "{}  {}  {:<20} {}",
                        format_age(entry.ts),
                        if entry.allowed { "ok " } else { "REF" },
                        entry.tool,
                        entry.actor
                    );
                    if let Some(summary) = &entry.result_summary {
                        println!("    {summary}");
                    }
                    println!("    {}", compact_json(&entry.args));
                }
                println!("\nshowing {} of {} entries", result.entries.len(), result.total);
            }
        }

        Command::Shutdown => {
            let _: serde_json::Value = client.call(method::SHUTDOWN, serde_json::json!({})).await?;
            println!("stopping");
        }
    }

    Ok(())
}

/// Download the model, with the user's consent and a progress line.
async fn fetch_model(
    paths: &Paths,
    yes: bool,
    runtime_only: bool,
    gpu: bool,
    with_reranker: bool,
    json: bool,
) -> Result<()> {
    use cortex_embed::fetch::{ModelStore, Progress};
    use cortex_embed::registry::{self, RERANKER, RUNTIME_CPU, RUNTIME_CUDA};

    let store = ModelStore::new(paths.state_dir());
    let model = registry::model_by_id("multilingual-e5-small")
        .context("the default model is missing from the registry")?;
    let runtime = if gpu { &RUNTIME_CUDA } else { &RUNTIME_CPU };

    let total = runtime.archive.bytes
        + if runtime_only { 0 } else { model.total_bytes() }
        + if with_reranker { RERANKER.total_bytes() } else { 0 };
    if !yes {
        // Rule R5 is about consent, not only about defaults. Say what will be downloaded, from
        // where, and how big it is — then ask.
        println!("About to download {:.0} MB:", total as f64 / 1_000_000.0);
        println!("  {} ({:.0} MB)", runtime.archive.url, runtime.archive.bytes as f64 / 1_000_000.0);
        if runtime.cuda {
            println!("  the CUDA build needs {} on this machine", runtime.requires);
        }
        if !runtime_only {
            for file in model.files {
                println!("  {} ({:.0} MB)", file.url, file.bytes as f64 / 1_000_000.0);
            }
        }
        if with_reranker {
            for file in RERANKER.files {
                println!("  {} ({:.0} MB)", file.url, file.bytes as f64 / 1_000_000.0);
            }
        }
        println!("into {}", store.root().display());
        print!("Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("nothing was downloaded");
            return Ok(());
        }
    }

    let mut last_percent = 0u64;
    let mut progress = |event: Progress<'_>| match event {
        Progress::Started { name, bytes } => {
            last_percent = 0;
            println!("fetching {name} ({:.0} MB)", bytes as f64 / 1_000_000.0);
        }
        Progress::Advanced { done, total, .. } => {
            let percent = if total > 0 { done * 100 / total } else { 0 };
            if percent >= last_percent + 5 {
                last_percent = percent;
                print!("\r  {percent} %");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
        Progress::Verified { name } => println!("\r  {name}: verified"),
        Progress::AlreadyPresent { name } => println!("  {name}: already there"),
    };

    store
        .fetch_runtime(runtime, &mut progress)
        .context("could not provision the ONNX runtime")?;
    if !runtime_only {
        store
            .fetch_model(model, &mut progress)
            .context("could not provision the model")?;
    }
    if with_reranker {
        store
            .fetch_reranker(&RERANKER, &mut progress)
            .context("could not provision the reranker")?;
    }

    println!("model store: {}", store.root().display());

    // Tell a running daemon, if there is one. Not being able to reach it is not a failure —
    // the files are on disk and the next start will find them.
    match Client::connect(&paths.socket()).await {
        Ok(mut client) => {
            let result: ModelReloadResult = client
                .call(method::MODEL_RELOAD, serde_json::json!({}))
                .await?;
            if json {
                print_json(&result)?;
            } else if result.loaded {
                println!("daemon reloaded: {}", result.backend.as_deref().unwrap_or(""));
            } else {
                println!(
                    "daemon did not load it: {}",
                    result.note.as_deref().unwrap_or("no reason given")
                );
            }
        }
        Err(_) => println!("no daemon running; it will pick the model up at the next start"),
    }

    Ok(())
}

fn print_model_status(status: &ModelStatusResult) {
    println!("model      {}", status.model);
    if !status.enabled {
        println!("state      disabled in the configuration");
        return;
    }

    match (&status.backend, &status.note) {
        (Some(backend), _) => println!("state      loaded — {backend}"),
        (None, Some(note)) => println!("state      not loaded — {note}"),
        (None, None) => println!("state      not loaded"),
    }

    println!(
        "files      model {} · runtime {} ({}{})",
        if status.model_present { "present" } else { "missing" },
        if status.runtime_present { "present" } else { "missing" },
        status.runtime_id,
        if status.runtime_is_cuda { ", gpu build" } else { "" }
    );
    if !status.model_present || !status.runtime_present {
        println!(
            "           run `cortex model fetch` ({:.0} MB)",
            status.download_bytes as f64 / 1_000_000.0
        );
    }
    println!("           {}", status.model_dir);

    if let Some(dim) = status.dim {
        println!(
            "vectors    {} over files, {} over memories, {dim} dimensions",
            status.vectors_in_index, status.memory_vectors_in_index
        );
    }
    println!(
        "coverage   {} of {} chunks embedded{}",
        status.chunks_embedded,
        status.chunks_total,
        if status.chunks_pending > 0 {
            format!(", {} pending", status.chunks_pending)
        } else if status.caught_up {
            " (complete)".to_string()
        } else {
            String::new()
        }
    );
    if status.embedded_this_run > 0 || status.busy_ms > 0 {
        println!(
            "this run   {} embedded, {} stale, {} failures, busy {:.1}s",
            status.embedded_this_run,
            status.stale,
            status.failures,
            status.busy_ms as f64 / 1000.0
        );
    }
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

/// A CLI argument naming a node: a number is an id, anything else is a path.
///
/// Resolved here rather than server-side because only the client knows what the user typed;
/// a daemon that guessed would eventually treat a file called `42` as a node id.
fn parse_target(text: &str) -> Target {
    match text.parse::<i64>() {
        Ok(id) => Target::Node(id),
        Err(_) => Target::Path(std::path::PathBuf::from(text)),
    }
}

/// A point in time: Unix seconds, or a span back from now written as `24h`, `30m` or `7d`.
fn parse_time(text: &str) -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let (value, unit) = text.split_at(text.len().saturating_sub(1));
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        // No suffix: an absolute timestamp.
        _ => return text.parse::<i64>().context("expected a Unix timestamp or a span like 24h"),
    };
    let amount: i64 = value.parse().context("expected a span like 24h")?;
    Ok(now - amount * scale)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Run a gated tool: plan first, then apply if the caller said so.
///
/// The plan is always made, even with `--yes`. That is not ceremony — the plan is what records
/// the state of the target, and applying it is what checks that the state has not changed since
/// (rule R6). Skipping it would mean an approval given for a filesystem nobody looked at.
async fn gated<P: serde::Serialize>(
    client: &mut Client,
    method_name: &str,
    make_params: impl Fn(Option<String>) -> P,
    apply: bool,
) -> Result<Mutation> {
    let planned: Mutation = client.call(method_name, make_params(None)).await?;
    match planned {
        Mutation::Planned { plan } if apply => {
            Ok(client.call(method_name, make_params(Some(plan.id))).await?)
        }
        other => Ok(other),
    }
}

fn report_mutation(result: &Mutation, as_json: bool) -> Result<()> {
    if as_json {
        return print_json(result);
    }
    match result {
        Mutation::Planned { plan } => {
            println!("plan {} — nothing has happened yet:", plan.id);
            for line in &plan.summary {
                println!("  {line}");
            }
            println!(
                "\nrun the same command with --yes within {} seconds to apply it",
                plan.expires_in_secs
            );
        }
        Mutation::Applied { undo_id, steps, trashed } => {
            for line in cortex_core::tools::describe_steps(steps) {
                println!("{line}");
            }
            if let Some(trashed) = trashed {
                println!("previous version kept at {}", trashed.display());
            }
            println!("`cortex undo` reverses this (entry {undo_id})");
        }
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// How long ago something happened, from a Unix timestamp.
///
/// Relative rather than absolute on purpose: a log read at a glance answers "was that just
/// now?", and getting a calendar date right would mean a time zone database for a column
/// nobody reads that way.
fn format_age(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ts);
    let age = (now - ts).max(0) as u64;
    format!("{:>8} ago", format_duration(age))
}

/// One line of JSON, cut short. Audit arguments are already redacted; this is about the width
/// of a terminal, not about secrecy.
fn compact_json(value: &serde_json::Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 120 {
        let head: String = text.chars().take(120).collect();
        format!("{head}…")
    } else {
        text
    }
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

/// Bytes, in the units a person reads.
///
/// Powers of a thousand rather than of 1024, because that is what the label on a disk says and
/// this list is about telling one disk from another.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let digits = if unit == 0 || value >= 100.0 { 0 } else { 1 };
    format!("{value:.digits$} {}", UNITS[unit])
}
