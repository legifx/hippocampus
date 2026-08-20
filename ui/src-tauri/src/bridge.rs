//! The connection to the daemon.
//!
//! Not `cortex_rpc::client::Client`, and the reason matters. That client is a request/response
//! client: it holds the socket while it waits for an answer. This surface cannot work that way,
//! because a search's *better* answer arrives seventeen milliseconds after its first one, on
//! the same connection, unsolicited (D7) — and waiting for it would put those seventeen
//! milliseconds in front of the next keystroke, which has an eighty-millisecond budget for the
//! whole round trip.
//!
//! So the socket is split. A reader task owns the read half and dispatches every line: a
//! response goes to whoever is waiting for that id, and a notification becomes a window event.
//! Calls are pipelined — the next keystroke's search goes out while the previous one's
//! reranked order is still on its way back.

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::oneshot;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("no daemon is listening at {path}. Start it with `cortexd`.")]
    NoDaemon { path: String },

    #[error("the connection to the daemon was lost")]
    Disconnected,

    #[error("{message} (code {code})")]
    Daemon { code: i64, message: String },

    #[error("could not read the daemon's answer: {0}")]
    Decode(String),

    #[error("i/o error: {0}")]
    Io(String),
}

impl serde::Serialize for BridgeError {
    /// Errors cross into JavaScript as an object rather than a string, so the frontend can
    /// branch on a refusal's code — "widen the allow-list" is a different reaction from
    /// "the daemon is not running", and a flattened message makes them indistinguishable.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut out = s.serialize_struct("BridgeError", 2)?;
        match self {
            BridgeError::Daemon { code, .. } => out.serialize_field("code", code)?,
            BridgeError::NoDaemon { .. } => out.serialize_field("code", &-1)?,
            _ => out.serialize_field("code", &-2)?,
        }
        out.serialize_field("message", &self.to_string())?;
        out.end()
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, BridgeError>>>>>;

/// A live connection. Cloneable and shared across every Tauri command.
///
/// `Debug` prints what is knowable without locking anything: the socket's state is in the
/// reader task, and a `Debug` that took the write lock would deadlock the first time somebody
/// formatted it inside an error path.
#[derive(Clone)]
pub struct Bridge {
    writer: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<UnixStream>>>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge")
            .field(
                "in_flight",
                &self.pending.lock().map(|p| p.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl Bridge {
    /// Connect and start the reader.
    ///
    /// `on_notification` receives everything the daemon sends unsolicited. It is a callback
    /// rather than a channel so that the Tauri layer decides what a notification becomes; this
    /// module does not know what a window is.
    pub async fn connect(
        socket: &Path,
        on_notification: impl Fn(String, Value) + Send + 'static,
    ) -> Result<Self, BridgeError> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|_| BridgeError::NoDaemon {
                path: socket.display().to_string(),
            })?;

        let (reader, writer) = tokio::io::split(stream);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        let reader_pending = pending.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            loop {
                match cortex_rpc::transport::read_message(&mut reader).await {
                    Ok(Some(value)) => dispatch(&reader_pending, &on_notification, value),
                    // A clean close or a broken stream both mean the same thing to everyone
                    // waiting: nobody is going to answer them.
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(error = %e, "the daemon connection failed");
                        break;
                    }
                }
            }
            let waiting: Vec<_> = reader_pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain()
                .map(|(_, tx)| tx)
                .collect();
            for tx in waiting {
                let _ = tx.send(Err(BridgeError::Disconnected));
            }
        });

        Ok(Self {
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    /// How long a call may take before it is worth a line in the journal.
    ///
    /// Not an error — a rebuild legitimately takes seconds. It exists so that "it feels slow"
    /// can be answered with which call was slow, from a log somebody already has, rather than
    /// with a benchmark written after the fact against a fixture that was never slow.
    const SLOW_CALL_MS: u128 = 50;

    /// Issue one call. Several may be in flight at once.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        let started = std::time::Instant::now();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, tx);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_vec(&request).map_err(|e| BridgeError::Decode(e.to_string()))?;
        line.push(b'\n');

        {
            // The lock covers the write only. Holding it across the await for the *answer*
            // would serialise every call and undo the point of splitting the socket.
            let mut writer = self.writer.lock().await;
            if let Err(e) = writer.write_all(&line).await {
                self.pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                return Err(BridgeError::Io(e.to_string()));
            }
            writer.flush().await.map_err(|e| BridgeError::Io(e.to_string()))?;
        }

        let answer = rx.await.map_err(|_| BridgeError::Disconnected)?;

        let elapsed = started.elapsed().as_millis();
        if elapsed >= Self::SLOW_CALL_MS {
            tracing::info!(method, ms = elapsed as u64, "slow call");
        }
        answer
    }

    /// Call and decode in one step.
    pub async fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, BridgeError> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value).map_err(|e| BridgeError::Decode(e.to_string()))
    }
}

