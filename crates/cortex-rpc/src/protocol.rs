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
    pub const FS_READ: &str = "fs.read";
    pub const FS_STAT: &str = "fs.stat";
    pub const FS_WRITE: &str = "fs.write";
    pub const FS_MOVE: &str = "fs.move";
    pub const FS_TRASH: &str = "fs.trash";
    pub const UNDO: &str = "undo";

    /// Every method this build serves. Used by `status` so a client can discover the surface.
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
        FS_READ,
        FS_STAT,
        FS_WRITE,
        FS_MOVE,
        FS_TRASH,
        UNDO,
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
