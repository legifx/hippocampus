//! Recall, measured rather than assumed.
//!
//! Latency benchmarks say the search is fast. This says whether it finds the right thing, and it
//! is the only instrument in the project that can. Without it, every ranking decision — the
//! fusion constant, the arm weights, the mode heuristic, whether the model is worth its half
//! gigabyte — is somebody's taste.
//!
//! ## What it does
//!
//! Plants the judged documents into a corpus of generated distractors, indexes them exactly the
//! way the daemon would (classify, extract, chunk, embed), runs the fifty queries through the
//! real search, and reports recall@10 broken down by what each query is testing.
//!
//! ## What the breakdown is for
//!
//! One number tells you something regressed. The breakdown tells you what: a drop confined to
//! `semantic` queries is the model or the vector arm, a drop in `name` queries is the path index,
//! and a gap between the German and the English halves is decision D6 asking to be revisited
//! (M2 Definition of Done §3).
//!
//! Each arm is also measured *alone*, which is how "the model earns its keep" stops being a
//! belief: if fused recall is not meaningfully above lexical-only recall, the vector arm is
//! costing hours of inference for nothing.

use clap::Parser;
use cortex_core::ids::NodeId;
use cortex_core::model::QueryMode;
use cortex_embed::{Embedder, Input};
use cortex_extract::pipeline;
use cortex_query::{search, PathIndex};
use cortex_store::chunks::ChunkWrite;
use cortex_store::embeddings::{EmbeddingWrite, ModelRef};
use cortex_store::queries::NodeUpsert;
use cortex_store::vectors::VectorIndex;
use cortex_store::Store;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "goldset", about = "Measure recall@10 against the judged queries")]
struct Args {
    /// Corpus of distractors. Generate it with `bench/corpus/generate.sh small`.
    #[arg(long, default_value = "bench/corpus/data/small")]
    corpus: PathBuf,

    /// Where the goldset documents are planted. Written from `documents.toml`, so deleting it
    /// is always safe.
    #[arg(long, default_value = "bench/corpus/data/goldset")]
    planted: PathBuf,

    /// Index cache. Built once per corpus and model.
    #[arg(long)]
    db: Option<PathBuf>,

    /// Path to `libonnxruntime.so`. Without it the run uses the hashing embedder, which is a
    /// useful control and not a substitute.
    #[arg(long, env = "HIPPOCAMPUS_TEST_RUNTIME")]
    runtime: Option<PathBuf>,

    /// Directory holding `model.onnx` and `tokenizer.json`.
    #[arg(long, env = "HIPPOCAMPUS_TEST_MODEL")]
    model: Option<PathBuf>,

    /// Results considered. The Definition of Done says ten.
    #[arg(long, default_value_t = 10)]
    at: usize,

    /// Rebuild the index even if the cache is there.
    #[arg(long)]
    rebuild: bool,

    /// Fail if recall drops more than this many points below `baseline.toml`.
    #[arg(long, default_value_t = 3.0)]
    tolerance: f64,

    /// Write the measured numbers to `baseline.toml`. Only for a deliberate, explained change.
    #[arg(long)]
    write_baseline: bool,

    /// Print the ranked results for one query id, with the arm each result came from.
    ///
    /// A recall number says something is missing; this says what was found instead, which is the
    /// only way to tell a ranking problem from a retrieval problem.
    #[arg(long, value_name = "QUERY_ID")]
    explain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Documents {
    document: Vec<Document>,
}

#[derive(Debug, Deserialize, Clone)]
struct Document {
    id: String,
    /// The language the document is written in. Not used for scoring — it is what makes a
    /// cross-language query recognisable as one when reading the file.
    #[allow(dead_code)]
    lang: String,
    path: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct Queries {
    query: Vec<Query>,
}

#[derive(Debug, Deserialize, Clone)]
struct Query {
    id: String,
    lang: String,
    kind: String,
    text: String,
    relevant: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Baseline {
    #[serde(default)]
    recall: BTreeMap<String, f64>,
}

fn main() {
    let mut args = Args::parse();
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));

