//! Turning a file into text.
//!
//! Everything here runs against content the user did not write and we do not control, so the
//! guiding assumption is that any given file may be hostile or simply broken. Two consequences:
//!
//! - **Every read is bounded.** A file larger than the ceiling is never opened; a file that is
//!   within the ceiling is still truncated at a byte budget. An extractor that can be made to
//!   allocate without limit is a denial of service against the machine it runs on.
//! - **Failure is per file, never per run.** An unreadable file yields an error for that file
//!   and nothing else. An index that gives up on a whole directory because one document is
//!   malformed would be useless on any real disk.

use crate::classify::Handling;
use std::io::Read;
use std::path::Path;

/// Text extracted from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub text: String,
    /// True when the byte budget cut the file short. Recorded so a later search can say why a
    /// document only half matches, instead of appearing to have lost its second half.
    pub truncated: bool,
    /// Bytes actually consumed from the file.
    pub bytes_read: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("no extractor for this kind of file")]
    Unsupported,
}

/// Ceilings applied while reading.
#[derive(Debug, Clone, Copy)]
pub struct ExtractLimits {
    /// Most bytes to read from one file.
    pub max_bytes: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            // Two megabytes of text is a very long document — a few hundred pages. Beyond that,
            // the tail of the file contributes almost nothing to whether it is the right file,
            // which is the question this index answers.
            max_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Extract text from a file.
pub fn extract(path: &Path, handling: Handling, limits: ExtractLimits) -> Result<Extracted, ExtractError> {
    if !handling.has_text() {
        return Err(ExtractError::Unsupported);
    }
    read_text(path, limits)
}

/// Read a text file, bounded, decoding leniently.
///
/// Decoding is lossy on purpose. A document with one malformed byte in the middle is still
/// worth finding, and refusing it would mean the index quietly disagrees with what the user
/// can plainly see in their editor.
fn read_text(path: &Path, limits: ExtractLimits) -> Result<Extracted, ExtractError> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = Vec::new();
    // `take` bounds the read itself rather than checking the size afterwards, so a file that
    // grows while being read cannot exceed the budget either.
    let bytes_read = file
        .by_ref()
        .take(limits.max_bytes as u64 + 1)
        .read_to_end(&mut buffer)?;

    let truncated = bytes_read > limits.max_bytes;
    if truncated {
        buffer.truncate(limits.max_bytes);
        // Do not cut a character in half; the tail is worth less than the corruption costs.
        while !buffer.is_empty() && std::str::from_utf8(&buffer).is_err() {
            buffer.pop();
        }
    }

    let text = strip_bom(String::from_utf8_lossy(&buffer).into_owned());

    Ok(Extracted {
        bytes_read: buffer.len(),
        text,
        truncated,
    })
}

/// Remove a UTF-8 byte order mark.
///
/// Left in place it becomes the first character of the first chunk, where it silently defeats
/// a heading match on the first line of every file written by certain editors.
fn strip_bom(text: String) -> String {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(name);
        std::fs::write(&path, contents).unwrap();
        (tmp, path)
    }

    #[test]
    fn plain_text_comes_back_intact() {
        let (_tmp, path) = write("a.txt", b"hello\nworld\n");
        let out = extract(&path, Handling::PlainText, ExtractLimits::default()).unwrap();
        assert_eq!(out.text, "hello\nworld\n");
        assert!(!out.truncated);
    }

    #[test]
    fn german_text_survives_a_round_trip() {
        let (_tmp, path) = write("de.md", "# Übersicht\n\nGrößere Änderungen\n".as_bytes());
        let out = extract(&path, Handling::Markup, ExtractLimits::default()).unwrap();
        assert!(out.text.contains("Übersicht"));
        assert!(out.text.contains("Größere"));
    }

    #[test]
    fn a_byte_order_mark_is_removed() {
        // Left in place it becomes the first character of the document and quietly breaks a
        // heading match on the first line.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"# Heading\n");
        let (_tmp, path) = write("bom.md", &bytes);

        let out = extract(&path, Handling::Markup, ExtractLimits::default()).unwrap();
        assert!(out.text.starts_with("# Heading"), "got {:?}", &out.text[..10.min(out.text.len())]);
    }

    #[test]
    fn a_large_file_is_truncated_and_says_so() {
        let (_tmp, path) = write("big.txt", &vec![b'a'; 10_000]);
        let out = extract(&path, Handling::PlainText, ExtractLimits { max_bytes: 1000 }).unwrap();

        assert!(out.truncated, "truncation must be reported, not silent");
        assert_eq!(out.text.len(), 1000);
    }

    #[test]
    fn truncation_never_cuts_a_character_in_half() {
        // Every character here is two bytes, so a budget that lands mid-character exercises
        // the backtracking. A half character would corrupt the text the index stores.
        let content = "ä".repeat(1000);
        let (_tmp, path) = write("umlauts.txt", content.as_bytes());

        let out = extract(&path, Handling::PlainText, ExtractLimits { max_bytes: 1001 }).unwrap();
        assert!(out.truncated);
        assert!(out.text.chars().all(|c| c == 'ä'), "truncation corrupted the text");
    }

    #[test]
    fn a_file_exactly_at_the_budget_is_not_marked_truncated() {
        let (_tmp, path) = write("exact.txt", &vec![b'a'; 1000]);
        let out = extract(&path, Handling::PlainText, ExtractLimits { max_bytes: 1000 }).unwrap();
        assert!(!out.truncated);
        assert_eq!(out.text.len(), 1000);
    }

    #[test]
    fn invalid_bytes_are_replaced_rather_than_refused() {
        // One bad byte must not cost the whole document. The user can see the file in their
        // editor; an index that disagrees looks broken.
        let (_tmp, path) = write("mixed.txt", b"good text \xFF\xFE more good text");
        let out = extract(&path, Handling::PlainText, ExtractLimits::default()).unwrap();
        assert!(out.text.contains("good text"));
        assert!(out.text.contains("more good text"));
    }

    #[test]
    fn an_empty_file_yields_empty_text() {
        let (_tmp, path) = write("empty.txt", b"");
        let out = extract(&path, Handling::PlainText, ExtractLimits::default()).unwrap();
        assert_eq!(out.text, "");
        assert!(!out.truncated);
    }

    #[test]
    fn a_handling_with_no_text_is_refused_rather_than_read() {
        let (_tmp, path) = write("image.jpg", b"\xFF\xD8\xFF\xE0");
        for handling in [Handling::Image, Handling::Pdf, Handling::MetadataOnly] {
            assert!(
                matches!(
                    extract(&path, handling, ExtractLimits::default()),
                    Err(ExtractError::Unsupported)
                ),
                "{handling:?} must not be read as text"
            );
        }
    }

    #[test]
    fn a_missing_file_is_an_error_for_that_file_only() {
        let err = extract(
            Path::new("/definitely/not/here.txt"),
            Handling::PlainText,
            ExtractLimits::default(),
        );
        assert!(matches!(err, Err(ExtractError::Io(_))));
    }
}
