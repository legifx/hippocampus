//! A thin client for the daemon socket.
//!
//! Used by the CLI, by the MCP server and by plugins. There is no second, richer client: if a
//! capability is missing here it is missing from the protocol, which is the point.

use crate::protocol::{Request, Response, RpcError, Version};
use crate::transport::{self, TransportError};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::BufReader;
use tokio::net::UnixStream;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not reach the daemon at {path}: {source}")]
    Connect {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error("daemon returned an error: {} (code {})", .0.message, .0.code)]
    Rpc(RpcError),

    #[error("daemon sent a response for a different request (expected {expected}, got {got})")]
    IdMismatch { expected: String, got: String },

    #[error("daemon closed the connection without answering")]
    NoResponse,

    #[error("could not decode the result: {0}")]
    Decode(#[from] serde_json::Error),
}

pub struct Client {
    stream: BufReader<UnixStream>,
    next_id: AtomicU64,
    /// Notifications that arrived while waiting for a response.
    notifications: std::collections::VecDeque<serde_json::Value>,
}

impl Client {
    pub async fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).await.map_err(|source| ClientError::Connect {
            path: socket.display().to_string(),
            source,
        })?;
        Ok(Self {
            stream: BufReader::new(stream),
            next_id: AtomicU64::new(1),
            notifications: std::collections::VecDeque::new(),
        })
    }

    /// Issue a call and decode its result.
    pub async fn call<P, R>(&mut self, method: &str, params: P) -> Result<R, ClientError>
    where
        P: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            jsonrpc: Version,
            id: Some(serde_json::json!(id)),
            method: method.to_string(),
            params: Some(serde_json::to_value(params)?),
        };

        transport::write_message(self.stream.get_mut(), &serde_json::to_value(&request)?).await?;

        // The daemon may send notifications between a request and its answer — a reranked
        // result list arriving for an *earlier* search, for instance. They are queued rather
        // than discarded, because the caller asked for them and the alternative is a protocol
        // where the second answer only arrives if nobody asks anything in the meantime.
        let response = loop {
            let value = transport::read_message(&mut self.stream)
                .await?
                .ok_or(ClientError::NoResponse)?;
            if value.get("id").is_none() {
                self.notifications.push_back(value);
                continue;
            }
            break serde_json::from_value::<Response>(value)?;
        };

        // A mismatched id means the stream is out of step; continuing would silently return
        // one call's answer for another.
        if response.id != serde_json::json!(id) {
            return Err(ClientError::IdMismatch {
                expected: id.to_string(),
                got: response.id.to_string(),
            });
        }

        if let Some(error) = response.error {
            return Err(ClientError::Rpc(error));
        }

        let result = response.result.unwrap_or(serde_json::Value::Null);
        Ok(serde_json::from_value(result)?)
    }

    /// Wait for the daemon to send an unsolicited update, up to `timeout`.
    ///
    /// This is how a result that improves after it was delivered reaches the client (decision
    /// D7): the search answers immediately with what it has, and the reranked order follows on
    /// the same connection. Returns `None` on timeout, which is not an error — the first answer
    /// was already a complete one, and a client is free to stop waiting.
    pub async fn next_notification(
        &mut self,
        method: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<serde_json::Value>, ClientError> {
        // Anything that arrived while an earlier call was in flight.
        while let Some(queued) = self.notifications.pop_front() {
            if queued.get("method").and_then(|m| m.as_str()) == Some(method) {
                return Ok(queued.get("params").cloned());
            }
        }

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match tokio::time::timeout(remaining, transport::read_message(&mut self.stream)).await {
                Err(_) => return Ok(None),
                Ok(Ok(None)) => return Ok(None),
                Ok(Err(e)) => return Err(e.into()),
                Ok(Ok(Some(value))) => {
                    if value.get("method").and_then(|m| m.as_str()) == Some(method) {
                        return Ok(value.get("params").cloned());
                    }
                    // Something else — a response to a call nobody is waiting for any more.
                    // Keep it rather than dropping it, so the stream stays interpretable.
                    self.notifications.push_back(value);
                }
            }
        }
    }
}
