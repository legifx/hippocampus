//! Turning a file into pixels.
//!
//! The quality rule from D14, in order of how much it matters:
//!
//! 1. **Never decode a full-resolution image to show a small preview.** A forty-megapixel JPEG
//!    is 160 MB of RGBA if it is decoded and then shrunk, and the shrinking throws away
//!    99.9 % of it. JPEG's DCT structure allows decoding at 1/2, 1/4 or 1/8 scale for almost
//!    nothing, and that is what this does.
//! 2. **Cap the long edge at 2048 px.** Above that nothing on screen is better, and the memory
//!    is real.
//! 3. **Render at the displayed size times the device pixel ratio**, not at whatever the file
//!    happens to be.
//!
//! Everything here runs in this process rather than the daemon or the GUI, and that is the
//! decision the whole crate exists for: these decoders read untrusted bytes, which is the most
//! likely place in the system for a crash or a memory-safety bug in a dependency. A malformed
//! image kills a helper that the GUI restarts, and the memory daemon never notices.

use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, RgbaImage};
use std::io::BufReader;
use std::path::Path;

/// The hard cap on the long edge, from D14.
pub const MAX_EDGE: u32 = 2048;

/// Larger than this and the file is not opened at all.
///
/// Not a performance guard — a guard against a decoder being handed something absurd. The
/// budget it protects is the machine's, and a two-gigabyte "image" is a denial of service
/// whether or not it is a deliberate one.
pub const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("{0}")]
    Unsupported(String),

    #[error("{path} is {size} bytes, which is more than this will open")]
    TooLarge { path: String, size: u64 },

    #[error("could not decode {path}: {reason}")]
    Decode { path: String, reason: String },

    #[error("i/o error on {path}: {reason}")]
    Io { path: String, reason: String },

    #[error("{tool} is not installed, so {kind} previews are unavailable")]
    MissingTool { tool: String, kind: String },
}

/// What kind of preview a path can have. Decided from the extension, because opening a file to
/// find out costs the same as the preview itself for the cases that are not previewable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Pdf,
    Video,
    Audio,
    /// Text and code are read through the daemon, not here — that path applies the deny-list
    /// and writes an audit entry, and a previewer that opened them directly would be a way
    /// around both.
    Text,
    None,
}

pub fn kind_of(path: &Path) -> Kind {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" => Kind::Image,
        "pdf" => Kind::Pdf,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => Kind::Video,
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" => Kind::Audio,
        "md" | "txt" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "h" | "cpp"
        | "hpp" | "java" | "kt" | "rb" | "sh" | "toml" | "yaml" | "yml" | "json" | "xml"
        | "html" | "css" | "sql" | "csv" | "ini" | "conf" | "log" | "kdl" | "lua" => Kind::Text,
        _ => Kind::None,
    }
}

