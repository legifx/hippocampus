//! Reconciliation: repairing drift between the index and the filesystem.
//!
//! **This is not a fallback, it is the correctness guarantee.** Watcher events are lost — the
//! kernel queue overflows under load, the daemon is not running when a file changes, a watch
//! could not be placed because the budget was exhausted. An index that silently diverges from
//! the filesystem is worse than no index, because the user stops being able to tell the
//! difference between "not indexed yet" and "not there".
//!
//! The scan compares what the index believes against what `stat` reports and repairs the
//! difference. It is deliberately cheap: `stat` only, never opening a file.
//!
//! Moves are recognised by `(dev, inode)` surviving a path change. Without that, renaming a
//! directory of ten thousand files would look like ten thousand deletions followed by ten
//! thousand creations — losing every node's history, salience and embeddings, and forcing a
//! full re-extraction of unchanged content.

use cortex_core::denylist;
use cortex_core::ids::NodeId;
use cortex_core::model::{Action, Actor, NodeKind};
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Nodes written per transaction. Large enough that commit cost disappears, small enough that
/// a failure rolls back a comprehensible amount of work.
const BATCH_SIZE: usize = 2_000;

/// Above this many changes in one pass, record a single summary episode instead of one per file.
const EPISODE_DETAIL_LIMIT: usize = 1_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReconcileReport {
    /// Present on disk, absent from the index.
    pub added: u64,
    /// Present in both, but size or mtime disagreed.
    pub updated: u64,
    /// Present in the index, gone from disk.
    pub removed: u64,
    /// Same file, different path — identified by `(dev, inode)`.
    pub renamed: u64,
    /// Present in both and identical. The overwhelming majority in a healthy index.
    pub unchanged: u64,
    /// Skipped by the deny-list or as a bulky subtree.
    pub skipped: u64,
}

impl ReconcileReport {
    /// Whether the scan actually had to change anything.
    pub fn is_clean(&self) -> bool {
        self.added == 0 && self.updated == 0 && self.removed == 0 && self.renamed == 0
    }
}

/// One entry as it exists on disk right now.
struct OnDisk {
    path: PathBuf,
    kind: NodeKind,
    size: i64,
    mtime: i64,
    ctime: i64,
    inode: i64,
    dev: i64,
}

/// Compare `root` against the index and repair the difference.
///
/// Two passes, and the order is not incidental. Renames are resolved **first**, against a
/// snapshot of the index, and the index is then re-read before anything else happens. Doing it
/// in one pass means a directory rename rewrites the paths of its descendants while the loop is
/// still holding the pre-rename map — and the stale entries it then tries to move would
/// clobber the rows the first move just wrote.
pub fn reconcile(store: &Store, root: &Path, extra_deny: &[String], now: i64) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    let on_disk = scan_disk(root, extra_deny, &mut report);

    apply_renames(store, &on_disk, root, now, &mut report);

    // Re-read: the rename pass has moved whole subtrees, so anything read before it is stale.
    let indexed = match store.nodes_under(root) {
        Ok(nodes) => nodes,
        Err(e) => {
            tracing::error!(error = %e, "could not read the index for reconciliation");
            return report;
        }
    };
    let indexed_by_path: HashMap<&Path, &cortex_store::queries::IndexedNode> = indexed
        .iter()
        .map(|node| (node.path.as_path(), node))
        .collect();

    let mut seen: HashMap<&Path, ()> = HashMap::with_capacity(on_disk.len());
    let mut to_write: Vec<&OnDisk> = Vec::new();

    for entry in &on_disk {
        seen.insert(entry.path.as_path(), ());

        match indexed_by_path.get(entry.path.as_path()) {
            Some(known) => {
                if known.size != entry.size || known.mtime != entry.mtime {
                    to_write.push(entry);
                    report.updated += 1;
                } else {
                    report.unchanged += 1;
                }
            }
            None => {
                to_write.push(entry);
                report.added += 1;
            }
        }
    }

    write_batched(store, &to_write, now);

    // Whatever the index still knows that disk no longer shows really is gone: renames were
    // consumed in the first pass.
    //
    // Removal is applied shallowest-first, and a path already covered by an ancestor's removal
    // is skipped — otherwise deleting a directory would be counted once for the directory and
    // again for every file it took with it.
    let mut vanished: Vec<&cortex_store::queries::IndexedNode> = indexed
        .iter()
        .filter(|node| !seen.contains_key(node.path.as_path()))
        .collect();
    vanished.sort_by_key(|node| node.path.components().count());

    let mut removed_roots: Vec<PathBuf> = Vec::new();
    for node in vanished {
        if removed_roots.iter().any(|p| node.path.starts_with(p)) {
            continue;
        }
        match store.remove_path(&node.path) {
            Ok(count) if count > 0 => {
                record_episode(store, Action::Delete, Some(node.id), now, &node.path);
                report.removed += count as u64;
                removed_roots.push(node.path.clone());
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, path = %node.path.display(), "could not remove"),
        }
    }

    report
}

