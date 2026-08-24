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
use crate::trash::{trash_for, DeletionDate, Trash};
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

    /// Refused by polkit, and obtainable: somebody with a password can allow it.
    ///
    /// Separate from [`ToolError::Io`] so that it reaches a client as a code rather than as a
    /// sentence — the window turns it into an authentication prompt, and matching on English
    /// prose to decide that would be a client reading a message meant for a person.
    #[error("{path}: {message}")]
    NeedsAuthorisation { path: PathBuf, message: String },

    #[error("the index could not be updated: {0}")]
    Store(String),
}

impl ToolError {
    pub fn code(&self) -> i32 {
        match self {
            ToolError::Refused(r) => r.code(),
            ToolError::NeedsAuthorisation { .. } => 1004,
            _ => -32603,
        }
    }

    pub(crate) fn io(path: &Path, e: std::io::Error) -> Self {
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
    /// The XDG trash. Deleting puts things where every other tool on the desktop looks for
    /// them, which is what makes it safe to stop using another file manager.
    pub trash: &'a Trash,
    pub actor: Actor,
    pub now: i64,
}

impl Surface<'_> {
    /// Step 1 for every tool. Separate from the policy check so that a disabled surface refuses
    /// before it has looked at any path — an agent must not be able to probe for the existence
    /// of files through a surface that is switched off.
    /// The kill-switch check on its own, for a caller that composes several tools into one
    /// answer and audits the result itself — `brain.recall` is that case.
    pub fn gate(&self, tool: &str) -> ToolResult<()> {
        self.gate_with(tool, &serde_json::json!({}))
    }

    /// The gate with the caller's own arguments, so a refusal records what was asked for.
    pub(crate) fn gate_args(&self, tool: &str, args: &serde_json::Value) -> ToolResult<()> {
        self.gate_with(tool, args)
    }

    fn gate_with(&self, tool: &str, args: &serde_json::Value) -> ToolResult<()> {
        if let Err(r) = self.policy.check_surface(self.enabled) {
            return Err(self.auditor.refuse(self.now, tool, args, r).into());
        }
        Ok(())
    }

    fn refuse<T>(&self, tool: &str, args: &serde_json::Value, r: Refusal) -> ToolResult<T> {
        Err(self.auditor.refuse(self.now, tool, args, r).into())
    }

