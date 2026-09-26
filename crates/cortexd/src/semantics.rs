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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

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

/// An embedder that can put its model down and pick it up again.
///
/// The model behind it is loaded eagerly at start — so a failure is reported at start, where a
/// person is looking — and released after a period with nothing to embed (D103). Two callers,
/// two behaviours, on purpose:
///
/// - **A query** (`embed` while released) must not wait. It gets an error naming the reason,
///   the search skips its semantic arm for this one call exactly as it does when no model has
///   been fetched, and a reload starts in the background so the next keystroke has the arm
///   back. A keystroke that waited two seconds for CUDA to come up would be the lag the whole
///   project is specified against.
/// - **The embedding stage** (`ensure_resident`) can wait: it is a background thread with a
///   batch in hand, and blocking itself for the load costs nobody anything.
pub struct IdleEmbedder {
    inner: RwLock<Option<Arc<dyn Embedder>>>,
    loader: Box<dyn Fn() -> std::result::Result<Arc<dyn Embedder>, String> + Send + Sync>,
    model: cortex_embed::ModelId,
    backend: String,
    max_batch: usize,
    last_used: Mutex<Instant>,
    loading: AtomicBool,
    releases: AtomicU64,
}

impl IdleEmbedder {
    fn new(
        first: Arc<dyn Embedder>,
        loader: impl Fn() -> std::result::Result<Arc<dyn Embedder>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            model: first.model(),
            backend: first.backend(),
            max_batch: first.max_batch(),
            inner: RwLock::new(Some(first)),
            loader: Box::new(loader),
            last_used: Mutex::new(Instant::now()),
            loading: AtomicBool::new(false),
            releases: AtomicU64::new(0),
        }
    }

    pub fn resident(&self) -> bool {
        self.inner.read().unwrap().is_some()
    }

    /// How many times the model has been put down since the daemon started.
    #[cfg(test)]
    pub fn releases(&self) -> u64 {
        self.releases.load(Ordering::Relaxed)
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    /// Load synchronously if the model is down. Returns whether it is resident afterwards.
    pub fn ensure_resident(&self) -> std::result::Result<(), String> {
        if self.resident() {
            return Ok(());
        }
        // One loader at a time. A second caller waits for the first rather than loading a
        // second copy of a half-gigabyte model.
        while self.loading.swap(true, Ordering::AcqRel) {
            std::thread::sleep(Duration::from_millis(50));
            if self.resident() {
                return Ok(());
            }
        }
        let result = if self.resident() {
            Ok(())
        } else {
            let started = Instant::now();
            match (self.loader)() {
                Ok(embedder) => {
                    *self.inner.write().unwrap() = Some(embedder);
                    tracing::info!(
                        took_ms = started.elapsed().as_millis() as u64,
                        "embedding model back after idling"
                    );
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        self.loading.store(false, Ordering::Release);
        self.touch();
        result
    }

    fn load_in_background(self: &Arc<Self>) {
        if self.loading.load(Ordering::Acquire) {
            return;
        }
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("cortex-embed-reload".into())
            .spawn(move || {
                if let Err(e) = me.ensure_resident() {
                    tracing::warn!(error = %e, "the embedding model could not be reloaded");
                }
            })
            .ok();
    }

    /// Put the model down if nothing has used it for `after`. Returns whether it did.
    ///
    /// Only the stage calls this, and only from its idle branch — so "nothing to embed" and
    /// "nothing asked for a query embedding" have both been true for the whole period.
    pub fn release_if_idle(&self, after: Duration) -> bool {
        if after.is_zero() || !self.resident() {
            return false;
        }
        let idle = self.last_used.lock().unwrap().elapsed();
        if idle < after {
            return false;
        }
        let taken = self.inner.write().unwrap().take();
        if taken.is_some() {
            self.releases.fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                idle_secs = idle.as_secs(),
                "embedding model released after idling; the index stays, the runtime's memory goes"
            );
            true
        } else {
            false
        }
    }
}

/// The trait, over the wrapper — so `Loaded.embedder` is one `Arc<dyn Embedder>` whichever
/// embedder is behind it, and neither search nor the stage knows the difference.
struct IdleHandle(Arc<IdleEmbedder>);

impl Embedder for IdleHandle {
    fn model(&self) -> cortex_embed::ModelId {
        self.0.model.clone()
    }

    fn embed(&self, inputs: &[cortex_embed::Input<'_>]) -> cortex_embed::Result<Vec<Vec<f32>>> {
        self.0.touch();
        let resident = self.0.inner.read().unwrap().clone();
        match resident {
            Some(inner) => inner.embed(inputs),
            None => {
                self.0.load_in_background();
                Err(cortex_embed::EmbedError::Inference(
                    "the embedding model was released after idling and is coming back; \
                     the semantic arm is skipped for this call"
                        .into(),
                ))
            }
        }
    }

    fn max_batch(&self) -> usize {
        self.0.max_batch
    }

    fn backend(&self) -> String {
        if self.0.resident() {
            self.0.backend.clone()
        } else {
            format!("{} — released after idling, reloads on demand", self.0.backend)
        }
    }
}

pub struct Semantics {
    loaded: RwLock<Option<Loaded>>,
    /// The releasable embedder behind `loaded`, when there is one. `None` for the hashing
    /// embedder, which holds nothing worth releasing.
    idle: RwLock<Option<Arc<IdleEmbedder>>>,
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
            idle: RwLock::new(None),
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

    /// The releasable embedder, for the stage's two calls. `None` when nothing is releasable.
    pub fn idle_embedder(&self) -> Option<Arc<IdleEmbedder>> {
        self.idle.read().unwrap().clone()
    }

    /// Bring the model back before a batch. A no-op while it is resident or when there is
    /// nothing releasable behind the embedder.
    pub fn ensure_embedder_resident(&self) -> std::result::Result<(), String> {
        match self.idle_embedder() {
            Some(idle) => idle.ensure_resident(),
            None => Ok(()),
        }
    }

    /// Put the model down if it has been idle for the configured period (D103).
    pub fn release_embedder_if_idle(&self) -> bool {
        let after = Duration::from_secs(self.config.unload_after_idle_secs);
        match self.idle_embedder() {
            Some(idle) => idle.release_if_idle(after),
            None => false,
        }
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
            *self.idle.write().unwrap() = None;
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
                Ok(embedder) => {
                    // Wrapped so that it can be put down after idling (D103). The loader
                    // re-reads nothing from `self` at reload time except what it was built
                    // with: a config change still goes through `load()`, as before.
                    let reload = self.onnx_loader(spec);
                    let idle = Arc::new(IdleEmbedder::new(embedder, reload));
                    *self.idle.write().unwrap() = Some(Arc::clone(&idle));
                    (Arc::new(IdleHandle(idle)) as Arc<dyn Embedder>, spec.dim)
                }
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

    /// A closure that loads the same model the same way, for the reload after an idle release.
    fn onnx_loader(
        &self,
        spec: &'static ModelSpec,
    ) -> impl Fn() -> std::result::Result<Arc<dyn Embedder>, String> + Send + Sync + 'static {
        let runtime = self.runtime_path();
        let model_dir = self.store.model_dir(spec);
        let config = self.config.clone();
        move || load_onnx_from(&runtime, &model_dir, spec, &config)
    }

    fn load_onnx(&self, spec: &'static ModelSpec) -> std::result::Result<Arc<dyn Embedder>, String> {
        load_onnx_from(&self.runtime_path(), &self.store.model_dir(spec), spec, &self.config)
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

#[cfg(feature = "onnx")]
fn load_onnx_from(
    runtime: &std::path::Path,
    model_dir: &std::path::Path,
    spec: &'static ModelSpec,
    config: &EmbeddingConfig,
) -> std::result::Result<Arc<dyn Embedder>, String> {
    use cortex_embed::onnx::{Device, OnnxEmbedder};

    let device = match config.device.as_str() {
        "cpu" => Device::Cpu,
        "cuda" => Device::Cuda,
        _ => Device::Auto,
    };
    let threads = if config.threads == 0 {
        // Zero means "let the runtime decide", which it does by using every core. Measured
        // on the reference workstation that is slower than eight threads, but it is the
        // caller's choice to make.
        0
    } else {
        config.threads
    };

    OnnxEmbedder::load(runtime, model_dir, *spec, device, threads, config.gpu_memory_mb)
        .map(|e| Arc::new(e) as Arc<dyn Embedder>)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "onnx"))]
fn load_onnx_from(
    _runtime: &std::path::Path,
    _model_dir: &std::path::Path,
    _spec: &'static ModelSpec,
    _config: &EmbeddingConfig,
) -> std::result::Result<Arc<dyn Embedder>, String> {
    Err("this build has no ONNX support; set embedding.model = \"hashing\"".into())
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

    /// A stand-in whose loads can be counted, so the release and the reload are observable.
    struct Counting(Arc<std::sync::atomic::AtomicUsize>);

    impl Embedder for Counting {
        fn model(&self) -> cortex_embed::ModelId {
            cortex_embed::ModelId { id: "fake".into(), version: "1".into(), dim: 4 }
        }
        fn embed(&self, inputs: &[cortex_embed::Input<'_>]) -> cortex_embed::Result<Vec<Vec<f32>>> {
            Ok(inputs.iter().map(|_| vec![1.0, 0.0, 0.0, 0.0]).collect())
        }
        fn backend(&self) -> String {
            "fake".into()
        }
    }

    fn idle_embedder() -> (Arc<IdleEmbedder>, Arc<std::sync::atomic::AtomicUsize>) {
        let loads = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let counter = Arc::clone(&loads);
        let first: Arc<dyn Embedder> = Arc::new(Counting(Arc::clone(&loads)));
        let idle = Arc::new(IdleEmbedder::new(first, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Counting(Arc::clone(&counter))) as Arc<dyn Embedder>)
        }));
        (idle, loads)
    }

    #[test]
    fn zero_means_the_model_is_never_released() {
        let (idle, _) = idle_embedder();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!idle.release_if_idle(Duration::ZERO));
        assert!(idle.resident());
    }

    #[test]
    fn a_model_nobody_used_is_released_and_a_query_does_not_wait_for_it() {
        // D103. The release happens; the next query is refused at once with a reason rather
        // than blocking on a reload; the reload happens in the background; the stage's own
        // call blocks until it is back.
        let (idle, loads) = idle_embedder();
        std::thread::sleep(Duration::from_millis(5));
        assert!(idle.release_if_idle(Duration::from_millis(1)));
        assert!(!idle.resident());
        assert_eq!(idle.releases(), 1);

        let handle = IdleHandle(Arc::clone(&idle));
        let started = Instant::now();
        let answer = handle.embed(&[cortex_embed::Input::Query("x")]);
        assert!(answer.is_err(), "a query on a released model is skipped, not served");
        assert!(started.elapsed() < Duration::from_millis(50), "and it did not wait");
        assert!(handle.backend().contains("released"));

        idle.ensure_resident().unwrap();
        assert!(idle.resident());
        assert!(handle.embed(&[cortex_embed::Input::Query("x")]).is_ok());
        // One reload, whether the background thread or the blocking call got there first.
        for _ in 0..100 {
            if !idle.loading.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(loads.load(Ordering::SeqCst), 2, "the model was loaded exactly once more");
    }

    #[test]
    fn a_model_in_use_is_not_released() {
        let (idle, _) = idle_embedder();
        let handle = IdleHandle(Arc::clone(&idle));
        handle.embed(&[cortex_embed::Input::Query("x")]).unwrap();
        assert!(!idle.release_if_idle(Duration::from_secs(60)));
        assert!(idle.resident());
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
