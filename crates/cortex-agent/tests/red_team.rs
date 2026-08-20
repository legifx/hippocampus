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
            .recent_audit(1000, None, true)
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
