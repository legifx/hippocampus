//! Running a query across every arm and turning the result into something explainable.
//!
//! The arms are independent by construction: the path index knows nothing about content, the
//! content index knows nothing about paths, and neither knows how the other is scored. They
//! meet exactly once, in [`cortex_query::fusion`], on ranks rather than on scores. Adding the
//! vector arm in M2's second half means producing one more `Ranking` here — not touching
//! anything else.
//!
//! Two things this module is careful about:
//!
//! **The database is consulted for survivors only.** A query produces a few hundred candidates
//! and displays fifty. Reading tier, salience and zone for every candidate would undo what the
//! in-memory index is for.
//!
//! **A snippet is never invented.** It is re-read from the file and checked against the hash the
//! chunk had when it was indexed; if the file has moved on, the stored preview is used instead.
//! Showing text that is no longer there is worse than showing less.

use cortex_core::ids::NodeId;
use cortex_core::model::{NodeKind, QueryMode, ScoreBreakdown, SearchHit, Tier};
use cortex_extract::snippet::{self, Location};
use crate::fusion::{self, Arm, Ranking};
use crate::path_index::PathIndex;
use crate::{lexical, mode};
use cortex_store::chunks::LexicalHit;
use cortex_store::Store;
use std::collections::HashMap;
use std::path::PathBuf;

/// Characters of context shown under a result.
const SNIPPET_CHARS: usize = 200;

/// How far past the requested limit the salience boost is allowed to reach.
///
/// The boost is bounded (`max_salience_boost`, 1.5 by default), so it can only promote a result
/// whose fused score is within that factor of the last one shown. With RRF scores of
/// `1 / (60 + rank)`, a factor of 1.5 reaches about thirty ranks. Three times the limit, and
/// never fewer than sixty-four, covers that with room to spare — and bounds the salience lookup
/// to one query over a small set rather than over every candidate.
const SALIENCE_HEADROOM: usize = 3;

/// How many chunk hits to ask the content index for, relative to the candidate budget.
///
/// More than one chunk of a document matches a typical query, and only the best one survives
/// per node. Asking for the same number of chunks as the number of documents wanted would
/// therefore return far fewer documents than the budget allows.
const CHUNK_OVERSAMPLE: usize = 4;

/// Everything one search produced, including what it would take to explain it.
pub struct Outcome {
    pub hits: Vec<SearchHit>,
    /// The mode that actually ran.
    pub mode: QueryMode,
    /// The query with any mode prefix stripped — what was really searched for.
    pub query: String,
    pub expression: Option<String>,
    pub path_candidates: usize,
    pub lexical_candidates: usize,
}

pub struct Params<'a> {
    pub store: &'a Store,
    pub index: &'a PathIndex,
    pub requested_mode: QueryMode,
    pub raw_query: &'a str,
    pub limit: usize,
    /// Treat the last word as a prefix. True while typing, false for an exact query.
    pub prefix: bool,
    pub candidates_per_arm: usize,
    pub rrf_k: f64,
    pub max_salience_boost: f64,
}

