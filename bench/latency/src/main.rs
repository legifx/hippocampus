//! Latency benchmarks.
//!
//! These are **gates, not reports**. The budgets in `docs/01-BUILD-PROMPT.md` are the
//! specification; this binary decides whether the specification is met and exits non-zero when
//! it is not. A benchmark that only prints numbers gets ignored within a fortnight.
//!
//! Everything is measured as P95 over many repetitions, because a mean hides exactly the
//! stalls a user notices.

use clap::Parser;
use cortex_core::ids::NodeId;
use cortex_core::model::QueryMode;
use cortex_extract::pipeline;
use cortex_query::fusion::{self, Arm, Ranking};
use cortex_query::{lexical, search, PathIndex};
use cortex_store::chunks::ChunkWrite;
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(name = "bench-latency", about = "Run the latency gates")]
struct Args {
    /// Corpus to measure against. Generate it with bench/corpus/generate.sh.
    #[arg(long, default_value = "bench/corpus/data/standard")]
    corpus: PathBuf,

    /// Repetitions per query.
    #[arg(long, default_value_t = 200)]
    reps: usize,

    /// Print the numbers but never fail. For exploring, not for CI.
    #[arg(long)]
    no_gate: bool,

    /// Where the content index for this corpus lives. Built once and reused, because building
    /// it is minutes of work and measuring retrieval is seconds.
    #[arg(long)]
    content_db: Option<PathBuf>,

    /// Build the content index again even if one is already there.
    #[arg(long)]
    rebuild_content: bool,

    /// Skip everything that needs a content index. For iterating on the path gates alone.
    #[arg(long)]
    skip_content: bool,
}

/// One measured path and the budget it must meet.
struct Gate {
    name: &'static str,
    budget: Duration,
    measured: Duration,
    detail: String,
}

