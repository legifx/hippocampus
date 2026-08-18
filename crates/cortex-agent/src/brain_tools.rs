//! The `brain.*` tools: everything an agent may write that is not a file.
//!
//! These are gated by the kill switch and audited like every other tool, but not by the write
//! allow-list. Rule R6 draws the line there deliberately: an agent's note *about* a document
//! cannot damage the document, and a brain an agent may read but never write is a brain that
//! never learns anything from the agent using it.
//!
//! What they must respect instead is rule **R10**. An annotation is not derivable from the
//! filesystem, so every assertion is written to `episodes` — the one table nothing rewrites —
//! carrying the *path* of its target rather than only its id, because a rebuild renumbers every
//! node. `Store::replay_assertions` puts the graph back from that log, and there is a test in
//! the storage crate that proves it.

use crate::audit::Auditor;
use crate::fs_tools::{Surface, ToolError, ToolResult};
use crate::policy::Refusal;
use cortex_core::ids::NodeId;
use cortex_core::model::{
    Actor, AnnotationView, EdgeSource, EdgeType, Episode, NeighborView, Node,
};
use cortex_core::tools::*;
use cortex_store::Store;
use std::path::PathBuf;

/// How many neighbours a traversal returns unless asked otherwise.
pub const DEFAULT_NEIGHBOR_LIMIT: usize = 100;
/// How far back `brain.working_set` looks.
pub const WORKING_SET_WINDOW_SECS: i64 = 60 * 60 * 4;

