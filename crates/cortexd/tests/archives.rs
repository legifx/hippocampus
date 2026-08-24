//! M6's archive item, against a running daemon.
//!
//! *"Archives: at minimum extract zip and tar."* The mandatory list says extract, so these are
//! about unpacking — and about the four things that make unpacking different from copying: an
//! entry's path comes from a stranger, a header's size is a claim, a cancellation must leave
//! nothing behind, and an undo has to take back what was written without taking anything else.
//!
//! What this covers that the library tests cannot: that the handle comes back before the
//! unpacking finishes, that it reports on the same two notifications a transfer does, and that
//! an entry the daemon refuses is *named* in the answer rather than quietly dropped.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
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

/// One connection, kept open: the notifications come back on the connection that started the
/// operation and nowhere else.
struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: i64,
}

impl Client {
    fn connect(socket: &Path) -> Self {
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        loop {
            let value = self.read();
            if value.get("id") == Some(&json!(id)) {
                return value;
            }
        }
    }

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

// ---------------------------------------------------------------------------
// The archives these tests unpack, built here so that what is inside them is visible.
// ---------------------------------------------------------------------------

/// A zip of ordinary files, one of them in a subdirectory.
fn ordinary_zip(at: &Path, payload: usize) {
    let file = std::fs::File::create(at).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions = Default::default();
    for name in ["one.txt", "two.txt", "nested/three.txt"] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&vec![b'x'; payload]).unwrap();
    }
    zip.finish().unwrap();
}

/// A zip with an entry that tries to write above the destination. Written through
/// `start_file_from_path`'s raw sibling so the name survives into the file — the point of the
/// test is that the daemon refuses it, not that a writer refuses to produce it.
fn escaping_zip(at: &Path) {
    let file = std::fs::File::create(at).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions = Default::default();
    zip.start_file("innocent.txt", options).unwrap();
    zip.write_all(b"fine").unwrap();
    zip.add_directory("../", options).ok();
    zip.start_file("../escaped.txt", options).unwrap();
    zip.write_all(b"should never be written").unwrap();
    zip.finish().unwrap();
}

fn tar_with<F: FnOnce(&mut tar::Builder<std::fs::File>)>(at: &Path, build: F) {
    let file = std::fs::File::create(at).unwrap();
    let mut builder = tar::Builder::new(file);
    build(&mut builder);
    builder.finish().unwrap();
}

fn tar_entry(builder: &mut tar::Builder<std::fs::File>, name: &str, data: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, name, data).unwrap();
}

/// A gzipped tar, which is the form nearly every tarball actually arrives in.
fn ordinary_tar_gz(at: &Path) {
    let mut plain = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut plain);
        for name in ["a.txt", "sub/b.txt"] {
            let data = b"hello from a tarball";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, &data[..]).unwrap();
        }
        builder.finish().unwrap();
    }
    let file = std::fs::File::create(at).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    encoder.write_all(&plain).unwrap();
    encoder.finish().unwrap();
}

// ---------------------------------------------------------------------------

#[test]
fn a_dry_run_says_what_is_inside_and_writes_nothing() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("notes.zip");
    ordinary_zip(&archive, 1_000);

    let answer = client.call(
        "fs.extract",
        json!({ "archive": archive.display().to_string(), "dry_run": true }),
    );
    let planned = &answer["result"];

    assert_eq!(planned["outcome"], json!("planned"));
    assert_eq!(planned["format"], json!("zip"));
    assert_eq!(planned["total_files"], json!(3));
    assert_eq!(planned["total_bytes"], json!(3_000));
    assert_eq!(planned["occupied"], json!(false));
    // Where it would go: a new directory beside the archive, named after it. A person has to be
    // able to read that before they press the button.
    assert_eq!(
        planned["destination"],
        json!(instance.work().join("notes").display().to_string())
    );
    let entries = planned["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|e| e["path"] == json!("nested/three.txt")));

    assert!(
        !instance.work().join("notes").exists(),
        "a dry run created the directory it was only asked about"
    );
}

#[test]
fn a_zip_lands_beside_its_archive_and_reports_its_way_there() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("notes.zip");
    // Big enough that the copy takes several chunks, so there is progress to report.
    ordinary_zip(&archive, 700_000);

    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    let started = &answer["result"];
    assert_eq!(started["outcome"], json!("started"));
    assert_eq!(started["total_files"], json!(3));
    let operation = started["operation"].as_u64().unwrap();

    let (progress, done) = client.drain_until_done();

    assert!(!progress.is_empty(), "a two-megabyte extraction reported nothing");
    let fractions: Vec<f64> = progress.iter().map(|p| p["fraction"].as_f64().unwrap()).collect();
    assert!(
        fractions.windows(2).all(|w| w[1] >= w[0]),
        "the fraction went backwards: {fractions:?}"
    );

    assert_eq!(done["operation"], json!(operation));
    assert_eq!(done["cancelled"], json!(false));
    assert!(done["failed"].as_array().unwrap().is_empty());
    assert_eq!(done["undo_ids"].as_array().unwrap().len(), 1);

    let out = instance.work().join("notes");
    assert_eq!(std::fs::read(out.join("one.txt")).unwrap().len(), 700_000);
    assert_eq!(std::fs::read(out.join("nested/three.txt")).unwrap().len(), 700_000);
}

