//! The context pack: memory assembled for a context window rather than for a result list.
//!
//! Every other read tool in this system answers with rows. An agent does not have rows, it has
//! a context window — and turning twenty results into two thousand tokens worth having means
//! dropping the restatements, deciding what fits, keeping the provenance and formatting the
//! rest. Done in the agent that is done without the scores or the vectors; done here it is done
//! once, with both. D77 is the argument.
//!
//! ## The shape of one pack
//!
//! 1. **Preferences and pins, unconditionally.** They are three words long and their whole
//!    value is being present. When they do not fit, the pack *says so* rather than dropping
//!    them quietly: an agent that silently loses "never run this without a dry run" is worse
//!    off than one that got an error.
//! 2. **The working set** — what the human is doing right now.
//! 3. **Memories**, lexical ∪ semantic, fused by RRF and then diversified by MMR.
//! 4. **The files those memories are about**, as a path and a byte range. Not content: the
//!    agent has `fs.read` and now knows exactly which bytes to ask for.
//!
//! Every item carries a basis — which arm found it, what it scored, who wrote it and when.
//!
//! ## Two things this module deliberately does not do
//!
//! **It does not need a model.** Rule R5 means a fresh install has no embedder, and a context
//! tool that only works after a model download is a context tool that does not work on the day
//! somebody tries it. Without one the semantic arm is absent and diversification falls back to
//! character trigrams, which is worse and is *reported as* worse in the pack's notes.
//!
//! **It does not decide what a memory means.** No summarising, no merging, no rewriting. What
//! comes out is what was written, attributed to whoever wrote it.

use cortex_core::ids::{ChunkId, NodeId};
use cortex_core::memory::{
    Basis, ContextPack, Memory, MemoryKind, PackArm, PackItem, ReaderScope,
};
use cortex_core::tokens;
use cortex_embed::{Embedder, Input};
use cortex_store::embeddings::ModelRef;
use cortex_store::memories::{MemoryFilter, PackItemRecord};
use cortex_store::vectors::VectorIndex;
use cortex_store::Store;
use std::collections::{HashMap, HashSet};

/// How many candidates each arm contributes before fusion.
pub const DEFAULT_CANDIDATES: usize = 60;
/// The MMR trade-off. 1.0 is pure relevance; 0.0 is pure novelty.
pub const DEFAULT_LAMBDA: f64 = 0.7;
/// RRF's damping constant, the same one the file search uses.
pub const DEFAULT_RRF_K: f64 = 60.0;
/// How long ago still counts as "right now" for the working set.
pub const WORKING_SET_WINDOW_SECS: i64 = 4 * 60 * 60;
/// How many unconditional memories are considered. A preference list longer than this is not a
/// preference list.
const MAX_UNCONDITIONAL: usize = 32;

/// The vector arm's two halves, over the **memory** collection.
#[derive(Clone, Copy)]
pub struct Semantic<'a> {
    pub embedder: &'a dyn Embedder,
    /// The memory collection, not the file one (D75).
    pub memory_vectors: &'a VectorIndex,
    pub model: ModelRef<'a>,
}

pub struct ContextParams<'a> {
    pub store: &'a Store,
    pub semantic: Option<Semantic<'a>>,
    /// Who is asking, expanded into the scopes they may see. Built by the daemon from the
    /// caller's identity, never by the caller.
    pub reader: &'a ReaderScope,
    /// What the agent is about to do. The query, in other words, but named for what it is.
    pub task: &'a str,
    pub budget_tokens: usize,
    /// Restrict to these kinds. Empty means all of them.
    pub kinds: &'a [MemoryKind],
    /// Include the files the selected memories are about.
    pub include_files: bool,
    /// Include memories that have been superseded. Off by default, and the whole point of
    /// supersession.
    pub include_history: bool,
    pub now: i64,
    pub candidates: usize,
    pub mmr_lambda: f64,
    pub rrf_k: f64,
    pub chars_per_token: f64,
}

impl<'a> ContextParams<'a> {
    pub fn new(
        store: &'a Store,
        reader: &'a ReaderScope,
        task: &'a str,
        budget_tokens: usize,
        now: i64,
    ) -> Self {
        Self {
            store,
            semantic: None,
            reader,
            task,
            budget_tokens,
            kinds: &[],
            include_files: true,
            include_history: false,
            now,
            candidates: DEFAULT_CANDIDATES,
            mmr_lambda: DEFAULT_LAMBDA,
            rrf_k: DEFAULT_RRF_K,
            chars_per_token: tokens::CHARS_PER_TOKEN,
        }
    }
}

