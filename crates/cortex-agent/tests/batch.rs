//! Moving and copying several things at once, which is what a file manager mostly does.
//!
//! The interesting cases are not the happy ones. Five files dragged into a folder is one
//! gesture, so what happens when the third of them fails decides whether the person can trust
//! the other four — and whether they can undo what did happen.

use cortex_agent::batch::BatchOutcome;
use cortex_agent::fs_tools::Surface;
use cortex_agent::plans::PlanBook;
use cortex_agent::policy::Policy;
use cortex_agent::transfer::Handle;
use cortex_agent::Auditor;
use cortex_core::config::AgentConfig;
use cortex_core::model::Actor;
use cortex_core::tools::{ConflictChoice, TransferMode, UndoParams};
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    trash: cortex_agent::trash::Trash,
    store: Arc<Store>,
    auditor: Auditor,
    plans: PlanBook,
    policy: Policy,
}

impl Env {
    /// A human's surface: no allow-list, no plan required (D44). That is the file manager's
    /// case, and the agent's case is covered by the red-team suite.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let config = AgentConfig {
            enabled: true,
            write_allowlist: Vec::new(),
            require_dry_run: true,
            undo_journal_depth: 1000,
        };
        let store = Arc::new(Store::open_in_memory().unwrap());
        let auditor = Auditor::new(store.clone(), &Actor::Human);

        Self {
            policy: Policy::new(&config, vec![]),
            trash: cortex_agent::trash::Trash::at(dir.path().join("Trash")),
            _dir: dir,
            root,
            store,
            auditor,
            plans: PlanBook::new(120),
        }
    }

    fn surface(&self) -> Surface<'_> {
        Surface {
            policy: &self.policy,
            enabled: true,
            store: &self.store,
            auditor: &self.auditor,
            plans: &self.plans,
            trash: &self.trash,
            actor: Actor::Human,
            now: 1000,
        }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn run(
        &self,
        sources: &[PathBuf],
        destination: &Path,
        mode: TransferMode,
        conflict: ConflictChoice,
    ) -> BatchOutcome {
        let (resolved, destination, total) =
            self.surface().plan_transfer(sources, destination, mode).unwrap();
        self.surface().run_transfer(
            &resolved,
            &destination,
            mode,
            conflict,
            total,
            &Handle::new(),
            |_| {},
        )
    }
}

#[test]
fn several_files_move_in_one_gesture() {
    let env = Env::new();
    let sources: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|n| env.file(n, n))
        .collect();
    let target = env.dir("target");

    let outcome = env.run(&sources, &target, TransferMode::Move, ConflictChoice::Fail);

    assert_eq!(outcome.moved.len(), 3);
    assert!(outcome.failed.is_empty());
    assert!(!outcome.cancelled);
    for name in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(std::fs::read_to_string(target.join(name)).unwrap(), name);
        assert!(!env.root.join(name).exists());
    }
}

#[test]
fn one_failure_does_not_stop_the_others() {
    let env = Env::new();
    let good_one = env.file("a.txt", "a");
    let good_two = env.file("c.txt", "c");
    let target = env.dir("target");

    // The middle one is already at the other end, and the default is to refuse.
    let clashing = env.file("b.txt", "new");
    std::fs::write(target.join("b.txt"), "old").unwrap();

    let outcome = env.run(
        &[good_one, clashing.clone(), good_two],
        &target,
        TransferMode::Move,
        ConflictChoice::Fail,
    );

    assert_eq!(outcome.moved.len(), 2, "the other two must still have moved");
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].source, clashing);
    assert!(outcome.failed[0].reason.contains("exists"), "{}", outcome.failed[0].reason);

    assert!(clashing.exists(), "a source that failed is still where it was");
    assert_eq!(std::fs::read_to_string(target.join("b.txt")).unwrap(), "old");
}

#[test]
fn a_partly_failed_batch_is_still_undoable() {
    let env = Env::new();
    let good = env.file("a.txt", "a");
    let clashing = env.file("b.txt", "new");
    let target = env.dir("target");
    std::fs::write(target.join("b.txt"), "old").unwrap();

    let outcome = env.run(
        &[good.clone(), clashing],
        &target,
        TransferMode::Move,
        ConflictChoice::Fail,
    );
    assert_eq!(outcome.undo_ids.len(), 1, "one entry for the one that worked");

    env.surface()
        .undo(&UndoParams {
            last: Some(10),
            plan: None,
            dry_run: false,
        })
        .unwrap();

    assert!(good.exists(), "the one that moved came back");
    assert_eq!(
        std::fs::read_to_string(target.join("b.txt")).unwrap(),
        "old",
        "and the one that never moved was left alone"
    );
}

