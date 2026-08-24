//! Wire protocol.
//!
//! Every client — GUI, CLI, MCP server, plugins — speaks exactly this. There is no privileged
//! side channel and no second protocol for "internal" clients; that is what keeps the surfaces
//! from drifting apart (build prompt, M3 Definition of Done §5).
//!
//! Framing is newline-delimited JSON: one JSON value per line. It is trivially debuggable with
//! `socat` and needs no length-prefix state machine.

use cortex_core::ids::NodeId;
use cortex_core::model::{AuditEntry, Episode, Node, QueryMode, ScoreBreakdown, SearchHit};
use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: Version,
    /// Absent for notifications, which expect no response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: Version,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: Version,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: serde_json::Value, error: RpcError) -> Self {
        Self {
            jsonrpc: Version,
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// The literal string `"2.0"`, enforced at the type level so a malformed version cannot be
/// silently accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Version;

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "2.0" {
            Ok(Version)
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported jsonrpc version: {s}"
            )))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    // Standard JSON-RPC codes.
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    // Application codes. Kept well away from the reserved range.
    /// The Agent Surface is switched off (hard rule R7).
    pub const AGENT_SURFACE_DISABLED: i32 = 1001;
    /// The operation was refused by the policy engine.
    pub const POLICY_REFUSED: i32 = 1002;
    /// The operation requires a confirmed dry-run plan first.
    pub const DRY_RUN_REQUIRED: i32 = 1003;

    /// The operation would work, if somebody with an administrator's password said so.
    ///
    /// Its own code because it is the one refusal a *client* can do something about: a window
    /// with a session can ask, where the daemon must never wait on a human. A page that had to
    /// recognise this by reading the message would break the first time the wording changed.
    pub const NEEDS_AUTHORISATION: i32 = 1004;

    /// The answer was produced, and it is too large to send.
    ///
    /// The protocol is newline-delimited JSON with a line limit, so an oversized reply cannot
    /// be written at all. Before this code existed the daemon simply failed to write it and the
    /// caller waited for an answer that was never coming — a hang, which is a worse failure
    /// than an error because it does not localise itself. A caller that sees this should ask
    /// for less: every method capable of producing a large answer takes a `limit`.
    pub const RESULT_TOO_LARGE: i32 = 1005;

    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(Self::METHOD_NOT_FOUND, format!("unknown method: {method}"))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(Self::INVALID_PARAMS, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Self::INTERNAL_ERROR, message)
    }
}

// ---------------------------------------------------------------------------
// Method parameter and result types.
//
// These are shared by the daemon and every client, so neither side hand-rolls JSON. A method
// added here without a matching handler is a compile-time-visible gap, not a runtime surprise.
// ---------------------------------------------------------------------------

/// Method names, in one place so clients cannot invent a typo'd variant.
pub mod method {
    pub const PING: &str = "ping";
    pub const STATUS: &str = "status";
    pub const SEARCH: &str = "search";
    pub const NODE_GET: &str = "node.get";
    pub const EPISODE_APPEND: &str = "episode.append";
    pub const EPISODES_RECENT: &str = "episodes.recent";
    pub const INDEX_PATH: &str = "index.path";
    pub const SEARCH_EXPLAIN: &str = "search.explain";
    /// Sent by the daemon, unsolicited, when a search's results improve after they were
    /// delivered. Never called by a client (D7).
    pub const SEARCH_UPDATE: &str = "search.update";
    pub const EXTRACT_STATUS: &str = "extract.status";
    pub const REBUILD_INDEX: &str = "rebuild.index";
    pub const CANONICAL_DUMP: &str = "canonical.dump";
    pub const MODEL_STATUS: &str = "model.status";
    pub const MODEL_RELOAD: &str = "model.reload";
    pub const EXTRACT_REQUEUE: &str = "extract.requeue";
    pub const ACTIVITY_PING: &str = "activity.ping";
    pub const WATCH_STATUS: &str = "watch.status";
    pub const ZONES_LIST: &str = "zones.list";
    pub const ZONE_SET_COLOR: &str = "zone.set_color";
    pub const RECONCILE: &str = "reconcile";
    pub const SHUTDOWN: &str = "shutdown";

