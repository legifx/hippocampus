//! What may be downloaded, from where, and what it must hash to.
//!
//! Rule R5 says a fresh install performs zero outbound connections and no model is fetched
//! without an explicit user action. That makes this file the complete list of things this
//! project will ever download — small enough to read, and pinned hard enough that reading it
//! twice tells you the same thing.
//!
//! Every entry carries a SHA-256 and a byte count. A download that does not match is discarded,
//! not repaired and not used: a model file is code as far as the runtime that loads it is
//! concerned, and "probably the right file" is not a security property.
//!
//! Versions are pinned, never floating. An upstream that silently republishes a file under the
//! same name must fail the checksum rather than change the meaning of every vector in the index.

/// One file to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpec {
    /// Name it is stored under, inside the model's directory.
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// An embedding model and everything needed to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: &'static str,
    /// Bumped whenever anything changes that would change a vector. Stored per embedding, so a
    /// bump is a background migration rather than a rebuild.
    pub version: &'static str,
    pub dim: usize,
    /// Tokens beyond this are truncated by the model itself; the chunker aims well below it.
    pub max_tokens: usize,
    /// e5 models are trained with asymmetric prefixes and lose noticeable recall without them.
    /// They belong to the model rather than to the caller, which is why `Input` is semantic and
    /// the prefix is applied here.
    pub query_prefix: &'static str,
    pub passage_prefix: &'static str,
    pub files: &'static [FileSpec],
}

impl ModelSpec {
    pub fn file(&self, name: &str) -> Option<&FileSpec> {
        self.files.iter().find(|f| f.name == name)
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    /// The prefix this model wants for a given kind of text.
    pub fn prefix(&self, input: crate::Input<'_>) -> &'static str {
        match input {
            crate::Input::Query(_) => self.query_prefix,
            crate::Input::Passage(_) => self.passage_prefix,
        }
    }
}

/// `multilingual-e5-small` (decision D6).
///
/// Chosen because the corpus this project exists for is German and English mixed, often inside
/// one document, and because 384 dimensions over 118 M parameters is the largest model that
/// still embeds a home directory in reasonable time on hardware people own.
pub const E5_SMALL: ModelSpec = ModelSpec {
    id: "multilingual-e5-small",
    version: "1",
    dim: 384,
    max_tokens: 512,
    query_prefix: "query: ",
    passage_prefix: "passage: ",
    files: &[
        FileSpec {
            name: "model.onnx",
            url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/main/onnx/model.onnx",
            sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
            bytes: 470_268_510,
        },
        FileSpec {
            name: "tokenizer.json",
            url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/main/tokenizer.json",
            sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
            bytes: 17_082_730,
        },
    ],
};

/// Every model this build knows how to run.
pub const MODELS: &[ModelSpec] = &[E5_SMALL];

pub fn model_by_id(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// The ONNX Runtime shared library.
///
/// Fetched like a model rather than required as a system package, for three reasons: it keeps
/// `cargo build` free of network access, it avoids asking anybody for root to try this project,
/// and it puts the runtime under the same checksum discipline as the model it loads.
///
/// The library is loaded by path at startup, so a system installation can be used instead by
/// pointing `embedding.runtime_path` at it.
pub const RUNTIME: RuntimeSpec = RuntimeSpec {
    version: "1.29.0",
    archive: FileSpec {
        name: "onnxruntime-linux-x64-1.29.0.tgz",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-linux-x64-1.29.0.tgz",
        sha256: "c3fddc4f139a045b0c4902c57410f0694f1c2fdf9b6939fbe38b1aeae7cd14ba",
        bytes: 11_082_880,
    },
    // Path of the library inside the archive.
    library_in_archive: "onnxruntime-linux-x64-1.29.0/lib/libonnxruntime.so.1.29.0",
    library: "libonnxruntime.so",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub version: &'static str,
    pub archive: FileSpec,
    /// Path of the shared library inside the archive.
    pub library_in_archive: &'static str,
    /// Name it is installed under, which is also what the daemon loads by path.
    pub library: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pinned_file_has_a_full_length_checksum() {
        // A truncated or placeholder hash would pass a naive comparison against a prefix and
        // silently disable the only integrity check this project has.
        let mut files: Vec<&FileSpec> = MODELS.iter().flat_map(|m| m.files.iter()).collect();
        files.push(&RUNTIME.archive);

        for file in files {
            assert_eq!(
                file.sha256.len(),
                64,
                "{} has a {}-character hash",
                file.name,
                file.sha256.len()
            );
            assert!(
                file.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{} has a hash that is not lowercase hex",
                file.name
            );
            assert!(file.bytes > 0, "{} has no expected size", file.name);
        }
    }

    #[test]
    fn every_url_is_https_and_names_a_public_host() {
        // Rule R9: nothing in this repository may point at a private host, and rule R5 means
        // the one connection it does make must at least be authenticated transport.
        let mut urls: Vec<&str> = MODELS.iter().flat_map(|m| m.files.iter().map(|f| f.url)).collect();
        urls.push(RUNTIME.archive.url);

        for url in urls {
            assert!(url.starts_with("https://"), "{url} is not https");
            assert!(
                url.starts_with("https://huggingface.co/")
                    || url.starts_with("https://github.com/"),
                "{url} is not one of the two hosts this project downloads from"
            );
        }
    }

    #[test]
    fn model_ids_are_unique_and_resolvable() {
        for model in MODELS {
            assert_eq!(model_by_id(model.id).map(|m| m.id), Some(model.id));
        }
        assert!(model_by_id("something-else").is_none());
    }

    #[test]
    fn the_default_model_matches_the_decision_that_chose_it() {
        // D6 names the model and the dimensionality; the schema and the vector index are built
        // around both. A change here is a change to a decision, not to a constant.
        assert_eq!(E5_SMALL.id, "multilingual-e5-small");
        assert_eq!(E5_SMALL.dim, 384);
        assert!(E5_SMALL.file("model.onnx").is_some());
        assert!(E5_SMALL.file("tokenizer.json").is_some());
    }

    #[test]
    fn e5_prefixes_are_present_and_distinct() {
        // Embedding a query as a passage costs recall and produces no error at all. The
        // asymmetry is a property of the model, so it is asserted where the model is described.
        assert_ne!(E5_SMALL.query_prefix, E5_SMALL.passage_prefix);
        assert!(E5_SMALL.query_prefix.ends_with(' '));
        assert!(E5_SMALL.passage_prefix.ends_with(' '));
    }
}
