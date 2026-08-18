//! Metadata indexing by directory walk.
//!
//! M0 scope: a manual, on-demand walk. The `fanotify` watcher in M1 replaces this as the
//! primary source of events, but the walk survives as the **reconcile scan** — watcher events
//! are dropped under load, and an index that silently drifts is worse than no index.
//!
//! No file is ever opened here. Metadata only, per rule R3.

use cortex_core::denylist;
use cortex_core::model::{Action, Actor, NodeKind};
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexOutcome {
    pub indexed: u64,
    /// Entries deliberately not indexed: sensitive paths and skipped subtrees.
    pub skipped: u64,
}

/// Walk `root` and record what is there.
///
/// Errors on individual entries are logged and skipped rather than aborting the walk: an
/// unreadable directory somewhere in a home tree is normal, and it must not cost the user the
/// other ninety-nine per cent of their index.
pub fn index_path(
    store: &Store,
    root: &Path,
    recursive: bool,
    extra_deny: &[String],
) -> IndexOutcome {
    let mut outcome = IndexOutcome::default();
    let now = unix_now();

    let mut queue: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        // The root itself may be a plain file.
        if dir == root && dir.is_file() {
            index_entry(store, &dir, NodeKind::File, now, extra_deny, &mut outcome);
            continue;
        }

        index_entry(store, &dir, NodeKind::Dir, now, extra_deny, &mut outcome);

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::debug!(path = %dir.display(), error = %e, "skipping unreadable directory");
                continue;
            }
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();

            let name = entry.file_name();
            let name = name.to_string_lossy();

            // Skipping the whole subtree is what keeps a home-directory walk affordable:
            // descending into node_modules costs far more than the paths are worth.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && denylist::should_skip_subtree(&name) {
                outcome.skipped += 1;
                continue;
            }

            if is_dir {
                if recursive {
                    queue.push(path);
                } else {
                    index_entry(store, &path, NodeKind::Dir, now, extra_deny, &mut outcome);
                }
            } else {
                index_entry(store, &path, NodeKind::File, now, extra_deny, &mut outcome);
            }
        }

        if !recursive {
            break;
        }
    }

    outcome
}

