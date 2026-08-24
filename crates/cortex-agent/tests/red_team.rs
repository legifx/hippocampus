//! The red-team suite M3 §2 asks for.
//!
//! For every gated tool it must be true that the tool refuses without an allow-list entry,
//! refuses without a confirmed dry-run, and leaves an audit entry when it refuses. Those three
//! assertions are written once, as a table, so that a tool added later without them is a
//! failing test rather than a gap nobody notices.
//!
//! The rest of the file is the attacks: escaping the allow-list through `..`, through a
//! symlink, through a plan made for a different file, through a plan used twice, through a
//! plan whose target changed underneath it, and through a surface that is supposed to be off.

use cortex_agent::fs_tools::{Surface, ToolError};
use cortex_agent::plans::PlanBook;
use cortex_agent::policy::{Policy, Refusal};
use cortex_agent::Auditor;
use cortex_core::config::AgentConfig;
use cortex_core::model::Actor;
use cortex_core::tools::*;
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Fixture {
    _dir: tempfile::TempDir,
    allowed: PathBuf,
    forbidden: PathBuf,
    trash: cortex_agent::trash::Trash,
    store: Arc<Store>,
    auditor: Auditor,
    plans: PlanBook,
    policy: Policy,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(|_| {})
    }

    fn with_config(tweak: impl FnOnce(&mut AgentConfig)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let forbidden = dir.path().join("forbidden");
        let trash = cortex_agent::trash::Trash::at(dir.path().join("Trash"));
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&forbidden).unwrap();

        let mut config = AgentConfig {
            enabled: true,
            write_allowlist: vec![allowed.clone()],
            require_dry_run: true,
            undo_journal_depth: 1000,
        };
        tweak(&mut config);

        let store = Arc::new(Store::open_in_memory().unwrap());
        let auditor = Auditor::new(store.clone(), &Actor::Agent("redteam".into()));
        Self {
            policy: Policy::new(&config, vec![]),
            _dir: dir,
            // Resolved, because macOS-style symlinked temp directories otherwise make every
            // comparison in here a false negative.
            allowed: allowed.canonicalize().unwrap(),
            forbidden: forbidden.canonicalize().unwrap(),
            trash,
            store,
            auditor,
            plans: PlanBook::new(120),
        }
    }

    fn surface(&self, enabled: bool) -> Surface<'_> {
        Surface {
            policy: &self.policy,
            enabled,
            store: &self.store,
            auditor: &self.auditor,
            plans: &self.plans,
            trash: &self.trash,
            actor: Actor::Agent("redteam".into()),
            now: 1000,
        }
    }

    fn refusals(&self) -> Vec<String> {
        self.store
            .recent_audit(1000, None, true, None)
            .unwrap()
            .into_iter()
            .map(|e| e.result_summary.unwrap_or_default())
            .collect()
    }
}

fn refusal(e: ToolError) -> Refusal {
    match e {
        ToolError::Refused(r) => r,
        other => panic!("expected a refusal, got {other}"),
    }
}

/// Apply a gated tool at `path` with an optional plan id. One closure per tool, so the table
/// below can treat them uniformly.
fn call(surface: &Surface<'_>, tool: &str, path: &Path, plan: Option<String>) -> Result<Mutation, ToolError> {
    match tool {
        "fs.write" => surface.write(&FsWriteParams {
            path: path.to_path_buf(),
            content: "payload".into(),
            mode: WriteMode::Overwrite,
            plan,
        }),
        "fs.move" => surface.move_path(&FsMoveParams {
            from: path.to_path_buf(),
            to: path.with_extension("moved"),
            plan,
        }),
        "fs.trash" => surface.trash(&FsTrashParams {
            path: path.to_path_buf(),
            plan,
        }),
        other => panic!("unknown tool {other}"),
    }
}

const GATED: [&str; 3] = ["fs.write", "fs.move", "fs.trash"];

#[test]
fn every_gated_tool_refuses_outside_the_allowlist_and_says_so_in_the_audit_log() {
    for tool in GATED {
        let f = Fixture::new();
        let target = f.forbidden.join("victim.txt");
        std::fs::write(&target, "original").unwrap();

        let err = call(&f.surface(true), tool, &target, None).unwrap_err();
        assert!(
            matches!(refusal(err), Refusal::OutsideAllowlist { .. }),
            "{tool} accepted a path outside the allow-list"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "original",
            "{tool} touched the file it claimed to refuse"
        );

        let refusals = f.refusals();
        assert_eq!(refusals.len(), 1, "{tool} refused without an audit entry");
        assert!(refusals[0].contains("outside_allowlist"));
    }
}

