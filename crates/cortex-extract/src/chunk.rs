//! Splitting text into retrievable units.
//!
//! This is pattern separation in the hippocampal sense: making similar things distinguishable.
//! Two properties matter more than the exact boundaries.
//!
//! **Structure-aware, not naive.** Cutting every 500 characters puts the answer to a question
//! half in one chunk and half in the next, and retrieval then finds neither. Markdown splits at
//! headings, code at top-level definitions, prose at paragraphs.
//!
//! **Hashed per chunk.** Each chunk carries a hash of its own text, so an unchanged chunk is
//! never re-embedded. Editing one paragraph of a long document then costs one embedding call
//! rather than hundreds. The build prompt calls this the largest single performance lever in
//! the whole ingest pipeline, and it is: everything else in extraction is linear in bytes,
//! while embedding is orders of magnitude more expensive per byte.

use crate::classify::Handling;

/// Target chunk size in characters.
///
/// The specification is given in tokens (300–600). No tokenizer exists in the pipeline yet, and
/// introducing one purely to measure chunk sizes would tie chunking to a model choice that has
/// not been made. Characters are the honest proxy: roughly 3.5–4 characters per token for the
/// German/English mix this corpus contains, so the window below lands inside the intended range.
/// When the embedder arrives with its own tokenizer, this becomes a token count and the
/// boundaries shift slightly — which is fine, because a chunk-size change is a re-index, and
/// re-indexing is a supported operation rather than a migration.
pub const TARGET_CHARS: usize = 1600;

/// Never emit a chunk longer than this, whatever the structure says.
pub const MAX_CHARS: usize = 3200;

/// Below this, a trailing fragment is folded into the previous chunk rather than emitted alone.
/// A three-word chunk matches everything and means nothing.
pub const MIN_CHARS: usize = 120;

/// Fraction of a chunk repeated at the start of the next.
///
/// Overlap is what stops a boundary from destroying a fact that straddles it. It costs storage
/// and duplicate matches; 15 % is the point where the specification puts it and there is no
/// measurement here to justify deviating.
pub const OVERLAP_FRACTION: f32 = 0.15;

/// One unit of retrievable text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub ordinal: usize,
    /// Byte offsets into the extracted text, so a snippet can be re-read from the file rather
    /// than stored (decision D16).
    pub byte_start: usize,
    pub byte_end: usize,
    /// BLAKE3 of the chunk text, hex encoded.
    pub hash: String,
    pub text: String,
}

impl Chunk {
    /// A short excerpt for result display, so the common case needs no file access.
    ///
    /// Bounded, and cut at a character boundary. This is an excerpt, never the chunk — storing
    /// the chunk would make the index a copy of the user's files, which decision D16 forbids.
    pub fn preview(&self, max_chars: usize) -> String {
        let mut out: String = self.text.chars().take(max_chars).collect();
        if out.len() < self.text.len() {
            out.push('…');
        }
        out.replace('\n', " ").trim().to_string()
    }
}

/// Split extracted text into chunks.
pub fn chunk(text: &str, handling: Handling) -> Vec<Chunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let sections = match handling {
        Handling::Markup => split_markdown(text),
        Handling::Code => split_code(text),
        _ => split_paragraphs(text),
    };

    assemble(text, sections)
}

/// A structural boundary: a byte offset in the source text where a new section starts.
fn split_markdown(text: &str) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut offset = 0;
    let mut in_fence = false;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();

        // Headings inside a fenced code block are shell comments, not headings.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && trimmed.starts_with('#') && offset > 0 {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            // A heading is only a heading when a space follows the hashes; `#hashtag` is not.
            if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
                boundaries.push(offset);
            }
        }
        offset += line.len();
    }

    boundaries
}

/// Boundaries at top-level definitions.
///
/// Deliberately heuristic rather than parsed. A tree-sitter grammar per language is the correct
/// answer and is planned, but it is one dependency per language and this heuristic already
/// captures the property that matters: a definition should not be split from its signature.
/// The rule — a non-indented line that introduces something — holds across the brace and
/// indentation families alike.
fn split_code(text: &str) -> Vec<usize> {
    const INTRODUCERS: &[&str] = &[
        "fn ", "pub fn", "pub async fn", "async fn", "impl", "struct", "enum", "trait", "mod ",
        "def ", "class ", "async def", "function ", "export ", "const ", "type ", "interface ",
        "func ", "package ", "public ", "private ", "protected ", "static ",
    ];

    let mut boundaries = vec![0];
    let mut offset = 0;

    for line in text.split_inclusive('\n') {
        let is_top_level = !line.starts_with(' ') && !line.starts_with('\t');
        if is_top_level && offset > 0 {
            let trimmed = line.trim_start();
            if INTRODUCERS.iter().any(|k| trimmed.starts_with(k)) {
                boundaries.push(offset);
            }
        }
        offset += line.len();
    }

    boundaries
}