pub fn run(params: Params<'_>) -> Outcome {
    let resolved = mode::resolve(params.requested_mode, params.raw_query);
    let query = resolved.query;
    let effective = effective_mode(resolved.mode);

    // Arm one: paths, from memory.
    let path_hits = params.index.search(query, params.candidates_per_arm);

    // Arm two: content, from the lexical index. `Locate` skips it deliberately — a path lookup
    // must not wait for content retrieval, and that is the ninety-per-cent case.
    let mut expression = None;
    let mut chunk_hits: Vec<LexicalHit> = Vec::new();
    if effective != QueryMode::Locate {
        if let Some(expr) = lexical::match_expression(query, params.prefix) {
            match params
                .store
                .search_chunks(&expr, params.candidates_per_arm * CHUNK_OVERSAMPLE)
            {
                Ok(hits) => chunk_hits = hits,
                // A malformed expression is a bug in the expression builder, not a reason to
                // answer nothing: the path arm still has an answer.
                Err(e) => tracing::warn!(error = %e, expression = %expr, "lexical arm failed"),
            }
            expression = Some(expr);
        }
    }

    // One row per document, represented by its best-matching chunk.
    let best = fusion::best_per_node(&chunk_hits, |h| h.node_id, |h| h.score);
    let lexical_order: Vec<&LexicalHit> = best.iter().map(|i| &chunk_hits[*i]).collect();

    let fused = fusion::fuse(
        &[
            Ranking {
                arm: Arm::Path,
                nodes: path_hits.iter().map(|h| h.node_id).collect(),
            },
            Ranking {
                arm: Arm::Lexical,
                nodes: lexical_order.iter().map(|h| h.node_id).collect(),
            },
        ],
        params.rrf_k,
    );

    let paths: HashMap<NodeId, &str> = path_hits
        .iter()
        .map(|h| (h.node_id, h.path.as_str()))
        .collect();
    let chunks: HashMap<NodeId, &LexicalHit> =
        lexical_order.iter().map(|h| (h.node_id, *h)).collect();
    let terms = lexical::plain_terms(query);

    // Salience is applied *before* the list is cut, or it could never promote anything into it.
    // One query over the headroom set, not one per candidate and not one per displayed row.
    let considered = params.limit.saturating_mul(SALIENCE_HEADROOM).max(64);
    let candidates: Vec<&fusion::Fused> = fused.iter().take(considered).collect();
    let salience = params
        .store
        .salience_of(&candidates.iter().map(|f| f.node_id).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut boosted: Vec<(&fusion::Fused, f64, f64)> = candidates
        .iter()
        .map(|f| {
            let boost = fusion::salience_boost(
                salience.get(&f.node_id).copied().unwrap_or(0.0),
                params.max_salience_boost,
            );
            (*f, boost, f.fused * boost)
        })
        .collect();
    // Same tie-break as fusion itself, for the same reason: two results with identical scores
    // must not swap places between runs.
    boosted.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.node_id.0.cmp(&b.0.node_id.0))
    });

    let hits = boosted
        .iter()
        .take(params.limit)
        .map(|(f, boost, final_score)| {
            let node = params.store.node_by_id(f.node_id).ok().flatten();
            let path = node
                .as_ref()
                .map(|n| n.path.clone())
                .or_else(|| paths.get(&f.node_id).map(PathBuf::from))
                .or_else(|| chunks.get(&f.node_id).map(|h| h.path.clone()))
                .unwrap_or_default();

            SearchHit {
                node_id: f.node_id,
                path,
                kind: node.as_ref().map(|n| n.kind).unwrap_or(NodeKind::File),
                tier: node.as_ref().map(|n| n.tier).unwrap_or(Tier::Stm),
                zone_id: node.as_ref().and_then(|n| n.zone_id),
                snippet: chunks.get(&f.node_id).and_then(|h| snippet_for(h, &terms)),
                score: ScoreBreakdown {
                    path_rank: f.path_rank,
                    lexical_rank: f.lexical_rank,
                    vector_rank: f.vector_rank,
                    fused: f.fused,
                    salience_boost: *boost,
                    rerank: None,
                    final_score: *final_score,
                },
            }
        })
        .collect();

    Outcome {
        hits,
        mode: effective,
        query: query.to_string(),
        expression,
        path_candidates: path_hits.len(),
        lexical_candidates: lexical_order.len(),
    }
}

/// Map a requested mode onto one this build can actually serve.
///
/// `Temporal` needs episode retrieval with time parsing, `Structural` needs graph traversal and
/// `Deep` needs the cold tier and a peer. None of those exist yet. Degrading to the fused search
/// is better than an error — but the response reports the mode that ran, so nobody is told they
/// got something they did not.
fn effective_mode(requested: QueryMode) -> QueryMode {
    match requested {
        QueryMode::Locate => QueryMode::Locate,
        QueryMode::Auto | QueryMode::Recall => QueryMode::Recall,
        QueryMode::Temporal | QueryMode::Structural | QueryMode::Deep => QueryMode::Recall,
    }
}

/// Re-read the matching chunk, falling back to the stored preview.
fn snippet_for(hit: &LexicalHit, terms: &[String]) -> Option<String> {
    let from_file = snippet::snippet(
        Location {
            path: &hit.path,
            byte_start: hit.byte_start as usize,
            byte_end: hit.byte_end as usize,
            hash: &hit.hash,
        },
        terms,
        SNIPPET_CHARS,
    );

    from_file.or_else(|| {
        hit.preview
            .as_deref()
            .map(|preview| snippet::excerpt(preview, terms, SNIPPET_CHARS))
    })
}

