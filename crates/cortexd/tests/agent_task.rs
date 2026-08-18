//! M3 §1, mechanically: a whole task through MCP, in the order an agent would do it.
//!
//! *Find a file by description, read it, annotate it, and link it to another.* Every step goes
//! over the MCP protocol on one long-lived connection, because that is how a client works and
//! because a per-call connection would hide any state the session depends on.
//!
//! What this test does not prove is the *semantic* half of "by vague description": it runs
//! without the embedding model, so the search arm that finds documents sharing no words with
//! the query is not exercised here. That arm is measured against a goldset in M2 instead. What
//! is proven here is the shape of the task — search, read, annotate, link, and find the note
//! again afterwards.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn bin(name: &str) -> PathBuf {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    dir.pop();
    let path = dir.join(name);
    assert!(
        path.exists(),
        "{} is missing; build the workspace first: `cargo build --workspace --bins`",
        path.display()
    );
    path
}

/// A daemon with a small corpus in it, and an MCP session attached.
struct Session {
    daemon: Child,
    mcp: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    root: tempfile::TempDir,
    work: PathBuf,
}

impl Session {
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::create_dir_all(&work).unwrap();

        // Three documents. Only one is about the thing the agent will be asked to find.
        std::fs::write(
            work.join("q3-storage-review.md"),
            "Storage expenses grew faster than expected after the archive migration.\n\
             The quarterly review recommends renegotiating the retention policy.\n",
        )
        .unwrap();
        std::fs::write(
            work.join("team-offsite.md"),
            "Notes from the offsite: the venue was fine, the coffee was not.\n",
        )
        .unwrap();
        std::fs::write(
            work.join("retention-policy.md"),
            "Documents are kept for seven years unless a retention hold applies.\n",
        )
        .unwrap();

        let work = work.canonicalize().unwrap();
        std::fs::write(
            root.path().join("config/config.toml"),
            format!(
                "[index]\nmetadata_roots = [\"{work}\"]\ncontent_roots = [\"{work}\"]\n\n\
                 [agent]\nenabled = true\nwrite_allowlist = [\"{work}\"]\n",
                work = work.display()
            ),
        )
        .unwrap();

        let daemon = Command::new(bin("cortexd"))
            .arg("--root")
            .arg(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let socket = root.path().join("run/cortex.sock");
        for _ in 0..100 {
            if socket.exists() && std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Extraction is a background stage; the content arm has nothing to search until it has
        // run. Waiting for it is what an agent's first search would effectively do anyway.
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let mut mcp = Command::new(bin("cortex-mcp"))
            .arg("--root")
            .arg(root.path())
            .arg("--actor")
            .arg("claude")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let stdin = mcp.stdin.take().unwrap();
        let stdout = BufReader::new(mcp.stdout.take().unwrap());
        let mut session = Self {
            daemon,
            mcp,
            stdin,
            stdout,
            next_id: 1,
            root,
            work,
        };

        let init = session.request("initialize", json!({}));
        assert_eq!(init["serverInfo"]["name"], json!("hippocampus-cortex"));
        session.notify("notifications/initialized");
        session
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("the server answered");
            assert!(!line.is_empty(), "the MCP server closed the connection");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value.get("id") == Some(&json!(id)) {
                return value["result"].clone();
            }
        }
    }

