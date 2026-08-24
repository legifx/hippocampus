//! M4 §5 and §6, against the real process.
//!
//! §5: "Preview of a 40-megapixel image appears (cached thumbnail) in < 50 ms and reaches full
//! quality without blocking the UI thread."
//! §6: "Killing the thumbnailer process mid-preview does not affect search or the daemon."
//!
//! Both are claims about a *process*, so both are tested by driving one over its pipes. A test
//! that called the library functions directly would prove neither: the isolation is the whole
//! feature, and a library call has none.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// A running thumbnailer, with a cache of its own so tests do not disturb the desktop's.
struct Previewer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    /// Kept alive for as long as the process that writes into it, when the test owns it.
    _cache: Option<tempfile::TempDir>,
}

impl Previewer {
    fn start(cache: tempfile::TempDir) -> Self {
        let mut previewer = Self::start_at(cache.path());
        previewer._cache = Some(cache);
        previewer
    }

    /// A process pointed at a cache directory somebody else owns — what a restart looks like.
    fn start_at(cache: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cortex-preview"))
            .env("XDG_CACHE_HOME", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the thumbnailer starts");

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            _cache: None,
        }
    }

    fn new_cache() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Send a request and collect everything that comes back until its response, so a caller
    /// sees the notifications that arrived in between.
    fn call(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        let id = self.next_id;
        self.next_id += 1;
        let request =
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        let mut notifications = Vec::new();
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).unwrap();
            assert!(read > 0, "the thumbnailer closed its output");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value.get("id") == Some(&serde_json::json!(id)) {
                return (notifications, value);
            }
            notifications.push(value);
        }
    }

    fn preview(&mut self, path: &Path, pixels: u32) -> (Vec<Value>, Value) {
        self.call(
            "preview",
            serde_json::json!({ "path": path.display().to_string(), "pixels": pixels }),
        )
    }
}

impl Drop for Previewer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A JPEG of the given size, with content a decoder has to work at.
fn photograph(path: &Path, width: u32, height: u32) {
    let mut image = image::RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        pixel.0 = [
            ((x * 7) % 256) as u8,
            ((y * 5) % 256) as u8,
            ((x ^ y) % 256) as u8,
        ];
    }
    image
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

fn thumbnail_dir(cache: &Path, size: &str) -> PathBuf {
    cache.join("thumbnails").join(size)
}

#[test]
fn a_forty_megapixel_photograph_is_previewed_and_then_found_in_the_cache_instantly() {
    let cache = Previewer::new_cache();
    let cache_path = cache.path().to_path_buf();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("enormous.jpg");

    // Forty megapixels, as §5 names. Roughly 7300 × 5500.
    photograph(&source, 7300, 5480);

    let mut previewer = Previewer::start(cache);

    // The first request has to do the work.
    let (_, first) = previewer.preview(&source, 256);
    let result = &first["result"];
    assert!(first.get("error").is_none(), "first render failed: {first}");
    assert_eq!(result["cached"], serde_json::json!(false));
    assert_eq!(result["kind"], serde_json::json!("image"));
    assert_eq!(result["width"].as_u64().unwrap().max(result["height"].as_u64().unwrap()), 256);

    let png = PathBuf::from(result["png"].as_str().unwrap());
    assert!(png.starts_with(thumbnail_dir(&cache_path, "large")));

    // The second is the number §5 is about: the cached thumbnail appearing.
    let started = std::time::Instant::now();
    let (_, second) = previewer.preview(&source, 256);
    let elapsed = started.elapsed();

    assert_eq!(second["result"]["cached"], serde_json::json!(true));
    assert_eq!(second["result"]["png"], result["png"]);
    println!(
        "cached preview of a {:.0} MP photograph in {} µs (budget 50 ms)",
        7300.0 * 5480.0 / 1e6,
        elapsed.as_micros()
    );
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "a cached thumbnail took {elapsed:?}, and §5 allows 50 ms"
    );
}

#[test]
fn a_small_thumbnail_is_offered_before_the_big_one_is_rendered() {
    let cache = Previewer::new_cache();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("photo.jpg");
    photograph(&source, 3000, 2000);

    let mut previewer = Previewer::start(cache);

    // Render the small bucket first, the way browsing a directory would.
    let (_, small) = previewer.preview(&source, 128);
    let small_png = small["result"]["png"].as_str().unwrap().to_string();

    // Now ask for a bigger one. The progressive contract says the small one arrives first, as
    // a notification, before any decoding starts.
    let (notifications, large) = previewer.preview(&source, 512);

    assert_eq!(
        notifications.len(),
        1,
        "expected exactly one partial, got {notifications:?}"
    );
    assert_eq!(notifications[0]["method"], serde_json::json!("preview.partial"));
    assert_eq!(notifications[0]["params"]["png"], serde_json::json!(small_png));
    assert_eq!(notifications[0]["params"]["size"], serde_json::json!("normal"));

    assert_eq!(large["result"]["size"], serde_json::json!("x-large"));
    assert_ne!(large["result"]["png"], serde_json::json!(small_png));
}

