//! `cortex-mcp` — the Agent Surface as an MCP server.
//!
//! It is a bridge and nothing more: MCP on stdin and stdout, JSON-RPC on the daemon's Unix
//! socket, and a table mapping one tool name to one method. No policy lives here, no tool logic
//! lives here, and that is the design rather than an omission — the daemon is where a refusal
//! is decided, so a bridge that could be talked into deciding differently would be a way round
//! it (M3 §5).
//!
//! **The kill switch reaches this process too** (R7, M3 §4). With the Agent Surface disabled
//! the server does not start: it reports the reason and exits, so a client sees no tools at all
//! rather than a list of tools that all refuse. A surface switched off mid-session refuses each
//! call at the daemon, which the bridge passes through unchanged.

mod tools;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_core::paths::Paths;
use cortex_rpc::client::Client;
use cortex_rpc::protocol::{method, AgentStatusResult, IdentifyParams, IdentifyResult};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The MCP revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Parser, Debug)]
#[command(name = "cortex-mcp", version, about = "Expose the Cortex brain to an agent over MCP")]
struct Args {
    /// Use an instance rooted at this directory instead of the standard locations.
    #[arg(long, value_name = "DIR")]
    root: Option<std::path::PathBuf>,

    /// The name this server identifies itself by. It appears in every episode and every audit
    /// entry, which is the only reason it exists — nothing is granted by it.
    #[arg(long, default_value = "mcp")]
    actor: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Logs go to stderr. Stdout is the protocol, and a stray line on it desynchronises the
    // client for the rest of the session.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cortex_mcp=info".into()),
        )
        .init();

    let args = Args::parse();
    let paths = match &args.root {
        Some(root) => Paths::rooted_at(root),
        None => Paths::from_env()?,
    };
    let socket = paths.socket();

    let mut client = Client::connect(&socket).await.with_context(|| {
        format!(
            "no daemon listening on {}. Start it with `cortexd`.",
            socket.display()
        )
    })?;

    // The kill switch, before anything is served. A client that connects to a server which
    // starts and then refuses everything has been told the surface is available; it has not.
    let status: AgentStatusResult = client
        .call(method::AGENT_STATUS, json!({}))
        .await
        .context("could not ask the daemon whether the Agent Surface is open")?;
    if !status.enabled {
        anyhow::bail!(
            "the Agent Surface is switched off{}. Turn it on with `cortex agent-surface on`.",
            if status.configured_enabled {
                ""
            } else {
                " in the configuration file, and no RPC can lift that"
            }
        );
    }

    let identified: IdentifyResult = client
        .call(
            method::IDENTIFY,
            IdentifyParams {
                actor: args.actor.clone(),
            },
        )
        .await?;
    tracing::info!(
        actor = %identified.actor,
        tools = tools::TOOLS.len(),
        "serving the Agent Surface over MCP"
    );

    serve(client).await
}

/// The stdio loop. One JSON value per line in, one per line out.
async fn serve(mut client: Client) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(e) => {
                write(&mut stdout, &error_response(Value::Null, -32700, &e.to_string())).await?;
                continue;
            }
        };

        let id = request.get("id").cloned();
        let method_name = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let params = request.get("params").cloned().unwrap_or(json!({}));

        let response = handle(&mut client, &method_name, params).await;

        // A request without an id is a notification; MCP sends several, and answering one
        // desynchronises the client.
        let Some(id) = id else { continue };
        match response {
            Ok(Some(result)) => write(&mut stdout, &ok_response(id, result)).await?,
            Ok(None) => {}
            Err(e) => write(&mut stdout, &error_response(id, -32603, &e.to_string())).await?,
        }
    }
    Ok(())
}

async fn handle(client: &mut Client, method_name: &str, params: Value) -> Result<Option<Value>> {
    match method_name {
        "initialize" => Ok(Some(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "hippocampus-cortex",
                "version": env!("CARGO_PKG_VERSION"),
            },
            // The first thing an agent reads, and the only place to say what to do *before* it
            // is stuck rather than after. It said nothing about the memory the agent now has,
            // which meant discovering `brain_context` was a matter of reading twenty tool
            // descriptions and guessing which one mattered.
            //
            // Deliberately short and in the imperative. An instructions block that explains the
            // architecture is a block that gets skimmed; this one says what to call first, what
            // to call last, and which two things are dangerous.
            "instructions":
                "A memory of this machine, and a memory of your own.\n\n\
                 **Start with `brain_context`.** Give it what you are about to do, in a \
                 sentence, and a token budget. It answers with what is known that bears on it: \
                 preferences that always apply, what the human has open right now, the memories \
                 that match, and the files those are about — already fitted to the budget, each \
                 item saying who wrote it and why it is there. Call it before you start, not \
                 after you are stuck.\n\n\
                 **Finish with `brain_outcome`.** Say which of that actually helped and which \
                 was noise. It is the only signal that teaches this memory what is worth \
                 keeping — a memory that keeps arriving in your context and is never reported \
                 useful decays faster than one nobody ever retrieves. Reporting honestly that \
                 something was noise is worth more than reporting that everything helped.\n\n\
                 **In between: `brain_remember`** anything that cost you effort to work out and \
                 would cost the next session the same — a flag, a convention, an approach that \
                 failed, a decision and its reason. Not what a file already says; that is \
                 indexed, and `brain_search` finds documents that share no words with the \
                 query. `brain_recall` brings back what belongs with a fragment, memories \
                 included.\n\n\
                 Two things to know: writing, moving and trashing files are **gated** — call \
                 once to get a plan, then again with the plan's id, and every one is undoable \
                 with `undo`. And what you remember is visible to the human, who can correct or \
                 delete it.",
        }))),

        // Notifications. Nothing to answer, and nothing to do.
        m if m.starts_with("notifications/") => Ok(None),

        "ping" => Ok(Some(json!({}))),

        "tools/list" => Ok(Some(json!({ "tools": tools::catalogue() }))),

        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|n| n.as_str())
                .context("tools/call without a name")?;
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            let Some(tool) = tools::find(name) else {
                return Ok(Some(tool_error(&format!(
                    "no tool called `{name}`. Call tools/list to see what there is."
                ))));
            };

            // The daemon decides. Its refusal is content, not a protocol error: an agent that
            // is told "outside the allow-list" can ask the user to widen it, whereas a
            // transport error tells it only that something went wrong.
            match client.call::<_, Value>(tool.method, arguments).await {
                Ok(result) => Ok(Some(json!({
                    "content": [{ "type": "text", "text": pretty(&result) }],
                    "structuredContent": result,
                }))),
                Err(e) => Ok(Some(tool_error(&e.to_string()))),
            }
        }

        // Everything else in MCP is optional and this server does not implement it.
        other => Ok(Some(json!({
            "error": format!("unsupported method: {other}")
        }))),
    }
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn ok_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn write(out: &mut tokio::io::Stdout, value: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    out.write_all(&line).await?;
    out.flush().await?;
    Ok(())
}