    /// The same, for tool modules outside this file.
    pub(crate) fn refuse_public<T>(
        &self,
        tool: &str,
        args: &serde_json::Value,
        r: Refusal,
    ) -> ToolResult<T> {
        self.refuse(tool, args, r)
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    /// Where a person would like to start, rather than where the filesystem starts.
    ///
    /// The deny-list applies here as it does everywhere (R1): a place that could not be listed
    /// is not offered, because a row that refuses when pressed is worse than no row.
    ///
    /// Not gated and not audited as a mutation, because it reads no file's contents — it reads
    /// the mount table, a config file and the names of some directories. It is still an audit
    /// entry, like every other call at this surface.
    pub fn places(&self, params: &FsPlacesParams) -> ToolResult<FsPlacesResult> {
        let tool = "fs.places";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

        let mut sections = Vec::new();
        for section in crate::places::sections(&crate::places::home_dir()) {
            let mut kept = Vec::new();
            for place in section.places {
                if self.policy.check_list(&place.path).is_ok() {
                    kept.push(place);
                }
            }
            if !kept.is_empty() {
                sections.push(cortex_core::tools::PlaceSection {
                    title: section.title,
                    places: kept,
                });
            }
        }

        let count: usize = sections.iter().map(|s| s.places.len()).sum();
        self.auditor
            .allowed(self.now, tool, &args, &summarize("places", count, None));
        Ok(FsPlacesResult { sections })
    }

    pub fn list(&self, params: &FsListParams) -> ToolResult<FsListResult> {
        let tool = "fs.list";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

        let root = match self.policy.check_list(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        // A share whose server has gone does not answer slowly — it does not answer, and the
        // `read_dir` below would hold this thread until the mount's own timeout expires, which
        // for a default NFS mount is minutes. So a path on a network mount is knocked on first,
        // once, with a deadline. A local path is not: the probe costs a thread, and the whole
        // point is that it is paid only where something can hang.
        if let Some(mount) = self.network_mount_of(&root) {
            let probe = root.clone();
            if cortex_core::mounts::answers_within(cortex_core::mounts::PROBE, move || {
                std::fs::symlink_metadata(&probe).is_ok()
            })
            .is_none()
            {
                let message = format!(
                    "{} did not answer within {} ms — the share may be gone",
                    mount.device,
                    cortex_core::mounts::PROBE.as_millis()
                );
                self.auditor.allowed(self.now, tool, &args, &message);
                return Err(ToolError::Io {
                    path: root,
                    message,
                });
            }
        }

        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        let depth = params.depth.unwrap_or(1).max(1);
        let mut entries = Vec::new();
        let mut hidden = 0usize;
        let mut truncated = false;

        // Once per listing, not once per entry: ten thousand files must not mean ten thousand
        // reads of `/etc/passwd`.
        let names = crate::users::Names::from_system();

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
                entries.push(entry_from(path, &md, &names, indexed));
            }
            if truncated {
                break;
            }
        }

        sort_entries(&mut entries, params.sort, params.reverse);
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

    /// The network mount a path is on, if it is on one.
    ///
    /// The mount table is re-read per listing rather than cached. It is a few kilobytes out of
    /// procfs against a call that is about to read a directory, and the alternative — a cached
    /// table — would be wrong exactly when it matters: at the moment somebody plugs in, mounts
    /// or loses a share.
    fn network_mount_of(&self, path: &Path) -> Option<cortex_core::mounts::Mount> {
        let mounts = cortex_core::mounts::read(Path::new(cortex_core::mounts::PROC_MOUNTS));
        cortex_core::mounts::network_mount_for(path, &mounts).cloned()
    }

    pub fn read(&self, params: &FsReadParams) -> ToolResult<FsReadResult> {
        let tool = "fs.read";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

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
        self.gate_with(tool, &args)?;

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
            writable: self.policy.check_write(&path, &self.actor).is_ok(),
            indexed: node.is_some(),
            node_id: node.as_ref().map(|n| n.id.0),
            mime: node.and_then(|n| n.mime),
            path,
        };
        self.auditor
            .allowed(self.now, tool, &args, if result.exists { "exists" } else { "absent" });
        Ok(result)
    }

    /// What would open this file, and what the desktop would open it with by default.
    ///
    /// A read, gated by the deny-list like every other read (R1): the file's name decides its
    /// type, so answering for a path an agent may not look at would leak the name it may not
    /// see. The rest is read from the desktop's own databases — see [`crate::apps`] for why
    /// this is the daemon's job and running the application is not.
    pub fn apps_for_path(&self, params: &AppsForPathParams) -> ToolResult<AppsForPathResult> {
        let tool = "apps.for_path";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

        let path = match self.policy.check_read(&params.path) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };

