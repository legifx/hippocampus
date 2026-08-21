//! Who may touch what.
//!
//! Every decision this module makes is a pure function of the configuration and a path. That is
//! deliberate: a policy that consults running state can be talked into a different answer by
//! calling things in a different order, and a policy that does I/O cannot be exhaustively
//! tested. The one piece of genuine runtime state — the kill switch — lives in
//! [`crate::surface`] and is passed in as a boolean.
//!
//! Two rules do the work:
//!
//! - **Reads** obey the deny-list (R1). The same classifier the indexer uses, so an agent can
//!   never read a file the index refused to open.
//! - **Writes** obey an allow-list (R6), *and* the deny-list. An allow-list is not a stricter
//!   deny-list: it is the opposite default. Everything is forbidden until named, so a path
//!   nobody thought about is safe rather than exposed.

use cortex_core::config::AgentConfig;
use cortex_core::denylist;
use cortex_core::model::Actor;
use std::path::{Component, Path, PathBuf};

/// Why a call was refused.
///
/// Each variant carries what the caller needs to fix it, and nothing else — in particular, a
/// refusal never reveals whether a path it declined to look at exists.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("the Agent Surface is switched off")]
    SurfaceDisabled,

    #[error("{path} is on the deny-list and is never readable by an agent")]
    Denied { path: PathBuf },

    #[error(
        "{path} is not inside the write allow-list; add it to agent.write_allowlist to permit \
         mutations there"
    )]
    OutsideAllowlist { path: PathBuf },

    #[error("the write allow-list is empty, so no filesystem mutation is permitted")]
    NoAllowlist,

    /// For the operations that are not about a path at all.
    ///
    /// Mounting a drive, ejecting one, changing which application opens a type of file: none of
    /// them writes a path an allow-list could name, and all of them change what happens the next
    /// time a *person* uses the machine. Refusing them with [`Refusal::OutsideAllowlist`] was
    /// the first attempt and it was worse than nothing — it told the caller to add `/dev/sdb1`
    /// to `agent.write_allowlist`, which would not have helped, because there is no
    /// configuration that permits this.
    #[error("{what} is not something an agent may do, and no configuration changes that")]
    NotForAgents { what: String },

    #[error("this operation requires a confirmed dry-run plan first")]
    DryRunRequired,

    #[error("no such plan, or it has already been applied")]
    UnknownPlan,

    #[error("the plan no longer matches the filesystem: {detail}")]
    PlanStale { detail: String },

    #[error("{path} could not be resolved: {detail}")]
    Unresolvable { path: PathBuf, detail: String },

    #[error("a relative path is ambiguous for an agent; pass an absolute path ({path})")]
    NotAbsolute { path: PathBuf },
}

impl Refusal {
    /// The wire error code. Kept here rather than in the RPC layer so that every surface
    /// reports the same refusal the same way — the drift M3 §5 exists to prevent.
    pub fn code(&self) -> i32 {
        match self {
            Refusal::SurfaceDisabled => 1001,
            Refusal::DryRunRequired | Refusal::UnknownPlan | Refusal::PlanStale { .. } => 1003,
            _ => 1002,
        }
    }

    /// A short, stable tag for the audit log. Reasons are grepped; sentences are not.
    pub fn tag(&self) -> &'static str {
        match self {
            Refusal::SurfaceDisabled => "surface_disabled",
            Refusal::Denied { .. } => "denied",
            Refusal::OutsideAllowlist { .. } => "outside_allowlist",
            Refusal::NoAllowlist => "no_allowlist",
            Refusal::NotForAgents { .. } => "not_for_agents",
            Refusal::DryRunRequired => "dry_run_required",
            Refusal::UnknownPlan => "unknown_plan",
            Refusal::PlanStale { .. } => "plan_stale",
            Refusal::Unresolvable { .. } => "unresolvable",
            Refusal::NotAbsolute { .. } => "not_absolute",
        }
    }

    /// What the audit log stores as the result of a refused call.
    pub fn summary(&self) -> String {
        format!("refused: {} — {self}", self.tag())
    }
}

