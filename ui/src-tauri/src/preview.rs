//! Supervising the thumbnailer.
//!
//! The previewer is a child process that is *expected* to die (D14): it reads untrusted bytes
//! with third-party decoders, and isolating it is only worth anything if its death is an
//! ordinary event rather than an incident. So there is no health check and no restart policy —
//! a request finds the process missing, starts one, and carries on. Killing it mid-preview
//! costs the preview and nothing else, which is what M4 §6 asks for.
//!
//! One request at a time. Previews are for the selected row, the selection is singular, and a
//! queue would mostly hold requests for rows the person has already scrolled past.

use serde::Deserialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("the thumbnailer could not be started: {0}")]
    Unavailable(String),

    #[error("{0}")]
    Refused(String),

    #[error("the thumbnailer stopped before it answered")]
    Died,
}

impl serde::Serialize for PreviewError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// What the pane gets back.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Preview {
    pub path: PathBuf,
    pub png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub kind: String,
    pub size: String,
    pub cached: bool,
    pub elapsed_ms: u64,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

/// The supervisor. Cheap to clone; one process behind it.
#[derive(Clone)]
pub struct Thumbnailer {
    binary: PathBuf,
    process: Arc<Mutex<Option<Process>>>,
}

impl Thumbnailer {
    /// Find the thumbnailer beside whatever binary is running.
    ///
    /// Beside, rather than on `PATH`: a development build and an installed one must not be able
    /// to pair up by accident, because the protocol between them is not versioned.
    pub fn beside_this_binary() -> Self {
        let binary = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("cortex-preview")))
            .unwrap_or_else(|| PathBuf::from("cortex-preview"));
        Self::at(binary)
    }

    /// A supervisor for a thumbnailer at a known path. Tests, and anyone packaging this
    /// somewhere the two binaries do not sit together.
    pub fn at(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            process: Arc::new(Mutex::new(None)),
        }
    }

    /// Kill the running process, if any. The next request starts a new one.
    ///
    /// Exists so that M4 §6 can be tested rather than argued about, and because a person
    /// watching a preview take too long should be able to give up on it.
    pub fn kill(&self) {
        let mut guard = self.process.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut process) = guard.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }

    /// Ask for a preview, starting the process if it is not running.
    ///
    /// `on_partial` receives the smaller cached thumbnail when there is one — before this
    /// returns, and often long before, which is the entire point of the progressive contract.
    pub fn preview(
        &self,
        path: &Path,
        pixels: u32,
        on_partial: impl Fn(Value),
    ) -> Result<Preview, PreviewError> {
        let mut guard = self.process.lock().unwrap_or_else(|p| p.into_inner());

        if guard.is_none() {
            *guard = Some(self.spawn()?);
        }

        let request = {
            let process = guard.as_mut().expect("just started");
            let id = process.next_id;
            process.next_id += 1;
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "preview",
                "params": { "path": path.display().to_string(), "pixels": pixels }
            })
        };

        match self.exchange(guard.as_mut().expect("just started"), &request, &on_partial) {
            Ok(answer) => decode(answer),
            Err(PreviewError::Died) => {
                // It died. That is allowed, and the honest thing is to say the preview failed
                // rather than to retry silently — a retry that also dies looks like a hang.
                *guard = None;
                Err(PreviewError::Died)
            }
            Err(other) => Err(other),
        }
    }

    fn spawn(&self) -> Result<Process, PreviewError> {
        let mut child = Command::new(&self.binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PreviewError::Unavailable(format!("{}: {e}", self.binary.display()))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            PreviewError::Unavailable("the thumbnailer has no standard input".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PreviewError::Unavailable("the thumbnailer has no standard output".into())
        })?;

        Ok(Process {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn exchange(
        &self,
        process: &mut Process,
        request: &Value,
        on_partial: &impl Fn(Value),
    ) -> Result<Value, PreviewError> {
        writeln!(process.stdin, "{request}").map_err(|_| PreviewError::Died)?;
        process.stdin.flush().map_err(|_| PreviewError::Died)?;

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        loop {
            let mut line = String::new();
            match process.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => return Err(PreviewError::Died),
                Ok(_) => {}
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id") == Some(&id) {
                return Ok(value);
            }
            if value.get("method").and_then(|m| m.as_str()) == Some("preview.partial") {
                on_partial(value.get("params").cloned().unwrap_or(Value::Null));
            }
        }
    }

    /// Stop the process if one is running. Called when the window closes.
    pub fn stop(&self) {
        let mut guard = self.process.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut process) = guard.take() {
            let _ = writeln!(
                process.stdin,
                r#"{{"jsonrpc":"2.0","id":0,"method":"shutdown"}}"#
            );
            let _ = process.child.wait();
        }
    }

    /// Whether a process is currently running. For the status line and for tests.
    pub fn is_running(&self) -> bool {
        self.process
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }
}

fn decode(answer: Value) -> Result<Preview, PreviewError> {
    if let Some(error) = answer.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("the thumbnailer refused without saying why");
        return Err(PreviewError::Refused(message.to_string()));
    }
    serde_json::from_value(answer.get("result").cloned().unwrap_or(Value::Null))
        .map_err(|e| PreviewError::Refused(format!("unreadable answer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_thumbnailer_is_named_rather_than_hung() {
        let thumbnailer = Thumbnailer::at("/nonexistent/cortex-preview");
        let error = thumbnailer
            .preview(Path::new("/tmp/whatever.png"), 128, |_| {})
            .unwrap_err();

        assert!(matches!(error, PreviewError::Unavailable(_)));
        assert!(
            error.to_string().contains("cortex-preview"),
            "the error must name what is missing: {error}"
        );
        assert!(!thumbnailer.is_running());
    }
}