/// The record the store keeps of one pack, so that an outcome reported ten minutes later can
/// be attributed to what the pack actually contained.
///
/// Here rather than as a method on [`ContextPack`] because the record type belongs to the
/// storage layer, and the pack itself is a domain type that the CLI deserialises without
/// linking storage at all.
pub fn records(pack: &ContextPack) -> Vec<PackItemRecord> {
    pack.items
        .iter()
        .enumerate()
        .map(|(rank, item)| PackItemRecord {
            node_id: NodeId(item.node_id),
            rank,
            arm: PackArm::parse(&item.arm).unwrap_or(PackArm::Lexical),
            score: item.basis.fused,
        })
        .collect()
}

/// Assemble a pack.
///
/// `pack_id` is supplied rather than generated here, because this crate has no clock and no
/// randomness and should not acquire either: the daemon knows what time it is.
pub fn build(params: ContextParams<'_>, pack_id: &str) -> ContextPack {
    let mut notes: Vec<String> = Vec::new();
    let visible = params.reader.visible_scopes();

    // The budget is against the *rendered block*, because that is what the caller pays for —
    // and the rendered block carries a title, a heading per section and a bullet per item that
    // no per-item count knows about. Fitting the items to the budget and then rendering them
    // produced a 540-token block for a 512-token budget, which the benchmark caught and no
    // unit test would have: the overshoot is proportional to the number of sections, so it only
    // appears once a pack is full enough to have several.
    //
    // So the furniture is reserved before anything is chosen. Over-reserving costs a line of
    // the pack; under-reserving breaks the promise the budget makes.
    let furniture = furniture_cost(params.task, params.chars_per_token);
    let available = params.budget_tokens.saturating_sub(furniture);
    let mut used = 0usize;
    let mut items: Vec<PackItem> = Vec::new();
    let mut taken: HashSet<i64> = HashSet::new();
    let mut dropped = 0usize;

    // ---- 1. The unconditional layer -------------------------------------------------------
    let unconditional = unconditional_memories(&params, &visible);
    let mut unconditional_dropped = 0usize;
    for memory in unconditional {
        let item = memory_item(&memory, PackArm::Unconditional, 0.0, None, None, params.chars_per_token);
        if used + item.tokens > available {
            unconditional_dropped += 1;
            dropped += 1;
            continue;
        }
        used += item.tokens;
        taken.insert(item.node_id);
        items.push(item);
    }
    if unconditional_dropped > 0 {
        notes.push(format!(
            "{unconditional_dropped} preference(s) or pinned memories did not fit in \
             {} tokens and were left out — the budget is too small for them",
            params.budget_tokens
        ));
    }

    // ---- 2. The working set ---------------------------------------------------------------
    let working = working_set_items(&params);
    for item in working {
        if used + item.tokens > available {
            dropped += 1;
            continue;
        }
        used += item.tokens;
        taken.insert(item.node_id);
        items.push(item);
    }

    // ---- 3. Memories, fused and then diversified ------------------------------------------
    let (candidates, arm_note) = memory_candidates(&params, &visible);
    if let Some(note) = arm_note {
        notes.push(note);
    }

    let ranked = diversify(&params, candidates);
    for scored in ranked {
        if taken.contains(&scored.memory.node_id) {
            continue;
        }
        let item = memory_item(
            &scored.memory,
            scored.arm,
            scored.fused,
            scored.lexical_rank,
            scored.semantic_rank,
            params.chars_per_token,
        );
        if used + item.tokens > available {
            dropped += 1;
            continue;
        }
        used += item.tokens;
        taken.insert(item.node_id);
        items.push(item);
    }

    // ---- 4. The files those memories are about --------------------------------------------
    if params.include_files {
        for item in subject_items(&params, &items, &taken) {
            if used + item.tokens > available {
                dropped += 1;
                continue;
            }
            used += item.tokens;
            taken.insert(item.node_id);
            items.push(item);
        }
    }

    // The reservation above should make this exact, and "should" is not a guarantee anybody can
    // rely on. So the block is measured and trimmed from the end until it fits — which makes
    // the promise true by construction rather than by arithmetic that a later change can drift
    // away from. In practice this loop runs zero times; the cost of it running is one render of
    // a pack that is at most a few dozen lines.
    let mut rendered = render(params.task, &items);
    let mut used_tokens = tokens::estimate_with(&rendered, params.chars_per_token);
    while used_tokens > params.budget_tokens && !items.is_empty() {
        items.pop();
        dropped += 1;
        rendered = render(params.task, &items);
        used_tokens = tokens::estimate_with(&rendered, params.chars_per_token);
    }
    if items.is_empty() && used_tokens > params.budget_tokens {
        // Not even the title fits. Answer with nothing rather than with something over budget.
        rendered = String::new();
        used_tokens = 0;
        notes.push(format!(
            "a budget of {} tokens is too small for any pack at all",
            params.budget_tokens
        ));
    }

    ContextPack {
        pack_id: pack_id.to_string(),
        task: params.task.to_string(),
        budget_tokens: params.budget_tokens,
        used_tokens,
        items,
        dropped,
        notes,
        rendered,
    }
}