/// Render `path` into an image whose long edge is at most `target`.
pub fn render(path: &Path, target: u32) -> Result<RgbaImage, RenderError> {
    let target = target.min(MAX_EDGE).max(1);

    let metadata = std::fs::metadata(path).map_err(|e| RenderError::Io {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(RenderError::TooLarge {
            path: path.display().to_string(),
            size: metadata.len(),
        });
    }

    match kind_of(path) {
        Kind::Image => render_image(path, target),
        Kind::Pdf => render_with_tool(path, target, Tool::Pdf),
        Kind::Video => render_with_tool(path, target, Tool::Video),
        Kind::Audio => Err(RenderError::Unsupported(
            "audio has no picture; the pane shows its metadata instead".into(),
        )),
        Kind::Text => Err(RenderError::Unsupported(
            "text is read through the daemon, where the deny-list applies".into(),
        )),
        Kind::None => Err(RenderError::Unsupported(
            "no preview is defined for this kind of file".into(),
        )),
    }
}

fn render_image(path: &Path, target: u32) -> Result<RgbaImage, RenderError> {
    let file = std::fs::File::open(path).map_err(|e| RenderError::Io {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;

    let reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| RenderError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    let format = reader.format();
    let decoded: DynamicImage = match format {
        // JPEG can be decoded at a fraction of its size almost for free, because the scaling
        // happens in the DCT domain rather than on pixels. This is the difference between
        // reading forty megapixels and reading six hundred kilopixels.
        Some(image::ImageFormat::Jpeg) => {
            let (width, height) = reader.into_dimensions().map_err(|e| RenderError::Decode {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;
            decode_jpeg_scaled(path, width, height, target)?
        }
        _ => reader.decode().map_err(|e| RenderError::Decode {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?,
    };

    Ok(fit(decoded, target))
}

/// Decode a JPEG at the smallest DCT scale that still covers the target.
///
/// `jpeg-decoder` rather than the `image` crate's own JPEG path: `image` 0.25 decodes JPEG with
/// zune-jpeg, which produces full-resolution pixels and nothing else. Scaling in the DCT domain
/// is the difference between reading forty megapixels and reading six hundred kilopixels, and
/// it is the specific promise D14 makes.
fn decode_jpeg_scaled(
    path: &Path,
    width: u32,
    height: u32,
    target: u32,
) -> Result<DynamicImage, RenderError> {
    let fail = |reason: String| RenderError::Decode {
        path: path.display().to_string(),
        reason,
    };

    let file = std::fs::File::open(path).map_err(|e| RenderError::Io {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));

    // The decoder rounds up to a scale it can actually do — 1/1, 1/2, 1/4 or 1/8 — so asking
    // for exactly the target never returns something too small to use.
    let long_edge = width.max(height).max(1);
    let ratio = (target as f32 / long_edge as f32).min(1.0);
    let wanted = (
        ((width as f32 * ratio).ceil() as u16).max(1),
        ((height as f32 * ratio).ceil() as u16).max(1),
    );
    decoder
        .scale(wanted.0, wanted.1)
        .map_err(|e| fail(e.to_string()))?;

    let pixels = decoder.decode().map_err(|e| fail(e.to_string()))?;
    let info = decoder
        .info()
        .ok_or_else(|| fail("the decoder produced no image information".into()))?;
    let (w, h) = (info.width as u32, info.height as u32);

    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => image::RgbImage::from_raw(w, h, pixels)
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| fail("the decoded pixels do not match the stated size".into())),
        jpeg_decoder::PixelFormat::L8 => image::GrayImage::from_raw(w, h, pixels)
            .map(DynamicImage::ImageLuma8)
            .ok_or_else(|| fail("the decoded pixels do not match the stated size".into())),
        jpeg_decoder::PixelFormat::L16 => {
            let samples: Vec<u16> = pixels
                .chunks_exact(2)
                .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                .collect();
            image::ImageBuffer::<image::Luma<u16>, _>::from_raw(w, h, samples)
                .map(DynamicImage::ImageLuma16)
                .ok_or_else(|| fail("the decoded pixels do not match the stated size".into()))
        }
        // CMYK is rare enough, and its conversion involved enough, that guessing at it would
        // produce a preview with the wrong colours. Saying so is better than showing that.
        other => Err(fail(format!(
            "this JPEG is {other:?}, which has no faithful conversion here"
        ))),
    }
}

/// Shrink to fit inside `target` on the long edge, preserving aspect. Never enlarges: an
/// upscaled thumbnail looks like a rendering bug rather than a small picture.
pub fn fit(image: DynamicImage, target: u32) -> RgbaImage {
    let (width, height) = (image.width().max(1), image.height().max(1));
    let long_edge = width.max(height);

    if long_edge <= target {
        return image.into_rgba8();
    }

    let ratio = target as f64 / long_edge as f64;
    let new = (
        ((width as f64 * ratio).round() as u32).max(1),
        ((height as f64 * ratio).round() as u32).max(1),
    );
    // Lanczos for the final step: this is the picture somebody looks at, and the cost is paid
    // once per file rather than once per frame.
    image.resize_exact(new.0, new.1, FilterType::Lanczos3).into_rgba8()
}

// ---------------------------------------------------------------------------
// Formats that need somebody else's decoder.
// ---------------------------------------------------------------------------

/// PDFs and video frames are rendered by whatever the machine already has.
///
/// Bundling a PDF engine or a demuxer would mean shipping and updating two of the largest
/// attack surfaces in desktop software. Where the tool is absent the pane says so, which is a
/// better outcome than a preview that silently never appears.
#[derive(Debug, Clone, Copy)]
enum Tool {
    Pdf,
    Video,
}

impl Tool {
    fn kind(self) -> &'static str {
        match self {
            Tool::Pdf => "PDF",
            Tool::Video => "video",
        }
    }

    /// Candidate commands, in preference order.
    fn candidates(self) -> &'static [&'static str] {
        match self {
            Tool::Pdf => &["pdftoppm", "mutool"],
            Tool::Video => &["ffmpeg"],
        }
    }
}

fn render_with_tool(path: &Path, target: u32, tool: Tool) -> Result<RgbaImage, RenderError> {
    let Some(program) = tool.candidates().iter().copied().find(|c| on_path(c)) else {
        return Err(RenderError::MissingTool {
            tool: tool.candidates().join(" or "),
            kind: tool.kind().to_string(),
        });
    };

    let dir = std::env::temp_dir().join(format!("cortex-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| RenderError::Io {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })?;
    let output = dir.join("frame.png");
    let _ = std::fs::remove_file(&output);

    let status = match (tool, program) {
        (Tool::Pdf, "pdftoppm") => std::process::Command::new("pdftoppm")
            .args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to"])
            .arg(target.to_string())
            .arg(path)
            // pdftoppm appends `.png` itself, so it is given the stem.
            .arg(output.with_extension(""))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
        (Tool::Pdf, _) => std::process::Command::new("mutool")
            .args(["draw", "-F", "png", "-w"])
            .arg(target.to_string())
            .args(["-o"])
            .arg(&output)
            .arg(path)
            .arg("1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
        (Tool::Video, _) => std::process::Command::new("ffmpeg")
            .args(["-loglevel", "quiet", "-i"])
            .arg(path)
            // A frame from a little way in: the first frame of a video is very often black.
            .args(["-ss", "00:00:03", "-frames:v", "1", "-vf"])
            .arg(format!("scale={target}:-1:force_original_aspect_ratio=decrease"))
            .args(["-y"])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status(),
    };

    let ok = status.map(|s| s.success()).unwrap_or(false);
    if !ok || !output.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(RenderError::Decode {
            path: path.display().to_string(),
            reason: format!("{program} produced no frame"),
        });
    }

    let rendered = image::open(&output).map_err(|e| RenderError::Decode {
        path: output.display().to_string(),
        reason: e.to_string(),
    });
    let _ = std::fs::remove_file(&output);
    let _ = std::fs::remove_dir(&dir);

    Ok(fit(rendered?, target))
}

fn on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_image(path: &Path, width: u32, height: u32, format: image::ImageFormat) {
        let mut image = image::RgbImage::new(width, height);
        // A gradient rather than a flat colour: a flat image compresses to nothing and would
        // not exercise a decoder the way a real photograph does.
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8]);
        }
        image.save_with_format(path, format).unwrap();
    }

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(kind_of(Path::new("/a/b.PNG")), Kind::Image);
        assert_eq!(kind_of(Path::new("/a/b.pdf")), Kind::Pdf);
        assert_eq!(kind_of(Path::new("/a/b.mkv")), Kind::Video);
        assert_eq!(kind_of(Path::new("/a/b.flac")), Kind::Audio);
        assert_eq!(kind_of(Path::new("/a/b.rs")), Kind::Text);
        assert_eq!(kind_of(Path::new("/a/binary")), Kind::None);
    }

    #[test]
    fn a_thumbnail_fits_inside_the_size_it_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wide.png");
        write_image(&source, 800, 200, image::ImageFormat::Png);

        let rendered = render(&source, 128).unwrap();
        assert_eq!(rendered.width().max(rendered.height()), 128);
        // And the shape is kept: 800×200 is 4:1, so 128 wide is 32 tall.
        assert_eq!((rendered.width(), rendered.height()), (128, 32));
    }

    #[test]
    fn a_small_image_is_never_enlarged() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tiny.png");
        write_image(&source, 40, 30, image::ImageFormat::Png);

        let rendered = render(&source, 512).unwrap();
        assert_eq!(
            (rendered.width(), rendered.height()),
            (40, 30),
            "an upscaled thumbnail looks like a bug rather than a small picture"
        );
    }

    #[test]
    fn the_long_edge_is_capped_however_much_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("huge.png");
        write_image(&source, 4000, 3000, image::ImageFormat::Png);

        let rendered = render(&source, 100_000).unwrap();
        assert_eq!(rendered.width(), MAX_EDGE, "D14 caps the long edge at 2048");
    }

    #[test]
    fn a_large_jpeg_is_not_decoded_at_full_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.jpg");
        // Four megapixels: big enough that a full decode is measurably slower than a scaled
        // one, small enough that the test stays quick.
        write_image(&source, 2048, 2048, image::ImageFormat::Jpeg);

        let scaled = std::time::Instant::now();
        let small = render(&source, 128).unwrap();
        let scaled = scaled.elapsed();

        let full = std::time::Instant::now();
        let large = render(&source, 2048).unwrap();
        let full = full.elapsed();

        assert_eq!(small.width().max(small.height()), 128);
        assert_eq!(large.width().max(large.height()), 2048);

        // The claim in D14 is that a small preview does not pay for a full decode. If the two
        // cost the same, the scaling is not happening where it was supposed to.
        assert!(
            scaled < full,
            "a 128 px preview took {scaled:?} and a full-size one {full:?}; \
             the DCT scaling is not being used"
        );
    }

    #[test]
    fn a_truncated_file_fails_with_a_reason_rather_than_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("broken.png");
        std::fs::write(&source, b"\x89PNG\r\n\x1a\n and then nothing").unwrap();

        let error = render(&source, 128).unwrap_err();
        assert!(matches!(error, RenderError::Decode { .. }), "{error}");
    }

    #[test]
    fn an_absurdly_large_file_is_not_opened_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("enormous.png");
        let file = std::fs::File::create(&source).unwrap();
        // A sparse file: the size is real, the bytes are not written.
        file.set_len(MAX_INPUT_BYTES + 1).unwrap();

        let error = render(&source, 128).unwrap_err();
        assert!(matches!(error, RenderError::TooLarge { .. }), "{error}");
    }

    #[test]
    fn text_is_refused_here_because_the_daemon_owns_that_path() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.md");
        std::fs::write(&source, "# heading").unwrap();

        let error = render(&source, 128).unwrap_err();
        assert!(error.to_string().contains("deny-list"), "{error}");
    }

    #[test]
    fn a_missing_tool_is_named_rather_than_reported_as_a_broken_file() {
        // Whether these are installed differs per machine, so the assertion is on the *shape*
        // of the answer: either a rendered frame, or an error that names the missing program.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("document.pdf");
        std::fs::write(&source, b"%PDF-1.4\n% not really a pdf\n").unwrap();

        match render(&source, 128) {
            Err(RenderError::MissingTool { tool, kind }) => {
                assert!(tool.contains("pdftoppm"));
                assert_eq!(kind, "PDF");
            }
            Err(RenderError::Decode { .. }) => {}
            other => panic!("unexpected outcome for a broken PDF: {other:?}"),
        }
    }
}