/// Recognise moves by `(dev, inode)` surviving a path change, and apply them.
///
/// Renames are applied shallowest-first so that moving a directory carries its descendants in
/// one operation. Every later rename whose source lies inside an already-moved subtree is then
/// redundant and is skipped.
fn apply_renames(
    store: &Store,
    on_disk: &[OnDisk],
    root: &Path,
    now: i64,
    report: &mut ReconcileReport,
) {
    let indexed = match store.nodes_under(root) {
        Ok(nodes) => nodes,
        Err(e) => {
            tracing::error!(error = %e, "could not read the index for rename detection");
            return;
        }
    };
    if indexed.is_empty() {
        return;
    }

    let indexed_paths: std::collections::HashSet<&Path> =
        indexed.iter().map(|n| n.path.as_path()).collect();
    let disk_paths: std::collections::HashSet<&Path> =
        on_disk.iter().map(|e| e.path.as_path()).collect();

    let mut by_identity: HashMap<(i64, i64), &cortex_store::queries::IndexedNode> = HashMap::new();
    for node in &indexed {
        if node.inode != 0 {
            by_identity.insert((node.dev, node.inode), node);
        }
    }

    let mut candidates: Vec<(PathBuf, PathBuf, NodeId)> = Vec::new();
    for entry in on_disk {
        if entry.inode == 0 || indexed_paths.contains(entry.path.as_path()) {
            continue;
        }
        let Some(previous) = by_identity.get(&(entry.dev, entry.inode)) else {
            continue;
        };
        // If the old path is still on disk, this is a hard link or a reused inode, not a move.
        // Treating it as a move would erase a file that is still there.
        if disk_paths.contains(previous.path.as_path()) {
            continue;
        }
        candidates.push((previous.path.clone(), entry.path.clone(), previous.id));
    }

    candidates.sort_by_key(|(from, _, _)| from.components().count());

    let mut moved_roots: Vec<PathBuf> = Vec::new();
    for (from, to, id) in candidates {
        if moved_roots.iter().any(|p| from.starts_with(p)) {
            continue;
        }
        match store.rename_node(&from, &to) {
            Ok(count) if count > 0 => {
                record_episode(store, Action::Move, Some(id), now, &to);
                report.renamed += 1;
                moved_roots.push(from);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, from = %from.display(), "could not record a move"),
        }
    }
}

/// Write entries in transaction-sized chunks.
///
/// Two things happen here that matter more than they look:
///
/// **Batching.** SQLite makes every autocommitted statement its own durability barrier. On the
/// reference corpus, writing a hundred thousand nodes one statement at a time took four minutes;
/// the same work in transactions of [`BATCH_SIZE`] takes seconds. The work was never the
/// bottleneck — the commits were.
///
/// **Episode summarisation.** A first index run, or a branch switch touching fifty thousand
/// files, is *one* thing that happened, not fifty thousand. Writing a per-file episode for each
/// would bury the episode log — the very structure "what was I working on on Tuesday" reads
/// from — under noise generated by the machine rather than by the human. Past
/// [`EPISODE_DETAIL_LIMIT`] changes in a single pass, one summary episode is recorded instead.
fn write_batched(store: &Store, entries: &[&OnDisk], now: i64) {
    if entries.is_empty() {
        return;
    }
    let detailed = entries.len() <= EPISODE_DETAIL_LIMIT;

    for chunk in entries.chunks(BATCH_SIZE) {
        let outcome = store.batch(|batch| {
            for entry in chunk {
                let record = NodeUpsert {
                    kind: entry.kind,
                    path: entry.path.clone(),
                    parent_id: None,
                    mime: None,
                    size: entry.size,
                    mtime: entry.mtime,
                    ctime: entry.ctime,
                    inode: entry.inode,
                    dev: entry.dev,
                    // Left unset: assign_zones works the whole index out at once afterwards.
                    zone_id: None,
                };
                let id = batch.upsert_node(&record, now)?;
                if detailed {
                    batch.append_episode(
                        now,
                        &Actor::System,
                        Action::Modify,
                        Some(id),
                        None,
                        &serde_json::json!({
                            "source": "reconcile",
                            "path": entry.path.display().to_string(),
                        }),
                    )?;
                }
            }
            Ok(())
        });

        if let Err(e) = outcome {
            tracing::warn!(error = %e, "a batch of index writes failed and was rolled back");
        }
    }

    if !detailed {
        let _ = store.append_episode(
            now,
            &Actor::System,
            Action::Create,
            None,
            None,
            &serde_json::json!({
                "source": "reconcile",
                "bulk": true,
                "count": entries.len(),
                "note": "bulk index pass; per-file episodes omitted",
            }),
        );
    }
}