// ---------------------------------------------------------------------------
// The layers
// ---------------------------------------------------------------------------

fn unconditional_memories(params: &ContextParams<'_>, visible: &[String]) -> Vec<Memory> {
    let filter = MemoryFilter {
        scopes: Some(visible.to_vec()),
        kinds: Vec::new(),
        include_superseded: params.include_history,
        include_expired: false,
        subject: None,
        session_id: None,
        limit: 500,
        now: params.now,
    };
    let all = params.store.list_memories(&filter).unwrap_or_default();
    all.into_iter()
        .filter(|m| m.kind.is_unconditional() || m.pinned)
        .take(MAX_UNCONDITIONAL)
        .collect()
}

fn working_set_items(params: &ContextParams<'_>) -> Vec<PackItem> {
    let since = params.now - WORKING_SET_WINDOW_SECS;
    let touched = params.store.recently_touched(since, 12).unwrap_or_default();
    touched
        .into_iter()
        .filter_map(|t| {
            let path = t.path?;
            let text = format!("{} ({})", path.display(), t.actions.join(", "));
            let cost = tokens::estimate_with(&text, params.chars_per_token);
            Some(PackItem {
                node_id: t.node_id,
                arm: PackArm::WorkingSet.as_str().to_string(),
                kind: None,
                scope: None,
                text,
                path: Some(path),
                byte_range: None,
                tokens: cost,
                basis: Basis {
                    arm: PackArm::WorkingSet.as_str().to_string(),
                    lexical_rank: None,
                    semantic_rank: None,
                    fused: 0.0,
                    salience: 0.0,
                    usefulness: 0.0,
                    confidence: 1.0,
                    actor: "human".into(),
                    written_at: t.last_touched,
                    helped: 0,
                    noise: 0,
                    retrieved: 0,
                    human_confirmed: false,
                },
            })
        })
        .collect()
}

struct Candidate {
    memory: Memory,
    chunk: Option<ChunkId>,
    arm: PackArm,
    fused: f64,
    lexical_rank: Option<u32>,
    semantic_rank: Option<u32>,
}

