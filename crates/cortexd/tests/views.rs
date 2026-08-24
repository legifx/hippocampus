//! M10 step 1, against a running daemon.
//!
//! *"Every existing behaviour is unchanged, the row shape is identical across both sources, and
//! the frontend contains no branch on 'am I browsing or searching'."* The last of those is a
//! property of the window; the first two are properties of this method, and they are what this
//! file asserts — including the one that is easy to lose, which is that a directory listed
//! through the view layer is the same listing `fs.list` gives.

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
            format!(
                "[index]\nmetadata_roots = [\"{w}\"]\n\n[agent]\nenabled = true\n",
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
                std::thread::sleep(std::time::Duration::from_millis(300));
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        self.writer.flush().unwrap();
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).expect("the daemon answered");
            assert!(!line.is_empty(), "the daemon closed the connection");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
    }
}

fn corpus(at: &std::path::Path) {
    std::fs::create_dir_all(at.join("unterordner")).unwrap();
    std::fs::write(at.join("beta.md"), b"the parser handles a nested list").unwrap();
    std::fs::write(at.join("alpha.md"), b"nothing to see").unwrap();
    std::fs::write(at.join("unterordner/tief.md"), b"deeper").unwrap();
}

#[test]
fn a_directory_opened_as_a_view_is_the_same_listing_fs_list_gives() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    let path = instance.work().display().to_string();

    let listed = client.call("fs.list", json!({ "path": path, "sort": "name" }));
    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": path }, "order": "name" }),
    );

    let entries = listed["result"]["entries"].as_array().unwrap();
    let rows = view["result"]["rows"].as_array().unwrap();
    assert_eq!(entries.len(), rows.len());

    // Same rows, same order, same facts. A second implementation of listing a directory would
    // show up here as a difference in one of them.
    let entry_paths: Vec<&str> = entries.iter().map(|e| e["path"].as_str().unwrap()).collect();
    let row_paths: Vec<&str> = rows.iter().map(|r| r["path"].as_str().unwrap()).collect();
    assert_eq!(entry_paths, row_paths);
    for (entry, row) in entries.iter().zip(rows) {
        assert_eq!(entry["kind"], row["kind"]);
        assert_eq!(entry["size"], row["size"]);
        assert_eq!(entry["mode"], row["mode"]);
        assert_eq!(entry["indexed"], row["indexed"]);
    }

    // And it says what it is.
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Einträge"), "{basis}");
    assert_eq!(view["result"]["order"], json!("name"));
    assert_eq!(view["result"]["origin"]["kind"], json!("dir"));
}

#[test]
fn a_query_and_a_directory_answer_in_one_shape() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    // The index has to have seen the files before a query can find them.
    client.call("reconcile", json!({}));
    let path = instance.work().display().to_string();

    let dir = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": path } }),
    );
    let query = client.call(
        "view.open",
        json!({ "source": { "kind": "query", "text": "beta" } }),
    );

    let dir_row = &dir["result"]["rows"][0];
    let hits = query["result"]["rows"].as_array().unwrap();
    assert!(!hits.is_empty(), "the query found nothing: {query}");
    let hit_row = &hits[0];

    // The fields a renderer needs are on both, under the same names. That is the whole of
    // step 1: there is nothing left for the frontend to branch on.
    for field in ["path", "kind", "size", "mtime", "indexed"] {
        assert!(dir_row.get(field).is_some(), "{field} missing from a dir row");
        assert!(hit_row.get(field).is_some(), "{field} missing from a query row");
    }

    // Since step 2 a directory row the index knows carries the same memory a hit does — that is
    // the point of the step. What is still absent is what only a *stat* can give.
    assert!(hit_row.get("tier").is_some() && hit_row.get("node_id").is_some());
    assert!(
        hit_row.get("mode").is_none(),
        "the index does not keep permissions; a stale mode next to a live one is worse than none"
    );

    // A query view keeps the order it was ranked in, and says so.
    assert_eq!(query["result"]["order"], json!("none"));
    let basis = query["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Treffer"), "{basis}");
    assert!(basis.contains("beta"), "{basis}");
}

#[test]
fn opening_a_directory_that_is_not_there_is_an_error_rather_than_an_empty_view() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    let missing = instance.work().join("gibt-es-nicht").display().to_string();

    let answer = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": missing } }),
    );
    assert!(
        answer.get("error").is_some(),
        "an empty view of a directory that does not exist is a lie: {answer}"
    );
}