#[test]
fn keeping_both_records_an_inverse_that_points_at_what_arrived() {
    let env = Env::new();
    let source = env.file("notes.md", "new");
    let target = env.dir("target");
    std::fs::write(target.join("notes.md"), "old").unwrap();

    let outcome = env.run(&[source], &target, TransferMode::Move, ConflictChoice::Keep);
    assert_eq!(outcome.moved.len(), 1);
    let arrived = &outcome.moved[0];
    assert_eq!(arrived.file_name().unwrap(), "notes (2).md");

    // The journal was written before the name was chosen. If it were not corrected afterwards,
    // this undo would delete nothing and report success.
    env.surface()
        .undo(&UndoParams { last: Some(1), plan: None, dry_run: false })
        .unwrap();

    assert!(!arrived.exists(), "the copy that arrived was not removed");
    assert_eq!(std::fs::read_to_string(target.join("notes.md")).unwrap(), "old");
    assert!(env.root.join("notes.md").exists(), "the source came back");
}

#[test]
fn skipping_leaves_the_existing_file_alone_and_reports_no_failure() {
    let env = Env::new();
    let source = env.file("notes.md", "new");
    let target = env.dir("target");
    std::fs::write(target.join("notes.md"), "old").unwrap();

    let outcome = env.run(&[source.clone()], &target, TransferMode::Move, ConflictChoice::Skip);

    assert!(outcome.moved.is_empty());
    assert!(outcome.failed.is_empty(), "skipping is a choice, not a failure");
    assert_eq!(std::fs::read_to_string(target.join("notes.md")).unwrap(), "old");
    assert!(source.exists());
}

#[test]
fn copying_leaves_the_source_where_it_was() {
    let env = Env::new();
    let source = env.file("a.txt", "content");
    let target = env.dir("target");

    let outcome = env.run(&[source.clone()], &target, TransferMode::Copy, ConflictChoice::Fail);

    assert_eq!(outcome.moved.len(), 1);
    assert!(source.exists());
    assert_eq!(std::fs::read_to_string(target.join("a.txt")).unwrap(), "content");

    // And undoing a copy removes the copy rather than the original.
    env.surface()
        .undo(&UndoParams { last: Some(1), plan: None, dry_run: false })
        .unwrap();
    assert!(source.exists(), "undoing a copy deleted the original");
    assert!(!target.join("a.txt").exists());
}

#[test]
fn a_directory_cannot_be_moved_into_itself() {
    let env = Env::new();
    let outer = env.dir("outer");
    env.file("outer/inner/file.txt", "x");
    let inner = outer.join("inner");

    let error = env
        .surface()
        .plan_transfer(&[outer.clone()], &inner, TransferMode::Move)
        .unwrap_err();
    assert!(error.to_string().contains("inside itself"), "{error}");
}

#[test]
fn the_total_is_known_before_anything_moves() {
    let env = Env::new();
    let a = env.file("a.txt", "1234567890");
    let b = env.file("sub/b.txt", "12345");
    let target = env.dir("target");

    let (_, _, total) = env
        .surface()
        .plan_transfer(&[a, b], &target, TransferMode::Copy)
        .unwrap();

    assert_eq!(total.files, 2);
    assert_eq!(total.bytes, 15, "a progress bar without a denominator is a spinner");
}

#[test]
fn a_destination_that_is_not_a_directory_is_refused_before_anything_happens() {
    let env = Env::new();
    let source = env.file("a.txt", "a");
    let not_a_dir = env.file("file.txt", "x");

    let error = env
        .surface()
        .plan_transfer(&[source.clone()], &not_a_dir, TransferMode::Move)
        .unwrap_err();
    assert!(error.to_string().contains("not a directory"), "{error}");
    assert!(source.exists());
}

#[test]
fn cancelling_between_files_stops_the_batch() {
    let env = Env::new();
    let sources: Vec<PathBuf> = (0..5).map(|i| env.file(&format!("f{i}.txt"), "x")).collect();
    let target = env.dir("target");

    let (resolved, destination, total) = env
        .surface()
        .plan_transfer(&sources, &target, TransferMode::Move)
        .unwrap();

    // Cancelled before it starts, which is the batch-level check: `run_transfer` looks between
    // sources, and a move within one filesystem is a rename with no inside to look in.
    let handle = Handle::new();
    handle.cancel();
    let outcome = env.surface().run_transfer(
        &resolved,
        &destination,
        TransferMode::Move,
        ConflictChoice::Fail,
        total,
        &handle,
        |_| {},
    );

    assert!(outcome.cancelled);
    assert!(outcome.moved.is_empty());
    for source in &sources {
        assert!(source.exists(), "a cancelled batch moved something anyway");
    }
}

