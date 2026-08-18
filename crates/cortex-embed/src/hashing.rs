//! An embedder that needs nothing.
//!
//! The hashing trick: each token is hashed into one of `dim` buckets and its weight added there,
//! then the vector is normalised. It is a real method — it was how large-scale text
//! classification worked before dense models — and it is genuinely weak: it captures lexical
//! overlap and nothing else. Two documents that say the same thing in different words look
//! unrelated to it, which is precisely what the real model is for.
//!
//! It exists for three reasons, none of them "a mock":
//!
//! 1. **The pipeline can be tested end to end without half a gigabyte of model.** Every test
//!    that asks "does a vector hit reach fusion, ranking and the result list" runs anywhere.
//! 2. **A fresh install has a working vector arm before anybody downloads anything** (rule R5).
//!    Weak retrieval is a better default than a broken pipeline that starts working later.
//! 3. **It is a control.** When the goldset measures the real model, the same goldset run
//!    against this embedder says how much of the recall is the model and how much was already
//!    there from lexical overlap.
//!
//! The sign trick — a second hash decides whether a token adds or subtracts — is from the
//! original feature-hashing paper, and it matters: without it, collisions only ever inflate
//! similarity, so unrelated documents drift together as the vocabulary grows.

use crate::{normalise, Embedder, Input, ModelId, Result};

/// Dimensions of the hashed space.
///
/// The same 384 the real model uses, so that switching between the two changes the vectors and
/// nothing else: the schema, the index and the budget all stay put.
pub const DIM: usize = 384;

#[derive(Debug, Default)]
pub struct HashingEmbedder;

impl HashingEmbedder {
    pub fn new() -> Self {
        Self
    }

    pub const ID: &'static str = "hashing";
    pub const VERSION: &'static str = "1";
}

impl Embedder for HashingEmbedder {
    fn model(&self) -> ModelId {
        ModelId {
            id: Self::ID.to_string(),
            version: Self::VERSION.to_string(),
            dim: DIM,
        }
    }

    fn embed(&self, inputs: &[Input<'_>]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|input| embed_one(input.text())).collect())
    }

    fn max_batch(&self) -> usize {
        // No model, no batching benefit: the cost is linear in tokens either way.
        1024
    }

    fn backend(&self) -> String {
        "hashing (no model)".to_string()
    }
}

fn embed_one(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; DIM];

    for token in tokens(text) {
        // Two *independently mixed* values, not two slices of one hash. FNV's output is not
        // uniform enough to cut in half: taking the bucket from the low bits and the sign from
        // the high bits makes both depend on different parts of the token, and for tokens that
        // share a suffix — `wort1`, `wort2`, … — that lands them in the same bucket with
        // systematically opposite signs. Two unrelated documents then score −0.54 instead of
        // roughly zero, which is a worse failure than the collisions the sign was meant to fix.
        let hash = mix(fnv1a(token.as_bytes()));
        let bucket = (hash % DIM as u64) as usize;
        let sign = if mix(hash ^ SIGN_SEED) & 1 == 0 { 1.0 } else { -1.0 };
        vector[bucket] += sign;
    }

    normalise(&mut vector);
    vector
}

/// Lowercased alphanumeric runs. The same shape of tokenisation the lexical arm uses, so the
/// two arms disagree about ranking rather than about what a word is.
fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
}

/// An arbitrary constant to make the second mix independent of the first.
const SIGN_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// splitmix64's finalizer. Cheap, and it avalanches — every input bit affects every output bit,
/// which is exactly the property FNV lacks and this construction needs.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::similarity;

    fn embed(text: &str) -> Vec<f32> {
        HashingEmbedder::new()
            .embed(&[Input::Passage(text)])
            .unwrap()
            .remove(0)
    }

    #[test]
    fn vectors_are_unit_length_and_the_right_size() {
        let v = embed("Die Blickachse bleibt konstant.");
        assert_eq!(v.len(), DIM);
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn overlapping_text_is_more_similar_than_unrelated_text() {
        // The property that makes this useful as a control rather than a placeholder: it really
        // does rank, just on lexical overlap alone.
        let a = embed("Die Kamerawinkel wurden an drei Montagepositionen verglichen");
        let b = embed("Die Kamerawinkel wurden erneut verglichen");
        let c = embed("Die Speicherkosten stiegen nach der Archivmigration");

        assert!(
            similarity(&a, &b) > similarity(&a, &c),
            "overlap {:.3} should beat unrelated {:.3}",
            similarity(&a, &b),
            similarity(&a, &c)
        );
    }

    #[test]
    fn it_is_deterministic_across_calls_and_processes() {
        // Vectors are persisted. A hash that varied per process — as `DefaultHasher` does —
        // would make every restart disagree with the index it wrote.
        assert_eq!(embed("kamerawinkel"), embed("kamerawinkel"));
        // Pinned against the published FNV-1a 64-bit value for "a", so the constants cannot
        // drift into something that is merely self-consistent.
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
    }

    #[test]
    fn case_and_punctuation_do_not_change_the_vector() {
        assert_eq!(embed("Kamerawinkel!"), embed("kamerawinkel"));
        assert_eq!(embed("a, b."), embed("A B"));
    }

    #[test]
    fn an_empty_text_is_a_zero_vector_rather_than_a_panic() {
        let v = embed("");
        assert_eq!(v.len(), DIM);
        assert!(v.iter().all(|x| *x == 0.0));
        assert!(!v.iter().any(|x| x.is_nan()));
    }

    #[test]
    fn a_batch_comes_back_in_order() {
        let out = HashingEmbedder::new()
            .embed(&[
                Input::Passage("erstes"),
                Input::Passage("zweites"),
                Input::Query("erstes"),
            ])
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], out[2], "this embedder has no query/passage asymmetry");
        assert_ne!(out[0], out[1]);
    }

    #[test]
    fn the_signed_hash_keeps_unrelated_documents_apart() {
        // Two failure modes at once, and this fixture provokes both. Without the sign, every
        // collision adds and long documents drift towards similarity 1. With a sign taken from
        // the same hash as the bucket, tokens that share a suffix collide *and* get opposite
        // signs, which drove this to −0.54 before the two values were mixed independently.
        let long_a: String = (0..400).map(|i| format!("wortaa{i} ")).collect();
        let long_b: String = (0..400).map(|i| format!("wortbb{i} ")).collect();
        let s = similarity(&embed(&long_a), &embed(&long_b));
        assert!(s.abs() < 0.2, "unrelated long documents scored {s:.3}");
    }

    #[test]
    fn buckets_are_spread_rather_than_clustered() {
        // The defect the test above found, stated directly: tokens that differ only in a shared
        // suffix must not all land in the same few buckets.
        let mut used = std::collections::HashSet::new();
        for i in 0..384 {
            let hash = mix(fnv1a(format!("gleicherstamm{i}").as_bytes()));
            used.insert(hash % DIM as u64);
        }
        assert!(
            used.len() > 200,
            "384 distinct tokens occupied only {} of {DIM} buckets",
            used.len()
        );
    }
}
