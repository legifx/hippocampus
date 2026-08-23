//! The domain model.
//!
//! Vocabulary here is binding — see `docs/00-GLOSSARY.md`. Do not introduce a synonym for a
//! term that already exists.

use crate::ids::{NodeId, SessionId, ZoneId};
use std::path::PathBuf;

/// What a node *is*. A node is not necessarily a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Dir,
    Chunk,
    Concept,
    Annotation,
    /// What is known and is not in a file. Has text and a scope, never a path. See M11 and D74:
    /// this is a node kind rather than a second store so that salience, decay, tiering,
    /// consolidation and `explain` apply to it without being written twice.
    Memory,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Chunk => "chunk",
            NodeKind::Concept => "concept",
            NodeKind::Annotation => "annotation",
            NodeKind::Memory => "memory",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "file" => NodeKind::File,
            "dir" => NodeKind::Dir,
            "chunk" => NodeKind::Chunk,
            "concept" => NodeKind::Concept,
            "annotation" => NodeKind::Annotation,
            "memory" => NodeKind::Memory,
            _ => return None,
        })
    }
}

/// Memory tier. Demotion is how the system forgets without losing anything.
///
/// Nothing is ever deleted by decay — it moves down. `Cold` keeps path and stat only and is
/// read exclusively on an explicit deep search. Because the index is reconstructible, a
/// demotion is always reversible.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// In the Working Set: the human is touching this right now.
    Working,
    /// Short-term memory: fully indexed, chunked and embedded.
    Stm,
    /// Long-term memory: consolidated, condensed, still locally searchable.
    Ltm,
    /// Metadata only. No embeddings.
    Cold,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Working => "working",
            Tier::Stm => "stm",
            Tier::Ltm => "ltm",
            Tier::Cold => "cold",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "working" => Tier::Working,
            "stm" => Tier::Stm,
            "ltm" => Tier::Ltm,
            "cold" => Tier::Cold,
            _ => return None,
        })
    }
}

/// Who caused an episode.
///
/// Attribution is not cosmetic: agent mutations must be distinguishable from human ones so
/// that the undo journal and the audit trail mean something.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum Actor {
    Human,
    Agent(String),
    System,
}

impl Actor {
    pub fn as_str(&self) -> String {
        match self {
            Actor::Human => "human".to_string(),
            Actor::Agent(name) => format!("agent:{name}"),
            Actor::System => "system".to_string(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "human" => Actor::Human,
            "system" => Actor::System,
            other => Actor::Agent(other.strip_prefix("agent:")?.to_string()),
        })
    }
}

/// What happened. Episodes are the raw material for every derived structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Modify,
    Open,
    Move,
    Delete,
    Search,
    Annotate,
    /// M11 assertions. `remember` and `forget_memory` are the authority for the memory layer:
    /// the `memories` row is a projection of them, so a rebuild replays both. A deletion that
    /// is not an episode returns the first time the index is rebuilt (D78).
    Remember,
    Supersede,
    ForgetMemory,
    /// A context pack's outcome report. Not an assertion — it moves a learned score, and a
    /// score is derived. It is an episode so that the Inspector can show who reported what.
    Outcome,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Modify => "modify",
            Action::Open => "open",
            Action::Move => "move",
            Action::Delete => "delete",
            Action::Search => "search",
            Action::Annotate => "annotate",
            Action::Remember => "remember",
            Action::Supersede => "supersede",
            Action::ForgetMemory => "forget_memory",
            Action::Outcome => "outcome",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "create" => Action::Create,
            "modify" => Action::Modify,
            "open" => Action::Open,
            "move" => Action::Move,
            "delete" => Action::Delete,
            "search" => Action::Search,
            "annotate" => Action::Annotate,
            "remember" => Action::Remember,
            "supersede" => Action::Supersede,
            "forget_memory" => Action::ForgetMemory,
            "outcome" => Action::Outcome,
            _ => return None,
        })
    }
}

/// A memory entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub path: PathBuf,
    pub parent_id: Option<NodeId>,
    pub mime: Option<String>,
    pub size: i64,
    /// Seconds since the Unix epoch. Stored as an integer because sub-second precision on
    /// mtime is not portable across the filesystems this has to work on.
    pub mtime: i64,
    pub ctime: i64,
    pub inode: i64,
    pub dev: i64,
    /// BLAKE3 of the content, hex encoded. `None` for directories and for nodes that have
    /// never been read (metadata-only tiers).
    pub content_hash: Option<String>,
    pub tier: Tier,
    pub salience: f64,
    pub pinned: bool,
    pub zone_id: Option<ZoneId>,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// A timestamped, immutable event.
