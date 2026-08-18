//! The shared thumbnail cache, as the XDG Thumbnail Managing Standard defines it.
//!
//! Sharing it is the point. Every file manager on the machine writes to the same directory
//! under the same naming rules, so a thumbnail this process renders is one Nautilus does not
//! have to, and vice versa. That is worth getting exactly right: a thumbnail written under a
//! name nobody else computes is not a shared cache, it is a private one that happens to live in
//! a shared directory.
//!
//! The rules, in full, because each one has bitten somebody:
//!
//! - The file name is the **MD5 of the file's URI**, not of its path and not of its contents.
//!   `file:///home/user/a.png`, percent-encoded, lowercase hex, `.png`.
//! - The PNG carries `Thumb::URI` and `Thumb::MTime` as `tEXt` chunks, and a thumbnail whose
//!   `Thumb::MTime` differs from the file's current mtime is **stale by definition** — that is
//!   the whole invalidation mechanism, and it is why the chunks are not optional.
//! - Directories are `0700` and files `0600`. A thumbnail of a private document is as revealing
//!   as the document.
//! - A file that cannot be thumbnailed gets a marker under `fail/<application>/`, so that every
//!   file manager on the machine stops retrying it rather than each learning separately.

use md5::{Digest, Md5};
use std::path::{Path, PathBuf};

/// The sizes the standard defines, longest edge in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThumbSize {
    Normal,
    Large,
    XLarge,
    XxLarge,
}

impl ThumbSize {
    pub fn pixels(self) -> u32 {
        match self {
            ThumbSize::Normal => 128,
            ThumbSize::Large => 256,
            ThumbSize::XLarge => 512,
            ThumbSize::XxLarge => 1024,
        }
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            ThumbSize::Normal => "normal",
            ThumbSize::Large => "large",
            ThumbSize::XLarge => "x-large",
            ThumbSize::XxLarge => "xx-large",
        }
    }

    /// The smallest standard size that covers `wanted` pixels on the long edge.
    ///
    /// Rendering into a bigger bucket than needed wastes the decode; rendering into a smaller
    /// one means showing an upscaled thumbnail, which looks like a bug.
    pub fn covering(wanted: u32) -> ThumbSize {
        for size in [
            ThumbSize::Normal,
            ThumbSize::Large,
            ThumbSize::XLarge,
            ThumbSize::XxLarge,
        ] {
            if size.pixels() >= wanted {
                return size;
            }
        }
        ThumbSize::XxLarge
    }

    /// Every size smaller than this one, largest first — the ladder a progressive preview
    /// climbs down looking for something to show immediately.
    pub fn smaller(self) -> Vec<ThumbSize> {
        let all = [
            ThumbSize::Normal,
            ThumbSize::Large,
            ThumbSize::XLarge,
            ThumbSize::XxLarge,
        ];
        all.iter()
            .copied()
            .filter(|s| s.pixels() < self.pixels())
            .rev()
            .collect()
    }

}

/// Where thumbnails live.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
    /// Written into the `fail/` marker path, per the standard.
    application: String,
}

impl Cache {
    /// The standard location: `$XDG_CACHE_HOME/thumbnails`, falling back to `~/.cache`.
    pub fn standard() -> std::io::Result<Self> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "neither XDG_CACHE_HOME nor HOME is set, so there is nowhere to cache",
                )
            })?;
        Ok(Self::at(base.join("thumbnails")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            application: "cortex-preview".to_string(),
        }
    }

    /// Where a thumbnail of `path` at `size` belongs, whether or not it is there.
    pub fn entry(&self, path: &Path, size: ThumbSize) -> PathBuf {
        self.root
            .join(size.dir_name())
            .join(format!("{}.png", digest(&file_uri(path))))
    }

    /// Where the "this cannot be thumbnailed" marker belongs.
    pub fn failure_marker(&self, path: &Path) -> PathBuf {
        self.root
            .join("fail")
            .join(&self.application)
            .join(format!("{}.png", digest(&file_uri(path))))
    }

    /// A usable thumbnail, or nothing.
    ///
    /// "Usable" means present *and* stamped with the file's current mtime. A stale thumbnail is
    /// worse than none: it shows the previous contents of a file with complete confidence.
    pub fn lookup(&self, path: &Path, size: ThumbSize) -> Option<PathBuf> {
        let entry = self.entry(path, size);
        let stamped = read_mtime_stamp(&entry)?;
        let actual = mtime_of(path)?;
        (stamped == actual).then_some(entry)
    }

    /// Has this file already been found un-thumbnailable, and not changed since?
    pub fn known_failure(&self, path: &Path) -> bool {
        let marker = self.failure_marker(path);
        match (read_mtime_stamp(&marker), mtime_of(path)) {
            (Some(stamped), Some(actual)) => stamped == actual,
            _ => false,
        }
    }

    /// Write a rendered thumbnail, with the metadata that makes it findable and invalidatable.
    pub fn store(
        &self,
        path: &Path,
        size: ThumbSize,
        image: &image::RgbaImage,
    ) -> std::io::Result<PathBuf> {
        let entry = self.entry(path, size);
        self.write_png(&entry, path, image)?;
        Ok(entry)
    }

    /// Record that this file cannot be thumbnailed, so no file manager on the machine keeps
    /// trying. The marker is a 1×1 image, as the standard prescribes.
    pub fn store_failure(&self, path: &Path) -> std::io::Result<()> {
        let marker = self.failure_marker(path);
        let pixel = image::RgbaImage::new(1, 1);
        self.write_png(&marker, path, &pixel)
    }

    fn write_png(
        &self,
        destination: &Path,
        source: &Path,
        image: &image::RgbaImage,
    ) -> std::io::Result<()> {
        if let Some(parent) = destination.parent() {
            create_private_dir(parent)?;
        }

        let uri = file_uri(source);
        let mtime = mtime_of(source).unwrap_or(0);
        let size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);

        let png = encode_png(
            image,
            &[
                ("Thumb::URI", uri.as_str()),
                ("Thumb::MTime", &mtime.to_string()),
                ("Thumb::Size", &size.to_string()),
                ("Software", "Hippocampus cortex-preview"),
            ],
        )?;

        // Write to a temporary name in the same directory and rename over: another file manager
        // reading this path must never see half a PNG, and rename is the only way to promise it.
        let temporary = destination.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&temporary, &png)?;
        restrict(&temporary)?;
        std::fs::rename(&temporary, destination)
    }
}

