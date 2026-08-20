//! M3 §3: an agent moves 100 files, `cortex undo` restores the exact prior state, verified by
//! the hash of a canonical filesystem dump.
//!
//! The dump is the instrument, and it is deliberately stricter than the claim: it covers every
//! path under the tree, its length and its content hash, sorted. Two trees with the same dump
//! hash differ in nothing this project can observe.

use cortex_agent::fs_tools::Surface;
use cortex_agent::plans::PlanBook;
use cortex_agent::policy::Policy;
use cortex_agent::Auditor;
use cortex_core::config::AgentConfig;
use cortex_core::model::Actor;
use cortex_core::tools::*;
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
    clock: std::cell::Cell<i64>,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let config = AgentConfig {
            enabled: true,
            write_allowlist: vec![root.clone()],
            require_dry_run: true,
            undo_journal_depth: 1000,
        };
        let store = Arc::new(Store::open_in_memory().unwrap());
        let auditor = Auditor::new(store.clone(), &Actor::Agent("mover".into()));

        Self {
            policy: Policy::new(&config, vec![]),
            trash: cortex_agent::trash::Trash::at(dir.path().join("Trash")),
            _dir: dir,
            root,
            store,
            auditor,
            plans: PlanBook::new(600),
            clock: std::cell::Cell::new(1000),
        }
    }

    /// A surface whose clock has moved on, so that trash slots and journal entries do not all
    /// share one timestamp.
    fn surface(&self) -> Surface<'_> {
        self.clock.set(self.clock.get() + 1);
        Surface {
            policy: &self.policy,
            enabled: true,
            store: &self.store,
            auditor: &self.auditor,
            plans: &self.plans,
            trash: &self.trash,
            actor: Actor::Agent("mover".into()),
            now: self.clock.get(),
        }
    }

    fn dump_hash(&self) -> String {
        canonical_dump_hash(&self.root)
    }
}

/// Every path under `root`, with its size and content hash, sorted, hashed.
fn canonical_dump_hash(root: &Path) -> String {
    let mut lines = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            let md = entry.metadata().unwrap();
            if md.is_dir() {
                lines.push(format!("d {rel}"));
                stack.push(path);
            } else {
                let content = std::fs::read(&path).unwrap_or_default();
                lines.push(format!(
                    "f {rel} {} {}",
                    md.len(),
                    blake3::hash(&content).to_hex()
                ));
            }
        }
    }
    lines.sort();
    blake3::hash(lines.join("\n").as_bytes()).to_hex().to_string()
}

/// Plan a mutation and apply it in one go — what a well-behaved agent does, and what every
/// test below wants without repeating six lines each time.
fn confirmed<F>(mut call: F) -> Mutation
where
    F: FnMut(Option<String>) -> Result<Mutation, cortex_agent::ToolError>,
{
    match call(None).unwrap() {
        Mutation::Planned { plan } => call(Some(plan.id)).unwrap(),
        applied => panic!("expected a plan first, got {applied:?}"),
    }
}

#[test]
fn a_hundred_moves_are_reversed_exactly() {
    let env = Env::new();
    let src = env.root.join("src");
    let dst = env.root.join("dst");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();

    for i in 0..100 {
        std::fs::write(src.join(format!("file-{i:03}.txt")), format!("contents of {i}")).unwrap();
    }
    let before = env.dump_hash();

    for i in 0..100 {
        let name = format!("file-{i:03}.txt");
        let from = src.join(&name);
        let to = dst.join(&name);
        confirmed(|plan| {
            env.surface().move_path(&FsMoveParams {
                from: from.clone(),
                to: to.clone(),
                plan,
            })
        });
    }

    assert_ne!(env.dump_hash(), before, "the moves did nothing at all");
    assert_eq!(std::fs::read_dir(&src).unwrap().count(), 0);
    assert_eq!(env.store.undo_depth().unwrap(), 100);

    let result = env
        .surface()
        .undo(&UndoParams {
            last: Some(100),
            plan: None,
            dry_run: false,
        })
        .unwrap();

    assert_eq!(result.undone.len(), 100);
    assert!(result.failed.is_empty(), "undo failed: {:?}", result.failed);
    assert_eq!(result.remaining, 0);
    assert_eq!(
        env.dump_hash(),
        before,
        "the tree after undo is not byte-for-byte the tree before the moves"
    );
}

#[test]
fn an_overwrite_is_reversed_from_the_trash() {
    let env = Env::new();
    let target = env.root.join("notes.txt");
    std::fs::write(&target, "the original, which must come back").unwrap();
    let before = env.dump_hash();

    confirmed(|plan| {
        env.surface().write(&FsWriteParams {
            path: target.clone(),
            content: "replaced".into(),
            mode: WriteMode::Overwrite,
            plan,
        })
    });
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "replaced");

    env.surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: false,
        })
        .unwrap();

    assert_eq!(env.dump_hash(), before);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "the original, which must come back"
    );
}

