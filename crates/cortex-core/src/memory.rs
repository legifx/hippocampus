//! What an agent knows that is not in a file: the scope it belongs to, the kind it is, and the
//! shape a context pack has.
//!
//! These are domain types with no storage in them, for the same reason the rest of
//! [`crate::tools`] is here: three surfaces share the vocabulary and the CLI has no business
//! linking the storage crate.
//!
//! The one line that governs the whole module is D74: **a memory is a node.** Nothing here
//! duplicates salience, tiering or decay — a memory gets those from `nodes` like everything
//! else. What is genuinely new is the scope, and it is new because a shared memory pool without
//! one fails in a specific way: one agent's wrong guess quietly becomes every agent's premise.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Who a memory belongs to.
///
/// Ordered from narrowest to widest, and that order is load-bearing: a write defaults to the
/// narrowest scope that can hold it, and [`MemoryScope::visible_from`] is what stops one agent
/// reading another's.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum MemoryScope {
    /// Scratch. Expires with the session that wrote it, and is the only scope with a TTL.
    Session(String),
    /// Private to one agent identity. The default for a write, because it is the narrowest
    /// scope that survives the session.
    Agent(String),
    /// Shared by everything working in one zone. The zone is the daemon's own, not a path the
    /// caller chose, so two agents naming the same project differently still share it.
    Project(i64),
    /// Everything sees it. Requires an explicit argument to write; never a default.
    Global,
}

impl MemoryScope {
    /// The stored form. Parsed by [`MemoryScope::parse`], and the two are tested against each
    /// other for every variant — a scope that round-trips wrongly is a scope leak.
    pub fn as_str(&self) -> String {
        match self {
            MemoryScope::Session(id) => format!("session:{id}"),
            MemoryScope::Agent(name) => format!("agent:{name}"),
            MemoryScope::Project(zone) => format!("project:{zone}"),
            MemoryScope::Global => "global".to_string(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "global" => MemoryScope::Global,
            other => {
                let (tag, rest) = other.split_once(':')?;
                match tag {
                    "session" if !rest.is_empty() => MemoryScope::Session(rest.to_string()),
                    "agent" if !rest.is_empty() => MemoryScope::Agent(rest.to_string()),
                    "project" => MemoryScope::Project(rest.parse().ok()?),
                    _ => return None,
                }
            }
        })
    }

    /// Whether a reader in `reader` may see a memory in `self`.
    ///
    /// The rule, and it is deliberately blunt: **an agent scope is visible only to itself.**
    /// Everything else is visible to anyone who is in the same session, the same project, or
    /// asking for global. A rule with exceptions is a rule that grows exceptions.
    pub fn visible_from(&self, reader: &ReaderScope) -> bool {
        match self {
            MemoryScope::Global => true,
            MemoryScope::Agent(name) => reader.agent.as_deref() == Some(name.as_str()),
            MemoryScope::Session(id) => reader.session.as_deref() == Some(id.as_str()),
            MemoryScope::Project(zone) => reader.projects.contains(zone),
        }
    }

    /// True for the one scope that is not written without being asked for.
    pub fn needs_explicit_request(&self) -> bool {
        matches!(self, MemoryScope::Global)
    }
}

impl fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// Who is reading, expanded into the scopes they may see.
///
/// Built by the daemon from the caller's identity and never by the caller: a reader that could
/// name its own agent scope could name somebody else's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReaderScope {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    /// Zone ids the reader is working in. Usually one; more when a task spans projects.
    #[serde(default)]
    pub projects: Vec<i64>,
}

impl ReaderScope {
    pub fn for_agent(name: impl Into<String>) -> Self {
        Self {
            agent: Some(name.into()),
            ..Default::default()
        }
    }

    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    pub fn with_project(mut self, zone: i64) -> Self {
        if !self.projects.contains(&zone) {
            self.projects.push(zone);
        }
        self
    }

    /// The SQL `IN` list this reader may see, as stored scope strings.
    ///
    /// Produced here rather than as a `WHERE` clause built by string concatenation at three
    /// call sites, because the third one is the one that forgets the agent check.
    pub fn visible_scopes(&self) -> Vec<String> {
        let mut out = vec![MemoryScope::Global.as_str()];
        if let Some(agent) = &self.agent {
            out.push(MemoryScope::Agent(agent.clone()).as_str());
        }
        if let Some(session) = &self.session {
            out.push(MemoryScope::Session(session.clone()).as_str());
        }
        for zone in &self.projects {
            out.push(MemoryScope::Project(*zone).as_str());
        }
        out
    }
}