#[test]
fn every_gated_tool_requires_a_confirmed_dry_run_first() {
    for tool in GATED {
        let f = Fixture::new();
        let target = f.allowed.join("victim.txt");
        std::fs::write(&target, "original").unwrap();

        let planned = call(&f.surface(true), tool, &target, None).unwrap();
        let Mutation::Planned { plan } = planned else {
            panic!("{tool} mutated the filesystem without a plan");
        };
        assert_eq!(plan.tool, tool);
        assert!(!plan.summary.is_empty(), "{tool} planned without saying what it would do");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "original",
            "{tool} applied its own dry run"
        );
    }
}

#[test]
fn every_gated_tool_refuses_a_made_up_plan_id_and_audits_it() {
    for tool in GATED {
        let f = Fixture::new();
        let target = f.allowed.join("victim.txt");
        std::fs::write(&target, "original").unwrap();

        let err = call(&f.surface(true), tool, &target, Some("plan-9999".into())).unwrap_err();
        assert_eq!(refusal(err), Refusal::UnknownPlan, "{tool} accepted an invented plan");
        assert!(f.refusals().iter().any(|r| r.contains("unknown_plan")));
    }
}

#[test]
fn every_gated_tool_refuses_while_the_surface_is_off() {
    for tool in GATED {
        let f = Fixture::new();
        let target = f.allowed.join("victim.txt");
        std::fs::write(&target, "original").unwrap();

        let err = call(&f.surface(false), tool, &target, None).unwrap_err();
        assert_eq!(refusal(err), Refusal::SurfaceDisabled, "{tool} served a disabled surface");
        assert!(f.refusals().iter().any(|r| r.contains("surface_disabled")));
    }
}

#[test]
fn reads_are_refused_while_the_surface_is_off_and_reveal_nothing() {
    let f = Fixture::new();
    let target = f.allowed.join("secret-name.txt");
    std::fs::write(&target, "content").unwrap();

    let surface = f.surface(false);
    let err = surface
        .read(&FsReadParams {
            path: target.clone(),
            byte_range: None,
        })
        .unwrap_err();
    assert_eq!(refusal(err), Refusal::SurfaceDisabled);

    // A disabled surface must not distinguish a path that exists from one that does not.
    let err = surface
        .stat(&FsStatParams {
            path: f.allowed.join("no-such-file"),
        })
        .unwrap_err();
    assert_eq!(refusal(err), Refusal::SurfaceDisabled);
}

#[test]
fn a_plan_cannot_be_replayed() {
    let f = Fixture::new();
    let target = f.allowed.join("once.txt");

    let Mutation::Planned { plan } = f
        .surface(true)
        .write(&FsWriteParams {
            path: target.clone(),
            content: "first".into(),
            mode: WriteMode::Overwrite,
            plan: None,
        })
        .unwrap()
    else {
        panic!("expected a plan")
    };

    f.surface(true)
        .write(&FsWriteParams {
            path: target.clone(),
            content: "first".into(),
            mode: WriteMode::Overwrite,
            plan: Some(plan.id.clone()),
        })
        .unwrap();

    let err = f
        .surface(true)
        .write(&FsWriteParams {
            path: target.clone(),
            content: "second".into(),
            mode: WriteMode::Overwrite,
            plan: Some(plan.id),
        })
        .unwrap_err();
    assert_eq!(refusal(err), Refusal::UnknownPlan);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "first");
}

#[test]
fn a_plan_for_one_file_cannot_be_spent_on_another() {
    let f = Fixture::new();
    let harmless = f.allowed.join("scratch.txt");
    let precious = f.allowed.join("precious.txt");
    std::fs::write(&precious, "keep me").unwrap();

    let Mutation::Planned { plan } = f
        .surface(true)
        .write(&FsWriteParams {
            path: harmless,
            content: "x".into(),
            mode: WriteMode::Overwrite,
            plan: None,
        })
        .unwrap()
    else {
        panic!("expected a plan")
    };

    let err = f
        .surface(true)
        .write(&FsWriteParams {
            path: precious.clone(),
            content: "gone".into(),
            mode: WriteMode::Overwrite,
            plan: Some(plan.id),
        })
        .unwrap_err();
    assert!(matches!(refusal(err), Refusal::PlanStale { .. }));
    assert_eq!(std::fs::read_to_string(&precious).unwrap(), "keep me");
}