    fn notify(&mut self, method: &str) {
        writeln!(self.stdin, r#"{{"jsonrpc":"2.0","method":"{method}"}}"#).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Call a tool and return its structured answer, failing loudly if the tool refused.
    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert!(
            result.get("isError").is_none(),
            "{tool} refused: {}",
            result["content"][0]["text"]
        );
        result["structuredContent"].clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.mcp.kill();
        let _ = self.mcp.wait();
        let _ = Command::new(bin("cortex"))
            .arg("--root")
            .arg(self.root.path())
            .arg("shutdown")
            .output();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

#[test]
fn an_agent_finds_reads_annotates_and_links_a_file_over_mcp() {
    let mut session = Session::start();

    // 1. Find it by what it is about, not by its name. The query says nothing about "q3" or
    //    "storage review"; it describes the content.
    let found = session.call(
        "brain_search",
        json!({ "query": "expenses grew after the archive migration", "limit": 5 }),
    );
    let hits = found["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "the search found nothing at all");
    let path = hits[0]["path"].as_str().unwrap().to_string();
    assert!(
        path.ends_with("q3-storage-review.md"),
        "the top hit was {path}"
    );

    // 2. Read it.
    let content = session.call("fs_read", json!({ "path": path }));
    assert!(
        content["content"]
            .as_str()
            .unwrap()
            .contains("retention policy"),
        "the file read back is not the file that was found"
    );

    // 3. Annotate it — the step that makes the next search better than this one was.
    let annotated = session.call(
        "brain_annotate",
        json!({
            "target": path,
            "note": "the recommendation here was accepted in the following quarter",
            "tags": ["decision", "storage"],
        }),
    );
    let annotation_id = annotated["annotation_id"].as_i64().unwrap();
    assert!(annotation_id > 0);

    // 4. Link it to the document it is about.
    let policy = session.work.join("retention-policy.md").display().to_string();
    let linked = session.call(
        "brain_link",
        json!({ "a": path, "b": policy, "edge": "derived_from" }),
    );
    assert_eq!(linked["edge"], json!("derived_from"));
    assert_eq!(linked["removed"], json!(false));

    // 5. And the point of all of it: the note and the link come back together, from either end.
    let neighbors = session.call("brain_neighbors", json!({ "target": policy, "depth": 1 }));
    let reached: Vec<&str> = neighbors["neighbors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["path"].as_str())
        .collect();
    assert!(
        reached.iter().any(|p| p.ends_with("q3-storage-review.md")),
        "the link is not traversable from the other end: {reached:?}"
    );

    let recalled = session.call("brain_recall", json!({ "cue": "retention policy", "depth": 1 }));
    let notes: Vec<&str> = recalled["associations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["note"].as_str())
        .collect();
    assert!(
        notes.iter().any(|n| n.contains("accepted in the following quarter")),
        "recall did not bring back the note that was written about a neighbour: {notes:?}"
    );
}

#[test]
fn an_agent_that_writes_must_plan_first_and_can_undo_afterwards() {
    let mut session = Session::start();
    let target = session.work.join("summary.md").display().to_string();

    // The first call changes nothing and comes back with a plan.
    let planned = session.call(
        "fs_write",
        json!({ "path": target, "content": "# Summary\n", "mode": "create" }),
    );
    assert_eq!(planned["outcome"], json!("planned"));
    assert!(!std::path::Path::new(&target).exists());
    let plan_id = planned["plan"]["id"].as_str().unwrap().to_string();

    // The second applies it.
    let applied = session.call(
        "fs_write",
        json!({ "path": target, "content": "# Summary\n", "mode": "create", "plan": plan_id }),
    );
    assert_eq!(applied["outcome"], json!("applied"));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "# Summary\n",
        "the plan was approved but nothing was written"
    );

    // And it is reversible from the same surface.
    let undone = session.call("undo", json!({ "last": 1 }));
    assert_eq!(undone["undone"].as_array().unwrap().len(), 1);
    assert!(
        !std::path::Path::new(&target).exists(),
        "undo left the file behind"
    );
}

#[test]
fn the_session_reports_the_tools_it_actually_has() {
    let mut session = Session::start();
    let listed = session.request("tools/list", json!({}));
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();

    // The four the Definition of Done's task needs, by name.
    for required in ["brain_search", "fs_read", "brain_annotate", "brain_link"] {
        assert!(names.contains(&required), "{required} is not offered");
    }
    // And every one of them carries a schema a client can validate against.
    for tool in listed["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], json!("object"), "{tool}");
    }
}