fn memory_candidates(
    params: &ContextParams<'_>,
    visible: &[String],
) -> (Vec<Candidate>, Option<String>) {
    let mut note = None;

    // The lexical arm.
    let mut lexical: Vec<NodeId> = Vec::new();
    let mut chunk_of: HashMap<i64, ChunkId> = HashMap::new();
    if let Some(expr) = crate::lexical::task_expression(params.task) {
        match params.store.search_memories_with(
            &expr,
            visible,
            params.now,
            params.candidates,
            params.include_history,
        )
        {
            Ok(hits) => {
                for hit in hits {
                    chunk_of.insert(hit.node_id.0, hit.chunk_id);
                    lexical.push(hit.node_id);
                }
            }
            Err(e) => tracing::warn!(error = %e, "the memory lexical arm failed"),
        }
    }

    // The semantic arm, over the memory collection only.
    let mut semantic: Vec<NodeId> = Vec::new();
    match params.semantic {
        None => {
            note = Some(
                "no embedding model is loaded, so this pack was assembled from the lexical arm \
                 alone and diversified by character overlap rather than by meaning"
                    .to_string(),
            );
        }
        Some(sem) => match sem.embedder.embed(&[Input::Query(params.task)]) {
            Ok(vectors) if !vectors.is_empty() => {
                match sem.memory_vectors.search(&vectors[0], params.candidates) {
                    Ok(hits) => {
                        for hit in hits {
                            if let Ok(Some(node)) = params.store.memory_for_chunk(hit.chunk_id) {
                                chunk_of.insert(node.0, hit.chunk_id);
                                semantic.push(node);
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "the memory vector arm failed"),
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, "embedding the task failed");
                note = Some("the task could not be embedded; lexical arm only".to_string());
            }
        },
    }

    let rankings = vec![
        crate::fusion::Ranking {
            arm: crate::fusion::Arm::Lexical,
            nodes: lexical,
        },
        crate::fusion::Ranking {
            arm: crate::fusion::Arm::Vector,
            nodes: semantic,
        },
    ];
    let fused = crate::fusion::fuse(&rankings, params.rrf_k);

    // Loading the rows one at a time is right here and would not be for the file search: the
    // fused list is at most `2 * candidates` long, and every one of them is about to be
    // rendered anyway.
    let mut out = Vec::with_capacity(fused.len());
    for entry in fused {
        let Ok(Some(memory)) = params.store.memory(entry.node_id) else {
            continue;
        };
        if !memory.is_live(params.now) && !params.include_history {
            continue;
        }
        if !params.kinds.is_empty() && !params.kinds.contains(&memory.kind) {
            continue;
        }
        // Scope is filtered in SQL by the lexical arm, but the vector arm reaches chunks
        // directly and would happily return another agent's memory. Checked again here, and
        // this is the check that matters.
        if !memory.scope.visible_from(params.reader) {
            continue;
        }
        let arm = match (entry.lexical_rank, entry.vector_rank) {
            (Some(_), Some(_)) | (Some(_), None) => PackArm::Lexical,
            (None, Some(_)) => PackArm::Semantic,
            (None, None) => PackArm::Lexical,
        };
        let chunk = chunk_of.get(&memory.node_id).copied();
        out.push(Candidate {
            memory,
            chunk,
            arm,
            fused: entry.fused,
            lexical_rank: entry.lexical_rank,
            semantic_rank: entry.vector_rank,
        });
    }
    (out, note)
}

/// Maximal Marginal Relevance over the fused candidates.
///
/// The failure this exists for is restatement. People and agents say the same thing more than
/// once, in slightly different words, and an embedding retriever ranks all of them highly for
/// the same query — so three slots buy one fact. λ trades relevance against novelty; 0.7 keeps
/// the ordering recognisable while dropping the near-copies.
///
/// The similarity is the embedding when there is one and character trigrams when there is not.
/// The fallback is genuinely worse and the pack says so, but it is much better than nothing:
/// two phrasings of one sentence overlap heavily as character trigrams too.
///
/// ## Two things about the cost, both of which the benchmark had to find
///
/// MMR is quadratic in the candidate count, and the first version made that much worse than it
/// had to be: it built both trigram sets *inside* the comparison, so a hundred and twenty
/// candidates meant fourteen thousand set constructions over the same sixty strings. The gate
/// measured 163 ms against a budget of 120, and the same pack with diversification switched off
/// measured 7 ms — which is what said the problem was here and not in retrieval. The sets are
/// built once per candidate now.
///
/// The second is that ranking every candidate is pointless when a dozen will fit. The loop
/// stops after [`MMR_PICKS`] and the remainder keeps its fused order, which is the order it
/// would have been dropped in anyway.
fn diversify(params: &ContextParams<'_>, candidates: Vec<Candidate>) -> Vec<Candidate> {
    if candidates.len() < 2 {
        return candidates;
    }
    let lambda = params.mmr_lambda.clamp(0.0, 1.0);
    if lambda >= 1.0 {
        return candidates;
    }

    // Embeddings for the candidates, when there is a model. Absent is normal: a fresh install
    // has no model (R5), and the trigram fallback below is what makes this work anyway.
    let vectors: Vec<Option<Vec<f32>>> = match params.semantic {
        Some(sem) => {
            let chunks: Vec<ChunkId> = candidates.iter().filter_map(|c| c.chunk).collect();
            let by_chunk = params
                .store
                .embeddings_for_chunks(sem.model, &chunks)
                .unwrap_or_default();
            candidates
                .iter()
                .map(|c| c.chunk.and_then(|chunk| by_chunk.get(&chunk).cloned()))
                .collect()
        }
        None => vec![None; candidates.len()],
    };
    // Built once per candidate rather than once per comparison. This is the difference between
    // 163 ms and single digits.
    let trigrams: Vec<HashSet<[char; 3]>> = candidates
        .iter()
        .map(|c| trigrams_of(&c.memory.text))
        .collect();

    // The best fused score, to put relevance and similarity on the same scale. Without it λ
    // means nothing: RRF scores are around 1/60 and cosine similarity is around 1.
    let best = candidates
        .iter()
        .map(|c| c.fused)
        .fold(f64::MIN, f64::max)
        .max(f64::MIN_POSITIVE);

    let picks = MMR_PICKS.min(candidates.len());
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    let mut order: Vec<usize> = Vec::with_capacity(candidates.len());
    // The highest similarity each remaining candidate has to anything already chosen, carried
    // forward instead of recomputed: one pass per pick rather than one per pick per choice.
    let mut redundancy = vec![0.0f64; candidates.len()];

    for _ in 0..picks {
        let mut best_at = 0usize;
        let mut best_score = f64::MIN;
        for (position, &index) in remaining.iter().enumerate() {
            let relevance = candidates[index].fused / best;
            let score = lambda * relevance - (1.0 - lambda) * redundancy[index];
            if score > best_score {
                best_score = score;
                best_at = position;
            }
        }
        let chosen = remaining.remove(best_at);
        order.push(chosen);
        for &index in &remaining {
            let similarity = match (&vectors[index], &vectors[chosen]) {
                (Some(left), Some(right)) => cosine(left, right),
                _ => jaccard(&trigrams[index], &trigrams[chosen]),
            };
            if similarity > redundancy[index] {
                redundancy[index] = similarity;
            }
        }
    }
    // Whatever was not ranked keeps the order fusion gave it. These are the candidates that
    // will not fit; putting them in a defined order costs nothing and makes the pack's tail
    // reproducible.
    order.extend(remaining);

    let mut taken: Vec<Option<Candidate>> = candidates.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| taken[index].take())
        .collect()
}

/// How many candidates the diversification actually ranks.
///
/// Forty: comfortably more than any budget this system has fits, and small enough that the
/// quadratic term is forty times forty rather than a hundred and twenty times a hundred and
/// twenty. Beyond it the fused order is kept, because those candidates are being dropped.
const MMR_PICKS: usize = 40;

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    for (x, y) in a.iter().zip(b) {
        dot += (*x as f64) * (*y as f64);
        left += (*x as f64) * (*x as f64);
        right += (*y as f64) * (*y as f64);
    }
    if left == 0.0 || right == 0.0 {
        return 0.0;
    }
    (dot / (left.sqrt() * right.sqrt())).clamp(-1.0, 1.0)
}

