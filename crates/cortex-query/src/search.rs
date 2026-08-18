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
use cortex_embed::{Embedder, Input, Reranker};
use cortex_store::chunks::LexicalHit;
use cortex_store::vectors::VectorIndex;
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
    pub vector_candidates: usize,
    /// Where the top hits' text can be read from, so a reranker can score the pairs. Only the
    /// hits that were actually returned, in their order.
    pub rerankable: Vec<Rerankable>,
    /// Why the vector arm did not run, when it did not. Shown by `cortex explain`, because
    /// "semantic search found nothing" and "semantic search is not running" look identical from
    /// the outside and have entirely different fixes.
    pub vector_note: Option<String>,
}

/// The vector arm's two halves: something to turn a query into a vector, and something to find
/// neighbours of it. Both or neither — an index without an embedder cannot be queried, and an
/// embedder without an index has nothing to search.
#[derive(Clone, Copy)]
pub struct Semantic<'a> {
    pub embedder: &'a dyn Embedder,
    pub vectors: &'a VectorIndex,
}

pub struct Params<'a> {
    pub store: &'a Store,
    pub index: &'a PathIndex,
    /// Absent when no model is provisioned, which is the default state of a fresh install.
    pub semantic: Option<Semantic<'a>>,
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

    // Arm three: meaning. Skipped for `Locate` for the same reason as the lexical arm, and
    // absent entirely until a model has been fetched.
    let mut vector_note = None;
    let mut vector_hits: Vec<ChunkMatch> = Vec::new();
    if effective != QueryMode::Locate && !query.trim().is_empty() {
        match params.semantic {
            None => vector_note = Some("no embedding model (run `cortex model fetch`)".into()),
            Some(semantic) if semantic.vectors.is_empty() => {
                vector_note = Some("nothing embedded yet".into())
            }
            Some(semantic) => match vector_arm(semantic, params.store, query, params.candidates_per_arm) {
                Ok(hits) => vector_hits = hits,
                Err(note) => vector_note = Some(note),
            },
        }
    }

    // One row per document, represented by its best-matching chunk.
    let best = fusion::best_per_node(&chunk_hits, |h| h.node_id, |h| h.score);
    let lexical_order: Vec<&LexicalHit> = best.iter().map(|i| &chunk_hits[*i]).collect();
    let best_vector = fusion::best_per_node(&vector_hits, |h| h.node_id, |h| h.score);
    let vector_order: Vec<&ChunkMatch> = best_vector.iter().map(|i| &vector_hits[*i]).collect();

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
            Ranking {
                arm: Arm::Vector,
                nodes: vector_order.iter().map(|h| h.node_id).collect(),
            },
        ],
        params.rrf_k,
    );

    let paths: HashMap<NodeId, &str> = path_hits
        .iter()
        .map(|h| (h.node_id, h.path.as_str()))
        .collect();
    // A document the vector arm found and the lexical arm did not still needs a snippet, and it
    // is the case where a snippet matters most: nothing in the query appears in the text, so the
    // path alone tells the reader nothing about why it is there.
    let mut chunks: HashMap<NodeId, ChunkMatch> = HashMap::new();
    for hit in &vector_order {
        chunks.insert(hit.node_id, (*hit).clone());
    }
    for hit in &lexical_order {
        chunks.insert(hit.node_id, ChunkMatch::from_lexical(hit));
    }
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

    let hits: Vec<SearchHit> = boosted
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

    let rerankable: Vec<Rerankable> = hits
        .iter()
        .map(|hit| match chunks.get(&hit.node_id) {
            Some(chunk) => Rerankable {
                node_id: hit.node_id,
                path: chunk.path.clone(),
                byte_start: chunk.byte_start,
                byte_end: chunk.byte_end,
                hash: chunk.hash.clone(),
                preview: chunk.preview.clone(),
            },
            // A path-only hit has no chunk at all. It is still rerankable — against its own
            // name, which is what the user matched on.
            None => Rerankable {
                node_id: hit.node_id,
                path: hit.path.clone(),
                byte_start: 0,
                byte_end: 0,
                hash: String::new(),
                preview: hit
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            },
        })
        .collect();

    Outcome {
        hits,
        rerankable,
        mode: effective,
        query: query.to_string(),
        expression,
        path_candidates: path_hits.len(),
        lexical_candidates: lexical_order.len(),
        vector_candidates: vector_order.len(),
        vector_note,
    }
}