    // The Agent Surface's own controls (hard rules R7 and R8). These are not agent tools —
    // they are how a human governs the agent tools, and they answer whether the surface is
    // open regardless of whether it is.
    pub const AGENT_STATUS: &str = "agent.status";
    pub const AGENT_SET_ENABLED: &str = "agent.set_enabled";
    pub const INDEX_SET_PAUSED: &str = "index.set_paused";
    pub const AUDIT_RECENT: &str = "audit.recent";

    /// Name yourself. Everything after it on this connection is attributed to that actor, in
    /// episodes and in the audit log. Unclaimed connections are `human`.
    pub const IDENTIFY: &str = "identify";

    // The agent tools themselves. `fs.write`, `fs.move` and `fs.trash` are gated: called
    // without a plan id they return a plan and change nothing.
    pub const FS_LIST: &str = "fs.list";
    /// Where a person would like to start: home, their folders, the drives, and one row for
    /// everything technical. See `cortex-agent/src/places.rs` for why the daemon decides this.
    pub const FS_PLACES: &str = "fs.places";
    pub const FS_READ: &str = "fs.read";
    /// What opens this file: the type the desktop's own database gives it, and the applications
    /// that claim that type, the default first. The daemon reads the association files; running
    /// the application belongs to whichever surface has a session (D49).
    pub const APPS_FOR_PATH: &str = "apps.for_path";
    /// Record which application opens a type, in the file the rest of the desktop reads.
    /// Refused for agents whatever the allow-list says — see `Surface::apps_set_default`.
    pub const APPS_SET_DEFAULT: &str = "apps.set_default";
    /// Mount a volume that is plugged in and not mounted, and answer with where it landed.
    /// Refused for agents: it is a hardware operation on somebody's desk, not a path a policy
    /// can be written about.
    pub const FS_MOUNT: &str = "fs.mount";
    /// Unmount one, optionally powering off the drive so it can be unplugged. Refused for
    /// agents, and the power-off is refused for a disk that cannot be unplugged.
    pub const FS_UNMOUNT: &str = "fs.unmount";
    pub const FS_STAT: &str = "fs.stat";
    pub const FS_WRITE: &str = "fs.write";
    pub const FS_MOVE: &str = "fs.move";
    pub const FS_TRASH: &str = "fs.trash";
    pub const UNDO: &str = "undo";

    // The operations a file manager needs and an agent may also use. `fs.transfer` answers with
    // a handle rather than a result, because copying can take a minute.
    pub const FS_TRANSFER: &str = "fs.transfer";
    pub const FS_MKDIR: &str = "fs.mkdir";
    pub const FS_RENAME: &str = "fs.rename";
    pub const OPERATION_CANCEL: &str = "operation.cancel";
    /// Sent by the daemon while a transfer runs, throttled to ten a second. Never called.
    pub const TRANSFER_PROGRESS: &str = "transfer.progress";
    /// Sent once when a transfer ends, however it ended. Never called.
    pub const TRANSFER_DONE: &str = "transfer.done";

    // Memory. These write to the brain and never to a file, so they are gated by the kill
    // switch and audited, but not by the write allow-list (R6).
    pub const BRAIN_SEARCH: &str = "brain.search";
    pub const BRAIN_ANNOTATE: &str = "brain.annotate";
    pub const BRAIN_PIN: &str = "brain.pin";
    pub const BRAIN_FORGET: &str = "brain.forget";
    pub const BRAIN_LINK: &str = "brain.link";
    pub const BRAIN_NEIGHBORS: &str = "brain.neighbors";
    pub const BRAIN_TIMELINE: &str = "brain.timeline";
    pub const BRAIN_WORKING_SET: &str = "brain.working_set";
    pub const BRAIN_DIFF: &str = "brain.diff";
    pub const BRAIN_RECALL: &str = "brain.recall";
    pub const BRAIN_EXPLAIN: &str = "brain.explain";
    /// The whole brain in one answer, for a visualisation. A read, and a large one.
    pub const BRAIN_GRAPH: &str = "brain.graph";
    /// Run a consolidation pass: replay, associate, score, tier. Writes to the brain only.
    pub const BRAIN_CONSOLIDATE: &str = "brain.consolidate";
    /// The last consolidation report, without running one.
    pub const BRAIN_REPORT: &str = "brain.report";