pub type Decision<T> = std::result::Result<T, Refusal>;

/// Whether the allow-list and the dry-run requirement apply.
///
/// `System` counts as the daemon acting on its own behalf — reconciliation, rebuilds — and is
/// not gated either. Only a named agent is.
pub fn is_agent(actor: &Actor) -> bool {
    matches!(actor, Actor::Agent(_))
}

/// The policy engine. Cheap to clone; built once per daemon from configuration.
#[derive(Debug, Clone)]
pub struct Policy {
    /// The configured master switch. A `false` here cannot be overridden at runtime — see
    /// [`crate::surface::KillSwitch`].
    configured_enabled: bool,
    write_allowlist: Vec<PathBuf>,
    require_dry_run: bool,
    extra_deny: Vec<String>,
    undo_journal_depth: usize,
}

impl Policy {
    pub fn new(agent: &AgentConfig, extra_deny: Vec<String>) -> Self {
        Self {
            configured_enabled: agent.enabled,
            // Resolved once, so that a symlinked allow-list entry does not silently fail to
            // match the canonical path of a file underneath it.
            write_allowlist: agent
                .write_allowlist
                .iter()
                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                .collect(),
            require_dry_run: agent.require_dry_run,
            extra_deny,
            undo_journal_depth: agent.undo_journal_depth,
        }
    }

    pub fn configured_enabled(&self) -> bool {
        self.configured_enabled
    }

    pub fn require_dry_run(&self) -> bool {
        self.require_dry_run
    }

    pub fn undo_journal_depth(&self) -> usize {
        self.undo_journal_depth
    }

    pub fn write_allowlist(&self) -> &[PathBuf] {
        &self.write_allowlist
    }

    /// The gate every agent call passes first.
    pub fn check_surface(&self, runtime_enabled: bool) -> Decision<()> {
        if self.configured_enabled && runtime_enabled {
            Ok(())
        } else {
            Err(Refusal::SurfaceDisabled)
        }
    }

    /// May an agent read this path? Returns the resolved path it may read.
    pub fn check_read(&self, path: &Path) -> Decision<PathBuf> {
        let resolved = resolve(path)?;
        // Classified on the resolved path: a symlink into `~/.ssh` is `~/.ssh`.
        if !denylist::classify(&resolved, &self.extra_deny).may_read_content() {
            return Err(Refusal::Denied { path: resolved });
        }
        Ok(resolved)
    }

    /// May an agent list this directory? Metadata is a weaker ask than content, so a path that
    /// is merely `MetadataOnly` — a large binary, say — can still be listed, while a
    /// `Sensitive` one cannot be looked at at all.
    pub fn check_list(&self, path: &Path) -> Decision<PathBuf> {
        let resolved = resolve(path)?;
        if !denylist::classify(&resolved, &self.extra_deny).may_index() {
            return Err(Refusal::Denied { path: resolved });
        }
        Ok(resolved)
    }

    /// May this actor mutate this path? Returns the resolved target.
    ///
    /// **The allow-list gates agents, not the person at the keyboard.** Rule R6 is titled "agent
    /// writes are gated" and says so in its first sentence; a file manager whose owner cannot
    /// move a file until they have listed the directory in a configuration file is not a file
    /// manager, and the same person can already use `rm`. What does apply to everybody is the
    /// deny-list: a human's file manager has no more business writing into `~/.ssh` unasked
    /// than an agent does, and nothing in this program needs to (D44).
    ///
    /// The target need not exist — a write creates files — but its parent must, or there is
    /// nothing to resolve against and a typo'd path would be judged on its literal text.
    pub fn check_write(&self, path: &Path, actor: &Actor) -> Decision<PathBuf> {
        let resolved = resolve(path)?;

        if is_agent(actor) {
            if self.write_allowlist.is_empty() {
                return Err(Refusal::NoAllowlist);
            }
            if !self.write_allowlist.iter().any(|root| under(&resolved, root)) {
                return Err(Refusal::OutsideAllowlist { path: resolved });
            }
        }

        // R1 is not an agent rule. It applies to whoever is asking.
        if !denylist::classify(&resolved, &self.extra_deny).may_index() {
            return Err(Refusal::Denied { path: resolved });
        }
        Ok(resolved)
    }