fn dispatch(
    pending: &Pending,
    on_notification: &impl Fn(String, Value),
    value: Value,
) {
    // No id means nobody asked for it: a notification. That is the whole rule, and it is the
    // JSON-RPC one rather than something this bridge invented.
    let Some(id) = value.get("id").and_then(|v| v.as_u64()) else {
        if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            on_notification(method.to_string(), params);
        }
        return;
    };

    let Some(tx) = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id)
    else {
        // An answer to a call that was abandoned — a search superseded by a newer one. Dropping
        // it is correct; logging it would be noise at one line per keystroke.
        return;
    };

    let answer = match value.get("error") {
        Some(error) => Err(BridgeError::Daemon {
            code: error.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603),
            message: error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("the daemon reported an error with no message")
                .to_string(),
        }),
        None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = tx.send(answer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;
    use tokio::net::UnixListener;

    /// A daemon that answers on its own schedule, so the tests can pin down the two properties
    /// that matter: calls do not have to come back in order, and an unsolicited message reaches
    /// the notification callback rather than a waiting caller.
    async fn fake_daemon(path: std::path::PathBuf, behaviour: &'static str) {
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = tokio::io::split(stream);
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let request: Value = serde_json::from_str(line.trim()).unwrap();
                line.clear();
                let id = request["id"].clone();
                let method = request["method"].as_str().unwrap_or("").to_string();

                if behaviour == "notify" {
                    let note = serde_json::json!({
                        "jsonrpc": "2.0", "method": "search.update",
                        "params": { "for": method }
                    });
                    let _ = writer
                        .write_all(format!("{note}\n").as_bytes())
                        .await;
                }

                if behaviour == "error" {
                    let response = serde_json::json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": 1002, "message": "outside the allow-list" }
                    });
                    let _ = writer.write_all(format!("{response}\n").as_bytes()).await;
                    continue;
                }

                // Deliberately slow for the first call, so that a second call issued afterwards
                // can overtake it. A bridge that serialised its calls would deadlock this test.
                let delay = if method == "slow" { 120 } else { 0 };
                let mut writer_clone = writer;
                let response = serde_json::json!({
                    "jsonrpc": "2.0", "id": id, "result": { "method": method }
                });
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                let _ = writer_clone
                    .write_all(format!("{response}\n").as_bytes())
                    .await;
                writer = writer_clone;
            }
        });
    }

    #[tokio::test]
    async fn a_missing_daemon_is_named_rather_than_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let err = Bridge::connect(&dir.path().join("nothing.sock"), |_, _| {})
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::NoDaemon { .. }));
        assert!(err.to_string().contains("cortexd"), "the error must say what to do");
    }

    #[tokio::test]
    async fn answers_may_come_back_out_of_order() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cortex.sock");
        fake_daemon(socket.clone(), "ordered").await;

        let bridge = Bridge::connect(&socket, |_, _| {}).await.unwrap();
        let slow = bridge.call("slow", serde_json::json!({}));
        let fast = bridge.call("fast", serde_json::json!({}));

        // The fast one must not be stuck behind the slow one: that is the whole reason this is
        // not the request/response client.
        let (slow, fast) = tokio::join!(slow, fast);
        assert_eq!(slow.unwrap()["method"], "slow");
        assert_eq!(fast.unwrap()["method"], "fast");
    }

    #[tokio::test]
    async fn a_notification_reaches_the_callback_and_not_a_caller() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cortex.sock");
        fake_daemon(socket.clone(), "notify").await;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let bridge = Bridge::connect(&socket, move |method, _| {
            recorder.lock().unwrap().push(method);
        })
        .await
        .unwrap();

        let result = bridge.call("search", serde_json::json!({})).await.unwrap();
        assert_eq!(result["method"], "search", "the notification was returned as the answer");

        // The notification is written before the response, so by the time the call returns it
        // has already been dispatched.
        assert_eq!(seen.lock().unwrap().as_slice(), ["search.update"]);
    }

    #[tokio::test]
    async fn a_refusal_keeps_its_code() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("cortex.sock");
        fake_daemon(socket.clone(), "error").await;

        let bridge = Bridge::connect(&socket, |_, _| {}).await.unwrap();
        let err = bridge.call("fs.write", serde_json::json!({})).await.unwrap_err();
        match err {
            BridgeError::Daemon { code, message } => {
                assert_eq!(code, 1002);
                assert!(message.contains("allow-list"));
            }
            other => panic!("expected a daemon refusal, got {other}"),
        }
    }
}
