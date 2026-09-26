//! Finding the candidates `brain.ask` shows its curator, and putting them in order.
//!
//! The first evaluation said plainly where the problem was: with twelve real questions about
//! real projects, retrieval put the right document first four times. The decision model did not
//! fix that and could not — it can only choose among what it is shown, and what it was shown was
//! a tokeniser's calibration corpus, a third-party repository's training data in JSON Lines and a
//! voice-over script, because those are large, varied text and match every question a little.
//!
//! Three changes, each answering one observed failure:
//!
//! 1. **Bulk data is not an answer.** Datasets, logs, dumps, lockfiles and build output are
//!    dropped from the candidates. They stay searchable through `brain.search`; they are simply
//!    never what "where is X decided" or "how does Y work" is asking for.
//! 2. **A question that names a project is about that project.** When a word in the question is
//!    the name of a zone, that zone's own documentation — README, agent entry file, docs/,
//!    progress and decision logs — joins the candidates, each represented by the passage that
//!    shares the most words with the question.
//! 3. **One order for everything.** Search hits, project documents and memories are fused: the
//!    retrieval rank, the cross-encoder's rank over the same passages, and a small bonus for
//!    documentation and for the named project. A memory must also score at least like the third
//!    best document, because the memory arm always returns its nearest neighbours, related or not.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::curator::Candidate;

/// Path segments that mark generated, vendored or bulk material.
const BULK_SEGMENTS: &[&str] = &[
    "node_modules", ".venv", "venv", "vendor", "dist", "build", "target", "__pycache__",
    "data", "datasets", "dataset", "evals", "eval_data", "checkpoints", "runs", "logs",
    "site-packages", ".git", ".cache",
];
/// Extensions that are data rather than prose or code.
const BULK_EXTENSIONS: &[&str] = &[
    "jsonl", "ndjson", "csv", "tsv", "parquet", "arrow", "npy", "npz", "pt", "bin", "safetensors",
    "gguf", "onnx", "lock", "sqlite", "db", "map", "min.js", "svg", "srt", "vtt",
];
/// Above this, a plain-text file is a corpus, not a note.
const MAX_TEXT_BYTES: u64 = 1_500_000;

/// Logs are bulk unless the question is about a log — then they are exactly what is wanted.
pub fn is_bulk_for(path: &str, question: &str) -> bool {
    let q = question.to_lowercase();
    let about_logs = q.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with("log") || w == "journal" || w == "protokoll");
    if path.to_ascii_lowercase().ends_with(".log") && !about_logs {
        return true;
    }
    is_bulk(path)
}

pub fn is_bulk(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower
        .split('/')
        .any(|seg| BULK_SEGMENTS.contains(&seg) || seg.starts_with("_bak") || seg.starts_with(".bak"))
    {
        return true;
    }
    if BULK_EXTENSIONS.iter().any(|ext| lower.ends_with(&format!(".{ext}"))) {
        return true;
    }
    if lower.ends_with(".txt") || lower.ends_with(".json") {
        return std::fs::metadata(path).map(|m| m.len() > MAX_TEXT_BYTES).unwrap_or(false);
    }
    false
}

pub fn is_doc(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".rst") || lower.ends_with(".adoc")
}

/// Words, and the parts of hyphenated words: "ESP32-Timer" is also "esp32" and "timer".
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in text
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .map(|w| w.trim_matches(|c: char| c == '.' || c == '-' || c == '_').to_string())
        .filter(|w| !w.is_empty())
    {
        if w.contains('-') {
            out.extend(w.split('-').filter(|p| !p.is_empty()).map(|p| p.to_string()));
        }
        out.push(w);
    }
    out
}

/// Zones the question names: a word equal to the zone's name, or — for names of five letters or
/// more — one that starts with it or that it starts with ("esp32ctl" for "ESP32", "laya-jev-bench"
/// for "laya-jev-bench"). Longest name first, at most two.
pub fn named_zones(question: &str, zones: &[(String, PathBuf)]) -> Vec<(String, PathBuf)> {
    let ws = words(question);
    let mut hits: Vec<(String, PathBuf)> = zones
        .iter()
        .filter(|(name, _)| {
            let n = name.to_lowercase();
            let bare = n.trim_end_matches('.');
            ws.iter().any(|w| {
                w == bare
                    || (bare.len() >= 5 && w.len() >= 5 && (w.starts_with(bare) || bare.starts_with(w.as_str())))
                    || (bare.contains('-') && question.to_lowercase().contains(bare))
            })
        })
        .cloned()
        .collect();
    hits.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    hits.dedup_by(|a, b| a.1 == b.1);
    hits.truncate(2);
    hits
}