#[test]
fn a_plan_goes_stale_when_the_target_changes_underneath_it() {
    let f = Fixture::new();
    let target = f.allowed.join("racy.txt");
    std::fs::write(&target, "before").unwrap();

    let Mutation::Planned { plan } = f
        .surface(true)
        .trash(&FsTrashParams {
            path: target.clone(),
            plan: None,
        })
        .unwrap()
    else {
        panic!("expected a plan")
    };

    // Somebody replaced the file between the plan and its approval.
    std::fs::write(&target, "something entirely different and longer").unwrap();

    let err = f
        .surface(true)
        .trash(&FsTrashParams {
            path: target.clone(),
            plan: Some(plan.id),
        })
        .unwrap_err();
    assert!(matches!(refusal(err), Refusal::PlanStale { .. }));
    assert!(target.exists(), "a stale plan must not still be applied");
}

#[test]
fn a_symlink_out_of_the_allowlist_is_refused() {
    let f = Fixture::new();
    let outside = f.forbidden.join("target.txt");
    std::fs::write(&outside, "not yours").unwrap();
    let link = f.allowed.join("innocent.txt");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let err = f
        .surface(true)
        .write(&FsWriteParams {
            path: link,
            content: "overwritten".into(),
            mode: WriteMode::Overwrite,
            plan: None,
        })
        .unwrap_err();
    assert!(matches!(refusal(err), Refusal::OutsideAllowlist { .. }));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "not yours");
}

#[test]
fn an_empty_allowlist_makes_the_filesystem_read_only() {
    let f = Fixture::with_config(|c| c.write_allowlist.clear());
    let target = f.allowed.join("victim.txt");
    std::fs::write(&target, "original").unwrap();

    for tool in GATED {
        let err = call(&f.surface(true), tool, &target, None).unwrap_err();
        assert_eq!(refusal(err), Refusal::NoAllowlist, "{tool}");
    }
    // Reading still works: an agent that cannot read is not safer, only useless.
    assert!(f
        .surface(true)
        .read(&FsReadParams {
            path: target,
            byte_range: None
        })
        .is_ok());
}

#[test]
fn the_denylist_is_not_lifted_by_the_allowlist() {
    let f = Fixture::new();
    let ssh = f.allowed.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_ed25519");
    std::fs::write(&key, "PRIVATE KEY").unwrap();

    let err = f
        .surface(true)
        .read(&FsReadParams {
            path: key.clone(),
            byte_range: None,
        })
        .unwrap_err();
    assert!(matches!(refusal(err), Refusal::Denied { .. }));

    let err = call(&f.surface(true), "fs.trash", &key, None).unwrap_err();
    assert!(matches!(refusal(err), Refusal::Denied { .. }));
}

#[test]
fn listing_hides_denied_entries_and_counts_them() {
    let f = Fixture::new();
    std::fs::write(f.allowed.join("visible.txt"), "hello").unwrap();
    std::fs::create_dir_all(f.allowed.join(".ssh")).unwrap();

    let result = f
        .surface(true)
        .list(&FsListParams {
            path: f.allowed.clone(),
            glob: None,
            depth: None,
            limit: None,
            sort: Default::default(),
            reverse: false,
        })
        .unwrap();

    let names: Vec<_> = result
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["visible.txt"]);
    assert_eq!(result.hidden, 1, "the hidden entry is counted, not named");
}

#[test]
fn every_refusal_is_audited_even_when_nothing_else_happened() {
    let f = Fixture::new();
    let before = f.store.audit_count().unwrap();

    let _ = f.surface(true).read(&FsReadParams {
        path: PathBuf::from("relative/path.txt"),
        byte_range: None,
    });

    assert_eq!(f.store.audit_count().unwrap(), before + 1);
    assert!(f.refusals()[0].contains("not_absolute"));
}

// ---------------------------------------------------------------------------
// Browsing. M6 turns the launcher into a file manager, and the order a listing comes back in
// is the daemon's decision — a frontend that sorted would be doing the data processing D4
// forbids, and two clients would eventually disagree about what "by name" means.
// ---------------------------------------------------------------------------