#[test]
fn the_deny_list_is_counted_in_the_basis_and_never_named() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    let work = instance.work();
    // A name the deny-list covers, with nothing sensitive in it: what is being tested is that
    // the *name* never reaches the answer.
    std::fs::write(work.join("id_ed25519"), b"not actually a key").unwrap();
    std::fs::write(work.join("ordinary.md"), b"words").unwrap();

    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": work.display().to_string() } }),
    );
    let rows = view["result"]["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .all(|r| !r["path"].as_str().unwrap().ends_with("id_ed25519")),
        "R1: a denied file must not be listed"
    );
    assert_eq!(view["result"]["hidden"], json!(1));
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Sperrliste"), "{basis}");
    assert!(!basis.contains("ed25519"), "R1: counted, never named — {basis}");
}

/// M10 step 2: the memory on every row, from one query per view.
#[test]
fn every_row_carries_what_the_brain_knows_about_it() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));
    let path = instance.work().display().to_string();

    // Something has happened to one of them: opening a file is an episode, and an episode is
    // what makes a row different from its neighbours.
    let touched = instance.work().join("beta.md").display().to_string();
    client.call("brain.pin", json!({ "target": touched }));

    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": path } }),
    );
    let rows = view["result"]["rows"].as_array().unwrap();
    let beta = rows
        .iter()
        .find(|r| r["path"].as_str().unwrap().ends_with("beta.md"))
        .expect("the directory holds beta.md");

    // Every column the paper's step 2 names, on an ordinary directory row.
    assert!(beta["node_id"].is_number(), "{beta}");
    assert!(beta["tier"].is_string(), "{beta}");
    assert!(beta["salience"].is_number(), "{beta}");
    assert!(beta["degree"].is_number(), "{beta}");
    assert_eq!(beta["pinned"], json!(true));
    assert_eq!(beta["indexed"], json!(true));

    // And the basis says how much of the directory the index actually knows, because a
    // salience column over files it has never seen would be a column of zeroes pretending to
    // be a judgement.
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Einträge"), "{basis}");
}

/// The order salience makes possible, and the tie-break that keeps it stable.
#[test]
fn a_directory_can_be_ordered_by_what_the_brain_thinks_matters() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    let work = instance.work();
    for name in ["a.md", "b.md", "c.md"] {
        std::fs::write(work.join(name), b"words").unwrap();
    }
    client.call("reconcile", json!({}));

    // Pinning is the one salience input a test can set directly, and it is the one a person
    // uses: "keep this".
    client.call(
        "brain.pin",
        json!({ "target": work.join("c.md").display().to_string() }),
    );
    // A consolidation pass is what turns inputs into scores.
    client.call("brain.consolidate", json!({}));

    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": work.display().to_string() }, "order": "salience" }),
    );
    let rows = view["result"]["rows"].as_array().unwrap();
    let order: Vec<&str> = rows
        .iter()
        .map(|r| r["path"].as_str().unwrap().rsplit('/').next().unwrap())
        .collect();

    assert_eq!(view["result"]["order"], json!("salience"));
    assert_eq!(
        order.first(),
        Some(&"c.md"),
        "the pinned file is what the brain thinks matters here: {order:?}"
    );
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Salienz"), "{basis}");

    // Asked twice, the same order: a listing that disagreed with itself would be worse than one
    // ordered by nothing.
    let again = client.call(
        "view.open",
        json!({ "source": { "kind": "dir", "path": work.display().to_string() }, "order": "salience" }),
    );
    assert_eq!(again["result"]["rows"], view["result"]["rows"]);
}

/// M10 step 3: the new places. Each is a `Source`, each answers in the same row shape, and each
/// says what it is.
#[test]
fn the_working_set_is_a_place_you_can_open() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));

    // Opening a file is an episode, and the working set is the episode log asked "what lately".
    let opened = instance.work().join("beta.md").display().to_string();
    client.call("brain.annotate", json!({ "target": opened, "note": "gerade angesehen" }));

    let view = client.call("view.open", json!({ "source": { "kind": "working_set" } }));
    let rows = view["result"]["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "something was just touched: {view}");
    // The same row shape as a directory: this is a place, not a report.
    for field in ["path", "kind", "size", "mtime", "indexed", "tier", "salience"] {
        assert!(rows[0].get(field).is_some(), "{field} missing: {}", rows[0]);
    }

    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Episodenprotokoll"), "{basis}");
    assert_eq!(view["result"]["origin"]["kind"], json!("working_set"));
}

