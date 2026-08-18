//! `multilingual-e5-small` through ONNX Runtime.
//!
//! ## What the model actually gives back
//!
//! One tensor: `last_hidden_state`, shaped `[batch, tokens, 384]`. Everything that makes it a
//! sentence embedding happens here — **mean pooling over the unpadded tokens, then L2
//! normalisation**. Both steps are silent when wrong. Pooling over padding drags every vector
//! towards the padding token, and skipping normalisation leaves the vector index comparing
//! magnitudes it was told were angles. Neither produces an error; both produce worse recall
//! that looks like a bad model.
//!
//! ## Why the runtime is loaded by path
//!
//! `ort` is built without `download-binaries`, so nothing is fetched while compiling and the
//! build needs no system OpenSSL. The library is opened at startup from wherever the model
//! store put it (or from `embedding.runtime_path`). Consequences worth knowing: the runtime is
//! provisioned like the model, under the same checksum discipline, and a daemon that has
//! neither simply runs without a vector arm.
//!
//! ## GPU
//!
//! Decision D6 puts embeddings on the local GPU. The CUDA execution provider is requested when
//! configured and the session falls back to CPU when it cannot be registered — which is what
//! happens on a machine with a driver but no CUDA runtime, and is reported rather than hidden.

use crate::registry::ModelSpec;
use crate::{normalise, EmbedError, Embedder, Input, ModelId, Result};
use ort::session::Session;
use ort::value::Value;
use std::path::Path;
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Which device to try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// Try the GPU, fall back to the CPU. The default, and D6's intent.
    #[default]
    Auto,
    Cpu,
    /// Refuse to start without the GPU. For measuring, not for daily use.
    Cuda,
}

pub struct OnnxEmbedder {
    // One session, one lock. ONNX Runtime sessions are thread-safe for concurrent `run`, but
    // the input tensors are built per call anyway, and embedding happens on one thread by
    // design — the queue is throttled, not parallel.
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    spec: ModelSpec,
    backend: String,
}

impl OnnxEmbedder {
    /// Load the runtime, then the model.
    ///
    /// `runtime` is the path to `libonnxruntime.so`. It is loaded once per process; a second
    /// call with a different path is ignored by `ort`, which is why the daemon resolves it
    /// before anything else starts.
    pub fn load(
        runtime: &Path,
        model_dir: &Path,
        spec: ModelSpec,
        device: Device,
        threads: usize,
    ) -> Result<Self> {
        if !runtime.exists() {
            return Err(EmbedError::NotProvisioned(format!(
                "{} (run `cortex model fetch`)",
                runtime.display()
            )));
        }
        let model_path = model_dir.join("model.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");
        for path in [&model_path, &tokenizer_path] {
            if !path.exists() {
                return Err(EmbedError::NotProvisioned(format!(
                    "{} (run `cortex model fetch`)",
                    path.display()
                )));
            }
        }

        ort::init_from(runtime.to_string_lossy().as_ref())
            .map_err(|e| EmbedError::Runtime {
                path: runtime.display().to_string(),
                message: e.to_string(),
            })?
            .commit();

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let (session, backend) = build_session(&model_path, device, threads)?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            spec,
            backend,
        })
    }

    fn encode(&self, inputs: &[Input<'_>]) -> Result<(Vec<i64>, Vec<i64>, usize, usize)> {
        // The prefix is the model's, not the caller's. Getting it wrong costs recall and
        // reports nothing, which is why `Input` carries the intent instead of the text.
        let prefixed: Vec<String> = inputs
            .iter()
            .map(|input| format!("{}{}", self.spec.prefix(*input), input.text()))
            .collect();

        let encodings = self
            .tokenizer
            .encode_batch(prefixed, true)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let batch = encodings.len();
        let sequence = encodings
            .iter()
            .map(|e| e.len().min(self.spec.max_tokens))
            .max()
            .unwrap_or(1)
            .max(1);

        let mut ids = vec![0i64; batch * sequence];
        let mut mask = vec![0i64; batch * sequence];
        for (row, encoding) in encodings.iter().enumerate() {
            let tokens = encoding.get_ids().iter().zip(encoding.get_attention_mask());
            for (column, (id, attention)) in tokens.take(sequence).enumerate() {
                ids[row * sequence + column] = *id as i64;
                mask[row * sequence + column] = *attention as i64;
            }
        }
        Ok((ids, mask, batch, sequence))
    }
}

impl Embedder for OnnxEmbedder {
    fn model(&self) -> ModelId {
        ModelId {
            id: self.spec.id.to_string(),
            version: self.spec.version.to_string(),
            dim: self.spec.dim,
        }
    }