/// A project's own documentation, most canonical first.
pub fn zone_docs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in [
        "README.md", "CLAUDE.md", "AGENTS.md", "PROGRESS.md", "HANDOFF.md", "KONZEPT.md", "PLAN.md",
        "BEDIENUNG.md", "STAND.md", "MESSUNGEN.md",
    ] {
        let p = root.join(name);
        if p.is_file() {
            out.push(p);
        }
    }
    for dir in ["docs", "doc"] {
        if let Ok(rd) = std::fs::read_dir(root.join(dir)) {
            let mut files: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && is_doc(&p.to_string_lossy()))
                .collect();
            files.sort();
            out.extend(files);
        }
    }
    out.truncate(14);
    out
}

/// The window of `text` that shares the most words with the question, about `chars` long.
pub fn best_window(text: &str, question: &str, chars: usize) -> String {
    let terms: HashSet<String> = words(question).into_iter().filter(|w| w.chars().count() >= 4).collect();
    let paragraphs: Vec<&str> = text.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    let mut best = (0usize, 0usize);
    for (i, p) in paragraphs.iter().enumerate() {
        let lower = p.to_lowercase();
        let score = terms.iter().filter(|t| lower.contains(t.as_str())).count();
        if score > best.0 {
            best = (score, i);
        }
    }
    let mut window = String::new();
    for p in paragraphs.iter().skip(best.1) {
        if window.chars().count() >= chars {
            break;
        }
        if !window.is_empty() {
            window.push(' ');
        }
        window.push_str(p.trim());
    }
    window.chars().take(chars).collect()
}

/// Candidates from the named projects' documentation, skipping paths already present.
pub fn project_candidates(
    question: &str,
    named: &[(String, PathBuf)],
    present: &HashSet<String>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (zone, root) in named {
        for doc in zone_docs(root) {
            let path = doc.display().to_string();
            if present.contains(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&doc) else { continue };
            // A decision log can be a hundred kilobytes; the window search reads it once.
            let snippet = best_window(&text, question, 420);
            out.push(Candidate {
                kind: "file",
                title: doc.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                path: Some(path),
                memory_id: None,
                snippet: Some(snippet),
                zone: Some(zone.clone()),
            });
        }
    }
    out
}

/// A candidate with what the final order is computed from.
pub struct Ranked {
    pub candidate: Candidate,
    /// Position in the order retrieval produced; project documents count as retrieved first.
    pub retrieval: usize,
    pub in_named_zone: bool,
}

