//! The reference peer (M7). Ships in the repository so that the federation tests assert
//! against a real process over the real protocol, and so that anybody writing a bridge has a
//! working example of the receiving end that fits on one screen.
//!
//! It is deliberately a *dumb* peer, the one D18 assumes: it keeps a set of event ids and a
//! log of what it received, and does nothing clever. Idempotency is the sender's job; this
//! peer only makes it observable — the `seen` count on every event says how often it arrived.
//!
//! Two modes:
//!
//! - `hc-test-peer push --state FILE`: read one JSON-RPC `sync.push` request on stdin, record
//!   the batch in FILE, answer on stdout. This is what the plugin transport runs.
//! - `hc-test-peer ingest --state FILE --dir DIR`: consume every `*.json` document in DIR the
//!   way a file-drop peer would on its own schedule, then delete it.
//!
//! Faults, for the tests: `--reject SUBSTRING` refuses events whose payload contains it,
//! `--quiet-ms N` asks for quiet, `--down` exits without answering, `--lamport N` reports a
//! clock, `--ontology FILE` sends a taxonomy down.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "hc-test-peer", about = "The reference federation peer: dumb on purpose.")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Answer one `sync.push` request from stdin.
    Push {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        reject: Option<String>,
        #[arg(long)]
        quiet_ms: Option<u64>,
        #[arg(long)]
        down: bool,
        #[arg(long)]
        lamport: Option<i64>,
        #[arg(long)]
        ontology: Option<PathBuf>,
    },
    /// Consume a file-drop directory.
    Ingest {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        dir: PathBuf,
    },
    /// Print the state as JSON.
    Show {
        #[arg(long)]
        state: PathBuf,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Every event ever received, by id, with how often.
    events: BTreeMap<String, Seen>,
    /// Ids in arrival order, duplicates included.
    arrivals: Vec<String>,
    /// One entry per batch: how many events, and the sender's clock.
    batches: Vec<BatchSeen>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Seen {
    seen: u64,
    lamport: i64,
    kind: String,
    ts: i64,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct BatchSeen {
    events: usize,
    lamport: i64,
    sent_at: i64,
}

fn load(path: &Path) -> Result<State> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.into()),
    }
}

fn save(path: &Path, state: &State) -> Result<()> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, serde_json::to_vec(state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn receive(state: &mut State, event: &serde_json::Value, reject: Option<&str>) -> Result<(), String> {
    let id = event
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("no event_id")?
        .to_string();
    let payload = event.get("payload").cloned().unwrap_or(serde_json::Value::Null);
    if let Some(needle) = reject {
        if payload.to_string().contains(needle) {
            return Err(format!("contains {needle:?}"));
        }
    }
    state.arrivals.push(id.clone());
    let entry = state.events.entry(id).or_insert_with(|| Seen {
        seen: 0,
        lamport: event.get("lamport").and_then(|v| v.as_i64()).unwrap_or(0),
        kind: event
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        ts: event.get("ts").and_then(|v| v.as_i64()).unwrap_or(0),
        payload,
    });
    entry.seen += 1;
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Push {
            state: state_path,
            reject,
            quiet_ms,
            down,
            lamport,
            ontology,
        } => {
            if down {
                std::process::exit(3);
            }
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            let request: serde_json::Value =
                serde_json::from_str(line.trim()).context("the request is not JSON")?;
            let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
            if request.get("method").and_then(|m| m.as_str()) != Some("sync.push") {
                let answer = serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": "only sync.push is served here" }
                });
                println!("{answer}");
                return Ok(());
            }
            let batch = request.get("params").cloned().unwrap_or_default();
            if batch.get("protocol").and_then(|v| v.as_i64()) != Some(1) {
                let answer = serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": 1, "message": "unknown protocol version" }
                });
                println!("{answer}");
                return Ok(());
            }

            let mut state = load(&state_path)?;
            let events = batch
                .get("events")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut accepted = Vec::new();
            let mut rejected = Vec::new();
            for event in &events {
                match receive(&mut state, event, reject.as_deref()) {
                    Ok(()) => accepted.push(event["event_id"].clone()),
                    Err(reason) => rejected.push(serde_json::json!({
                        "event_id": event["event_id"], "reason": reason
                    })),
                }
            }
            state.batches.push(BatchSeen {
                events: events.len(),
                lamport: batch.get("lamport").and_then(|v| v.as_i64()).unwrap_or(0),
                sent_at: batch.get("sent_at").and_then(|v| v.as_i64()).unwrap_or(0),
            });
            save(&state_path, &state)?;

            let mut result = serde_json::json!({ "accepted": accepted, "rejected": rejected });
            if let Some(q) = quiet_ms {
                result["quiet_for_ms"] = q.into();
            }
            if let Some(l) = lamport {
                result["lamport"] = l.into();
            }
            if let Some(path) = ontology {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                result["ontology"] = serde_json::from_str(&text)?;
            }
            let answer = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            let mut out = std::io::stdout().lock();
            writeln!(out, "{answer}")?;
            out.flush()?;
        }
        Command::Ingest { state: state_path, dir } => {
            let mut state = load(&state_path)?;
            let mut docs: Vec<PathBuf> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                .filter(|p| p.file_name().map(|n| n != "ontology.json").unwrap_or(true))
                .collect();
            // A file-drop peer sees files in whatever order its watcher hands them over; the
            // sender's clock is inside each document, and that is what order means.
            docs.sort();
            let mut events: Vec<serde_json::Value> = docs
                .iter()
                .map(|p| -> Result<serde_json::Value> {
                    Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
                })
                .collect::<Result<_>>()?;
            events.sort_by_key(|e| e.get("lamport").and_then(|v| v.as_i64()).unwrap_or(0));
            for event in &events {
                let _ = receive(&mut state, event, None);
            }
            state.batches.push(BatchSeen {
                events: events.len(),
                lamport: events
                    .iter()
                    .filter_map(|e| e.get("lamport").and_then(|v| v.as_i64()))
                    .max()
                    .unwrap_or(0),
                sent_at: 0,
            });
            save(&state_path, &state)?;
            for doc in docs {
                std::fs::remove_file(doc)?;
            }
            println!("{}", events.len());
        }
        Command::Show { state } => {
            println!("{}", serde_json::to_string_pretty(&load(&state)?)?);
        }
    }
    Ok(())
}