fn trigrams_of(text: &str) -> HashSet<[char; 3]> {
    let chars: Vec<char> = text
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// Jaccard overlap of two trigram sets: shared over union.
///
/// The union is computed from the two sizes and the intersection rather than by building a
/// third set, which is the whole reason the sets are precomputed.
fn jaccard(left: &HashSet<[char; 3]>, right: &HashSet<[char; 3]>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    // Intersect the smaller against the larger: the cost is the size of the one being walked.
    let (small, large) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let shared = small.iter().filter(|t| large.contains(*t)).count() as f64;
    let union = (left.len() + right.len()) as f64 - shared;
    if union == 0.0 {
        return 0.0;
    }
    shared / union
}

fn subject_items(
    params: &ContextParams<'_>,
    items: &[PackItem],
    taken: &HashSet<i64>,
) -> Vec<PackItem> {
    let mut out = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    for item in items {
        let Some(path) = item.path.as_ref() else {
            continue;
        };
        // Only the subjects of memories, not the working set's own paths.
        if item.kind.is_none() {
            continue;
        }
        let node = match params.store.node_by_path(path) {
            Ok(Some(node)) => node,
            _ => continue,
        };
        if taken.contains(&node.id.0) || !seen.insert(node.id.0) {
            continue;
        }
        let text = format!("{}", path.display());
        let cost = tokens::estimate_with(&text, params.chars_per_token);
        out.push(PackItem {
            node_id: node.id.0,
            arm: PackArm::Subject.as_str().to_string(),
            kind: None,
            scope: None,
            text,
            path: Some(path.clone()),
            byte_range: None,
            tokens: cost,
            basis: Basis {
                arm: PackArm::Subject.as_str().to_string(),
                lexical_rank: None,
                semantic_rank: None,
                fused: 0.0,
                salience: node.salience,
                usefulness: 0.0,
                confidence: 1.0,
                actor: "system".into(),
                written_at: node.last_seen,
                helped: 0,
                noise: 0,
                retrieved: 0,
                human_confirmed: false,
            },
        });
    }
    out
}

fn memory_item(
    memory: &Memory,
    arm: PackArm,
    fused: f64,
    lexical_rank: Option<u32>,
    semantic_rank: Option<u32>,
    chars_per_token: f64,
) -> PackItem {
    let cost = tokens::estimate_with(&memory.text, chars_per_token) + PER_ITEM_OVERHEAD;
    PackItem {
        node_id: memory.node_id,
        arm: arm.as_str().to_string(),
        kind: Some(memory.kind),
        scope: Some(memory.scope.clone()),
        text: memory.text.clone(),
        path: memory.subject_path.clone(),
        byte_range: None,
        tokens: cost,
        basis: Basis {
            arm: arm.as_str().to_string(),
            lexical_rank,
            semantic_rank,
            fused,
            salience: memory.salience,
            usefulness: memory.usefulness,
            confidence: memory.confidence,
            actor: memory.actor.clone(),
            written_at: memory.ts,
            helped: memory.helped_count,
            noise: memory.noise_count,
            retrieved: memory.retrieved_count,
            human_confirmed: memory.human_confirmed,
        },
    }
}

/// What one rendered line costs beyond its text: the bullet, the kind label and the newline.
const PER_ITEM_OVERHEAD: usize = 6;

/// The headings and the title, which every pack pays for before it holds anything.
///
/// Every section is reserved whether or not it appears. The alternative is circular — which
/// headings appear depends on which items fit, which depends on the reservation — and paying
/// for four headings that turn out to be empty costs a line of the pack, while the circularity
/// costs the promise the budget makes.
fn furniture_cost(task: &str, chars_per_token: f64) -> usize {
    let mut sample = String::from("# What the memory knows about: ");
    sample.push_str(task);
    sample.push('\n');
    for heading in [
        "Always",
        "Open right now",
        "Relevant",
        "Related",
        "Files these are about",
    ] {
        sample.push_str("\n## ");
        sample.push_str(heading);
        sample.push('\n');
    }
    tokens::estimate_with(&sample, chars_per_token)
}

/// Format the pack.
///
/// Plain text with headings rather than JSON, because this goes into a prompt and JSON in a
/// prompt costs punctuation tokens for structure a language model does not need. The structured
/// form is still there — it is [`ContextPack::items`] — for anything that wants to render it
/// itself.
fn render(task: &str, items: &[PackItem]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("# What the memory knows about: ");
    out.push_str(task);
    out.push('\n');

    let sections: [(PackArm, &str); 5] = [
        (PackArm::Unconditional, "Always"),
        (PackArm::WorkingSet, "Open right now"),
        (PackArm::Lexical, "Relevant"),
        (PackArm::Semantic, "Related"),
        (PackArm::Subject, "Files these are about"),
    ];

    for (arm, heading) in sections {
        let matching: Vec<&PackItem> = items
            .iter()
            .filter(|item| item.arm == arm.as_str())
            .collect();
        if matching.is_empty() {
            continue;
        }
        out.push_str("\n## ");
        out.push_str(heading);
        out.push('\n');
        for item in matching {
            out.push_str("- ");
            if let Some(kind) = item.kind {
                out.push('[');
                out.push_str(kind.as_str());
                out.push_str("] ");
            }
            out.push_str(item.text.trim());
            if let Some(path) = &item.path {
                if item.kind.is_some() {
                    out.push_str(" — about ");
                    out.push_str(&path.display().to_string());
                }
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::memory::MemoryScope;
    use cortex_core::model::Actor;
    use cortex_store::memories::NewMemory;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn write(store: &Store, scope: MemoryScope, kind: MemoryKind, text: &str) -> NodeId {
        store
            .remember(
                &NewMemory::new(scope, kind, text),
                &Actor::Agent("a".into()),
                100,
            )
            .unwrap()
            .node_id
    }

    fn pack(store: &Store, reader: &ReaderScope, task: &str, budget: usize) -> ContextPack {
        build(ContextParams::new(store, reader, task, budget, 200), "p1")
    }

    #[test]
    fn a_pack_carries_the_memory_and_its_basis() {
        let store = store();
        write(
            &store,
            MemoryScope::Global,
            MemoryKind::Procedure,
            "the hygiene gate runs before every commit and must not be skipped",
        );
        let reader = ReaderScope::for_agent("a");
        // The lexical arm has no stemmer, so the task has to share a token with the memory.
        // That is a real limit of a pack assembled without a model, and the pack says so in
        // its notes rather than this test pretending otherwise.
        let pack = pack(&store, &reader, "hygiene gate", 2048);

        assert_eq!(pack.items.len(), 1);
        let item = &pack.items[0];
        assert_eq!(item.kind, Some(MemoryKind::Procedure));
        assert_eq!(item.basis.actor, "agent:a");
        assert!(item.basis.lexical_rank.is_some());
        assert!(pack.rendered.contains("hygiene gate"));
        assert!(pack.rendered.contains("[procedure]"));
    }

    #[test]
    fn a_preference_is_in_every_pack_whether_or_not_it_matches() {
        let store = store();
        write(
            &store,
            MemoryScope::Global,
            MemoryKind::Preference,
            "answer in German unless asked otherwise",
        );
        write(
            &store,
            MemoryScope::Global,
            MemoryKind::Fact,
            "the watcher debounces for three hundred milliseconds",
        );
        let reader = ReaderScope::for_agent("a");

        // A task with nothing to do with either.
        let pack = pack(&store, &reader, "unrelated aardvark taxonomy", 2048);
        let texts: Vec<&str> = pack.items.iter().map(|i| i.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("German")),
            "the preference was not unconditional"
        );
        assert!(
            !texts.iter().any(|t| t.contains("debounces")),
            "an unrelated fact was included anyway"
        );
    }

    /// The one case that must not fail quietly: a budget too small for the preferences.
    #[test]
    fn a_budget_too_small_for_the_preferences_says_so() {
        let store = store();
        for index in 0..6 {
            write(
                &store,
                MemoryScope::Global,
                MemoryKind::Preference,
                &format!("preference number {index}, stated at some length so that it costs a few tokens"),
            );
        }
        let reader = ReaderScope::for_agent("a");
        let pack = pack(&store, &reader, "anything", 20);

        assert!(pack.dropped > 0);
        assert!(
            pack.notes.iter().any(|n| n.contains("budget is too small")),
            "preferences were dropped without a word: {:?}",
            pack.notes
        );
    }

    /// Definition of Done §3, at the three budgets it names.
    #[test]
    fn the_budget_is_honoured_at_every_size() {
        let store = store();
        for index in 0..300 {
            write(
                &store,
                MemoryScope::Global,
                MemoryKind::Fact,
                &format!(
                    "fact number {index} about the indexing pipeline, the watcher, the chunker \
                     and the embedder, stated in a full sentence"
                ),
            );
        }
        let reader = ReaderScope::for_agent("a");
        for budget in [512usize, 2048, 8192] {
            let pack = pack(&store, &reader, "the indexing pipeline", budget);
            assert!(
                pack.used_tokens <= budget,
                "a {budget}-token budget produced {} tokens",
                pack.used_tokens
            );
            assert!(!pack.items.is_empty(), "a {budget}-token budget produced nothing");
            // And no item was cut in half.
            for item in &pack.items {
                assert!(!item.text.is_empty());
            }
        }
    }

    #[test]
    fn a_bigger_budget_carries_more() {
        let store = store();
        for index in 0..100 {
            write(
                &store,
                MemoryScope::Global,
                MemoryKind::Fact,
                &format!("a distinct fact numbered {index} about the pipeline and its stages"),
            );
        }
        let reader = ReaderScope::for_agent("a");
        let small = pack(&store, &reader, "pipeline stages", 256);
        let large = pack(&store, &reader, "pipeline stages", 4096);
        assert!(large.items.len() > small.items.len());
    }

    /// Definition of Done §5. λ = 1.0 is pure relevance and lets the restatements through;
    /// λ = 0.7 must not, and must still find the distinct facts.
    #[test]
    fn diversification_drops_restatements_without_losing_the_distinct_facts() {
        let store = store();
        // Five ways of saying one thing…
        for phrasing in [
            "the watcher debounces filesystem events for three hundred milliseconds",
            "filesystem events are debounced by the watcher for three hundred milliseconds",
            "the watcher waits three hundred milliseconds before acting on an event",
            "events from the filesystem are held three hundred milliseconds by the watcher",
            "debouncing in the watcher is three hundred milliseconds for filesystem events",
        ] {
            write(&store, MemoryScope::Global, MemoryKind::Fact, phrasing);
        }
        // …and three distinct things that also mention the watcher.
        for other in [
            "the watcher is inotify based and reconciles after a blind period",
            "the watcher assigns a zone to a new file before the next reconcile",
            "the watcher survives a move by matching device and inode",
        ] {
            write(&store, MemoryScope::Global, MemoryKind::Fact, other);
        }

        let reader = ReaderScope::for_agent("a");
        // Big enough that the greedy pack has room for several restatements — the whole point
        // is what it spends the room on. A budget that fits one item measures nothing, which is
        // what this test did before the furniture was reserved out of the budget.
        let budget = 200;
        let greedy = build(
            ContextParams {
                mmr_lambda: 1.0,
                ..ContextParams::new(&store, &reader, "the watcher", budget, 200)
            },
            "greedy",
        );
        let diverse = build(
            ContextParams {
                mmr_lambda: DEFAULT_LAMBDA,
                ..ContextParams::new(&store, &reader, "the watcher", budget, 200)
            },
            "diverse",
        );

        let duplicates = |pack: &ContextPack| {
            pack.items
                .iter()
                .filter(|i| i.text.contains("three hundred milliseconds"))
                .count()
        };
        let distinct = |pack: &ContextPack| {
            pack.items
                .iter()
                .filter(|i| !i.text.contains("three hundred milliseconds"))
                .count()
        };

        // The Definition of Done asks for both numbers to be recorded, not only for the
        // inequality to hold — so they are pinned here. A change that makes diversification
        // more or less aggressive should have to edit this line and say why.
        assert_eq!(
            (duplicates(&greedy), distinct(&greedy)),
            (4, 1),
            "the greedy pack changed shape; λ = 1.0 is pure relevance and spends its slots on \
             restatements"
        );
        assert_eq!(
            (duplicates(&diverse), distinct(&diverse)),
            (2, 3),
            "the diversified pack changed shape; λ = 0.7 should keep the fact once and spend \
             the rest on the distinct ones"
        );
        assert!(
            duplicates(&diverse) < duplicates(&greedy),
            "diversification kept {} restatements against {}",
            duplicates(&diverse),
            duplicates(&greedy)
        );
        assert!(
            distinct(&diverse) >= distinct(&greedy),
            "diversification lost distinct facts: {} against {}",
            distinct(&diverse),
            distinct(&greedy)
        );
        assert!(
            duplicates(&diverse) >= 1,
            "the fact itself was dropped, not just its restatements"
        );
    }

    #[test]
    fn a_pack_never_crosses_into_another_agents_scope() {
        let store = store();
        write(
            &store,
            MemoryScope::Agent("b".into()),
            MemoryKind::Fact,
            "the deployment key lives in the vault",
        );
        write(
            &store,
            MemoryScope::Agent("a".into()),
            MemoryKind::Fact,
            "the deployment target is staging",
        );
        let pack = pack(&store, &ReaderScope::for_agent("a"), "deployment", 4096);
        let texts: Vec<&str> = pack.items.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.contains("staging")));
        assert!(
            !texts.iter().any(|t| t.contains("vault")),
            "another agent's memory reached the pack"
        );
    }

    #[test]
    fn a_superseded_memory_is_absent_unless_history_is_asked_for() {
        let store = store();
        let old = write(
            &store,
            MemoryScope::Global,
            MemoryKind::Fact,
            "the budget for the keystroke path is fifty milliseconds",
        );
        let new = write(
            &store,
            MemoryScope::Global,
            MemoryKind::Fact,
            "the budget for the keystroke path is eighty milliseconds",
        );
        store
            .supersede_memory(old, new, &Actor::Agent("a".into()), 150)
            .unwrap();

        let reader = ReaderScope::for_agent("a");
        let current = pack(&store, &reader, "keystroke budget", 4096);
        assert!(current.items.iter().all(|i| i.node_id != old.0));
        assert!(current.items.iter().any(|i| i.node_id == new.0));

        let historic = build(
            ContextParams {
                include_history: true,
                ..ContextParams::new(&store, &reader, "keystroke budget", 4096, 200)
            },
            "h",
        );
        assert!(historic.items.iter().any(|i| i.node_id == old.0));
    }

    #[test]
    fn an_empty_memory_layer_produces_an_empty_pack_and_not_an_error() {
        let store = store();
        let pack = pack(&store, &ReaderScope::for_agent("a"), "anything at all", 2048);
        assert!(pack.items.is_empty());
        assert_eq!(pack.used_tokens, 0);
        assert!(pack.rendered.is_empty());
    }

    #[test]
    fn without_a_model_the_pack_says_which_arm_it_used() {
        let store = store();
        write(&store, MemoryScope::Global, MemoryKind::Fact, "one thing");
        write(&store, MemoryScope::Global, MemoryKind::Fact, "another thing");
        let pack = pack(&store, &ReaderScope::for_agent("a"), "thing", 2048);
        assert!(
            pack.notes.iter().any(|n| n.contains("no embedding model")),
            "a lexical-only pack did not admit it: {:?}",
            pack.notes
        );
    }

    #[test]
    fn the_records_a_pack_leaves_behind_match_what_it_returned() {
        let store = store();
        write(&store, MemoryScope::Global, MemoryKind::Fact, "the first thing");
        write(&store, MemoryScope::Global, MemoryKind::Fact, "the second thing");
        let pack = pack(&store, &ReaderScope::for_agent("a"), "thing", 2048);

        let records = records(&pack);
        assert_eq!(records.len(), pack.items.len());
        for (record, item) in records.iter().zip(&pack.items) {
            assert_eq!(record.node_id.0, item.node_id);
        }
        // Only the memories are reportable; the working set and file rows are not.
        assert_eq!(pack.memory_ids().len(), 2);
    }
}