#[test]
fn the_cold_tier_is_a_view_that_explains_itself() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));

    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "tier", "tier": "cold" } }),
    );
    assert!(view.get("error").is_none(), "{view}");

    // The sentence is the feature. Somebody looking at a list of files the system has "let go
    // of" needs to be told, in the same breath, that nothing was deleted.
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Gelöscht ist nichts"), "{basis}");
    assert!(basis.contains("Metadaten"), "{basis}");
    assert_eq!(view["result"]["order"], json!("none"));
}

#[test]
fn the_graph_around_a_file_is_a_place_and_a_file_outside_the_index_is_an_error() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));

    let beta = instance.work().join("beta.md").display().to_string();
    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "neighbours", "path": beta } }),
    );
    assert!(view.get("error").is_none(), "{view}");
    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("verbunden"), "{basis}");
    assert!(basis.contains("beta.md"), "{basis}");

    // A path the index has never seen has no neighbours, and saying so beats an empty list that
    // looks like "this file is connected to nothing".
    let unknown = instance.work().join("nie-gesehen.md").display().to_string();
    let refused = client.call(
        "view.open",
        json!({ "source": { "kind": "neighbours", "path": unknown } }),
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not in the index"),
        "{refused}"
    );
}

#[test]
fn a_slice_of_time_lists_each_file_once() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));

    // Three things happening to one file is one row, not three: a timeline of episodes would be
    // a log, and a place is a list of things.
    let beta = instance.work().join("beta.md").display().to_string();
    for note in ["eins", "zwei", "drei"] {
        client.call("brain.annotate", json!({ "target": beta, "note": note }));
    }

    let view = client.call(
        "view.open",
        json!({ "source": { "kind": "timeline", "from": 0, "to": 4_000_000_000i64 } }),
    );
    let rows = view["result"]["rows"].as_array().unwrap();
    let betas = rows
        .iter()
        .filter(|r| r["path"].as_str().unwrap().ends_with("beta.md"))
        .count();
    assert_eq!(betas, 1, "one file, one row: {rows:?}");

    let basis = view["result"]["basis"].as_str().unwrap();
    assert!(basis.contains("Zeitfenster"), "{basis}");
}

/// M10 step 4: why a row is where it is, and what has happened to it — in one answer.
#[test]
fn one_call_says_why_a_row_is_here_and_what_happened_to_it() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    corpus(&instance.work());
    client.call("reconcile", json!({}));

    let beta = instance.work().join("beta.md").display().to_string();
    client.call("brain.annotate", json!({ "target": beta, "note": "gehört zum Parser" }));
    client.call("brain.consolidate", json!({}));

    let why = client.call("node.why", json!({ "path": beta }));
    let result = &why["result"];

    assert_eq!(result["path"], json!(beta));
    assert!(result["salience"].is_number(), "{result}");
    assert!(result["tier"].is_string(), "{result}");
    // The note, from the same answer: the panel used to ask `brain.neighbors` for it, and that
    // question is a *place* now.
    let notes = result["annotations"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0]["note"].as_str().unwrap().contains("Parser"));

    // And the history, newest first, from the episode log rather than from `mtime`: reading a
    // file is an episode too, which is the half `mtime` cannot answer.
    //
    // **An annotation's episode is not in here, and that is right.** It is recorded against the
    // annotation node — the thing that was created — rather than against the file it is about,
    // so a rebuild replays it as what it was. The note itself is above, in `annotations`, with
    // its own timestamp; the panel shows it there rather than twice.
    let episodes = result["episodes"].as_array().unwrap();
    assert!(!episodes.is_empty(), "{result}");
    assert!(
        episodes.iter().all(|e| e["node_id"] == result["node_id"]),
        "every episode here is about this file: {episodes:?}"
    );

    // The components exist once a pass has scored it, and they are the stored ones rather than
    // a number recomputed for display.
    assert!(
        result["components"].is_object(),
        "a consolidation pass ran, so the breakdown should be there: {result}"
    );
    assert!(result["components"]["recency"].is_number());
}

#[test]
fn a_file_the_index_has_never_seen_is_told_so_rather_than_explained() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());
    let unknown = instance.work().join("nie-gesehen.md").display().to_string();

    let answer = client.call("node.why", json!({ "path": unknown }));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nothing to explain"),
        "{answer}"
    );
}