/// Explain one node's position in a result list.
pub fn explain(outcome: &Outcome, node_id: NodeId) -> (Option<u32>, Option<ScoreBreakdown>) {
    match outcome.hits.iter().position(|h| h.node_id == node_id) {
        Some(index) => (Some(index as u32 + 1), Some(outcome.hits[index].score.clone())),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::model::NodeKind;
    use cortex_store::chunks::ChunkWrite;
    use cortex_store::queries::NodeUpsert;
    use std::path::Path;

    struct Fixture {
        tmp: tempfile::TempDir,
        store: Store,
        index: PathIndex,
    }

    impl Fixture {
        /// Build a small corpus: files on disk, nodes in the store, chunks in the lexical index.
        fn new(files: &[(&str, &str)]) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let store = Store::open_in_memory().unwrap();
            let mut paths = Vec::new();

            for (name, contents) in files {
                let path = tmp.path().join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, contents).unwrap();

                let id = store
                    .upsert_node(
                        &NodeUpsert {
                            kind: NodeKind::File,
                            path: path.clone(),
                            parent_id: None,
                            mime: None,
                            size: contents.len() as i64,
                            mtime: 1,
                            ctime: 1,
                            inode: 1,
                            dev: 1,
                            zone_id: None,
                        },
                        1000,
                    )
                    .unwrap();
                paths.push((id, path.display().to_string()));

                for c in cortex_extract::chunk::chunk(contents, cortex_extract::classify::Handling::PlainText) {
                    store
                        .replace_chunks(
                            id,
                            &[ChunkWrite {
                                ordinal: c.ordinal as i64,
                                byte_start: c.byte_start as i64,
                                byte_end: c.byte_end as i64,
                                preview: Some(c.preview(240)),
                                hash: c.hash.clone(),
                                text: c.text.clone(),
                            }],
                        )
                        .unwrap();
                }
            }

            let index = PathIndex::build(paths.iter().map(|(id, p)| (*id, p.as_str())));
            Self { tmp, store, index }
        }

        fn search(&self, query: &str) -> Outcome {
            self.search_as(QueryMode::Auto, query)
        }

        fn search_with_limit(&self, query: &str, limit: usize) -> Outcome {
            run(Params {
                store: &self.store,
                index: &self.index,
                requested_mode: QueryMode::Auto,
                raw_query: query,
                limit,
                prefix: true,
                candidates_per_arm: 100,
                rrf_k: 60.0,
                max_salience_boost: 1.5,
            })
        }

        fn search_as(&self, mode: QueryMode, query: &str) -> Outcome {
            run(Params {
                store: &self.store,
                index: &self.index,
                requested_mode: mode,
                raw_query: query,
                limit: 10,
                prefix: true,
                candidates_per_arm: 100,
                rrf_k: 60.0,
                max_salience_boost: 1.5,
            })
        }

        fn path(&self, name: &str) -> PathBuf {
            self.tmp.path().join(name)
        }
    }

    fn names(outcome: &Outcome) -> Vec<String> {
        outcome
            .hits
            .iter()
            .map(|h| {
                Path::new(&h.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn a_word_that_appears_only_inside_a_file_still_finds_it() {
        // The whole reason the content arm exists: no file manager's search finds this.
        let fixture = Fixture::new(&[
            ("notizen.txt", "Die Blickachse bleibt über die Sequenz konstant.\n"),
            ("anderes.txt", "Nichts hiermit zu tun.\n"),
        ]);

        let outcome = fixture.search("blickachse");
        assert_eq!(names(&outcome), vec!["notizen.txt"]);
        assert_eq!(outcome.hits[0].score.lexical_rank, Some(1));
        assert_eq!(outcome.hits[0].score.path_rank, None);
    }

    #[test]
    fn a_filename_query_is_answered_from_paths_alone() {
        let fixture = Fixture::new(&[("bericht-2026.md", "Inhalt ohne das gesuchte Wort.\n")]);
        let outcome = fixture.search("bericht-2026.md");
        assert_eq!(outcome.mode, QueryMode::Locate);
        assert!(outcome.expression.is_none(), "locate must not touch the content index");
        assert_eq!(names(&outcome), vec!["bericht-2026.md"]);
        assert_eq!(outcome.hits[0].score.path_rank, Some(1));
    }

    #[test]
    fn a_file_matching_in_both_arms_outranks_one_matching_in_either() {
        let fixture = Fixture::new(&[
            ("kamerawinkel.md", "Notizen zur Aufnahme.\n"),
            ("beliebig.md", "Der Kamerawinkel war zu steil.\n"),
            (
                "kamerawinkel-notizen.md",
                "Der Kamerawinkel bestimmt die Bildwirkung.\n",
            ),
        ]);

        let outcome = fixture.search("kamerawinkel");
        assert_eq!(
            names(&outcome)[0],
            "kamerawinkel-notizen.md",
            "the file both arms found must lead: {:?}",
            names(&outcome)
        );
        assert!(outcome.hits[0].score.path_rank.is_some());
        assert!(outcome.hits[0].score.lexical_rank.is_some());
    }

    #[test]
    fn a_content_hit_carries_a_snippet_read_from_the_file() {
        let fixture = Fixture::new(&[(
            "lang.txt",
            &format!(
                "{}Die Blickachse bleibt konstant.{}",
                "Vorlauf ".repeat(60),
                " Nachlauf".repeat(60)
            ),
        )]);

        let outcome = fixture.search("blickachse");
        let snippet = outcome.hits[0].snippet.as_deref().expect("expected a snippet");
        assert!(snippet.contains("Blickachse"), "snippet missed the match: {snippet}");
        assert!(snippet.chars().count() <= 210);
    }

    #[test]
    fn a_snippet_is_refused_rather_than_faked_when_the_file_has_changed() {
        let fixture = Fixture::new(&[("wandelbar.txt", "Die Blickachse bleibt konstant.\n")]);
        std::fs::write(fixture.path("wandelbar.txt"), "Etwas völlig anderes steht hier.\n").unwrap();

        let outcome = fixture.search("blickachse");
        let snippet = outcome.hits[0].snippet.as_deref().unwrap();
        assert!(
            !snippet.contains("völlig anderes"),
            "the snippet quoted text that was never matched: {snippet}"
        );
    }

    #[test]
    fn punctuation_in_a_query_cannot_break_the_search() {
        // Straight into FTS5 these are syntax errors. The search has to survive them, because
        // people type them.
        let fixture = Fixture::new(&[("code.txt", "Ein Aufruf von foo(bar) im Beispiel.\n")]);
        for query in ["foo(bar)", "was ist das?", "c++", "\"unbalanced"] {
            let outcome = fixture.search(query);
            assert!(
                outcome.expression.is_some() || outcome.mode == QueryMode::Locate,
                "{query:?} produced no lexical arm at all"
            );
        }
        assert!(!fixture.search("foo(bar)").hits.is_empty());
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_everything() {
        let fixture = Fixture::new(&[("irgendwas.txt", "Inhalt.\n")]);
        assert!(fixture.search("   ").hits.is_empty());
    }

    #[test]
    fn a_mode_that_is_not_built_yet_degrades_and_says_so() {
        let fixture = Fixture::new(&[("notiz.txt", "Blickachse.\n")]);
        let outcome = fixture.search_as(QueryMode::Temporal, "blickachse");
        assert_eq!(
            outcome.mode,
            QueryMode::Recall,
            "the response must report what actually ran"
        );
    }

    #[test]
    fn salience_can_promote_a_result_into_the_list_it_would_have_missed() {
        // The bug this prevents is silent: applying the boost after truncating the list makes
        // it decoration. It changes the number printed beside a result and can never change
        // which results there are, which is the one thing a ranking signal is for.
        let fixture = Fixture::new(&[
            ("a-blickachse.txt", "Die Blickachse bleibt konstant.\n"),
            ("b-blickachse.txt", "Die Blickachse bleibt konstant.\n"),
            ("c-blickachse.txt", "Die Blickachse bleibt konstant.\n"),
        ]);

        let unboosted = fixture.search_with_limit("blickachse", 1);
        let first = unboosted.hits[0].node_id;

        // Make a *different* document matter.
        let promoted = fixture
            .store
            .all_paths_with_ids()
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .find(|id| *id != first)
            .unwrap();
        fixture.store.set_salience(promoted, 1.0).unwrap();

        let boosted = fixture.search_with_limit("blickachse", 1);
        assert_eq!(
            boosted.hits[0].node_id, promoted,
            "a salient document must be able to enter a list of one"
        );
        assert_eq!(boosted.hits[0].score.salience_boost, 1.5);
        assert!(boosted.hits[0].score.final_score > boosted.hits[0].score.fused);
    }

    #[test]
    fn every_hit_can_explain_itself() {
        // Explainability is a Definition-of-Done item, not a debugging aid: a ranking nobody
        // can interrogate is a ranking nobody can fix.
        let fixture = Fixture::new(&[("notiz.txt", "Die Blickachse bleibt konstant.\n")]);
        let outcome = fixture.search("blickachse");
        let (rank, score) = explain(&outcome, outcome.hits[0].node_id);
        assert_eq!(rank, Some(1));
        let score = score.unwrap();
        assert!(score.fused > 0.0);
        assert_eq!(score.salience_boost, 1.0, "no salience engine until M5");
        assert_eq!(score.final_score, score.fused * score.salience_boost);
    }
}