fn listing(f: &Fixture, sort: SortBy, reverse: bool) -> Vec<String> {
    f.surface(true)
        .list(&FsListParams {
            path: f.allowed.clone(),
            glob: None,
            depth: None,
            limit: None,
            sort,
            reverse,
        })
        .unwrap()
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().to_string())
        .collect()
}

/// A directory with a shape that tells the orders apart: two directories, three files whose
/// name, size and age each imply a different sequence.
fn browsable() -> Fixture {
    let f = Fixture::new();
    std::fs::create_dir_all(f.allowed.join("Zebra")).unwrap();
    std::fs::create_dir_all(f.allowed.join("alpha")).unwrap();
    std::fs::write(f.allowed.join("big.bin"), vec![0u8; 3000]).unwrap();
    std::fs::write(f.allowed.join("Middle.txt"), vec![0u8; 2000]).unwrap();
    std::fs::write(f.allowed.join("tiny.md"), b"x").unwrap();

    // Ages, so `modified` has something to order by. Newest is the smallest file.
    let epoch = std::time::UNIX_EPOCH;
    for (name, secs) in [("big.bin", 1_000), ("Middle.txt", 2_000), ("tiny.md", 3_000)] {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(f.allowed.join(name))
            .unwrap();
        file.set_modified(epoch + std::time::Duration::from_secs(secs)).unwrap();
    }
    f
}

#[test]
fn a_listing_is_ordered_by_name_with_directories_first() {
    let f = browsable();
    assert_eq!(
        listing(&f, SortBy::Name, false),
        // Case-insensitive: `Zebra` after `alpha`, not before it because of its capital.
        vec!["alpha", "Zebra", "big.bin", "Middle.txt", "tiny.md"]
    );
}

#[test]
fn ordering_by_size_and_by_age_keeps_directories_at_the_top() {
    let f = browsable();
    assert_eq!(
        listing(&f, SortBy::Size, false),
        vec!["alpha", "Zebra", "big.bin", "Middle.txt", "tiny.md"],
        "largest first, and a directory is a place rather than a size"
    );
    assert_eq!(
        listing(&f, SortBy::Modified, false),
        vec!["alpha", "Zebra", "tiny.md", "Middle.txt", "big.bin"],
        "newest first — 'sorted by date' has only ever meant that"
    );
}

#[test]
fn reversing_keeps_the_directories_together() {
    let f = browsable();
    let reversed = listing(&f, SortBy::Name, true);
    assert_eq!(
        reversed,
        vec!["Zebra", "alpha", "tiny.md", "Middle.txt", "big.bin"],
        "a reversal that moved the directories to the bottom would be a different sort"
    );
}

#[test]
fn the_order_is_stable_across_two_identical_listings() {
    let f = browsable();
    // Two files of the same size must not swap places between two listings of one directory.
    std::fs::write(f.allowed.join("same-a.bin"), vec![0u8; 3000]).unwrap();
    std::fs::write(f.allowed.join("same-b.bin"), vec![0u8; 3000]).unwrap();

    let first = listing(&f, SortBy::Size, false);
    let second = listing(&f, SortBy::Size, false);
    assert_eq!(first, second);
    let a = first.iter().position(|n| n == "same-a.bin").unwrap();
    let b = first.iter().position(|n| n == "same-b.bin").unwrap();
    assert!(a < b, "the tie-break is the name, always");
}

// ---------------------------------------------------------------------------
// What a listing says about an entry beyond its name. M6 asks for permissions, ownership and
// symbolic links "displayed correctly", and correctly is the load-bearing word: a link reported
// as its target is indistinguishable from a copy, and deleting one then looks like deleting the
// other.
// ---------------------------------------------------------------------------

fn entry_named<'a>(result: &'a cortex_core::tools::FsListResult, name: &str) -> &'a FsEntry {
    for entry in &result.entries {
        if entry.path.file_name().is_some_and(|n| n == name) {
            return entry;
        }
    }
    panic!("no entry called {name} in {:?}", result.entries);
}

fn list_all(f: &Fixture) -> cortex_core::tools::FsListResult {
    f.surface(true)
        .list(&FsListParams {
            path: f.allowed.clone(),
            glob: None,
            depth: None,
            limit: None,
            sort: Default::default(),
            reverse: false,
        })
        .unwrap()
}