#[test]
fn nothing_is_offered_early_when_there_is_nothing_to_offer() {
    let cache = Previewer::new_cache();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("fresh.png");
    photograph(&source, 600, 400);

    let mut previewer = Previewer::start(cache);
    let (notifications, _) = previewer.preview(&source, 512);
    assert!(
        notifications.is_empty(),
        "a cold cache has no placeholder to send, and inventing one would show the wrong picture"
    );
}

#[test]
fn a_thumbnail_written_by_this_process_is_one_any_file_manager_can_find() {
    let cache = Previewer::new_cache();
    let cache_path = cache.path().to_path_buf();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("shared.png");
    photograph(&source, 400, 300);
    // Written as a PNG despite the JPEG helper's name — the extension decides the decoder.
    image::open(&source).ok();

    let mut previewer = Previewer::start(cache);
    let (_, answer) = previewer.preview(&source, 128);
    let png = PathBuf::from(answer["result"]["png"].as_str().unwrap());

    // The name is the MD5 of the URI, under `normal/`, exactly where every other thumbnailer
    // on the machine looks.
    assert_eq!(png.parent().unwrap(), thumbnail_dir(&cache_path, "normal"));
    assert_eq!(png.extension().unwrap(), "png");
    assert_eq!(png.file_stem().unwrap().len(), 32);

    let data = std::fs::read(&png).unwrap();
    let text = String::from_utf8_lossy(&data[..data.len().min(2048)]);
    assert!(text.contains("Thumb::URI"), "no URI stamp");
    assert!(text.contains("Thumb::MTime"), "no mtime stamp — nothing could invalidate it");
}

#[test]
fn a_denied_path_is_refused_without_being_opened() {
    let cache = Previewer::new_cache();
    let work = tempfile::tempdir().unwrap();
    let ssh = work.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_ed25519.png");
    photograph(&key, 100, 100);

    let mut previewer = Previewer::start(cache);
    let (_, answer) = previewer.preview(&key, 128);

    let message = answer["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("deny-list"),
        "a previewer that renders what the indexer refuses to read is a way around R1: {answer}"
    );
}

#[test]
fn a_broken_file_is_refused_once_and_remembered() {
    let cache = Previewer::new_cache();
    let cache_path = cache.path().to_path_buf();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("truncated.png");
    std::fs::write(&source, b"\x89PNG\r\n\x1a\n and then nothing at all").unwrap();

    let mut previewer = Previewer::start(cache);
    let (_, first) = previewer.preview(&source, 128);
    assert!(first.get("error").is_some());

    // The marker is what stops every file manager on the machine from retrying it.
    let fail_dir = cache_path.join("thumbnails/fail/cortex-preview");
    assert!(fail_dir.is_dir(), "no failure marker was written");

    let (_, second) = previewer.preview(&source, 128);
    let message = second["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("already been found"),
        "the second attempt re-read a file already known to be undecodable: {second}"
    );
}

#[test]
fn the_process_survives_a_file_that_would_take_the_whole_machine() {
    let cache = Previewer::new_cache();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("enormous.png");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(1024 * 1024 * 1024).unwrap();

    let mut previewer = Previewer::start(cache);
    let (_, answer) = previewer.preview(&source, 128);
    assert!(answer.get("error").is_some());

    // And it is still answering afterwards, which is the property that matters.
    let (_, pong) = previewer.call("ping", serde_json::json!({}));
    assert_eq!(pong["result"]["pong"], serde_json::json!(true));
}

#[test]
fn killing_it_mid_preview_leaves_nothing_behind_but_a_dead_process() {
    let cache = Previewer::new_cache();
    let cache_path = cache.path().to_path_buf();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("enormous.jpg");
    photograph(&source, 7300, 5480);

    let mut previewer = Previewer::start(cache);

    // Ask for the most expensive render there is, then kill the process while it runs. §6 asks
    // that this be harmless; the thing that could make it harmful is a half-written thumbnail
    // in the shared cache that another file manager then trusts.
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 99, "method": "preview",
        "params": { "path": source.display().to_string(), "pixels": 2048 }
    });
    writeln!(previewer.stdin, "{request}").unwrap();
    previewer.stdin.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    previewer.child.kill().unwrap();
    let status = previewer.child.wait().unwrap();
    assert!(!status.success(), "the process was supposed to be killed");

    // Whatever is in the cache must be either complete or absent — never a truncated PNG under
    // the name the whole desktop shares.
    for size in ["normal", "large", "x-large", "xx-large"] {
        let dir = thumbnail_dir(&cache_path, size);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                // A `.tmpNNN` file is the rename-over pattern doing its job: it is not under a
                // name anything looks for, and it is not a corrupt thumbnail.
                continue;
            }
            assert!(
                image::open(&path).is_ok(),
                "{} is in the shared cache under a real name and does not decode",
                path.display()
            );
        }
    }
}