fn main() {
    let args = Args::parse();

    if !args.corpus.is_dir() {
        eprintln!(
            "no corpus at {}. Run bench/corpus/generate.sh first.",
            args.corpus.display()
        );
        std::process::exit(2);
    }

    println!("collecting paths from {}…", args.corpus.display());
    let paths = collect_paths(&args.corpus);
    println!("  {} paths", paths.len());

    let mut gates = Vec::new();

    // ---- Index construction -------------------------------------------------------------
    // Not in the build prompt's budget table, but it is on the daemon's startup path, so it
    // is measured rather than assumed.
    let started = Instant::now();
    let index = PathIndex::build(
        paths
            .iter()
            .enumerate()
            .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
    );
    let build_time = started.elapsed();
    println!(
        "  index built in {:?}, {:.1} MiB resident",
        build_time,
        index.memory_bytes() as f64 / (1024.0 * 1024.0)
    );

    // ---- Fuzzy path match, tier one -----------------------------------------------------
    // These must actually occur in the corpus. A query that matches nothing falls through to
    // tier two, so a badly chosen list would measure the wrong tier and call it the fast one.
    let literal_queries = [
        "report", "bericht", "aurora", "src", "rechnung", "IMG_2024",
        "invoice", "zusammenfassung", "basalt", "budget",
        "aurora report", "bericht 2026",
    ];
    for q in &literal_queries {
        assert!(
            !index.search(q, 1).is_empty(),
            "benchmark query {q:?} matches nothing in the corpus — it would measure tier two"
        );
    }
    let (p50, p95, p99) = measure(&literal_queries, args.reps, |q| {
        let hits = index.search(q, 50);
        std::hint::black_box(hits.len());
    });
    gates.push(Gate {
        name: "fuzzy path match (literal, tier one)",
        budget: Duration::from_millis(10),
        measured: p95,
        detail: format!("P50 {p50:?}, P95 {p95:?}, P99 {p99:?}"),
    });

    // ---- Typing a query, character by character -----------------------------------------
    // The genuinely interactive path, and the one a naive benchmark misses entirely: the first
    // one or two keystrokes have no trigrams to narrow with, so they take the full scan every
    // time. If this is slow, the search *feels* slow no matter how good the steady state is.
    let prefixes: Vec<String> = ["report", "aurora", "bericht"]
        .iter()
        .flat_map(|word| (1..=word.len()).map(move |n| word[..n].to_string()))
        .collect();
    let prefix_refs: Vec<&str> = prefixes.iter().map(|s| s.as_str()).collect();
    let (t50, t95, t99) = measure(&prefix_refs, args.reps.min(60), |q| {
        let hits = index.search(q, 50);
        std::hint::black_box(hits.len());
    });
    gates.push(Gate {
        name: "keystroke path (every prefix, incl. 1-2 chars)",
        budget: Duration::from_millis(10),
        measured: t95,
        detail: format!("P50 {t50:?}, P95 {t95:?}, P99 {t99:?}"),
    });

    // ---- Abbreviations ------------------------------------------------------------------
    // Held to a looser budget on purpose. This tier is entered only when the literal search
    // came back empty, so it trades latency for finding something rather than nothing.
    let scattered_queries = ["prjaur", "brcht", "zsmnf", "rprtmd"];
    let (s50, s95, s99) = measure(&scattered_queries, args.reps.min(60), |q| {
        let hits = index.search(q, 50);
        std::hint::black_box(hits.len());
    });
    gates.push(Gate {
        name: "abbreviation (full-scan tier)",
        budget: Duration::from_millis(25),
        measured: s95,
        detail: format!("P50 {s50:?}, P95 {s95:?}, P99 {s99:?}"),
    });

    // ---- Content retrieval --------------------------------------------------------------
    // Everything below needs a lexical index over the corpus content, which is expensive to
    // build and cheap to keep. It is cached beside the corpus and rebuilt when either changes.
    if !args.skip_content {
        let db_path = args.content_db.clone().unwrap_or_else(|| {
            let mut name = args.corpus.clone().into_os_string();
            name.push("-content.db");
            PathBuf::from(name)
        });
        let store = open_content_index(&db_path, &args.corpus, &paths, args.rebuild_content);

        // Queries drawn from the corpus vocabulary. Each is asserted to match, because a query
        // that finds nothing measures an empty index rather than a fast one.
        let content_queries = [
            "kamerawinkel",
            "budget",
            "latenzmessungen",
            "quartalspruefung",
            "archivmigration",
            "camera angles",
            "storage costs",
            "sensor pipeline",
            "puffer volllaeuft",
            "identischen index",
        ];
        for q in &content_queries {
            let expression = lexical::match_expression(q, false).expect("query has no terms");
            let hits = store.search_chunks(&expression, 1).expect("lexical query failed");
            assert!(!hits.is_empty(), "benchmark query {q:?} matches no content in the corpus");
        }

        // ---- BM25 top-100 ---------------------------------------------------------------
        let (b50, b95, b99) = measure(&content_queries, args.reps.min(60), |q| {
            let expression = lexical::match_expression(q, false).unwrap();
            let hits = store.search_chunks(&expression, 100).unwrap();
            std::hint::black_box(hits.len());
        });
        gates.push(Gate {
            name: "BM25 top-100",
            budget: Duration::from_millis(30),
            measured: b95,
            detail: format!("P50 {b50:?}, P95 {b95:?}, P99 {b99:?}"),
        });

        // ---- The case the budget does not cover -------------------------------------------
        // A query for a word that is in a third of the corpus. FTS5 scores every match before
        // it can rank them, so this is linear in matches and no top-k trick avoids it. It is
        // reported rather than gated because nobody searches for "und" — but it is reported
        // because the moment somebody does, this is the number they will feel, and a corpus
        // realistic enough to pass the gate must not be allowed to hide it.
        let stopwords = ["und", "the", "die"];
        let matched: usize = stopwords
            .iter()
            .map(|w| {
                let expression = lexical::match_expression(w, false).unwrap();
                store
                    .search_chunks(&expression, 1_000_000)
                    .map(|h| h.len())
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        let (w50, w95, _) = measure(&stopwords, args.reps.min(20), |q| {
            let expression = lexical::match_expression(q, false).unwrap();
            let hits = store.search_chunks(&expression, 100).unwrap();
            std::hint::black_box(hits.len());
        });
        println!();
        println!(
            "worst case, a stopword query matching up to {matched} chunks: P50 {w50:?}, P95 {w95:?}"
        );

        // ---- RRF fusion + salience boost -------------------------------------------------
        // Measured on full candidate lists from both arms, which is the worst case the daemon
        // ever hands it.
        let arm_a: Vec<NodeId> = (1..=100).map(NodeId).collect();
        let arm_b: Vec<NodeId> = (50..=150).map(NodeId).collect();
        let (f50, f95, f99) = measure(&["fusion"], args.reps, |_| {
            let fused = fusion::fuse(
                &[
                    Ranking { arm: Arm::Path, nodes: arm_a.clone() },
                    Ranking { arm: Arm::Lexical, nodes: arm_b.clone() },
                ],
                60.0,
            );
            let boosted: f64 = fused
                .iter()
                .map(|f| f.fused * fusion::salience_boost(0.5, 1.5))
                .sum();
            std::hint::black_box(boosted);
        });
        gates.push(Gate {
            name: "RRF fusion + salience boost",
            budget: Duration::from_millis(5),
            measured: f95,
            detail: format!("P50 {f50:?}, P95 {f95:?}, P99 {f99:?}"),
        });

        // ---- The whole hybrid query ------------------------------------------------------
        // Both arms, fusion, and a snippet re-read from disk for every displayed row. Not a
        // gate until M4 puts a keystroke budget on it — reported now so the number is known
        // before the GUI depends on it.
        let (h50, h95, h99) = measure(&content_queries, args.reps.min(40), |q| {
            let outcome = search::run(search::Params {
                store: &store,
                index: &index,
                requested_mode: QueryMode::Auto,
                raw_query: q,
                limit: 50,
                prefix: true,
                candidates_per_arm: 100,
                rrf_k: 60.0,
                max_salience_boost: 1.5,
            });
            std::hint::black_box(outcome.hits.len());
        });
        println!();
        println!("hybrid search, both arms and snippets: P50 {h50:?}, P95 {h95:?}, P99 {h99:?}");
        println!("  (the M4 keystroke budget is 80 ms; not gated until then)");
    }

    // ---- Memory -------------------------------------------------------------------------
    let index_mib = index.memory_bytes() as f64 / (1024.0 * 1024.0);
    println!();
    println!("path index: {index_mib:.1} MiB for {} paths", index.len());

    // ---- Report -------------------------------------------------------------------------
    println!();
    println!("{:<48} {:>10} {:>10}  {}", "gate", "measured", "budget", "");
    let mut failed = 0;
    for gate in &gates {
        let ok = gate.measured <= gate.budget;
        if !ok {
            failed += 1;
        }
        println!(
            "{:<48} {:>10} {:>10}  {}  {}",
            gate.name,
            format!("{:.2?}", gate.measured),
            format!("{:.2?}", gate.budget),
            if ok { "PASS" } else { "FAIL" },
            gate.detail
        );
    }

    if failed > 0 && !args.no_gate {
        println!("\n{failed} gate(s) failed.");
        std::process::exit(1);
    }
    println!("\nall gates passed.");
}

/// Run `f` over every query `reps` times and return P50, P95 and P99 of the individual runs.
fn measure(queries: &[&str], reps: usize, mut f: impl FnMut(&str)) -> (Duration, Duration, Duration) {
    // Warm up: the first run of anything pays for cold caches and lazily-touched pages, and
    // including that in a percentile measures the wrong thing.
    for q in queries {
        f(q);
    }

    let mut samples = Vec::with_capacity(queries.len() * reps);
    for _ in 0..reps {
        for q in queries {
            let started = Instant::now();
            f(q);
            samples.push(started.elapsed());
        }
    }
    samples.sort_unstable();

    let at = |p: f64| samples[((samples.len() as f64 * p) as usize).min(samples.len() - 1)];
    (at(0.50), at(0.95), at(0.99))
}

/// Open the cached content index, building it first if it is missing or stale.
///
/// Building runs the real pipeline — classify, extract, chunk, write — so the index under
/// measurement is the one the daemon would produce, not an approximation of it.
fn open_content_index(db: &Path, corpus: &Path, paths: &[String], rebuild: bool) -> Store {
    if rebuild {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = db.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(p));
        }
    }

    let store = Store::open(db).expect("could not open the content index");
    let existing = store.extract_progress().expect("could not read progress");
    if existing.chunks > 0 && !rebuild {
        println!(
            "content index: {} chunks over {} files (cached at {})",
            existing.chunks,
            existing.done,
            db.display()
        );
        return store;
    }

    println!("building the content index — this happens once per corpus…");
    let settings = pipeline::Settings {
        content_roots: vec![corpus.to_path_buf()],
        ..pipeline::Settings::default()
    };

    let started = Instant::now();
    let mut extracted = 0usize;
    let mut chunks = 0usize;

    // Written in batches for the same reason the daemon does it: SQLite's commit barrier, not
    // the work, dominates otherwise.
    for batch in paths.chunks(512) {
        store
            .batch(|tx| {
                for path in batch {
                    let path = PathBuf::from(path);
                    let Ok(metadata) = std::fs::metadata(&path) else {
                        continue;
                    };
                    let id = tx.upsert_node(
                        &NodeUpsert {
                            kind: cortex_core::model::NodeKind::File,
                            path: path.clone(),
                            parent_id: None,
                            mime: None,
                            size: metadata.len() as i64,
                            mtime: 0,
                            ctime: 0,
                            inode: 0,
                            dev: 0,
                            zone_id: None,
                        },
                        0,
                    )?;

                    let outcome = pipeline::process(&path, metadata.len(), None, &settings);
                    if let pipeline::Outcome::Extracted { chunks: produced, .. } = outcome {
                        let writes: Vec<ChunkWrite> = produced
                            .into_iter()
                            .map(|c| ChunkWrite {
                                ordinal: c.ordinal as i64,
                                byte_start: c.byte_start as i64,
                                byte_end: c.byte_end as i64,
                                preview: Some(c.preview(settings.preview_chars)),
                                hash: c.hash,
                                text: c.text,
                            })
                            .collect();
                        chunks += writes.len();
                        extracted += 1;
                        tx.replace_chunks(id, &writes)?;
                        tx.set_extraction_result(
                            id,
                            None,
                            None,
                            cortex_core::model::ExtractState::Done,
                            None,
                        )?;
                    }
                }
                Ok(())
            })
            .expect("could not write a batch of chunks");
    }

    println!(
        "  {} files extracted into {} chunks in {:.1?}",
        extracted,
        chunks,
        started.elapsed()
    );
    store
}

/// Walk the corpus, applying the same deny-list the daemon would.
///
/// Using the real classifier matters: benchmarking against paths the daemon would never index
/// would measure a corpus that does not exist in production.
fn collect_paths(root: &PathBuf) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![root.clone()];

    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

            if is_dir {
                if !cortex_core::denylist::should_skip_subtree(&name) {
                    queue.push(path);
                }
                continue;
            }
            if cortex_core::denylist::classify(&path, &[]).may_index() {
                if let Some(s) = path.to_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}
