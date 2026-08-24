//! What the memory columns cost on a ten-thousand-entry directory.
//!
//! §5 of `docs/paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md` requires this measurement *before* the
//! design is committed to, and states the threshold: **if the join costs more than 15 ms on ten
//! thousand rows, the columns belong in a deferred second message instead.**
//!
//! Both numbers are measured against the same database in the same run: the old shape — one
//! `node_by_path` per entry, which is what the listing already did to decide whether a row is
//! "new" — and the new one, a single statement. The comparison is what matters; the absolute
//! numbers are a warm page cache on whatever machine ran this, and a cold one is slower for both.

use cortex_core::model::{EdgeSource, EdgeType, NodeKind};
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::PathBuf;
use std::time::Instant;

const ROWS: usize = 10_000;

fn corpus(store: &Store) -> PathBuf {
    let root = PathBuf::from("/corpus");
    let now = 1_700_000_000;
    let parent = store
        .upsert_node(
            &NodeUpsert {
                kind: NodeKind::Dir,
                path: root.clone(),
                parent_id: None,
                mime: None,
                size: 0,
                mtime: now,
                ctime: now,
                inode: 1,
                dev: 1,
                zone_id: None,
            },
            now,
        )
        .unwrap();

    // One transaction: ten thousand of these one at a time is a test that measures SQLite's
    // commit rather than anything this file is about.
    store
        .batch(|batch| {
            for i in 0..ROWS {
                batch.upsert_node(
                    &NodeUpsert {
                        kind: NodeKind::File,
                        path: root.join(format!("file-{i:05}.md")),
                        parent_id: Some(parent),
                        mime: None,
                        size: 1024 + i as i64,
                        mtime: now - i as i64,
                        ctime: now,
                        inode: 2 + i as i64,
                        dev: 1,
                        zone_id: None,
                    },
                    now,
                )?;
            }
            Ok(())
        })
        .unwrap();

    // A realistic index is not uniform: a tenth of the files have been touched and carry
    // episodes and edges, the rest are inert. Measuring against a corpus where every row has
    // neither would measure an empty join.
    for i in (0..ROWS).step_by(10) {
        let path = root.join(format!("file-{i:05}.md"));
        let id = store.node_by_path(&path).unwrap().unwrap().id;
        store
            .append_episode(
                now - i as i64,
                &cortex_core::model::Actor::Human,
                cortex_core::model::Action::Open,
                Some(id),
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        store
            .insert_edge(id, parent, EdgeType::CoAccess, 1.0, EdgeSource::Observed, now)
            .unwrap();
    }

    root
}

#[test]
fn the_memory_columns_cost_less_than_the_per_row_queries_they_replace() {
    let store = Store::open_in_memory().unwrap();
    let root = corpus(&store);

    // What the listing does today: one statement per entry, to decide whether the index knows
    // the path. Ten thousand of them.
    let paths: Vec<PathBuf> = (0..ROWS)
        .map(|i| root.join(format!("file-{i:05}.md")))
        .collect();
    let before = Instant::now();
    let mut known = 0;
    for path in &paths {
        if store.node_by_path(path).unwrap().is_some() {
            known += 1;
        }
    }
    let per_row = before.elapsed();
    assert_eq!(known, ROWS);

    // What it does with the columns: one statement per chunk of the directory — twelve of them
    // for ten thousand rows, against ten thousand above.
    let before = Instant::now();
    let columns = cortex_store::memory_columns::for_paths(&store, &paths).unwrap();
    let joined = before.elapsed();
    assert_eq!(columns.len(), ROWS);

    println!("{ROWS} rows: {per_row:?} of per-row queries, {joined:?} for one joined statement");

    // The paper's threshold, applied to the difference it is about: the columns must not add
    // 15 ms to a listing. They are measured against the queries they replace rather than
    // against zero, because the listing was never free.
    let added = joined.saturating_sub(per_row);
    assert!(
        added < std::time::Duration::from_millis(15),
        "the join added {added:?} over the per-row queries it replaces; §5 says that at 15 ms \
         the columns belong in a deferred second message instead"
    );

    // And the numbers that make the assertion meaningful: some rows have to have edges and
    // episodes, or this measured an empty join.
    let touched = columns.values().filter(|c| c.degree > 0).count();
    assert_eq!(touched, ROWS / 10);
    assert!(columns.values().any(|c| c.last_episode.is_some()));
}
