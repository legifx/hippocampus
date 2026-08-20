//! One slow request must not hold up the ones behind it.
//!
//! This is the shape of a complaint that took three sessions to pin down: "scrolling through
//! the results is laggy". The journal answered it in the end, and the answer was not that
//! anything was slow. It was this, three lines from a real run:
//!
//! ```text
//! 18:39:22.084861  slow call method="search"          ms=152
//! 18:39:22.085244  slow call method="brain.neighbors" ms=152
//! 18:39:22.085253  slow call method="fs.read"         ms=152
//! ```
//!
//! Three different calls, the same duration to the millisecond, all finishing within 0.4 ms of
//! each other. They were in a queue. The connection handler awaited each request before reading
//! the next, and scrolling a list asks for a row's neighbours and its preview on every stop.
//!
//! What is tested here is the property, not the timing: a request sent second comes back before
//! a slow request sent first. Timing assertions on a shared machine are how a suite acquires a
//! test that fails on Tuesdays.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn daemon_binary() -> PathBuf {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    dir.pop();
    let path = dir.join("cortexd");
    assert!(path.exists(), "build the workspace first: `cargo build --workspace --bins`");
    path
}

struct Instance {
    child: Child,
    root: tempfile::TempDir,
}

impl Instance {
    /// A daemon with enough files under it that a listing is real work.
    fn start(files: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        for i in 0..files {
            std::fs::write(work.join(format!("file-{i:05}.txt")), "x").unwrap();
        }

        let work = work.canonicalize().unwrap();
        std::fs::write(
            root.path().join("config/config.toml"),
            format!("[index]\nmetadata_roots = [\"{w}\"]\n\n[agent]\nenabled = true\n", w = work.display()),
        )
        .unwrap();

        let child = Command::new(daemon_binary())
            .arg("--root")
            .arg(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let instance = Self { child, root };
        for _ in 0..100 {
            if UnixStream::connect(instance.socket()).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(200));
                return instance;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the daemon never started listening");
    }

    fn socket(&self) -> PathBuf {
        self.root.path().join("run/cortex.sock")
    }

    fn work(&self) -> PathBuf {
        self.root.path().join("work").canonicalize().unwrap()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(socket: &std::path::Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .unwrap();
        Self {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn send(&mut self, id: i64, method: &str, params: Value) {
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        self.writer.flush().unwrap();
    }

    /// The id of the next answer to arrive, whatever it is an answer to.
    fn next_answer(&mut self) -> i64 {
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).expect("the daemon answered");
            assert!(!line.is_empty(), "the daemon closed the connection");
            let value: Value = serde_json::from_str(&line).unwrap();
            // Notifications carry no id and are not answers.
            if let Some(id) = value.get("id").and_then(Value::as_i64) {
                return id;
            }
        }
    }
}

#[test]
fn a_quick_read_overtakes_a_slow_one_on_the_same_connection() {
    // Four thousand files, listed with no limit: enough work that the answer does not come back
    // before the second request has been written.
    let daemon = Instance::start(4000);
    let mut client = Client::connect(&daemon.socket());

    client.send(
        1,
        "fs.list",
        json!({ "path": daemon.work().display().to_string(), "limit": 4000 }),
    );
    // Sent immediately behind it, and trivial: it names one path and answers from memory.
    client.send(2, "ping", json!({}));

    let first = client.next_answer();
    let second = client.next_answer();

    assert_eq!(
        (first, second),
        (2, 1),
        "the ping came back after the listing, so it was queued behind it"
    );
}

#[test]
fn a_write_still_waits_its_turn() {
    // The other half of the rule. Writes are not on the concurrent list, because their order
    // means something — an undo undoes the previous thing — and a test that only checked reads
    // would let somebody add `fs.move` to that list without noticing what it implies.
    let daemon = Instance::start(200);
    let mut client = Client::connect(&daemon.socket());

    let dir = daemon.work().display().to_string();
    client.send(1, "fs.mkdir", json!({ "path": format!("{dir}/one") }));
    client.send(2, "fs.mkdir", json!({ "path": format!("{dir}/two") }));
    client.send(3, "fs.mkdir", json!({ "path": format!("{dir}/three") }));

    assert_eq!(
        [client.next_answer(), client.next_answer(), client.next_answer()],
        [1, 2, 3],
        "writes are answered in the order they were asked for"
    );
}

#[test]
fn every_answer_arrives_exactly_once() {
    // Concurrency's other failure mode: an answer written twice, or one lost. Twenty requests,
    // reads interleaved with writes so that both paths are in play at the same time.
    //
    // **The answers are deliberately small.** The first version of this test asked for twenty
    // full listings before reading anything, which fills the socket in both directions — the
    // daemon blocks writing an answer nobody is reading, so it stops reading, so the client
    // blocks writing its next request. That is a deadlock in the test rather than in the
    // daemon, and it hung instead of failing. A test that can hang is worse than one that can
    // fail, because it stops the suite instead of reporting.
    let daemon = Instance::start(500);
    let mut client = Client::connect(&daemon.socket());
    let dir = daemon.work().display().to_string();

    for id in 1..=20 {
        if id % 5 == 0 {
            client.send(id, "fs.mkdir", json!({ "path": format!("{dir}/d{id}") }));
        } else {
            client.send(id, "fs.stat", json!({ "path": format!("{dir}/file-00001.txt") }));
        }
    }

    let mut seen = std::collections::BTreeSet::new();
    for _ in 1..=20 {
        let id = client.next_answer();
        assert!(seen.insert(id), "the daemon answered {id} twice");
    }
    assert_eq!(seen.len(), 20, "every request was answered");
    assert_eq!(*seen.first().unwrap(), 1);
    assert_eq!(*seen.last().unwrap(), 20);
}