/// What kind of thing a memory is.
///
/// This is a retrieval prior, not a separate index. A `Failure` should surface when an agent is
/// about to repeat it; a `Preference` should surface always, because it is three words long and
/// its entire value is being present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something that is the case. "The test corpus is generated, not checked in."
    Fact,
    /// Something the human wants. Small, cheap, and always included in a pack.
    Preference,
    /// How to do something. "Run the hygiene check before committing, never with --no-verify."
    Procedure,
    /// A choice and its reason. The closest thing to a decision-log entry an agent can write.
    Decision,
    /// Something that was tried and did not work. Worth more than most facts, because the cost
    /// of rediscovering it was already paid once.
    Failure,
    /// Something noticed that may or may not matter. The default when nothing else fits, and
    /// the kind that decays first.
    Observation,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Preference => "preference",
            MemoryKind::Procedure => "procedure",
            MemoryKind::Decision => "decision",
            MemoryKind::Failure => "failure",
            MemoryKind::Observation => "observation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fact" => MemoryKind::Fact,
            "preference" => MemoryKind::Preference,
            "procedure" => MemoryKind::Procedure,
            "decision" => MemoryKind::Decision,
            "failure" => MemoryKind::Failure,
            "observation" => MemoryKind::Observation,
            _ => return None,
        })
    }

    /// Included in every context pack regardless of the query, budget permitting.
    ///
    /// Only preferences. The temptation is to add `Failure` here — an agent about to repeat a
    /// mistake would benefit — but failures are numerous and long, and a rule that fills the
    /// budget before the query is considered is a rule that makes every pack worse.
    pub fn is_unconditional(self) -> bool {
        matches!(self, MemoryKind::Preference)
    }

    pub const ALL: [MemoryKind; 6] = [
        MemoryKind::Fact,
        MemoryKind::Preference,
        MemoryKind::Procedure,
        MemoryKind::Decision,
        MemoryKind::Failure,
        MemoryKind::Observation,
    ];
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One memory, as it is stored and as every surface reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub node_id: i64,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub text: String,
    /// The node this memory is *about*, if any. An `annotates` edge is written alongside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_path: Option<std::path::PathBuf>,
    /// The writer's own claim about how sure it is. **Never treated as evidence** — a model
    /// asserting 0.95 has asserted a number, not established one. It is shown to the human and
    /// it breaks ties; that is all it does.
    pub confidence: f64,
    /// Earned, by [`Outcome`]. This is the one score in the system that comes from being useful
    /// rather than from being touched — see D76 for why the distinction matters.
    pub usefulness: f64,
    pub helped_count: i64,
    pub noise_count: i64,
    /// How often this memory has been *put into a pack*. Counted separately from the two above
    /// because the ratio is the signal: retrieved fifty times and never reported is a memory
    /// costing tokens for nothing.
    pub retrieved_count: i64,
    pub human_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub actor: String,
    pub ts: i64,
    /// Salience as the memory engine last computed it, carried here so a listing does not need
    /// a second query per row.
    #[serde(default)]
    pub salience: f64,
    #[serde(default)]
    pub pinned: bool,
}

impl Memory {
    /// Whether this memory may be returned by an ordinary retrieval.
    pub fn is_live(&self, now: i64) -> bool {
        self.superseded_by.is_none() && self.expires_at.map(|at| at > now).unwrap_or(true)
    }
}

/// Who reported an outcome, and therefore how much it counts.
///
/// An agent reporting on its own success is an unreliable narrator. It is not ignored — it is
/// discounted, and which of the three moved a number is printed by `brain.explain` (D76).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reporter {
    /// A human said so. The only report treated as evidence at full weight.
    Human,
    /// The agent that used the pack said so.
    Agent,
}

impl Reporter {
    pub fn weight(self) -> f64 {
        match self {
            Reporter::Human => 1.0,
            Reporter::Agent => 0.35,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Reporter::Human => "human",
            Reporter::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "human" => Reporter::Human,
            "agent" => Reporter::Agent,
            _ => return None,
        })
    }
}