/// Enough to re-read one result's text for reranking.
#[derive(Debug, Clone)]
pub struct Rerankable {
    pub node_id: NodeId,
    pub path: PathBuf,
    pub byte_start: i64,
    pub byte_end: i64,
    pub hash: String,
    /// The stored excerpt, used when the file has moved on. Weaker than the chunk but better
    /// than dropping the candidate, which would let a stale file quietly lose its ranking.
    pub preview: Option<String>,
}

/// A chunk that matched, whichever arm found it.
#[derive(Debug, Clone)]
struct ChunkMatch {
    node_id: NodeId,
    path: PathBuf,
    byte_start: i64,
    byte_end: i64,
    hash: String,
    preview: Option<String>,
    score: f64,
}

impl ChunkMatch {
    fn from_lexical(hit: &LexicalHit) -> Self {
        Self {
            node_id: hit.node_id,
            path: hit.path.clone(),
            byte_start: hit.byte_start,
            byte_end: hit.byte_end,
            hash: hit.hash.clone(),
            preview: hit.preview.clone(),
            score: hit.score,
        }
    }
}

/// Embed the query, find its neighbours, and turn chunk ids back into locations.
///
/// Errors come back as a note rather than as a failure: the other two arms have an answer, and a
/// search that returns nothing because one arm broke is worse than one that returns less.
fn vector_arm(
    semantic: Semantic<'_>,
    store: &Store,
    query: &str,
    candidates: usize,
) -> std::result::Result<Vec<ChunkMatch>, String> {
    // `Input::Query`, not `Passage`. The model is asymmetric and embedding a query as a passage
    // costs recall silently — see `cortex_embed::Input`.
    let vector = semantic
        .embedder
        .embed(&[Input::Query(query)])
        .map_err(|e| format!("could not embed the query: {e}"))?
        .pop()
        .ok_or_else(|| "the embedder returned nothing".to_string())?;

    if vector.len() != semantic.vectors.dim() {
        // A model was swapped without rebuilding the index. Refuse rather than compare vectors
        // from two different spaces, which produces confident nonsense.
        return Err(format!(
            "the model produces {} dimensions but the index holds {}; rebuild the vector index",
            vector.len(),
            semantic.vectors.dim()
        ));
    }

    let hits = semantic
        .vectors
        .search(&vector, candidates * CHUNK_OVERSAMPLE)
        .map_err(|e| format!("vector search failed: {e}"))?;
    let ids: Vec<cortex_core::ids::ChunkId> = hits.iter().map(|h| h.chunk_id).collect();
    let locations = store
        .chunk_locations(&ids)
        .map_err(|e| format!("could not resolve vector hits: {e}"))?;

    let by_id: HashMap<_, _> = locations.into_iter().map(|l| (l.chunk_id, l)).collect();
    Ok(hits
        .into_iter()
        .filter_map(|hit| {
            let location = by_id.get(&hit.chunk_id)?;
            Some(ChunkMatch {
                node_id: location.node_id,
                path: location.path.clone(),
                byte_start: location.byte_start,
                byte_end: location.byte_end,
                hash: location.hash.clone(),
                preview: location.preview.clone(),
                score: hit.similarity as f64,
            })
        })
        .collect())
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
fn snippet_for(hit: &ChunkMatch, terms: &[String]) -> Option<String> {
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

/// Reorder the top results by asking a cross-encoder to read each one against the query.
///
/// This is the last stage and the only one that reads a query and a passage *together*, which is
/// why it is also the only one that can tell "storage costs went up" from "storage costs were
/// discussed" — and why it cannot run over the whole corpus. It reorders; it never adds. A
/// document that retrieval did not surface cannot be rescued here, which is the honest limit of
/// reranking and worth knowing before reaching for it as a fix.
///
/// Failure is not fatal: the fused order stands and the caller keeps what it had.
pub fn rerank(
    outcome: &Outcome,
    reranker: &dyn Reranker,
    top_n: usize,
    snippet_chars: usize,
) -> std::result::Result<Vec<SearchHit>, String> {
    let considered = top_n.min(outcome.hits.len());
    if considered == 0 {
        return Ok(outcome.hits.clone());
    }

    // Read each candidate's text. The chunk, when it is still what the index says it is; the
    // stored preview otherwise; the file name for a hit that has no content at all.
    let mut texts: Vec<String> = Vec::with_capacity(considered);
    for candidate in outcome.rerankable.iter().take(considered) {
        let text = if candidate.byte_end > candidate.byte_start {
            snippet::chunk_text(Location {
                path: &candidate.path,
                byte_start: candidate.byte_start as usize,
                byte_end: candidate.byte_end as usize,
                hash: &candidate.hash,
            })
        } else {
            None
        };
        texts.push(
            text.or_else(|| candidate.preview.clone())
                .unwrap_or_else(|| candidate.path.display().to_string()),
        );
    }

    // In batches, not all at once. Attention is quadratic in sequence length and linear in batch
    // size, so thirty long pairs in one call asked for 377 MB of GPU memory and failed on a card
    // that was busy with something else — measured, on a machine where a language model held
    // most of the card. Several smaller calls cost a little more wall-clock and a great deal
    // less peak memory.
    let passages: Vec<&str> = texts.iter().map(|t| t.as_str()).collect();
    let mut scores = Vec::with_capacity(passages.len());
    for group in passages.chunks(reranker.max_batch()) {
        scores.extend(
            reranker
                .score(&outcome.query, group)
                .map_err(|e| e.to_string())?,
        );
    }

    // Fused with the order retrieval already produced, not substituted for it.
    //
    // Substituting was the first implementation and the goldset rejected it: MRR fell from 0.882
    // to 0.746 and top-1 from 86 % to 72 %, because when a cross-encoder finds nothing it likes,
    // its scores collapse into a narrow negative band where the ordering is noise — and replacing
    // a good order with noise is worse than leaving it alone. Measured on this corpus it ranked
    // word-salad distractors above the right answer.
    //
    // Reciprocal rank fusion is the mechanism this project already uses for exactly this
    // situation: two opinions on incomparable scales, combined on ranks. A document the reranker
    // loves rises; one it dislikes falls; neither can be thrown away by a model having a bad day.
    let mut by_rerank: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
    by_rerank.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut rerank_position = vec![considered; considered];
    for (position, (index, _)) in by_rerank.iter().enumerate() {
        rerank_position[*index] = position;
    }

    let mut reordered: Vec<SearchHit> = outcome.hits.clone();
    let mut fused: Vec<(usize, f64)> = Vec::with_capacity(considered);
    for index in 0..considered {
        reordered[index].score.rerank = Some(scores[index] as f64);
        let retrieval_rank = index + 1;
        let reranked_rank = rerank_position[index] + 1;
        let combined = 1.0 / (RERANK_RRF_K + retrieval_rank as f64)
            + 1.0 / (RERANK_RRF_K + reranked_rank as f64);
        reordered[index].score.final_score = combined;
        fused.push((index, combined));
    }

    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let head: Vec<SearchHit> = fused
        .iter()
        .map(|(index, _)| reordered[*index].clone())
        .collect();
    let tail: Vec<SearchHit> = reordered.into_iter().skip(considered).collect();

    let _ = snippet_chars;
    Ok(head.into_iter().chain(tail).collect())
}

/// `k` for fusing the reranked order with the retrieved one.
///
/// Deliberately smaller than the `k` used across retrieval arms (60). There the arms are peers;
/// here the two orders are opinions about the *same* short list, and a large `k` would flatten
/// the difference between rank one and rank thirty into nothing. Ten lets the reranker actually
/// move things while still requiring it to disagree strongly before it can bury a result that
/// retrieval put first.
const RERANK_RRF_K: f64 = 10.0;

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
        vectors: VectorIndex,
        embedder: cortex_embed::hashing::HashingEmbedder,
        /// Whether the vector arm is wired up for this fixture.
        semantic: bool,
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
            let vectors = VectorIndex::open(
                &tmp.path().join("vectors.usearch"),
                cortex_embed::hashing::DIM,
            )
            .unwrap();
            Self {
                tmp,
                store,
                index,
                vectors,
                embedder: cortex_embed::hashing::HashingEmbedder::new(),
                semantic: false,
            }
        }

        /// Embed every chunk, as the embedding stage would.
        ///
        /// The hashing embedder is used on purpose: it is weak, it is real, and it needs no
        /// download, so these tests exercise the same code path the model does without pinning
        /// the suite to half a gigabyte of it.
        fn with_vectors(mut self) -> Self {
            let model = self.embedder.model();
            let reference = cortex_store::embeddings::ModelRef {
                id: &model.id,
                version: &model.version,
                dim: model.dim,
            };
            loop {
                let pending = self.store.chunks_without_embedding(reference, 64).unwrap();
                if pending.is_empty() {
                    break;
                }
                let mut rows = Vec::new();
                for chunk in &pending {
                    let text = cortex_extract::snippet::chunk_text(Location {
                        path: &chunk.path,
                        byte_start: chunk.byte_start as usize,
                        byte_end: chunk.byte_end as usize,
                        hash: &chunk.hash,
                    })
                    .expect("the fixture wrote these files itself");
                    let vector = self
                        .embedder
                        .embed(&[Input::Passage(&text)])
                        .unwrap()
                        .remove(0);
                    rows.push(cortex_store::embeddings::EmbeddingWrite {
                        chunk_id: chunk.chunk_id,
                        chunk_hash: chunk.hash.clone(),
                        vector,
                    });
                }
                self.store.upsert_embeddings(reference, &rows, 1).unwrap();
                self.vectors
                    .add_many(rows.iter().map(|r| (r.chunk_id, r.vector.as_slice())))
                    .unwrap();
            }
            self.semantic = true;
            self
        }

        fn semantic(&self) -> Option<Semantic<'_>> {
            self.semantic.then_some(Semantic {
                embedder: &self.embedder,
                vectors: &self.vectors,
            })
        }

        fn search(&self, query: &str) -> Outcome {
            self.search_as(QueryMode::Auto, query)
        }

        fn search_with_limit(&self, query: &str, limit: usize) -> Outcome {
            run(Params {
                store: &self.store,
                index: &self.index,
                semantic: self.semantic(),
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
                semantic: self.semantic(),
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
    fn the_vector_arm_contributes_a_ranking_of_its_own() {
        let fixture = Fixture::new(&[
            ("montage.txt", "Die Montage der Kamera dauerte drei Stunden.\n"),
            ("anderes.txt", "Der Haushalt wurde im Sommer freigegeben.\n"),
        ])
        .with_vectors();

        let outcome = fixture.search("montage kamera");
        assert!(outcome.vector_candidates > 0, "the vector arm produced nothing");
        assert!(
            outcome.hits.iter().any(|h| h.score.vector_rank.is_some()),
            "no result carries a vector rank"
        );
        assert!(outcome.vector_note.is_none(), "{:?}", outcome.vector_note);
    }

    #[test]
    fn a_document_the_lexical_arm_misses_can_still_be_found() {
        // The lexical arm requires every term to be present. A document holding only some of
        // them is invisible to it and still relevant — which is the whole reason for a second
        // content arm, and is testable even with the deliberately weak hashing embedder.
        let fixture = Fixture::new(&[
            ("beide.txt", "Die Montage der Kamera dauerte drei Stunden.\n"),
            ("nur-montage.txt", "Die Montage begann am Dienstag und war schnell erledigt.\n"),
        ])
        .with_vectors();

        let outcome = fixture.search("montage kamera");
        let partial = outcome
            .hits
            .iter()
            .find(|h| h.path.ends_with("nur-montage.txt"))
            .expect("the partially matching document must appear at all");

        assert!(
            partial.score.lexical_rank.is_none(),
            "the fixture is wrong: the lexical arm found it after all"
        );
        assert!(
            partial.score.vector_rank.is_some(),
            "it should have been found by the vector arm"
        );
    }

    #[test]
    fn a_result_found_only_by_meaning_still_gets_a_snippet() {
        // The case where a snippet matters most: none of the query words appear in the text, so
        // the path alone says nothing about why the result is there.
        let fixture = Fixture::new(&[(
            "nur-montage.txt",
            "Die Montage begann am Dienstag und war schnell erledigt.\n",
        )])
        .with_vectors();

        let outcome = fixture.search("montage kamera");
        let hit = &outcome.hits[0];
        assert!(hit.score.vector_rank.is_some());
        assert!(
            hit.snippet.as_deref().is_some_and(|s| s.contains("Montage")),
            "expected a snippet from the file, got {:?}",
            hit.snippet
        );
    }

    #[test]
    fn without_a_model_the_search_says_so_instead_of_failing() {
        // A fresh install has no model, and that must read as "not running" rather than as
        // "found nothing" — the two have completely different fixes.
        let fixture = Fixture::new(&[("notiz.txt", "Die Blickachse bleibt konstant.\n")]);
        let outcome = fixture.search("blickachse");

        assert!(!outcome.hits.is_empty(), "the other arms must still answer");
        assert_eq!(outcome.vector_candidates, 0);
        assert!(
            outcome
                .vector_note
                .as_deref()
                .is_some_and(|n| n.contains("cortex model fetch")),
            "the note must name the fix: {:?}",
            outcome.vector_note
        );
    }

    #[test]
    fn locate_mode_does_not_pay_for_the_vector_arm() {
        // Embedding a query costs milliseconds. The ninety-per-cent case is a path lookup, and
        // it must not wait for a model.
        let fixture = Fixture::new(&[("notizen.md", "Inhalt.\n")]).with_vectors();
        let outcome = fixture.search_as(QueryMode::Locate, "notizen.md");
        assert_eq!(outcome.vector_candidates, 0);
        assert!(outcome.vector_note.is_none(), "locate must not even explain itself");
    }

    #[test]
    fn an_index_from_another_model_is_refused_rather_than_compared() {
        // Vectors from two models are numbers in unrelated spaces. Comparing them produces
        // confident nonsense, which is worse than an empty arm and much harder to notice.
        let fixture = Fixture::new(&[("notiz.txt", "Die Blickachse bleibt konstant.\n")])
            .with_vectors();

        let mismatched = VectorIndex::open(&fixture.tmp.path().join("other.usearch"), 16).unwrap();
        mismatched.add(cortex_core::ids::ChunkId(1), &[0.25; 16]).unwrap();

        let outcome = run(Params {
            store: &fixture.store,
            index: &fixture.index,
            semantic: Some(Semantic {
                embedder: &fixture.embedder,
                vectors: &mismatched,
            }),
            requested_mode: QueryMode::Auto,
            raw_query: "blickachse",
            limit: 10,
            prefix: true,
            candidates_per_arm: 100,
            rrf_k: 60.0,
            max_salience_boost: 1.5,
        });

        assert_eq!(outcome.vector_candidates, 0);
        assert!(
            outcome.vector_note.as_deref().is_some_and(|n| n.contains("dimensions")),
            "{:?}",
            outcome.vector_note
        );
        assert!(!outcome.hits.is_empty(), "the other arms must still answer");
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