fn index_entry(
    store: &Store,
    path: &Path,
    kind: NodeKind,
    now: i64,
    extra_deny: &[String],
    outcome: &mut IndexOutcome,
) {
    // Rule R1: the verdict is consulted before anything touches the file, and a sensitive path
    // does not enter the index at all — not even as a path.
    if !denylist::classify(path, extra_deny).may_index() {
        outcome.skipped += 1;
        return;
    }

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "skipping unreadable entry");
            outcome.skipped += 1;
            return;
        }
    };

    let upsert = NodeUpsert {
        kind,
        path: path.to_path_buf(),
        parent_id: None,
        mime: None,
        size: metadata.len() as i64,
        mtime: system_time_to_unix(metadata.modified().ok()),
        ctime: system_time_to_unix(metadata.created().ok()),
        inode: inode_of(&metadata),
        dev: device_of(&metadata),
        zone_id: None,
    };

    match store.upsert_node(&upsert, now) {
        Ok(node_id) => {
            outcome.indexed += 1;
            // The episode log is the ground truth from which everything else is rebuilt, so
            // even a plain indexing pass records that it happened.
            if let Err(e) = store.append_episode(
                now,
                &Actor::System,
                Action::Create,
                Some(node_id),
                None,
                &serde_json::json!({ "source": "walk" }),
            ) {
                tracing::warn!(error = %e, "could not record an episode");
            }
        }
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "could not index entry");
            outcome.skipped += 1;
        }
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn system_time_to_unix(time: Option<SystemTime>) -> i64 {
    time.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn inode_of(metadata: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino() as i64
}

#[cfg(unix)]
fn device_of(metadata: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    metadata.dev() as i64
}

#[cfg(not(unix))]
fn inode_of(_metadata: &std::fs::Metadata) -> i64 {
    0
}

#[cfg(not(unix))]
fn device_of(_metadata: &std::fs::Metadata) -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("proj/src")).unwrap();
        std::fs::create_dir_all(root.join("proj/node_modules/react")).unwrap();
        std::fs::create_dir_all(root.join(".ssh")).unwrap();

        std::fs::write(root.join("docs/report.md"), "hello").unwrap();
        std::fs::write(root.join("docs/notes.txt"), "world").unwrap();
        std::fs::write(root.join("proj/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("proj/node_modules/react/index.js"), "x").unwrap();
        std::fs::write(root.join(".ssh/id_ed25519"), "PRIVATE KEY").unwrap();
        std::fs::write(root.join(".env"), "TOKEN=abc").unwrap();

        tmp
    }

    fn indexed_paths(store: &Store) -> Vec<String> {
        store.all_paths().unwrap()
    }

    #[test]
    fn a_recursive_walk_finds_ordinary_files() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        let outcome = index_path(&store, tmp.path(), true, &[]);

        assert!(outcome.indexed > 0);
        let paths = indexed_paths(&store);
        assert!(paths.iter().any(|p| p.ends_with("docs/report.md")));
        assert!(paths.iter().any(|p| p.ends_with("proj/src/main.rs")));
    }

    #[test]
    fn secrets_never_enter_the_index() {
        // This is hard rule R1 and it is the one test in this file that must never be relaxed.
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        index_path(&store, tmp.path(), true, &[]);

        let paths = indexed_paths(&store);
        for forbidden in [".ssh", "id_ed25519", ".env"] {
            assert!(
                !paths.iter().any(|p| p.contains(forbidden)),
                "{forbidden} leaked into the index: {paths:?}"
            );
        }
    }

    #[test]
    fn bulky_subtrees_are_skipped_entirely() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        let outcome = index_path(&store, tmp.path(), true, &[]);

        let paths = indexed_paths(&store);
        assert!(
            !paths.iter().any(|p| p.contains("node_modules")),
            "node_modules should not be walked"
        );
        assert!(outcome.skipped > 0, "skipped entries should be counted");
    }

    #[test]
    fn a_non_recursive_walk_stays_in_one_directory() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        index_path(&store, &tmp.path().join("proj"), false, &[]);

        let paths = indexed_paths(&store);
        assert!(
            !paths.iter().any(|p| p.ends_with("main.rs")),
            "a shallow walk must not descend: {paths:?}"
        );
    }

    #[test]
    fn indexing_a_single_file_works() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        let outcome = index_path(&store, &tmp.path().join("docs/report.md"), true, &[]);

        assert_eq!(outcome.indexed, 1);
        assert_eq!(indexed_paths(&store).len(), 1);
    }

    #[test]
    fn walking_twice_updates_rather_than_duplicates() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();

        index_path(&store, tmp.path(), true, &[]);
        let first = indexed_paths(&store);
        index_path(&store, tmp.path(), true, &[]);
        let second = indexed_paths(&store);

        assert_eq!(first, second, "a second pass must not duplicate nodes");
    }

    #[test]
    fn a_missing_root_is_reported_rather_than_panicking() {
        let store = Store::open_in_memory().unwrap();
        let outcome = index_path(&store, Path::new("/definitely/not/here"), true, &[]);
        assert_eq!(outcome.indexed, 0);
    }

    #[test]
    fn extra_deny_patterns_are_honoured() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        index_path(&store, tmp.path(), true, &["*.md".to_string()]);

        let paths = indexed_paths(&store);
        assert!(
            !paths.iter().any(|p| p.ends_with(".md")),
            "a user deny pattern must be applied: {paths:?}"
        );
        assert!(paths.iter().any(|p| p.ends_with("notes.txt")));
    }

    #[test]
    fn indexing_records_episodes() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        let outcome = index_path(&store, tmp.path(), true, &[]);

        let stats = store.stats().unwrap();
        assert_eq!(
            stats.episodes as u64, outcome.indexed,
            "every indexed node should leave a trace in the episode log"
        );
    }

    #[test]
    fn file_size_and_mtime_are_recorded() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        index_path(&store, &tmp.path().join("docs/report.md"), true, &[]);

        let node = store
            .node_by_path(&tmp.path().join("docs/report.md"))
            .unwrap()
            .unwrap();
        assert_eq!(node.size, 5, "\"hello\" is five bytes");
        assert!(node.mtime > 0, "mtime should be populated");
    }
}
