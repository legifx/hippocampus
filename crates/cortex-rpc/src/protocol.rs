//! Wire protocol.
//!
//! Every client — GUI, CLI, MCP server, plugins — speaks exactly this. There is no privileged
//! side channel and no second protocol for "internal" clients; that is what keeps the surfaces
//! from drifting apart (build prompt, M3 Definition of Done §5).
//!
//! Framing is newline-delimited JSON: one JSON value per line. It is trivially debuggable with
//! `socat` and needs no length-prefix state machine.

use cortex_core::ids::NodeId;
use cortex_core::model::{Episode, Node, QueryMode, ScoreBreakdown, SearchHit};
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
    pub const EXTRACT_STATUS: &str = "extract.status";
    pub const EXTRACT_REQUEUE: &str = "extract.requeue";
    pub const ACTIVITY_PING: &str = "activity.ping";
    pub const WATCH_STATUS: &str = "watch.status";
    pub const ZONES_LIST: &str = "zones.list";
    pub const ZONE_SET_COLOR: &str = "zone.set_color";
    pub const RECONCILE: &str = "reconcile";
    pub const SHUTDOWN: &str = "shutdown";

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
        ACTIVITY_PING,
        WATCH_STATUS,
        ZONES_LIST,
        ZONE_SET_COLOR,
        RECONCILE,
        SHUTDOWN,
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