    // The agent brain (M11). Writes to the brain, never to a file, so the same rule applies:
    // gated by the kill switch, audited, and not subject to the write allow-list (R6).
    /// Write a memory. The scope is resolved from the caller's identity, never taken verbatim.
    pub const BRAIN_REMEMBER: &str = "brain.remember";
    /// A context pack: budgeted, deduplicated and attributed. The tool an agent reaches for
    /// first, and the one M11 exists to provide.
    pub const BRAIN_CONTEXT: &str = "brain.context";
    /// Report which items of a pack helped and which were noise. The learning signal (D76).
    pub const BRAIN_OUTCOME: &str = "brain.outcome";
    /// List memories. An agent sees its own scopes; only a human may ask for all of them.
    pub const BRAIN_MEMORIES: &str = "brain.memories";
    /// Mark one memory as replaced by another.
    pub const BRAIN_SUPERSEDE: &str = "brain.supersede";
    /// Delete a memory, recording the deletion so that it survives a rebuild (D78).
    pub const BRAIN_FORGET_MEMORY: &str = "brain.forget_memory";

    /// Open an agent session, so that attribution can answer "this task" rather than "this
    /// agent, some time in the last year".
    pub const AGENT_SESSION_BEGIN: &str = "agent.session.begin";
    pub const AGENT_SESSION_END: &str = "agent.session.end";
    pub const AGENT_SESSIONS: &str = "agent.sessions";

    /// Every method this build serves. Used by `status` so a client can discover the surface.
    /// Methods that may be answered out of order, concurrently with each other.
    ///
    /// **An allow-list, so that a new method is serialised until somebody has thought about
    /// it.** Everything here reads: it does not write a file, does not write the index beyond
    /// an audit row, and does not depend on having happened before or after its neighbours.
    ///
    /// The reason this exists: a connection used to answer one request at a time, so a single
    /// slow call held up everything queued behind it. The journal shows the signature clearly —
    /// a `search`, a `brain.neighbors` and an `fs.read` all reporting **152 ms** and all
    /// finishing within 0.4 ms of each other. They were not slow. They were in a queue. That is
    /// what scrolling a result list feels like: every row asks for its neighbours and its
    /// preview, and each one waits behind the last.
    ///
    /// Writes stay in order on purpose. `fs.transfer`, `undo` and the rest have an order that
    /// means something — an undo undoes the previous thing — and making them concurrent would
    /// be a much larger claim than "these reads are independent".
    pub fn is_concurrent(method: &str) -> bool {
        matches!(
            method,
            PING | STATUS
                | SEARCH
                | SEARCH_EXPLAIN
                | NODE_GET
                | EPISODES_RECENT
                | EXTRACT_STATUS
                | MODEL_STATUS
                | WATCH_STATUS
                | ZONES_LIST
                | AGENT_STATUS
                | BRAIN_GRAPH
                | BRAIN_REPORT
                | AUDIT_RECENT
                | FS_LIST
                | FS_PLACES
                | FS_READ
                | FS_STAT
                | APPS_FOR_PATH
                | BRAIN_SEARCH
                | BRAIN_NEIGHBORS
                | BRAIN_TIMELINE
                | BRAIN_WORKING_SET
                // Reads, and the pack is the expensive one: it runs two arms and a
                // diversification pass, and an agent asking for it must not hold up the window
                // asking for a preview.
                | BRAIN_MEMORIES
                | BRAIN_CONTEXT
                | AGENT_SESSIONS
        )
    }

