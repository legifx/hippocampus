//! M7 §5: with the Extension absent, the daemon starts normally and every Core test passes.
//!
//! Federation is switched on here and pointed at a plugin that is not there. The daemon must
//! come up, answer `status`, and say in `sync.status` — in words — why nothing is moving.
//! A daemon that refused to start because its bridge was missing would make the file manager
//! depend on the peer, which is the dependency D10 forbids.

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
    fn start(federation: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("a.txt"), "x").unwrap();
        let work = work.canonicalize().unwrap();
        std::fs::write(
            root.path().join("config/config.toml"),
            format!(
                "[index]\nmetadata_roots = [\"{w}\"]\n\n[agent]\nenabled = true\n\n{federation}\n",
                w = work.display()
            ),
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

    fn call(&self, method: &str, params: Value) -> Value {
        let stream = UnixStream::connect(self.socket()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        writeln!(
            writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        )
        .unwrap();
        writer.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert!(
            response.get("error").is_none(),
            "{method} failed: {}",
            response["error"]
        );
        response["result"].clone()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_daemon_starts_without_its_bridge_and_says_so() {
    let daemon = Instance::start(
        "[federation]\nenabled = true\ntransport = \"plugin\"\nplugin = \"not-installed\"\n",
    );
    let status = daemon.call("status", json!({}));
    assert!(status["methods"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m == "sync.status"));

    let sync = daemon.call("sync.status", json!({}));
    assert_eq!(sync["enabled"], true);
    assert_eq!(sync["transport"], "none");
    let error = sync["last_error"].as_str().unwrap();
    assert!(error.contains("not-installed"), "{error}");
    assert!(sync["basis"].as_str().unwrap().contains("no working transport"));

    let kicked = daemon.call("sync.now", json!({}));
    assert_eq!(kicked["kicked"], false);
}

#[test]
fn off_by_default_means_nothing_is_queued_and_the_status_says_off() {
    let daemon = Instance::start("");
    // Something happened — an episode exists — and still nothing was queued (R5).
    daemon.call("episode.append", json!({ "actor": "human", "action": "open" }));
    let sync = daemon.call("sync.status", json!({}));
    assert_eq!(sync["enabled"], false);
    assert_eq!(sync["pending"], 0);
    assert!(sync["basis"].as_str().unwrap().contains("off"));
}

#[test]
fn a_file_drop_peer_receives_what_happened_and_replay_sends_it_again_with_the_same_ids() {
    let inbox = tempfile::tempdir().unwrap();
    let daemon = Instance::start(&format!(
        "[federation]\nenabled = true\ntransport = \"file-drop\"\ndrop_dir = \"{}\"\nbatch_interval_ms = 100\n",
        inbox.path().display()
    ));
    daemon.call(
        "brain.remember",
        json!({ "scope": "global", "kind": "fact", "text": "the peer hears this" }),
    );
    daemon.call("sync.now", json!({}));
    let names = |dir: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    };
    let has_memory = |dir: &std::path::Path| {
        names(dir).iter().any(|n| {
            std::fs::read_to_string(dir.join(n))
                .map(|t| t.contains("the peer hears this"))
                .unwrap_or(false)
        })
    };
    for _ in 0..100 {
        if has_memory(inbox.path()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(has_memory(inbox.path()), "the memory never reached the drop directory");
    // Let whatever else the daemon has to say about a fresh index arrive too.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let first = names(inbox.path());

    // The peer takes its mail; a replay brings the same documents back, and no others.
    for n in &first {
        std::fs::remove_file(inbox.path().join(n)).unwrap();
    }
    daemon.call("sync.replay", json!({}));
    let mut again = Vec::new();
    for _ in 0..50 {
        again = names(inbox.path());
        if again.len() >= first.len() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(again, first, "a replay must reproduce the same identifiers");

    let sync = daemon.call("sync.status", json!({}));
    assert_eq!(sync["transport"], "file-drop");
    assert!(sync["batches_sent"].as_u64().unwrap() >= 2);
    assert_eq!(sync["pending"], 0);
}