fn record_episode(store: &Store, action: Action, node: Option<NodeId>, now: i64, path: &Path) {
    // Reconciliation is the system noticing something it missed, which is exactly the kind of
    // thing the episode log exists to remember.
    let _ = store.append_episode(
        now,
        &Actor::System,
        action,
        node,
        None,
        &serde_json::json!({ "source": "reconcile", "path": path.display().to_string() }),
    );
}

/// Walk the tree with `stat`, applying the deny-list. No file is ever opened.
fn scan_disk(root: &Path, extra_deny: &[String], report: &mut ReconcileReport) -> Vec<OnDisk> {
    let mut out = Vec::new();
    let mut queue = vec![root.to_path_buf()];

    while let Some(dir) = queue.pop() {
        if let Some(entry) = stat_entry(&dir, extra_deny, report) {
            out.push(entry);
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

            if is_dir {
                let name = entry.file_name();
                if denylist::should_skip_subtree(&name.to_string_lossy()) {
                    report.skipped += 1;
                    continue;
                }
                queue.push(path);
            } else if let Some(entry) = stat_entry(&path, extra_deny, report) {
                out.push(entry);
            }
        }
    }

    out
}

fn stat_entry(path: &Path, extra_deny: &[String], report: &mut ReconcileReport) -> Option<OnDisk> {
    // Rule R1: the verdict comes before anything touches the file.
    if !denylist::classify(path, extra_deny).may_index() {
        report.skipped += 1;
        return None;
    }

    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some(OnDisk {
        path: path.to_path_buf(),
        kind: if metadata.is_dir() {
            NodeKind::Dir
        } else {
            NodeKind::File
        },
        size: metadata.len() as i64,
        mtime: unix_seconds(metadata.modified().ok()),
        ctime: unix_seconds(metadata.created().ok()),
        inode: inode_of(&metadata),
        dev: device_of(&metadata),
    })
}