#[test]
fn a_symlink_is_reported_as_a_symlink_and_not_as_what_it_points_at() {
    let f = Fixture::new();
    let target = f.allowed.join("target.txt");
    std::fs::write(&target, "hello").unwrap();
    std::os::unix::fs::symlink(&target, f.allowed.join("link.txt")).unwrap();

    let result = list_all(&f);
    let link = entry_named(&result, "link.txt");

    assert_eq!(link.kind, FsKind::Symlink, "a link is a link, not a file");
    assert_eq!(link.target.as_deref(), Some(target.as_path()));
    assert_eq!(link.target_kind, Some(FsKind::File));
    assert!(!link.broken);

    // And the target itself is untouched by any of this.
    assert_eq!(entry_named(&result, "target.txt").kind, FsKind::File);
    assert!(entry_named(&result, "target.txt").target.is_none());
}

#[test]
fn a_link_to_a_directory_says_so_so_that_it_can_be_opened_like_one() {
    // Without `target_kind` this entry renders as "not a directory", and pressing Enter on it
    // tries to preview a directory instead of descending into it.
    let f = Fixture::new();
    std::fs::create_dir(f.allowed.join("somewhere")).unwrap();
    std::os::unix::fs::symlink(f.allowed.join("somewhere"), f.allowed.join("shortcut")).unwrap();

    let link = list_all(&f);
    let link = entry_named(&link, "shortcut");
    assert_eq!(link.kind, FsKind::Symlink);
    assert_eq!(link.target_kind, Some(FsKind::Dir));
}

#[test]
fn a_broken_link_is_listed_rather_than_hidden() {
    // A dangling link looks exactly like a working one until somebody tries it, which is the
    // argument for showing it: the listing is the only place it can be noticed.
    let f = Fixture::new();
    std::os::unix::fs::symlink(f.allowed.join("gone.txt"), f.allowed.join("dangling")).unwrap();

    let result = list_all(&f);
    let link = entry_named(&result, "dangling");
    assert_eq!(link.kind, FsKind::Symlink);
    assert!(link.broken, "the link points at nothing and says so");
    assert_eq!(link.target_kind, None);
    assert_eq!(link.target.as_deref(), Some(f.allowed.join("gone.txt").as_path()));
}

#[test]
fn a_relative_link_is_reported_as_written() {
    // Rewriting it to an absolute path would answer "where does this end up" instead of "what
    // does this link say", and the two differ the moment the directory is moved.
    let f = Fixture::new();
    std::fs::write(f.allowed.join("target.txt"), "hello").unwrap();
    std::os::unix::fs::symlink("target.txt", f.allowed.join("relative")).unwrap();

    let link = list_all(&f);
    assert_eq!(
        entry_named(&link, "relative").target.as_deref(),
        Some(std::path::Path::new("target.txt"))
    );
}

#[test]
fn permissions_and_ownership_come_back_with_every_entry() {
    let f = Fixture::new();
    let script = f.allowed.join("run.sh");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o750)).unwrap();

    let result = list_all(&f);
    let entry = entry_named(&result, "run.sh");

    let mode = entry.mode.expect("a listing on Unix knows the mode");
    assert_eq!(mode & 0o777, 0o750);
    assert_eq!(cortex_core::tools::format_mode(entry.kind, mode), "-rwxr-x---");

    // The owner is whoever is running the test, which is the only thing that can be asserted
    // portably — and the name, if this machine has an `/etc/passwd` that mentions them.
    let owner = entry.owner.as_ref().expect("an owner");
    assert_eq!(owner.id, unsafe { libc_getuid() });
    assert!(entry.group.is_some());
}