        let xdg = crate::apps::Xdg::from_env();
        let mime = crate::apps::mime_for(&xdg, &path);
        let applications = if params.all {
            crate::apps::all_applications(&xdg)
        } else {
            crate::apps::applications_for(&xdg, &mime)
        };

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &summarize(&mime, applications.len(), None),
        );
        Ok(AppsForPathResult {
            path,
            mime,
            applications,
        })
    }

    /// Record which application opens a type of file, from now on and for the whole desktop.
    ///
    /// **Never for an agent, and not because of the allow-list.** The allow-list is about which
    /// paths may be written (D44); this is a different kind of power. Changing what opens
    /// `text/plain` changes what happens the next time a *person* double-clicks a file, in this
    /// window and in every other application — an agent that could point a common type at a
    /// program of its choosing would have arranged for that program to be run later, by
    /// somebody else, with no further call to this surface to refuse. The audit log would
    /// record one innocuous line and nothing afterwards.
    ///
    /// It writes no undo entry, and that is a limit rather than an oversight: the undo journal
    /// describes filesystem operations, and this is a configuration key. What the previous
    /// default was is recorded in the audit line, which is what somebody putting it back needs.
    pub fn apps_set_default(
        &self,
        params: &AppsSetDefaultParams,
    ) -> ToolResult<AppsSetDefaultResult> {
        let tool = "apps.set_default";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

        if crate::policy::is_agent(&self.actor) {
            return self.refuse(
                tool,
                &args,
                Refusal::NotForAgents {
                    what: format!("choosing what opens {}", params.mime),
                },
            );
        }

        let xdg = crate::apps::Xdg::from_env();
        let previous = crate::apps::current_default(&xdg, &params.mime);
        let file = crate::apps::set_default(&xdg, &params.mime, &params.application).map_err(
            |e| match e {
                crate::apps::SetDefaultError::NoSuchApplication(id) => ToolError::Io {
                    path: PathBuf::from(id),
                    message: "no such application is installed".into(),
                },
                crate::apps::SetDefaultError::Io { path, message } => {
                    ToolError::Io { path, message }
                }
            },
        )?;

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            &format!(
                "{} opens {} (was {})",
                params.application,
                params.mime,
                previous.as_deref().unwrap_or("nothing in particular")
            ),
        );
        Ok(AppsSetDefaultResult {
            mime: params.mime.clone(),
            application: params.application.clone(),
            file,
        })
    }

    /// Mount a volume that is plugged in but not mounted, and answer with where it landed.
    ///
    /// **Never for an agent.** Like `apps.set_default`, this is not the allow-list's kind of
    /// question — no path an agent could name is written. It is a hardware operation on
    /// somebody's desk: mounting a drive exposes a filesystem the machine had deliberately not
    /// exposed, and it is the step before reading everything on it. The person at the keyboard
    /// does it by pressing Enter on a row they can see; nothing else has any business doing it.
    ///
    /// Already mounted is success, not an error: two clicks on a slow drive are one intention.
    pub fn mount(&self, params: &FsMountParams) -> ToolResult<FsMountResult> {
        let tool = "fs.mount";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;
        self.refuse_hardware(tool, &args, &params.device)?;

        let volume = self.found(tool, &args, &params.device)?;

        if let Some(point) = volume.mount_point {
            self.auditor
                .allowed(self.now, tool, &args, &format!("already at {}", point.display()));
            return Ok(FsMountResult {
                device: params.device.clone(),
                mount_point: point,
                mounted_now: false,
            });
        }

        // Asking is the exception and it is never reached on its own: a client sends
        // `interactive` only after an ordinary attempt was refused for want of a password, and
        // only because a person is standing in front of the answer (D61).
        let udisks = if params.interactive {
            crate::volumes::Udisks::asking(crate::volumes::PATIENCE)
        } else {
            crate::volumes::Udisks::default()
        };
        let mount_point = udisks
            .mount(&params.device)
            .map_err(|e| self.failed(tool, &args, &params.device, e))?;

        self.auditor
            .allowed(self.now, tool, &args, &format!("mounted at {}", mount_point.display()));
        Ok(FsMountResult {
            device: params.device.clone(),
            mount_point,
            mounted_now: true,
        })
    }

    /// Unmount a volume, and cut the power to its drive when asked to.
    ///
    /// Refused for an agent, for the reason `mount` gives — and the case for refusing is
    /// stronger here, because unmounting a drive somebody is copying to is how a copy ends up
    /// half-written.
    ///
    /// **Ejecting a disk that cannot be unplugged is refused outright.** `power-off` on an
    /// internal disk is a way to lose a machine, and "removable" is read from the kernel rather
    /// than inferred from the mount point.
    pub fn unmount(&self, params: &FsUnmountParams) -> ToolResult<FsUnmountResult> {
        let tool = "fs.unmount";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;
        self.refuse_hardware(tool, &args, &params.device)?;

        let volume = self.found(tool, &args, &params.device)?;
        let as_io = |e: crate::volumes::VolumeError| self.failed(tool, &args, &params.device, e);

        let plan = crate::volumes::plan_unmount(&volume, params.eject).map_err(as_io)?;
        let udisks = crate::volumes::Udisks::default();
        match plan {
            crate::volumes::Unmounting::Nothing => {}
            crate::volumes::Unmounting::Unmount => {
                udisks.unmount(&params.device).map_err(as_io)?;
            }
            crate::volumes::Unmounting::Eject => {
                let disk = crate::volumes::disk_of(
                    Path::new(crate::volumes::SYS_BLOCK),
                    &params.device,
                )
                .unwrap_or_else(|| params.device.clone());
                udisks
                    .eject(&params.device, &disk, volume.mount_point.is_some())
                    .map_err(as_io)?;
            }
        }

        self.auditor.allowed(
            self.now,
            tool,
            &args,
            if params.eject { "ejected" } else { "unmounted" },
        );
        Ok(FsUnmountResult {
            device: params.device.clone(),
            ejected: params.eject,
        })
    }

    /// The volume, or a refusal that is in the audit log like every other one.
    ///
    /// Rule R8 is that a decision leaves a trace whether it was allowed or refused, and the
    /// first version of these two methods returned every hardware failure with no trace at all —
    /// so a refused mount was invisible in `cortex audit` and the only evidence it had happened
    /// was a screenshot of the window.
    fn found(
        &self,
        tool: &str,
        args: &serde_json::Value,
        device: &Path,
    ) -> ToolResult<crate::volumes::Volume> {
        match self.volume(device) {
            Some(volume) => Ok(volume),
            None => {
                let message = "no such volume on this machine";
                self.auditor.allowed(self.now, tool, args, message);
                Err(ToolError::Io {
                    path: device.to_path_buf(),
                    message: message.into(),
                })
            }
        }
    }

    /// The same for a failure that came back from udisks2.
    fn failed(
        &self,
        tool: &str,
        args: &serde_json::Value,
        device: &Path,
        error: crate::volumes::VolumeError,
    ) -> ToolError {
        let message = error.to_string();
        self.auditor.allowed(self.now, tool, args, &message);
        let path = device.to_path_buf();
        match error {
            // The one failure a client can act on rather than only report.
            crate::volumes::VolumeError::NeedsPermission { .. } => {
                ToolError::NeedsAuthorisation { path, message }
            }
            _ => ToolError::Io { path, message },
        }
    }

    /// What the machine knows about one device, from the same scan the places screen uses.
    fn volume(&self, device: &Path) -> Option<crate::volumes::Volume> {
        let mounts = crate::places::read_mounts(Path::new("/proc/mounts"));
        crate::volumes::scan(
            Path::new(crate::volumes::SYS_BLOCK),
            Path::new(crate::volumes::UDEV_DATA),
            &mounts,
        )
        .into_iter()
        .find(|v| v.device == device)
    }

    /// The refusal `fs.mount` and `fs.unmount` share.
    ///
    /// A separate step from the allow-list because it is a different question: not "may this
    /// path be written" but "may this actor touch the hardware at all". The answer for an agent
    /// is no, with no configuration that changes it.
    fn refuse_hardware(
        &self,
        tool: &str,
        args: &serde_json::Value,
        device: &Path,
    ) -> ToolResult<()> {
        if crate::policy::is_agent(&self.actor) {
            return self.refuse(
                tool,
                args,
                Refusal::NotForAgents {
                    what: format!("{tool} on {}", device.display()),
                },
            );
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Mutation. Each of these either returns a plan or applies one.
    // -----------------------------------------------------------------------

    pub fn write(&self, params: &FsWriteParams) -> ToolResult<Mutation> {
        let tool = "fs.write";
        let args = serde_json::to_value(params).unwrap_or_default();
        self.gate_with(tool, &args)?;

        let path = match self.policy.check_write(&params.path, &self.actor) {
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

        // Overwriting is destructive, so the previous version goes to the trash rather than
        // being lost — the *real* trash, where the person can find it without knowing this
        // program exists (D34, revised by D45). Appending is not destructive: truncating back
        // to the old length restores it exactly.
        //
        // Inverse steps are stored in the order of the forward steps they undo and run in
        // reverse, so an overwrite — "put the old one in the trash, then write the new one" —
        // is undone by "delete the new one, then take the old one back out". Getting that order
        // wrong makes the restore land on an occupied path and fail, which is how it was found.
        let overwriting_something = action == PlanAction::Overwrite && existing.is_some();

        let inverse = match action {
            PlanAction::Create => vec![InverseStep::Delete { path: path.clone() }],
            PlanAction::Overwrite if overwriting_something => vec![
                // The name is filled in once the trash has chosen one, below.
                InverseStep::Untrash {
                    trash: PathBuf::new(),
                    name: String::new(),
                },
                InverseStep::Delete { path: path.clone() },
            ],
            PlanAction::Overwrite => vec![InverseStep::Delete { path: path.clone() }],
            PlanAction::Append => vec![InverseStep::Truncate {
                path: path.clone(),
                size: existing.as_ref().map(|e| e.size).unwrap_or(0),
            }],
            _ => unreachable!("write produces no other action"),
        };

        let undo_id = self.journal(tool, &[step.clone()], &inverse)?;

        let mut trashed = None;
        if overwriting_something {
            let trash = trash_for(&path, self.trash).map_err(|e| ToolError::Io {
                path: path.clone(),
                message: e.to_string(),
            })?;
            match trash.put(&path, DeletionDate::from_unix(self.now)) {
                Ok(entry) => {
                    self.store.replace_inverse(
                        undo_id,
                        &serde_json::json!({
                            "steps": [
                                InverseStep::Untrash {
                                    trash: trash.root().to_path_buf(),
                                    name: entry.name.clone(),
                                },
                                InverseStep::Delete { path: path.clone() },
                            ]
                        }),
                    )?;
                    trashed = Some(entry.path);
                }
                Err(e) => {
                    let _ = self.store.mark_undone(undo_id, self.now);
                    let error = ToolError::Io {
                        path: path.clone(),
                        message: e.to_string(),
                    };
                    self.auditor
                        .allowed(self.now, tool, &args, &format!("failed: {error}"));
                    return Err(error);
                }
            }
        }

        let outcome = (|| -> std::io::Result<()> {
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
        self.gate_with(tool, &args)?;

        // Both ends are mutations: a move removes the source as surely as it creates the
        // target, so allow-listing only the destination would let an agent empty a directory it
        // was never given.
        let from = match self.policy.check_write(&params.from, &self.actor) {
            Ok(p) => p,
            Err(r) => return self.refuse(tool, &args, r),
        };
        let to = match self.policy.check_write(&params.to, &self.actor) {
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
        self.gate_with(tool, &args)?;

        let path = match self.policy.check_write(&params.path, &self.actor) {
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

        // Which trash: the home one, or the one at the top of this path's own filesystem. A
        // trash on the wrong filesystem would mean copying every byte to delete something.
        let trash = trash_for(&path, self.trash).map_err(|e| ToolError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;

        // The journal entry names the trash and the entry rather than a path, so undoing goes
        // through the same code that a person clicking "restore" would: the `.trashinfo` is
        // removed with the file, and the two never disagree.
        let undo_id = self.journal(
            tool,
            &[step.clone()],
            &[InverseStep::Untrash {
                trash: trash.root().to_path_buf(),
                // Filled in below, once the trash has chosen a name.
                name: String::new(),
            }],
        )?;

        let entry = match trash.put(&path, DeletionDate::from_unix(self.now)) {
            Ok(entry) => entry,
            Err(e) => {
                let _ = self.store.mark_undone(undo_id, self.now);
                let error = ToolError::Io {
                    path: path.clone(),
                    message: e.to_string(),
                };
                self.auditor
                    .allowed(self.now, tool, &args, &format!("failed: {error}"));
                return Err(error);
            }
        };

        // Now that the name is known, replace the journal's inverse with one that can be used.
        self.store.replace_inverse(
            undo_id,
            &serde_json::json!({
                "steps": [InverseStep::Untrash {
                    trash: trash.root().to_path_buf(),
                    name: entry.name.clone(),
                }]
            }),
        )?;

        self.finish(tool, &args, undo_id, Ok(()), &path, Action::Delete)?;
        let _ = self.store.remove_path(&path);

        Ok(Mutation::Applied {
            undo_id,
            steps: vec![step],
            trashed: Some(entry.path),
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
        self.gate_with(tool, &args)?;

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
            None if self.policy.requires_plan(&self.actor) => Ok(None),
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
    pub(crate) fn journal(&self, tool: &str, steps: &[PlanStep], inverse: &[InverseStep]) -> ToolResult<i64> {
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

    pub(crate) fn record_episode(
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
    /// Undo a trashing, through the trash's own restore so the `.trashinfo` goes with it.
    Untrash { trash: PathBuf, name: String },
    /// Undo a directory that was created. Empty only: removing a directory somebody has since
    /// filled would be undoing more than was done.
    RemoveDir { path: PathBuf },
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
            InverseStep::RemoveDir { path } => match std::fs::remove_dir(path) {
                Ok(()) => {}
                // Not empty any more, or already gone. Either way, removing it would undo
                // somebody else's work rather than ours.
                Err(_) => {}
            },
            InverseStep::Untrash { trash, name } => {
                crate::trash::Trash::at(trash)
                    .restore(name)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
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

/// Everything a listing can say about one entry.
///
/// One function rather than three construction sites, because the fields that were added for
/// M6 — permissions, ownership, where a link points — are the kind that get added in one place
/// and forgotten in the other two, and then a file shows its owner in a listing and not in a
/// conflict dialog for no reason anybody can explain.
///
/// `md` must come from `symlink_metadata` (or `DirEntry::metadata`, which is the same thing on
/// Unix). A link described by its target's metadata is a link the reader cannot see is a link.
pub(crate) fn entry_from(
    path: PathBuf,
    md: &std::fs::Metadata,
    names: &crate::users::Names,
    indexed: bool,
) -> FsEntry {
    let kind = kind_of(md);
    let mut entry = FsEntry {
        path,
        kind,
        size: md.len(),
        mtime: crate::plans::mtime_of(md),
        indexed,
        mode: None,
        owner: None,
        group: None,
        target: None,
        target_kind: None,
        broken: false,
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        entry.mode = Some(md.mode());
        entry.owner = Some(names.user(md.uid()));
        entry.group = Some(names.group(md.gid()));
    }

    if kind == FsKind::Symlink {
        // Verbatim, relative if it was written relative: that is what the link says, and
        // resolving it would be answering a different question than "where does this point".
        entry.target = std::fs::read_link(&entry.path).ok();
        // One extra stat, and only for links. `metadata` follows, which is the point — it is
        // also what reports a dangling link, by failing.
        match std::fs::metadata(&entry.path) {
            Ok(target) => entry.target_kind = Some(kind_of(&target)),
            Err(_) => entry.broken = true,
        }
    }

    entry
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

/// Order a listing.
///
/// Directories first in every order except `None`. That is a convention rather than a law, and
/// it is the one every file manager follows, because a directory is a place and a file is a
/// thing and mixing them by size answers a question nobody asked.
fn sort_entries(entries: &mut [FsEntry], sort: SortBy, reverse: bool) {
    let dirs_first = |a: &FsEntry, b: &FsEntry| {
        let rank = |e: &FsEntry| u8::from(e.kind != FsKind::Dir);
        rank(a).cmp(&rank(b))
    };
    // The tie-break is the name, always: two files of the same size must not swap places
    // between two listings of the same directory.
    let by_name = |a: &FsEntry, b: &FsEntry| {
        let name = |e: &FsEntry| {
            e.path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        name(a).cmp(&name(b))
    };

    match sort {
        SortBy::None => {}
        SortBy::Name => entries.sort_by(|a, b| dirs_first(a, b).then_with(|| by_name(a, b))),
        SortBy::Size => {
            entries.sort_by(|a, b| dirs_first(a, b).then(b.size.cmp(&a.size)).then_with(|| by_name(a, b)))
        }
        SortBy::Modified => entries
            .sort_by(|a, b| dirs_first(a, b).then(b.mtime.cmp(&a.mtime)).then_with(|| by_name(a, b))),
    }

    if reverse && sort != SortBy::None {
        // Reversed within each group, so directories stay together at the top. A reversal that
        // moved them to the bottom would be a different sort, not the same one backwards.
        let split = entries.iter().position(|e| e.kind != FsKind::Dir).unwrap_or(entries.len());
        entries[..split].reverse();
        entries[split..].reverse();
    }
}

fn entry_at(path: &Path) -> Option<FsEntry> {
    let md = std::fs::symlink_metadata(path).ok()?;
    Some(entry_from(
        path.to_path_buf(),
        &md,
        &crate::users::Names::from_system(),
        false,
    ))
}