    pub const ALL: &[&str] = &[
        PING,
        STATUS,
        SEARCH,
        NODE_GET,
        EPISODE_APPEND,
        EPISODES_RECENT,
        INDEX_PATH,
        SEARCH_EXPLAIN,
        EXTRACT_STATUS,
        EXTRACT_REQUEUE,
        REBUILD_INDEX,
        CANONICAL_DUMP,
        MODEL_STATUS,
        MODEL_RELOAD,
        ACTIVITY_PING,
        WATCH_STATUS,
        ZONES_LIST,
        ZONE_SET_COLOR,
        RECONCILE,
        SHUTDOWN,
        AGENT_STATUS,
        AGENT_SET_ENABLED,
        INDEX_SET_PAUSED,
        AUDIT_RECENT,
        IDENTIFY,
        FS_LIST,
        FS_PLACES,
        FS_READ,
        FS_STAT,
        APPS_FOR_PATH,
        APPS_SET_DEFAULT,
        FS_WRITE,
        FS_MOVE,
        FS_TRASH,
        FS_TRANSFER,
        FS_MKDIR,
        FS_RENAME,
        FS_MOUNT,
        FS_UNMOUNT,
        OPERATION_CANCEL,
        UNDO,
        BRAIN_SEARCH,
        BRAIN_ANNOTATE,
        BRAIN_PIN,
        BRAIN_FORGET,
        BRAIN_LINK,
        BRAIN_NEIGHBORS,
        BRAIN_TIMELINE,
        BRAIN_WORKING_SET,
        BRAIN_GRAPH,
        BRAIN_CONSOLIDATE,
        BRAIN_REPORT,
        BRAIN_REMEMBER,
        BRAIN_CONTEXT,
        BRAIN_OUTCOME,
        BRAIN_MEMORIES,
        BRAIN_SUPERSEDE,
        BRAIN_FORGET_MEMORY,
        AGENT_SESSION_BEGIN,
        AGENT_SESSION_END,
        AGENT_SESSIONS,
        BRAIN_DIFF,
        BRAIN_RECALL,
        BRAIN_EXPLAIN,
    ];
}

/// How perception is doing. Degradation must be visible, per decision D20.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStatusResult {
    /// False when no watcher could be started at all; the index then relies purely on
    /// periodic reconciliation.
    pub active: bool,
    pub watched_dirs: usize,
    pub budget: usize,
    /// True when the budget was hit, so some subtree is unwatched.
    pub degraded: bool,
    pub unwatched: usize,
    /// Times the kernel queue overflowed. Any value above zero means events were lost and a
    /// reconcile was triggered.
    pub overflows: u64,
    pub paused: bool,
    /// Paths being watched.
    pub roots: Vec<String>,
    pub last_reconcile: Option<ReconcileSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReconcileSummary {
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
    pub renamed: u64,
    pub unchanged: u64,
    pub skipped: u64,
}

