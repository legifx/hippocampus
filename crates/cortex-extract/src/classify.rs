//! Deciding what a file is, and what may be done with it.
//!
//! ## Why magic bytes, and where that argument stops
//!
//! Type detection by file extension is wrong in both directions: a `.txt` holding a JPEG, and a
//! source file with no extension at all, are both ordinary. So binary formats are identified by
//! their magic number.
//!
//! But **plain text has no magic number.** There is no byte sequence that means "this is a
//! Markdown document". So the rule is narrower than the slogan:
//!
//! 1. The deny-list decides whether the file may be opened at all (rule R1). Nothing below
//!    happens until it has said yes.
//! 2. A magic number, if present, identifies the format authoritatively.
//! 3. Otherwise the content is sniffed: valid UTF-8, no NUL bytes, few control characters.
//!    That is what makes something text.
//! 4. Only *then* is the extension consulted, and only to pick which text handler to use.
//!    It never decides whether a file is readable, and it can never override a magic number.

use cortex_core::denylist::{self, Verdict};
use std::path::Path;

/// How many bytes are enough to tell what a file is.
///
/// Magic numbers live in the first few dozen bytes; the rest of this window is for the text
/// sniff, which needs enough to be confident but must not read a gigabyte to classify it.
pub const SNIFF_BYTES: usize = 8192;

/// What the ingest pipeline should do with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handling {
    /// Record path and stat. Never open it.
    MetadataOnly,
    /// Plain text with no further structure.
    PlainText,
    /// Markdown or a similar heading-structured document.
    Markup,
    /// Source code. Chunked along definition boundaries.
    Code,
    /// A PDF. Extraction lands in a later step; for now it is recorded and skipped.
    Pdf,
    /// An image. EXIF only; OCR is opt-in and not implemented yet.
    Image,
}

impl Handling {
    /// Whether this file has extractable text at all.
    pub fn has_text(self) -> bool {
        matches!(self, Handling::PlainText | Handling::Markup | Handling::Code)
    }
}

/// The result of classifying a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub handling: Handling,
    /// The MIME type, when one could be determined.
    pub mime: Option<String>,
    /// Why this file will not be read, when it will not be.
    pub skipped_because: Option<SkipReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The deny-list forbids reading it (rule R1).
    Denied,
    /// It is outside every configured content root (rule R3).
    OutsideContentRoots,
    /// It is a known binary format with no text in it.
    Binary,
    /// It is larger than the configured ceiling.
    TooLarge,
    /// It could not be read.
    Unreadable,
}

/// Limits applied before anything expensive happens.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Files above this size are recorded but never opened.
    ///
    /// A ceiling is not an optimisation. Without one, a single multi-gigabyte log file can
    /// consume the entire extraction budget and stall every other file behind it.
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 32 * 1024 * 1024,
        }
    }
}

/// Classify a file.
///
/// `content_roots` implements rule R3: metadata indexing may cover a whole home directory, but
/// content is only read inside roots the user explicitly named.
pub fn classify(
    path: &Path,
    size: u64,
    content_roots: &[std::path::PathBuf],
    extra_deny: &[String],
    limits: Limits,
) -> Classification {
    let metadata_only = |reason: SkipReason| Classification {
        handling: Handling::MetadataOnly,
        mime: None,
        skipped_because: Some(reason),
    };

    // Rule R1, before anything opens the file.
    if denylist::classify(path, extra_deny) != Verdict::Allow {
        return metadata_only(SkipReason::Denied);
    }

    // Rule R3.
    if !content_roots.iter().any(|root| path.starts_with(root)) {
        return metadata_only(SkipReason::OutsideContentRoots);
    }

    if size > limits.max_file_bytes {
        return metadata_only(SkipReason::TooLarge);
    }

    let Ok(head) = read_head(path) else {
        return metadata_only(SkipReason::Unreadable);
    };

    classify_bytes(path, &head)
}

/// Classify from a byte prefix. Split out so the rules are testable without a filesystem.
pub fn classify_bytes(path: &Path, head: &[u8]) -> Classification {
    // An empty file is text with nothing in it, which is harmless and cheap to record.
    if head.is_empty() {
        return Classification {
            handling: Handling::PlainText,
            mime: Some("text/plain".into()),
            skipped_because: None,
        };
    }

    // A magic number is authoritative and outranks the extension.
    if let Some(kind) = infer::get(head) {
        let mime = kind.mime_type().to_string();
        let handling = match mime.as_str() {
            "application/pdf" => Handling::Pdf,
            m if m.starts_with("image/") => Handling::Image,
            _ => Handling::MetadataOnly,
        };
        return Classification {
            handling,
            mime: Some(mime),
            skipped_because: (handling == Handling::MetadataOnly).then_some(SkipReason::Binary),
        };
    }

    // No magic number. Is it text at all?
    if !looks_like_text(head) {
        return Classification {
            handling: Handling::MetadataOnly,
            mime: None,
            skipped_because: Some(SkipReason::Binary),
        };
    }

    // Only now does the extension get a say, and only to choose among text handlers.
    let (handling, mime) = text_handler(path);
    Classification {
        handling,
        mime: Some(mime.to_string()),
        skipped_because: None,
    }
}

