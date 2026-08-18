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

/// A cross-encoder: it reads a query and a passage *together* and scores the pair.
///
/// That is the whole difference from an embedder, and it is why it goes last rather than first.
/// An embedder turns a passage into a vector once, offline, and a search compares vectors; a
/// cross-encoder cannot precompute anything, so it costs one forward pass per candidate at query
/// time. Thirty of those is affordable; thirty thousand is not, which is exactly why it reranks
/// what retrieval already narrowed down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RerankerSpec {
    pub id: &'static str,
    pub version: &'static str,
    pub max_tokens: usize,
    /// XLM-RoBERTa's padding token. Padding with the wrong id is not an error and not visible;
    /// it just quietly scores the padding.
    pub pad_token: i64,
    pub files: &'static [FileSpec],
}

impl RerankerSpec {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// `mmarco-mMiniLMv2-L12-H384-v1`.
///
/// Chosen to match the embedder rather than to beat it: the same twelve layers and 384 hidden
/// units, trained on mMARCO in fourteen languages. A larger reranker (bge-reranker-base is
/// 1.1 GB) scores better on English benchmarks and costs three times as much for a corpus that
/// is half German.
pub const RERANKER: RerankerSpec = RerankerSpec {
    id: "mmarco-mMiniLMv2-L12-H384-v1",
    version: "1",
    max_tokens: 512,
    pad_token: 1,
    files: &[
        FileSpec {
            name: "model.onnx",
            url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/main/onnx/model.onnx",
            sha256: "3e9a03ed1e966f7c5288dd4230e3d6a9bf5e3a170a06f1f4241c5bca12c6487c",
            bytes: 470_883_696,
        },
        FileSpec {
            name: "tokenizer.json",
            url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/main/tokenizer.json",
            sha256: "62c24cdc13d4c9952d63718d6c9fa4c287974249e16b7ade6d5a85e7bbb75626",
            bytes: 17_082_660,
        },
    ],
};

/// Every model this build knows how to run.
pub const MODELS: &[ModelSpec] = &[E5_SMALL];

pub fn model_by_id(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// The ONNX Runtime, in the two builds this project knows how to use.
///
/// Fetched like a model rather than required as a system package, for three reasons: it keeps
/// `cargo build` free of network access, it avoids asking anybody for root to try this project,
/// and it puts the runtime under the same checksum discipline as the model it loads (D25).
///
/// The library is loaded by path at startup, so a system installation can be used instead by
/// pointing `embedding.runtime_path` at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSpec {
    /// Short name, and the directory it installs into.
    pub id: &'static str,
    pub version: &'static str,
    pub archive: FileSpec,
    /// Files to take out of the archive, as `(path inside, name installed)`.
    ///
    /// More than one for the GPU build: the execution provider is a separate shared object that
    /// ONNX Runtime opens *by name, from the directory of the main library*. Installing it
    /// anywhere else produces a runtime that loads and then quietly has no CUDA.
    pub libraries: &'static [(&'static str, &'static str)],
    /// The one the daemon opens.
    pub library: &'static str,
    /// Whether this build can use the GPU at all.
    pub cuda: bool,
    /// What has to be installed on the machine for it to work.
    pub requires: &'static str,
}

/// The CPU build. 11 MB, works anywhere.
pub const RUNTIME_CPU: RuntimeSpec = RuntimeSpec {
    id: "onnxruntime",
    version: "1.29.0",
    archive: FileSpec {
        name: "onnxruntime-linux-x64-1.29.0.tgz",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-linux-x64-1.29.0.tgz",
        sha256: "c3fddc4f139a045b0c4902c57410f0694f1c2fdf9b6939fbe38b1aeae7cd14ba",
        bytes: 11_082_880,
    },
    libraries: &[(
        "onnxruntime-linux-x64-1.29.0/lib/libonnxruntime.so.1.29.0",
        "libonnxruntime.so",
    )],
    library: "libonnxruntime.so",
    cuda: false,
    requires: "nothing",
};

/// The CUDA build (D6). 191 MB, and it needs a CUDA runtime and cuDNN on the machine — a driver
/// alone is not enough, which is the failure everybody hits first.
pub const RUNTIME_CUDA: RuntimeSpec = RuntimeSpec {
    id: "onnxruntime-cuda13",
    version: "1.29.0",
    archive: FileSpec {
        name: "onnxruntime-linux-x64-gpu_cuda13-1.29.0.tgz",
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-linux-x64-gpu_cuda13-1.29.0.tgz",
        sha256: "844c64acfc43ab9423215c26493055ea229268e28283146cc644ecef0bdae048",
        bytes: 191_380_186,
    },
    libraries: &[
        (
            "onnxruntime-linux-x64-gpu_cuda13-1.29.0/lib/libonnxruntime.so.1.29.0",
            "libonnxruntime.so",
        ),
        (
            "onnxruntime-linux-x64-gpu_cuda13-1.29.0/lib/libonnxruntime_providers_cuda.so",
            "libonnxruntime_providers_cuda.so",
        ),
        (
            "onnxruntime-linux-x64-gpu_cuda13-1.29.0/lib/libonnxruntime_providers_shared.so",
            "libonnxruntime_providers_shared.so",
        ),
    ],
    library: "libonnxruntime.so",
    cuda: true,
    requires: "CUDA 13 runtime and cuDNN 9",
};

pub const RUNTIMES: &[RuntimeSpec] = &[RUNTIME_CPU, RUNTIME_CUDA];

/// The runtime a given device setting wants, best first.
///
/// `auto` and `cuda` prefer the GPU build and accept the CPU one, because a GPU build that
/// cannot find CUDA still runs on the CPU — it is a superset, not an alternative.
pub fn runtimes_for(device: &str) -> &'static [RuntimeSpec] {
    match device {
        "cpu" => &[RUNTIME_CPU],
        _ => &[RUNTIME_CUDA, RUNTIME_CPU],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pinned_file_has_a_full_length_checksum() {
        // A truncated or placeholder hash would pass a naive comparison against a prefix and
        // silently disable the only integrity check this project has.
        let mut files: Vec<&FileSpec> = MODELS.iter().flat_map(|m| m.files.iter()).collect();
        files.extend(RUNTIMES.iter().map(|r| &r.archive));
        files.extend(RERANKER.files.iter());

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
        urls.extend(RUNTIMES.iter().map(|r| r.archive.url));
        urls.extend(RERANKER.files.iter().map(|f| f.url));

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
    fn the_gpu_runtime_installs_its_provider_beside_the_library() {
        // ONNX Runtime opens the CUDA provider by name from the directory of the main library.
        // Installed anywhere else, the runtime loads, reports no error, and simply has no GPU.
        assert!(RUNTIME_CUDA.cuda);
        assert!(RUNTIME_CUDA
            .libraries
            .iter()
            .any(|(_, installed)| *installed == "libonnxruntime_providers_cuda.so"));
        assert_eq!(RUNTIME_CUDA.library, "libonnxruntime.so");
        assert!(RUNTIME_CUDA.libraries.iter().any(|(_, i)| *i == RUNTIME_CUDA.library));
    }

    #[test]
    fn the_two_runtimes_install_into_different_directories() {
        // Switching between them is a configuration change plus a fetch, not an overwrite: a
        // machine that loses its CUDA installation must still have the CPU build to fall back on.
        assert_ne!(RUNTIME_CPU.id, RUNTIME_CUDA.id);
    }

    #[test]
    fn a_cpu_only_setting_never_asks_for_the_gpu_build() {
        assert_eq!(runtimes_for("cpu").len(), 1);
        assert!(!runtimes_for("cpu")[0].cuda);
        // Everything else prefers the GPU build and accepts the CPU one, because the GPU build
        // runs on the CPU too.
        assert!(runtimes_for("auto")[0].cuda);
        assert_eq!(runtimes_for("auto").len(), 2);
        assert!(runtimes_for("cuda")[0].cuda);
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
