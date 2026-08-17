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

        let value = transport::read_message(&mut self.stream)
            .await?
            .ok_or(ClientError::NoResponse)?;
        let response: Response = serde_json::from_value(value)?;

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
}
