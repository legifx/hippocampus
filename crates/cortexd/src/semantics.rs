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
use cortex_embed::registry::{self, ModelSpec};
use cortex_embed::Embedder;
use cortex_store::vectors::VectorIndex;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// A loaded model and the index that belongs to it. Both or neither.
#[derive(Clone)]
pub struct Loaded {
    pub embedder: Arc<dyn Embedder>,
    pub vectors: Arc<VectorIndex>,
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
    pub note: Option<String>,
    pub model_present: bool,
    pub runtime_present: bool,
    pub model_dir: String,
    pub runtime_path: String,
}

pub struct Semantics {
    loaded: RwLock<Option<Loaded>>,
    note: RwLock<Option<String>>,
    store: ModelStore,
    state_dir: PathBuf,
    config: EmbeddingConfig,
}

impl Semantics {
    pub fn new(state_dir: &std::path::Path, config: EmbeddingConfig) -> Self {
        Self {
            loaded: RwLock::new(None),
            note: RwLock::new(None),
            store: ModelStore::new(state_dir),
            state_dir: state_dir.to_path_buf(),
            config,
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

    /// Where the runtime library should be, honouring an override.
    pub fn runtime_path(&self) -> PathBuf {
        self.config
            .runtime_path
            .clone()
            .unwrap_or_else(|| self.store.runtime_library(&registry::RUNTIME))
    }

    pub fn model_store(&self) -> &ModelStore {
        &self.store
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

        let path = self.state_dir.join("vectors.usearch");
        let vectors = match VectorIndex::open(&path, dim) {
            Ok(index) => Arc::new(index),
            Err(e) => {
                let note = format!("the vector index could not be opened: {e}");
                self.set_note(&note);
                return Err(note);
            }
        };

        let backend = embedder.backend();
        *self.loaded.write().unwrap() = Some(Loaded { embedder, vectors });
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

        OnnxEmbedder::load(&runtime, &self.store.model_dir(spec), *spec, device, threads)
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
        SemanticStatus {
            enabled: self.config.enabled,
            model: self.config.model.clone(),
            loaded: loaded.is_some(),
            backend: loaded.as_ref().map(|l| l.embedder.backend()),
            dim: loaded.as_ref().map(|l| l.embedder.model().dim),
            vectors: loaded.as_ref().map(|l| l.vectors.len()).unwrap_or(0),
            note: self.note(),
            model_present: spec.map(|s| self.store.model_present(s)).unwrap_or(false),
            runtime_present: self.runtime_path().exists(),
            model_dir: spec
                .map(|s| self.store.model_dir(s).display().to_string())
                .unwrap_or_default(),
            runtime_path: self.runtime_path().display().to_string(),
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
        let semantics = Semantics::new(tmp.path(), config("multilingual-e5-small"));

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
        let semantics = Semantics::new(tmp.path(), config("hashing"));

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
        );

        assert!(semantics.load().is_err());
        assert!(semantics.get().is_none());
        assert!(semantics.status().note.is_some_and(|n| n.contains("disabled")));
    }

    #[test]
    fn an_unknown_model_name_is_reported_rather_than_guessed_at() {
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("mistral-embed-9000"));
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
        );
        assert_eq!(semantics.runtime_path(), elsewhere);
    }

    #[test]
    fn loading_twice_keeps_working_rather_than_leaving_a_gap() {
        // `model.reload` can be called at any time, including while a search is running.
        let tmp = tempfile::tempdir().unwrap();
        let semantics = Semantics::new(tmp.path(), config("hashing"));
        semantics.load().unwrap();
        let first = semantics.get().unwrap();
        semantics.load().unwrap();
        assert!(semantics.get().is_some());
        assert_eq!(first.embedder.model().dim, semantics.get().unwrap().embedder.model().dim);
    }
}