/// One spatial zone. See decision D15.
///
/// Only the hue travels: saturation and lightness are fixed by the theme, so no zone can be
/// louder than another. The name and icon are what make the colour redundant rather than
/// load-bearing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub id: i64,
    pub root_path: String,
    pub name: String,
    pub color_hue: u16,
    pub icon: Option<String>,
    /// True when the user chose this colour, so regeneration leaves it alone.
    pub user_override: bool,
    /// Nodes currently assigned to this zone.
    pub node_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZonesListResult {
    pub zones: Vec<ZoneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneSetColorParams {
    pub id: i64,
    /// Hue in degrees, 0–359.
    pub hue: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileParams {
    /// Reconcile only this path. Defaults to every configured root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileResult {
    pub summary: ReconcileSummary,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub pong: bool,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResult {
    pub version: String,
    pub schema_version: i64,
    pub uptime_secs: u64,
    pub database: String,
    pub files: i64,
    pub dirs: i64,
    pub chunks: i64,
    pub episodes: i64,
    pub embeddings: i64,
    pub outbox_pending: i64,
    /// Whether the Agent Surface is currently accepting agent traffic (rule R7).
    pub agent_surface_enabled: bool,
    /// Paths held in the in-memory path index.
    pub indexed_paths: usize,
    /// Approximate heap used by the path index, so the memory budget is observable.
    pub index_memory_bytes: usize,
    pub watcher: Option<WatchStatusResult>,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    /// `Auto` — the default — lets the daemon detect the mode, so the heuristic lives in one
    /// place instead of in every client.
    #[serde(default)]
    pub mode: QueryMode,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Treat the last word as a prefix, which is what makes results appear while the user is
    /// still typing. Defaults to true; a benchmark or a goldset run turns it off to measure
    /// exact matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<bool>,
    /// Follow up with a reranked list. Defaults to the daemon's configuration; a client that
    /// will not wait for it should turn it off so the daemon does not spend the inference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank: Option<bool>,
}

/// A better answer to a search that has already been answered.
///
/// Sent as a JSON-RPC notification, so it has no id of its own and carries the id of the request
/// it refines. A client that does not wait for it loses nothing: the first answer was complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchUpdateParams {
    /// The `id` of the `search` request these results replace.
    pub request_id: serde_json::Value,
    pub hits: Vec<SearchHit>,
    /// Milliseconds the reranker took, measured from when the first answer was sent.
    pub elapsed_ms: u64,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// The mode that actually ran. Not necessarily the one that was asked for: `Auto` is
    /// resolved server-side, and a mode whose machinery is not built yet degrades to one that
    /// is. A client that shows the mode must show this one.
    pub mode: QueryMode,
    /// Milliseconds spent server-side. The latency budgets are the specification, so the
    /// protocol reports the number rather than making clients guess it.
    pub elapsed_ms: u64,
    /// True when a later, better-ranked update is still coming (reranking, from M2).
    pub partial: bool,
}

/// Why a particular node ranked where it did — or did not appear at all.
///
/// Without this the search is neither debuggable nor trustworthy: "it did not find my file" has
/// half a dozen causes, and only the daemon can tell them apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchExplainParams {
    pub query: String,
    #[serde(default)]
    pub mode: QueryMode,
    /// The node to explain, by path or by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchExplainResult {
    /// The query as it was searched, after any mode prefix was stripped.
    pub query: String,
    pub mode: QueryMode,
    /// The FTS5 expression the content arm ran, when it ran one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    pub node: Option<Node>,
    /// Position in the result list, one-based. `None` when it did not make the list.
    pub rank: Option<u32>,
    pub score: Option<ScoreBreakdown>,
    pub path_candidates: usize,
    pub lexical_candidates: usize,
    pub vector_candidates: usize,
    /// Why the vector arm did not run, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_note: Option<String>,
    pub extract_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_note: Option<String>,
    pub chunks: usize,
    /// When the node explained is a memory rather than a file (M11).
    ///
    /// A memory has no path and does not compete in the file ranking, so the rank and the score
    /// above are legitimately absent for one — and an explanation that only said "did not
    /// appear" would be true and useless. What is worth knowing about a memory is its scope,
    /// what it has earned, and whether something has replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<cortex_core::memory::Memory>,
    /// Plain sentences, in the order they should be read. The point of the whole method.
    pub reasons: Vec<String>,
}

/// How far content extraction has got. The counterpart to `watch.status` for the replay stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractStatusResult {
    /// False when no content root is configured, in which case nothing is extracted at all —
    /// the default, and rule R3's whole point.
    pub enabled: bool,
    pub content_roots: Vec<String>,
    pub pending: i64,
    pub done: i64,
    pub skipped: i64,
    pub failed: i64,
    pub chunks: i64,
    /// Counters since this daemon started.
    pub files_extracted: u64,
    pub files_unchanged: u64,
    pub chunks_written: u64,
    pub chunks_reused: u64,
    pub chunks_deleted: u64,
    pub busy_ms: u64,
    /// Seconds since a client last reported human activity. Drives the CPU share.
    pub idle_secs: u64,
    /// Whether the daemon currently considers a human present. Sent rather than derived,
    /// because the threshold is configuration the client does not have.
    pub user_active: bool,
    pub cpu_share: f64,
    pub paused: bool,
}