impl Surface<'_> {
    // -----------------------------------------------------------------------
    // Writing to memory
    // -----------------------------------------------------------------------

    pub fn annotate(&self, params: &AnnotateParams) -> ToolResult<AnnotateResult> {
        let tool = "brain.annotate";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        if params.note.trim().is_empty() {
            return Err(ToolError::Io {
                path: PathBuf::new(),
                message: "an annotation with no text says nothing".into(),
            });
        }

        let node = self.resolve_target(tool, &args, &params.target)?;
        let path = node.path.display().to_string();
        let annotation = self.store.annotate(
            node.id,
            &path,
            &params.tags,
            &params.note,
            &self.actor,
            self.now,
        )?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("annotation {annotation} on node {}", node.id.0),
        );
        Ok(AnnotateResult {
            annotation_id: annotation,
            target_id: node.id.0,
            target_path: Some(node.path),
        })
    }

    pub fn pin(&self, params: &PinParams, pinned: bool) -> ToolResult<PinResult> {
        let tool = if pinned { "brain.pin" } else { "brain.forget" };
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let node = self.resolve_target(tool, &args, &params.target)?;
        self.store.set_pinned(node.id, pinned)?;
        if !pinned {
            self.store.set_salience(node.id, 0.0)?;
        }

        self.store.record_assertion(
            &self.actor,
            self.now,
            Some(node.id),
            serde_json::json!({
                "assertion": if pinned { "pin" } else { "forget" },
                "target": node.id.0,
                "target_path": node.path.display().to_string(),
            }),
        )?;

        self.auditor
            .allowed(self.now, tool, &args, &format!("node {}", node.id.0));
        Ok(PinResult {
            target_id: node.id.0,
            pinned,
            // Saying what a pin does not yet do is worth more than implying it does.
            note: if pinned {
                "pinned: decay and tier demotion will never evict this (the engine itself is M5)"
                    .into()
            } else {
                "salience reset to zero; the file itself is untouched".into()
            },
        })
    }

    pub fn link(&self, params: &LinkParams) -> ToolResult<LinkResult> {
        let tool = "brain.link";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let edge_name = params.edge.as_deref().unwrap_or("similar_to");
        let Some(edge) = EdgeType::parse(edge_name) else {
            return Err(ToolError::Io {
                path: PathBuf::new(),
                message: format!(
                    "unknown edge type `{edge_name}`; the schema allows contains, co_access, \
                     imports, similar_to, derived_from, annotates"
                ),
            });
        };

        let a = self.resolve_target(tool, &args, &params.a)?;
        let b = self.resolve_target(tool, &args, &params.b)?;
        if a.id == b.id {
            return Err(ToolError::Io {
                path: PathBuf::new(),
                message: "a node cannot be linked to itself".into(),
            });
        }

        if params.remove {
            self.store.remove_edge(a.id, b.id, edge)?;
        } else {
            // `Explicit`: an assertion by a human or an agent, which no inference process may
            // prune. That is the whole difference between this and an edge the indexer guessed.
            self.store
                .insert_edge(a.id, b.id, edge, 1.0, EdgeSource::Explicit, self.now)?;
        }

        self.store.record_assertion(
            &self.actor,
            self.now,
            Some(a.id),
            serde_json::json!({
                "assertion": if params.remove { "unlink" } else { "link" },
                "a": a.id.0,
                "b": b.id.0,
                "a_path": a.path.display().to_string(),
                "b_path": b.path.display().to_string(),
                "edge": edge.as_str(),
            }),
        )?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("{} {} {}", a.id.0, edge.as_str(), b.id.0),
        );
        Ok(LinkResult {
            a_id: a.id.0,
            b_id: b.id.0,
            edge: edge.as_str().to_string(),
            removed: params.remove,
        })
    }

    // -----------------------------------------------------------------------
    // Reading memory
    // -----------------------------------------------------------------------

    pub fn neighbors(&self, params: &NeighborsParams) -> ToolResult<NeighborsResult> {
        let tool = "brain.neighbors";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let node = self.resolve_target(tool, &args, &params.target)?;
        let types: Vec<EdgeType> = params
            .edge_types
            .iter()
            .filter_map(|t| EdgeType::parse(t))
            .collect();

        let found = self.store.neighbors(
            node.id,
            &types,
            params.depth.unwrap_or(1),
            params.limit.unwrap_or(DEFAULT_NEIGHBOR_LIMIT),
        )?;
        let annotations = self.store.annotations_for(node.id)?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &crate::audit::summarize("neighbours", found.len(), None),
        );
        Ok(NeighborsResult {
            target_id: node.id.0,
            neighbors: found
                .into_iter()
                .map(|n| NeighborView {
                    node_id: n.node_id,
                    path: n.path,
                    kind: n.kind,
                    edge: n.edge,
                    source: n.source,
                    weight: n.weight,
                    depth: n.depth,
                    note: n.note,
                })
                .collect(),
            annotations: annotations
                .into_iter()
                .map(|a| AnnotationView {
                    node_id: a.node_id,
                    target_id: a.target_id,
                    tags: a.tags,
                    note: a.note,
                    actor: a.actor,
                    ts: a.ts,
                })
                .collect(),
        })
    }

    pub fn timeline(&self, params: &TimelineParams) -> ToolResult<TimelineResult> {
        let tool = "brain.timeline";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let episodes = self.store.episodes_in_range(
            params.from,
            params.to,
            params.actor.as_deref(),
            params.action.as_deref(),
            params.limit.unwrap_or(200).min(5000),
        )?;
        let total = self.store.episode_count()?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &crate::audit::summarize("episodes", episodes.len(), None),
        );
        Ok(TimelineResult { episodes, total })
    }

    /// What the human is doing right now, as far as anything here can tell.
    ///
    /// Recent episodes, and it says so. Working memory proper needs salience, which is M5; a
    /// tool that returned this and called it a working set would be claiming a faculty the
    /// daemon does not have yet.
    pub fn working_set(&self, idle_secs: Option<u64>) -> ToolResult<WorkingSetResult> {
        let tool = "brain.working_set";
        let args = serde_json::json!({});
        self.gate_args(tool, &args)?;

        let since = self.now - WORKING_SET_WINDOW_SECS;
        let entries = self.store.recently_touched(since, 50)?;

        let nodes: Vec<WorkingSetEntry> = entries
            .into_iter()
            .map(|t| WorkingSetEntry {
                node_id: t.node_id,
                path: t.path,
                last_touched: t.last_touched,
                actions: t.actions,
                pinned: t.pinned,
            })
            .collect();

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &crate::audit::summarize("nodes", nodes.len(), None),
        );
        Ok(WorkingSetResult {
            nodes,
            idle_secs,
            basis: format!(
                "episodes from the last {} hours; salience-weighted working memory is M5",
                WORKING_SET_WINDOW_SECS / 3600
            ),
        })
    }

    /// What changed, for an agent that should not poll.
    pub fn diff(&self, params: &DiffParams) -> ToolResult<DiffResult> {
        let tool = "brain.diff";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let limit = params.limit.unwrap_or(500).min(5000);
        // One more than asked for, so `more` is a fact rather than a guess.
        let mut changes = self
            .store
            .episodes_after(params.since.unwrap_or(0), limit + 1)?;
        let more = changes.len() > limit;
        changes.truncate(limit);

        let cursor = changes
            .last()
            .map(|e| e.id.0)
            .unwrap_or(params.since.unwrap_or(0));

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &crate::audit::summarize("changes", changes.len(), None),
        );
        Ok(DiffResult {
            changes,
            cursor,
            more,
        })
    }

    // -----------------------------------------------------------------------
    // Shared
    // -----------------------------------------------------------------------

    /// Turn a path or an id into the node it names.
    ///
    /// A path that is not indexed is a refusal rather than an implicit index: indexing is the
    /// daemon's job and happens under rules an agent does not get to shortcut (R1, R3).
    fn resolve_target(
        &self,
        tool: &str,
        args: &serde_json::Value,
        target: &Target,
    ) -> ToolResult<Node> {
        let found = match target {
            Target::Node(id) => self.store.node_by_id(NodeId(*id))?,
            Target::Path(path) => {
                // The deny-list first: an agent must not learn that a path exists by annotating
                // it, any more than by reading it.
                let resolved = match self.policy.check_list(path) {
                    Ok(p) => p,
                    Err(r) => return Err(self.auditor.refuse(self.now, tool, args, r).into()),
                };
                self.store.node_by_path(&resolved)?
            }
        };

        found.ok_or_else(|| {
            self.auditor.refuse(
                self.now,
                tool,
                args,
                Refusal::Unresolvable {
                    path: PathBuf::from(target.describe()),
                    detail: "not in the index; only indexed nodes can be annotated or linked"
                        .into(),
                },
            )
            .into()
        })
    }
}

/// Read the actor a store call should be attributed to. Kept as a function so the daemon and
/// the tests build it the same way.
pub fn auditor_for(store: std::sync::Arc<Store>, actor: &Actor) -> Auditor {
    Auditor::new(store, actor)
}

/// The episodes a timeline answer is made of, re-exported so callers need not name the store.
pub type TimelineEpisode = Episode;
