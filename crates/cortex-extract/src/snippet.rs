//! Producing the line of text under a search result.
//!
//! Decision D16 says the index stores byte offsets and a short bounded preview, never the chunk
//! itself. Most results are answered from that preview and touch no file at all. A snippet that
//! has to show *where the query matched* is different: it re-reads the chunk from the file using
//! the stored offsets.
//!
//! ## Why re-extracting rather than seeking
//!
//! The offsets address the **extracted text**, not the bytes on disk. For plain text the two
//! differ already — a byte order mark is stripped during extraction — and for any real
//! extractor they differ completely. So the file is put through the same extraction again,
//! bounded to just past the end of the chunk, and the slice is taken from the result. Reading a
//! few kilobytes is cheaper than storing every document twice.
//!
//! ## Why the hash is checked
//!
//! A file can change between being indexed and being displayed. The offsets then address
//! different text, and showing it would put words on screen that the search never matched. The
//! chunk hash is recomputed over the slice and the snippet is refused if it disagrees, so the
//! caller falls back to the stored preview instead of inventing an excerpt.

use crate::chunk::floor_char_boundary;
use crate::classify::Handling;
use crate::extract::{self, ExtractLimits};
use std::path::Path;

/// Read a little past the chunk so that a multi-byte character straddling the end, or a byte
/// order mark shifting every offset, cannot cut the slice short.
const SLACK_BYTES: usize = 64;

/// Where a chunk lives, and what it must still hash to.
#[derive(Debug, Clone, Copy)]
pub struct Location<'a> {
    pub path: &'a Path,
    pub byte_start: usize,
    pub byte_end: usize,
    /// The hash recorded when the chunk was indexed. The snippet is refused if the text no
    /// longer matches it.
    pub hash: &'a str,
}

/// Re-read a chunk and return an excerpt centred on the first query term found in it.
///
/// Returns `None` when the file cannot be read, the offsets do not address it any more, or the
/// text has changed since indexing. Every one of those is a case for falling back to the stored
/// preview rather than for showing something wrong.
pub fn snippet(location: Location<'_>, terms: &[String], max_chars: usize) -> Option<String> {
    let text = chunk_text(location)?;
    Some(excerpt(&text, terms, max_chars))
}

/// The exact text of a chunk as it is on disk right now, or `None` if it has changed.
pub fn chunk_text(location: Location<'_>) -> Option<String> {
    if location.byte_end <= location.byte_start {
        return None;
    }

    let extracted = extract::extract(
        location.path,
        // Every text handler currently shares one reader, so the handling does not change what
        // is read. When an extractor arrives whose output is not the file's own bytes — PDF is
        // the next one — the handling will have to be stored per node and passed in here.
        Handling::PlainText,
        ExtractLimits {
            max_bytes: location.byte_end + SLACK_BYTES,
        },
    )
    .ok()?;

    let full = &extracted.text;
    if location.byte_end > full.len() {
        return None;
    }
    if !full.is_char_boundary(location.byte_start) || !full.is_char_boundary(location.byte_end) {
        return None;
    }

    let slice = &full[location.byte_start..location.byte_end];
    let hash = blake3::hash(slice.as_bytes()).to_hex().to_string();
    if hash != location.hash {
        return None;
    }
    Some(slice.to_string())
}

/// Cut a window out of `text` around the first term that occurs in it.
///
/// Public because it is also how a preview is turned into a snippet when the file is gone: the
/// preview is text, and the same windowing applies.
pub fn excerpt(text: &str, terms: &[String], max_chars: usize) -> String {
    let flat = collapse_whitespace(text);
    if flat.chars().count() <= max_chars {
        return flat;
    }

    let lower = flat.to_lowercase();
    let hit = terms
        .iter()
        .filter(|t| !t.is_empty())
        .filter_map(|t| lower.find(&t.to_lowercase()))
        .min();

    // No term found — a match on a folded form (FTS5 folds diacritics, this does not) or on a
    // different chunk of the same file. The head of the text is still the most useful thing to
    // show, and it is never wrong, only less pointed.
    let Some(hit) = hit else {
        return with_ellipsis(&flat, 0, max_chars, false);
    };

    // Leave a third of the window in front of the match so the reader sees its context rather
    // than a term hard against the left edge.
    let lead = max_chars / 3;
    let start = char_index_before(&flat, hit, lead);
    with_ellipsis(&flat, start, max_chars, start > 0)
}

/// Byte index `lead` characters before the byte index `at`.
fn char_index_before(text: &str, at: usize, lead: usize) -> usize {
    let at = floor_char_boundary(text, at);
    let mut index = at;
    for _ in 0..lead {
        let Some(previous) = text[..index].chars().next_back() else {
            break;
        };
        index -= previous.len_utf8();
    }
    // Prefer to start at a word boundary; a snippet beginning mid-word reads as corruption.
    match text[index..at].find(' ') {
        Some(offset) if offset < 24 => index + offset + 1,
        _ => index,
    }
}

fn with_ellipsis(text: &str, start: usize, max_chars: usize, lead_ellipsis: bool) -> String {
    let mut out = String::new();
    if lead_ellipsis {
        out.push('…');
    }
    let taken: String = text[start..].chars().take(max_chars).collect();
    let truncated = start + taken.len() < text.len();
    out.push_str(taken.trim_end());
    if truncated {
        out.push('…');
    }
    out
}