///
/// This is the only structure in the system that is never updated after creation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Episode {
    pub id: crate::ids::EpisodeId,
    pub ts: i64,
    pub actor: Actor,
    pub action: Action,
    pub node_id: Option<NodeId>,
    pub session_id: Option<SessionId>,
    /// Free-form, action-specific detail. Kept as JSON because the shape genuinely differs
    /// per action and a normalised table here would be a table with thirty nullable columns.
    pub metadata: serde_json::Value,
}

/// How a search result was produced. Every result must be able to explain itself.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoreBreakdown {
    /// Rank contributed by trigram path matching, if it matched at all.
    pub path_rank: Option<u32>,
    /// Rank contributed by lexical (BM25) retrieval.
    pub lexical_rank: Option<u32>,
    /// Rank contributed by vector similarity.
    pub vector_rank: Option<u32>,
    /// Score after reciprocal rank fusion, before any boost.
    pub fused: f64,
    /// Multiplicative salience boost that was applied.
    pub salience_boost: f64,
    /// Score after the cross-encoder reranked this result, if reranking has run yet.
    pub rerank: Option<f64>,
    /// The score actually used for ordering.
    pub final_score: f64,
}

/// One search result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub node_id: NodeId,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub tier: Tier,
    /// Bytes, and seconds since the epoch.
    ///
    /// A result list that cannot say how big a file is or when it changed makes the reader open
    /// candidates to find out, which is the work the search exists to save. Both are already on
    /// the node the hit came from, so carrying them costs the answer sixteen bytes.
    pub size: i64,
    pub mtime: i64,
    pub zone_id: Option<ZoneId>,
    /// A short excerpt, when the hit came from content rather than from the path.
    pub snippet: Option<String>,
    pub score: ScoreBreakdown,
}

/// How far content extraction has got with a file.
///
/// Stored on the node so the extraction queue can be *resumed* rather than recomputed. Without
/// it, every restart would have to re-classify the whole index to find out what is left to do,
/// which on a large corpus is the same cost as extracting again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractState {
    /// Never looked at, or the file changed since it was.
    Pending,
    /// Extracted and chunked. Says nothing about how much text was found — a file may legally
    /// produce zero chunks.
    Done,
    /// Deliberately not read: denied, outside the content roots, binary, or too large. This is
    /// a normal outcome, not a failure, and it must not be retried on every pass.
    Skipped,
    /// Reading was attempted and did not work. Distinguished from `Skipped` because a failure
    /// is worth retrying after the file changes, and worth reporting when it does not.
    Failed,
}

impl ExtractState {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractState::Pending => "pending",
            ExtractState::Done => "done",
            ExtractState::Skipped => "skipped",
            ExtractState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => ExtractState::Pending,
            "done" => ExtractState::Done,
            "skipped" => ExtractState::Skipped,
            "failed" => ExtractState::Failed,
            _ => return None,
        })
    }
}

/// Which retrieval strategy a query asks for.
///
/// The mode is guessed heuristically and can be forced with a prefix. `Locate` is roughly
/// ninety per cent of real usage, which is why it must be the fastest path in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    /// Not specified: let the daemon detect it. The default, and what every client sends
    /// unless the user forced a mode, because the heuristic belongs in one place rather than
    /// in each client.
    #[default]
    Auto,
    /// Path and name, purely lexical.
    Locate,
    /// Semantic recall — pattern completion from a fragment.
    Recall,
    /// Episodic: "what was I working on on Tuesday". Needs no vectors at all.
    Temporal,
    /// Graph traversal from a starting node.
    Structural,
    /// Includes the cold tier and any federated peer. Explicitly triggered, may take seconds.
    Deep,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_round_trips_through_its_string_form() {
        // The string form is what lands in SQLite, so a round trip failure here silently
        // corrupts attribution rather than failing loudly.
        for actor in [
            Actor::Human,
            Actor::System,
            Actor::Agent("claude".into()),
            Actor::Agent("some-agent-with-dashes".into()),
        ] {
            let s = actor.as_str();
            assert_eq!(Actor::parse(&s), Some(actor.clone()), "round trip failed for {s}");
        }
    }

    #[test]
    fn enum_string_forms_round_trip() {
        for kind in [
            NodeKind::File,
            NodeKind::Dir,
            NodeKind::Chunk,
            NodeKind::Concept,
            NodeKind::Annotation,
        ] {
            assert_eq!(NodeKind::parse(kind.as_str()), Some(kind));
        }
        for tier in [Tier::Working, Tier::Stm, Tier::Ltm, Tier::Cold] {
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
        }
        for state in [
            ExtractState::Pending,
            ExtractState::Done,
            ExtractState::Skipped,
            ExtractState::Failed,
        ] {
            // These strings are also a CHECK constraint in migration 0002, so a typo here is
            // not a display bug — it is an insert that the database refuses at runtime.
            assert_eq!(ExtractState::parse(state.as_str()), Some(state));
        }
        for action in [
            Action::Create,
            Action::Modify,
            Action::Open,
            Action::Move,
            Action::Delete,
            Action::Search,
            Action::Annotate,
        ] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
    }

    #[test]
    fn unknown_strings_are_rejected_rather_than_defaulted() {
        assert_eq!(NodeKind::parse("directory"), None);
        assert_eq!(Tier::parse("hot"), None);
        assert_eq!(Action::parse("touch"), None);
    }

    #[test]
    fn tiers_order_from_hot_to_cold() {
        assert!(Tier::Working < Tier::Stm);
        assert!(Tier::Stm < Tier::Ltm);
        assert!(Tier::Ltm < Tier::Cold);
    }
}


