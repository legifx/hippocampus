//! The real model, when it is on the machine.
//!
//! These tests need half a gigabyte of downloaded model and a runtime library, so they run only
//! when told where to find them:
//!
//! ```sh
//! HIPPOCAMPUS_TEST_RUNTIME=~/.local/state/hippocampus/models/onnxruntime/1.29.0/libonnxruntime.so \
//! HIPPOCAMPUS_TEST_MODEL=~/.local/state/hippocampus/models/multilingual-e5-small/1 \
//!   cargo test -p cortex-embed --test onnx_model -- --nocapture
//! ```
//!
//! They are not `#[ignore]`d, because an ignored test is one nobody runs. They print that they
//! were skipped, which is the honest state: unverified, not passing.
//!
//! What they check is the part of an embedder that fails *quietly*. A wrong prefix, pooling over
//! padding or a missing normalisation all produce plausible vectors and worse recall, and none
//! of them produces an error. The only way to catch them is to assert on the geometry.

#![cfg(feature = "onnx")]

use cortex_embed::onnx::{Device, OnnxEmbedder};
use cortex_embed::{similarity, Embedder, Input, E5_SMALL};
use std::path::PathBuf;

fn embedder() -> Option<OnnxEmbedder> {
    let runtime = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_RUNTIME").ok()?);
    let model = PathBuf::from(std::env::var("HIPPOCAMPUS_TEST_MODEL").ok()?);
    if !runtime.exists() || !model.join("model.onnx").exists() {
        eprintln!("skipped: no model at {}", model.display());
        return None;
    }
    match OnnxEmbedder::load(&runtime, &model, E5_SMALL, Device::Cpu, 4) {
        Ok(embedder) => Some(embedder),
        Err(e) => panic!("the model is present but would not load: {e}"),
    }
}

macro_rules! with_model {
    ($name:ident) => {
        match embedder() {
            Some(e) => e,
            None => {
                eprintln!("skipped {}: set HIPPOCAMPUS_TEST_RUNTIME and HIPPOCAMPUS_TEST_MODEL", stringify!($name));
                return;
            }
        }
    };
}

#[test]
fn vectors_have_the_dimensions_the_registry_promises() {
    let embedder = with_model!(vectors_have_the_dimensions_the_registry_promises);
    let vectors = embedder
        .embed(&[Input::Passage("Die Blickachse bleibt konstant.")])
        .unwrap();

    assert_eq!(vectors[0].len(), E5_SMALL.dim);
    assert_eq!(embedder.model().dim, E5_SMALL.dim);
    let length: f32 = vectors[0].iter().map(|v| v * v).sum();
    assert!(
        (length - 1.0).abs() < 1e-4,
        "vectors must be unit length, got {length}"
    );
}

#[test]
fn batching_does_not_change_a_vector() {
    // The padding test, and the reason it is first among equals. Inputs of different lengths are
    // padded to the longest in the batch; if pooling averages over the padding, a short text
    // embedded next to a long one comes out different from the same text embedded alone. Recall
    // then depends on what happened to be in the same batch, which is untraceable in production
    // and invisible in every test that embeds one thing at a time.
    let embedder = with_model!(batching_does_not_change_a_vector);

    let short = "Kamerawinkel.";
    let long = "Die Kamerawinkel wurden an drei verschiedenen Montagepositionen verglichen, \
                und die Ergebnisse wurden anschliessend in einem laengeren Bericht zusammengefasst, \
                der auch die Abweichungen zwischen den Messreihen behandelt.";

    let alone = embedder.embed(&[Input::Passage(short)]).unwrap().remove(0);
    let batched = embedder
        .embed(&[Input::Passage(short), Input::Passage(long)])
        .unwrap()
        .remove(0);

    let agreement = similarity(&alone, &batched);
    assert!(
        agreement > 0.9999,
        "the same text embedded alone and in a batch differed: similarity {agreement:.6}"
    );
}

#[test]
fn a_query_and_a_passage_are_embedded_differently() {
    // e5 is asymmetric. If the prefixes were dropped, these would be identical — and the model
    // would still work well enough that nobody would notice until recall was measured.
    let embedder = with_model!(a_query_and_a_passage_are_embedded_differently);
    let text = "kamerawinkel bei der montage";
    let vectors = embedder
        .embed(&[Input::Query(text), Input::Passage(text)])
        .unwrap();

    let agreement = similarity(&vectors[0], &vectors[1]);
    assert!(
        agreement < 0.999,
        "query and passage embeddings are identical, so the prefixes are not being applied"
    );
    assert!(
        agreement > 0.7,
        "query and passage embeddings of the same text should still be close, got {agreement:.3}"
    );
}