/// Collapse every run of whitespace to a single space.
///
/// A snippet is one line in a result list. Newlines and indentation from the source would break
/// the layout, and in code they are most of the bytes.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk;

    fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_chunk_after_a_malformed_byte_is_read_back() {
        // The file as extraction saw it: read whole, decoded leniently. Every chunk must come back
        // through the bounded read a snippet uses, including the ones past the bad byte (D131).
        let tmp = tempfile::tempdir().unwrap();
        let mut bytes = b"Kopf \xff\x00 mit einem kaputten Byte.\n\n".to_vec();
        bytes.extend("Die Blickachse bleibt konstant und die Kamera folgt ihr. ".repeat(80).as_bytes());
        let path = tmp.path().join("mixed.bin");
        std::fs::write(&path, &bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let chunks = chunk::chunk(&text, Handling::PlainText);
        assert!(chunks.len() >= 2, "the fixture must produce more than one chunk");

        for c in &chunks {
            let read_back = chunk_text(Location {
                path: &path,
                byte_start: c.byte_start,
                byte_end: c.byte_end,
                hash: &c.hash,
            });
            assert_eq!(read_back.as_deref(), Some(c.text.as_str()), "chunk at {}", c.byte_start);
        }
    }

    #[test]
    fn a_chunk_is_read_back_exactly_as_it_was_indexed() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "Die Blickachse bleibt konstant und die Kamera folgt ihr. ".repeat(40);
        let text = format!("# Kamerawinkel\n\n{body}\n\n# Budget\n\n{body}\n");
        let text = text.as_str();
        let path = write(tmp.path(), "notes.md", text);
        let chunks = chunk::chunk(text, Handling::Markup);
        assert!(chunks.len() >= 2, "the fixture must produce more than one chunk");

        for c in &chunks {
            let read_back = chunk_text(Location {
                path: &path,
                byte_start: c.byte_start,
                byte_end: c.byte_end,
                hash: &c.hash,
            });
            assert_eq!(read_back.as_deref(), Some(c.text.as_str()));
        }
    }

    #[test]
    fn offsets_survive_a_byte_order_mark() {
        // Extraction strips the BOM, so every offset is shifted three bytes from the file. A
        // snippet path that seeks into the file rather than re-extracting reads three bytes of
        // the wrong text — and only for files written by certain editors, which is the worst
        // kind of bug to find later.
        let tmp = tempfile::tempdir().unwrap();
        let text = "Übersicht über die Änderungen an der Blickachse.\n";
        let path = write(tmp.path(), "bom.txt", &format!("\u{feff}{text}"));

        let chunks = chunk::chunk(text, Handling::PlainText);
        let c = &chunks[0];
        let read_back = chunk_text(Location {
            path: &path,
            byte_start: c.byte_start,
            byte_end: c.byte_end,
            hash: &c.hash,
        });
        assert_eq!(read_back.as_deref(), Some(c.text.as_str()));
    }

    #[test]
    fn an_edited_file_refuses_to_produce_a_snippet() {
        let tmp = tempfile::tempdir().unwrap();
        let text = "the original sentence that was indexed\n";
        let path = write(tmp.path(), "edited.txt", text);
        let c = chunk::chunk(text, Handling::PlainText).remove(0);

        write(tmp.path(), "edited.txt", "something else entirely now in this file\n");
        assert_eq!(
            snippet(
                Location {
                    path: &path,
                    byte_start: c.byte_start,
                    byte_end: c.byte_end,
                    hash: &c.hash,
                },
                &["original".to_string()],
                80,
            ),
            None,
            "stale offsets must never be shown as a match"
        );
    }

    #[test]
    fn a_missing_file_is_none_rather_than_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(chunk_text(Location {
            path: &tmp.path().join("gone.txt"),
            byte_start: 0,
            byte_end: 10,
            hash: "whatever",
        })
        .is_none());
    }

    #[test]
    fn the_window_is_centred_on_the_match_not_on_the_start() {
        let text = format!("{} zielwort {}", "füllwort ".repeat(40), "nachwort ".repeat(40));
        let out = excerpt(&text, &["zielwort".to_string()], 60);
        assert!(out.contains("zielwort"), "the matched term must be visible: {out}");
        assert!(out.starts_with('…'), "a window that starts mid-text must say so: {out}");
        assert!(out.chars().count() <= 62, "window overran: {}", out.chars().count());
    }

    #[test]
    fn a_short_chunk_is_returned_whole_and_on_one_line() {
        let out = excerpt("erste Zeile\n\n\tzweite   Zeile\n", &[], 200);
        assert_eq!(out, "erste Zeile zweite Zeile");
    }

    #[test]
    fn an_unmatched_term_still_yields_the_head_of_the_text() {
        let text = "a".repeat(500);
        let out = excerpt(&text, &["nothing here".to_string()], 40);
        assert!(out.ends_with('…'));
        assert!(!out.starts_with('…'));
        assert!(out.chars().count() <= 41);
    }

    #[test]
    fn multibyte_text_is_never_cut_through_a_character() {
        // Every offset in this module is arithmetic on bytes, and German text is where that
        // arithmetic finds a character boundary that is not there.
        let text = "ä".repeat(400);
        for width in [1usize, 7, 39, 100] {
            let out = excerpt(&text, &["ä".to_string()], width);
            assert!(out.chars().all(|c| c == 'ä' || c == '…'), "corrupted at width {width}");
        }
    }
}
