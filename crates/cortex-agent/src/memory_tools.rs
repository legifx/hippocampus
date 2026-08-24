//! The `brain.remember` family: an agent's own memory, gated and audited like everything else.
//!
//! These write to the brain and never to a file, so rule R6 puts them on the same footing as
//! `brain.annotate`: the kill switch reaches them, the audit log records them, and the write
//! allow-list does not apply. A note an agent writes about a document cannot damage the
//! document, and a brain an agent may read but never write is a brain that learns nothing from
//! the agent using it.
//!
//! **The scope resolution here is the security boundary of the whole milestone.** A caller says
//! which *kind* of scope it wants — session, agent, project, global — and the daemon resolves
//! that against the caller's own identity. A parameter that took a scope string verbatim would
//! let any agent read or write any other's simply by spelling it correctly, and it would look
//! exactly like a convenience while doing it.

use crate::audit::summarize;
use crate::fs_tools::{Surface, ToolError, ToolResult};
use crate::policy::Refusal;
use cortex_core::ids::NodeId;
use cortex_core::memory::{Memory, MemoryKind, MemoryScope, ReaderScope, Reporter};
use cortex_core::model::Actor;
use cortex_core::tools::*;
use cortex_store::memories::{MemoryFilter, NewMemory};
use std::path::Path;

/// How many possible duplicates a write reports back.
const DUPLICATE_HINTS: usize = 3;
/// The default listing size for `brain.memories`.
const DEFAULT_MEMORY_LIMIT: usize = 100;