    /// Does this actor have to present a plan before mutating?
    ///
    /// Only an agent. A person who clicked "delete" in a dialog has confirmed; asking them to
    /// confirm a second time through a protocol they cannot see would be ceremony rather than
    /// safety.
    pub fn requires_plan(&self, actor: &Actor) -> bool {
        self.require_dry_run && is_agent(actor)
    }
}

/// Resolve a path to an absolute, symlink-free form without requiring that it exists.
///
/// `canonicalize` refuses a path whose last component is missing, which is precisely the case
/// for every file an agent is about to create. So the deepest existing ancestor is canonicalised
/// and the remainder appended — which still defeats `..` and symlinked parents, the two ways a
/// literal string check gets walked out of its allow-list.
pub fn resolve(path: &Path) -> Decision<PathBuf> {
    if !path.is_absolute() {
        return Err(Refusal::NotAbsolute {
            path: path.to_path_buf(),
        });
    }

    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();

    loop {
        match existing.canonicalize() {
            Ok(base) => {
                let mut out = base;
                for part in tail.iter().rev() {
                    out.push(part);
                }
                // A `..` left in the tail would climb out of the resolved base after the fact.
                if out.components().any(|c| c == Component::ParentDir) {
                    return Err(Refusal::Unresolvable {
                        path: path.to_path_buf(),
                        detail: "contains `..` above a path that does not exist".into(),
                    });
                }
                return Ok(out);
            }
            Err(_) => {
                let Some(name) = existing.file_name().map(|n| n.to_owned()) else {
                    return Err(Refusal::Unresolvable {
                        path: path.to_path_buf(),
                        detail: "no existing ancestor".into(),
                    });
                };
                tail.push(name);
                if !existing.pop() {
                    return Err(Refusal::Unresolvable {
                        path: path.to_path_buf(),
                        detail: "no existing ancestor".into(),
                    });
                }
            }
        }
    }
}

