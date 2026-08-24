//! M3 §5: the three surfaces are proven to share one implementation.
//!
//! The claim is not "they were written from the same spec". It is that MCP, the socket and the
//! CLI reach the *same code*, and the way to prove that is to run one operation through all
//! three against one daemon and compare the answers byte for byte.
//!
//! This is an end-to-end test: it starts a real daemon over a real socket and drives two real
//! binaries. That is the point — a test that used the library directly would prove that the
//! library agrees with itself.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Where cargo put the binaries. `CARGO_BIN_EXE_` only covers this package's own, and this test
/// needs all three, so they are found next to the test executable instead.
fn bin(name: &str) -> PathBuf {
    let mut dir = std::env::current_exe().expect("test executable has a path");
    dir.pop(); // deps/
    dir.pop(); // debug/
    let path = dir.join(name);
    assert!(
        path.exists(),
        "{} is missing. This test drives all three surfaces, so build them first: \
         `cargo build --workspace --bins`",
        path.display()
    );
    path
}

struct Daemon {
    child: Child,
    root: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("notes.txt"), "a file every surface will be asked about\n")
            .unwrap();

        let work = work.canonicalize().unwrap();
        std::fs::write(
            root.path().join("config/config.toml"),
            format!(
                "[index]\nmetadata_roots = [\"{work}\"]\n\n\
                 [agent]\nenabled = true\nrequire_dry_run = true\nwrite_allowlist = [\"{work}\"]\n",
                work = work.display()
            ),
        )
        .unwrap();

        let child = Command::new(bin("cortexd"))
            .arg("--root")
            .arg(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cortexd starts");

        let daemon = Self { child, root };
        daemon.wait_for_socket();
        daemon
    }

    fn wait_for_socket(&self) {
        let socket = self.root.path().join("run/cortex.sock");
        for _ in 0..100 {
            if socket.exists() && std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                // The socket accepts, but the first reconcile may still be running. One `status`
                // round trip is enough to know the daemon is answering.
                std::thread::sleep(std::time::Duration::from_millis(200));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the daemon never started listening");
    }

    fn root(&self) -> &Path {
        self.root.path()
    }

    fn work(&self) -> PathBuf {
        self.root.path().join("work").canonicalize().unwrap()
    }

    /// Surface one: the socket, spoken directly. No client library, so a bug in the client
    /// cannot make two surfaces agree with each other and with nothing else.
    fn rpc(&self, method: &str, params: Value) -> Value {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(self.root.path().join("run/cortex.sock")).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        // Identify first, so this connection is attributed like the other two.
        for (id, method, params) in [
            (1, "identify", json!({ "actor": "surfaces" })),
            (2, method, params),
        ] {
            let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            writeln!(writer, "{request}").unwrap();
        }
        writer.flush().unwrap();

        let mut last = Value::Null;
        for _ in 0..2 {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            last = serde_json::from_str(&line).unwrap();
        }
        last.get("result").cloned().unwrap_or(last)
    }

    /// Surface two: the CLI, in the shape a script or an agent would use it.
    fn cli(&self, args: &[&str]) -> Value {
        let output = Command::new(bin("cortex"))
            .arg("--root")
            .arg(self.root())
            .arg("--json")
            .arg("--as")
            .arg("surfaces")
            .args(args)
            .output()
            .expect("cortex runs");
        assert!(
            output.status.success(),
            "cortex {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("the CLI printed JSON")
    }

    /// One MCP tool call, with the envelope left on — the text block and the structured
    /// content, rather than only the latter.
    fn mcp_raw(&self, tool: &str, arguments: Value) -> Value {
        let mut child = Command::new(bin("cortex-mcp"))
            .arg("--root")
            .arg(self.root())
            .arg("--actor")
            .arg("surfaces")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cortex-mcp starts");
        {
            let stdin = child.stdin.as_mut().unwrap();
            for request in [
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name": tool, "arguments": arguments}}),
            ] {
                writeln!(stdin, "{request}").unwrap();
            }
        }
        let output = child.wait_with_output().unwrap();
        for line in output.stdout.lines() {
            let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if value.get("id") == Some(&json!(2)) {
                return value["result"].clone();
            }
        }
        panic!("cortex-mcp answered no tools/call");
    }

    /// What an MCP client is told when it connects.
    fn mcp_initialize(&self) -> Value {
        let mut child = Command::new(bin("cortex-mcp"))
            .arg("--root")
            .arg(self.root())
            .arg("--actor")
            .arg("surfaces")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cortex-mcp starts");
        {
            let stdin = child.stdin.as_mut().unwrap();
            writeln!(
                stdin,
                "{}",
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
            )
            .unwrap();
        }
        let output = child.wait_with_output().unwrap();
        for line in output.stdout.lines() {
            let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if value.get("id") == Some(&json!(1)) {
                return value["result"].clone();
            }
        }
        panic!("cortex-mcp answered no initialize");
    }

    /// Surface three: MCP over stdio, driven the way a client drives it.
    fn mcp(&self, tool: &str, arguments: Value) -> Value {
        let mut child = Command::new(bin("cortex-mcp"))
            .arg("--root")
            .arg(self.root())
            .arg("--actor")
            .arg("surfaces")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cortex-mcp starts");

        {
            let stdin = child.stdin.as_mut().unwrap();
            for request in [
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name": tool, "arguments": arguments}}),
            ] {
                writeln!(stdin, "{request}").unwrap();
            }
        }

        let output = child.wait_with_output().unwrap();
        let mut result = Value::Null;
        for line in output.stdout.lines() {
            let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if value.get("id") == Some(&json!(2)) {
                result = value["result"].clone();
            }
        }
        assert!(
            result.get("isError").is_none(),
            "the MCP call failed: {result}"
        );
        // `structuredContent` is the daemon's answer verbatim; the text block beside it is the
        // same thing pretty-printed for a model to read.
        result["structuredContent"].clone()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = Command::new(bin("cortex"))
            .arg("--root")
            .arg(self.root.path())
            .arg("shutdown")
            .output();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn one_read_through_three_surfaces_gives_one_answer() {
    let daemon = Daemon::start();
    let path = daemon.work().join("notes.txt");
    let path_str = path.display().to_string();

    let over_socket = daemon.rpc("fs.stat", json!({ "path": path_str }));
    let over_cli = daemon.cli(&["stat", &path_str]);
    let over_mcp = daemon.mcp("fs_stat", json!({ "path": path_str }));

    assert_eq!(over_socket, over_cli, "the CLI answered differently from the socket");
    assert_eq!(over_socket, over_mcp, "MCP answered differently from the socket");
    assert_eq!(over_socket["exists"], json!(true));
    assert_eq!(over_socket["writable"], json!(true), "the allow-list covers this path");
}

#[test]
fn one_listing_through_three_surfaces_gives_one_answer() {
    let daemon = Daemon::start();
    let dir = daemon.work().display().to_string();

    let over_socket = daemon.rpc("fs.list", json!({ "path": dir, "depth": 1 }));
    let over_cli = daemon.cli(&["ls", &dir, "--depth", "1"]);
    let over_mcp = daemon.mcp("fs_list", json!({ "path": dir, "depth": 1 }));

    assert_eq!(over_socket, over_cli);
    assert_eq!(over_socket, over_mcp);
    assert_eq!(over_socket["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn the_places_are_the_same_through_all_three() {
    // The screen a person opens onto is built by the daemon (D52) precisely so that there is
    // one answer to "which of these is personal and which is plumbing". Three answers would be
    // three opinions, and the CLI's would be the one nobody noticed had drifted.
    let daemon = Daemon::start();

    let over_socket = daemon.rpc("fs.places", json!({}));
    let over_cli = daemon.cli(&["places"]);
    let over_mcp = daemon.mcp("fs_places", json!({}));

    // How full a disk is cannot be compared across three calls, and comparing it made this test
    // fail whenever something wrote to the disk between them — which, on a machine that is
    // compiling, is often. It is *measured* on each call rather than reported from a cache, so
    // three identical numbers were never the property being asserted. Each surface must report
    // a capacity, and the rest must match exactly.
    let capacities = |value: &Value| -> Vec<bool> {
        let mut out = Vec::new();
        for section in value["sections"].as_array().unwrap() {
            for place in section["places"].as_array().unwrap() {
                out.push(place.get("capacity").is_some());
            }
        }
        out
    };
    let strip = |mut value: Value| -> Value {
        for section in value["sections"].as_array_mut().unwrap() {
            for place in section["places"].as_array_mut().unwrap() {
                if let Some(object) = place.as_object_mut() {
                    object.remove("capacity");
                }
            }
        }
        value
    };

    assert_eq!(capacities(&over_socket), capacities(&over_cli));
    assert_eq!(capacities(&over_socket), capacities(&over_mcp));
    let over_socket = strip(over_socket);
    assert_eq!(over_socket, strip(over_cli));
    assert_eq!(over_socket, strip(over_mcp));

    // And whatever else is on this machine, the last section is the one row that holds
    // everything technical.
    let sections = over_socket["sections"].as_array().unwrap();
    let last = sections.last().unwrap();
    assert_eq!(last["title"], json!("System"));
    assert_eq!(last["places"].as_array().unwrap().len(), 1);
    assert_eq!(last["places"][0]["path"], json!("/"));
}

#[test]
fn what_opens_a_file_is_the_same_answer_over_the_socket_and_the_cli() {
    // Two surfaces rather than three, on purpose. `apps.for_path` is not on the MCP bridge: an
    // agent has no session to open a window in, and the tool it would reach for — "what would a
    // person open this with" — is one it has no use for and one whose companion,
    // `apps.set_default`, it is refused outright (see the red-team suite). Reaching the same
    // code from the window and from the CLI is what matters here, because that is the pair a
    // person compares when the window shows something surprising.
    let daemon = Daemon::start();
    let file = daemon.work().join("notes.txt").display().to_string();

    let over_socket = daemon.rpc("apps.for_path", json!({ "path": file }));
    let over_cli = daemon.cli(&["apps", &file]);

    assert_eq!(over_socket, over_cli);
    // Whatever is installed on the machine running this, the file is text and the answer says
    // so — the type is read from the desktop's database, not from the index.
    assert_eq!(over_socket["mime"], json!("text/plain"));
    assert!(over_socket["applications"].is_array());
}

#[test]
fn a_gated_tool_plans_identically_on_all_three() {
    let daemon = Daemon::start();
    let target = daemon.work().join("new.txt").display().to_string();
    let args = json!({ "path": target, "content": "hello", "mode": "create" });

    let strip = |mut plan: Value| -> Value {
        // The id is a counter and is expected to differ; everything that describes the *effect*
        // must not.
        plan["plan"]["id"] = json!("<id>");
        plan
    };

    let over_socket = strip(daemon.rpc("fs.write", args.clone()));
    let over_cli = strip(daemon.cli(&["write", &target, "hello"]));
    let over_mcp = strip(daemon.mcp("fs_write", args));

    assert_eq!(over_socket, over_cli);
    assert_eq!(over_socket, over_mcp);
    assert_eq!(over_socket["outcome"], json!("planned"));
    assert_eq!(over_socket["plan"]["summary"].as_array().unwrap().len(), 1);
    assert!(
        !Path::new(&target).exists(),
        "three dry runs created the file between them"
    );
}

#[test]
fn a_refusal_is_the_same_refusal_on_all_three() {
    let daemon = Daemon::start();
    // Outside the allow-list: the daemon's own state directory.
    let forbidden = daemon.root().join("state/sneaky.txt").display().to_string();

    let over_socket = daemon.rpc("fs.write", json!({ "path": forbidden, "content": "x" }));
    assert_eq!(over_socket["error"]["code"], json!(1002));
    assert_eq!(over_socket["error"]["data"]["refusal"], json!("outside_allowlist"));

    // The CLI reports it as a failure with the same message on stderr.
    let output = Command::new(bin("cortex"))
        .args(["--root"])
        .arg(daemon.root())
        .args(["--as", "surfaces", "write", &forbidden, "x", "--yes"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("write allow-list"), "CLI said: {stderr}");
    assert!(stderr.contains("1002"), "the CLI dropped the refusal code");

    // MCP returns it as tool content rather than a protocol error, so the agent can act on it.
    let mut child = Command::new(bin("cortex-mcp"))
        .arg("--root")
        .arg(daemon.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#).unwrap();
        let call = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"fs_write","arguments":{"path": forbidden, "content":"x"}}});
        writeln!(stdin, "{call}").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let mut found = false;
    for line in output.stdout.lines() {
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if value.get("id") == Some(&json!(2)) {
            assert_eq!(value["result"]["isError"], json!(true));
            let text = value["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("write allow-list"), "MCP said: {text}");
            found = true;
        }
    }
    assert!(found, "MCP never answered the call");
}

#[test]
fn every_surface_leaves_the_same_kind_of_audit_entry() {
    let daemon = Daemon::start();
    let path = daemon.work().join("notes.txt").display().to_string();

    daemon.rpc("fs.stat", json!({ "path": path }));
    daemon.cli(&["stat", &path]);
    daemon.mcp("fs_stat", json!({ "path": path }));

    let audit = daemon.rpc("audit.recent", json!({ "limit": 200 }));
    let entries: Vec<&Value> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["tool"] == json!("fs.stat") && e["actor"] == json!("agent:surfaces"))
        .collect();

    assert!(
        entries.len() >= 3,
        "expected an audit entry per surface, found {}",
        entries.len()
    );
    for entry in entries {
        assert_eq!(entry["allowed"], json!(true));
        assert_eq!(entry["args"]["path"], json!(path));
    }
}

#[test]
fn the_kill_switch_stops_mcp_from_starting_at_all() {
    let daemon = Daemon::start();
    daemon.rpc("agent.set_enabled", json!({ "enabled": false }));

    let output = Command::new(bin("cortex-mcp"))
        .arg("--root")
        .arg(daemon.root())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "the MCP server served a disabled surface"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("switched off"),
        "the refusal did not say why: {stderr}"
    );

    // And the tools refuse over the socket too, rather than only at the bridge.
    let refused = daemon.rpc("fs.stat", json!({ "path": "/tmp" }));
    assert_eq!(refused["error"]["code"], json!(1001));

    daemon.rpc("agent.set_enabled", json!({ "enabled": true }));
}

/// M11 §1: a memory written in one session comes back in the next, identically on all three.
///
/// The write goes in over the socket, and the read comes out of all three — which is the
/// interesting direction. A memory that only the surface that wrote it can see would pass a
/// same-surface round trip and fail the moment an agent connected over MCP rather than over the
/// CLI, which is exactly the arrangement this project expects.
#[test]
fn a_memory_written_once_is_returned_by_every_surface() {
    let daemon = Daemon::start();

    let written = daemon.rpc(
        "brain.remember",
        json!({
            "text": "the reference corpus is generated from a seed and is never checked in",
            "kind": "fact",
            "scope": "global",
        }),
    );
    let memory_id = written["memory_id"].as_i64().expect("a memory id");
    assert_eq!(written["scope"], json!("global"));

    let task = "where does the reference corpus come from";
    let over_socket = daemon.rpc("brain.context", json!({ "task": task, "budget_tokens": 512 }));
    let over_cli = daemon.cli(&["context", task, "--budget", "512"]);
    let over_mcp = daemon.mcp("brain_context", json!({ "task": task, "budget_tokens": 512 }));

    // Two fields legitimately differ between three calls made at three moments: the pack id,
    // which is derived from the second it was built in, and `retrieved`, which counts how often
    // a memory has been put into a pack and is therefore one higher on each successive call.
    // That second one is not noise — it is D76's bookkeeping working — so it is stripped from
    // the comparison and asserted separately.
    let items = |pack: &Value| -> Value {
        let mut items = pack["items"].clone();
        for item in items.as_array_mut().unwrap() {
            item["basis"]["retrieved"] = json!(null);
        }
        items
    };
    assert_eq!(items(&over_socket), items(&over_cli), "the CLI differed");
    assert_eq!(items(&over_socket), items(&over_mcp), "MCP differed");

    // Absent means zero: the basis omits the counts nobody has moved yet, because "helped 0,
    // noise 0, retrieved 0" on every item of every pack on a fresh install is the absence of an
    // argument rather than one. The first call therefore has no `retrieved` field at all.
    let retrieved = |pack: &Value| -> u64 {
        let item = pack["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["node_id"].as_i64() == Some(memory_id))
            .expect("the memory is in the pack");
        item["basis"]["retrieved"].as_u64().unwrap_or(0)
    };
    assert!(
        retrieved(&over_mcp) > retrieved(&over_socket),
        "putting a memory into a pack did not count as a retrieval"
    );

    let ids: Vec<i64> = over_socket["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["node_id"].as_i64())
        .collect();
    assert!(
        ids.contains(&memory_id),
        "the memory written a moment ago is not in the pack: {over_socket}"
    );

    // And the basis is there, on every surface, because a memory an agent cannot attribute is
    // one it cannot argue with.
    let item = over_mcp["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["node_id"].as_i64() == Some(memory_id))
        .expect("the memory is in the MCP pack too");
    assert_eq!(item["basis"]["actor"], json!("agent:surfaces"));
    assert!(item["basis"]["lexical_rank"].as_u64().is_some());
}

/// A session counts what happened during it — which it did not, for every session ever opened.
///
/// `note_session_call` was written, tested and called by nothing, so `agent_sessions.calls`
/// stayed at zero and the Inspector printed "0 Aufrufe" beside every session in the list. The
/// counting is in the dispatcher rather than in each handler, because a refusal never reaches a
/// handler and a refusal is the interesting half of the number.
#[test]
fn a_session_counts_the_calls_and_the_refusals_that_belong_to_it() {
    use std::os::unix::net::UnixStream;

    let daemon = Daemon::start();
    let stream = UnixStream::connect(daemon.root().join("run/cortex.sock")).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let mut send = |id: i64, method: &str, params: Value| -> Value {
        writeln!(
            writer,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        writer.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    send(1, "identify", json!({ "actor": "counter" }));
    let opened = send(2, "agent.session.begin", json!({ "task": "counting" }));
    let session = opened["result"]["session_id"].as_str().unwrap().to_string();

    // Three that work and one that cannot: writing outside an empty allow-list.
    send(3, "brain.remember", json!({ "text": "one", "scope": "global" }));
    send(4, "brain.remember", json!({ "text": "two", "scope": "global" }));
    send(5, "brain.memories", json!({}));
    let refused = send(6, "fs.write", json!({ "path": "/etc/nope", "content": "x" }));
    assert!(refused.get("error").is_some(), "the fixture's refusal was allowed");

    let listed = send(7, "agent.sessions", json!({ "limit": 10 }));
    let sessions = listed["result"]["sessions"].as_array().unwrap();
    let mine = sessions
        .iter()
        .find(|s| s["id"].as_str() == Some(session.as_str()))
        .expect("the session is listed");

    // Six calls precede the listing: identify, begin, and the four above. The listing itself
    // has not been counted yet when the answer is built.
    assert!(
        mine["calls"].as_i64().unwrap_or(0) >= 4,
        "a session that ran six calls reported {}: {mine}",
        mine["calls"]
    );
    assert_eq!(
        mine["refusals"].as_i64(),
        Some(1),
        "the refusal was not counted, and a refusal is the interesting half"
    );
    assert_eq!(mine["memories"].as_i64(), Some(0), "memories are scoped to the session that writes them, and these were global-scoped without a session_id");
}

/// A pack costs its budget once, not four times.
///
/// An MCP result carries the answer twice — pretty-printed as a text block, and verbatim as
/// structured content — and the pack used to carry its own text twice as well, in `rendered`
/// and in an item beside it. Four copies of every memory, for a feature whose entire purpose is
/// a token budget.
///
/// The numbers are asserted as ratios rather than as byte counts, because the fixture's content
/// will change and the property will not.
#[test]
fn a_context_pack_does_not_send_its_text_four_times() {
    let daemon = Daemon::start();
    for text in [
        "the hygiene gate runs before every commit and must not be skipped",
        "the reference corpus is generated from a seed and never checked in",
        "the thumbnailer runs in its own process so a bad image cannot take the daemon down",
    ] {
        daemon.rpc("brain.remember", json!({ "text": text, "kind": "fact", "scope": "global" }));
    }

    let raw = daemon.mcp_raw("brain_context", json!({ "task": "committing a change", "budget_tokens": 2048 }));
    let rendered = raw["structuredContent"]["rendered"].as_str().unwrap_or("");
    assert!(rendered.contains("hygiene gate"), "the fixture found nothing: {raw}");

    // The text block is the rendered pack itself, not a pretty-printed copy of the whole answer.
    let block = raw["content"][0]["text"].as_str().unwrap_or("");
    assert_eq!(block, rendered, "the text block is not the pack the daemon composed");

    // And no item repeats what `rendered` already says.
    let items = raw["structuredContent"]["items"].as_array().unwrap();
    assert!(!items.is_empty());
    for item in items {
        assert!(
            item.get("text").is_none(),
            "an item carries the text that `rendered` already carries: {item}"
        );
        // What it does carry is what a caller needs to report on it and to argue with it.
        assert!(item.get("node_id").is_some());
        assert!(item.get("basis").is_some());
    }

    // The exact property, rather than a ratio that depends on how long the fixture's sentences
    // happen to be: one memory's text appears **twice** in the whole answer — once in the block
    // a model reads, once in the structured copy a client can act on — and not four times.
    let whole = serde_json::to_string(&raw).unwrap();
    let occurrences = whole.matches("hygiene gate").count();
    assert_eq!(
        occurrences, 2,
        "a memory's text appears {occurrences} times in one answer; it should appear in the \
         text block and in the structured copy, and nowhere else"
    );
}

/// The first thing an agent reads names the tool it should call first.
///
/// Not a style check. The instructions block is the only place to tell an agent what to do
/// *before* it is stuck, and it said nothing about the memory the agent now has — which made
/// discovering the one tool that matters a matter of reading twenty descriptions and guessing.
#[test]
fn the_mcp_instructions_say_what_to_call_first_and_last() {
    let daemon = Daemon::start();
    let hello = daemon.mcp_initialize();
    let instructions = hello["instructions"].as_str().unwrap_or("");

    for expected in ["brain_context", "brain_outcome", "brain_remember"] {
        assert!(
            instructions.contains(expected),
            "an agent reading this would not know about {expected}: {instructions}"
        );
    }
    // And the property that is easy to leave out and is the whole of the learning signal.
    assert!(
        instructions.contains("noise"),
        "the instructions ask for outcomes without saying that a negative one is worth having"
    );
}

/// Pattern completion reaches what an agent learned, not only the files it learned about.
///
/// `brain.recall` walked outwards from what the *file* search found, so a memory was reachable
/// only when it happened to hang off a file the cue also matched — reliable for the easy half
/// of the question and silent for the other. The cue goes to the memory layer too now.
#[test]
fn recall_brings_back_a_memory_the_cue_names() {
    let daemon = Daemon::start();

    daemon.rpc(
        "brain.remember",
        json!({
            "text": "the thumbnailer runs in its own process so a malformed image cannot take \
                     the daemon with it",
            "kind": "decision",
            "scope": "global",
        }),
    );

    // A cue that names the memory and matches no file in this fixture at all.
    let recalled = daemon.rpc("brain.recall", json!({ "cue": "why the thumbnailer is separate" }));
    let memories = recalled["memories"].as_array().cloned().unwrap_or_default();
    assert!(
        memories
            .iter()
            .any(|m| m["text"].as_str().unwrap_or("").contains("malformed image")),
        "the cue named a memory and recall did not bring it back: {recalled}"
    );
    // And it arrives whole, with who wrote it — a memory a caller cannot attribute is one it
    // cannot argue with.
    assert_eq!(memories[0]["actor"], json!("agent:surfaces"));
}

/// M11 §7: scope isolation, asserted at the surface rather than only in the store.
#[test]
fn one_agents_memory_does_not_reach_another_agent() {
    let daemon = Daemon::start();

    // Two agents, distinguished only by the name they identify with.
    let as_agent = |name: &str, method: &str, params: Value| -> Value {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(daemon.root().join("run/cortex.sock")).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        for (id, method, params) in [
            (1, "identify", json!({ "actor": name })),
            (2, method, params),
        ] {
            writeln!(
                writer,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
            )
            .unwrap();
        }
        writer.flush().unwrap();
        let mut last = Value::Null;
        for _ in 0..2 {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            last = serde_json::from_str(&line).unwrap();
        }
        last.get("result").cloned().unwrap_or(last)
    };

    let mine = as_agent(
        "alpha",
        "brain.remember",
        json!({ "text": "the staging deployment target is the blue cluster", "kind": "fact" }),
    );
    let mine_id = mine["memory_id"].as_i64().unwrap();
    assert_eq!(mine["scope"], json!("agent:alpha"), "the default scope is not the writer's own");

    let task = "the staging deployment target";
    let alpha_sees = as_agent("alpha", "brain.context", json!({ "task": task }));
    let beta_sees = as_agent("beta", "brain.context", json!({ "task": task }));

    let ids = |pack: &Value| -> Vec<i64> {
        pack["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|item| item["node_id"].as_i64())
            .collect()
    };
    assert!(ids(&alpha_sees).contains(&mine_id));
    assert!(
        !ids(&beta_sees).contains(&mine_id),
        "another agent's private memory reached the pack: {beta_sees}"
    );

    // And beta cannot enumerate it either.
    let listed = as_agent("beta", "brain.memories", json!({}));
    let texts = serde_json::to_string(&listed).unwrap();
    assert!(
        !texts.contains("blue cluster"),
        "another agent's memory was listed: {texts}"
    );

    // Asking for every scope is refused for an agent outright.
    let refused = as_agent("beta", "brain.memories", json!({ "everything": true }));
    assert!(
        refused.get("error").is_some() || refused["code"].is_i64(),
        "an agent was allowed to enumerate every scope: {refused}"
    );
}

/// M11 §9: with the surface off, nothing is written and nothing is read.
#[test]
fn the_kill_switch_reaches_the_memory_tools() {
    let daemon = Daemon::start();

    daemon.rpc(
        "brain.remember",
        json!({ "text": "written while the surface was on", "scope": "global" }),
    );
    let off = daemon.rpc("agent.set_enabled", json!({ "enabled": false }));
    assert_eq!(off["enabled"], json!(false));

    for (method, params) in [
        ("brain.remember", json!({ "text": "written while off", "scope": "global" })),
        ("brain.context", json!({ "task": "anything" })),
        ("brain.memories", json!({})),
        ("brain.outcome", json!({ "pack_id": "p", "helped": [] })),
    ] {
        let answer = daemon.rpc(method, params);
        assert!(
            answer.get("error").is_some() || answer["code"].is_i64(),
            "{method} answered while the Agent Surface was switched off: {answer}"
        );
    }

    // Three things still answer with the surface off, and they are the three you look at when
    // you have just switched it off: what the state is, what happened, and who was working.
    // `audit.recent` and `agent.status` were already on that side of the line; `agent.sessions`
    // was gated, which meant the window lost its session list at exactly the wrong moment.
    for (method, params) in [
        ("agent.status", json!({})),
        ("audit.recent", json!({ "limit": 5 })),
        ("agent.sessions", json!({ "limit": 5 })),
    ] {
        let answer = daemon.rpc(method, params);
        assert!(
            answer.get("error").is_none() && !answer["code"].is_i64(),
            "{method} refused while the surface was off, and it is an observation rather than \
             an action: {answer}"
        );
    }

    // Back on, and the memory written before is still the only one.
    daemon.rpc("agent.set_enabled", json!({ "enabled": true }));
    let listed = daemon.rpc("brain.memories", json!({}));
    let texts = serde_json::to_string(&listed).unwrap();
    assert!(texts.contains("while the surface was on"));
    assert!(
        !texts.contains("written while off"),
        "a memory was written through a switched-off surface: {texts}"
    );
}