#[test]
fn a_creation_is_reversed_by_deleting_it() {
    let env = Env::new();
    let before = env.dump_hash();
    let target = env.root.join("new.txt");

    confirmed(|plan| {
        env.surface().write(&FsWriteParams {
            path: target.clone(),
            content: "hello".into(),
            mode: WriteMode::Create,
            plan,
        })
    });
    assert!(target.exists());

    env.surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: false,
        })
        .unwrap();
    assert!(!target.exists());
    assert_eq!(env.dump_hash(), before);
}

#[test]
fn an_append_is_reversed_by_truncating_it() {
    let env = Env::new();
    let target = env.root.join("log.txt");
    std::fs::write(&target, "line one\n").unwrap();
    let before = env.dump_hash();

    confirmed(|plan| {
        env.surface().write(&FsWriteParams {
            path: target.clone(),
            content: "line two\n".into(),
            mode: WriteMode::Append,
            plan,
        })
    });
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "line one\nline two\n");

    env.surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: false,
        })
        .unwrap();
    assert_eq!(env.dump_hash(), before);
}

#[test]
fn trashing_is_reversed_by_putting_the_file_back() {
    let env = Env::new();
    let target = env.root.join("keep.txt");
    std::fs::write(&target, "not actually rubbish").unwrap();
    let before = env.dump_hash();

    confirmed(|plan| {
        env.surface().trash(&FsTrashParams {
            path: target.clone(),
            plan,
        })
    });
    assert!(!target.exists(), "trash left the file where it was");

    env.surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: false,
        })
        .unwrap();
    assert_eq!(env.dump_hash(), before);
}

#[test]
fn a_dry_run_undo_changes_nothing() {
    let env = Env::new();
    let target = env.root.join("f.txt");
    confirmed(|plan| {
        env.surface().write(&FsWriteParams {
            path: target.clone(),
            content: "x".into(),
            mode: WriteMode::Create,
            plan,
        })
    });
    let after_write = env.dump_hash();

    let result = env
        .surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: true,
        })
        .unwrap();

    assert_eq!(result.undone.len(), 1);
    assert!(result.dry_run);
    assert_eq!(env.dump_hash(), after_write, "a dry run undid something");
    assert_eq!(env.store.undo_depth().unwrap(), 1, "the entry is still undoable");
}

#[test]
fn undo_refuses_to_overwrite_something_that_took_the_place() {
    let env = Env::new();
    let target = env.root.join("contested.txt");
    std::fs::write(&target, "original").unwrap();

    confirmed(|plan| {
        env.surface().trash(&FsTrashParams {
            path: target.clone(),
            plan,
        })
    });
    // Somebody put a different file there in the meantime.
    std::fs::write(&target, "somebody else's work").unwrap();

    let result = env
        .surface()
        .undo(&UndoParams {
            last: Some(1),
            plan: None,
            dry_run: false,
        })
        .unwrap();

    assert!(result.undone.is_empty());
    assert_eq!(result.failed.len(), 1);
    // The contract is that the failure says *which* file it could not put back, not that it
    // uses a particular word for it. The first version of this asserted the word and broke
    // when the trash started phrasing it better.
    assert!(
        result.failed[0].reason.contains(&target.display().to_string()),
        "the reason does not name the file: {}",
        result.failed[0].reason
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "somebody else's work",
        "undo destroyed work it did not create"
    );
    assert_eq!(
        env.store.undo_depth().unwrap(),
        1,
        "a failed undo must stay undoable"
    );
}

#[test]
fn the_journal_is_bounded() {
    let env = Env::new();
    let mut config = AgentConfig {
        enabled: true,
        write_allowlist: vec![env.root.clone()],
        require_dry_run: false,
        undo_journal_depth: 5,
    };
    config.require_dry_run = false;
    let policy = Policy::new(&config, vec![]);

    for i in 0..20 {
        let surface = Surface {
            policy: &policy,
            enabled: true,
            store: &env.store,
            auditor: &env.auditor,
            plans: &env.plans,
            trash: &env.trash,
            actor: Actor::Agent("mover".into()),
            now: 2000 + i,
        };
        surface
            .write(&FsWriteParams {
                path: env.root.join(format!("f{i}.txt")),
                content: "x".into(),
                mode: WriteMode::Create,
                plan: None,
            })
            .unwrap();
    }

    assert_eq!(
        env.store.undo_depth().unwrap(),
        5,
        "the journal grew past its configured depth"
    );
}

#[test]
fn every_mutation_leaves_an_episode_attributed_to_the_agent() {
    let env = Env::new();
    let target = env.root.join("attributed.txt");

    confirmed(|plan| {
        env.surface().write(&FsWriteParams {
            path: target.clone(),
            content: "x".into(),
            mode: WriteMode::Create,
            plan,
        })
    });

    let episodes = env.store.recent_episodes(10).unwrap();
    let mine: Vec<_> = episodes
        .iter()
        .filter(|e| e.actor == Actor::Agent("mover".into()))
        .collect();
    assert!(!mine.is_empty(), "a mutation left no attributed episode");
    assert_eq!(mine[0].action, cortex_core::model::Action::Create);
}