fn unix_seconds(time: Option<std::time::SystemTime>) -> i64 {
    time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
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
        std::fs::write(root.join("docs/one.md"), "one").unwrap();
        std::fs::write(root.join("docs/two.md"), "two").unwrap();
        std::fs::write(root.join("proj/src/main.rs"), "fn main() {}").unwrap();
        tmp
    }

    fn paths(store: &Store) -> Vec<String> {
        store.all_paths().unwrap()
    }

    #[test]
    fn a_first_scan_indexes_everything() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();

        let report = reconcile(&store, tmp.path(), &[], 100);

        assert!(report.added > 0);
        assert_eq!(report.removed, 0);
        assert!(paths(&store).iter().any(|p| p.ends_with("docs/one.md")));
    }

    #[test]
    fn a_second_scan_changes_nothing() {
        // The common case by far, and the one that must be cheap and quiet.
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);

        let report = reconcile(&store, tmp.path(), &[], 200);

        assert!(report.is_clean(), "an unchanged tree should reconcile clean: {report:?}");
        assert!(report.unchanged > 0);
    }

    #[test]
    fn a_file_created_while_the_watcher_was_blind_is_picked_up() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);

        std::fs::write(tmp.path().join("docs/appeared.md"), "new").unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        assert_eq!(report.added, 1);
        assert!(paths(&store).iter().any(|p| p.ends_with("appeared.md")));
    }

    #[test]
    fn a_file_deleted_while_the_watcher_was_blind_is_removed() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);

        std::fs::remove_file(tmp.path().join("docs/two.md")).unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        assert_eq!(report.removed, 1);
        assert!(!paths(&store).iter().any(|p| p.ends_with("two.md")));
    }

    #[test]
    fn a_changed_file_is_updated_rather_than_re_added() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);
        let before = store
            .node_by_path(&tmp.path().join("docs/one.md"))
            .unwrap()
            .unwrap();

        // Changing the size is enough; mtime resolution is one second here, so relying on
        // mtime alone would make this test depend on how fast the machine is.
        std::fs::write(tmp.path().join("docs/one.md"), "one, considerably longer now").unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        assert_eq!(report.updated, 1);
        assert_eq!(report.added, 0);
        let after = store
            .node_by_path(&tmp.path().join("docs/one.md"))
            .unwrap()
            .unwrap();
        assert_eq!(before.id, after.id, "an update must not replace the node");
        assert!(after.size > before.size);
    }

    #[test]
    fn a_rename_is_recognised_by_identity_rather_than_as_a_delete_and_create() {
        // The property that makes moving a large directory affordable.
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);
        let before = store
            .node_by_path(&tmp.path().join("docs/one.md"))
            .unwrap()
            .unwrap();

        std::fs::rename(
            tmp.path().join("docs/one.md"),
            tmp.path().join("docs/renamed.md"),
        )
        .unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        assert_eq!(report.renamed, 1, "expected a rename, got {report:?}");
        assert_eq!(report.added, 0);
        assert_eq!(report.removed, 0);

        let after = store
            .node_by_path(&tmp.path().join("docs/renamed.md"))
            .unwrap()
            .unwrap();
        assert_eq!(before.id, after.id, "identity must survive a rename");
    }

    #[test]
    fn moving_a_directory_does_not_churn_its_contents() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);
        let before = store
            .node_by_path(&tmp.path().join("proj/src/main.rs"))
            .unwrap()
            .unwrap();

        std::fs::rename(tmp.path().join("proj"), tmp.path().join("project")).unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        assert_eq!(report.added, 0, "moving a directory must not re-create its files: {report:?}");
        assert_eq!(report.removed, 0, "nor delete them: {report:?}");

        let after = store
            .node_by_path(&tmp.path().join("project/src/main.rs"))
            .unwrap()
            .unwrap();
        assert_eq!(before.id, after.id);
    }

    #[test]
    fn secrets_are_not_indexed_by_reconciliation_either() {
        // The deny-list has to hold on every path into the index, not just the walker.
        let tmp = corpus();
        std::fs::create_dir_all(tmp.path().join(".ssh")).unwrap();
        std::fs::write(tmp.path().join(".ssh/id_ed25519"), "NOT-A-REAL-KEY").unwrap();
        std::fs::write(tmp.path().join(".env"), "NOT_A_REAL_TOKEN=x").unwrap();

        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);

        let indexed = paths(&store);
        for forbidden in [".ssh", "id_ed25519", ".env"] {
            assert!(
                !indexed.iter().any(|p| p.contains(forbidden)),
                "{forbidden} leaked in through reconciliation"
            );
        }
    }

    #[test]
    fn bulky_subtrees_are_skipped() {
        let tmp = corpus();
        std::fs::create_dir_all(tmp.path().join("proj/node_modules/pkg")).unwrap();
        std::fs::write(tmp.path().join("proj/node_modules/pkg/index.js"), "x").unwrap();

        let store = Store::open_in_memory().unwrap();
        let report = reconcile(&store, tmp.path(), &[], 100);

        assert!(report.skipped > 0);
        assert!(!paths(&store).iter().any(|p| p.contains("node_modules")));
    }

    #[test]
    fn reconciliation_reproduces_a_from_scratch_index() {
        // Hard rule R10 in miniature: after arbitrary churn, a reconciled index and a freshly
        // built one must be indistinguishable. If they can diverge, the index is not a cache.
        let tmp = corpus();
        let incremental = Store::open_in_memory().unwrap();
        reconcile(&incremental, tmp.path(), &[], 100);

        std::fs::write(tmp.path().join("docs/three.md"), "three").unwrap();
        std::fs::remove_file(tmp.path().join("docs/two.md")).unwrap();
        std::fs::rename(
            tmp.path().join("proj/src/main.rs"),
            tmp.path().join("proj/src/lib.rs"),
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("docs/sub")).unwrap();
        std::fs::write(tmp.path().join("docs/sub/deep.md"), "deep").unwrap();

        reconcile(&incremental, tmp.path(), &[], 200);

        let from_scratch = Store::open_in_memory().unwrap();
        reconcile(&from_scratch, tmp.path(), &[], 300);

        assert_eq!(
            paths(&incremental),
            paths(&from_scratch),
            "an incrementally reconciled index diverged from a fresh one"
        );
    }

    #[test]
    fn removing_a_directory_counts_its_subtree_once() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);

        std::fs::remove_dir_all(tmp.path().join("proj")).unwrap();
        let report = reconcile(&store, tmp.path(), &[], 200);

        // proj, proj/src, proj/src/main.rs
        assert_eq!(report.removed, 3, "unexpected removal count: {report:?}");
        assert!(!paths(&store).iter().any(|p| p.contains("proj")));
    }

    #[test]
    fn reconciliation_records_what_it_repaired() {
        let tmp = corpus();
        let store = Store::open_in_memory().unwrap();
        reconcile(&store, tmp.path(), &[], 100);
        let baseline = store.stats().unwrap().episodes;

        std::fs::write(tmp.path().join("docs/late.md"), "late").unwrap();
        reconcile(&store, tmp.path(), &[], 200);

        assert!(
            store.stats().unwrap().episodes > baseline,
            "a repair should leave a trace in the episode log"
        );
    }

    #[test]
    fn a_missing_root_is_not_a_panic() {
        let store = Store::open_in_memory().unwrap();
        let report = reconcile(&store, Path::new("/definitely/not/here"), &[], 100);
        assert_eq!(report.added, 0);
    }
}