impl Surface<'_> {
    /// The reader this caller is, expanded into the scopes it may see.
    ///
    /// Built here and never taken from the wire. The `project_paths` a caller names are turned
    /// into zone ids by the daemon's own zone table, so naming a path it cannot read buys
    /// nothing: the zone either exists or the scope is simply absent.
    pub fn reader_scope(&self, session: Option<&str>, project_paths: &[std::path::PathBuf]) -> ReaderScope {
        let mut reader = ReaderScope::default();
        if let Actor::Agent(name) = &self.actor {
            reader.agent = Some(name.clone());
        }
        if let Some(session) = session {
            reader.session = Some(session.to_string());
        }
        for path in project_paths {
            if let Some(zone) = self.zone_of(path) {
                reader = reader.with_project(zone);
            }
        }
        reader
    }

    fn zone_of(&self, path: &Path) -> Option<i64> {
        self.store
            .node_by_path(path)
            .ok()
            .flatten()
            .and_then(|node| node.zone_id)
            .map(|zone| zone.0)
    }

    /// Turn a requested scope kind into the scope this caller may actually write.
    fn resolve_scope(
        &self,
        request: ScopeRequest,
        session: Option<&str>,
        about: Option<&Path>,
    ) -> Result<MemoryScope, Refusal> {
        match request {
            ScopeRequest::Session => match session {
                Some(id) => Ok(MemoryScope::Session(id.to_string())),
                None => Err(Refusal::NotForAgents {
                    what: "writing a session memory without a session_id".into(),
                }),
            },
            ScopeRequest::Agent => match &self.actor {
                Actor::Agent(name) => Ok(MemoryScope::Agent(name.clone())),
                // A human or the daemon itself has no agent scope. Writing to `global` instead
                // would be a silent widening, so it is refused and the caller says what it
                // meant.
                _ => Err(Refusal::NotForAgents {
                    what: "writing an agent-scoped memory without being an agent".into(),
                }),
            },
            ScopeRequest::Project => {
                let path = about.ok_or_else(|| Refusal::NotForAgents {
                    what: "writing a project memory without naming a path in it".into(),
                })?;
                let zone = self.zone_of(path).ok_or_else(|| Refusal::NotForAgents {
                    what: format!("writing a project memory about {}, which is in no zone the index knows", path.display()),
                })?;
                Ok(MemoryScope::Project(zone))
            }
            ScopeRequest::Global => Ok(MemoryScope::Global),
        }
    }

    // -----------------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------------

    pub fn remember(&self, params: &RememberParams) -> ToolResult<RememberResult> {
        let tool = "brain.remember";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let kind = match &params.kind {
            Some(name) => MemoryKind::parse(name).ok_or_else(|| ToolError::Io {
                path: Default::default(),
                message: format!(
                    "{name} is not a memory kind; it is one of fact, preference, procedure, \
                     decision, failure, observation"
                ),
            })?,
            None => MemoryKind::Observation,
        };

        let scope = match self.resolve_scope(
            params.scope,
            params.session_id.as_deref(),
            params.about.as_deref(),
        ) {
            Ok(scope) => scope,
            Err(r) => return self.refuse_public(tool, &args, r),
        };

        // The subject, if it names one. A path the index has never seen is not an error — the
        // memory is still worth having — but it loses the edge, and the result says so by
        // leaving `about` unresolved.
        let subject = params
            .about
            .as_ref()
            .and_then(|path| self.store.node_by_path(path).ok().flatten())
            .map(|node| (node.id, node.path.display().to_string()));

        let text = params.text.trim().to_string();
        let duplicates = self
            .store
            .near_duplicate_memories(&scope, kind, &text, DUPLICATE_HINTS)
            .unwrap_or_default();

        let expires_at = match (&scope, params.ttl_secs) {
            (MemoryScope::Session(_), Some(ttl)) if ttl > 0 => Some(self.now + ttl),
            // Session scope with no TTL still expires: scratch that outlives its session is not
            // scratch. A day is long enough for any single task and short enough to be scratch.
            (MemoryScope::Session(_), _) => Some(self.now + DEFAULT_SESSION_TTL_SECS),
            _ => None,
        };

        let new = NewMemory {
            scope: scope.clone(),
            kind,
            text,
            subject,
            confidence: params.confidence.unwrap_or(0.5),
            session_id: params.session_id.clone(),
            expires_at,
            supersedes: params.supersedes.iter().map(|id| NodeId(*id)).collect(),
        };

        let written = self
            .store
            .remember(&new, &self.actor, self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;

        // A global write by an agent is called out in the audit summary rather than looking
        // like any other write, because it is the one that a person may want to review.
        let summary = if matches!(scope, MemoryScope::Global) && matches!(self.actor, Actor::Agent(_))
        {
            format!(
                "memory {} written to the GLOBAL scope by an agent",
                written.node_id.0
            )
        } else {
            format!("memory {} in {scope}", written.node_id.0)
        };
        self.auditor.allowed(self.now, tool, &args, &summary);

        Ok(RememberResult {
            memory_id: written.node_id.0,
            scope: scope.as_str(),
            kind: kind.as_str().to_string(),
            deduplicated: written.deduplicated,
            superseded: written.superseded,
            possible_duplicates: duplicates,
        })
    }

    pub fn supersede(&self, params: &SupersedeParams) -> ToolResult<ChangedResult> {
        let tool = "brain.supersede";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        for id in [params.old, params.new] {
            if let Err(r) = self.may_touch_memory(NodeId(id)) {
                return self.refuse_public(tool, &args, r);
            }
        }
        let changed = self
            .store
            .supersede_memory(NodeId(params.old), NodeId(params.new), &self.actor, self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("memory {} superseded by {}", params.old, params.new),
        );
        Ok(ChangedResult { changed })
    }

    /// Delete a memory, and record the deletion as an assertion so it survives a rebuild.
    pub fn forget_memory(&self, params: &ForgetMemoryParams) -> ToolResult<ChangedResult> {
        let tool = "brain.forget_memory";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        if let Err(r) = self.may_touch_memory(NodeId(params.memory_id)) {
            return self.refuse_public(tool, &args, r);
        }
        let changed = self
            .store
            .forget_memory(NodeId(params.memory_id), &self.actor, self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("memory {} forgotten", params.memory_id),
        );
        Ok(ChangedResult { changed })
    }

    /// An agent may only change memories it can see. A human may change any of them, which is
    /// the whole point of the Inspector (D78).
    fn may_touch_memory(&self, node: NodeId) -> Result<(), Refusal> {
        if !matches!(self.actor, Actor::Agent(_)) {
            return Ok(());
        }
        let memory = match self.store.memory(node) {
            Ok(Some(memory)) => memory,
            // Nothing there. Not a refusal — the call will report `changed: false` — but not a
            // reason to look up anything else either.
            _ => return Ok(()),
        };
        let reader = self.reader_scope(memory.session_id.as_deref(), &[]);
        if memory.scope.visible_from(&reader) || matches!(memory.scope, MemoryScope::Global) {
            Ok(())
        } else {
            Err(Refusal::NotForAgents {
                what: format!("changing memory {}, which belongs to another scope", node.0),
            })
        }
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub fn memories(&self, params: &MemoriesParams) -> ToolResult<MemoriesResult> {
        let tool = "brain.memories";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let kinds: Vec<MemoryKind> = params
            .kinds
            .iter()
            .filter_map(|k| MemoryKind::parse(k))
            .collect();

        let (filter, basis) = if params.everything {
            if matches!(self.actor, Actor::Agent(_)) {
                // A machine that can enumerate every other machine's private scope has no
                // scope isolation at all, whatever the listing is called.
                return self.refuse_public(
                    tool,
                    &args,
                    Refusal::NotForAgents {
                        what: "listing every scope in the index".into(),
                    },
                );
            }
            (
                MemoryFilter {
                    kinds: kinds.clone(),
                    include_superseded: params.include_superseded,
                    limit: params.limit.unwrap_or(DEFAULT_MEMORY_LIMIT),
                    ..MemoryFilter::everything(self.now)
                },
                "every memory in the index, in every scope, including superseded and expired ones"
                    .to_string(),
            )
        } else {
            let reader =
                self.reader_scope(params.session_id.as_deref(), &params.project_paths);
            let scopes = reader.visible_scopes();
            (
                MemoryFilter {
                    scopes: Some(scopes.clone()),
                    kinds: kinds.clone(),
                    include_superseded: params.include_superseded,
                    include_expired: false,
                    subject: params
                        .about
                        .as_ref()
                        .and_then(|p| self.store.node_by_path(p).ok().flatten())
                        .map(|n| n.id),
                    session_id: None,
                    limit: params.limit.unwrap_or(DEFAULT_MEMORY_LIMIT),
                    now: self.now,
                },
                format!(
                    "memories visible to this caller: {}. Superseded {}.",
                    scopes.join(", "),
                    if params.include_superseded {
                        "included"
                    } else {
                        "excluded"
                    }
                ),
            )
        };

        let memories: Vec<Memory> = self
            .store
            .list_memories(&filter)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &summarize("memories", memories.len(), None),
        );
        Ok(MemoriesResult { memories, basis })
    }

    // -----------------------------------------------------------------------
    // The learning signal
    // -----------------------------------------------------------------------

    pub fn outcome(&self, params: &OutcomeParams) -> ToolResult<OutcomeResult> {
        let tool = "brain.outcome";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        // Who is reporting decides how much it counts, and the caller does not get to say. An
        // agent reporting on its own success is an unreliable narrator; it is discounted rather
        // than ignored (D76).
        let reporter = match self.actor {
            Actor::Agent(_) => Reporter::Agent,
            _ => Reporter::Human,
        };

        let helped: Vec<NodeId> = params.helped.iter().map(|id| NodeId(*id)).collect();
        let noise: Vec<NodeId> = params.noise.iter().map(|id| NodeId(*id)).collect();

        let applied = self
            .store
            .report_outcome(&params.pack_id, &helped, &noise, reporter, &self.actor, self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!(
                "{} helped, {} noise, {} ignored, reported by {}",
                applied.helped,
                applied.noise,
                applied.ignored,
                reporter.as_str()
            ),
        );
        Ok(OutcomeResult {
            helped: applied.helped,
            noise: applied.noise,
            ignored: applied.ignored,
            reporter: reporter.as_str().to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    pub fn session_begin(&self, params: &SessionBeginParams, generated: &str) -> ToolResult<SessionResult> {
        let tool = "agent.session.begin";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let id = params
            .session_id
            .clone()
            .unwrap_or_else(|| generated.to_string());
        let name = params.name.clone().unwrap_or_else(|| match &self.actor {
            Actor::Agent(name) => name.clone(),
            other => other.as_str(),
        });
        self.store
            .begin_agent_session(&id, &name, params.task.as_deref(), self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        self.auditor
            .allowed(self.now, tool, &args, &format!("session {id} for {name}"));
        Ok(SessionResult { session_id: id })
    }

    pub fn session_end(&self, params: &SessionEndParams) -> ToolResult<ChangedResult> {
        let tool = "agent.session.end";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let changed = self
            .store
            .end_agent_session(&params.session_id, self.now)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        // Ending a session is also when its scratch goes. Failing to expire is not a reason to
        // fail the call — the memories carry an expiry and the next pass will take them.
        if let Err(e) = self.store.expire_memories(self.now) {
            tracing::warn!(error = %e, "could not expire session memories");
        }
        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("session {} ended", params.session_id),
        );
        Ok(ChangedResult { changed })
    }

    pub fn sessions(&self, params: &SessionsParams) -> ToolResult<SessionsResult> {
        let tool = "agent.sessions";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_args(tool, &args)?;

        let limit = params.limit.unwrap_or(50);
        let sessions = self
            .store
            .agent_sessions(limit)
            .map_err(|e| ToolError::Store(e.to_string()))?;
        let open = sessions.iter().filter(|s| s.ended_at.is_none()).count();
        self.auditor
            .allowed(self.now, tool, &args, &summarize("sessions", sessions.len(), None));
        Ok(SessionsResult {
            basis: format!(
                "the {} most recent agent sessions, newest first; {open} still running",
                sessions.len().min(limit)
            ),
            open,
            sessions,
        })
    }
}

/// How long a session memory lives when the caller does not say.
///
/// A day: long enough for any single task, short enough that scratch which outlived its session
/// is still scratch rather than a permanent belief nobody remembers agreeing to.
pub const DEFAULT_SESSION_TTL_SECS: i64 = 24 * 60 * 60;