/// What a retrieval arm was, per pack item, so that a pack can explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackArm {
    /// A preference or a pin, included because it is included.
    Unconditional,
    /// From the working set: what the human is doing right now.
    WorkingSet,
    /// The lexical arm over memory text.
    Lexical,
    /// The vector arm over the memory collection.
    Semantic,
    /// Reached by following an edge from something else in the pack.
    Associative,
    /// A file a memory in this pack points at.
    Subject,
}

impl PackArm {
    pub fn as_str(self) -> &'static str {
        match self {
            PackArm::Unconditional => "unconditional",
            PackArm::WorkingSet => "working_set",
            PackArm::Lexical => "lexical",
            PackArm::Semantic => "semantic",
            PackArm::Associative => "associative",
            PackArm::Subject => "subject",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unconditional" => PackArm::Unconditional,
            "working_set" => PackArm::WorkingSet,
            "lexical" => PackArm::Lexical,
            "semantic" => PackArm::Semantic,
            "associative" => PackArm::Associative,
            "subject" => PackArm::Subject,
            _ => return None,
        })
    }
}

/// Why one item is in the pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basis {
    pub arm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lexical_rank: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_rank: Option<u32>,
    pub fused: f64,
    // Everything below is omitted when it is zero, and absent means zero. A memory nobody has
    // reported on yet — which on a fresh install is every memory — carried five fields saying
    // so, on every item of every pack. The basis is there to be argued with, and "helped: 0,
    // noise: 0, retrieved: 0" is not an argument, it is the absence of one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub salience: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub usefulness: f64,
    pub confidence: f64,
    /// Who wrote it, and when. A memory an agent cannot attribute is one it cannot argue with.
    pub actor: String,
    pub written_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_count")]
    pub helped: i64,
    #[serde(default, skip_serializing_if = "is_zero_count")]
    pub noise: i64,
    #[serde(default, skip_serializing_if = "is_zero_count")]
    pub retrieved: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub human_confirmed: bool,
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn is_zero_count(value: &i64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackItem {
    pub node_id: i64,
    pub arm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MemoryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
    /// The memory itself — **not sent on the wire**, because `rendered` already carries it.
    ///
    /// A pack exists to fit inside a token budget, and it was spending that budget twice: once
    /// in `rendered`, which is the deliverable, and once again in an item beside it. An MCP
    /// client then receives the whole answer twice more — pretty-printed as a text block and
    /// verbatim as structured content — so a 2 048-token budget cost an agent something closer
    /// to eight thousand. A feature whose entire purpose is a budget cannot be four times over
    /// it.
    ///
    /// The field stays in the struct because [`super::memory::ContextPack`]'s renderer needs it
    /// on the way out, and it is skipped on the way to anybody. What a caller gets per item is
    /// what it needs to *report* on and to *explain*: the id, the arm, the kind, the scope, the
    /// path and the basis.
    #[serde(skip)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_range: Option<(i64, i64)>,
    pub tokens: usize,
    pub basis: Basis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPack {
    pub pack_id: String,
    pub task: String,
    pub budget_tokens: usize,
    /// What the pack is estimated to cost. An estimate on purpose, and biased upwards — see
    /// `cortex_core::tokens` and D79.
    pub used_tokens: usize,
    pub items: Vec<PackItem>,
    /// Candidates that were found and did not fit. A pack that silently drops half its
    /// candidates and one that had nothing to say look identical without this.
    pub dropped: usize,
    /// Things the caller should know: a budget too small for the preferences, a missing model,
    /// a scope with nothing in it.
    pub notes: Vec<String>,
    /// The block, formatted and ready to paste into a prompt.
    pub rendered: String,
}

impl ContextPack {
    /// The memories in it — the only items an outcome can be reported against.
    pub fn memory_ids(&self) -> Vec<crate::ids::NodeId> {
        self.items
            .iter()
            .filter(|item| item.kind.is_some())
            .map(|item| crate::ids::NodeId(item.node_id))
            .collect()
    }
}


/// One agent's run against the surface.
///
/// Here rather than in the storage crate because three surfaces report it and one of them —
/// the CLI — has no business linking storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    pub calls: i64,
    pub refusals: i64,
    /// Memories written during it. Derived, and the number a human actually wants.
    #[serde(default)]
    pub memories: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_round_trips_through_its_stored_form() {
        for scope in [
            MemoryScope::Session("s-1".into()),
            MemoryScope::Agent("claude".into()),
            MemoryScope::Project(42),
            MemoryScope::Global,
        ] {
            let text = scope.as_str();
            assert_eq!(
                MemoryScope::parse(&text),
                Some(scope.clone()),
                "scope {text} did not survive a round trip"
            );
        }
    }

    #[test]
    fn a_scope_with_no_identifier_is_not_a_scope() {
        // `agent:` with nothing after it would otherwise parse into an agent named "", which
        // every other agent named "" would then share.
        assert_eq!(MemoryScope::parse("agent:"), None);
        assert_eq!(MemoryScope::parse("session:"), None);
        assert_eq!(MemoryScope::parse("project:not-a-number"), None);
        assert_eq!(MemoryScope::parse("nonsense"), None);
    }

    #[test]
    fn one_agents_scope_is_invisible_to_another() {
        let mine = MemoryScope::Agent("a".into());
        assert!(mine.visible_from(&ReaderScope::for_agent("a")));
        assert!(!mine.visible_from(&ReaderScope::for_agent("b")));
        // And to a reader with no agent identity at all, which is what the CLI is.
        assert!(!mine.visible_from(&ReaderScope::default()));
    }

    #[test]
    fn a_session_scope_belongs_to_its_session_and_not_to_its_agent() {
        // The trap: an agent that wrote session scratch under its own name, then reconnected
        // with a new session, would otherwise still see the scratch it meant to throw away.
        let scratch = MemoryScope::Session("s-1".into());
        let same_agent_new_session = ReaderScope::for_agent("a").with_session("s-2");
        assert!(!scratch.visible_from(&same_agent_new_session));
        assert!(scratch.visible_from(&ReaderScope::for_agent("a").with_session("s-1")));
    }

    #[test]
    fn global_is_visible_to_everyone_including_nobody() {
        assert!(MemoryScope::Global.visible_from(&ReaderScope::default()));
    }

    #[test]
    fn the_visible_list_never_contains_another_agents_scope() {
        let reader = ReaderScope::for_agent("a").with_session("s-1").with_project(7);
        let visible = reader.visible_scopes();
        assert!(visible.contains(&"global".to_string()));
        assert!(visible.contains(&"agent:a".to_string()));
        assert!(visible.contains(&"session:s-1".to_string()));
        assert!(visible.contains(&"project:7".to_string()));
        assert!(!visible.iter().any(|s| s == "agent:b"));
        assert_eq!(visible.len(), 4);
    }

    #[test]
    fn every_memory_kind_round_trips() {
        for kind in MemoryKind::ALL {
            assert_eq!(MemoryKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn only_preferences_are_unconditional() {
        let unconditional: Vec<_> = MemoryKind::ALL
            .into_iter()
            .filter(|k| k.is_unconditional())
            .collect();
        assert_eq!(unconditional, vec![MemoryKind::Preference]);
    }

    #[test]
    fn a_human_report_outweighs_an_agents_report_on_itself() {
        assert!(Reporter::Human.weight() > Reporter::Agent.weight());
        assert!(Reporter::Agent.weight() > 0.0, "discounted, not ignored");
    }

    #[test]
    fn a_superseded_memory_is_not_live_and_neither_is_an_expired_one() {
        let base = Memory {
            node_id: 1,
            scope: MemoryScope::Global,
            kind: MemoryKind::Fact,
            text: "x".into(),
            subject_id: None,
            subject_path: None,
            confidence: 0.5,
            usefulness: 0.0,
            helped_count: 0,
            noise_count: 0,
            retrieved_count: 0,
            human_confirmed: false,
            expires_at: None,
            superseded_by: None,
            session_id: None,
            actor: "agent:a".into(),
            ts: 0,
            salience: 0.0,
            pinned: false,
        };
        assert!(base.is_live(100));
        assert!(!Memory {
            superseded_by: Some(2),
            ..base.clone()
        }
        .is_live(100));
        assert!(!Memory {
            expires_at: Some(50),
            ..base.clone()
        }
        .is_live(100));
        assert!(Memory {
            expires_at: Some(150),
            ..base
        }
        .is_live(100));
    }
}