/// Boundaries at blank lines.
fn split_paragraphs(text: &str) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut offset = 0;
    let mut previous_blank = false;

    for line in text.split_inclusive('\n') {
        let blank = line.trim().is_empty();
        if previous_blank && !blank && offset > 0 {
            boundaries.push(offset);
        }
        previous_blank = blank;
        offset += line.len();
    }

    boundaries
}

/// Turn structural boundaries into chunks of a workable size.
///
/// Structure proposes; size disposes. A section far below the target is merged with its
/// neighbours, and one far above it is split, because a chunk that is either tiny or enormous
/// retrieves badly regardless of how well it matches the document's own structure.
fn assemble(text: &str, boundaries: Vec<usize>) -> Vec<Chunk> {
    let mut sections: Vec<(usize, usize)> = Vec::new();
    for (i, start) in boundaries.iter().enumerate() {
        let end = boundaries.get(i + 1).copied().unwrap_or(text.len());
        if end > *start {
            sections.push((*start, end));
        }
    }

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut pending: Option<(usize, usize)> = None;

    let flush = |start: usize, end: usize, chunks: &mut Vec<Chunk>| {
        for (piece_start, piece_end) in hard_split(text, start, end) {
            let body = &text[piece_start..piece_end];
            if body.trim().is_empty() {
                continue;
            }
            let with_overlap_start = overlap_start(text, chunks.last(), piece_start);
            let body = &text[with_overlap_start..piece_end];

            chunks.push(Chunk {
                ordinal: chunks.len(),
                byte_start: with_overlap_start,
                byte_end: piece_end,
                hash: blake3::hash(body.as_bytes()).to_hex().to_string(),
                text: body.to_string(),
            });
        }
    };

    for (start, end) in sections {
        let (start, end) = match pending.take() {
            Some((pending_start, _)) => (pending_start, end),
            None => (start, end),
        };

        if end - start < MIN_CHARS {
            // Too small to stand alone: carry it forward and merge with what follows.
            pending = Some((start, end));
            continue;
        }
        flush(start, end, &mut chunks);
    }

    if let Some((start, end)) = pending {
        match chunks.last_mut() {
            // A trailing fragment joins the previous chunk rather than becoming a chunk of
            // three words that matches everything.
            Some(last) if end - start < MIN_CHARS && last.byte_end == start => {
                last.byte_end = end;
                last.text = text[last.byte_start..end].to_string();
                last.hash = blake3::hash(last.text.as_bytes()).to_hex().to_string();
            }
            _ => flush(start, end, &mut chunks),
        }
    }

    chunks
}

/// Split a span that exceeds `MAX_CHARS`, preferring line boundaries.
fn hard_split(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    if end - start <= MAX_CHARS {
        return vec![(start, end)];
    }

    let mut pieces = Vec::new();
    let mut cursor = start;

    while end - cursor > MAX_CHARS {
        // The search window itself has to end on a character boundary. Slicing a string at an
        // arbitrary byte offset panics, and the offset here is arithmetic on a byte budget with
        // no knowledge of what characters are at that position.
        let window_end = floor_char_boundary(text, end.min(cursor + MAX_CHARS));

        // Prefer a line break, then any character boundary. Splitting mid-word is ugly but
        // splitting mid-character would corrupt the text.
        let cut = text[cursor..window_end]
            .rfind('\n')
            .map(|i| cursor + i + 1)
            .filter(|c| *c > cursor)
            .unwrap_or_else(|| floor_char_boundary(text, (cursor + TARGET_CHARS).min(end)));

        // Guarantee progress: if no usable boundary was found ahead of the cursor, fall back to
        // the window edge. Without this a pathological input could loop forever.
        let cut = if cut > cursor { cut } else { window_end.max(cursor + 1) };

        pieces.push((cursor, cut));
        cursor = cut;
    }

    if cursor < end {
        pieces.push((cursor, end));
    }
    pieces
}

