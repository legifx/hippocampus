//! The overlay's Rust half, against a real daemon.
//!
//! The frontend is covered by its own suites — layout, keyboard and render latency, in a real
//! browser. What those cannot reach is the socket: they run against a fixture, because a
//! browser has no Unix socket. So this covers the other side, with a real `cortexd` on a real
//! socket, and between them the whole path is tested.
//!
//! It also measures the part of the cold start this process controls. M4 §1 wants the number
//! from the running application, and the application reports it — but a window that never
//! opens reports nothing, so the connection half is measured here where it can be.

use hippocampus_ui_lib::bridge::{Bridge, BridgeError};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

fn daemon_binary() -> PathBuf {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    dir.pop();
    let path = dir.join("cortexd");
    assert!(
        path.exists(),
        "{} is missing; build the workspace first: `cargo build --workspace --bins`",
        path.display()
    );
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
        std::fs::write(
            work.join("architecture-notes.md"),
            "The overlay is a launcher rather than a file manager.\n",
        )
        .unwrap();

        let work = work.canonicalize().unwrap();
        std::fs::write(
            root.path().join("config/config.toml"),
            format!(
                "[index]\nmetadata_roots = [\"{w}\"]\ncontent_roots = [\"{w}\"]\n\n\
                 [agent]\nenabled = true\n",
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
            .expect("cortexd starts");

        let instance = Self { child, root };
        instance.wait_for_socket();
        instance
    }

    fn wait_for_socket(&self) {
        for _ in 0..100 {
            if std::os::unix::net::UnixStream::connect(self.socket()).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(300));
                return;
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

async fn connect(socket: &Path) -> (Bridge, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let bridge = Bridge::connect(socket, move |method, _| {
        recorder.lock().unwrap().push(method);
    })
    .await
    .expect("the bridge connects");
    (bridge, seen)
}

#[tokio::test]
async fn the_overlay_can_search_the_daemon_it_connects_to() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;

    let result = bridge
        .call("search", json!({ "query": "launcher", "limit": 20 }))
        .await
        .expect("the search answers");

    let hits = result["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "the daemon found nothing in its own corpus");

    // The shape the frontend's types expect. A field renamed in the daemon would break the
    // overlay silently otherwise: TypeScript cannot check a JSON boundary.
    let first = &hits[0];
    for field in ["node_id", "path", "kind", "size", "mtime", "tier", "score"] {
        assert!(first.get(field).is_some(), "a hit has no `{field}`");
    }
    for field in [
        "path_rank",
        "lexical_rank",
        "vector_rank",
        "fused",
        "salience_boost",
        "final_score",
    ] {
        assert!(first["score"].get(field).is_some(), "a score has no `{field}`");
    }
    assert!(result.get("mode").is_some());
    assert!(result.get("elapsed_ms").is_some());
    assert!(result.get("partial").is_some());
}

#[tokio::test]
async fn every_command_the_overlay_exposes_reaches_a_method_the_daemon_serves() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;
    let path = instance.work().join("architecture-notes.md").display().to_string();

    // One call per Tauri command, with the arguments that command sends. A method renamed in
    // the protocol shows up here rather than as an empty pane at runtime.
    let calls: Vec<(&str, serde_json::Value)> = vec![
        ("status", json!({})),
        ("extract.status", json!({})),
        ("zones.list", json!({})),
        ("brain.timeline", json!({ "limit": 5 })),
        ("brain.neighbors", json!({ "target": path.clone(), "depth": 1, "edge_types": [] })),
        ("node.get", json!({ "path": path.clone() })),
        ("fs.read", json!({ "path": path.clone(), "byte_range": [0, 1024] })),
        ("search.explain", json!({ "query": "launcher", "path": path.clone() })),
        ("activity.ping", json!({})),
    ];

    for (method, params) in calls {
        let answer = bridge.call(method, params).await;
        assert!(answer.is_ok(), "{method} failed: {:?}", answer.err());
    }
}

#[tokio::test]
async fn the_context_pane_gets_annotations_without_the_frontend_filtering_anything() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;
    let path = instance.work().join("architecture-notes.md").display().to_string();

    bridge
        .call(
            "brain.annotate",
            json!({ "target": path.clone(), "note": "die Entscheidung dazu steht in D4", "tags": ["design"] }),
        )
        .await
        .expect("annotating works");

    // The sidebar asks for every edge type except `annotates`, because the notes come back in
    // their own field. That split has to happen in the daemon — a frontend that filtered would
    // be doing the data processing D4 forbids.
    let context = bridge
        .call(
            "brain.neighbors",
            json!({
                "target": path,
                "depth": 1,
                "edge_types": ["contains", "co_access", "imports", "similar_to", "derived_from"],
            }),
        )
        .await
        .expect("neighbours answer");

    let annotations = context["annotations"].as_array().unwrap();
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0]["tags"][0], "design");

    let neighbours = context["neighbors"].as_array().unwrap();
    assert!(
        neighbours.iter().all(|n| n["kind"] != "annotation"),
        "the daemon returned the annotation in the edge list too, so the pane would show it twice"
    );
}

