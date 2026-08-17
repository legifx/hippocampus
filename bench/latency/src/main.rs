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
use cortex_query::PathIndex;
use std::path::PathBuf;
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