/// Where the overlap for a new chunk should begin.
fn overlap_start(text: &str, previous: Option<&Chunk>, start: usize) -> usize {
    let Some(previous) = previous else {
        return start;
    };
    // Only overlap with text that directly precedes this chunk; a gap means the structure
    // already separated them and repeating across it would be misleading.
    if previous.byte_end != start {
        return start;
    }

    let overlap = (TARGET_CHARS as f32 * OVERLAP_FRACTION) as usize;
    let candidate = start.saturating_sub(overlap).max(previous.byte_start);
    floor_char_boundary(text, candidate)
}

/// Largest character boundary at or below `index`.
pub(crate) fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(chunks: &[Chunk]) -> Vec<&str> {
        chunks.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn empty_text_produces_no_chunks() {
        assert!(chunk("", Handling::PlainText).is_empty());
        assert!(chunk("   \n\n  ", Handling::PlainText).is_empty());
    }

    #[test]
    fn a_short_document_is_one_chunk() {
        let chunks = chunk("Just a short note about the camera angles.", Handling::PlainText);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].ordinal, 0);
    }

    #[test]
    fn markdown_splits_at_headings() {
        let text = format!(
            "# First\n\n{}\n\n# Second\n\n{}\n",
            "Prose about cameras. ".repeat(20),
            "Prose about budgets. ".repeat(20)
        );
        let chunks = chunk(&text, Handling::Markup);

        assert!(chunks.len() >= 2, "expected a split at the heading");
        assert!(chunks[0].text.contains("First"));
        assert!(
            chunks.iter().any(|c| c.text.contains("# Second")),
            "the second heading should start a chunk"
        );
    }

    #[test]
    fn a_heading_inside_a_fenced_block_is_not_a_heading() {
        // It is a shell comment. Splitting there would cut a code example in half.
        let text = format!(
            "# Real heading\n\n{}\n```sh\n# not a heading\necho hi\n```\n{}\n",
            "Body text. ".repeat(30),
            "More body. ".repeat(30)
        );
        let chunks = chunk(&text, Handling::Markup);
        assert!(
            !chunks.iter().any(|c| c.text.trim_start().starts_with("# not a heading")),
            "split inside a code fence: {:?}",
            texts(&chunks)
        );
    }

    #[test]
    fn a_hashtag_is_not_a_heading() {
        let text = format!("Some prose.\n\n#hashtag not a heading\n\n{}\n", "More prose. ".repeat(40));
        let chunks = chunk(&text, Handling::Markup);
        assert!(!chunks.iter().any(|c| c.text.starts_with("#hashtag")));
    }

    #[test]
    fn code_splits_at_top_level_definitions() {
        let text = format!(
            "use std::fmt;\n\nfn first() {{\n{}}}\n\nfn second() {{\n{}}}\n",
            "    let x = 1;\n".repeat(30),
            "    let y = 2;\n".repeat(30)
        );
        let chunks = chunk(&text, Handling::Code);
        assert!(chunks.len() >= 2, "expected a split between definitions");
        assert!(
            chunks.iter().any(|c| c.text.contains("fn second")),
            "the second definition should begin a chunk"
        );
    }

    #[test]
    fn an_indented_definition_does_not_start_a_chunk() {
        // A nested function belongs to its parent; splitting there separates it from the
        // signature that gives it meaning.
        let text = format!(
            "fn outer() {{\n{}    fn inner() {{}}\n{}}}\n",
            "    let a = 1;\n".repeat(20),
            "    let b = 2;\n".repeat(20)
        );
        let chunks = chunk(&text, Handling::Code);
        assert!(
            !chunks.iter().any(|c| c.text.trim_start().starts_with("fn inner")),
            "split at a nested definition: {:?}",
            texts(&chunks)
        );
    }

    #[test]
    fn prose_splits_at_paragraphs() {
        let text = format!("{}\n\n{}\n", "First paragraph. ".repeat(60), "Second paragraph. ".repeat(60));
        let chunks = chunk(&text, Handling::PlainText);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn oversized_sections_are_split() {
        // One paragraph with no internal structure still has to become workable chunks.
        let text = "word ".repeat(5000);
        let chunks = chunk(&text, Handling::PlainText);

        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(
                c.text.len() <= MAX_CHARS + (TARGET_CHARS as f32 * OVERLAP_FRACTION) as usize,
                "chunk of {} bytes exceeds the ceiling",
                c.text.len()
            );
        }
    }

    #[test]
    fn tiny_sections_are_merged_rather_than_emitted_alone() {
        // A chunk of three words matches everything and means nothing.
        let text = "# A\n\nx\n\n# B\n\ny\n\n# C\n\nz\n";
        let chunks = chunk(text, Handling::Markup);
        assert_eq!(chunks.len(), 1, "tiny sections should merge: {:?}", texts(&chunks));
    }

    #[test]
    fn consecutive_chunks_overlap() {
        // Overlap is what stops a boundary from destroying a fact that straddles it.
        let text = "word ".repeat(3000);
        let chunks = chunk(&text, Handling::PlainText);
        assert!(chunks.len() >= 2);

        for pair in chunks.windows(2) {
            assert!(
                pair[1].byte_start < pair[0].byte_end,
                "chunk {} does not overlap its predecessor",
                pair[1].ordinal
            );
        }
    }

    #[test]
    fn offsets_address_the_original_text_exactly() {
        // Offsets are what makes storing the text unnecessary (D16). If they are wrong, every
        // snippet re-read from disk is wrong too.
        let text = format!("# One\n\n{}\n\n# Two\n\n{}\n", "Alpha. ".repeat(50), "Beta. ".repeat(50));
        for c in chunk(&text, Handling::Markup) {
            assert_eq!(
                &text[c.byte_start..c.byte_end],
                c.text,
                "chunk {} offsets do not address its own text",
                c.ordinal
            );
        }
    }

    #[test]
    fn ordinals_are_sequential_from_zero() {
        let text = "word ".repeat(4000);
        for (i, c) in chunk(&text, Handling::PlainText).iter().enumerate() {
            assert_eq!(c.ordinal, i);
        }
    }

    #[test]
    fn the_hash_covers_exactly_the_chunk_text() {
        for c in chunk(&"word ".repeat(3000), Handling::PlainText) {
            assert_eq!(c.hash, blake3::hash(c.text.as_bytes()).to_hex().to_string());
        }
    }

    #[test]
    fn identical_text_hashes_identically_and_differing_text_does_not() {
        // The whole re-embedding economy rests on this.
        let a = chunk("The camera angles were compared.", Handling::PlainText);
        let b = chunk("The camera angles were compared.", Handling::PlainText);
        let c = chunk("The camera angles were compared!", Handling::PlainText);

        assert_eq!(a[0].hash, b[0].hash);
        assert_ne!(a[0].hash, c[0].hash);
    }

    #[test]
    fn editing_one_section_leaves_the_other_hashes_alone() {
        // The property that turns a one-paragraph edit into one embedding call instead of
        // hundreds. If this breaks, ingest cost silently multiplies.
        let head = format!("# Alpha\n\n{}\n\n", "Stable prose about cameras. ".repeat(40));
        let before = format!("{head}# Beta\n\n{}\n", "Original text. ".repeat(40));
        let after = format!("{head}# Beta\n\n{}\n", "Rewritten entirely. ".repeat(40));

        let a = chunk(&before, Handling::Markup);
        let b = chunk(&after, Handling::Markup);

        assert_eq!(a[0].hash, b[0].hash, "an untouched leading section changed hash");
        assert_ne!(
            a.last().unwrap().hash,
            b.last().unwrap().hash,
            "the edited section should have changed"
        );
    }

    #[test]
    fn multibyte_text_is_never_split_mid_character() {
        // Every character is two bytes, so any careless offset arithmetic corrupts it.
        let text = "äöü ".repeat(4000);
        let chunks = chunk(&text, Handling::PlainText);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(text.is_char_boundary(c.byte_start), "start is mid-character");
            assert!(text.is_char_boundary(c.byte_end), "end is mid-character");
            assert_eq!(&text[c.byte_start..c.byte_end], c.text);
        }
    }

    #[test]
    fn a_preview_is_bounded_and_single_line() {
        let chunks = chunk(&format!("# Title\n\n{}", "Body text. ".repeat(200)), Handling::Markup);
        let preview = chunks[0].preview(80);

        assert!(preview.chars().count() <= 81, "preview is not bounded");
        assert!(!preview.contains('\n'), "a preview must be one line");
        assert!(preview.ends_with('…'), "a truncated preview should say so");
    }

    #[test]
    fn a_preview_shorter_than_the_limit_is_not_marked_truncated() {
        let chunks = chunk("Short note.", Handling::PlainText);
        assert_eq!(chunks[0].preview(80), "Short note.");
    }

    #[test]
    fn chunking_is_deterministic() {
        let text = format!("# H\n\n{}\n\n## H2\n\n{}", "Alpha. ".repeat(80), "Beta. ".repeat(80));
        assert_eq!(chunk(&text, Handling::Markup), chunk(&text, Handling::Markup));
    }
}