#[test]
fn a_german_query_ranks_english_passages_by_content() {
    // The property D6 actually needs: a German query, faced with English documents, must put the
    // relevant one first.
    //
    // What it deliberately does *not* assert is that the relevant English passage beats an
    // irrelevant German one. Measured on this model, same-language similarity carries an offset
    // of roughly 0.08–0.12 over cross-language similarity, which is larger than the gap content
    // opens up on short texts:
    //
    //   query (DE) "wie wurden die kamerawinkel verglichen"
    //     0.898  DE relevant      0.794  DE unrelated
    //     0.783  EN relevant      0.695  EN unrelated
    //
    // Ranking is correct within each language and the language bias sits on top of both. It is a
    // property of the model, it is recorded rather than worked around, and the goldset is what
    // will say whether it costs recall in practice.
    let embedder = with_model!(a_german_query_ranks_english_passages_by_content);
    let vectors = embedder
        .embed(&[
            Input::Query("wie wurden die kamerawinkel verglichen"),
            Input::Passage("The camera angles were compared across three different mounting positions."),
            Input::Passage("Storage costs grew faster than expected after the archive migration."),
        ])
        .unwrap();

    let relevant = similarity(&vectors[0], &vectors[1]);
    let unrelated = similarity(&vectors[0], &vectors[2]);
    assert!(
        relevant > unrelated + 0.02,
        "a German query should prefer the relevant English passage ({relevant:.3}) over an \
         irrelevant one ({unrelated:.3})"
    );
}

#[test]
fn the_language_bias_is_smaller_than_the_content_signal_within_a_language() {
    // Stated as a number so that a future model change is visibly better or worse rather than
    // differently vague. If the content gap ever falls below the language offset *within* a
    // language too, the model is not doing retrieval at all and D6 needs revisiting.
    let embedder = with_model!(the_language_bias_is_smaller_than_the_content_signal_within_a_language);
    let vectors = embedder
        .embed(&[
            Input::Query("warum sind die speicherkosten gestiegen"),
            Input::Passage("Die Speicherkosten stiegen nach der Archivmigration schneller als erwartet."),
            Input::Passage("Die Kamerawinkel wurden an drei verschiedenen Montagepositionen verglichen."),
        ])
        .unwrap();

    let content_gap = similarity(&vectors[0], &vectors[1]) - similarity(&vectors[0], &vectors[2]);
    assert!(
        content_gap > 0.05,
        "content only separated the candidates by {content_gap:.3}"
    );
}

#[test]
fn a_paraphrase_beats_a_word_match() {
    // What the model buys over the lexical arm. The relevant passage shares no content word with
    // the query; the irrelevant one shares two.
    let embedder = with_model!(a_paraphrase_beats_a_word_match);
    let vectors = embedder
        .embed(&[
            Input::Query("was kostet die aufbewahrung der alten daten"),
            Input::Passage("Die Speicherkosten stiegen nach der Archivmigration schneller als erwartet."),
            Input::Passage("Die alten Daten wurden am Montag nach Bremen gefahren."),
        ])
        .unwrap();

    let paraphrase = similarity(&vectors[0], &vectors[1]);
    let word_match = similarity(&vectors[0], &vectors[2]);
    assert!(
        paraphrase > word_match,
        "the paraphrase scored {paraphrase:.3} against {word_match:.3} for the word match; \
         this is the entire reason the vector arm exists"
    );
}

#[test]
fn an_empty_input_is_a_vector_rather_than_a_crash() {
    let embedder = with_model!(an_empty_input_is_a_vector_rather_than_a_crash);
    let vectors = embedder.embed(&[Input::Passage("")]).unwrap();
    assert_eq!(vectors[0].len(), E5_SMALL.dim);
    assert!(vectors[0].iter().all(|v| v.is_finite()));

    assert!(embedder.embed(&[]).unwrap().is_empty());
}

#[test]
fn text_longer_than_the_model_window_is_truncated_rather_than_refused() {
    // A chunk is bounded, but a caller could still hand over something enormous — and the model
    // has a hard 512-token window. Truncating is the documented behaviour; failing would make
    // one oversized file able to stall a corpus.
    let embedder = with_model!(text_longer_than_the_model_window_is_truncated_rather_than_refused);
    let huge = "wort ".repeat(20_000);
    let vectors = embedder.embed(&[Input::Passage(&huge)]).unwrap();
    assert_eq!(vectors[0].len(), E5_SMALL.dim);
    assert!(vectors[0].iter().all(|v| v.is_finite()));
}