/// The embedding model, the vector index, and how far the two have got.
///
/// One method rather than three, because the questions people actually ask — "is semantic search
/// working", "why not", "how far along is it" — are answered by the same handful of numbers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatusResult {
    pub enabled: bool,
    pub model: String,
    /// Whether an embedder is loaded right now.
    pub loaded: bool,
    pub backend: Option<String>,
    pub dim: Option<usize>,
    /// Why it is not loaded, when it is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub model_present: bool,
    pub runtime_present: bool,
    /// Which runtime build is installed — the CPU one or the CUDA one.
    pub runtime_id: String,
    pub runtime_is_cuda: bool,
    pub model_dir: String,
    pub runtime_path: String,
    /// Bytes the model would take to download, for a client that wants to warn first.
    pub download_bytes: u64,

    pub vectors_in_index: usize,
    pub chunks_total: i64,
    pub chunks_embedded: i64,
    /// Chunks still waiting for a vector. Zero and `caught_up` together mean semantic search
    /// covers the whole corpus.
    pub chunks_pending: i64,
    pub caught_up: bool,

    /// Counters since this daemon started.
    pub embedded_this_run: u64,
    pub stale: u64,
    pub failures: u64,
    pub busy_ms: u64,
}

/// Throw the index away and derive it again from the filesystem (rule R10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildParams {
    /// Also drop the vectors. They are keyed by content hash and survive a rebuild by default,
    /// which is what makes it affordable — dropping them means re-running the model over the
    /// whole corpus.
    #[serde(default)]
    pub drop_embeddings: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildResult {
    /// Nodes re-derived from the filesystem.
    pub indexed: u64,
    /// Explicit assertions — annotations, links, pins — put back from the episode log. They
    /// are not derivable from the filesystem, so a rebuild has to replay them (R10).
    #[serde(default)]
    pub assertions_replayed: usize,
    /// Assertions whose target no longer exists. Dropped rather than invented, and counted so
    /// the loss is visible.
    #[serde(default)]
    pub assertions_dropped: usize,
    /// Chunks still to be extracted. Extraction runs in the background, so a rebuild returns
    /// before the index is complete — poll `extract.status` to see it finish.
    pub extraction_pending: i64,
    pub embeddings_kept: i64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalDumpResult {
    /// One line per indexed path: stat, content hash, extraction state and every chunk hash.
    /// Stable across rebuilds by construction — ids and discovery times are left out.
    pub dump: String,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReloadResult {
    pub loaded: bool,
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractRequeueResult {
    pub requeued: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityPingResult {
    pub idle_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGetResult {
    pub node: Option<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeAppendParams {
    pub actor: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodesRecentParams {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodesRecentResult {
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPathParams {
    pub path: String,
    /// Metadata only. Content extraction is opt-in and arrives in M2 (rule R3).
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPathResult {
    pub indexed: u64,
    pub skipped: u64,
    pub elapsed_ms: u64,
}


// ---------------------------------------------------------------------------
// Agent Surface governance (R7, R8).
// ---------------------------------------------------------------------------

/// What the Agent Surface will and will not do right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatusResult {
    /// The effective answer: are agent tools being served?
    pub enabled: bool,
    /// What the configuration file permits. When this is false, `enabled` cannot be turned
    /// back on over RPC — a switch thrown in a file is not undone by a message (D32).
    pub configured_enabled: bool,
    pub indexing_paused: bool,
    pub require_dry_run: bool,
    /// The paths agents may mutate, resolved. Empty means no mutation is possible at all.
    pub write_allowlist: Vec<String>,
    /// How many operations could still be undone.
    pub undo_depth: i64,
    pub audit_entries: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSetEnabledParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSetPausedParams {
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSetPausedResult {
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecentParams {
    #[serde(default)]
    pub limit: Option<usize>,
    /// Restrict to one actor, e.g. `agent:claude`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Only the calls that were turned away. Usually the interesting ones.
    #[serde(default)]
    pub refused_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecentResult {
    pub entries: Vec<AuditEntry>,
    /// Total rows in the log, so a client can say "showing 20 of 4 812".
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifyParams {
    /// `human`, `system`, or an agent's name. A bare name is taken as an agent name — nothing
    /// verifies the claim, and the value of it is attribution, not access control.
    pub actor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifyResult {
    pub actor: String,
    /// Whether the agent tools will answer this connection at all.
    pub agent_surface_enabled: bool,
}

/// `brain.recall`: a fragment in, a context out.
///
/// Search answers "which files match these words". Recall answers "what belongs with this",
/// which is search followed by a walk of the associative graph around what it found — pattern
/// completion rather than retrieval (CA3 in the analogy the project is named after).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallParams {
    /// The fragment. A phrase, a filename, a half-remembered sentence.
    pub cue: String,
    /// How far to walk out from each hit.
    #[serde(default)]
    pub depth: Option<u32>,
    /// How many hits to expand around.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    /// What the search itself found, in its own order.
    pub hits: Vec<SearchHit>,
    /// What the graph adds around those hits, and why each one is here.
    pub associations: Vec<RecalledAssociation>,
    pub mode: QueryMode,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecalledAssociation {
    pub node_id: i64,
    pub path: Option<String>,
    /// The hit this was reached from.
    pub via: String,
    pub edge: String,
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "ping");
        assert_eq!(req.id, Some(serde_json::json!(1)));
        assert!(req.params.is_none());
    }

    #[test]
    fn a_wrong_protocol_version_is_rejected() {
        let json = r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#;
        assert!(serde_json::from_str::<Request>(json).is_err());
    }

    #[test]
    fn a_missing_protocol_version_is_rejected() {
        let json = r#"{"id":1,"method":"ping"}"#;
        assert!(serde_json::from_str::<Request>(json).is_err());
    }

    #[test]
    fn a_notification_has_no_id() {
        let json = r#"{"jsonrpc":"2.0","method":"ping"}"#;
        let req: Request = serde_json::from_str(json).unwrap();
        assert!(req.id.is_none());
    }

    #[test]
    fn responses_serialise_without_the_unused_half() {
        // A response carrying both `result` and `error`, or neither, is malformed JSON-RPC.
        let ok = Response::ok(serde_json::json!(1), serde_json::json!({"a": 1}));
        let text = serde_json::to_string(&ok).unwrap();
        assert!(text.contains("\"result\""));
        assert!(!text.contains("\"error\""));

        let err = Response::err(serde_json::json!(1), RpcError::method_not_found("nope"));
        let text = serde_json::to_string(&err).unwrap();
        assert!(text.contains("\"error\""));
        assert!(!text.contains("\"result\""));
    }

    #[test]
    fn error_codes_stay_out_of_the_reserved_range() {
        // JSON-RPC reserves -32768..=-32000. Application codes must not collide with it.
        for code in [
            RpcError::AGENT_SURFACE_DISABLED,
            RpcError::POLICY_REFUSED,
            RpcError::DRY_RUN_REQUIRED,
        ] {
            assert!(
                !(-32768..=-32000).contains(&code),
                "application code {code} collides with the reserved range"
            );
        }
    }

    #[test]
    fn search_params_default_to_letting_the_daemon_choose_the_mode() {
        // `Auto`, not `Locate`. A client that says nothing wants the best answer, and the
        // heuristic that decides what that means belongs in the daemon — one implementation
        // rather than one per client.
        let params: SearchParams = serde_json::from_str(r#"{"query":"foo"}"#).unwrap();
        assert_eq!(params.mode, QueryMode::Auto);
        assert!(params.limit.is_none());
        assert!(params.prefix.is_none(), "prefix matching is decided server-side too");
    }

    #[test]
    fn a_client_built_before_the_content_arm_still_parses() {
        // The M1 wire format, verbatim. Adding fields to a shared type is only safe while old
        // messages keep working, and that is worth an assertion rather than an assumption.
        let params: SearchParams =
            serde_json::from_str(r#"{"query":"foo","mode":"locate","limit":10}"#).unwrap();
        assert_eq!(params.mode, QueryMode::Locate);
        assert_eq!(params.limit, Some(10));
    }

    #[test]
    fn the_method_list_has_no_duplicates() {
        let mut sorted = method::ALL.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "duplicate method name in method::ALL");
    }
}
