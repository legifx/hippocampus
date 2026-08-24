//! Whether this daemon can think in vectors, and what to do when it cannot.
//!
//! The model is a deliberate half-gigabyte download (rule R5), so the normal state of a fresh
//! install is **no model**, and everything here is built around that being fine rather than
//! broken. Lexical and path search work; the vector arm reports that it is not running, and says
//! what would make it run.
//!
//! The other thing this exists for is arriving *late*. `cortex model fetch` puts files on disk
//! while the daemon is already up, and asking somebody to restart a memory daemon to finish
//! installing it is the kind of small indignity that makes software feel unfinished. So the
//! embedder lives behind a lock, the embedding stage waits for it to appear, and `model.reload`
//! swaps it in.

use cortex_core::config::EmbeddingConfig;
use cortex_embed::fetch::ModelStore;
use cortex_embed::hashing::HashingEmbedder;
use cortex_embed::registry::{self, ModelSpec, RuntimeSpec};
use cortex_embed::{Embedder, Reranker};
use cortex_store::vectors::VectorIndex;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// A loaded model and the index that belongs to it. Both or neither.
#[derive(Clone)]
pub struct Loaded {
    pub embedder: Arc<dyn Embedder>,
    /// The ANN index over file chunks.
    pub vectors: Arc<VectorIndex>,
    /// The ANN index over memory chunks — a second collection of the same type, in its own
    /// file (D75).
    ///
    /// Not a filter over the first one, and the reason is arithmetic rather than tidiness: a
    /// memory recall against an index holding 122 137 file chunks competes for its twenty slots
    /// against every one of them and loses. Over-fetching and filtering afterwards does not fix
    /// it, because the memory that answers the question can be at rank five thousand. Two
    /// collections make the memory arm exact and microsecond-cheap, and cost one more file that
    /// is rebuildable from the same table as the first.
    pub memory_vectors: Arc<VectorIndex>,
}

/// What `cortex status` and `cortex model status` need to say.
#[derive(Debug, Clone)]
pub struct SemanticStatus {
    pub enabled: bool,
    pub model: String,
    pub loaded: bool,
    pub backend: Option<String>,
    pub dim: Option<usize>,
    pub vectors: usize,
    /// The memory collection's size (M11). Reported separately because it is a separate file
    /// with a separate rebuild path, and one number covering both would make a full memory
    /// index and an empty file one look like a half-filled single one.
    pub memory_vectors: usize,
    pub note: Option<String>,
    pub model_present: bool,
    pub runtime_present: bool,
    /// Which runtime build is in use, and what it needs from the machine.
    pub runtime_id: String,
    pub runtime_is_cuda: bool,
    pub model_dir: String,
    pub runtime_path: String,
}

pub struct Semantics {
    loaded: RwLock<Option<Loaded>>,
    /// The cross-encoder, if one has been fetched. Loaded separately from the embedder: it is a
    /// second half-gigabyte download, it is useless without the first, and a machine that has
    /// one and not the other should get the half that works rather than neither.
    reranker: RwLock<Option<Arc<dyn Reranker>>>,
    reranker_note: RwLock<Option<String>>,
    note: RwLock<Option<String>>,
    store: ModelStore,
    state_dir: PathBuf,
    config: EmbeddingConfig,
    /// Whether the search configuration wants reranking at all.
    search_rerank: bool,
}

impl Semantics {
    pub fn new(state_dir: &std::path::Path, config: EmbeddingConfig, search_rerank: bool) -> Self {
        Self {
            loaded: RwLock::new(None),
            reranker: RwLock::new(None),
            reranker_note: RwLock::new(None),
            note: RwLock::new(None),
            store: ModelStore::new(state_dir),
            state_dir: state_dir.to_path_buf(),
            config,
            search_rerank,
        }
    }

    pub fn get(&self) -> Option<Loaded> {
        self.loaded.read().unwrap().clone()
    }

    pub fn note(&self) -> Option<String> {
        self.note.read().unwrap().clone()
    }

