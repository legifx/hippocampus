//! M6 §4, against a running daemon.
//!
//! *"A copy of 10 GB across filesystems shows accurate progress, can be cancelled, and leaves
//! no partial file behind on cancellation."* The size is not the variable — the properties are,
//! and they are the same at 10 MB as at 10 GB, so this uses a size a test suite can afford and
//! asserts each property directly.
//!
//! What this covers that the library tests cannot: that the handle comes back before the copy
//! finishes, that progress arrives as notifications on the connection that asked, and that a
//! cancel sent over the socket reaches a copy already in flight.

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
    fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::create_dir_all(&work).unwrap();

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

/// One connection, kept open, because a transfer's notifications come back on the connection
/// that started it and nowhere else.
struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: i64,
}

impl Client {
    fn connect(socket: &std::path::Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        Self {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
            next_id: 1,
        }
    }

    fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        self.writer.flush().unwrap();
        id
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("the daemon answered");
        assert!(!line.is_empty(), "the daemon closed the connection");
        serde_json::from_str(&line).unwrap()
    }

    /// Send and wait for that call's answer, collecting notifications on the way.
    fn call(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        let id = self.send(method, params);
        let mut notifications = Vec::new();
        loop {
            let value = self.read();
            if value.get("id") == Some(&json!(id)) {
                return (notifications, value);
            }
            notifications.push(value);
        }
    }

    /// Everything until the transfer says it is done.
    fn drain_until_done(&mut self) -> (Vec<Value>, Value) {
        let mut progress = Vec::new();
        loop {
            let value = self.read();
            match value.get("method").and_then(|m| m.as_str()) {
                Some("transfer.done") => return (progress, value["params"].clone()),
                Some("transfer.progress") => progress.push(value["params"].clone()),
                _ => {}
            }
        }
    }
}

/// A file big enough that copying it takes several chunks, so there is a middle.
fn big_file(path: &std::path::Path, bytes: usize) {
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

#[test]
fn a_copy_answers_with_a_handle_and_then_reports_its_way_to_done() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let source = instance.work().join("source");
    let target = instance.work().join("target");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    for i in 0..3 {
        big_file(&source.join(format!("f{i}.bin")), 2_000_000);
    }

    let sources: Vec<String> = (0..3)
        .map(|i| source.join(format!("f{i}.bin")).display().to_string())
        .collect();

    let (_, answer) = client.call(
        "fs.transfer",
        json!({ "sources": sources, "destination": target.display().to_string(), "mode": "copy" }),
    );
    let started = &answer["result"];

    // The total is known before a byte has moved: that is what makes a bar a bar.
    assert_eq!(started["total_files"], json!(3));
    assert_eq!(started["total_bytes"], json!(6_000_000));
    let operation = started["operation"].as_u64().unwrap();

    let (progress, done) = client.drain_until_done();

    assert!(!progress.is_empty(), "a six-megabyte copy reported nothing");
    // Throttled: ten a second, so a copy this size is a handful of messages, not thousands.
    assert!(progress.len() < 200, "{} progress messages", progress.len());

    let fractions: Vec<f64> = progress.iter().map(|p| p["fraction"].as_f64().unwrap()).collect();
    assert!(
        fractions.windows(2).all(|w| w[1] >= w[0]),
        "the fraction went backwards: {fractions:?}"
    );
    assert!(fractions.iter().all(|f| (0.0..=1.0).contains(f)));

    assert_eq!(done["operation"], json!(operation));
    assert_eq!(done["cancelled"], json!(false));
    assert_eq!(done["moved"].as_array().unwrap().len(), 3);
    assert!(done["failed"].as_array().unwrap().is_empty());
    assert_eq!(done["undo_ids"].as_array().unwrap().len(), 3);

    for i in 0..3 {
        assert_eq!(
            std::fs::metadata(target.join(format!("f{i}.bin"))).unwrap().len(),
            2_000_000
        );
    }
}