#[test]
fn a_new_process_picks_up_what_the_dead_one_had_already_written() {
    let cache = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("photo.jpg");
    photograph(&source, 2000, 1500);

    {
        let mut first = Previewer::start_at(cache.path());
        let (_, answer) = first.preview(&source, 256);
        assert_eq!(answer["result"]["cached"], serde_json::json!(false));
        // Killed rather than shut down: this is a crash, which is the case D14 designs for.
        first.child.kill().unwrap();
        first.child.wait().unwrap();
    }

    let mut restarted = Previewer::start_at(cache.path());
    let (_, again) = restarted.preview(&source, 256);
    assert_eq!(
        again["result"]["cached"],
        serde_json::json!(true),
        "a restarted thumbnailer re-rendered work that was already in the shared cache"
    );
}

/// A viewport's worth of rows, asked about at once — and never rendered.
///
/// This is the call the list makes while somebody scrolls, so its cost has to be a stat per path
/// whatever is in the directory. The three answers are the three states a row can be in: it has
/// a picture, it could have one, or it never will.
#[test]
fn a_whole_viewport_is_looked_up_at_once_and_nothing_is_rendered() {
    let cache = Previewer::new_cache();
    let cache_path = cache.path().to_path_buf();
    let work = tempfile::tempdir().unwrap();

    let cached = work.path().join("already-seen.jpg");
    let uncached = work.path().join("never-seen.jpg");
    let text = work.path().join("notes.md");
    let gone = work.path().join("not-there.png");
    photograph(&cached, 400, 300);
    photograph(&uncached, 400, 300);
    std::fs::write(&text, "words").unwrap();

    let mut previewer = Previewer::start(cache);
    // One of them has been previewed before, which is the only way into the cache.
    previewer.preview(&cached, 128);

    let before: Vec<PathBuf> = std::fs::read_dir(thumbnail_dir(&cache_path, "normal"))
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();

    let (_, answer) = previewer.call(
        "thumbnails",
        serde_json::json!({
            "paths": [
                cached.display().to_string(),
                uncached.display().to_string(),
                text.display().to_string(),
                gone.display().to_string(),
            ],
            "pixels": 128
        }),
    );
    let result = &answer["result"];

    let found: Vec<&str> = result["found"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(found, vec![cached.display().to_string()]);
    assert!(
        std::path::Path::new(result["found"][0]["png"].as_str().unwrap()).exists(),
        "the answer names a file that is not there"
    );

    let missing: Vec<&str> = result["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["path"].as_str().unwrap())
        .collect();
    assert_eq!(missing, vec![uncached.display().to_string()]);
    assert_eq!(result["missing"][0]["kind"], serde_json::json!("image"));

    // A text file and a path that is not there are both "stop asking": one is read through the
    // daemon, the other has nothing to read. A client that had to tell them apart would end up
    // retrying one of them on every scroll.
    let skipped: Vec<&str> = result["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(
        skipped,
        vec![text.display().to_string(), gone.display().to_string()]
    );

    let after: Vec<PathBuf> = std::fs::read_dir(thumbnail_dir(&cache_path, "normal"))
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert_eq!(
        before.len(),
        after.len(),
        "the lookup rendered something; scrolling a directory of photographs would start a \
         decode per row"
    );
}

/// R1 does not weaken because the question was asked about several files at once.
#[test]
fn a_denied_path_is_skipped_by_the_batch_rather_than_named() {
    let cache = Previewer::new_cache();
    let work = tempfile::tempdir().unwrap();
    let ssh = work.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_ed25519.png");
    photograph(&key, 100, 100);

    let mut previewer = Previewer::start(cache);
    let (_, answer) = previewer.call(
        "thumbnails",
        serde_json::json!({ "paths": [key.display().to_string()], "pixels": 128 }),
    );
    let result = &answer["result"];

    assert!(result["found"].as_array().unwrap().is_empty());
    assert!(
        result["missing"].as_array().unwrap().is_empty(),
        "a denied file was offered as something worth rendering"
    );
    assert_eq!(result["skipped"].as_array().unwrap().len(), 1);
}