/// The fused order. `k` damps the reciprocal ranks so that the first few places do not dominate.
pub fn order(
    question: &str,
    ranked: Vec<Ranked>,
    reranker: Option<&dyn cortex_embed::Reranker>,
) -> Vec<Candidate> {
    const K: f32 = 8.0;
    let bonus = |r: &Ranked| -> f32 {
        let mut b = 0.0;
        if r.candidate.path.as_deref().is_some_and(is_doc) {
            b += 0.025;
        }
        if r.in_named_zone {
            b += 0.05;
        }
        b
    };

    let scores: Option<Vec<f32>> = reranker.and_then(|reranker| {
        let texts: Vec<String> = ranked
            .iter()
            .map(|r| {
                let c = &r.candidate;
                format!("{} {}", c.title, c.snippet.as_deref().unwrap_or(""))
            })
            .collect();
        let passages: Vec<&str> = texts.iter().map(|t| t.as_str()).collect();
        let mut scores = Vec::with_capacity(passages.len());
        for group in passages.chunks(reranker.max_batch()) {
            match reranker.score(question, group) {
                Ok(s) => scores.extend(s),
                Err(e) => {
                    tracing::debug!(error = %e, "brain.ask: the cross-encoder failed; retrieval order stands");
                    return None;
                }
            }
        }
        Some(scores)
    });

    let mut fused: Vec<(f32, Ranked)> = match &scores {
        Some(scores) => {
            let mut by_score: Vec<usize> = (0..ranked.len()).collect();
            by_score.sort_by(|a, b| scores[*b].partial_cmp(&scores[*a]).unwrap_or(std::cmp::Ordering::Equal));
            let mut rerank_rank = vec![0usize; ranked.len()];
            for (pos, idx) in by_score.iter().enumerate() {
                rerank_rank[*idx] = pos;
            }
            // The bar a memory must clear: the third-best non-memory passage, and never below zero.
            let mut file_scores: Vec<f32> = ranked
                .iter()
                .zip(scores.iter())
                .filter(|(r, _)| r.candidate.kind != "memory")
                .map(|(_, s)| *s)
                .collect();
            file_scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let bar = file_scores.get(2).copied().unwrap_or(0.0).max(0.0);
            ranked
                .into_iter()
                .enumerate()
                .filter(|(i, r)| r.candidate.kind != "memory" || scores[*i] >= bar)
                .map(|(i, r)| {
                    let s = 1.0 / (K + r.retrieval as f32) + 1.0 / (K + rerank_rank[i] as f32) + bonus(&r);
                    (s, r)
                })
                .collect()
        }
        None => ranked
            .into_iter()
            .map(|r| {
                // Without the cross-encoder nothing can say a memory is relevant; it goes behind
                // the two best documents, visible to the decision model and not in front of it.
                let retrieval = if r.candidate.kind == "memory" { r.retrieval.max(2) } else { r.retrieval };
                (1.0 / (K + retrieval as f32) + bonus(&r), r)
            })
            .collect(),
    };
    fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    fused.into_iter().map(|(_, r)| r.candidate).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasets_logs_and_vendored_code_are_bulk_and_docs_are_not() {
        assert!(is_bulk("/w/p/kev/evals/v8/test.jsonl"));
        assert!(is_bulk("/w/p/app/node_modules/x/README.md"));
        assert!(is_bulk_for("/w/p/bench/run.log", "Wie läuft der Bench?"));
        assert!(!is_bulk_for("/w/p/bench/run.log", "Wo ist das Log vom Bench-Lauf?"));
        assert!(is_bulk("/w/p/app/_bak_ui_20260924/index.html"));
        assert!(!is_bulk("/w/p/takt/docs/02-DECISIONS.md"));
        assert!(!is_bulk("/w/p/esp32ctl/src/main.py"));
    }

    #[test]
    fn a_project_named_in_the_question_is_found_and_a_short_word_is_not_a_project() {
        let zones = vec![
            ("esp32ctl".to_string(), PathBuf::from("/w/esp32ctl")),
            ("laya-jev-bench".to_string(), PathBuf::from("/w/laya-jev-bench")),
            ("takt".to_string(), PathBuf::from("/w/takt")),
            ("church.".to_string(), PathBuf::from("/w/church.")),
            ("ai".to_string(), PathBuf::from("/w/ai")),
        ];
        let names = |q: &str| named_zones(q, &zones).into_iter().map(|z| z.0).collect::<Vec<_>>();
        assert_eq!(names("Wie steuere ich den ESP32-Timer?"), vec!["esp32ctl"]);
        assert_eq!(names("Wie steuere ich esp32ctl?"), vec!["esp32ctl"]);
        assert_eq!(names("Was empfiehlt laya-jev-bench?"), vec!["laya-jev-bench"]);
        assert_eq!(names("Wie stellt takt zu?"), vec!["takt"]);
        assert_eq!(names("Was macht die church app?"), vec!["church."]);
        assert_eq!(names("Wie hilft KI im Alltag?"), Vec::<String>::new());
    }

    #[test]
    fn the_window_is_the_paragraph_that_shares_the_most_words() {
        let text = "Intro text here.\n\nUnrelated paragraph.\n\nDie Zustellung an das Gehirn läuft über SSH.\n\nTail.";
        let w = best_window(text, "Wie läuft die Zustellung ans Gehirn?", 60);
        assert!(w.starts_with("Die Zustellung"), "{w}");
    }

    fn cand(kind: &'static str, path: &str) -> Candidate {
        Candidate {
            kind,
            path: Some(path.into()),
            memory_id: None,
            title: path.into(),
            snippet: None,
            zone: None,
        }
    }

    #[test]
    fn without_a_cross_encoder_the_named_project_and_docs_come_first_and_memories_wait() {
        let ranked = vec![
            Ranked { candidate: cand("memory", "/m"), retrieval: 0, in_named_zone: false },
            Ranked { candidate: cand("file", "/x/a.py"), retrieval: 0, in_named_zone: false },
            Ranked { candidate: cand("file", "/takt/docs/02.md"), retrieval: 1, in_named_zone: true },
        ];
        let out: Vec<_> = order("q", ranked, None).into_iter().map(|c| c.path.unwrap()).collect();
        assert_eq!(out, vec!["/takt/docs/02.md", "/x/a.py", "/m"]);
    }
}
