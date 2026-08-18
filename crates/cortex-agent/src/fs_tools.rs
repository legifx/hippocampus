//! The `fs.*` tools: the only way an agent touches the filesystem.
//!
//! Every method here follows the same four steps, in this order, and the order is the point:
//!
//! 1. **Gate.** Is the surface open at all (R7)?
//! 2. **Decide.** Deny-list for reads, allow-list for writes ([`crate::policy`]).
//! 3. **Do**, recording an episode with actor attribution and — for a mutation — an undo entry
//!    written *before* the filesystem is touched.
//! 4. **Audit** (R8), on both paths. A refusal that leaves no trace is the failure mode this
//!    whole layer exists to prevent.
//!
//! Reads are ungated beyond the deny-list, because an agent that cannot read is useless and a
//! local agent can read anyway. Mutations are gated hard: allow-list, plan, journal, episode.
//! The asymmetry is deliberate — reading the wrong file is a privacy problem, and writing the
//! wrong file is a data-loss problem, and only one of them is undoable.

use crate::audit::{summarize, Auditor};
use crate::plans::{Fingerprint, PlanBook};
use crate::policy::{Policy, Refusal};
use cortex_core::model::{Action, Actor};
use cortex_core::tools::*;
use cortex_store::Store;
use std::path::{Path, PathBuf};

/// Most a single `fs.read` returns. An agent that wants more asks for a range; a tool that
/// silently returns forty megabytes has broken whatever is on the other end of it.
pub const MAX_READ_BYTES: u64 = 1024 * 1024;
/// Most a single `fs.list` returns without being asked for more.
pub const DEFAULT_LIST_LIMIT: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error(transparent)]
    Refused(#[from] Refusal),

    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },

    #[error("the index could not be updated: {0}")]
    Store(String),
}

impl ToolError {
    pub fn code(&self) -> i32 {
        match self {
            ToolError::Refused(r) => r.code(),
            _ => -32603,
        }
    }

    fn io(path: &Path, e: std::io::Error) -> Self {
        ToolError::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        }
    }
}

impl From<cortex_store::StoreError> for ToolError {
    fn from(e: cortex_store::StoreError) -> Self {
        ToolError::Store(e.to_string())
    }
}

pub type ToolResult<T> = std::result::Result<T, ToolError>;

/// One agent's view of the surface, for the duration of one call.
///
/// Everything it needs is borrowed rather than owned, so the same code runs whether the caller
/// arrived over the socket, through the CLI or from the MCP server — there is one
/// implementation of every tool, which is what M3 §5 asks to be proven rather than asserted.
pub struct Surface<'a> {
    pub policy: &'a Policy,
    /// The kill switch's runtime state.
    pub enabled: bool,
    pub store: &'a Store,
    pub auditor: &'a Auditor,
    pub plans: &'a PlanBook,
    /// Where displaced content goes so that a destructive operation stays reversible.
    pub trash: &'a Path,
    pub actor: Actor,
    pub now: i64,
}