#[test]
fn cancelling_a_copy_keeps_what_already_arrived_and_nothing_half_written() {
    let env = Env::new();
    // Big enough that a copy takes several chunks and there is a middle to stop in. A move
    // would be a rename, which has no middle.
    let sources: Vec<PathBuf> = (0..4)
        .map(|i| env.file(&format!("f{i}.bin"), &"x".repeat(400_000)))
        .collect();
    let target = env.dir("target");

    let (resolved, destination, total) = env
        .surface()
        .plan_transfer(&sources, &target, TransferMode::Copy)
        .unwrap();

    let handle = Handle::new();
    let stopper = handle.clone();
    let outcome = env.surface().run_transfer(
        &resolved,
        &destination,
        TransferMode::Copy,
        ConflictChoice::Fail,
        total,
        &handle,
        move |p| {
            if p.files_done >= 2 {
                stopper.cancel();
            }
        },
    );

    assert!(outcome.cancelled);
    assert!(outcome.moved.len() < 4, "cancelling copied everything anyway");
    assert_eq!(outcome.undo_ids.len(), outcome.moved.len());

    // Everything that arrived is whole, and the file that was interrupted left nothing.
    for arrived in &outcome.moved {
        assert_eq!(std::fs::metadata(arrived).unwrap().len(), 400_000);
    }
    let leftovers: Vec<String> = std::fs::read_dir(&target)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers.len(), outcome.moved.len(), "{leftovers:?}");
    assert!(!leftovers.iter().any(|n| n.contains("hippocampus-part")), "{leftovers:?}");
}

#[test]
fn making_a_directory_is_undoable_and_refuses_to_bury_a_typo() {
    let env = Env::new();

    let made = env.surface().mkdir(&env.root.join("neu")).unwrap();
    assert!(made.is_dir());

    // `create_dir`, not `create_dir_all`: three levels because two were misspelled is not what
    // anybody asked for, and the inverse would only remove one of them.
    let error = env
        .surface()
        .mkdir(&env.root.join("a/b/c"))
        .unwrap_err();
    assert!(!env.root.join("a").exists(), "{error}");

    env.surface()
        .undo(&UndoParams { last: Some(1), plan: None, dry_run: false })
        .unwrap();
    assert!(!made.exists());
}

#[test]
fn undoing_a_new_directory_leaves_it_alone_once_it_has_something_in_it() {
    let env = Env::new();
    let made = env.surface().mkdir(&env.root.join("neu")).unwrap();
    std::fs::write(made.join("somebody-elses.txt"), "work").unwrap();

    env.surface()
        .undo(&UndoParams { last: Some(1), plan: None, dry_run: false })
        .unwrap();

    assert!(
        made.exists(),
        "removing a directory somebody has since filled would undo more than was done"
    );
}

#[test]
fn renaming_stays_in_its_directory() {
    let env = Env::new();
    let file = env.file("before.txt", "content");

    let renamed = env.surface().rename(&file, "after.txt").unwrap();
    assert_eq!(renamed, env.root.join("after.txt"));
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "content");
    assert!(!file.exists());

    env.surface()
        .undo(&UndoParams { last: Some(1), plan: None, dry_run: false })
        .unwrap();
    assert!(file.exists());
}

#[test]
fn a_rename_cannot_be_a_move_in_disguise() {
    let env = Env::new();
    let file = env.file("sub/a.txt", "x");
    env.dir("elsewhere");

    for attempt in ["../elsewhere/a.txt", "sub/a.txt", "", "/etc/passwd"] {
        let error = env.surface().rename(&file, attempt).unwrap_err();
        assert!(
            error.to_string().contains("not a path")
                || error.to_string().contains("stays in its directory"),
            "`{attempt}` was accepted or refused for the wrong reason: {error}"
        );
    }
    assert!(file.exists());
}

#[test]
fn renaming_onto_an_existing_name_is_refused() {
    let env = Env::new();
    let file = env.file("a.txt", "a");
    env.file("b.txt", "b");

    let error = env.surface().rename(&file, "b.txt").unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(std::fs::read_to_string(env.root.join("b.txt")).unwrap(), "b");
}
