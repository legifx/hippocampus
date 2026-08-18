//! One file in, one decision out.
//!
//! `classify`, `extract` and `chunk` are deliberately independent and individually testable.
//! This module is the only place that knows the order they run in, so the ingest loop can ask
//! a single question — "what should I record about this file?" — without also owning the rules.
//!
//! ## The short circuit that pays for the whole stage
//!
//! Before anything is chunked, the extracted text is hashed and compared with what the index
//! already holds. A file that was rewritten with identical content — a save with no edit, a
//! `git checkout`, an rsync that copied everything again — stops here. That is not a rare case;
//! it is most of what a filesystem does all day.
//!
//! The hash covers the *extracted* text, not the file on disk. Two consequences, both intended:
//! a metadata-only change (permissions, mtime) produces the same hash and no work, and a change
//! beyond the extraction byte budget produces the same hash as well — correctly, because
//! nothing past that budget is indexed, so nothing about the index would change.

use crate::chunk::{self, Chunk};
use crate::classify::{self, Classification, Handling, SkipReason};
use crate::extract::{self, ExtractLimits};
use std::path::{Path, PathBuf};

/// Everything the pipeline needs that is not the file itself.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Rule R3: content is only read inside these roots.
    pub content_roots: Vec<PathBuf>,
    /// Rule R1: extra deny-list patterns on top of the built-in minimum.
    pub extra_deny: Vec<String>,
    pub classify_limits: classify::Limits,
    pub extract_limits: ExtractLimits,
    /// Characters kept per chunk for display without touching the file again.
    pub preview_chars: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            content_roots: Vec::new(),
            extra_deny: Vec::new(),
            classify_limits: classify::Limits::default(),
            extract_limits: ExtractLimits::default(),
            // Enough for a result row in a launcher without becoming a copy of the document.
            preview_chars: 240,
        }
    }
}

/// What the pipeline decided about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Text was read and split. The chunks replace whatever the index holds for this file.
    Extracted {
        content_hash: String,
        mime: Option<String>,
        chunks: Vec<Chunk>,
        /// True when the byte budget cut the file short, so a search can explain why a long
        /// document only half matches.
        truncated: bool,
    },
    /// The content is byte-identical to what was extracted last time. Nothing to write.
    Unchanged { content_hash: String },
    /// Deliberately not read. A normal outcome, and not one to retry until the file changes.
    Skipped {
        reason: SkipReason,
        mime: Option<String>,
    },
    /// Reading was attempted and failed. Worth retrying when the file changes; worth reporting
    /// when it does not.
    Failed { note: String },
}

impl Outcome {
    /// A short human-readable reason, for `nodes.extract_note` and for `cortex explain`.
    pub fn note(&self) -> Option<String> {
        match self {
            Outcome::Extracted { truncated: true, .. } => {
                Some("truncated at the extraction byte budget".to_string())
            }
            Outcome::Extracted { .. } | Outcome::Unchanged { .. } => None,
            Outcome::Skipped { reason, .. } => Some(describe(*reason).to_string()),
            Outcome::Failed { note } => Some(note.clone()),
        }
    }
}

fn describe(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::Denied => "on the deny-list (rule R1)",
        SkipReason::OutsideContentRoots => "outside every content root (rule R3)",
        SkipReason::Binary => "binary, no text to extract",
        SkipReason::TooLarge => "larger than the extraction ceiling",
        SkipReason::Unreadable => "could not be opened",
    }
}

/// Run the whole pipeline over one file.
///
/// `previous_hash` is what the index already believes about this file's content; passing it is
/// what makes a re-index cheap.
pub fn process(
    path: &Path,
    size: u64,
    previous_hash: Option<&str>,
    settings: &Settings,
) -> Outcome {
    let Classification {
        handling,
        mime,
        skipped_because,
    } = classify::classify(
        path,
        size,
        &settings.content_roots,
        &settings.extra_deny,
        settings.classify_limits,
    );

    if let Some(reason) = skipped_because {
        return Outcome::Skipped { reason, mime };
    }

    // A recognised format with no text extractor yet — PDFs and images today. It is recorded
    // and skipped rather than failed: nothing went wrong, the capability is simply not built.
    if !handling.has_text() {
        return Outcome::Skipped {
            reason: SkipReason::Binary,
            mime,
        };
    }

    let extracted = match extract::extract(path, handling, settings.extract_limits) {
        Ok(extracted) => extracted,
        Err(e) => {
            return Outcome::Failed {
                note: e.to_string(),
            }
        }
    };

    let content_hash = blake3::hash(extracted.text.as_bytes()).to_hex().to_string();
    if previous_hash == Some(content_hash.as_str()) {
        return Outcome::Unchanged { content_hash };
    }

    Outcome::Extracted {
        chunks: chunk::chunk(&extracted.text, handling),
        content_hash,
        mime,
        truncated: extracted.truncated,
    }
}