impl Surface<'_> {
    /// Step 1 for every tool. Separate from the policy check so that a disabled surface refuses
    /// before it has looked at any path — an agent must not be able to probe for the existence
    /// of files through a surface that is switched off.
    fn gate(&self, tool: &str, args: &serde_json::Value) -> ToolResult<()> {
        if let Err(r) = self.policy.check_surface(self.enabled) {
            return Err(self.auditor.refuse(self.now, tool, args, r).into());
        }
        Ok(())
    }

    fn refuse<T>(&self, tool: &str, args: &serde_json::Value, r: Refusal) -> ToolResult<T> {
        Err(self.auditor.refuse(self.now, tool, args, r).into())
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub fn list(&self, params: &FsListParams) -> ToolResult<FsListResult> {
        let tool = "fs.list";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let root = match self.policy.check_list(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        let depth = params.depth.unwrap_or(1).max(1);
        let mut entries = Vec::new();
        let mut hidden = 0usize;
        let mut truncated = false;

        let mut queue = vec![(root.clone(), 1u32)];
        while let Some((dir, level)) = queue.pop() {
            let reader = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) if dir == root => return Err(ToolError::io(&dir, e)),
                // A subdirectory that cannot be read is not a reason to fail the whole listing;
                // it is one line missing from an answer that is otherwise correct.
                Err(_) => continue,
            };

            for entry in reader.flatten() {
                let path = entry.path();
                // The deny-list applies to every entry, not only to the directory asked for.
                if self.policy.check_list(&path).is_err() {
                    hidden += 1;
                    continue;
                }
                if entries.len() >= limit {
                    truncated = true;
                    break;
                }

                let md = match entry.metadata() {
                    Ok(md) => md,
                    Err(_) => continue,
                };
                let kind = kind_of(&md);
                if kind == FsKind::Dir && level < depth {
                    queue.push((path.clone(), level + 1));
                }
                let indexed = self
                    .store
                    .node_by_path(&path)
                    .map(|n| n.is_some())
                    .unwrap_or(false);
                entries.push(FsEntry {
                    path,
                    kind,
                    size: md.len(),
                    mtime: crate::plans::mtime_of(&md),
                    indexed,
                });
            }
            if truncated {
                break;
            }
        }

        entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &summarize("entries", entries.len(), None),
        );
        Ok(FsListResult {
            path: root,
            entries,
            truncated,
            hidden,
        })
    }

    pub fn read(&self, params: &FsReadParams) -> ToolResult<FsReadResult> {
        let tool = "fs.read";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let path = match self.policy.check_read(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        let md = std::fs::metadata(&path).map_err(|e| ToolError::io(&path, e))?;
        if md.is_dir() {
            return Err(ToolError::Io {
                path,
                message: "is a directory; use fs.list".into(),
            });
        }
        let total = md.len();

        let (offset, mut want) = match params.byte_range {
            Some((start, end)) if end > start => (start, end - start),
            Some((start, _)) => (start, 0),
            None => (0, total),
        };
        let truncated = want > MAX_READ_BYTES;
        want = want.min(MAX_READ_BYTES);

        let raw = read_range(&path, offset, want).map_err(|e| ToolError::io(&path, e))?;
        let bytes = raw.len() as u64;
        let (content, note) = match String::from_utf8(raw) {
            Ok(text) => (text, None),
            Err(e) => (
                String::from_utf8_lossy(e.as_bytes()).into_owned(),
                Some("not valid UTF-8; invalid sequences were replaced".to_string()),
            ),
        };

        // The episode is what makes a read visible in the timeline later. Without it, "what has
        // the agent been looking at" is answerable only from the audit log, which is a
        // different question with a different retention.
        self.record_episode(Action::Open, &path, serde_json::json!({ "bytes": bytes }))?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &summarize("bytes read", 0, Some(bytes as usize)),
        );
        Ok(FsReadResult {
            path,
            content,
            offset,
            bytes,
            total_bytes: total,
            truncated: truncated || offset + bytes < total && params.byte_range.is_none(),
            note,
        })
    }

    pub fn stat(&self, params: &FsStatParams) -> ToolResult<FsStatResult> {
        let tool = "fs.stat";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let path = match self.policy.check_list(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        let md = std::fs::symlink_metadata(&path).ok();
        let node = self.store.node_by_path(&path).ok().flatten();

        let result = FsStatResult {
            exists: md.is_some(),
            kind: md.as_ref().map(kind_of),
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: md.as_ref().map(crate::plans::mtime_of).unwrap_or(0),
            readable: self.policy.check_read(&path).is_ok(),
            writable: self.policy.check_write(&path).is_ok(),
            indexed: node.is_some(),
            node_id: node.as_ref().map(|n| n.id.0),
            mime: node.and_then(|n| n.mime),
            path,
        };
        self.auditor
            .allowed(self.now, tool, &args, if result.exists { "exists" } else { "absent" });
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Mutation. Each of these either returns a plan or applies one.
    // -----------------------------------------------------------------------

    pub fn write(&self, params: &FsWriteParams) -> ToolResult<Mutation> {
        let tool = "fs.write";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let path = match self.policy.check_write(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        let existing = entry_at(&path);
        let action = match (params.mode, existing.is_some()) {
            (WriteMode::Create, true) => {
                return self.refuse(
                    tool,
                    &args,
                    Refusal::PlanStale {
                        detail: format!("{} already exists; use mode `overwrite`", path.display()),
                    },
                )
            }
            (WriteMode::Create, false) => PlanAction::Create,
            (WriteMode::Overwrite, _) => PlanAction::Overwrite,
            (WriteMode::Append, _) => PlanAction::Append,
        };

        let step = PlanStep {
            action,
            path: path.clone(),
            from: None,
            replaces: existing.clone(),
        };

        let Some(stored) = self.confirm(tool, &args, params.plan.as_deref(), vec![step.clone()])?
        else {
            return Ok(self.plan(tool, vec![step], vec![Fingerprint::of(&path)]));
        };
        let _ = stored;

        // Overwriting is destructive, so the previous version is moved aside rather than lost.
        // Appending is not: truncating back to the old length restores it exactly.
        //
        // The inverse steps are stored in the order of the forward steps they undo, and run in
        // reverse — so an overwrite, which is "move the old one away, then write the new one",
        // is undone by "delete the new one, then move the old one back". Getting that order
        // wrong makes the restore land on an occupied path and fail, which is how it was found.
        let (trashed, inverse) = match action {
            PlanAction::Create => (None, vec![InverseStep::Delete { path: path.clone() }]),
            PlanAction::Overwrite if existing.is_some() => {
                let dest = self.trash_slot(&path)?;
                (
                    Some(dest.clone()),
                    vec![
                        InverseStep::Restore {
                            from: dest,
                            to: path.clone(),
                        },
                        InverseStep::Delete { path: path.clone() },
                    ],
                )
            }
            PlanAction::Overwrite => (None, vec![InverseStep::Delete { path: path.clone() }]),
            PlanAction::Append => (
                None,
                vec![InverseStep::Truncate {
                    path: path.clone(),
                    size: existing.as_ref().map(|e| e.size).unwrap_or(0),
                }],
            ),
            _ => unreachable!("write produces no other action"),
        };

        let undo_id = self.journal(tool, &[step.clone()], &inverse)?;

        let outcome = (|| -> std::io::Result<()> {
            if let (Some(dest), true) = (&trashed, action == PlanAction::Overwrite) {
                move_file(&path, dest)?;
            }
            match action {
                PlanAction::Append => {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
                    f.write_all(params.content.as_bytes())
                }
                _ => std::fs::write(&path, params.content.as_bytes()),
            }
        })();

        self.finish(
            tool,
            &args,
            undo_id,
            outcome.map_err(|e| ToolError::io(&path, e)),
            &path,
            if action == PlanAction::Create { Action::Create } else { Action::Modify },
        )?;

        Ok(Mutation::Applied {
            undo_id,
            steps: vec![step],
            trashed,
        })
    }

    pub fn move_path(&self, params: &FsMoveParams) -> ToolResult<Mutation> {
        let tool = "fs.move";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        // Both ends are mutations: a move removes the source as surely as it creates the
        // target, so allow-listing only the destination would let an agent empty a directory it
        // was never given.
        let from = match self.policy.check_write(&params.from) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };
        let to = match self.policy.check_write(&params.to) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };
        if !from.exists() {
            return Err(ToolError::Io {
                path: from,
                message: "does not exist".into(),
            });
        }
        if to.exists() {
            return self.refuse(
                tool,
                &args,
                Refusal::PlanStale {
                    detail: format!("{} already exists; trash it first", to.display()),
                },
            );
        }

        let step = PlanStep {
            action: PlanAction::Move,
            path: to.clone(),
            from: Some(from.clone()),
            replaces: None,
        };

        let Some(stored) = self.confirm(tool, &args, params.plan.as_deref(), vec![step.clone()])?
        else {
            return Ok(self.plan(tool, vec![step], vec![Fingerprint::of(&from)]));
        };
        let _ = stored;

        let undo_id = self.journal(
            tool,
            &[step.clone()],
            &[InverseStep::Restore {
                from: to.clone(),
                to: from.clone(),
            }],
        )?;

        let outcome = move_file(&from, &to).map_err(|e| ToolError::io(&from, e));
        self.finish(tool, &args, undo_id, outcome, &to, Action::Move)?;

        // The index knows this file under its old path. Renaming the node rather than waiting
        // for the watcher keeps `fs.stat` honest immediately afterwards, and the rename is what
        // the watcher would have done anyway.
        let _ = self.store.rename_node(&from, &to);

        Ok(Mutation::Applied {
            undo_id,
            steps: vec![step],
            trashed: None,
        })
    }

    pub fn trash(&self, params: &FsTrashParams) -> ToolResult<Mutation> {
        let tool = "fs.trash";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let path = match self.policy.check_write(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };
        let Some(existing) = entry_at(&path) else {
            return Err(ToolError::Io {
                path,
                message: "does not exist".into(),
            });
        };

        let step = PlanStep {
            action: PlanAction::Trash,
            path: path.clone(),
            from: None,
            replaces: Some(existing),
        };

        let Some(stored) = self.confirm(tool, &args, params.plan.as_deref(), vec![step.clone()])?
        else {
            return Ok(self.plan(tool, vec![step], vec![Fingerprint::of(&path)]));
        };
        let _ = stored;

        let dest = self.trash_slot(&path)?;
        let undo_id = self.journal(
            tool,
            &[step.clone()],
            &[InverseStep::Restore {
                from: dest.clone(),
                to: path.clone(),
            }],
        )?;

        let outcome = move_file(&path, &dest).map_err(|e| ToolError::io(&path, e));
        self.finish(tool, &args, undo_id, outcome, &path, Action::Delete)?;

        let _ = self.store.remove_path(&path);

        Ok(Mutation::Applied {
            undo_id,
            steps: vec![step],
            trashed: Some(dest),
        })
    }

    // -----------------------------------------------------------------------
    // Undo
    // -----------------------------------------------------------------------

    /// Reverse recent mutations, newest first.
    ///
    /// Not gated by the allow-list: undo only ever puts things back where they were, and a
    /// user whose allow-list shrank since the mutation still needs to be able to reverse it.
    /// It is gated by the surface switch, and it is audited.
    pub fn undo(&self, params: &UndoParams) -> ToolResult<UndoResult> {
        let tool = "undo";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate(tool, &args)?;

        let wanted = params.last.unwrap_or(1);
        let candidates = self.store.undoable(wanted.max(1).min(10_000))?;
        let selected: Vec<_> = match &params.plan {
            Some(plan) => self
                .store
                .undoable(10_000)?
                .into_iter()
                .filter(|e| e.operation.get("plan").and_then(|p| p.as_str()) == Some(plan.as_str()))
                .collect(),
            None => candidates,
        };

        let mut undone = Vec::new();
        let mut failed = Vec::new();

        for entry in selected {
            let description = describe_operation(&entry.operation);
            if params.dry_run {
                undone.push(UndoneEntry {
                    id: entry.id,
                    description,
                });
                continue;
            }

            // Claim the entry first. Two `cortex undo` calls racing must not both apply the
            // same inverse — the second would move a file that is already back where it was.
            if !self.store.mark_undone(entry.id, self.now)? {
                continue;
            }

            match apply_inverse(&entry.inverse) {
                Ok(()) => {
                    self.record_episode_raw(
                        Action::Move,
                        None,
                        serde_json::json!({ "undo": entry.id, "operation": entry.operation }),
                    )?;
                    undone.push(UndoneEntry {
                        id: entry.id,
                        description,
                    });
                }
                Err(e) => {
                    // Put it back: an inverse that failed has not been applied, and marking it
                    // done would quietly drop the only record of how to reverse it.
                    let _ = self.store.push_undo(
                        entry.ts,
                        &entry.actor,
                        &entry.operation,
                        &entry.inverse,
                    );
                    failed.push(UndoFailure {
                        id: entry.id,
                        description,
                        reason: e.to_string(),
                    });
                }
            }
        }

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!("{} undone, {} failed", undone.len(), failed.len()),
        );
        Ok(UndoResult {
            undone,
            failed,
            dry_run: params.dry_run,
            remaining: self.store.undo_depth()?,
        })
    }

    // -----------------------------------------------------------------------
    // Shared machinery
    // -----------------------------------------------------------------------

    /// Turn steps into a registered plan.
    fn plan(&self, tool: &str, steps: Vec<PlanStep>, fingerprints: Vec<Fingerprint>) -> Mutation {
        let summary = crate::plans::summarize(&steps);
        let plan = self.plans.register(
            Plan {
                id: String::new(),
                tool: tool.to_string(),
                steps,
                expires_in_secs: 0,
                summary,
            },
            self.auditor.actor(),
            fingerprints,
            self.now,
        );
        self.auditor.allowed(
            self.now,
            tool,
            &serde_json::json!({ "dry_run": true }),
            &format!("planned as {}", plan.id),
        );
        Mutation::Planned { plan }
    }

    /// Check the caller's plan id, if the configuration demands one.
    ///
    /// Returns `None` when the caller has no plan and must be given one — the "mandatory dry
    /// run" of R6. With `require_dry_run` off, every call is its own confirmation, which is a
    /// setting for a machine nobody else uses and is not the default.
    fn confirm(
        &self,
        tool: &str,
        args: &serde_json::Value,
        plan_id: Option<&str>,
        steps: Vec<PlanStep>,
    ) -> ToolResult<Option<Option<crate::plans::StoredPlan>>> {
        match plan_id {
            None if self.policy.require_dry_run() => Ok(None),
            // No plan needed and none given: the call is its own confirmation.
            None => Ok(Some(None)),
            Some(id) => {
                let stored = match self.plans.take(id, self.auditor.actor(), self.now) {
                    Ok(s) => s,
                    Err(r) => return self.refuse(tool, args, r),
                };
                if let Err(r) = PlanBook::verify(&stored) {
                    return self.refuse(tool, args, r);
                }
                // The plan must describe the call being made. Otherwise a plan for a harmless
                // write would be a token for any write at all.
                if stored.plan.steps != steps {
                    return self.refuse(
                        tool,
                        args,
                        Refusal::PlanStale {
                            detail: "the plan was made for a different operation".into(),
                        },
                    );
                }
                Ok(Some(Some(stored)))
            }
        }
    }

    /// Write the journal entry that can reverse this operation. Before the operation, always.
    fn journal(&self, tool: &str, steps: &[PlanStep], inverse: &[InverseStep]) -> ToolResult<i64> {
        let operation = serde_json::json!({
            "tool": tool,
            "actor": self.auditor.actor(),
            "steps": steps,
        });
        let inverse = serde_json::json!({ "steps": inverse });
        let id = self
            .store
            .push_undo(self.now, self.auditor.actor(), &operation, &inverse)?;
        self.store.trim_undo(self.policy.undo_journal_depth())?;
        Ok(id)
    }

    /// Close out a mutation: audit it, record the episode, and unwind the journal entry if the
    /// filesystem refused.
    fn finish(
        &self,
        tool: &str,
        args: &serde_json::Value,
        undo_id: i64,
        outcome: ToolResult<()>,
        path: &Path,
        action: Action,
    ) -> ToolResult<()> {
        match outcome {
            Ok(()) => {
                self.record_episode(action, path, serde_json::json!({ "undo": undo_id }))?;
                self.auditor
                    .allowed(self.now, tool, args, &format!("applied, undo {undo_id}"));
                Ok(())
            }
            Err(e) => {
                // Nothing happened, so nothing is undoable.
                let _ = self.store.mark_undone(undo_id, self.now);
                self.auditor.allowed(self.now, tool, args, &format!("failed: {e}"));
                Err(e)
            }
        }
    }

    fn record_episode(
        &self,
        action: Action,
        path: &Path,
        metadata: serde_json::Value,
    ) -> ToolResult<()> {
        let node = self.store.node_by_path(path).ok().flatten().map(|n| n.id);
        let mut metadata = metadata;
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("path".into(), serde_json::json!(path.display().to_string()));
        }
        self.record_episode_raw(action, node, metadata)
    }

    fn record_episode_raw(
        &self,
        action: Action,
        node: Option<cortex_core::ids::NodeId>,
        metadata: serde_json::Value,
    ) -> ToolResult<()> {
        self.store
            .append_episode(self.now, &self.actor, action, node, None, &metadata)?;
        Ok(())
    }

    /// Reserve a directory in the trash and return the path the file will take inside it.
    ///
    /// A directory per operation rather than a flat namespace, so two files with the same name
    /// trashed a second apart do not collide, and so restoring one is a rename rather than a
    /// search.
    fn trash_slot(&self, original: &Path) -> ToolResult<PathBuf> {
        std::fs::create_dir_all(self.trash)
            .map_err(|e| ToolError::io(self.trash, e))?;
        let name = original
            .file_name()
            .map(|n| n.to_owned())
            .unwrap_or_else(|| std::ffi::OsString::from("unnamed"));

        for suffix in 0..10_000u32 {
            let slot = self.trash.join(format!("{}-{}", self.now, suffix));
            match std::fs::create_dir(&slot) {
                Ok(()) => return Ok(slot.join(name)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(ToolError::io(&slot, e)),
            }
        }
        Err(ToolError::Io {
            path: self.trash.to_path_buf(),
            message: "no free trash slot".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// The inverse of a mutation.
// ---------------------------------------------------------------------------

/// What `cortex undo` runs. Stored as JSON in the journal, so an entry written by one build is
/// still readable by the next.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum InverseStep {
    /// Undo a creation.
    Delete { path: PathBuf },
    /// Undo a move or a trashing: put the file back where it was.
    Restore { from: PathBuf, to: PathBuf },
    /// Undo an append: cut the file back to the length it had.
    Truncate { path: PathBuf, size: u64 },
}

fn apply_inverse(inverse: &serde_json::Value) -> std::io::Result<()> {
    let steps: Vec<InverseStep> = inverse
        .get("steps")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unreadable inverse")
        })?;

    // Reverse order: the inverse of "do A then B" is "undo B then undo A".
    for step in steps.iter().rev() {
        match step {
            InverseStep::Delete { path } => match std::fs::remove_file(path) {
                Ok(()) => {}
                // Already gone is the state being asked for.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            },
            InverseStep::Restore { from, to } => {
                if to.exists() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("{} is occupied again", to.display()),
                    ));
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                move_file(from, to)?;
            }
            InverseStep::Truncate { path, size } => {
                let f = std::fs::OpenOptions::new().write(true).open(path)?;
                f.set_len(*size)?;
            }
        }
    }
    Ok(())
}