/// The URI of a local path, percent-encoded as the standard requires.
///
/// Encoded by hand rather than with a dependency: the rule is "everything outside the unreserved
/// set", it is four lines, and a URI encoder that disagreed with the rest of the desktop by one
/// character would produce thumbnails nobody else can find.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub fn digest(uri: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(uri.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn mtime_of(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Read `Thumb::MTime` out of a PNG's text chunks without decoding the image.
///
/// Walking the chunk headers is a few hundred bytes of reading; decoding a 1024×1024 PNG to
/// find out it is stale would cost more than rendering a new one.
pub fn read_mtime_stamp(png: &Path) -> Option<i64> {
    let data = std::fs::read(png).ok()?;
    read_text_chunk(&data, "Thumb::MTime")?.parse().ok()
}

pub fn read_text_chunk(png: &[u8], key: &str) -> Option<String> {
    // 8-byte signature, then length/type/data/crc chunks.
    if png.len() < 8 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut offset = 8;
    while offset + 8 <= png.len() {
        let length = u32::from_be_bytes(png[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &png[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length)?;
        if data_end > png.len() {
            return None;
        }

        if kind == b"tEXt" {
            let chunk = &png[data_start..data_end];
            if let Some(split) = chunk.iter().position(|b| *b == 0) {
                if &chunk[..split] == key.as_bytes() {
                    return String::from_utf8(chunk[split + 1..].to_vec()).ok();
                }
            }
        }
        if kind == b"IDAT" || kind == b"IEND" {
            // Text chunks that matter come before the pixels; anything after is not metadata
            // this cares about, and scanning a megabyte of image data for it is waste.
            return None;
        }
        offset = data_end + 4;
    }
    None
}

/// Encode an image as a PNG carrying the given `tEXt` chunks.
///
/// The `image` crate's encoder writes no text chunks, so the header is assembled here and the
/// encoder's own output is spliced in after it. That is less clever than it sounds: a PNG is a
/// signature followed by chunks, and inserting chunks between two of them is concatenation.
fn encode_png(image: &image::RgbaImage, text: &[(&str, &str)]) -> std::io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .map_err(|e| std::io::Error::other(format!("could not encode a thumbnail: {e}")))?;

    // Everything up to and including IHDR stays; the text chunks go straight after it.
    let ihdr_end = find_chunk_end(&encoded, b"IHDR")
        .ok_or_else(|| std::io::Error::other("the encoder produced a PNG with no header"))?;

    let mut out = Vec::with_capacity(encoded.len() + 256);
    out.extend_from_slice(&encoded[..ihdr_end]);
    for (key, value) in text {
        out.extend_from_slice(&text_chunk(key, value));
    }
    out.extend_from_slice(&encoded[ihdr_end..]);
    Ok(out)
}

fn find_chunk_end(png: &[u8], kind: &[u8; 4]) -> Option<usize> {
    let mut offset = 8;
    while offset + 8 <= png.len() {
        let length = u32::from_be_bytes(png[offset..offset + 4].try_into().ok()?) as usize;
        let this = &png[offset + 4..offset + 8];
        let end = offset + 12 + length;
        if this == kind {
            return Some(end);
        }
        offset = end;
    }
    None
}

fn text_chunk(key: &str, value: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(key.len() + value.len() + 1);
    data.extend_from_slice(key.as_bytes());
    data.push(0);
    data.extend_from_slice(value.as_bytes());

    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(&data);

    let mut crc_input = Vec::with_capacity(data.len() + 4);
    crc_input.extend_from_slice(b"tEXt");
    crc_input.extend_from_slice(&data);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    chunk
}

/// PNG's CRC-32. Written out because pulling a compression crate in for one polynomial would be
/// a dependency for four lines of arithmetic.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([220, 40, 40, 255]))
    }

    #[test]
    fn the_uri_matches_what_the_rest_of_the_desktop_computes() {
        // Plain MD5 of the URI bytes, pinned against values computed outside this code. If it
        // disagrees, every thumbnail this process writes lands under a name no other file
        // manager looks for — and nothing would fail, it would just quietly stop sharing.
        assert_eq!(
            digest("file:///home/user/photo/me.png"),
            "b455dc41961b534b99fb4974a19a109d"
        );
        assert_eq!(digest(""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(digest("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn spaces_and_non_ascii_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new("/tmp/a b/ü.png")),
            "file:///tmp/a%20b/%C3%BC.png"
        );
    }

    #[test]
    fn a_thumbnail_is_found_by_the_name_the_standard_gives_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("picture.png");
        std::fs::write(&source, b"not really a png").unwrap();

        let entry = cache.entry(&source, ThumbSize::Large);
        assert_eq!(entry.parent().unwrap().file_name().unwrap(), "large");
        assert_eq!(
            entry.file_name().unwrap().to_string_lossy(),
            format!("{}.png", digest(&file_uri(&source)))
        );
    }

    #[test]
    fn a_stored_thumbnail_is_found_again_and_carries_its_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("picture.png");
        std::fs::write(&source, b"source bytes").unwrap();

        let stored = cache.store(&source, ThumbSize::Normal, &red(128, 96)).unwrap();
        assert!(stored.exists());

        let found = cache.lookup(&source, ThumbSize::Normal);
        assert_eq!(found.as_deref(), Some(stored.as_path()));

        let data = std::fs::read(&stored).unwrap();
        assert_eq!(
            read_text_chunk(&data, "Thumb::URI").as_deref(),
            Some(file_uri(&source).as_str())
        );
        assert!(read_text_chunk(&data, "Thumb::MTime").is_some());
        // And it is still a PNG every other reader can decode.
        let decoded = image::load_from_memory(&data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (128, 96));
    }

    #[test]
    fn a_thumbnail_of_a_file_that_has_changed_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("picture.png");
        std::fs::write(&source, b"first").unwrap();
        cache.store(&source, ThumbSize::Normal, &red(64, 64)).unwrap();
        assert!(cache.lookup(&source, ThumbSize::Normal).is_some());

        // Rewrite it with a different mtime. A stale thumbnail shows the previous contents of a
        // file with complete confidence, which is worse than showing nothing.
        std::fs::write(&source, b"second").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        filetime_set(&source, later);

        assert_eq!(cache.lookup(&source, ThumbSize::Normal), None);
    }

    #[test]
    fn a_failure_is_remembered_so_nothing_retries_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("broken.png");
        std::fs::write(&source, b"truncated").unwrap();

        assert!(!cache.known_failure(&source));
        cache.store_failure(&source).unwrap();
        assert!(cache.known_failure(&source));

        // The marker lives under the application's own directory, per the standard.
        let marker = cache.failure_marker(&source);
        assert_eq!(
            marker.parent().unwrap().file_name().unwrap(),
            "cortex-preview"
        );
    }

    #[test]
    fn a_changed_file_is_retried_even_if_it_failed_before() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("broken.png");
        std::fs::write(&source, b"truncated").unwrap();
        cache.store_failure(&source).unwrap();

        std::fs::write(&source, b"repaired").unwrap();
        filetime_set(&source, std::time::SystemTime::now() + std::time::Duration::from_secs(120));
        assert!(
            !cache.known_failure(&source),
            "a file that changed deserves another attempt"
        );
    }

    #[test]
    fn sizes_ladder_up_and_down_as_the_standard_names_them() {
        assert_eq!(ThumbSize::covering(100), ThumbSize::Normal);
        assert_eq!(ThumbSize::covering(129), ThumbSize::Large);
        assert_eq!(ThumbSize::covering(4000), ThumbSize::XxLarge);

        assert_eq!(
            ThumbSize::XLarge.smaller(),
            vec![ThumbSize::Large, ThumbSize::Normal]
        );
        assert!(ThumbSize::Normal.smaller().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn thumbnails_are_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let source = dir.path().join("private.png");
        std::fs::write(&source, b"x").unwrap();

        let stored = cache.store(&source, ThumbSize::Normal, &red(32, 32)).unwrap();
        let mode = std::fs::metadata(&stored).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a thumbnail is as revealing as its document");

        let dir_mode = std::fs::metadata(stored.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    /// Set a file's mtime. `filetime` would be a dependency for one syscall.
    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }
}