/// The handling a file would get, without reading it. Used by `cortex explain`.
pub fn handling_of(path: &Path, size: u64, settings: &Settings) -> (Handling, Option<SkipReason>) {
    let c = classify::classify(
        path,
        size,
        &settings.content_roots,
        &settings.extra_deny,
        settings.classify_limits,
    );
    (c.handling, c.skipped_because)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_rooted_at(root: &Path) -> Settings {
        Settings {
            content_roots: vec![root.to_path_buf()],
            ..Settings::default()
        }
    }

    fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn size_of(path: &Path) -> u64 {
        std::fs::metadata(path).unwrap().len()
    }

    #[test]
    fn a_markdown_file_is_extracted_and_split_at_its_headings() {
        // Sections have to be long enough to survive assembly: the chunker packs short sections
        // together on purpose, because a three-line chunk matches everything and means nothing.
        let tmp = tempfile::tempdir().unwrap();
        let body = "Die Blickachse bleibt konstant und die Kamera folgt ihr. ".repeat(40);
        let document = format!("# Kamerawinkel\n\n{body}\n\n# Budget\n\n{body}\n");
        let path = write(tmp.path(), "notes.md", document.as_bytes());

        match process(&path, size_of(&path), None, &settings_rooted_at(tmp.path())) {
            Outcome::Extracted { chunks, content_hash, .. } => {
                assert!(chunks.len() >= 2, "expected a split at the heading, got {}", chunks.len());
                // The split is asserted on the boundary rather than on the chunk text, because
                // every chunk carries an overlap from its predecessor and therefore starts
                // slightly before its own section.
                let heading = document.find("# Budget").unwrap();
                assert!(
                    chunks.iter().any(|c| c.byte_end == heading),
                    "the heading must end a chunk; got boundaries {:?}",
                    chunks.iter().map(|c| c.byte_end).collect::<Vec<_>>()
                );
                assert_eq!(content_hash.len(), 64, "BLAKE3 hex is 64 characters");
            }
            other => panic!("expected extraction, got {other:?}"),
        }
    }

    #[test]
    fn an_unchanged_file_stops_before_chunking() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "stable.txt", b"nothing here ever changes\n");
        let settings = settings_rooted_at(tmp.path());

        let first = process(&path, size_of(&path), None, &settings);
        let Outcome::Extracted { content_hash, .. } = first else {
            panic!("expected extraction");
        };

        let again = process(&path, size_of(&path), Some(&content_hash), &settings);
        assert_eq!(again, Outcome::Unchanged { content_hash });
    }

    #[test]
    fn touching_a_file_without_editing_it_produces_the_same_hash() {
        // The case the short circuit exists for: an editor that rewrites on every save, a
        // checkout that restores identical bytes. Nothing about the index should change.
        let tmp = tempfile::tempdir().unwrap();
        let settings = settings_rooted_at(tmp.path());
        let path = write(tmp.path(), "resaved.txt", b"same text\n");
        let Outcome::Extracted { content_hash: first, .. } =
            process(&path, size_of(&path), None, &settings)
        else {
            panic!("expected extraction")
        };

        std::fs::write(&path, b"same text\n").unwrap();
        let Outcome::Extracted { content_hash: second, .. } =
            process(&path, size_of(&path), None, &settings)
        else {
            panic!("expected extraction")
        };
        assert_eq!(first, second);
    }

    #[test]
    fn a_file_outside_the_content_roots_is_skipped_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "private.txt", b"metadata only\n");
        // No content roots at all: the default, and rule R3's whole point.
        let outcome = process(&path, size_of(&path), None, &Settings::default());
        assert!(matches!(
            outcome,
            Outcome::Skipped { reason: SkipReason::OutsideContentRoots, .. }
        ));
        assert_eq!(
            outcome.note().as_deref(),
            Some("outside every content root (rule R3)")
        );
    }

    #[test]
    fn a_denied_file_inside_a_content_root_is_still_denied() {
        // Rule R1 outranks rule R3. Naming a directory as a content root must not make its
        // secrets readable — the deny-list is a floor, not a default.
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), ".ssh/id_ed25519", b"not a key, but named like one\n");
        let outcome = process(&path, size_of(&path), None, &settings_rooted_at(tmp.path()));
        assert!(matches!(
            outcome,
            Outcome::Skipped { reason: SkipReason::Denied, .. }
        ));
    }

    #[test]
    fn an_oversized_file_is_never_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "huge.log", b"x");
        let settings = Settings {
            classify_limits: classify::Limits { max_file_bytes: 4 },
            ..settings_rooted_at(tmp.path())
        };
        // The size is what the caller passes, so a stale stat cannot be used to smuggle a large
        // file past the ceiling — the ceiling is re-checked against the read budget anyway.
        let outcome = process(&path, 1024 * 1024, None, &settings);
        assert!(matches!(
            outcome,
            Outcome::Skipped { reason: SkipReason::TooLarge, .. }
        ));
    }

    #[test]
    fn a_missing_file_fails_for_itself_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gone.txt");
        let outcome = process(&path, 10, None, &settings_rooted_at(tmp.path()));
        // Classification cannot read the head either, so this surfaces as a skip with a
        // reason rather than a failure. Either way it is one file's problem.
        assert!(matches!(
            outcome,
            Outcome::Skipped { reason: SkipReason::Unreadable, .. }
        ));
    }

    #[test]
    fn a_binary_file_is_recorded_and_skipped_even_with_a_text_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.extend_from_slice(&[0u8; 64]);
        let path = write(tmp.path(), "photo.txt", &jpeg);

        let outcome = process(&path, size_of(&path), None, &settings_rooted_at(tmp.path()));
        assert!(matches!(
            outcome,
            Outcome::Skipped { reason: SkipReason::Binary, .. }
        ));
    }

    #[test]
    fn an_empty_file_extracts_to_no_chunks_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "empty.md", b"");
        match process(&path, 0, None, &settings_rooted_at(tmp.path())) {
            Outcome::Extracted { chunks, .. } => assert!(chunks.is_empty()),
            other => panic!("expected an empty extraction, got {other:?}"),
        }
    }

    #[test]
    fn truncation_is_reported_in_the_note() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "long.txt", "wort ".repeat(2000).as_bytes());
        let settings = Settings {
            extract_limits: ExtractLimits { max_bytes: 100 },
            ..settings_rooted_at(tmp.path())
        };
        let outcome = process(&path, size_of(&path), None, &settings);
        assert!(matches!(outcome, Outcome::Extracted { truncated: true, .. }));
        assert_eq!(
            outcome.note().as_deref(),
            Some("truncated at the extraction byte budget")
        );
    }
}