    fn embed(&self, inputs: &[Input<'_>]) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let (ids, mask, batch, sequence) = self.encode(inputs)?;
        let types = vec![0i64; batch * sequence];

        let shape = [batch, sequence];
        // The output tensor borrows the session, so the copy out of it happens while the lock
        // is still held rather than by trying to outlive it.
        let (width, hidden) = {
            let mut session = self.session.lock().unwrap();
            let outputs = session
                .run(ort::inputs![
                    "input_ids" => Value::from_array((shape, ids)).map_err(inference)?,
                    "attention_mask" => Value::from_array((shape, mask.clone())).map_err(inference)?,
                    "token_type_ids" => Value::from_array((shape, types)).map_err(inference)?
                ])
                .map_err(inference)?;
            let (dims, data) = outputs["last_hidden_state"]
                .try_extract_tensor::<f32>()
                .map_err(inference)?;
            (*dims.last().unwrap_or(&0) as usize, data.to_vec())
        };
        let data = hidden;

        if width != self.spec.dim {
            return Err(EmbedError::Inference(format!(
                "model produced {width} dimensions, expected {}",
                self.spec.dim
            )));
        }

        let mut vectors = Vec::with_capacity(batch);
        for row in 0..batch {
            let mut vector = vec![0f32; width];
            let mut counted = 0f32;
            for token in 0..sequence {
                // Padding is not text. Averaging over it pulls every short input towards the
                // same point, and short inputs are most of what a search engine sees.
                if mask[row * sequence + token] == 0 {
                    continue;
                }
                counted += 1.0;
                let offset = (row * sequence + token) * width;
                for (d, value) in vector.iter_mut().enumerate() {
                    *value += data[offset + d];
                }
            }
            if counted > 0.0 {
                for value in vector.iter_mut() {
                    *value /= counted;
                }
            }
            normalise(&mut vector);
            vectors.push(vector);
        }

        Ok(vectors)
    }

    fn max_batch(&self) -> usize {
        // Padding is per batch, so a batch mixing a one-line note with a full chunk pays the
        // long one's cost for both. Sixteen keeps that waste bounded while still amortising the
        // per-call overhead.
        16
    }

    fn backend(&self) -> String {
        self.backend.clone()
    }
}

/// Build the session, trying the GPU first when asked to.
fn build_session(model: &Path, device: Device, threads: usize) -> Result<(Session, String)> {
    let cpu = |reason: Option<String>| -> Result<(Session, String)> {
        let mut builder = Session::builder()
            .map_err(inference)?
            .with_intra_threads(threads)
            .map_err(inference)?;
        let session = builder.commit_from_file(model).map_err(inference)?;
        let mut backend = format!("onnx, cpu ({threads} threads)");
        if let Some(reason) = reason {
            backend.push_str(&format!(" — no gpu: {reason}"));
        }
        Ok((session, backend))
    };

    match device {
        Device::Cpu => cpu(None),
        Device::Auto | Device::Cuda => {
            let attempt = (|| {
                let mut builder = Session::builder()?
                    .with_execution_providers([ort::ep::CUDA::default().build().error_on_failure()])?
                    .with_intra_threads(threads)?;
                builder.commit_from_file(model)
            })();

            match attempt {
                Ok(session) => Ok((session, "onnx, cuda".to_string())),
                Err(e) if device == Device::Cuda => Err(EmbedError::Inference(format!(
                    "the CUDA execution provider was required but could not be used: {e}"
                ))),
                Err(e) => {
                    // The expected case on a machine with a driver but no CUDA runtime. Say so
                    // once, in the backend string, so `cortex status` can show why the first
                    // index run is taking hours.
                    tracing::info!(error = %e, "CUDA unavailable; embedding on the CPU");
                    cpu(Some(short_reason(&e.to_string())))
                }
            }
        }
    }
}

/// ONNX Runtime's provider errors are several lines of C++ detail. Keep the first sentence.
fn short_reason(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or(message)
        .chars()
        .take(120)
        .collect()
}

fn inference(e: impl std::fmt::Display) -> EmbedError {
    EmbedError::Inference(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_without_a_runtime_says_what_to_run() {
        // The failure a fresh install hits, and the only useful thing to say about it.
        let tmp = tempfile::tempdir().unwrap();
        let error = match OnnxEmbedder::load(
            &tmp.path().join("libonnxruntime.so"),
            tmp.path(),
            crate::E5_SMALL,
            Device::Cpu,
            1,
        ) {
            Err(e) => e,
            Ok(_) => panic!("loading without a runtime must fail"),
        };

        let message = error.to_string();
        assert!(matches!(error, EmbedError::NotProvisioned(_)));
        assert!(
            message.contains("cortex model fetch"),
            "the error must name the command that fixes it: {message}"
        );
    }

    #[test]
    fn loading_without_a_model_says_which_file_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("libonnxruntime.so");
        std::fs::write(&runtime, b"not really a library").unwrap();

        let error = match OnnxEmbedder::load(&runtime, tmp.path(), crate::E5_SMALL, Device::Cpu, 1)
        {
            Err(e) => e,
            Ok(_) => panic!("loading without a model must fail"),
        };
        assert!(error.to_string().contains("model.onnx"), "{error}");
    }

    #[test]
    fn a_multi_line_provider_error_is_shortened_to_something_printable() {
        let long = "Failed to find CUDA\nstack trace line\nanother line";
        assert_eq!(short_reason(long), "Failed to find CUDA");
    }
}
