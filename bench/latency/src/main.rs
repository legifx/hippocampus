//! Latency benchmarks.
//!
//! These are **gates, not reports**. The budgets in `docs/01-BUILD-PROMPT.md` are the
//! specification; this binary decides whether the specification is met and exits non-zero when
//! it is not. A benchmark that only prints numbers gets ignored within a fortnight.
//!
//! Everything is measured as P95 over many repetitions, because a mean hides exactly the
//! stalls a user notices.

use clap::Parser;
use cortex_core::ids::{ChunkId, NodeId};
use cortex_core::model::QueryMode;
use cortex_embed::{Embedder, Input};
use cortex_extract::pipeline;
use cortex_query::fusion::{self, Arm, Ranking};
use cortex_query::{lexical, search, PathIndex};
use cortex_store::chunks::ChunkWrite;
use cortex_store::queries::NodeUpsert;
use cortex_store::vectors::VectorIndex;
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

        // ---- Searching while the daemon writes ------------------------------------------
        //
        // The gate every other one here was missing. Each measurement above is taken against a
        // store that nothing else is touching, and the daemon is *never* in that state: it
        // indexes, extracts and embeds in the background, and the search a person is waiting
        // for happens at the same time as all of it.
        //
        // Which is exactly what was going wrong. The journal from a week of real use shows
        // searches at 932 ms against an 80 ms budget, and `extract.status` — a status call, a
        // heartbeat, nothing — at 988 ms. Neither is slow on its own; they were waiting.
        //
        // **What is compared is the gap, not the number.** The absolute figure drifts with the
        // corpus — this gate writes to the same database it reads from, because contending is
        // the entire point — so its budget is the quiet measurement above plus a margin. A
        // reader that waits for a writer shows up as a multiple, not as a few milliseconds, and
        // a budget derived from the same run cannot be confounded by a corpus that grew.
        {
            use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

            let stop = AtomicBool::new(false);
            let batches = AtomicU64::new(0);


            std::thread::scope(|scope| {
                // A writer doing what the indexer does: batches of upserts, in a transaction,
                // one after another. Not a synthetic hammer — this is the shape of the work
                // that is running whenever anything on the machine changes.
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        // A duty cycle, not a hammer. The indexer spends most of its time
                        // reading files, extracting text and embedding it; the database is the
                        // punctuation. A writer that never pauses starves the reader completely
                        // — which the first version of this gate demonstrated by not finishing
                        // in half an hour — and measures a situation the daemon is never in.
                        std::thread::sleep(Duration::from_millis(10));

                        // The same rows, with the values they already hold. The write work is
                        // real — a transaction, a commit, the same index churn the indexer
                        // causes — and the corpus is in the state it started in when this
                        // finishes. The first version wrote *changed* rows, which left the
                        // benchmark database different after every run and made two runs of the
                        // same code disagree by 50%. A benchmark that alters what it measures
                        // is worse than no benchmark, because it produces numbers.
                        let _ = store.batch(|tx| {
                            for path in paths.iter().take(256) {
                                let path = PathBuf::from(path);
                                let Ok(metadata) = std::fs::metadata(&path) else {
                                    continue;
                                };
                                tx.upsert_node(
                                    &NodeUpsert {
                                        kind: cortex_core::model::NodeKind::File,
                                        path,
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
                            }
                            Ok(())
                        });
                        batches.fetch_add(1, Ordering::Relaxed);
                    }
                });

                let (w50, w95, w99) = measure(&content_queries, args.reps.min(60), |q| {
                    let expression = lexical::match_expression(q, false).unwrap();
                    let hits = store.search_chunks(&expression, 100).unwrap();
                    std::hint::black_box(hits.len());
                });
                stop.store(true, Ordering::Relaxed);

                gates.push(Gate {
                    name: "BM25 top-100, while the indexer writes",
                    // Half again as long as the same query with nothing else running. A reader
                    // that waits for a writer is not 50% slower, it is several times slower —
                    // measured before the connection pool, the gap was +70% with a writer at a
                    // 10% duty cycle, and in production it was searches of 932 ms against a
                    // budget of 80.
                    budget: b95 + b95 / 2,
                    measured: w95,
                    detail: format!(
                        "P50 {w50:?}, P95 {w95:?}, P99 {w99:?}, {} write batches alongside",
                        batches.load(Ordering::Relaxed)
                    ),
                });
            });
        }

        // ---- Vector ANN top-100 -----------------------------------------------------------
        // Built from pseudo-random unit vectors rather than from the model. Approximate-nearest-
        // neighbour latency is a function of the graph — its size, its dimensionality and its
        // connectivity — and not of what the vectors mean, so this measures the same thing the
        // real index would while taking seconds instead of hours of inference. What it does not
        // measure is recall, which is the goldset's job.
        let vector_count = store.extract_progress().map(|p| p.chunks as usize).unwrap_or(0).max(1000);
        println!();
        println!("building a {vector_count}-vector index for the ANN gate…");
        let built = Instant::now();
        let vectors = VectorIndex::open(&db_path.with_extension("bench.usearch"), VECTOR_DIM).unwrap();
        vectors.clear().unwrap();
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let sample: Vec<Vec<f32>> = (0..vector_count)
            .map(|i| {
                let v = pseudo_unit_vector(&mut seed);
                vectors.add(ChunkId(i as i64 + 1), &v).unwrap();
                v
            })
            .collect();
        println!(
            "  {} vectors, {:.1?}, {:.1} MiB resident",
            vectors.len(),
            built.elapsed(),
            vectors.memory_bytes() as f64 / (1024.0 * 1024.0)
        );

        let probes: Vec<usize> = (0..20).map(|i| i * (vector_count / 20).max(1) % vector_count).collect();
        let labels: Vec<String> = probes.iter().map(|i| i.to_string()).collect();
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let (v50, v95, v99) = measure(&label_refs, args.reps.min(60), |label| {
            let index: usize = label.parse().unwrap();
            let hits = vectors.search(&sample[index], 100).unwrap();
            std::hint::black_box(hits.len());
        });
        gates.push(Gate {
            name: "vector ANN top-100",
            budget: Duration::from_millis(40),
            measured: v95,
            detail: format!("P50 {v50:?}, P95 {v95:?}, P99 {v99:?}"),
        });

        // ---- Embedding the query ----------------------------------------------------------
        // Only with a real model on the machine, because that is the only number worth having:
        // the hashing embedder is microseconds and would flatter the hot path into meaning
        // nothing. Reported rather than gated — the budget table gates the ANN search, and this
        // is the cost of getting to it.
        if let Some(embedder) = real_embedder() {
            let queries = ["kamerawinkel", "was war der stand beim budget", "aurora report"];
            let (q50, q95, _) = measure(&queries, args.reps.min(20), |q| {
                let v = embedder.embed(&[Input::Query(q)]).unwrap();
                std::hint::black_box(v.len());
            });
            println!();
            println!("embedding one query with {}: P50 {q50:?}, P95 {q95:?}", embedder.backend());
        } else {
            println!();
            println!("query embedding not measured: set HIPPOCAMPUS_TEST_RUNTIME and HIPPOCAMPUS_TEST_MODEL");
        }

        // ---- Cross-encoder rerank of the top 30 -------------------------------------------
        // The one gate that only exists when the model does. Measured over thirty realistic
        // pairs, which is what `search.rerank_top_n` defaults to.
        if let Some(reranker) = real_reranker() {
            let passages: Vec<String> = (0..30)
                .map(|i| {
                    format!(
                        "Abschnitt {i}. {}",
                        "Ein Absatz von der Laenge eines echten Chunks, der ueber mehrere Saetze \
                         geht und daher realistisch tokenisiert wird. "
                            .repeat(6)
                    )
                })
                .collect();
            let refs: Vec<&str> = passages.iter().map(|p| p.as_str()).collect();
            let queries = ["warum sind die kosten gestiegen", "camera mounting comparison"];
            let (r50, r95, r99) = measure(&queries, args.reps.min(20), |q| {
                let scores = reranker.score(q, &refs).unwrap();
                std::hint::black_box(scores.len());
            });
            gates.push(Gate {
                name: "cross-encoder rerank, top 30",
                budget: Duration::from_millis(150),
                measured: r95,
                detail: format!("P50 {r50:?}, P95 {r95:?}, P99 {r99:?}, {}", reranker.backend()),
            });
        } else {
            println!("reranking not measured: set HIPPOCAMPUS_TEST_RERANKER");
        }

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
                // The lexical and path arms only. The vector arm is measured on its own below,
                // because it needs an index that a benchmark can build in seconds rather than
                // the hours of inference the real corpus would take.
                semantic: None,
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

