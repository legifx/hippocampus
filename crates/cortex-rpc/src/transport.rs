//! Newline-delimited JSON over a Unix socket.
//!
//! One JSON value per line, UTF-8, `\n` terminated. Chosen over a length-prefixed framing
//! because it can be driven by hand with `socat` or `nc` — a protocol you cannot poke at by
//! hand is a protocol you debug badly.

use std::io;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Refuse absurdly long lines rather than allocating for them.
///
/// A client that sends a gigabyte on one line is either broken or hostile; either way the
/// daemon should not grow to hold it.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),

    #[error("message exceeds the {MAX_LINE_BYTES} byte line limit")]
    LineTooLong,

    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("the peer closed the connection")]
    Closed,
}

/// Read one newline-delimited JSON value.
///
/// Returns `Ok(None)` on a clean end of stream. Blank lines are skipped — a client that sends
/// a stray newline should not be disconnected for it.
pub async fn read_message<R>(reader: &mut BufReader<R>) -> Result<Option<serde_json::Value>, TransportError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        if read > MAX_LINE_BYTES {
            return Err(TransportError::LineTooLong);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return Ok(Some(serde_json::from_str(trimmed)?));
    }
}

/// Write one newline-delimited JSON value and flush it.
///
/// Flushing per message is deliberate: this is a request/response protocol on a local socket,
/// and a buffered reply that never leaves is indistinguishable from a hung daemon.
pub async fn write_message<W>(writer: &mut W, value: &serde_json::Value) -> Result<(), TransportError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_LINE_BYTES {
        return Err(TransportError::LineTooLong);
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn read_all(input: &str) -> Vec<serde_json::Value> {
        let mut reader = BufReader::new(input.as_bytes());
        let mut out = Vec::new();
        while let Some(value) = read_message(&mut reader).await.unwrap() {
            out.push(value);
        }
        out
    }

    #[tokio::test]
    async fn reads_one_value_per_line() {
        let values = read_all("{\"a\":1}\n{\"b\":2}\n").await;
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["a"], 1);
        assert_eq!(values[1]["b"], 2);
    }

    #[tokio::test]
    async fn a_clean_end_of_stream_is_not_an_error() {
        let mut reader = BufReader::new(&b""[..]);
        assert!(read_message(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn blank_lines_are_skipped_rather_than_fatal() {
        let values = read_all("\n\n{\"a\":1}\n\n").await;
        assert_eq!(values.len(), 1);
    }

    #[tokio::test]
    async fn a_missing_trailing_newline_still_yields_the_value() {
        let values = read_all("{\"a\":1}").await;
        assert_eq!(values.len(), 1);
    }

    #[tokio::test]
    async fn malformed_json_is_reported_rather_than_skipped() {
        let mut reader = BufReader::new(&b"not json\n"[..]);
        let err = read_message(&mut reader).await.unwrap_err();
        assert!(matches!(err, TransportError::Json(_)));
    }

    #[tokio::test]
    async fn a_written_message_can_be_read_back() {
        let mut buffer: Vec<u8> = Vec::new();
        let value = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
        write_message(&mut buffer, &value).await.unwrap();

        assert_eq!(*buffer.last().unwrap(), b'\n', "messages must be newline terminated");

        let mut reader = BufReader::new(&buffer[..]);
        assert_eq!(read_message(&mut reader).await.unwrap().unwrap(), value);
    }

    #[tokio::test]
    async fn an_oversized_message_is_refused_before_it_is_sent() {
        let mut buffer: Vec<u8> = Vec::new();
        let huge = serde_json::json!({ "s": "x".repeat(MAX_LINE_BYTES) });
        let err = write_message(&mut buffer, &huge).await.unwrap_err();
        assert!(matches!(err, TransportError::LineTooLong));
        assert!(buffer.is_empty(), "nothing must be written when the message is refused");
    }
}
