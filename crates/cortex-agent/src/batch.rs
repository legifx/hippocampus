//! Moving and copying several things at once, with everything the surface owes each one.
//!
//! The single-path tools in [`crate::fs_tools`] answer immediately: a rename is a rename. This
//! is for the operations that can take a minute, and the difference is not only duration —
//! it changes what "failed" means. Five files dragged into a folder is one gesture to the
//! person who made it, so stopping at the first problem would leave them guessing which of the
//! five arrived. Every source is attempted and every failure is reported with its reason.
//!
//! Each source that succeeds gets its own journal entry, so undoing a batch of five is five
//! reversals rather than one all-or-nothing lump — which is also what makes a partly-failed
//! batch undoable at all.

use crate::fs_tools::{InverseStep, Surface, ToolError, ToolResult};
use crate::policy::Refusal;
use crate::transfer::{self, Handle, Measure, OnConflict, Progress};
use cortex_core::model::Action;
use cortex_core::tools::{Conflict, ConflictChoice, FsEntry, FsKind, TransferFailure, TransferMode};
use std::path::{Path, PathBuf};

/// What a batch would do, checked before any of it runs.
#[derive(Debug)]
pub struct TransferPlan {
    /// The sources, resolved.
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    pub total: Measure,
    /// Destinations that already exist. Empty means nothing has to be asked.
    pub conflicts: Vec<Conflict>,
}

/// What a path is, for a conflict to describe. `None` when there is nothing there.
fn describe(path: &Path) -> Option<FsEntry> {
    let md = std::fs::symlink_metadata(path).ok()?;
    Some(crate::fs_tools::entry_from(
        path.to_path_buf(),
        &md,
        &crate::users::Names::from_system(),
        false,
    ))
}

/// What to show when the thing being described has already gone.
///
/// A conflict dialog needs two sides even when one of them vanished between the dry run and the
/// question; showing an empty row would be worse than showing a name with nothing behind it.
fn placeholder(path: &Path) -> FsEntry {
    FsEntry {
        path: path.to_path_buf(),
        kind: FsKind::File,
        size: 0,
        mtime: 0,
        indexed: false,
        mode: None,
        owner: None,
        group: None,
        target: None,
        target_kind: None,
        broken: false,
    }
}

/// What a batch did.
#[derive(Debug, Default)]
pub struct BatchOutcome {
    pub moved: Vec<PathBuf>,
    pub failed: Vec<TransferFailure>,
    pub undo_ids: Vec<i64>,
    pub cancelled: bool,
}

impl From<ConflictChoice> for OnConflict {
    fn from(choice: ConflictChoice) -> Self {
        match choice {
            // `Skip` never reaches the transfer: it is decided before anything is opened,
            // because the cheapest way to not copy a file is to not start.
            ConflictChoice::Fail | ConflictChoice::Skip => OnConflict::Fail,
            ConflictChoice::Replace => OnConflict::Replace,
            ConflictChoice::Keep => OnConflict::Keep,
        }
    }
}