/// `getuid` without a dependency on libc: it cannot fail and takes no arguments.
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// An agent may not mount or eject a drive.
///
/// The same shape of refusal as the one below and for a related reason, but the failure is
/// physical rather than social: mounting exposes a filesystem the machine had deliberately not
/// exposed, and ejecting one that somebody is copying to leaves a half-written file on a stick
/// that has already been pulled out. Neither is a question about a path, so no allow-list entry
/// can permit them.
#[test]
fn an_agent_may_not_touch_the_drives() {
    let f = Fixture::new();
    // A device that does not exist: the refusal must come before anything looks at the machine,
    // so that an agent cannot use this call to find out what is plugged in.
    let device = std::path::PathBuf::from("/dev/definitely-not-here");

    let mount = f
        .surface(true)
        .mount(&FsMountParams { device: device.clone(), interactive: false })
        .unwrap_err();
    let unmount = f
        .surface(true)
        .unmount(&FsUnmountParams { device: device.clone(), eject: true })
        .unwrap_err();

    for error in [mount, unmount] {
        let refused = refusal(error);
        assert!(matches!(refused, Refusal::NotForAgents { .. }), "{refused}");
        assert!(
            !refused.to_string().contains("allow-list"),
            "an allow-list entry would not help, so it must not be suggested: {refused}"
        );
    }
    assert_eq!(f.refusals().len(), 2, "both are in the audit log (R8)");
}

/// A hardware failure is in the audit log, like every other decision (R8).
///
/// It was not, at first: a refused mount returned an error and left no trace, so `cortex audit`
/// showed the successful ones and nothing else — and the only evidence that a refusal had
/// happened at all was a screenshot of the window. An audit log that records only what worked
/// is a log that answers the one question nobody asks.
#[test]
fn a_mount_that_fails_leaves_a_trace() {
    let f = Fixture::new();
    let mut human = f.surface(true);
    human.actor = Actor::Human;

    let error = human
        .mount(&FsMountParams {
            device: std::path::PathBuf::from("/dev/definitely-not-here"),
            interactive: false,
        })
        .unwrap_err();
    assert!(error.to_string().contains("no such volume"), "{error}");

    let entries = f.store.recent_audit(10, None, false, None).unwrap();
    let mount = entries
        .iter()
        .find(|e| e.tool == "fs.mount")
        .expect("the attempt is in the audit log");
    assert!(
        mount.result_summary.as_deref().unwrap_or("").contains("no such volume"),
        "and says what happened: {:?}",
        mount.result_summary
    );
}

/// An agent may not decide which program opens a person's files.
///
/// This is not the allow-list's kind of refusal and it is not covered by the table above: the
/// allow-list is about *paths*, and this call writes no path an agent could name. What it
/// changes is what happens the next time a **person** opens a file — in this window and in every
/// other application on the desktop. An agent that could point `text/plain` at a program of its
/// choosing would have arranged to be run later, by somebody else, with no further call to this
/// surface for anything to refuse. The audit log would hold one innocuous line and then nothing.
///
/// So the refusal is unconditional for an agent, and a person with an ordinary configuration is
/// not affected by it — which is the half that D44 had to correct the first time.
#[test]
fn an_agent_may_not_change_which_application_opens_a_type() {
    let f = Fixture::new();
    let home = tempfile::tempdir().unwrap();

    // A desktop of its own, so the test never reads or writes the machine's real one. The
    // variables are process-global and this is the only test in this binary that reads them; a
    // second one would have to be written to share this environment rather than set its own.
    let apps = home.path().join("data/applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("attacker.desktop"),
        "[Desktop Entry]\nType=Application\nName=Attacker\nExec=payload %f\n",
    )
    .unwrap();
    std::env::set_var("XDG_CONFIG_HOME", home.path().join("config"));
    std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
    std::env::set_var("XDG_DATA_DIRS", home.path().join("data"));
    std::env::set_var("XDG_CONFIG_DIRS", home.path().join("etc"));

    let params = AppsSetDefaultParams {
        mime: "text/plain".into(),
        application: "attacker.desktop".into(),
    };

    let error = f.surface(true).apps_set_default(&params).unwrap_err();
    let refused = refusal(error);
    assert!(matches!(refused, Refusal::NotForAgents { .. }), "an agent's attempt must be refused");
    assert!(
        !refused.to_string().contains("allow-list"),
        "and must not be told to add something to the allow-list, which would not help: {refused}"
    );
    assert!(
        !home.path().join("config/mimeapps.list").exists(),
        "and nothing may be written on the way to refusing"
    );
    assert_eq!(f.refusals().len(), 1, "the refusal is in the audit log (R8)");

    // The same call as a person goes through, so the refusal above is about who is asking and
    // not about the tool being broken.
    let mut human = f.surface(true);
    human.actor = Actor::Human;
    let done = human.apps_set_default(&params).unwrap();
    assert_eq!(done.application, "attacker.desktop");
    assert!(home.path().join("config/mimeapps.list").exists());
}