    fn spec(&self) -> Option<&'static ModelSpec> {
        registry::model_by_id(&self.config.model)
    }

    /// Which runtime this configuration wants, and where it would live.
    ///
    /// `auto` and `cuda` prefer the GPU build; the CPU build is the fallback rather than an
    /// alternative, because the GPU build also runs on the CPU. An explicit `runtime_path`
    /// overrides both, for a system installation.
    pub fn runtime_choice(&self) -> (Option<&'static RuntimeSpec>, PathBuf) {
        if let Some(path) = &self.config.runtime_path {
            return (None, path.clone());
        }
        let candidates = registry::runtimes_for(&self.config.device);
        for runtime in candidates {
            if self.store.runtime_present(runtime) {
                return (Some(runtime), self.store.runtime_library(runtime));
            }
        }
        // None installed: name the preferred one, so the error says what to fetch.
        let preferred = &candidates[0];
        (Some(preferred), self.store.runtime_library(preferred))
    }

    pub fn runtime_path(&self) -> PathBuf {
        self.runtime_choice().1
    }

    pub fn model_store(&self) -> &ModelStore {
        &self.store
    }

    pub fn reranker(&self) -> Option<Arc<dyn Reranker>> {
        self.reranker.read().unwrap().clone()
    }

    pub fn reranker_note(&self) -> Option<String> {
        self.reranker_note.read().unwrap().clone()
    }

    /// Load the cross-encoder, if it is there.
    ///
    /// Failure is quiet by design: without it the search is the same search, just in the order
    /// retrieval produced. Every other component treats the reranker as optional, so this must
    /// not be the one place that makes it mandatory.
    #[cfg(feature = "onnx")]
    pub fn load_reranker(&self) -> std::result::Result<String, String> {
        use cortex_embed::onnx::{Device, OnnxReranker};
        use cortex_embed::registry::RERANKER;

        if !self.search_rerank {
            let note = "reranking is switched off in the configuration".to_string();
            *self.reranker_note.write().unwrap() = Some(note.clone());
            return Err(note);
        }

        let device = match self.config.device.as_str() {
            "cpu" => Device::Cpu,
            "cuda" => Device::Cuda,
            _ => Device::Auto,
        };
        match OnnxReranker::load(
            &self.runtime_path(),
            &self.store.reranker_dir(&RERANKER),
            RERANKER,
            device,
            self.config.threads,
            self.config.gpu_memory_mb,
        ) {
            Ok(reranker) => {
                let backend = reranker.backend();
                *self.reranker.write().unwrap() = Some(Arc::new(reranker));
                *self.reranker_note.write().unwrap() = None;
                Ok(backend)
            }
            Err(e) => {
                let note = e.to_string();
                *self.reranker_note.write().unwrap() = Some(note.clone());
                Err(note)
            }
        }
    }

    #[cfg(not(feature = "onnx"))]
    pub fn load_reranker(&self) -> std::result::Result<String, String> {
        Err("this build has no ONNX support".into())
    }

    /// Try to load, and record why if it does not work.
    ///
    /// Returns the backend description on success. Called at startup and again after
    /// `cortex model fetch`, and safe to call repeatedly — a failure leaves whatever was already
    /// loaded in place rather than tearing a working embedder down.
    pub fn load(&self) -> std::result::Result<String, String> {
        if !self.config.enabled {
            self.set_note("embedding is disabled in the configuration");
            return Err("embedding is disabled in the configuration".into());
        }

        let (embedder, dim): (Arc<dyn Embedder>, usize) = if self.config.model == HashingEmbedder::ID
        {
            // The built-in embedder. Weak, but it needs nothing, so it is what makes the whole
            // vector path testable and demonstrable on a machine with no model at all.
            (
                Arc::new(HashingEmbedder::new()),
                cortex_embed::hashing::DIM,
            )
        } else {
            let Some(spec) = self.spec() else {
                let note = format!("unknown embedding model {:?}", self.config.model);
                self.set_note(&note);
                return Err(note);
            };
            match self.load_onnx(spec) {
                Ok(embedder) => (embedder, spec.dim),
                Err(note) => {
                    self.set_note(&note);
                    return Err(note);
                }
            }
        };

        let path = cortex_core::paths::vector_index_in(&self.state_dir);
        let vectors = match VectorIndex::open(&path, dim) {
            Ok(index) => Arc::new(index),
            Err(e) => {
                let note = format!("the vector index could not be opened: {e}");
                self.set_note(&note);
                return Err(note);
            }
        };
        let memory_path = cortex_core::paths::memory_vector_index_in(&self.state_dir);
        let memory_vectors = match VectorIndex::open(&memory_path, dim) {
            Ok(index) => Arc::new(index),
            Err(e) => {
                let note = format!("the memory vector index could not be opened: {e}");
                self.set_note(&note);
                return Err(note);
            }
        };

        let backend = embedder.backend();
        *self.loaded.write().unwrap() = Some(Loaded {
            embedder,
            vectors,
            memory_vectors,
        });
        *self.note.write().unwrap() = None;
        Ok(backend)
    }

    #[cfg(feature = "onnx")]
    fn load_onnx(&self, spec: &'static ModelSpec) -> std::result::Result<Arc<dyn Embedder>, String> {
        use cortex_embed::onnx::{Device, OnnxEmbedder};

        let runtime = self.runtime_path();
        let device = match self.config.device.as_str() {
            "cpu" => Device::Cpu,
            "cuda" => Device::Cuda,
            _ => Device::Auto,
        };
        let threads = if self.config.threads == 0 {
            // Zero means "let the runtime decide", which it does by using every core. Measured
            // on the reference workstation that is slower than eight threads, but it is the
            // caller's choice to make.
            0
        } else {
            self.config.threads
        };

        OnnxEmbedder::load(
            &runtime,
            &self.store.model_dir(spec),
            *spec,
            device,
            threads,
            self.config.gpu_memory_mb,
        )
        .map(|e| Arc::new(e) as Arc<dyn Embedder>)
        .map_err(|e| e.to_string())
    }

    #[cfg(not(feature = "onnx"))]
    fn load_onnx(&self, _spec: &'static ModelSpec) -> std::result::Result<Arc<dyn Embedder>, String> {
        Err("this build has no ONNX support; set embedding.model = \"hashing\"".into())
    }

    fn set_note(&self, note: &str) {
        *self.note.write().unwrap() = Some(note.to_string());
    }

    pub fn status(&self) -> SemanticStatus {
        let loaded = self.get();
        let spec = self.spec();
        let (runtime, runtime_path) = self.runtime_choice();
        SemanticStatus {
            runtime_id: runtime.map(|r| r.id.to_string()).unwrap_or_else(|| "configured path".into()),
            runtime_is_cuda: runtime.map(|r| r.cuda).unwrap_or(false),
            enabled: self.config.enabled,
            model: self.config.model.clone(),
            loaded: loaded.is_some(),
            backend: loaded.as_ref().map(|l| l.embedder.backend()),
            dim: loaded.as_ref().map(|l| l.embedder.model().dim),
            vectors: loaded.as_ref().map(|l| l.vectors.len()).unwrap_or(0),
            memory_vectors: loaded.as_ref().map(|l| l.memory_vectors.len()).unwrap_or(0),
            note: self.note(),
            model_present: spec.map(|s| self.store.model_present(s)).unwrap_or(false),
            runtime_present: runtime
                .map(|r| self.store.runtime_present(r))
                .unwrap_or_else(|| runtime_path.exists()),
            model_dir: spec
                .map(|s| self.store.model_dir(s).display().to_string())
                .unwrap_or_default(),
            runtime_path: runtime_path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(model: &str) -> EmbeddingConfig {
        EmbeddingConfig {
            model: model.to_string(),
            ..EmbeddingConfig::default()
        }
    }

    #[test]
    fn a_machine_with_no_model_loads_nothing_and_says_why() {
        // The default state of a fresh install, and it must not read as a failure.
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("multilingual-e5-small"), true);

        assert!(semantics.load().is_err());
        assert!(semantics.get().is_none());

        let status = semantics.status();
        assert!(!status.loaded);
        assert!(!status.model_present);
        assert!(
            status.note.as_deref().is_some_and(|n| n.contains("cortex model fetch")),
            "the note must say what to run: {:?}",
            status.note
        );
    }

    #[test]
    fn the_built_in_embedder_needs_nothing_at_all() {
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("hashing"), true);

        let backend = semantics.load().expect("the hashing embedder cannot fail to load");
        assert!(backend.contains("hashing"));

        let loaded = semantics.get().expect("it must be available afterwards");
        assert_eq!(loaded.embedder.model().dim, cortex_embed::hashing::DIM);
        assert_eq!(loaded.vectors.dim(), cortex_embed::hashing::DIM);
        assert!(semantics.status().loaded);
        assert!(semantics.note().is_none());
    }

    #[test]
    fn disabling_it_in_the_configuration_is_honoured_even_with_a_model_present() {
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(
            tmp.path(),
            EmbeddingConfig {
                enabled: false,
                model: "hashing".to_string(),
                ..EmbeddingConfig::default()
            },
            true,
        );

        assert!(semantics.load().is_err());
        assert!(semantics.get().is_none());
        assert!(semantics.status().note.is_some_and(|n| n.contains("disabled")));
    }

    #[test]
    fn an_unknown_model_name_is_reported_rather_than_guessed_at() {
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("mistral-embed-9000"), true);
        let error = semantics.load().unwrap_err();
        assert!(error.contains("unknown embedding model"), "{error}");
    }

    #[test]
    fn a_configured_runtime_path_overrides_the_state_directory() {
        // For a system installation of onnxruntime, or a build with different providers.
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("usr/lib/libonnxruntime.so");
        let semantics = Semantics::new(
            tmp.path(),
            EmbeddingConfig {
                runtime_path: Some(elsewhere.clone()),
                ..EmbeddingConfig::default()
            },
            true,
        );
        assert_eq!(semantics.runtime_path(), elsewhere);
    }

    #[test]
    fn loading_twice_keeps_working_rather_than_leaving_a_gap() {
        // `model.reload` can be called at any time, including while a search is running.
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("hashing"), true);
        semantics.load().unwrap();
        let first = semantics.get().unwrap();
        semantics.load().unwrap();
        assert!(semantics.get().is_some());
        assert_eq!(first.embedder.model().dim, semantics.get().unwrap().embedder.model().dim);
    }
}
