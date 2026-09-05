//! A batch handed to a declared subprocess (D9, D17).
//!
//! One process per batch, JSON-RPC over stdio, one request and one response: the plugin reads
//! a `sync.push` request on stdin, answers on stdout, and exits. A process per batch costs a
//! few milliseconds against a batch interval of a second, and buys crash isolation and the
//! certainty that a plugin holds no state the Core cannot see — if it wants a connection kept
//! open, that is its business, and it can keep one in a daemon of its own.
//!
//! What the plugin does with the batch is not the Core's concern and is not visible to it: the
//! declaration names a command and nothing else (`cortex_core::plugins`).

use crate::protocol::{SyncAck, SyncBatch, METHOD_PUSH};
use crate::transport::{Transport, TransportError};
use cortex_core::plugins::PluginDecl;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

pub struct Plugin {
    name: String,
    decl: PluginDecl,
}

impl Plugin {
    pub fn new(name: impl Into<String>, decl: PluginDecl) -> Self {
        Self {
            name: name.into(),
            decl,
        }
    }
}

impl Transport for Plugin {
    fn name(&self) -> String {
        format!("plugin:{}", self.name)
    }

    fn push(&mut self, batch: &SyncBatch) -> Result<SyncAck, TransportError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": METHOD_PUSH,
            "params": batch,
        });

        let mut child = Command::new(&self.decl.command)
            .args(&self.decl.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| TransportError::Unreachable(format!("could not start plugin: {e}")))?;

        // Write on this thread, read on another, so a plugin that answers before it has read
        // everything cannot deadlock against a full pipe.
        let stdout = child.stdout.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let mut reader = BufReader::new(stdout);
            let read = reader.read_line(&mut line);
            read.map(|_| line)
        });

        let written = (|| -> std::io::Result<()> {
            let mut stdin = child.stdin.take().expect("piped");
            serde_json::to_writer(&mut stdin, &request)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            stdin.write_all(b"\n")?;
            stdin.flush()
        })();
        // A plugin that exits before reading closes the pipe; that is a refusal we will see in
        // the exit status, not an unreachable peer.
        let _ = written;

        let deadline = std::time::Instant::now() + Duration::from_millis(self.decl.timeout_ms);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(TransportError::Unreachable(format!(
                        "plugin did not answer within {} ms",
                        self.decl.timeout_ms
                    )));
                }
                Err(e) => return Err(TransportError::Unreachable(e.to_string())),
            }
        };

        let line = reader
            .join()
            .map_err(|_| TransportError::Unreachable("reader thread panicked".into()))?
            .map_err(|e| TransportError::Unreachable(e.to_string()))?;

        if line.trim().is_empty() {
            return Err(TransportError::Unreachable(format!(
                "plugin exited with {status} without answering"
            )));
        }
        let response: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|e| TransportError::Malformed(format!("{e}: {}", line.trim())))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("no message")
                .to_string();
            return Err(TransportError::Refused(message));
        }
        let result = response
            .get("result")
            .ok_or_else(|| TransportError::Malformed("neither result nor error".into()))?;
        serde_json::from_value(result.clone())
            .map_err(|e| TransportError::Malformed(format!("result is not an ack: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;
    use std::path::PathBuf;

    fn decl(command: &str, args: &[&str], timeout_ms: u64) -> PluginDecl {
        PluginDecl::from_toml(
            &format!(
                "kind = \"sync\"\ncommand = {command:?}\nargs = [{}]\ntimeout_ms = {timeout_ms}\n",
                args.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(", ")
            ),
            &PathBuf::from("<test>"),
        )
        .unwrap()
    }

    fn empty_batch() -> SyncBatch {
        SyncBatch {
            protocol: PROTOCOL_VERSION,
            origin: "o".into(),
            lamport: 0,
            sent_at: 0,
            events: vec![],
        }
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_unreachable_peer() {
        let mut p = Plugin::new("x", decl("/nonexistent/hippocampus-plugin", &[], 1000));
        assert!(matches!(
            p.push(&empty_batch()),
            Err(TransportError::Unreachable(_))
        ));
    }

    #[test]
    fn a_plugin_that_never_answers_is_killed_at_the_deadline() {
        let started = std::time::Instant::now();
        let mut p = Plugin::new("x", decl("sleep", &["30"], 200));
        assert!(matches!(
            p.push(&empty_batch()),
            Err(TransportError::Unreachable(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn an_error_response_is_a_refusal_and_a_result_is_an_ack() {
        let mut refusing = Plugin::new(
            "x",
            decl(
                "sh",
                &["-c", r#"cat >/dev/null; echo '{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"busy"}}'"#],
                2000,
            ),
        );
        match refusing.push(&empty_batch()) {
            Err(TransportError::Refused(m)) => assert_eq!(m, "busy"),
            other => panic!("{other:?}"),
        }
        let mut acking = Plugin::new(
            "x",
            decl(
                "sh",
                &["-c", r#"cat >/dev/null; echo '{"jsonrpc":"2.0","id":1,"result":{"accepted":["a"],"quiet_for_ms":500}}'"#],
                2000,
            ),
        );
        let ack = acking.push(&empty_batch()).unwrap();
        assert_eq!(ack.accepted, vec!["a"]);
        assert_eq!(ack.quiet_for_ms, Some(500));
    }
}