/// How two nodes are connected. The associative index proper — this is the part of the schema
/// the hippocampus analogy is actually about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// A directory contains a file.
    Contains,
    /// Opened together, close in time.
    CoAccess,
    /// One file references another.
    Imports,
    /// Near-duplicate or semantically close.
    SimilarTo,
    /// One was produced from the other.
    DerivedFrom,
    /// An annotation about a node.
    Annotates,
    /// One memory replaced another. The old one keeps its row and leaves retrieval (M11).
    Supersedes,
    /// Two nodes were in the same context pack. Weak, observed, and the raw material for
    /// co-access between things an agent used together rather than things a human opened
    /// together.
    RecalledWith,
}

impl EdgeType {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeType::Contains => "contains",
            EdgeType::CoAccess => "co_access",
            EdgeType::Imports => "imports",
            EdgeType::SimilarTo => "similar_to",
            EdgeType::DerivedFrom => "derived_from",
            EdgeType::Annotates => "annotates",
            EdgeType::Supersedes => "supersedes",
            EdgeType::RecalledWith => "recalled_with",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "contains" => EdgeType::Contains,
            "co_access" => EdgeType::CoAccess,
            "imports" => EdgeType::Imports,
            "similar_to" => EdgeType::SimilarTo,
            "derived_from" => EdgeType::DerivedFrom,
            "annotates" => EdgeType::Annotates,
            "supersedes" => EdgeType::Supersedes,
            "recalled_with" => EdgeType::RecalledWith,
            _ => return None,
        })
    }
}

/// How the index came to believe an edge.
///
/// `Explicit` edges are user or agent assertions and must never be pruned by a process that
/// only trusts its own inferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeSource {
    Observed,
    Inferred,
    Explicit,
}

impl EdgeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeSource::Observed => "observed",
            EdgeSource::Inferred => "inferred",
            EdgeSource::Explicit => "explicit",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "observed" => EdgeSource::Observed,
            "inferred" => EdgeSource::Inferred,
            "explicit" => EdgeSource::Explicit,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Agent Surface records. Neither is derived from the filesystem: they are history about
// callers, not about files, which is why they live beside the domain types rather than in the
// index proper.
// ---------------------------------------------------------------------------

/// One recorded access to the Agent Surface, allowed or refused.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    /// `human`, `system` or `agent:<name>`.
    pub actor: String,
    pub tool: String,
    /// The arguments as the caller sent them, after redaction.
    pub args: serde_json::Value,
    /// For an allowed call, what came back — a shape and a size, never the payload. For a
    /// refusal, the reason.
    pub result_summary: Option<String>,
    pub allowed: bool,
}

/// A mutation and its inverse, written before the mutation is applied.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UndoEntry {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub operation: serde_json::Value,
    pub inverse: serde_json::Value,
    /// False once the inverse has been applied — an undone entry is kept as history, not
    /// deleted, so `cortex undo` cannot silently run twice over the same operation.
    pub applied: bool,
    pub undone_at: Option<i64>,
}

/// One edge, as the Agent Surface reports it. Mirrors the store's own shape so that the wire
/// format does not depend on the storage crate — the CLI must not have to link SQLite to read
/// an answer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NeighborView {
    pub node_id: i64,
    pub path: Option<std::path::PathBuf>,
    pub kind: String,
    pub edge: EdgeType,
    pub source: EdgeSource,
    pub weight: f64,
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One annotation, as the Agent Surface reports it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnnotationView {
    pub node_id: i64,
    pub target_id: i64,
    pub tags: Vec<String>,
    pub note: String,
    pub actor: String,
    pub ts: i64,
}