#[tokio::test]
async fn a_refusal_arrives_with_its_code_rather_than_as_a_failure() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;

    // The deny-list, which applies to everybody including the person at the keyboard (D44).
    // The overlay needs the *code* to say something useful about a refusal rather than "an
    // error occurred".
    let ssh = instance.work().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();

    let err = bridge
        .call(
            "fs.write",
            json!({ "path": ssh.join("id_ed25519").display().to_string(), "content": "x" }),
        )
        .await
        .unwrap_err();

    match err {
        BridgeError::Daemon { code, message } => {
            assert_eq!(code, 1002);
            assert!(message.contains("deny-list"), "{message}");
        }
        other => panic!("expected a policy refusal, got {other}"),
    }
}

#[tokio::test]
async fn the_overlay_is_not_held_to_the_agent_allowlist() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;

    // No allow-list is configured, and this connection has not identified as an agent — so it
    // is the person at the keyboard, using their own file manager (D44). An empty allow-list
    // making the GUI read-only was the bug that found this.
    let target = instance.work().join("written-by-the-human.txt");
    let answer = bridge
        .call(
            "fs.write",
            json!({ "path": target.display().to_string(), "content": "hello" }),
        )
        .await
        .expect("the human path is not gated by the agent allow-list");

    assert_eq!(answer["outcome"], json!("applied"), "{answer}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");

    // And no plan was needed either: a person who clicked the button has confirmed.
    assert!(answer.get("plan").is_none());
}

#[tokio::test]
async fn connecting_costs_a_small_part_of_the_cold_start_budget() {
    let instance = Instance::start();

    // M4 §1 gives the whole cold start 300 ms. The connection is the part this process controls
    // and the part a window that never opens can still be measured on; the rest is WebKit
    // starting and the bundle parsing, which the app reports about itself at runtime.
    let started = std::time::Instant::now();
    let (bridge, _) = connect(&instance.socket()).await;
    let connected = started.elapsed();

    let ping = std::time::Instant::now();
    bridge.call("ping", json!({})).await.expect("ping answers");
    let round_trip = ping.elapsed();

    println!(
        "connect {} µs, round trip {} µs (the whole cold start budget is 300 ms)",
        connected.as_micros(),
        round_trip.as_micros()
    );
    assert!(
        connected < std::time::Duration::from_millis(50),
        "connecting took {connected:?}, which is a sixth of the whole budget"
    );
}

#[tokio::test]
async fn the_bridge_survives_the_daemon_going_away() {
    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;
    bridge.call("ping", json!({})).await.expect("ping answers");

    drop(instance);
    std::thread::sleep(std::time::Duration::from_millis(300));

    // A dead daemon must produce an error the overlay can render, not a hang. The window stays
    // up and says what is wrong; that is the whole difference between a launcher that failed
    // and one that froze.
    let err = bridge.call("ping", json!({})).await.unwrap_err();
    assert!(
        matches!(err, BridgeError::Disconnected | BridgeError::Io(_)),
        "unexpected error after the daemon stopped: {err}"
    );
}

// ---------------------------------------------------------------------------
// M4 §6: killing the thumbnailer mid-preview does not affect search or the daemon.
// ---------------------------------------------------------------------------

fn thumbnailer_binary() -> PathBuf {
    let mut dir = std::env::current_exe().unwrap();
    dir.pop();
    dir.pop();
    let path = dir.join("cortex-preview");
    assert!(
        path.exists(),
        "{} is missing; build the workspace first: `cargo build --workspace --bins`",
        path.display()
    );
    path
}

/// Write a picture big enough that rendering it takes long enough to interrupt.
fn big_picture(path: &Path) {
    let mut png = Vec::new();
    // A PNG written by hand would be a second image encoder; the previewer's own test crate has
    // one, and here a file that fails to decode is enough — the point is that the process is
    // *busy*, and it is busy either way.
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    png.extend_from_slice(&vec![0u8; 4 * 1024 * 1024]);
    std::fs::write(path, png).unwrap();
}

#[tokio::test]
async fn killing_the_thumbnailer_leaves_search_working() {
    use hippocampus_ui_lib::preview::Thumbnailer;

    let instance = Instance::start();
    let (bridge, _) = connect(&instance.socket()).await;
    let thumbnailer = Thumbnailer::at(thumbnailer_binary());

    let picture = instance.work().join("picture.png");
    big_picture(&picture);

    // One request to get the process running, whatever it answers.
    let _ = thumbnailer.preview(&picture, 256, |_| {});
    assert!(thumbnailer.is_running());

    thumbnailer.kill();
    assert!(!thumbnailer.is_running());

    // The claim §6 makes: search is unaffected. It shares nothing with the thumbnailer but the
    // window, and this is what proves it.
    let result = bridge
        .call("search", serde_json::json!({ "query": "launcher", "limit": 5 }))
        .await
        .expect("search still answers after the thumbnailer died");
    assert!(!result["hits"].as_array().unwrap().is_empty());

    // And a later preview starts a new process rather than failing for ever.
    let _ = thumbnailer.preview(&picture, 256, |_| {});
    assert!(
        thumbnailer.is_running(),
        "the supervisor did not restart the thumbnailer after it was killed"
    );

    thumbnailer.stop();
}