fn describe_operation(operation: &serde_json::Value) -> String {
    let tool = operation.get("tool").and_then(|t| t.as_str()).unwrap_or("?");
    let steps = operation
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|s| s.len())
        .unwrap_or(0);
    let first = operation
        .get("steps")
        .and_then(|s| s.get(0))
        .and_then(|s| s.get("path"))
        .and_then(|p| p.as_str())
        .unwrap_or("");
    if steps > 1 {
        format!("{tool} on {first} and {} more", steps - 1)
    } else {
        format!("{tool} on {first}")
    }
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Move a file, across filesystems if it has to be.
///
/// `rename` fails with `EXDEV` when source and destination are on different mounts, and the
/// trash lives in the state directory, which is very often exactly that — a home directory on
/// one disk and `~/.local/state` on another is an ordinary setup, not an exotic one.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

fn read_range(path: &Path, offset: u64, want: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    if offset > 0 {
        f.seek(SeekFrom::Start(offset))?;
    }
    let mut buf = Vec::new();
    f.take(want).read_to_end(&mut buf)?;
    Ok(buf)
}

fn kind_of(md: &std::fs::Metadata) -> FsKind {
    if md.is_dir() {
        FsKind::Dir
    } else if md.is_file() {
        FsKind::File
    } else if md.is_symlink() {
        FsKind::Symlink
    } else {
        FsKind::Other
    }
}

fn entry_at(path: &Path) -> Option<FsEntry> {
    let md = std::fs::symlink_metadata(path).ok()?;
    Some(FsEntry {
        path: path.to_path_buf(),
        kind: kind_of(&md),
        size: md.len(),
        mtime: crate::plans::mtime_of(&md),
        indexed: false,
    })
}