/// Whether a byte prefix looks like human-readable text.
///
/// Three signals, in order of how decisive they are: a NUL byte means binary, full stop;
/// invalid UTF-8 that is not merely a truncated character at the window edge means binary;
/// and a high proportion of control characters means binary even when the bytes decode.
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }

    match std::str::from_utf8(head) {
        Ok(_) => {}
        Err(e) => {
            // The window may have cut a multi-byte character in half. That is an artefact of
            // where we stopped reading, not evidence about the file.
            let truncated_at_edge = e.error_len().is_none() && e.valid_up_to() + 4 >= head.len();
            if !truncated_at_edge {
                return false;
            }
        }
    }

    let control = head
        .iter()
        .filter(|b| **b < 0x20 && !matches!(**b, b'\n' | b'\r' | b'\t' | 0x0c))
        .count();
    control * 100 / head.len().max(1) < 5
}

/// Which text handler an extension implies. Never consulted for binary detection.
fn text_handler(path: &Path) -> (Handling, &'static str) {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" | "mdx" => (Handling::Markup, "text/markdown"),
        "rst" => (Handling::Markup, "text/x-rst"),
        "org" => (Handling::Markup, "text/x-org"),
        "adoc" | "asciidoc" => (Handling::Markup, "text/asciidoc"),

        "rs" => (Handling::Code, "text/rust"),
        "py" | "pyi" => (Handling::Code, "text/x-python"),
        "js" | "mjs" | "cjs" | "jsx" => (Handling::Code, "text/javascript"),
        "ts" | "tsx" => (Handling::Code, "text/typescript"),
        "go" => (Handling::Code, "text/x-go"),
        "c" | "h" => (Handling::Code, "text/x-c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => (Handling::Code, "text/x-c++"),
        "java" => (Handling::Code, "text/x-java"),
        "rb" => (Handling::Code, "text/x-ruby"),
        "sh" | "bash" | "zsh" => (Handling::Code, "text/x-shellscript"),
        "toml" => (Handling::Code, "text/toml"),
        "yaml" | "yml" => (Handling::Code, "text/yaml"),
        "json" => (Handling::Code, "application/json"),
        "sql" => (Handling::Code, "text/x-sql"),
        "css" | "scss" => (Handling::Code, "text/css"),
        "html" | "htm" | "xml" => (Handling::Code, "text/html"),

        _ => (Handling::PlainText, "text/plain"),
    }
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; SNIFF_BYTES];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn of(name: &str, bytes: &[u8]) -> Classification {
        classify_bytes(Path::new(name), bytes)
    }

    #[test]
    fn markdown_is_recognised_as_markup() {
        let c = of("/x/notes.md", b"# Heading\n\nSome prose.\n");
        assert_eq!(c.handling, Handling::Markup);
        assert_eq!(c.mime.as_deref(), Some("text/markdown"));
    }

    #[test]
    fn source_files_are_recognised_as_code() {
        for (name, mime) in [
            ("/x/main.rs", "text/rust"),
            ("/x/app.py", "text/x-python"),
            ("/x/index.ts", "text/typescript"),
        ] {
            let c = of(name, b"fn main() {}\n");
            assert_eq!(c.handling, Handling::Code, "{name}");
            assert_eq!(c.mime.as_deref(), Some(mime));
        }
    }

    #[test]
    fn an_unknown_extension_holding_text_is_still_text() {
        // The point of sniffing: a file with no extension, or a strange one, is readable if
        // its contents are readable.
        let c = of("/x/README", b"just some words\n");
        assert_eq!(c.handling, Handling::PlainText);

        let c = of("/x/notes.qqq", b"just some words\n");
        assert_eq!(c.handling, Handling::PlainText);
    }

    #[test]
    fn a_magic_number_outranks_the_extension() {
        // A JPEG named .txt is a JPEG. This is the case extension-based detection gets wrong.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
        let c = of("/x/definitely-not-an-image.txt", &jpeg);
        assert_eq!(c.handling, Handling::Image);
        assert_eq!(c.mime.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn a_pdf_is_recognised_by_its_magic_number() {
        let c = of("/x/report.pdf", b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        assert_eq!(c.handling, Handling::Pdf);
        assert_eq!(c.mime.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn binary_data_without_a_known_magic_number_is_not_read() {
        let mut bytes = vec![0x00, 0x01, 0x02, 0x03];
        bytes.extend_from_slice(&[0xFF; 100]);
        let c = of("/x/blob.dat", &bytes);
        assert_eq!(c.handling, Handling::MetadataOnly);
        assert_eq!(c.skipped_because, Some(SkipReason::Binary));
    }

    #[test]
    fn a_nul_byte_means_binary_however_text_like_the_rest_looks() {
        let mut bytes = b"this looks like text for a while".to_vec();
        bytes.push(0);
        bytes.extend_from_slice(b"and then continues");
        assert_eq!(of("/x/thing.txt", &bytes).handling, Handling::MetadataOnly);
    }

    #[test]
    fn a_character_cut_in_half_by_the_sniff_window_is_not_binary() {
        // The window ends mid-character. That is a fact about where we stopped reading, not
        // about the file, and treating it as binary would silently drop UTF-8 documents.
        let mut bytes = "Gr".repeat(10).into_bytes();
        bytes.extend_from_slice(&"ö".as_bytes()[..1]);
        assert_eq!(of("/x/text.txt", &bytes).handling, Handling::PlainText);
    }

    #[test]
    fn genuinely_invalid_utf8_early_on_is_binary() {
        let mut bytes = vec![0xC3, 0x28];
        bytes.extend_from_slice(&[b'a'; 200]);
        assert_eq!(of("/x/text.txt", &bytes).handling, Handling::MetadataOnly);
    }

    #[test]
    fn german_text_survives_classification() {
        let c = of("/x/bericht.md", "# Übersicht\n\nGrößere Änderungen.\n".as_bytes());
        assert_eq!(c.handling, Handling::Markup);
    }

    #[test]
    fn an_empty_file_is_harmless_text() {
        let c = of("/x/empty.txt", b"");
        assert_eq!(c.handling, Handling::PlainText);
        assert!(c.skipped_because.is_none());
    }

    #[test]
    fn a_denied_path_is_never_opened_whatever_it_contains() {
        // Rule R1 comes before everything, including the size and root checks.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("id_ed25519");
        std::fs::write(&path, b"NOT-A-REAL-KEY").unwrap();

        let c = classify(&path, 14, &[tmp.path().to_path_buf()], &[], Limits::default());
        assert_eq!(c.handling, Handling::MetadataOnly);
        assert_eq!(c.skipped_because, Some(SkipReason::Denied));
    }

    #[test]
    fn a_file_outside_every_content_root_is_not_read() {
        // Rule R3: metadata indexing may span a home directory; content extraction may not.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notes.md");
        std::fs::write(&path, b"# hello").unwrap();

        let c = classify(&path, 7, &[PathBuf::from("/somewhere/else")], &[], Limits::default());
        assert_eq!(c.handling, Handling::MetadataOnly);
        assert_eq!(c.skipped_because, Some(SkipReason::OutsideContentRoots));

        let c = classify(&path, 7, &[tmp.path().to_path_buf()], &[], Limits::default());
        assert_eq!(c.handling, Handling::Markup);
    }

    #[test]
    fn an_oversized_file_is_recorded_but_not_read() {
        // One enormous log file must not be able to consume the whole extraction budget.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("huge.log");
        std::fs::write(&path, b"x").unwrap();

        let limits = Limits { max_file_bytes: 10 };
        let c = classify(&path, 5_000_000, &[tmp.path().to_path_buf()], &[], limits);
        assert_eq!(c.handling, Handling::MetadataOnly);
        assert_eq!(c.skipped_because, Some(SkipReason::TooLarge));
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_panicking() {
        let c = classify(
            Path::new("/definitely/not/here.txt"),
            10,
            &[PathBuf::from("/definitely")],
            &[],
            Limits::default(),
        );
        assert_eq!(c.skipped_because, Some(SkipReason::Unreadable));
    }

    #[test]
    fn only_text_handlings_claim_to_have_text() {
        assert!(Handling::PlainText.has_text());
        assert!(Handling::Markup.has_text());
        assert!(Handling::Code.has_text());
        assert!(!Handling::Pdf.has_text(), "PDF extraction is not implemented yet");
        assert!(!Handling::Image.has_text());
        assert!(!Handling::MetadataOnly.has_text());
    }
}
