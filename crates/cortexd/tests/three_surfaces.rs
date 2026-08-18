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
