//! `cortex-preview` — the thumbnailer.
//!
//! A process whose whole job is to be killable (D14). Preview generation reads untrusted bytes
//! with third-party decoders, which is the likeliest place in this system for a crash or a
//! memory-safety bug in a dependency; isolating it means a malformed image kills a helper the
//! GUI restarts, and the memory daemon never hears about it.
//!
//! It speaks the same newline-delimited JSON-RPC as everything else here, over stdin and
//! stdout, so its parent can supervise it with a pipe and no protocol of its own.
//!
//! **Progressive by construction.** A request answers twice: a `preview.partial` notification
//! with whatever the shared cache already has at a smaller size, sent before any decoding
//! starts, and then the full render as the response. That is the ordering D14 asks for, and it
//! is why the notification is not merely an optimisation — it is what makes a forty-megapixel
//! photograph appear instantly and sharpen a moment later.

mod cache;
mod render;

use cache::{Cache, ThumbSize};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct PreviewParams {
    path: PathBuf,
    /// The size the pane wants, in logical pixels on the long edge. Multiplied by the device
    /// pixel ratio by the caller, because only the caller knows what its display is doing.
    #[serde(default)]
    pixels: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PreviewResult {
    path: PathBuf,
    /// The PNG to display. Always inside the shared thumbnail cache.
    png: PathBuf,
    width: u32,
    height: u32,
    kind: render::Kind,
    /// Which cache bucket this is, so a caller can tell a placeholder from the real thing.
    size: ThumbSize,
    /// True when this came out of the cache rather than being rendered now.
    cached: bool,
    elapsed_ms: u64,
}

/// A whole viewport's worth of rows, asked about at once.
#[derive(Debug, Deserialize)]
struct ThumbnailsParams {
    paths: Vec<PathBuf>,
    #[serde(default)]
    pixels: Option<u32>,
}

/// What the cache already holds, and what it would take to hold the rest.
///
/// **This never renders.** A list scrolling past two hundred rows must not be able to start two
/// hundred decodes, and a call that *might* render is a call whose cost depends on the
/// directory somebody happens to be in. The client renders the misses it still wants, one at a
/// time, through `preview` — which is the method that costs something and looks like it.
#[derive(Debug, Serialize)]
struct ThumbnailsResult {
    found: Vec<FoundThumbnail>,
    /// Not cached, and worth rendering: the kind says what it would cost.
    missing: Vec<MissingThumbnail>,
    /// Not cached and not worth asking about again — on the deny-list, gone, or already found
    /// un-thumbnailable and unchanged since. Named rather than omitted, so a client can stop
    /// asking instead of retrying the same file on every scroll.
    skipped: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct FoundThumbnail {
    path: PathBuf,
    png: PathBuf,
    width: u32,
    height: u32,
    size: ThumbSize,
}

#[derive(Debug, Serialize)]
struct MissingThumbnail {
    path: PathBuf,
    kind: render::Kind,
}

#[derive(Debug, Serialize)]
struct PartialParams {
    path: PathBuf,
    png: PathBuf,
    size: ThumbSize,
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cortex_preview=warn".into()),
        )
        .init();

    let cache = match Cache::standard() {
        Ok(cache) => cache,
        Err(e) => {
            tracing::error!(error = %e, "no thumbnail cache");
            std::process::exit(1);
        }
    };

    // The deny-list the rest of the system uses (R1). A previewer that rendered what the
    // indexer refuses to read would be a way to look at exactly those files.
    let deny: Vec<String> = Vec::new();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(e) => {
                respond(&mut stdout, serde_json::Value::Null, Err(&e.to_string()));
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(serde_json::json!({}));

        match method {
            "ping" => respond(&mut stdout, id, Ok(serde_json::json!({ "pong": true }))),
            "shutdown" => {
                respond(&mut stdout, id, Ok(serde_json::json!({ "stopping": true })));
                return;
            }
            "preview" => match serde_json::from_value::<PreviewParams>(params) {
                Ok(params) => {
                    let answer = preview(&cache, &deny, &params, &mut stdout);
                    match answer {
                        Ok(result) => respond(&mut stdout, id, Ok(serde_json::json!(result))),
                        Err(reason) => respond(&mut stdout, id, Err(&reason)),
                    }
                }
                Err(e) => respond(&mut stdout, id, Err(&e.to_string())),
            },
            "thumbnails" => match serde_json::from_value::<ThumbnailsParams>(params) {
                Ok(params) => respond(
                    &mut stdout,
                    id,
                    Ok(serde_json::json!(thumbnails(&cache, &deny, &params))),
                ),
                Err(e) => respond(&mut stdout, id, Err(&e.to_string())),
            },
            other => respond(&mut stdout, id, Err(&format!("unknown method: {other}"))),
        }
    }
}

/// Look several paths up in the shared cache, and render none of them.
///
/// One call per viewport rather than one per row: a list that asked per row would make two
/// hundred round trips through a pipe that answers one at a time, and the answer for the row at
/// the bottom would arrive after the person had scrolled past it.
fn thumbnails(cache: &Cache, deny: &[String], params: &ThumbnailsParams) -> ThumbnailsResult {
    let wanted = params.pixels.unwrap_or(128).clamp(1, render::MAX_EDGE);
    let size = ThumbSize::covering(wanted);

    let mut result = ThumbnailsResult {
        found: Vec::new(),
        missing: Vec::new(),
        skipped: Vec::new(),
    };

    for path in &params.paths {
        // R1 first, and before anything is opened, exactly as `preview` does it: a file the
        // deny-list covers must not be distinguishable from one that has no thumbnail.
        if !cortex_core::denylist::classify(path, deny).may_read_content() || !path.is_file() {
            result.skipped.push(path.clone());
            continue;
        }

        // The bucket that was asked for, or a smaller one. A smaller thumbnail in a 20-pixel
        // row is indistinguishable from the right one, and it is already on the disk.
        let hit = cache
            .lookup(path, size)
            .map(|png| (png, size))
            .or_else(|| {
                size.smaller()
                    .into_iter()
                    .find_map(|smaller| cache.lookup(path, smaller).map(|png| (png, smaller)))
            });

        if let Some((png, found_size)) = hit {
            let (width, height) = png_dimensions(&png).unwrap_or((0, 0));
            result.found.push(FoundThumbnail {
                path: path.clone(),
                png,
                width,
                height,
                size: found_size,
            });
            continue;
        }

        if cache.known_failure(path) {
            result.skipped.push(path.clone());
            continue;
        }

        let kind = render::kind_of(path);
        match kind {
            // Text is read through the daemon and never here, and a file with no renderer is
            // not a miss — it is a row that will never have a picture.
            render::Kind::Text | render::Kind::None => result.skipped.push(path.clone()),
            _ => result.missing.push(MissingThumbnail {
                path: path.clone(),
                kind,
            }),
        }
    }
    result
}

fn preview(
    cache: &Cache,
    deny: &[String],
    params: &PreviewParams,
    out: &mut impl Write,
) -> Result<PreviewResult, String> {
    let started = std::time::Instant::now();
    let path = params.path.as_path();

    // Before anything is opened. A refusal here must not depend on whether the file exists,
    // and must certainly not depend on having read it.
    if !cortex_core::denylist::classify(path, deny).may_read_content() {
        return Err(format!(
            "{} is on the deny-list and is never opened",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }

    let wanted = params.pixels.unwrap_or(256).clamp(1, render::MAX_EDGE);
    let size = ThumbSize::covering(wanted);
    let kind = render::kind_of(path);

    // The cheapest possible answer: exactly what was asked for, already rendered.
    if let Some(png) = cache.lookup(path, size) {
        let (width, height) = png_dimensions(&png).unwrap_or((0, 0));
        return Ok(PreviewResult {
            path: path.to_path_buf(),
            png,
            width,
            height,
            kind,
            size,
            cached: true,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }

    if cache.known_failure(path) {
        return Err(format!(
            "{} has already been found un-thumbnailable and has not changed since",
            path.display()
        ));
    }

    // Something to look at *now*, from a smaller bucket, before the decode starts. This is the
    // progressive half of D14, and it costs one stat per size.
    for smaller in size.smaller() {
        if let Some(png) = cache.lookup(path, smaller) {
            notify(
                out,
                "preview.partial",
                serde_json::json!(PartialParams {
                    path: path.to_path_buf(),
                    png,
                    size: smaller,
                }),
            );
            break;
        }
    }

    let rendered = match render::render(path, size.pixels()) {
        Ok(image) => image,
        Err(e) => {
            // Only a genuine "this file cannot be thumbnailed" is remembered. A missing PDF
            // tool is a fact about the machine, and marking the file would mean it stays
            // unpreviewable after the tool is installed.
            if !matches!(e, render::RenderError::MissingTool { .. }) {
                let _ = cache.store_failure(path);
            }
            return Err(e.to_string());
        }
    };

    let (width, height) = (rendered.width(), rendered.height());
    let png = cache
        .store(path, size, &rendered)
        .map_err(|e| format!("could not write the thumbnail: {e}"))?;

    Ok(PreviewResult {
        path: path.to_path_buf(),
        png,
        width,
        height,
        kind,
        size,
        cached: false,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Read a PNG's size out of its header. Cheaper than decoding it to ask.
fn png_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut header = [0u8; 24];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut header).ok()?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(header[16..20].try_into().ok()?),
        u32::from_be_bytes(header[20..24].try_into().ok()?),
    ))
}

fn respond(out: &mut impl Write, id: serde_json::Value, result: Result<serde_json::Value, &str>) {
    let message = match result {
        Ok(value) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(message) => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32000, "message": message }
        }),
    };
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn notify(out: &mut impl Write, method: &str, params: serde_json::Value) {
    let message = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}
