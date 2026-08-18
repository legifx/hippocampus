//! Turning text into vectors.
//!
//! Two implementations, and the difference between them is the point:
//!
//! - [`onnx::OnnxEmbedder`] runs `multilingual-e5-small` (decision D6). It needs a model on
//!   disk, a runtime library, and half a gigabyte the user consented to download.
//! - [`hashing::HashingEmbedder`] needs nothing. It is a real embedding method — the hashing
//!   trick over word tokens — which makes it weak but not fake: similar text really does produce
//!   similar vectors, so the entire pipeline can be tested, benchmarked and demonstrated on a
//!   machine that has never downloaded a model.
//!
//! Having the second one is what keeps the first one optional. Rule R5 says a fresh install
//! reaches for nothing; a system that could only work after a download would fail that at the
//! first honest reading.

pub mod fetch;
pub mod hashing;
pub mod registry;

#[cfg(feature = "onnx")]
pub mod onnx;

pub use registry::{ModelSpec, E5_SMALL};

/// Text on its way to a model, and what kind of text it is.
///
/// The distinction is not decoration. Asymmetric models — e5 among them — are trained with
/// different prefixes for the thing being searched for and the thing being searched, and using
/// the wrong one costs recall while producing no error whatsoever. Making the caller say which
/// it has means the prefix can be applied by the code that knows the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input<'a> {
    Query(&'a str),
    Passage(&'a str),
}

impl<'a> Input<'a> {
    pub fn text(&self) -> &'a str {
        match self {
            Input::Query(t) | Input::Passage(t) => t,
        }
    }
}

/// Which model produced a vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub id: String,
    pub version: String,
    pub dim: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model files are missing: {0}")]
    NotProvisioned(String),

    #[error("the ONNX runtime could not be loaded from {path}: {message}")]
    Runtime { path: String, message: String },

    #[error("inference failed: {0}")]
    Inference(String),

    #[error("tokenizer: {0}")]
    Tokenizer(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;

/// Anything that can turn text into vectors.
pub trait Embedder: Send + Sync {
    fn model(&self) -> ModelId;

    /// Embed a batch. The returned vectors are L2-normalised, in the order given.
    ///
    /// Normalisation is part of the contract rather than the caller's job: cosine similarity
    /// over unit vectors is a dot product, which is what both the vector index and every
    /// re-scoring path assume. A model that forgot to normalise would produce a ranking that
    /// is subtly wrong everywhere and obviously wrong nowhere.
    fn embed(&self, inputs: &[Input<'_>]) -> Result<Vec<Vec<f32>>>;

    /// Largest batch worth sending at once.
    fn max_batch(&self) -> usize {
        16
    }

    /// A short description for `cortex status`, e.g. "onnx, cpu (8 threads)".
    fn backend(&self) -> String;
}

/// Normalise in place to unit length. A zero vector is left alone rather than producing NaNs.
pub fn normalise(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Cosine similarity of two unit vectors: their dot product.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalising_produces_unit_length() {
        let mut v = vec![3.0, 4.0];
        normalise(&mut v);
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(v, vec![0.6, 0.8]);
    }

    #[test]
    fn a_zero_vector_survives_normalisation() {
        // An empty chunk, or a chunk of characters the tokenizer drops entirely. Dividing by
        // zero here would put NaNs into the index, and NaN comparisons make a ranking silently
        // arbitrary rather than loudly wrong.
        let mut v = vec![0.0, 0.0, 0.0];
        normalise(&mut v);
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn similarity_of_a_vector_with_itself_is_one() {
        let mut v = vec![1.0, 2.0, 3.0];
        normalise(&mut v);
        assert!((similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_input_keeps_its_text_whichever_kind_it_is() {
        assert_eq!(Input::Query("was war das").text(), "was war das");
        assert_eq!(Input::Passage("das war es").text(), "das war es");
    }
}