    let documents: Documents = read_toml(&here.join("documents.toml"));
    let queries: Queries = read_toml(&here.join("queries.toml"));
    validate(&documents, &queries);

    if !args.corpus.is_dir() {
        eprintln!(
            "no corpus at {}. Run bench/corpus/generate.sh small first.",
            args.corpus.display()
        );
        std::process::exit(2);
    }

    plant(&documents.document, &args.planted);

    // Absolute from here on. The index stores whatever path it was given, and a lookup by
    // canonical path against relative entries finds nothing at all — which looks exactly like an
    // empty corpus and is nothing of the sort.
    args.planted = args
        .planted
        .canonicalize()
        .expect("the planted directory was just written");
    args.corpus = args.corpus.canonicalize().expect("the corpus exists");

    // The embedder decides the cache name: vectors from two models must never share an index.
    let embedder = load_embedder(&args);
    let model = embedder.model();
    println!("embedder: {} ({})", model.id, embedder.backend());

    let db = args.db.clone().unwrap_or_else(|| {
        PathBuf::from(format!(
            "bench/corpus/data/goldset-{}-{}.db",
            model.id, model.version
        ))
    });
    let (store, vectors) = build_index(&db, &args, embedder.as_ref(), &model);

    let paths = store.all_paths_with_ids().expect("could not read paths");
    let index = PathIndex::build(paths.iter().map(|(id, p)| (*id, p.as_str())));

    // Which node is which judged document. The planted path is the identity, because two
    // documents can legitimately contain similar text.
    let node_of: BTreeMap<String, NodeId> = documents
        .document
        .iter()
        .filter_map(|doc| {
            let full = args.planted.join(&doc.path);
            let canonical = full.canonicalize().ok()?;
            store
                .node_by_path(&canonical)
                .ok()
                .flatten()
                .map(|node| (doc.id.clone(), node.id))
        })
        .collect();
    if node_of.len() != documents.document.len() {
        eprintln!(
            "only {} of {} judged documents were indexed — the corpus is incomplete",
            node_of.len(),
            documents.document.len()
        );
        std::process::exit(2);
    }

    println!(
        "corpus: {} indexed paths, {} chunks, {} vectors",
        index.len(),
        store.extract_progress().unwrap().chunks,
        vectors.len()
    );
    println!();

    if let Some(id) = &args.explain {
        let query = queries
            .query
            .iter()
            .find(|q| &q.id == id)
            .unwrap_or_else(|| panic!("no query {id}"));
        explain_query(query, &node_of, &store, &index, embedder.as_ref(), &vectors, args.at);
        return;
    }

    let arms: [(&str, Option<&VectorIndex>, bool); 3] = [
        ("fused", Some(&vectors), true),
        ("lexical only", None, true),
        ("vector only", Some(&vectors), false),
    ];

    let mut measured = BTreeMap::new();
    for (name, vector_index, lexical) in arms {
        let report = run_arm(
            &queries.query,
            &node_of,
            &store,
            &index,
            embedder.as_ref(),
            vector_index,
            lexical,
            args.at,
        );
        print_report(name, &report, &queries.query);
        for (key, value) in report.by_kind {
            measured.insert(format!("{name}/{key}"), value);
        }
        measured.insert(format!("{name}/overall"), report.overall);
        measured.insert(format!("{name}/de"), report.german);
        measured.insert(format!("{name}/en"), report.english);
        println!();
    }

    let fused = measured["fused/overall"];
    let lexical_only = measured["lexical only/overall"];
    println!("what the model buys: {:+.1} points over lexical alone", (fused - lexical_only) * 100.0);

    let german = measured["fused/de"];
    let english = measured["fused/en"];
    // Directional on purpose. The Definition of Done asks whether German is *worse* than
    // English — a multilingual model that happens to do better on German is not a reason to
    // revisit the model that was chosen for German.
    let shortfall = ((english - german) / english.max(f64::EPSILON)).max(0.0);
    println!(
        "German {:.1} % against English {:.1} % — German is {:.0} % {}",
        german * 100.0,
        english * 100.0,
        (shortfall * 100.0).max(((german - english) / english.max(f64::EPSILON) * 100.0).max(0.0)),
        if german < english { "behind" } else { "ahead" }
    );
    if shortfall > 0.10 {
        // M2 Definition of Done §3. Not a hard failure here, because the goldset is small enough
        // that a couple of queries move it — but it is the trigger to revisit D6, and saying so
        // is the whole point of measuring it.
        println!("  ^ more than the 10 % the build prompt allows; decision D6 needs revisiting");
    }