#[test]
fn a_cancel_sent_over_the_socket_reaches_a_copy_in_flight() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let source = instance.work().join("source");
    let target = instance.work().join("target");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    // Big enough that the copy is still running when a cancel makes the round trip. This is
    // the one place in the suite where the *size* matters: a small copy finishes before the
    // message arrives, and the test would be measuring the disk rather than the cancel.
    const FILES: usize = 5;
    const EACH: usize = 30_000_000;
    for i in 0..FILES {
        big_file(&source.join(format!("f{i}.bin")), EACH);
    }
    let sources: Vec<String> = (0..FILES)
        .map(|i| source.join(format!("f{i}.bin")).display().to_string())
        .collect();

    let (_, answer) = client.call(
        "fs.transfer",
        json!({ "sources": sources, "destination": target.display().to_string(), "mode": "copy" }),
    );
    let operation = answer["result"]["operation"].as_u64().unwrap();

    // Wait for it to be genuinely under way, then stop it. The first progress message always
    // gets through the throttle, so this does not wait for a tenth of a second on purpose.
    let mut finished_early = None;
    loop {
        let value = client.read();
        match value.get("method").and_then(|m| m.as_str()) {
            Some("transfer.progress") => break,
            // Handled rather than ignored: if the copy outran the test, that is a fact to
            // report, not a reason to block until the read times out.
            Some("transfer.done") => {
                finished_early = Some(value["params"].clone());
                break;
            }
            _ => {}
        }
    }
    assert!(
        finished_early.is_none(),
        "the copy finished before a cancel could be sent; the payload is too small for the disk"
    );

    client.send("operation.cancel", json!({ "operation": operation }));
    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(true), "the cancel never arrived: {done}");
    assert!(
        done["moved"].as_array().unwrap().len() < FILES,
        "cancelling copied everything anyway"
    );

    // §4's last clause: nothing half-written, under any name.
    let leftovers: Vec<String> = std::fs::read_dir(&target)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !leftovers.iter().any(|n| n.contains("hippocampus-part")),
        "a partial file was left behind: {leftovers:?}"
    );
    for name in &leftovers {
        assert_eq!(
            std::fs::metadata(target.join(name)).unwrap().len() as usize,
            EACH,
            "{name} is a partial file wearing a finished name"
        );
    }
}

#[test]
fn a_refusal_comes_back_as_a_call_failing_rather_than_an_operation_starting() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    // A destination that is not a directory. The caller must learn this from the call, not by
    // waiting for a notification about an operation that was never going to work.
    let file = instance.work().join("a-file.txt");
    std::fs::write(&file, "x").unwrap();
    let source = instance.work().join("source.txt");
    std::fs::write(&source, "y").unwrap();

    let (_, answer) = client.call(
        "fs.transfer",
        json!({
            "sources": [source.display().to_string()],
            "destination": file.display().to_string(),
            "mode": "move"
        }),
    );

    assert!(answer.get("error").is_some(), "{answer}");
    assert!(answer["error"]["message"].as_str().unwrap().contains("not a directory"));
    assert!(source.exists());
}

#[test]
fn making_a_directory_and_renaming_answer_immediately() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let made = instance.work().join("neu");
    let (_, answer) = client.call("fs.mkdir", json!({ "path": made.display().to_string() }));
    assert!(answer.get("error").is_none(), "{answer}");
    assert!(made.is_dir());

    let (_, answer) = client.call(
        "fs.rename",
        json!({ "path": made.display().to_string(), "name": "umbenannt" }),
    );
    assert!(answer.get("error").is_none(), "{answer}");
    assert!(instance.work().join("umbenannt").is_dir());
    assert!(!made.exists());
}

#[test]
fn deleting_puts_things_where_the_rest_of_the_desktop_looks() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    // The daemon's own trash, pointed at this instance's data directory rather than the real
    // home one, is what `XDG_DATA_HOME` would decide — so this asserts the layout, which is the
    // part that has to match what other tools expect.
    let victim = instance.work().join("notes.md");
    std::fs::write(&victim, "content").unwrap();

    let (_, answer) = client.call("fs.trash", json!({ "path": victim.display().to_string() }));
    assert!(answer.get("error").is_none(), "{answer}");

    let trashed = PathBuf::from(answer["result"]["trashed"].as_str().unwrap());
    assert!(!victim.exists());
    assert!(trashed.exists());
    assert_eq!(trashed.parent().unwrap().file_name().unwrap(), "files");

    let info = trashed
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("info")
        .join(format!("{}.trashinfo", trashed.file_name().unwrap().to_string_lossy()));
    let text = std::fs::read_to_string(&info).expect("a trashed file needs its .trashinfo");
    assert!(text.contains("[Trash Info]"));
    assert!(text.contains(&victim.display().to_string()));
}