impl Surface<'_> {
    /// Check a batch before any of it runs, and say what it would move.
    ///
    /// Separate from doing it because the caller needs the total before the first byte — a
    /// progress bar without a denominator is a spinner — and because refusing on policy grounds
    /// must happen before anything is touched, not per file as the batch discovers them.
    pub fn plan_transfer(
        &self,
        sources: &[PathBuf],
        destination: &Path,
        mode: TransferMode,
    ) -> ToolResult<TransferPlan> {
        let tool = tool_name(mode);
        let args = serde_json::json!({
            "sources": sources.len(),
            "destination": destination.display().to_string(),
        });
        self.gate_args(tool, &args)?;

        let destination = match self.policy.check_write(destination, &self.actor) {
            Ok(path) => path,
            Err(r) => return Err(self.auditor.refuse(self.now, tool, &args, r).into()),
        };
        if !destination.is_dir() {
            return Err(ToolError::Io {
                path: destination,
                message: "is not a directory to put things in".into(),
            });
        }

        let mut resolved = Vec::with_capacity(sources.len());
        let mut conflicts = Vec::new();
        let mut total = Measure::default();

        for source in sources {
            // A move reads *and* removes, so both ends are checked as writes. A copy only
            // reads the source, but the deny-list still applies to reading it.
            let checked = match mode {
                TransferMode::Move => self.policy.check_write(source, &self.actor),
                TransferMode::Copy => self.policy.check_read(source),
            };
            let checked = match checked {
                Ok(path) => path,
                Err(r) => return Err(self.auditor.refuse(self.now, tool, &args, r).into()),
            };

            if destination.starts_with(&checked) {
                return Err(ToolError::Io {
                    path: checked,
                    message: "cannot be put inside itself".into(),
                });
            }

            let measured = transfer::measure(&checked).map_err(|e| ToolError::Io {
                path: checked.clone(),
                message: e.to_string(),
            })?;
            total.files += measured.files;
            total.directories += measured.directories;
            total.bytes += measured.bytes;

            // What is already there, if anything. Collected here rather than discovered during
            // the transfer, so a client can ask the question before the first byte moves.
            if let Some(name) = checked.file_name() {
                let target = destination.join(name);
                if let Some(existing) = describe(&target) {
                    conflicts.push(Conflict {
                        source: checked.clone(),
                        destination: target,
                        existing,
                        incoming: describe(&checked).unwrap_or_else(|| placeholder(&checked)),
                    });
                }
            }

            resolved.push(checked);
        }

        Ok(TransferPlan {
            sources: resolved,
            destination,
            total,
            conflicts,
        })
    }

    /// Run a batch that `plan_transfer` has already checked.
    ///
    /// Blocking: this is called on a thread that can afford to be, and the alternative — an
    /// async copy loop — would buy nothing, because the work is `read` and `write` syscalls
    /// rather than waiting.
    pub fn run_transfer(
        &self,
        sources: &[PathBuf],
        destination: &Path,
        mode: TransferMode,
        conflict: ConflictChoice,
        total: Measure,
        handle: &Handle,
        mut on_progress: impl FnMut(&Progress),
    ) -> BatchOutcome {
        let tool = tool_name(mode);
        let mut outcome = BatchOutcome::default();

        for source in sources {
            if handle.is_cancelled() {
                outcome.cancelled = true;
                break;
            }

            let Some(name) = source.file_name() else {
                outcome.failed.push(TransferFailure {
                    source: source.clone(),
                    reason: "has no name to give it at the other end".into(),
                });
                continue;
            };
            let target = destination.join(name);

            if conflict == ConflictChoice::Skip && target.exists() {
                continue;
            }

            match self.transfer_one(source, &target, mode, conflict, total, handle, &mut on_progress) {
                Ok((arrived, undo_id)) => {
                    outcome.moved.push(arrived);
                    outcome.undo_ids.push(undo_id);
                }
                Err(TransferOne::Cancelled) => {
                    outcome.cancelled = true;
                    break;
                }
                Err(TransferOne::Failed(reason)) => outcome.failed.push(TransferFailure {
                    source: source.clone(),
                    reason,
                }),
            }
        }

        self.auditor.allowed(
            self.now,
            tool,
            &serde_json::json!({ "sources": sources.len() }),
            &format!(
                "{} moved, {} failed{}",
                outcome.moved.len(),
                outcome.failed.len(),
                if outcome.cancelled { ", cancelled" } else { "" }
            ),
        );
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn transfer_one(
        &self,
        source: &Path,
        target: &Path,
        mode: TransferMode,
        conflict: ConflictChoice,
        total: Measure,
        handle: &Handle,
        on_progress: &mut impl FnMut(&Progress),
    ) -> Result<(PathBuf, i64), TransferOne> {
        // The journal entry first, as everywhere else here: a crash mid-operation must leave
        // something that says what was being attempted.
        let step = cortex_core::tools::PlanStep {
            action: match mode {
                TransferMode::Copy => cortex_core::tools::PlanAction::Create,
                TransferMode::Move => cortex_core::tools::PlanAction::Move,
            },
            path: target.to_path_buf(),
            from: Some(source.to_path_buf()),
            replaces: None,
        };
        let inverse = match mode {
            // Undoing a copy deletes what arrived. Undoing a move puts it back.
            TransferMode::Copy => InverseStep::Delete {
                path: target.to_path_buf(),
            },
            TransferMode::Move => InverseStep::Restore {
                from: target.to_path_buf(),
                to: source.to_path_buf(),
            },
        };
        let undo_id = self
            .journal(tool_name(mode), &[step], &[inverse])
            .map_err(|e| TransferOne::Failed(e.to_string()))?;

        let result = match mode {
            TransferMode::Copy => transfer::copy(
                source,
                target,
                conflict.into(),
                handle,
                total,
                &mut *on_progress,
            ),
            TransferMode::Move => transfer::move_to(
                source,
                target,
                conflict.into(),
                handle,
                total,
                &mut *on_progress,
            ),
        };

        match result {
            Ok(arrived) => {
                // The journal entry was written against the name that was asked for, and a
                // `Keep` conflict may have chosen another. Correcting it is not optional: an
                // inverse pointing at a path that does not exist undoes nothing.
                if arrived != *target {
                    let corrected = match mode {
                        TransferMode::Copy => InverseStep::Delete {
                            path: arrived.clone(),
                        },
                        TransferMode::Move => InverseStep::Restore {
                            from: arrived.clone(),
                            to: source.to_path_buf(),
                        },
                    };
                    let _ = self.store.replace_inverse(
                        undo_id,
                        &serde_json::json!({ "steps": [corrected] }),
                    );
                }

                let action = match mode {
                    TransferMode::Copy => Action::Create,
                    TransferMode::Move => Action::Move,
                };
                let _ = self.record_episode(
                    action,
                    &arrived,
                    serde_json::json!({ "from": source.display().to_string(), "undo": undo_id }),
                );

                if mode == TransferMode::Move {
                    let _ = self.store.rename_node(source, &arrived);
                }
                Ok((arrived, undo_id))
            }
            Err(transfer::TransferError::Cancelled) => {
                let _ = self.store.mark_undone(undo_id, self.now);
                Err(TransferOne::Cancelled)
            }
            Err(e) => {
                // Nothing arrived, so nothing is undoable.
                let _ = self.store.mark_undone(undo_id, self.now);
                Err(TransferOne::Failed(e.to_string()))
            }
        }
    }

    /// Make a directory. Instant, so it answers rather than starting an operation.
    pub fn mkdir(&self, path: &Path) -> ToolResult<PathBuf> {
        let tool = "fs.mkdir";
        let args = serde_json::json!({ "path": path.display().to_string() });
        self.gate_args(tool, &args)?;

        let resolved = match self.policy.check_write(path, &self.actor) {
            Ok(p) => p,
            Err(r) => return Err(self.auditor.refuse(self.now, tool, &args, r).into()),
        };
        if resolved.exists() {
            return Err(ToolError::Io {
                path: resolved,
                message: "already exists".into(),
            });
        }

        let undo_id = self.journal(
            tool,
            &[cortex_core::tools::PlanStep {
                action: cortex_core::tools::PlanAction::Create,
                path: resolved.clone(),
                from: None,
                replaces: None,
            }],
            &[InverseStep::RemoveDir {
                path: resolved.clone(),
            }],
        )?;

        // `create_dir`, not `create_dir_all`: making three levels because two were misspelled
        // is not what anybody asked for, and the inverse would only remove one of them.
        match std::fs::create_dir(&resolved) {
            Ok(()) => {
                let _ = self.record_episode(Action::Create, &resolved, serde_json::json!({}));
                self.auditor
                    .allowed(self.now, tool, &args, &format!("created, undo {undo_id}"));
                Ok(resolved)
            }
            Err(e) => {
                let _ = self.store.mark_undone(undo_id, self.now);
                Err(ToolError::io(&resolved, e))
            }
        }
    }

    /// Rename within a directory. Also instant.
    pub fn rename(&self, path: &Path, name: &str) -> ToolResult<PathBuf> {
        let tool = "fs.rename";
        let args = serde_json::json!({ "path": path.display().to_string(), "name": name });
        self.gate_args(tool, &args)?;

        if name.is_empty() || name.contains('/') {
            return Err(ToolError::Io {
                path: path.to_path_buf(),
                message: "a name is not a path: it cannot be empty or contain a slash".into(),
            });
        }

        let from = match self.policy.check_write(path, &self.actor) {
            Ok(p) => p,
            Err(r) => return Err(self.auditor.refuse(self.now, tool, &args, r).into()),
        };
        let to = from.with_file_name(name);
        // Checked as well as derived: `..` in a name would otherwise rename across directories,
        // and the second check is what makes that impossible rather than unlikely.
        let to = match self.policy.check_write(&to, &self.actor) {
            Ok(p) => p,
            Err(r) => return Err(self.auditor.refuse(self.now, tool, &args, r).into()),
        };
        if to.parent() != from.parent() {
            return Err(ToolError::Io {
                path: to,
                message: "a rename stays in its directory".into(),
            });
        }
        if to.exists() {
            return Err(ToolError::Io {
                path: to,
                message: "already exists".into(),
            });
        }

        let undo_id = self.journal(
            tool,
            &[cortex_core::tools::PlanStep {
                action: cortex_core::tools::PlanAction::Move,
                path: to.clone(),
                from: Some(from.clone()),
                replaces: None,
            }],
            &[InverseStep::Restore {
                from: to.clone(),
                to: from.clone(),
            }],
        )?;

        match std::fs::rename(&from, &to) {
            Ok(()) => {
                let _ = self.store.rename_node(&from, &to);
                let _ = self.record_episode(Action::Move, &to, serde_json::json!({ "undo": undo_id }));
                self.auditor
                    .allowed(self.now, tool, &args, &format!("renamed, undo {undo_id}"));
                Ok(to)
            }
            Err(e) => {
                let _ = self.store.mark_undone(undo_id, self.now);
                Err(ToolError::io(&from, e))
            }
        }
    }
}

enum TransferOne {
    Cancelled,
    Failed(String),
}

fn tool_name(mode: TransferMode) -> &'static str {
    match mode {
        TransferMode::Copy => "fs.copy",
        TransferMode::Move => "fs.transfer",
    }
}

/// A refusal that reads sensibly when a batch is refused as a whole.
pub fn batch_refusal(refusal: Refusal) -> ToolError {
    ToolError::Refused(refusal)
}