#[test]
fn a_tarball_is_unpacked_and_gzip_is_recognised_by_its_bytes() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    // Named `.bin`, so nothing but the magic bytes can identify it.
    let archive = instance.work().join("bundle.bin");
    ordinary_tar_gz(&archive);

    let answer = client.call(
        "fs.extract",
        json!({ "archive": archive.display().to_string(), "dry_run": true }),
    );
    assert_eq!(answer["result"]["format"], json!("tar.gz"), "{answer}");
    assert_eq!(answer["result"]["total_files"], json!(2));

    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    assert_eq!(answer["result"]["outcome"], json!("started"));
    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(false));

    let out = instance.work().join("bundle");
    assert_eq!(
        std::fs::read_to_string(out.join("sub/b.txt")).unwrap(),
        "hello from a tarball"
    );
}

#[test]
fn an_entry_that_would_escape_is_refused_and_named_while_the_rest_arrives() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("hostile.zip");
    escaping_zip(&archive);

    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    assert_eq!(answer["result"]["outcome"], json!("started"));
    let (_, done) = client.drain_until_done();

    let refused = done["failed"].as_array().unwrap();
    assert!(
        refused.iter().any(|f| f["reason"]
            .as_str()
            .unwrap()
            .contains("outside the destination")),
        "the escaping entry was not reported: {refused:?}"
    );

    assert!(
        !instance.work().join("escaped.txt").exists(),
        "an entry wrote above its destination"
    );
    // And the honest half: the rest of the archive still arrived.
    assert_eq!(
        std::fs::read_to_string(instance.work().join("hostile/innocent.txt")).unwrap(),
        "fine"
    );
}

#[test]
fn a_link_inside_an_archive_is_refused_rather_than_followed() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("linky.tar");
    tar_with(&archive, |builder| {
        tar_entry(builder, "ordinary.txt", b"here");
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_cksum();
        builder.append_data(&mut header, "sneaky", &b""[..]).unwrap();
    });

    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    assert_eq!(answer["result"]["outcome"], json!("started"));
    let (_, done) = client.drain_until_done();

    let refused = done["failed"].as_array().unwrap();
    assert!(
        refused
            .iter()
            .any(|f| f["reason"].as_str().unwrap().contains("symbolic link")),
        "the link was not reported: {refused:?}"
    );
    let out = instance.work().join("linky");
    assert!(!out.join("sneaky").exists(), "a link from an archive was created");
    assert_eq!(std::fs::read_to_string(out.join("ordinary.txt")).unwrap(), "here");
}

#[test]
fn a_cancelled_extraction_leaves_nothing_behind() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("large.zip");
    // Large enough that a cancel sent immediately after the handle arrives lands mid-flight.
    ordinary_zip(&archive, 40_000_000);

    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    let operation = answer["result"]["operation"].as_u64().unwrap();

    let cancelled = client.call("operation.cancel", json!({ "operation": operation }));
    assert_eq!(cancelled["result"]["cancelled"], json!(true));

    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(true));
    assert!(done["moved"].as_array().unwrap().is_empty());
    assert!(
        done["undo_ids"].as_array().unwrap().is_empty(),
        "a cancelled extraction offered an undo for files it did not keep"
    );

    let out = instance.work().join("large");
    assert!(
        !out.exists(),
        "a cancelled extraction left {} behind",
        out.display()
    );
}

#[test]
fn undo_takes_back_what_it_wrote_and_nothing_else() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("notes.zip");
    ordinary_zip(&archive, 100);

    client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(false));

    let out = instance.work().join("notes");
    // Something of the person's own, put there after the extraction. Undo must not take this.
    std::fs::write(out.join("mine.txt"), "written by a person").unwrap();

    let answer = client.call("undo", json!({}));
    assert!(answer.get("error").is_none(), "undo failed: {answer}");

    assert!(!out.join("one.txt").exists(), "an extracted file survived the undo");
    assert!(
        !out.join("nested").exists(),
        "an extracted directory survived the undo"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("mine.txt")).unwrap(),
        "written by a person",
        "undo removed a file the extraction never wrote"
    );
    assert!(
        out.exists(),
        "undo removed a directory that was no longer empty"
    );
}

#[test]
fn a_second_extraction_beside_the_first_gets_its_own_name() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let archive = instance.work().join("notes.zip");
    ordinary_zip(&archive, 50);

    client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(false));

    // The default is to refuse rather than to guess, and the refusal has to arrive as an error
    // on the call rather than as a notification later.
    let answer = client.call("fs.extract", json!({ "archive": archive.display().to_string() }));
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("already exists"),
        "a second extraction over the first was not refused: {answer}"
    );

    let answer = client.call(
        "fs.extract",
        json!({ "archive": archive.display().to_string(), "conflict": "keep" }),
    );
    let destination = answer["result"]["destination"].as_str().unwrap().to_string();
    assert!(destination.ends_with("notes (2)"), "{destination}");
    let (_, done) = client.drain_until_done();
    assert_eq!(done["cancelled"], json!(false));
    assert!(Path::new(&destination).join("one.txt").exists());
}

#[test]
fn something_that_is_not_an_archive_is_refused_by_the_call() {
    let instance = Instance::start();
    let mut client = Client::connect(&instance.socket());

    let not_an_archive = instance.work().join("notes.txt");
    std::fs::write(&not_an_archive, "just words").unwrap();

    let answer = client.call(
        "fs.extract",
        json!({ "archive": not_an_archive.display().to_string() }),
    );
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a zip or a tar"),
        "{answer}"
    );
}