/// Is `path` inside `root`, or `root` itself? Compared component-wise, so `/data-private` is
/// not inside `/data`.
fn under(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test in here is about the *agent* path. The human path has its own tests below,
    /// because "the allow-list does not apply to me" is exactly the kind of claim that needs
    /// its own assertions rather than an absence of them.
    fn agent() -> Actor {
        Actor::Agent("test".into())
    }

    fn config(allow: Vec<PathBuf>) -> AgentConfig {
        AgentConfig {
            enabled: true,
            write_allowlist: allow,
            require_dry_run: true,
            undo_journal_depth: 100,
        }
    }

    #[test]
    fn a_disabled_surface_refuses_before_anything_else() {
        let p = Policy::new(&config(vec![]), vec![]);
        assert_eq!(p.check_surface(false), Err(Refusal::SurfaceDisabled));
        assert_eq!(p.check_surface(true), Ok(()));

        let mut off = config(vec![]);
        off.enabled = false;
        let p = Policy::new(&off, vec![]);
        assert_eq!(
            p.check_surface(true),
            Err(Refusal::SurfaceDisabled),
            "configuration wins over runtime state"
        );
    }

    #[test]
    fn writes_need_an_allowlist_entry() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();

        let p = Policy::new(&config(vec![allowed.clone()]), vec![]);

        assert!(p.check_write(&allowed.join("new.txt"), &agent()).is_ok());
        assert!(matches!(
            p.check_write(&elsewhere.join("new.txt"), &agent()),
            Err(Refusal::OutsideAllowlist { .. })
        ));
    }

    #[test]
    fn an_empty_allowlist_permits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = Policy::new(&config(vec![]), vec![]);
        assert_eq!(
            p.check_write(&dir.path().join("f.txt"), &agent()),
            Err(Refusal::NoAllowlist)
        );
    }

    #[test]
    fn dotdot_cannot_escape_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let secret = dir.path().join("secret");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&secret).unwrap();

        let p = Policy::new(&config(vec![allowed.clone()]), vec![]);
        let escape = allowed.join("..").join("secret").join("f.txt");
        assert!(
            matches!(p.check_write(&escape, &agent()), Err(Refusal::OutsideAllowlist { .. })),
            "`..` must be resolved before the allow-list is consulted"
        );
    }

    #[test]
    fn a_symlinked_parent_cannot_escape_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let secret = dir.path().join("secret");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&secret).unwrap();
        std::os::unix::fs::symlink(&secret, allowed.join("link")).unwrap();

        let p = Policy::new(&config(vec![allowed.clone()]), vec![]);
        assert!(
            matches!(
                p.check_write(&allowed.join("link").join("f.txt"), &agent()),
                Err(Refusal::OutsideAllowlist { .. })
            ),
            "the link target is what gets written, so it is what gets checked"
        );
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_inside() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("data");
        let sibling = dir.path().join("data-private");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        let p = Policy::new(&config(vec![allowed]), vec![]);
        assert!(matches!(
            p.check_write(&sibling.join("f.txt"), &agent()),
            Err(Refusal::OutsideAllowlist { .. })
        ));
    }

    #[test]
    fn the_denylist_still_applies_inside_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("home");
        let ssh = allowed.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();

        let p = Policy::new(&config(vec![allowed.clone()]), vec![]);
        assert!(
            matches!(p.check_write(&ssh.join("id_ed25519"), &agent()), Err(Refusal::Denied { .. })),
            "allow-listing a home directory does not allow-list the keys in it"
        );
        assert!(matches!(
            p.check_read(&ssh.join("id_ed25519")),
            Err(Refusal::Denied { .. })
        ));
    }

    #[test]
    fn relative_paths_are_refused_rather_than_guessed() {
        let p = Policy::new(&config(vec![PathBuf::from("/tmp")]), vec![]);
        assert!(matches!(
            p.check_read(Path::new("notes.txt")),
            Err(Refusal::NotAbsolute { .. })
        ));
    }

    #[test]
    fn a_missing_file_still_resolves_through_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("does-not-exist-yet.txt");
        let resolved = resolve(&target).unwrap();
        assert_eq!(resolved.file_name(), target.file_name());
        assert!(resolved.is_absolute());
    }

    #[test]
    fn a_human_is_not_held_to_the_agent_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();

        // No allow-list at all — the configuration an agent is refused everything under.
        let p = Policy::new(&config(vec![]), vec![]);

        assert!(
            p.check_write(&elsewhere.join("new.txt"), &Actor::Human).is_ok(),
            "a file manager whose owner cannot move a file until they edit a config file is \
             not a file manager, and the same person can already use `rm`"
        );
        assert_eq!(
            p.check_write(&elsewhere.join("new.txt"), &agent()),
            Err(Refusal::NoAllowlist),
            "and the agent path is unchanged"
        );
    }

    #[test]
    fn the_denylist_applies_to_the_human_too() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();

        let p = Policy::new(&config(vec![]), vec![]);
        assert!(
            matches!(
                p.check_write(&ssh.join("id_ed25519"), &Actor::Human),
                Err(Refusal::Denied { .. })
            ),
            "R1 is not an agent rule; nothing in this program needs to write there"
        );
    }

    #[test]
    fn only_an_agent_has_to_present_a_plan() {
        let p = Policy::new(&config(vec![PathBuf::from("/tmp")]), vec![]);
        assert!(p.requires_plan(&agent()));
        assert!(!p.requires_plan(&Actor::Human));
        assert!(!p.requires_plan(&Actor::System), "the daemon is not an agent either");
    }

    #[test]
    fn refusal_codes_are_stable() {
        assert_eq!(Refusal::SurfaceDisabled.code(), 1001);
        assert_eq!(Refusal::NoAllowlist.code(), 1002);
        assert_eq!(Refusal::DryRunRequired.code(), 1003);
    }
}