    let baseline_path = here.join("baseline.toml");
    if args.write_baseline {
        write_baseline(&baseline_path, &measured);
        println!("\nbaseline written to {}", baseline_path.display());
        return;
    }

    match std::fs::read_to_string(&baseline_path) {
        Ok(text) => {
            let baseline: Baseline = toml::from_str(&text).expect("baseline.toml is not valid");
            let mut regressions = Vec::new();
            for (key, previous) in &baseline.recall {
                let Some(now) = measured.get(key) else { continue };
                let drop = (previous - now) * 100.0;
                if drop > args.tolerance {
                    regressions.push(format!("{key}: {:.1} % → {:.1} % ({drop:.1} points)", previous * 100.0, now * 100.0));
                }
            }
            if regressions.is_empty() {
                println!("\nno regression against the baseline.");
            } else {
                println!("\nrecall regressed:");
                for line in &regressions {
                    println!("  {line}");
                }
                std::process::exit(1);
            }
        }
        Err(_) => println!("\nno baseline yet — run with --write-baseline to record one."),
    }
}

/// Show what one query actually returned.
fn explain_query(
    query: &Query,
    node_of: &BTreeMap<String, NodeId>,
    store: &Store,
    index: &PathIndex,
    embedder: &dyn Embedder,
    vectors: &VectorIndex,
    at: usize,
) {
    let outcome = search::run(search::Params {
        store,
        index,
        semantic: Some(search::Semantic { embedder, vectors }),
        requested_mode: QueryMode::Auto,
        raw_query: &query.text,
        limit: at.max(20),
        prefix: false,
        candidates_per_arm: 100,
        rrf_k: 60.0,
        max_salience_boost: 1.5,
    });

    let wanted: BTreeMap<NodeId, &str> = query
        .relevant
        .iter()
        .map(|id| (node_of[id], id.as_str()))
        .collect();

    println!("{} [{}] {:?}", query.id, query.kind, query.text);
    println!("  mode {:?}, {} path / {} lexical / {} vector candidates",
        outcome.mode, outcome.path_candidates, outcome.lexical_candidates, outcome.vector_candidates);
    if let Some(note) = &outcome.vector_note {
        println!("  vector arm: {note}");
    }
    println!();
    for (rank, hit) in outcome.hits.iter().enumerate() {
        let marker = match wanted.get(&hit.node_id) {
            Some(id) => format!(" <-- relevant: {id}"),
            None => String::new(),
        };
        println!(
            "  {:>2}. p{:<4} l{:<4} v{:<4} {:.5}  {}{}",
            rank + 1,
            hit.score.path_rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
            hit.score.lexical_rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
            hit.score.vector_rank.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
            hit.score.final_score,
            hit.path.file_name().unwrap_or_default().to_string_lossy(),
            marker
        );
    }

    let missing: Vec<&str> = wanted
        .iter()
        .filter(|(node, _)| !outcome.hits.iter().any(|h| &h.node_id == *node))
        .map(|(_, id)| *id)
        .collect();
    if missing.is_empty() {
        println!("\n  everything relevant is in the list");
    } else {
        println!("\n  not returned at all: {}", missing.join(", "));
    }
}

struct Report {
    overall: f64,
    german: f64,
    english: f64,
    by_kind: BTreeMap<String, f64>,
    misses: Vec<(String, String)>,
}

#[allow(clippy::too_many_arguments)]
fn run_arm(
    queries: &[Query],
    node_of: &BTreeMap<String, NodeId>,
    store: &Store,
    index: &PathIndex,
    embedder: &dyn Embedder,
    vectors: Option<&VectorIndex>,
    lexical: bool,
    at: usize,
) -> Report {
    let mut per_query = Vec::new();
    let mut misses = Vec::new();

    for query in queries {
        let outcome = search::run(search::Params {
            store,
            index,
            semantic: vectors.map(|v| search::Semantic { embedder, vectors: v }),
            requested_mode: QueryMode::Auto,
            raw_query: &query.text,
            limit: at,
            // Exact matching: a goldset measures retrieval, not what happens halfway through
            // typing a word.
            prefix: false,
            candidates_per_arm: 100,
            rrf_k: 60.0,
            max_salience_boost: 1.5,
        });

        let found: Vec<NodeId> = outcome.hits.iter().map(|h| h.node_id).collect();
        let wanted: Vec<NodeId> = query
            .relevant
            .iter()
            .map(|id| node_of[id])
            .collect();
        // Suppressing one arm is done by ignoring its ranks rather than by not running it, so
        // that every arm sees the identical candidate set and the comparison is fair.
        let hit_count = wanted
            .iter()
            .filter(|node| {
                found.iter().enumerate().any(|(rank, candidate)| {
                    candidate == *node && arm_allowed(&outcome, rank, lexical, vectors.is_some())
                })
            })
            .count();

        let recall = hit_count as f64 / wanted.len() as f64;
        if recall < 1.0 {
            misses.push((query.id.clone(), query.text.clone()));
        }
        per_query.push((query, recall));
    }

    let mean = |rows: &[(&Query, f64)]| -> f64 {
        if rows.is_empty() {
            return 0.0;
        }
        rows.iter().map(|(_, r)| r).sum::<f64>() / rows.len() as f64
    };

    let mut by_kind: BTreeMap<String, f64> = BTreeMap::new();
    for kind in ["name", "lexical", "semantic", "crosslang"] {
        let rows: Vec<(&Query, f64)> = per_query
            .iter()
            .filter(|(q, _)| q.kind == kind)
            .copied()
            .collect();
        if !rows.is_empty() {
            by_kind.insert(kind.to_string(), mean(&rows));
        }
    }

    let german: Vec<(&Query, f64)> = per_query.iter().filter(|(q, _)| q.lang == "de").copied().collect();
    let english: Vec<(&Query, f64)> = per_query.iter().filter(|(q, _)| q.lang == "en").copied().collect();

    Report {
        overall: mean(&per_query),
        german: mean(&german),
        english: mean(&english),
        by_kind,
        misses,
    }
}

/// Whether a hit at this rank should count for the arm being measured.
///
/// "Lexical only" and "vector only" are measured by ignoring the other arm's contribution rather
/// than by re-running the search without it, because fusion changes the ranks: a comparison
/// between two differently-ranked lists would measure the fusion, not the arm.
fn arm_allowed(outcome: &search::Outcome, rank: usize, lexical: bool, vector: bool) -> bool {
    let score = &outcome.hits[rank].score;
    let by_path = score.path_rank.is_some();
    let by_lexical = score.lexical_rank.is_some();
    let by_vector = score.vector_rank.is_some();

    // The path arm is always allowed: it is not one of the two content arms being compared, and
    // suppressing it would make "lexical only" mean something nobody ships.
    by_path || (lexical && by_lexical) || (vector && by_vector)
}

fn print_report(name: &str, report: &Report, queries: &[Query]) {
    println!(
        "{name:<14} recall@10  overall {:.1} %   de {:.1} %   en {:.1} %",
        report.overall * 100.0,
        report.german * 100.0,
        report.english * 100.0
    );
    let kinds: Vec<String> = report
        .by_kind
        .iter()
        .map(|(kind, value)| format!("{kind} {:.0} %", value * 100.0))
        .collect();
    println!("{:<14} {}", "", kinds.join("   "));

    if !report.misses.is_empty() {
        let shown: Vec<String> = report
            .misses
            .iter()
            .take(6)
            .map(|(id, text)| {
                let kind = queries
                    .iter()
                    .find(|q| &q.id == id)
                    .map(|q| q.kind.as_str())
                    .unwrap_or("");
                format!("{id} [{kind}] {text:?}")
            })
            .collect();
        println!("{:<14} missed: {}", "", shown.join("; "));
        if report.misses.len() > 6 {
            println!("{:<14}         … and {} more", "", report.misses.len() - 6);
        }
    }
}

/// Write the judged documents into the corpus.
fn plant(documents: &[Document], root: &Path) {
    for doc in documents {
        let path = root.join(&doc.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("could not create the goldset directory");
        }
        // Written every run: the file on disk is derived from the checked-in text, so editing
        // the corpus by hand can never silently change what is being measured.
        std::fs::write(&path, &doc.text).expect("could not plant a goldset document");
    }
}

fn load_embedder(args: &Args) -> Box<dyn Embedder> {
    match (&args.runtime, &args.model) {
        (Some(runtime), Some(model)) => {
            match cortex_embed::onnx::OnnxEmbedder::load(
                runtime,
                model,
                cortex_embed::E5_SMALL,
                cortex_embed::onnx::Device::Auto,
                8,
            ) {
                Ok(embedder) => Box::new(embedder),
                Err(e) => {
                    eprintln!("could not load the model: {e}");
                    std::process::exit(2);
                }
            }
        }
        _ => {
            println!(
                "no model given — measuring with the hashing embedder, which is a control and \
                 not a result. Pass --runtime and --model for the real number."
            );
            Box::new(cortex_embed::hashing::HashingEmbedder::new())
        }
    }
}

/// Index both corpora exactly as the daemon would, and cache the result.
fn build_index(
    db: &Path,
    args: &Args,
    embedder: &dyn Embedder,
    model: &cortex_embed::ModelId,
) -> (Store, VectorIndex) {
    let vector_path = db.with_extension("usearch");
    if args.rebuild {
        for path in [db.to_path_buf(), vector_path.clone()] {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
        }
    }

    let store = Store::open(db).expect("could not open the goldset index");
    let vectors = VectorIndex::open(&vector_path, model.dim).expect("could not open the vectors");
    let reference = ModelRef {
        id: &model.id,
        version: &model.version,
        dim: model.dim,
    };

    // A cache is only usable if the run that wrote it finished. This is not caution for its own
    // sake: a run that crashed part-way through embedding left a database and an index that
    // *looked* complete — the counts matched — and produced 34 % recall where the same corpus
    // measures 91 %. A measuring instrument that can quietly report the wrong number is worse
    // than no instrument, so completeness is recorded explicitly rather than inferred.
    let complete = store
        .meta_get(CACHE_MARKER)
        .ok()
        .flatten()
        .is_some_and(|value| value == cache_generation(model));
    if complete && !args.rebuild {
        println!("index: cached at {}", db.display());
        return (store, vectors);
    }
    if !args.rebuild {
        // Whatever is there was written by a run that did not finish, or by a different model or
        // schema. Start again rather than build on top of it.
        store.clear_derived(false).expect("could not clear a stale cache");
        vectors.clear().expect("could not clear a stale vector index");
    }
    let _ = store.meta_set(CACHE_MARKER, "");

    // Extract.
    let started = Instant::now();
    let roots = [args.planted.clone(), args.corpus.clone()];
    let settings = pipeline::Settings {
        content_roots: roots.to_vec(),
        ..pipeline::Settings::default()
    };
    let mut files = Vec::new();
    for root in &roots {
        collect(root, &mut files);
    }
    println!("indexing {} files…", files.len());

    for batch in files.chunks(512) {
        store
            .batch(|tx| {
                for path in batch {
                    let Ok(metadata) = std::fs::metadata(path) else { continue };
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
                    if let pipeline::Outcome::Extracted { chunks, .. } =
                        pipeline::process(path, metadata.len(), None, &settings)
                    {
                        let writes: Vec<ChunkWrite> = chunks
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
            .expect("could not write a batch");
    }
    let chunks = store.extract_progress().unwrap().chunks;
    println!("  {chunks} chunks in {:.1?}", started.elapsed());

    // Embed.
    let started = Instant::now();
    let mut done = 0;
    loop {
        let pending = store
            .chunks_without_embedding(reference, 64)
            .expect("could not read the embedding queue");
        if pending.is_empty() {
            break;
        }
        for group in pending.chunks(16) {
            let mut texts = Vec::new();
            let mut keys = Vec::new();
            for chunk in group {
                if let Some(text) = cortex_extract::snippet::chunk_text(
                    cortex_extract::snippet::Location {
                        path: &chunk.path,
                        byte_start: chunk.byte_start as usize,
                        byte_end: chunk.byte_end as usize,
                        hash: &chunk.hash,
                    },
                ) {
                    texts.push(text);
                    keys.push((chunk.chunk_id, chunk.hash.clone()));
                }
            }
            if keys.is_empty() {
                continue;
            }
            let inputs: Vec<Input<'_>> = texts.iter().map(|t| Input::Passage(t)).collect();
            let produced = embedder.embed(&inputs).expect("embedding failed");
            let rows: Vec<EmbeddingWrite> = keys
                .iter()
                .zip(produced)
                .map(|((id, hash), vector)| EmbeddingWrite {
                    chunk_id: *id,
                    chunk_hash: hash.clone(),
                    vector,
                })
                .collect();
            store
                .upsert_embeddings(reference, &rows, 0)
                .expect("could not store embeddings");
            vectors
                .add_many(rows.iter().map(|r| (r.chunk_id, r.vector.as_slice())))
                .expect("could not index vectors");
            done += rows.len();
            if done % 512 == 0 {
                print!("\r  embedded {done} of {chunks}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }
    vectors.save().expect("could not save the vector index");
    println!("\r  embedded {done} chunks in {:.1?}", started.elapsed());

    // Last, and only on the way out: everything above it is what the marker vouches for.
    store
        .meta_set(CACHE_MARKER, &cache_generation(model))
        .expect("could not mark the cache complete");

    (store, vectors)
}

/// Key under which the cache records that it was finished, and by what.
const CACHE_MARKER: &str = "goldset_cache";

/// What a usable cache must have been built by: this model, this version, this schema.
fn cache_generation(model: &cortex_embed::ModelId) -> String {
    format!("{}/{}/schema{}", model.id, model.version, cortex_store::migrations::target_version())
}

fn collect(root: &Path, out: &mut Vec<PathBuf>) {
    let mut queue = vec![root.to_path_buf()];
    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !cortex_core::denylist::should_skip_subtree(&name) {
                    queue.push(path);
                }
            } else if cortex_core::denylist::classify(&path, &[]).may_index() {
                out.push(path);
            }
        }
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("{} is not valid: {e}", path.display()))
}

fn validate(documents: &Documents, queries: &Queries) {
    let ids: std::collections::HashSet<&str> =
        documents.document.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        documents.document.len(),
        "two goldset documents share an id"
    );

    let mut seen = std::collections::HashSet::new();
    for query in &queries.query {
        assert!(seen.insert(query.id.as_str()), "duplicate query id {}", query.id);
        assert!(!query.relevant.is_empty(), "{} judges nothing relevant", query.id);
        for id in &query.relevant {
            assert!(ids.contains(id.as_str()), "{} names unknown document {id}", query.id);
        }
        assert!(
            ["name", "lexical", "semantic", "crosslang"].contains(&query.kind.as_str()),
            "{} has unknown kind {}",
            query.id,
            query.kind
        );
        assert!(["de", "en"].contains(&query.lang.as_str()), "{} has unknown lang", query.id);
    }
    println!(
        "{} documents, {} queries",
        documents.document.len(),
        queries.query.len()
    );
}

fn write_baseline(path: &Path, measured: &BTreeMap<String, f64>) {
    let mut text = String::from(
        "# Recall@10 as last measured, by arm and by what the query tests.\n\
         #\n\
         # Written by `goldset --write-baseline`. A drop of more than three points against any of\n\
         # these fails the run — which is the point: a ranking change that helps one kind of query\n\
         # and quietly ruins another is the failure mode this file exists to catch.\n\
         #\n\
         # Update it only together with the reason it changed.\n\n[recall]\n",
    );
    for (key, value) in measured {
        text.push_str(&format!("\"{key}\" = {value:.4}\n"));
    }
    std::fs::write(path, text).expect("could not write the baseline");
}