/// Dimensions of the vectors used for the ANN gate. The real model's width (D6), because the
/// graph's cost depends on it.
const VECTOR_DIM: usize = 384;

/// A deterministic unit vector. xorshift, so the benchmark input never drifts.
fn pseudo_unit_vector(seed: &mut u64) -> Vec<f32> {
    let mut v = Vec::with_capacity(VECTOR_DIM);
    for _ in 0..VECTOR_DIM {
        let mut x = *seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *seed = x;
        v.push((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 8_388_608.0 - 1.0);
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for value in v.iter_mut() {
        *value /= norm;
    }
    v
}

/// The real embedder, when the machine has one. Optional on purpose: the latency gates must run
/// on a machine that has never downloaded half a gigabyte.
fn real_embedder() -> Option<Box<dyn Embedder>> {
    let runtime = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_RUNTIME").ok()?);
    let model = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_MODEL").ok()?);
    cortex_embed::onnx::OnnxEmbedder::load(
        &runtime,
        &model,
        cortex_embed::E5_SMALL,
        cortex_embed::onnx::Device::Cpu,
        8,
        1024,
    )
    .ok()
    .map(|e| Box::new(e) as Box<dyn Embedder>)
}

/// The cross-encoder, when the machine has one.
fn real_reranker() -> Option<Box<dyn cortex_embed::Reranker>> {
    let runtime = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_RUNTIME").ok()?);
    let model = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_RERANKER").ok()?);
    cortex_embed::onnx::OnnxReranker::load(
        &runtime,
        &model,
        cortex_embed::RERANKER,
        cortex_embed::onnx::Device::Auto,
        8,
        1024,
    )
    .ok()
    .map(|r| Box::new(r) as Box<dyn cortex_embed::Reranker>)
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
