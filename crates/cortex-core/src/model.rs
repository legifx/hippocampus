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
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Chunk => "chunk",
            NodeKind::Concept => "concept",
            NodeKind::Annotation => "annotation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "file" => NodeKind::File,
            "dir" => NodeKind::Dir,
            "chunk" => NodeKind::Chunk,
            "concept" => NodeKind::Concept,
            "annotation" => NodeKind::Annotation,
            _ => return None,
        })
    }
}

/// Memory tier. Demotion is how the system forgets without losing anything.
///
/// Nothing is ever deleted by decay — it moves down. `Cold` keeps path and stat only and is
/// read exclusively on an explicit deep search. Because the index is reconstructible, a
/// demotion is always reversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
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
    pub zone_id: Option<ZoneId>,
    /// A short excerpt, when the hit came from content rather than from the path.
    pub snippet: Option<String>,
    pub score: ScoreBreakdown,
}

/// Which retrieval strategy a query asks for.
///
/// The mode is guessed heuristically and can be forced with a prefix. `Locate` is roughly
/// ninety per cent of real usage, which is why it must be the fastest path in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    /// Path and name, purely lexical.
    #[default]
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
